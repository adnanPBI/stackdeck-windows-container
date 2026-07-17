<#
.SYNOPSIS
  Preflight checks for local Windows 10/11 PyStack Hyper-V deployment.
.DESCRIPTION
  Runs safe checks by default. Use -EnableFeatures to enable Hyper-V features.
#>
[CmdletBinding()]
param(
  [switch]$EnableFeatures,
  [string]$VmRoot = "$env:SystemDrive\StackDeck\VMs\pystack-linux",
  [string]$ImageRoot = "$env:SystemDrive\StackDeck\Images"
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

function Check-QemuImg {
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
  [pscustomobject]@{ Check = "qemu-img"; Status = "MISSING"; Detail = "Required for the default Ubuntu .img path; provide --image-vhdx to avoid conversion" }
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
$results.Add((Check-Command "Convert-VHD" $true))
$results.Add((Check-Command "ssh" $true))
$results.Add((Check-Command "scp" $true))
$results.Add((Check-Command "ssh-keygen" $true))
$results.Add((Check-QemuImg))
$results.Add((Check-Command "oscdimg" $false))
$results.Add((Check-Command "oscdimg.exe" $false))
$results.Add((Check-Command "genisoimage" $false))
$results.Add((Check-Command "tar" $true))
$results.Add((Check-Command "cargo" $true))
$results.Add((Check-Command "rustc" $true))
$results.Add((Check-Command "node" $false))
$results.Add((Check-Command "npm" $false))

$vmDrive = Split-Path -Qualifier $VmRoot
$imageDrive = Split-Path -Qualifier $ImageRoot
if ($vmDrive -and -not (Test-Path $vmDrive)) {
  $results.Add([pscustomobject]@{ Check = "VM root drive"; Status = "MISSING"; Detail = $vmDrive })
} else {
  New-Item -ItemType Directory -Force -Path $VmRoot | Out-Null
  $results.Add([pscustomobject]@{ Check = "VM root"; Status = "OK"; Detail = $VmRoot })
}
if ($imageDrive -and -not (Test-Path $imageDrive)) {
  $results.Add([pscustomobject]@{ Check = "Image root drive"; Status = "MISSING"; Detail = $imageDrive })
} else {
  New-Item -ItemType Directory -Force -Path $ImageRoot | Out-Null
  $results.Add([pscustomobject]@{ Check = "Image root"; Status = "OK"; Detail = $ImageRoot })
}

$results | Format-Table -AutoSize

$bad = $results | Where-Object { $_.Status -in @("MISSING", "Disabled") -and $_.Check -notin @("oscdimg", "oscdimg.exe", "genisoimage") }
if ($bad) {
  Write-Error "Preflight failed. Fix missing required items above. The default Ubuntu .img path requires qemu-img; oscdimg/genisoimage are optional because the runtime has an IMAPI2 ISO fallback."
}
