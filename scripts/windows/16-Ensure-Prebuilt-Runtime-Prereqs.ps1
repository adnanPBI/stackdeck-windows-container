#Requires -Version 5.1

[CmdletBinding()]
param(
  [switch]$EnableFeatures,
  [switch]$AllowWindows10,
  [string]$ImageVhdx = ""
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$results = New-Object System.Collections.Generic.List[object]

function Add-Check([string]$Name, [bool]$Passed, [string]$Detail) {
  $results.Add([pscustomobject]@{
    Check = $Name
    Status = $(if ($Passed) { "OK" } else { "MISSING" })
    Detail = $Detail
  })
}

function Test-Admin {
  $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
  $principal = [Security.Principal.WindowsPrincipal]::new($identity)
  return $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

function Resolve-Command([string[]]$Names) {
  foreach ($name in $Names) {
    $cmd = Get-Command $name -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($cmd) { return $cmd }
  }
  return $null
}

function Get-CommandDetail($Command) {
  if (-not $Command) { return "not found" }
  $pathProperty = $Command.PSObject.Properties["Path"]
  if ($pathProperty -and $pathProperty.Value) { return [string]$pathProperty.Value }
  $sourceProperty = $Command.PSObject.Properties["Source"]
  if ($sourceProperty -and $sourceProperty.Value) { return [string]$sourceProperty.Value }
  $fullNameProperty = $Command.PSObject.Properties["FullName"]
  if ($fullNameProperty -and $fullNameProperty.Value) { return [string]$fullNameProperty.Value }
  return [string]$Command
}

$admin = Test-Admin
Add-Check "Administrator PowerShell" $admin "Required for Hyper-V runtime initialization"

$os = Get-CimInstance Win32_OperatingSystem
$caption = [string]$os.Caption
$arch = [string]$os.OSArchitecture
$is11 = $caption -match 'Windows 11'
$is10 = $caption -match 'Windows 10'
$editionOk = $caption -match '(?i)\b(Pro|Professional|Enterprise|Education)\b'
$osOk = ($arch -match '64') -and $editionOk -and ($is11 -or ($AllowWindows10 -and $is10))
Add-Check "Supported Windows baseline" $osOk "$caption; architecture=$arch; AllowWindows10=$AllowWindows10"

$cpu = Get-CimInstance Win32_Processor | Select-Object -First 1
$hypervisor = [bool](Get-CimInstance Win32_ComputerSystem).HypervisorPresent
if ($cpu) {
  # Windows may mask raw CPU virtualization / SLAT flags once the hypervisor
  # is running. Feature state, switch and live VM tests remain separate gates.
  $hardwareOk = ([bool]$cpu.VirtualizationFirmwareEnabled -or $hypervisor)
  $slatOk = ([bool]$cpu.SecondLevelAddressTranslationExtensions -or $hypervisor)
  Add-Check "Hardware virtualization" $hardwareOk "Firmware=$($cpu.VirtualizationFirmwareEnabled); HypervisorPresent=$hypervisor"
  Add-Check "SLAT" $slatOk "ProcessorSLAT=$($cpu.SecondLevelAddressTranslationExtensions); HypervisorPresent=$hypervisor"
}

if ($EnableFeatures) {
  if (-not $admin) { throw "-EnableFeatures requires Administrator PowerShell." }
  $hv = Enable-WindowsOptionalFeature -Online -FeatureName Microsoft-Hyper-V-All -All -NoRestart
  $mgmt = Enable-WindowsOptionalFeature -Online -FeatureName Microsoft-Hyper-V-Management-PowerShell -All -NoRestart
  if ($hv.RestartNeeded -or $mgmt.RestartNeeded) {
    Add-Check "Windows restart" $false "Restart Windows, then rerun this preflight without -EnableFeatures."
  }
}

# DISM can emit a terminating COMException to unelevated Windows PowerShell,
# even when -ErrorAction SilentlyContinue is supplied. Report it as MISSING.
$feature = try { Get-WindowsOptionalFeature -Online -FeatureName Microsoft-Hyper-V-All -ErrorAction Stop } catch { $null }
Add-Check "Hyper-V enabled" ([bool]($feature -and $feature.State -eq "Enabled")) $(if ($feature) { "state=$($feature.State)" } else { "feature state unavailable" })

foreach ($cmdName in @("New-VM", "Resize-VHD", "Convert-VHD", "ssh", "scp", "ssh-keygen", "tar")) {
  $cmd = Resolve-Command @($cmdName)
  $detail = Get-CommandDetail $cmd
  Add-Check $cmdName ([bool]$cmd) $detail
}

$defaultSwitch = try { Get-VMSwitch -Name "Default Switch" -ErrorAction Stop } catch { $null }
Add-Check "Hyper-V Default Switch" ([bool]$defaultSwitch) $(if ($defaultSwitch) { $defaultSwitch.Name } else { "not found" })

$qemu = Resolve-Command @("qemu-img.exe", "qemu-img")
if (-not $qemu) {
  foreach ($candidate in @(
    "$env:ProgramFiles\qemu\qemu-img.exe",
    "${env:ProgramFiles(x86)}\qemu\qemu-img.exe",
    "$env:ChocolateyInstall\bin\qemu-img.exe"
  )) {
    if ($candidate -and (Test-Path -LiteralPath $candidate -PathType Leaf)) {
      $qemu = Get-Item -LiteralPath $candidate
      break
    }
  }
}
if ($ImageVhdx) {
  $validVhdx = [IO.Path]::IsPathRooted($ImageVhdx) -and (Test-Path -LiteralPath $ImageVhdx -PathType Leaf) -and ([IO.Path]::GetExtension($ImageVhdx) -ieq ".vhdx")
  Add-Check "Preconverted VHDX" $validVhdx $ImageVhdx
  Add-Check "qemu-img" $true "Optional because -ImageVhdx was supplied"
} else {
  $qemuDetail = if ($qemu) { Get-CommandDetail $qemu } else { "required for the pinned Debian QCOW2 image" }
  Add-Check "qemu-img" ([bool]$qemu) $qemuDetail
}

$isoTool = Resolve-Command @("oscdimg.exe", "oscdimg", "genisoimage")
$isoDetail = if ($isoTool) { Get-CommandDetail $isoTool } else { "optional; StackDeck falls back to Windows IMAPI2" }
$results.Add([pscustomobject]@{ Check = "cloud-init ISO helper"; Status = $(if ($isoTool) { "OK" } else { "OPTIONAL" }); Detail = $isoDetail })

$results | Format-Table -AutoSize
$bad = @($results | Where-Object { $_.Status -eq "MISSING" })
if ($bad.Count -gt 0) {
  throw "Prebuilt runtime prerequisite check failed. Fix the MISSING items above."
}
Write-Host "Prebuilt runtime prerequisites passed." -ForegroundColor Green
