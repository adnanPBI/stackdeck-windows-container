#Requires -Version 5.1
#Requires -RunAsAdministrator

[CmdletBinding()]
param(
  [Parameter(Mandatory = $false)]
  [string]$StackDeckExe = "stackdeck-cli",
  [ValidateNotNullOrEmpty()]
  [string]$VmName = "stackdeck-linux",
  [ValidateNotNullOrEmpty()]
  [string]$VmRoot = "$env:SystemDrive\StackDeck\VMs\stackdeck-linux",
  [ValidateRange(8, 2048)]
  [UInt32]$DiskGB = 10,
  [ValidateRange(1024, 1048576)]
  [UInt32]$MemoryMB = 4096,
  [ValidateRange(1, 256)]
  [UInt32]$Cpus = 2,
  [ValidateNotNullOrEmpty()]
  [string]$SwitchName = "Default Switch",
  [ValidatePattern("^[A-Za-z0-9](?:[A-Za-z0-9.-]{0,252}[A-Za-z0-9])?$")]
  [string]$WindowsHost = $env:COMPUTERNAME,
  [ValidateNotNullOrEmpty()]
  [string]$SmbUser = "$env:USERDOMAIN\$env:USERNAME",
  [Security.SecureString]$SmbPassword,
  [switch]$SkipSmb
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

function Invoke-StackDeck {
  param([Parameter(Mandatory = $true)][string[]]$ArgumentList)

  & $script:StackDeckPath @ArgumentList
  $exitCode = $LASTEXITCODE
  if ($exitCode -ne 0) {
    # Do not echo ArgumentList: configure may contain the SMB password.
    throw "stackdeck failed with exit code $exitCode."
  }
}

function Invoke-StackDeckWithSecretStdin {
  param(
    [Parameter(Mandatory = $true)][string[]]$ArgumentList,
    [Parameter(Mandatory = $true)][string]$Secret
  )

  $previousEncoding = $OutputEncoding
  try {
    $OutputEncoding = [Text.UTF8Encoding]::new($false)
    $Secret | & $script:StackDeckPath @ArgumentList
    $exitCode = $LASTEXITCODE
    if ($exitCode -ne 0) { throw "stackdeck failed with exit code $exitCode." }
  } finally {
    $OutputEncoding = $previousEncoding
  }
}

function Set-StackDeckPrivateDirectoryAcl {
  param([Parameter(Mandatory = $true)][string]$Path)

  $acl = New-Object Security.AccessControl.DirectorySecurity
  $acl.SetAccessRuleProtection($true, $false)
  $inheritance = [Security.AccessControl.InheritanceFlags]"ContainerInherit, ObjectInherit"
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
  [IO.Directory]::SetAccessControl($Path, $acl)
}

function Initialize-StackDeckMachineConfigRoot {
  $dataRoot = Join-Path $env:ProgramData "StackDeck"
  $configRoot = Join-Path $dataRoot "config"
  foreach ($path in @($dataRoot, $configRoot)) {
    if (Test-Path -LiteralPath $path) {
      $item = Get-Item -LiteralPath $path -Force
      if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "Refusing to use a reparse point for StackDeck machine config: $path"
      }
    }
    [IO.Directory]::CreateDirectory($path) | Out-Null
    Set-StackDeckPrivateDirectoryAcl -Path $path
  }
  return $configRoot
}

if (-not [IO.Path]::IsPathRooted($VmRoot)) {
  throw "VM root must be an absolute local filesystem path: $VmRoot"
}
$VmRoot = [IO.Path]::GetFullPath($VmRoot)
[IO.Directory]::CreateDirectory($VmRoot) | Out-Null

if (-not (Get-VMSwitch -Name $SwitchName -ErrorAction SilentlyContinue)) {
  throw "Hyper-V virtual switch was not found: $SwitchName"
}

if (-not $SkipSmb -and -not $SmbPassword) {
  throw "Supply the Windows SMB password as a SecureString, for example: -SmbPassword (Read-Host -AsSecureString)."
}
$parsedWindowsHost = $null
if (-not [Net.IPAddress]::TryParse($WindowsHost, [ref]$parsedWindowsHost)) {
  foreach ($label in $WindowsHost.Split(".")) {
    if (-not $label -or $label.Length -gt 63 -or
        $label.StartsWith("-") -or $label.EndsWith("-")) {
      throw "-WindowsHost must be a valid IPv4 address, DNS name, or NetBIOS name."
    }
  }
} elseif ($parsedWindowsHost.AddressFamily -ne [Net.Sockets.AddressFamily]::InterNetwork) {
  throw "-WindowsHost currently supports IPv4 or a DNS/NetBIOS name, not IPv6."
}

$StackDeckPath = Resolve-ApplicationPath $StackDeckExe
$env:STACKDECK_CONFIG_DIR = Initialize-StackDeckMachineConfigRoot
$passwordPointer = [IntPtr]::Zero
$plainPassword = $null
$configureArgs = $null
try {
  $configureArgs = @(
    "hyperv", "configure",
    "--vm-name", $VmName,
    "--vm-root", $VmRoot,
    "--disk-gb", "$DiskGB",
    "--memory-mb", "$MemoryMB",
    "--cpus", "$Cpus",
    "--switch-name", $SwitchName,
    "--portproxy", "true"
  )
  if (-not $SkipSmb) {
    $passwordPointer = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($SmbPassword)
    $plainPassword = [Runtime.InteropServices.Marshal]::PtrToStringBSTR($passwordPointer)
    if ([string]::IsNullOrWhiteSpace($plainPassword)) {
      throw "SMB password must not be empty."
    }
    $configureArgs += @(
      "--windows-host", $WindowsHost,
      "--smb-user", $SmbUser,
      "--smb-password-stdin"
    )
    Invoke-StackDeckWithSecretStdin -ArgumentList $configureArgs -Secret $plainPassword
  } else {
    Invoke-StackDeck -ArgumentList $configureArgs
  }
} finally {
  $configureArgs = $null
  $plainPassword = $null
  if ($passwordPointer -ne [IntPtr]::Zero) {
    [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($passwordPointer)
  }
}

Invoke-StackDeck -ArgumentList @("hyperv", "ensure-key")
Invoke-StackDeck -ArgumentList @("hyperv", "doctor")
