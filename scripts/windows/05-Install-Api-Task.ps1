#Requires -Version 5.1
#Requires -RunAsAdministrator

[CmdletBinding()]
param(
  [string]$StackDeckExe = "stackdeck-cli",
  [ValidateNotNullOrEmpty()]
  [ValidatePattern("^[^*?\[\]\\/\x00-\x1F]{1,200}$")]
  [string]$TaskName = "StackDeck Local Docker API",
  [string]$HostIp = "127.0.0.1",
  [ValidateRange(1, 65535)]
  [Int32]$Port = 23750,
  [string]$Token = "",
  [switch]$RotateToken,
  [ValidateRange(5, 120)]
  [UInt32]$StartupTimeout = 30
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Resolve-ApplicationPath {
  param([Parameter(Mandatory = $true)][string]$Value)

  if ([IO.Path]::IsPathRooted($Value) -or $Value -match "[\\/]") {
    if (-not (Test-Path -LiteralPath $Value -PathType Leaf)) {
      throw "StackDeck executable was not found: $Value"
    }
    return (Resolve-Path -LiteralPath $Value).ProviderPath
  }
  $command = Get-Command -Name $Value -CommandType Application -ErrorAction SilentlyContinue |
    Select-Object -First 1
  if (-not $command) {
    throw "StackDeck executable was not found on PATH: $Value"
  }
  return $command.Path
}

function ConvertTo-PowerShellLiteral {
  param([Parameter(Mandatory = $true)][string]$Value)
  return "'" + $Value.Replace("'", "''") + "'"
}

function Set-PrivateAcl {
  param(
    [Parameter(Mandatory = $true)][string]$Path,
    [switch]$Directory
  )

  if ($Directory) {
    $acl = New-Object Security.AccessControl.DirectorySecurity
    $inheritance = [Security.AccessControl.InheritanceFlags]"ContainerInherit, ObjectInherit"
  } else {
    $acl = New-Object Security.AccessControl.FileSecurity
    $inheritance = [Security.AccessControl.InheritanceFlags]::None
  }
  $acl.SetAccessRuleProtection($true, $false)
  $propagation = [Security.AccessControl.PropagationFlags]::None
  $access = [Security.AccessControl.FileSystemRights]::FullControl
  $allow = [Security.AccessControl.AccessControlType]::Allow
  $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
  $sids = @(
    $identity.User,
    [Security.Principal.SecurityIdentifier]::new("S-1-5-18"),
    [Security.Principal.SecurityIdentifier]::new("S-1-5-32-544")
  )
  foreach ($sid in $sids) {
    $rule = [Security.AccessControl.FileSystemAccessRule]::new(
      $sid, $access, $inheritance, $propagation, $allow
    )
    [void]$acl.AddAccessRule($rule)
  }

  if ($Directory) {
    [IO.Directory]::SetAccessControl($Path, $acl)
  } else {
    [IO.File]::SetAccessControl($Path, $acl)
  }
}

function Assert-NoReparsePoint {
  param([Parameter(Mandatory = $true)][string]$Path)

  if (Test-Path -LiteralPath $Path) {
    $item = Get-Item -LiteralPath $Path -Force
    if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
      throw "Refusing to use a reparse point for privileged API task data: $Path"
    }
  }
}

function New-ApiToken {
  $bytes = New-Object byte[] 32
  $rng = [Security.Cryptography.RandomNumberGenerator]::Create()
  try {
    $rng.GetBytes($bytes)
  } finally {
    $rng.Dispose()
  }
  return [Convert]::ToBase64String($bytes)
}

function Assert-ValidToken {
  param([Parameter(Mandatory = $true)][string]$Value)
  if ($Value -notmatch "^[\x20-\x7E]{32,}$") {
    throw "The API token must contain at least 32 printable ASCII characters."
  }
}

function Stop-TaskAndWait {
  param([Parameter(Mandatory = $true)][string]$Name)

  $task = Get-ScheduledTask -TaskName $Name -ErrorAction SilentlyContinue
  if (-not $task -or $task.State -notin @("Running", "Queued")) {
    return
  }
  Stop-ScheduledTask -TaskName $Name
  $deadline = [DateTime]::UtcNow.AddSeconds(15)
  do {
    Start-Sleep -Milliseconds 250
    $task = Get-ScheduledTask -TaskName $Name -ErrorAction SilentlyContinue
    if (-not $task -or $task.State -notin @("Running", "Queued")) {
      return
    }
  } while ([DateTime]::UtcNow -lt $deadline)
  throw "Scheduled task '$Name' did not stop within 15 seconds."
}

function Wait-ApiReady {
  param(
    [Parameter(Mandatory = $true)][Uri]$Uri,
    [Parameter(Mandatory = $true)][string]$BearerToken
  )

  $deadline = [DateTime]::UtcNow.AddSeconds($StartupTimeout)
  $lastError = $null
  while ([DateTime]::UtcNow -lt $deadline) {
    try {
      $response = Invoke-WebRequest -Uri $Uri -Headers @{
        Authorization = "Bearer $BearerToken"
      } -UseBasicParsing -TimeoutSec 3
      if ($response.StatusCode -eq 200) {
        $task = Get-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue
        if ($task -and $task.State -eq "Running") {
          Start-Sleep -Seconds 1
          $task = Get-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue
          if ($task -and $task.State -eq "Running") {
            return
          }
        }
        $lastError = "HTTP was healthy, but the registered task was not stably running"
      } else {
        $lastError = "HTTP status $($response.StatusCode)"
      }
    } catch {
      $lastError = $_.Exception.Message
    }
    Start-Sleep -Seconds 1
  }

  $taskInfo = Get-ScheduledTaskInfo -TaskName $TaskName -ErrorAction SilentlyContinue
  $lastTaskResult = if ($taskInfo) { $taskInfo.LastTaskResult } else { "unavailable" }
  throw "Docker API did not become ready within $StartupTimeout seconds. Last task result: $lastTaskResult. Last HTTP error: $lastError"
}

$parsedHost = $null
if (-not [Net.IPAddress]::TryParse($HostIp, [ref]$parsedHost) -or
    -not $parsedHost.Equals([Net.IPAddress]::Loopback)) {
  throw "This installer currently permits only the IPv4 loopback binding 127.0.0.1."
}

$StackDeckPath = Resolve-ApplicationPath $StackDeckExe
$DataRoot = Join-Path $env:ProgramData "StackDeck"
$ConfigRoot = Join-Path $DataRoot "config"
[void](Assert-NoReparsePoint -Path $DataRoot)
[IO.Directory]::CreateDirectory($DataRoot) | Out-Null
Set-PrivateAcl -Path $DataRoot -Directory
[void](Assert-NoReparsePoint -Path $ConfigRoot)
[IO.Directory]::CreateDirectory($ConfigRoot) | Out-Null
Set-PrivateAcl -Path $ConfigRoot -Directory

$TokenPath = Join-Path $ConfigRoot "docker-api-token.txt"
[void](Assert-NoReparsePoint -Path $TokenPath)
$previousTokenExists = Test-Path -LiteralPath $TokenPath -PathType Leaf
$previousToken = if ($previousTokenExists) {
  [IO.File]::ReadAllText($TokenPath).Trim()
} else {
  $null
}

if ($Token) {
  Assert-ValidToken $Token
  if ($previousTokenExists -and -not $RotateToken -and $Token -cne $previousToken) {
    throw "A token already exists. Use -RotateToken to replace it intentionally."
  }
} elseif ($previousTokenExists -and -not $RotateToken) {
  $Token = $previousToken
} else {
  $Token = New-ApiToken
}
Assert-ValidToken $Token

$existingTask = Get-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue
$previousTaskXml = if ($existingTask) {
  Export-ScheduledTask -TaskName $TaskName
} else {
  $null
}
$previousTaskWasRunning = $existingTask -and $existingTask.State -eq "Running"
$installError = $null

try {
  if ($existingTask) {
    Stop-TaskAndWait -Name $TaskName
  }

  [IO.File]::WriteAllText($TokenPath, $Token, [Text.Encoding]::ASCII)
  Set-PrivateAcl -Path $TokenPath

  $tokenLiteral = ConvertTo-PowerShellLiteral $TokenPath
  $configLiteral = ConvertTo-PowerShellLiteral $ConfigRoot
  $exeLiteral = ConvertTo-PowerShellLiteral $StackDeckPath
  $hostLiteral = ConvertTo-PowerShellLiteral $HostIp
  $taskScript = @"
`$ErrorActionPreference = 'Stop'
`$env:STACKDECK_CONFIG_DIR = $configLiteral
`$env:STACKDECK_DOCKER_API_TOKEN = [IO.File]::ReadAllText($tokenLiteral).Trim()
& $exeLiteral daemon serve --host $hostLiteral --port $Port
if (`$LASTEXITCODE -ne 0) { exit `$LASTEXITCODE }
"@
  $encodedCommand = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($taskScript))
  $powerShellPath = (Get-Command powershell.exe -CommandType Application -ErrorAction Stop).Path
  $action = New-ScheduledTaskAction -Execute $powerShellPath `
    -Argument "-NoLogo -NoProfile -NonInteractive -WindowStyle Hidden -ExecutionPolicy Bypass -EncodedCommand $encodedCommand" `
    -WorkingDirectory ([IO.Path]::GetDirectoryName($StackDeckPath))
  $currentUser = [Security.Principal.WindowsIdentity]::GetCurrent().Name
  $trigger = New-ScheduledTaskTrigger -AtLogOn -User $currentUser
  $principal = New-ScheduledTaskPrincipal -UserId $currentUser -LogonType Interactive -RunLevel Highest
  $settings = New-ScheduledTaskSettingsSet -ExecutionTimeLimit ([TimeSpan]::Zero) `
    -RestartCount 3 -RestartInterval (New-TimeSpan -Minutes 1) `
    -StartWhenAvailable -MultipleInstances IgnoreNew `
    -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries

  Register-ScheduledTask -TaskName $TaskName -Action $action -Trigger $trigger `
    -Principal $principal -Settings $settings -Description "StackDeck local Docker API shim" `
    -Force | Out-Null
  Start-ScheduledTask -TaskName $TaskName

  $uriHost = if ($parsedHost.AddressFamily -eq [Net.Sockets.AddressFamily]::InterNetworkV6) {
    "[$HostIp]"
  } else {
    $HostIp
  }
  $pingUri = [Uri]"http://${uriHost}:$Port/_ping"
  Wait-ApiReady -Uri $pingUri -BearerToken $Token
} catch {
  $installError = $_
  try {
    $currentTask = Get-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue
    if ($currentTask) {
      Stop-TaskAndWait -Name $TaskName
    }
    if ($previousTokenExists) {
      [IO.File]::WriteAllText($TokenPath, $previousToken, [Text.Encoding]::ASCII)
      Set-PrivateAcl -Path $TokenPath
    } elseif (Test-Path -LiteralPath $TokenPath) {
      Remove-Item -LiteralPath $TokenPath -Force
    }

    if ($previousTaskXml) {
      Register-ScheduledTask -TaskName $TaskName -Xml $previousTaskXml -Force | Out-Null
      if ($previousTaskWasRunning) {
        Start-ScheduledTask -TaskName $TaskName
      }
    } elseif ($currentTask) {
      Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false
    }
  } catch {
    Write-Warning "API task rollback also failed: $($_.Exception.Message)"
  }
  throw $installError
}

Write-Host "Docker API task installed and healthy on http://${HostIp}:$Port" -ForegroundColor Green
Write-Host "Bearer token saved to $TokenPath"
