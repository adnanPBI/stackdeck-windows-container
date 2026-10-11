<#
.SYNOPSIS
  Preflight checks for the supported StackDeck Windows Hyper-V deployment baseline.
.DESCRIPTION
  Runs safe checks by default. Use -EnableFeatures to enable Hyper-V features.
#>
[CmdletBinding()]
param(
  [switch]$EnableFeatures,
  [switch]$SkipDesktop,
  [switch]$AllowWindows10,
  [string]$ImageVhdx = "",
  [string]$VmRoot = "$env:SystemDrive\StackDeck\VMs\stackdeck-linux",
  [string]$ImageRoot = "$env:SystemDrive\StackDeck\VMs\stackdeck-linux\images"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$MinimumRustVersion = [Version]"1.88.0"
$MinimumCargoVersion = [Version]"1.88.0"
$MinimumNode20Version = [Version]"20.19.0"
$MinimumNode22Version = [Version]"22.12.0"

function Test-Admin {
  $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
  $principal = [Security.Principal.WindowsPrincipal]::new($identity)
  return $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

function Check-Command($Name, $Required = $true) {
  $cmd = Get-Command $Name -ErrorAction SilentlyContinue | Select-Object -First 1
  if ($cmd) {
    $pathProperty = $cmd.PSObject.Properties["Path"]
    $detail = if ($pathProperty) { $pathProperty.Value } else { $cmd.Source }
    [pscustomobject]@{ Check = $Name; Status = "OK"; Detail = $detail }
  } elseif ($Required) {
    [pscustomobject]@{ Check = $Name; Status = "MISSING"; Detail = "Required" }
  } else {
    [pscustomobject]@{ Check = $Name; Status = "OPTIONAL"; Detail = "Not found" }
  }
}

function Check-QemuImg([bool]$Required) {
  $cmd = Get-Command qemu-img.exe -ErrorAction SilentlyContinue
  if (-not $cmd) { $cmd = Get-Command qemu-img -ErrorAction SilentlyContinue }
  if ($cmd) {
    return [pscustomobject]@{ Check = "qemu-img"; Status = "OK"; Detail = $cmd.Source }
  }
  foreach ($candidate in @("$env:ProgramFiles\qemu\qemu-img.exe", "${env:ProgramFiles(x86)}\qemu\qemu-img.exe", "$env:ChocolateyInstall\bin\qemu-img.exe")) {
    if ($candidate -and (Test-Path $candidate)) {
      return [pscustomobject]@{ Check = "qemu-img"; Status = "OK"; Detail = $candidate }
    }
  }
  if ($Required) {
    [pscustomobject]@{ Check = "qemu-img"; Status = "MISSING"; Detail = "Required for qcow2 conversion; provide -ImageVhdx to use a pre-converted image" }
  } else {
    [pscustomobject]@{ Check = "qemu-img"; Status = "OPTIONAL"; Detail = "Not required when -ImageVhdx is supplied" }
  }
}

function Get-CheckedVersionOutput {
  param(
    [Parameter(Mandatory = $true)][string]$CommandName,
    [string[]]$Arguments = @("--version")
  )

  $command = Get-Command $CommandName -CommandType Application -ErrorAction SilentlyContinue |
    Select-Object -First 1
  if (-not $command) {
    return $null
  }
  $output = & $command.Path @Arguments 2>&1
  $exitCode = $LASTEXITCODE
  if ($exitCode -ne 0) {
    return $null
  }
  return ($output | Out-String).Trim()
}

function ConvertTo-Version {
  param([Parameter(Mandatory = $true)][string]$Text)

  $match = [regex]::Match($Text, "(?<version>\d+\.\d+\.\d+)")
  if (-not $match.Success) {
    return $null
  }
  return [Version]$match.Groups["version"].Value
}

function Check-RustToolchain {
  $versionText = Get-CheckedVersionOutput "rustc.exe"
  if (-not $versionText) {
    return [pscustomobject]@{ Check = "rustc"; Status = "MISSING"; Detail = "Required and must execute successfully" }
  }
  $version = ConvertTo-Version $versionText
  if (-not $version -or $version -lt $MinimumRustVersion) {
    return [pscustomobject]@{ Check = "rustc"; Status = "MISSING"; Detail = "Workspace policy requires Rust $MinimumRustVersion or newer; found '$versionText'" }
  }

  $verbose = Get-CheckedVersionOutput -CommandName "rustc.exe" -Arguments @("-vV")
  $hostMatch = [regex]::Match("$verbose", "(?m)^host:\s*(?<host>\S+)\s*$")
  if (-not $hostMatch.Success -or $hostMatch.Groups["host"].Value -ne "x86_64-pc-windows-msvc") {
    $foundHost = if ($hostMatch.Success) { $hostMatch.Groups["host"].Value } else { "unknown" }
    return [pscustomobject]@{ Check = "Rust host"; Status = "MISSING"; Detail = "Expected x86_64-pc-windows-msvc; found $foundHost" }
  }

  return [pscustomobject]@{ Check = "rustc"; Status = "OK"; Detail = "$version; host x86_64-pc-windows-msvc" }
}

function Check-Cargo {
  $versionText = Get-CheckedVersionOutput "cargo.exe"
  if (-not $versionText) {
    return [pscustomobject]@{ Check = "cargo"; Status = "MISSING"; Detail = "Required and must execute successfully" }
  }
  $version = ConvertTo-Version $versionText
  if (-not $version -or $version -lt $MinimumCargoVersion) {
    return [pscustomobject]@{ Check = "cargo"; Status = "MISSING"; Detail = "Workspace policy requires Cargo $MinimumCargoVersion or newer; found '$versionText'" }
  }
  return [pscustomobject]@{ Check = "cargo"; Status = "OK"; Detail = $versionText }
}

function Check-Node {
  param([bool]$Required)

  $versionText = Get-CheckedVersionOutput "node.exe"
  if (-not $versionText) {
    $status = if ($Required) { "MISSING" } else { "OPTIONAL" }
    return [pscustomobject]@{ Check = "node"; Status = $status; Detail = "Node was not found" }
  }
  $version = ConvertTo-Version $versionText
  $supported = $version -and (
    ($version.Major -eq 20 -and $version -ge $MinimumNode20Version) -or
    ($version -ge $MinimumNode22Version)
  )
  if (-not $supported) {
    $status = if ($Required) { "MISSING" } else { "OPTIONAL" }
    return [pscustomobject]@{ Check = "node"; Status = $status; Detail = "Vite requires Node ^20.19.0 or >=22.12.0; found '$versionText'" }
  }
  return [pscustomobject]@{ Check = "node"; Status = "OK"; Detail = $versionText }
}

function Check-Npm {
  param([bool]$Required)

  $versionText = Get-CheckedVersionOutput "npm.cmd"
  if (-not $versionText) {
    $status = if ($Required) { "MISSING" } else { "OPTIONAL" }
    return [pscustomobject]@{ Check = "npm"; Status = $status; Detail = "npm was not found or failed to run" }
  }
  return [pscustomobject]@{ Check = "npm"; Status = "OK"; Detail = $versionText }
}

function Resolve-LocalRoot {
  param(
    [Parameter(Mandatory = $true)][string]$Path,
    [Parameter(Mandatory = $true)][string]$Label
  )

  if (-not [IO.Path]::IsPathRooted($Path)) {
    throw "$Label must be an absolute local filesystem path: $Path"
  }
  return [IO.Path]::GetFullPath($Path)
}

$results = New-Object System.Collections.Generic.List[object]
$admin = Test-Admin
$results.Add([pscustomobject]@{ Check = "Administrator"; Status = $(if($admin){"OK"}else{"MISSING"}); Detail = "Run PowerShell as Administrator for install/init" })

$os = Get-CimInstance Win32_OperatingSystem
$caption = [string]$os.Caption
$architecture = [string]$os.OSArchitecture
$isWindows11 = $caption -match 'Windows 11'
$isWindows10 = $caption -match 'Windows 10'
$isHyperVEdition = $caption -match '(?i)\b(Pro|Professional|Enterprise|Education)\b'
$is64Bit = $architecture -match '64'
$windowsSupported = $is64Bit -and $isHyperVEdition -and ($isWindows11 -or ($AllowWindows10 -and $isWindows10))
$windowsDetail = "$caption; architecture=$architecture"
if ($isWindows10 -and -not $AllowWindows10) {
  $windowsDetail += "; Windows 10 requires explicit -AllowWindows10 and qualification against the client's supported ESU/LTSC estate"
}
$results.Add([pscustomobject]@{
  Check = "Supported Windows edition"
  Status = $(if($windowsSupported){"OK"}else{"MISSING"})
  Detail = $windowsDetail
})

$cpu = Get-CimInstance Win32_Processor | Select-Object -First 1
$hypervisorPresent = [bool](Get-CimInstance Win32_ComputerSystem).HypervisorPresent
if ($cpu) {
  # A running hypervisor may mask the processor feature flags.
  # Hyper-V installation, switch discovery and real VM health are separate gates.
  $virtualization = ([bool]$cpu.VirtualizationFirmwareEnabled -or $hypervisorPresent)
  $slat = ([bool]$cpu.SecondLevelAddressTranslationExtensions -or $hypervisorPresent)
  $results.Add([pscustomobject]@{
    Check = "Hardware virtualization"
    Status = $(if($virtualization){"OK"}else{"MISSING"})
    Detail = "VirtualizationFirmwareEnabled=$($cpu.VirtualizationFirmwareEnabled); HypervisorPresent=$hypervisorPresent"
  })
  $results.Add([pscustomobject]@{
    Check = "SLAT"
    Status = $(if($slat){"OK"}else{"MISSING"})
    Detail = "SecondLevelAddressTranslationExtensions=$($cpu.SecondLevelAddressTranslationExtensions); HypervisorPresent=$hypervisorPresent"
  })
}

if ($EnableFeatures) {
  if (-not $admin) { throw "-EnableFeatures requires Administrator PowerShell." }
  $hyperVResult = Enable-WindowsOptionalFeature -Online -FeatureName Microsoft-Hyper-V-All -All -NoRestart
  $managementResult = Enable-WindowsOptionalFeature -Online -FeatureName Microsoft-Hyper-V-Management-PowerShell -All -NoRestart
  if ($hyperVResult.RestartNeeded -or $managementResult.RestartNeeded) {
    $results.Add([pscustomobject]@{ Check = "Windows restart"; Status = "MISSING"; Detail = "Restart Windows, then rerun prerequisites without -EnableFeatures" })
  }
}

$feature = try { Get-WindowsOptionalFeature -Online -FeatureName Microsoft-Hyper-V-All -ErrorAction Stop } catch { $null }
$featureStatus = if ($feature -and $feature.State -eq "Enabled") { "OK" } else { "MISSING" }
$featureDetail = if ($feature) { "Windows optional feature state: $($feature.State)" } else { "Unable to query Windows optional feature state" }
$results.Add([pscustomobject]@{ Check = "Microsoft-Hyper-V-All"; Status = $featureStatus; Detail = $featureDetail })
$results.Add((Check-Command "New-VM" $true))
$results.Add((Check-Command "Resize-VHD" $true))
$results.Add((Check-Command "Convert-VHD" $true))
$defaultSwitch = try { Get-VMSwitch -Name "Default Switch" -ErrorAction Stop } catch { $null }
$results.Add([pscustomobject]@{ Check = "Hyper-V Default Switch"; Status = $(if($defaultSwitch){"OK"}else{"MISSING"}); Detail = $(if($defaultSwitch){$defaultSwitch.Name}else{"Required by the default StackDeck configuration"}) })
$results.Add((Check-Command "ssh" $true))
$results.Add((Check-Command "scp" $true))
$results.Add((Check-Command "ssh-keygen" $true))
$results.Add((Check-QemuImg (-not [bool]$ImageVhdx)))
$results.Add((Check-Command "oscdimg" $false))
$results.Add((Check-Command "oscdimg.exe" $false))
$results.Add((Check-Command "genisoimage" $false))
$results.Add((Check-Command "tar" $true))
$results.Add((Check-Cargo))
$results.Add((Check-RustToolchain))
$results.Add((Check-Node (-not $SkipDesktop)))
$results.Add((Check-Npm (-not $SkipDesktop)))

if ($ImageVhdx) {
  if (-not [IO.Path]::IsPathRooted($ImageVhdx) -or -not (Test-Path -LiteralPath $ImageVhdx -PathType Leaf)) {
    $results.Add([pscustomobject]@{ Check = "Image VHDX"; Status = "MISSING"; Detail = "Expected an existing absolute file path: $ImageVhdx" })
  } elseif ([IO.Path]::GetExtension($ImageVhdx) -ne ".vhdx") {
    $results.Add([pscustomobject]@{ Check = "Image VHDX"; Status = "MISSING"; Detail = "Image must use the .vhdx extension: $ImageVhdx" })
  } else {
    $results.Add([pscustomobject]@{ Check = "Image VHDX"; Status = "OK"; Detail = (Resolve-Path -LiteralPath $ImageVhdx).ProviderPath })
  }
}

$VmRoot = Resolve-LocalRoot $VmRoot "VM root"
$ImageRoot = Resolve-LocalRoot $ImageRoot "Image root"
$vmDrive = Split-Path -Qualifier $VmRoot
$imageDrive = Split-Path -Qualifier $ImageRoot
if ($vmDrive -and -not (Test-Path $vmDrive)) {
  $results.Add([pscustomobject]@{ Check = "VM root drive"; Status = "MISSING"; Detail = $vmDrive })
} else {
  [IO.Directory]::CreateDirectory($VmRoot) | Out-Null
  $results.Add([pscustomobject]@{ Check = "VM root"; Status = "OK"; Detail = $VmRoot })
}
if ($imageDrive -and -not (Test-Path $imageDrive)) {
  $results.Add([pscustomobject]@{ Check = "Image root drive"; Status = "MISSING"; Detail = $imageDrive })
} else {
  [IO.Directory]::CreateDirectory($ImageRoot) | Out-Null
  $results.Add([pscustomobject]@{ Check = "Image root"; Status = "OK"; Detail = $ImageRoot })
}

$results | Format-Table -AutoSize

$bad = $results | Where-Object { $_.Status -eq "MISSING" -and $_.Check -notin @("oscdimg", "oscdimg.exe", "genisoimage") }
if ($bad) {
  Write-Error "Preflight failed. Fix the required items above. oscdimg/genisoimage remain optional because the runtime has an IMAPI2 ISO fallback."
}
