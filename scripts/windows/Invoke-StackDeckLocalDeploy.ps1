#Requires -Version 5.1

[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)]
  [ValidateSet("Prereqs", "Build", "Configure", "Init", "Smoke", "ApiTask", "Diagnostics", "All")]
  [string]$Phase,
  [string]$StackDeckExe = "stackdeck-cli",
  [string]$Sha256 = "",
  [string]$Sha512 = "",
  [Security.SecureString]$SmbPassword,
  [string]$VmName = "stackdeck-linux",
  [string]$VmRoot = "$env:SystemDrive\StackDeck\VMs\stackdeck-linux",
  [string]$ImageRoot = "",
  [UInt32]$DiskGB = 10,
  [UInt32]$MemoryMB = 4096,
  [UInt32]$Cpus = 2,
  [string]$SwitchName = "Default Switch",
  [string]$WindowsHost = $env:COMPUTERNAME,
  [string]$SmbUser = "$env:USERDOMAIN\$env:USERNAME",
  [switch]$SkipSmb,
  [string]$ImageVhdx = "",
  [string]$ImageUrl = "",
  [ValidateRange(60, 3600)]
  [UInt32]$InitTimeout = 900,
  [string]$ComposeFile = "examples\smoke\docker-compose.yml",
  [string]$SmokeServiceName = "web",
  [Uri]$SmokeHealthUri = "http://127.0.0.1:18080/",
  [ValidateRange(10, 600)]
  [UInt32]$SmokeStartupTimeout = 90,
  [switch]$KeepSmokeRunning,
  [switch]$RemoveSmokeVolumes,
  [ValidatePattern("^[^*?\[\]\\/\x00-\x1F]{1,200}$")]
  [string]$ApiTaskName = "StackDeck Local Docker API",
  [string]$ApiHostIp = "127.0.0.1",
  [ValidateRange(1, 65535)]
  [Int32]$ApiPort = 23750,
  [string]$ApiToken = "",
  [switch]$RotateApiToken,
  [ValidateRange(5, 120)]
  [UInt32]$ApiStartupTimeout = 30,
  [string]$DiagnosticsConfig = "stack.json",
  [string]$DiagnosticsOutput = ".stackdeck\diagnostics",
  [ValidateRange(1, 100000)]
  [UInt32]$DiagnosticsTail = 200,
  [switch]$EnableFeatures,
  [switch]$AllowWindows10,
  [switch]$SkipTests,
  [switch]$SkipDesktop,
  [switch]$InstallApiTask,
  [switch]$CollectDiagnostics
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$Root = $PSScriptRoot
$RepoRoot = (Resolve-Path -LiteralPath (Join-Path $Root "..\..")).ProviderPath
$EffectiveImageRoot = if ($ImageRoot) { $ImageRoot } else { Join-Path $VmRoot "images" }

function Test-Admin {
  $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
  $principal = [Security.Principal.WindowsPrincipal]::new($identity)
  return $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

function Assert-Admin {
  if (-not (Test-Admin)) {
    throw "Phase '$Phase' requires Administrator PowerShell."
  }
}

function Resolve-InputFile {
  param([Parameter(Mandatory = $true)][string]$Value)

  if ([IO.Path]::IsPathRooted($Value)) {
    return $Value
  }
  if (Test-Path -LiteralPath $Value -PathType Leaf) {
    return (Resolve-Path -LiteralPath $Value).ProviderPath
  }
  $candidate = Join-Path $RepoRoot $Value
  if (Test-Path -LiteralPath $candidate -PathType Leaf) {
    return (Resolve-Path -LiteralPath $candidate).ProviderPath
  }
  return $Value
}

function Stop-ApiTask {
  $task = Get-ScheduledTask -TaskName $ApiTaskName -ErrorAction SilentlyContinue
  if (-not $task -or $task.State -notin @("Running", "Queued")) {
    return $false
  }

  Stop-ScheduledTask -TaskName $ApiTaskName
  $deadline = [DateTime]::UtcNow.AddSeconds(15)
  do {
    Start-Sleep -Milliseconds 250
    $task = Get-ScheduledTask -TaskName $ApiTaskName -ErrorAction SilentlyContinue
    if (-not $task -or $task.State -notin @("Running", "Queued")) {
      return $true
    }
  } while ([DateTime]::UtcNow -lt $deadline)
  throw "Scheduled task '$ApiTaskName' did not stop within 15 seconds."
}

function Restore-ApiTask {
  param([bool]$WasRunning)

  if (-not $WasRunning) {
    return
  }
  $task = Get-ScheduledTask -TaskName $ApiTaskName -ErrorAction SilentlyContinue
  if (-not $task) {
    throw "Cannot restore scheduled task because it no longer exists: $ApiTaskName"
  }
  if ($task.State -ne "Running") {
    Start-ScheduledTask -TaskName $ApiTaskName
  }
  $deadline = [DateTime]::UtcNow.AddSeconds(15)
  do {
    Start-Sleep -Milliseconds 250
    $task = Get-ScheduledTask -TaskName $ApiTaskName -ErrorAction SilentlyContinue
    if ($task -and $task.State -eq "Running") {
      Start-Sleep -Seconds 1
      $task = Get-ScheduledTask -TaskName $ApiTaskName -ErrorAction SilentlyContinue
      if ($task -and $task.State -eq "Running") {
        return
      }
    }
  } while ([DateTime]::UtcNow -lt $deadline)
  throw "Scheduled task '$ApiTaskName' did not return to a stable Running state."
}

switch ($Phase) {
  "Prereqs" {
    $params = @{ VmRoot = $VmRoot; ImageRoot = $EffectiveImageRoot }
    if ($EnableFeatures) { $params["EnableFeatures"] = $true }
    if ($AllowWindows10) { $params["AllowWindows10"] = $true }
    if ($SkipDesktop) { $params["SkipDesktop"] = $true }
    if ($ImageVhdx) { $params["ImageVhdx"] = (Resolve-InputFile $ImageVhdx) }
    & (Join-Path $Root "00-Ensure-Prereqs.ps1") @params
  }
  "Build" {
    $taskWasRunning = Stop-ApiTask
    $primaryError = $null
    try {
      $params = @{}
      if ($SkipTests) { $params["SkipTests"] = $true }
      if ($SkipDesktop) { $params["SkipDesktop"] = $true }
      & (Join-Path $Root "01-Build-Release.ps1") @params
    } catch {
      $primaryError = $_
    } finally {
      try {
        Restore-ApiTask -WasRunning $taskWasRunning
      } catch {
        if ($primaryError) {
          Write-Warning "The build failed and the API task could not be restored: $($_.Exception.Message)"
        } else {
          $primaryError = $_
        }
      }
    }
    if ($primaryError) {
      throw $primaryError
    }
  }
  "Configure" {
    Assert-Admin
    $params = @{
      StackDeckExe = $StackDeckExe; VmName = $VmName; VmRoot = $VmRoot
      DiskGB = $DiskGB; MemoryMB = $MemoryMB; Cpus = $Cpus
      SwitchName = $SwitchName; WindowsHost = $WindowsHost; SmbUser = $SmbUser
    }
    if ($SkipSmb) { $params["SkipSmb"] = $true }
    elseif ($SmbPassword) { $params["SmbPassword"] = $SmbPassword }
    & (Join-Path $Root "02-Configure-HyperV.ps1") @params
  }
  "Init" {
    Assert-Admin
    $params = @{ StackDeckExe = $StackDeckExe; Timeout = $InitTimeout }
    if ($ImageUrl) { $params["ImageUrl"] = $ImageUrl }
    if ($ImageVhdx) { $params["ImageVhdx"] = (Resolve-InputFile $ImageVhdx) }
    if ($Sha256) { $params["Sha256"] = $Sha256 }
    if ($Sha512) { $params["Sha512"] = $Sha512 }
    & (Join-Path $Root "03-Init-HyperV-Runtime.ps1") @params
  }
  "Smoke" {
    Assert-Admin
    $params = @{
      StackDeckExe = $StackDeckExe; ComposeFile = (Resolve-InputFile $ComposeFile)
      ServiceName = $SmokeServiceName; HealthUri = $SmokeHealthUri
      StartupTimeout = $SmokeStartupTimeout
    }
    if ($KeepSmokeRunning) { $params["KeepRunning"] = $true }
    if ($RemoveSmokeVolumes) { $params["RemoveVolumes"] = $true }
    & (Join-Path $Root "04-Smoke-Test.ps1") @params
  }
  "ApiTask" {
    Assert-Admin
    $params = @{
      StackDeckExe = $StackDeckExe; TaskName = $ApiTaskName
      HostIp = $ApiHostIp; Port = $ApiPort; StartupTimeout = $ApiStartupTimeout
    }
    if ($ApiToken) { $params["Token"] = $ApiToken }
    if ($RotateApiToken) { $params["RotateToken"] = $true }
    & (Join-Path $Root "05-Install-Api-Task.ps1") @params
  }
  "Diagnostics" {
    Assert-Admin
    & (Join-Path $Root "06-Diagnostics.ps1") `
      -StackDeckExe $StackDeckExe `
      -Config (Resolve-InputFile $DiagnosticsConfig) `
      -Output $DiagnosticsOutput `
      -Tail $DiagnosticsTail
  }
  "All" {
    Assert-Admin

    if (-not $SkipSmb -and -not $SmbPassword) {
      throw "Phase 'All' requires -SmbPassword. Prefer (Read-Host -AsSecureString)."
    }
    if ($Sha256 -and $Sha256 -notmatch "^[A-Fa-f0-9]{64}$") {
      throw "-Sha256 must contain exactly 64 hexadecimal characters."
    }
    if ($Sha512 -and $Sha512 -notmatch "^[A-Fa-f0-9]{128}$") {
      throw "-Sha512 must contain exactly 128 hexadecimal characters."
    }
    if (-not $ImageVhdx -and $ImageUrl) {
      $parsedImageUri = $null
      if (-not [Uri]::TryCreate($ImageUrl, [UriKind]::Absolute, [ref]$parsedImageUri) -or
          $parsedImageUri.Scheme -ne "https") {
        throw "-ImageUrl must be an absolute HTTPS URL."
      }
      if (-not $Sha256 -and -not $Sha512) {
        throw "Custom downloaded images require -Sha256 or -Sha512."
      }
    }
    if ($CollectDiagnostics) {
      $resolvedDiagnosticsConfig = Resolve-InputFile $DiagnosticsConfig
      if (-not (Test-Path -LiteralPath $resolvedDiagnosticsConfig -PathType Leaf)) {
        throw "Diagnostics configuration was not found: $DiagnosticsConfig"
      }
      $resolvedDiagnosticsOutput = if ([IO.Path]::IsPathRooted($DiagnosticsOutput)) {
        [IO.Path]::GetFullPath($DiagnosticsOutput)
      } else {
        [IO.Path]::GetFullPath((Join-Path (Get-Location).Path $DiagnosticsOutput))
      }
      if (Test-Path -LiteralPath $resolvedDiagnosticsOutput) {
        throw "Diagnostics output already exists: $resolvedDiagnosticsOutput"
      }
    }

    $prereqParams = @{ VmRoot = $VmRoot; ImageRoot = $EffectiveImageRoot }
    if ($EnableFeatures) { $prereqParams["EnableFeatures"] = $true }
    if ($AllowWindows10) { $prereqParams["AllowWindows10"] = $true }
    if ($SkipDesktop) { $prereqParams["SkipDesktop"] = $true }
    if ($ImageVhdx) { $prereqParams["ImageVhdx"] = (Resolve-InputFile $ImageVhdx) }
    # This stops the deployment before build/init when enabling Hyper-V requires
    # a restart; 00-Ensure-Prereqs reports that state as a failure.
    & (Join-Path $Root "00-Ensure-Prereqs.ps1") @prereqParams

    $taskWasRunning = Stop-ApiTask
    $buildError = $null
    try {
      $buildParams = @{}
      if ($SkipTests) { $buildParams["SkipTests"] = $true }
      if ($SkipDesktop) { $buildParams["SkipDesktop"] = $true }
      & (Join-Path $Root "01-Build-Release.ps1") @buildParams
    } catch {
      $buildError = $_
    } finally {
      try {
        Restore-ApiTask -WasRunning $taskWasRunning
      } catch {
        if ($buildError) {
          Write-Warning "The build failed and the API task could not be restored: $($_.Exception.Message)"
        } else {
          $buildError = $_
        }
      }
    }
    if ($buildError) {
      throw $buildError
    }

    $BuiltStackDeck = Join-Path $RepoRoot "target\release\stackdeck-cli.exe"
    if (-not (Test-Path -LiteralPath $BuiltStackDeck -PathType Leaf)) {
      throw "Built StackDeck executable was not found: $BuiltStackDeck"
    }

    $configureParams = @{
      StackDeckExe = $BuiltStackDeck; VmName = $VmName; VmRoot = $VmRoot
      DiskGB = $DiskGB; MemoryMB = $MemoryMB; Cpus = $Cpus
      SwitchName = $SwitchName; WindowsHost = $WindowsHost; SmbUser = $SmbUser
    }
    if ($SkipSmb) { $configureParams["SkipSmb"] = $true }
    elseif ($SmbPassword) { $configureParams["SmbPassword"] = $SmbPassword }
    & (Join-Path $Root "02-Configure-HyperV.ps1") @configureParams

    $initParams = @{
      StackDeckExe = $BuiltStackDeck; ImageUrl = $ImageUrl; Timeout = $InitTimeout
    }
    if ($ImageVhdx) { $initParams["ImageVhdx"] = (Resolve-InputFile $ImageVhdx) }
    if ($Sha256) { $initParams["Sha256"] = $Sha256 }
    if ($Sha512) { $initParams["Sha512"] = $Sha512 }
    & (Join-Path $Root "03-Init-HyperV-Runtime.ps1") @initParams

    $smokeParams = @{
      StackDeckExe = $BuiltStackDeck; ComposeFile = (Resolve-InputFile $ComposeFile)
      ServiceName = $SmokeServiceName; HealthUri = $SmokeHealthUri
      StartupTimeout = $SmokeStartupTimeout
    }
    if ($KeepSmokeRunning) { $smokeParams["KeepRunning"] = $true }
    if ($RemoveSmokeVolumes) { $smokeParams["RemoveVolumes"] = $true }
    & (Join-Path $Root "04-Smoke-Test.ps1") @smokeParams

    if ($InstallApiTask) {
      $apiParams = @{
        StackDeckExe = $BuiltStackDeck; TaskName = $ApiTaskName
        HostIp = $ApiHostIp; Port = $ApiPort; StartupTimeout = $ApiStartupTimeout
      }
      if ($ApiToken) { $apiParams["Token"] = $ApiToken }
      if ($RotateApiToken) { $apiParams["RotateToken"] = $true }
      & (Join-Path $Root "05-Install-Api-Task.ps1") @apiParams
    }

    if ($CollectDiagnostics) {
      & (Join-Path $Root "06-Diagnostics.ps1") `
        -StackDeckExe $BuiltStackDeck `
        -Config $resolvedDiagnosticsConfig `
        -Output $DiagnosticsOutput `
        -Tail $DiagnosticsTail
    }
  }
}
