<#
.SYNOPSIS
  Preflight checks for local Windows 10/11 PyStack Hyper-V deployment.
.DESCRIPTION
  Runs safe checks by default. Use -EnableFeatures to enable Hyper-V features.
#>
[CmdletBinding()]
param(
  [switch]$EnableFeatures,
  [string]$VmRoot = "D:\StackDeck\VMs\pystack-linux",
  [string]$ImageRoot = "D:\StackDeck\Images"
)

$ErrorActionPreference = "Stop"

function Test-Admin {
  $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
  $principal = [Security.Principal.WindowsPrincipal]::new($identity)
  return $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

function Check-Command($Name, $Required = $true) {
  $cmd = Get-Command $Name -ErrorAction SilentlyContinue
  if ($cmd) {
    [pscustomobject]@{ Check = $Name; Status = "OK"; Detail = $cmd.Source }
  } elseif ($Required) {
    [pscustomobject]@{ Check = $Name; Status = "MISSING"; Detail = "Required" }
  } else {
    [pscustomobject]@{ Check = $Name; Status = "OPTIONAL"; Detail = "Not found" }
  }
}

$results = New-Object System.Collections.Generic.List[object]
$admin = Test-Admin
$results.Add([pscustomobject]@{ Check = "Administrator"; Status = $(if($admin){"OK"}else{"MISSING"}); Detail = "Run PowerShell as Administrator for install/init" })

$os = Get-CimInstance Win32_OperatingSystem
$results.Add([pscustomobject]@{ Check = "Windows"; Status = "OK"; Detail = $os.Caption })

if ($EnableFeatures) {
  if (-not $admin) { throw "-EnableFeatures requires Administrator PowerShell." }
  Enable-WindowsOptionalFeature -Online -FeatureName Microsoft-Hyper-V-All -All -NoRestart | Out-Null
  Enable-WindowsOptionalFeature -Online -FeatureName Microsoft-Hyper-V-Management-PowerShell -All -NoRestart | Out-Null
  Write-Warning "Restart Windows if Hyper-V was newly enabled."
}

$feature = Get-WindowsOptionalFeature -Online -FeatureName Microsoft-Hyper-V-All -ErrorAction SilentlyContinue
$results.Add([pscustomobject]@{ Check = "Microsoft-Hyper-V-All"; Status = $feature.State; Detail = "Windows optional feature" })
$results.Add((Check-Command "New-VM" $true))
$results.Add((Check-Command "Resize-VHD" $true))
$results.Add((Check-Command "ssh" $true))
$results.Add((Check-Command "scp" $true))
$results.Add((Check-Command "ssh-keygen" $true))
$results.Add((Check-Command "qemu-img" $false))
$results.Add((Check-Command "qemu-img.exe" $false))
$results.Add((Check-Command "oscdimg" $false))
$results.Add((Check-Command "oscdimg.exe" $false))
$results.Add((Check-Command "genisoimage" $false))
$results.Add((Check-Command "tar" $true))
$results.Add((Check-Command "cargo" $true))
$results.Add((Check-Command "rustc" $true))
$results.Add((Check-Command "node" $false))
$results.Add((Check-Command "npm" $false))

if (-not (Test-Path "D:\")) {
  $results.Add([pscustomobject]@{ Check = "D drive"; Status = "MISSING"; Detail = "Configure another -VmRoot/-ImageRoot" })
} else {
  New-Item -ItemType Directory -Force -Path $VmRoot | Out-Null
  New-Item -ItemType Directory -Force -Path $ImageRoot | Out-Null
  $results.Add([pscustomobject]@{ Check = "VM root"; Status = "OK"; Detail = $VmRoot })
  $results.Add([pscustomobject]@{ Check = "Image root"; Status = "OK"; Detail = $ImageRoot })
}

$results | Format-Table -AutoSize

$bad = $results | Where-Object { $_.Status -in @("MISSING", "Disabled") -and $_.Check -notin @("qemu-img", "qemu-img.exe", "oscdimg", "oscdimg.exe", "genisoimage") }
if ($bad) {
  Write-Error "Preflight failed. Fix missing required items above. For image conversion you also need qemu-img unless you provide a preconverted VHDX. For cloud-init ISO you need oscdimg or genisoimage."
}
