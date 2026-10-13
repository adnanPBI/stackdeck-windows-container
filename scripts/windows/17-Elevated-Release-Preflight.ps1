#Requires -Version 5.1
<#
.SYNOPSIS
  Evidence-backed Windows host qualification. Use -EnableFeatures only from
  Administrator PowerShell; never restarts or mutates existing VMs.
#>
[CmdletBinding()]
param(
  [switch]$EnableFeatures,
  [string]$Output = ""
)
Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$repo = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "..\..")).ProviderPath
if (-not $Output) { $Output = Join-Path $repo "target\release-evidence\elevated-host-preflight.json" }
$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = [Security.Principal.WindowsPrincipal]::new($identity)
$admin = $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
$items = New-Object System.Collections.Generic.List[object]
function Record([string]$Check, [bool]$Passed, [string]$Detail) {
  $script:items.Add([pscustomobject]@{
    check = $Check
    status = $(if($Passed){"PASS"}else{"BLOCKED"})
    detail = $Detail
  })
}
Record "Elevated process token" $admin "User=$($identity.Name); Admin=$admin"
$os = Get-CimInstance Win32_OperatingSystem
$baseline = ($os.Caption -match 'Windows 11') -and ($os.OSArchitecture -match '64') -and ($os.Caption -match '(Pro|Enterprise|Education)')
Record "Windows support baseline" $baseline "$($os.Caption); $($os.Version)"
$hypervisor = [bool](Get-CimInstance Win32_ComputerSystem).HypervisorPresent
Record "Hypervisor available" $hypervisor "HypervisorPresent=$hypervisor"
$cpu = Get-CimInstance Win32_Processor | Select-Object -First 1
$virtualization = [bool]($hypervisor -or ($cpu -and $cpu.VirtualizationFirmwareEnabled))
Record "Hardware virtualization" $virtualization "FirmwareVirtualization=$($cpu.VirtualizationFirmwareEnabled)"
$needsRestart = $false
if ($EnableFeatures) {
  if (-not $admin) { throw "Enabling Windows features requires UAC-approved Administrator PowerShell." }
  foreach ($featureName in @("Microsoft-Hyper-V-All","Microsoft-Hyper-V-Management-PowerShell")) {
    $current = Get-WindowsOptionalFeature -Online -FeatureName $featureName
    if ($current.State -ne "Enabled") {
      $result = Enable-WindowsOptionalFeature -Online -FeatureName $featureName -All -NoRestart
      if ($result.RestartNeeded) { $needsRestart = $true }
    }
  }
}
$feature = $null
if ($admin) {
  try { $feature = Get-WindowsOptionalFeature -Online -FeatureName Microsoft-Hyper-V-All -ErrorAction Stop }
  catch { Record "Hyper-V feature query" $false $_.Exception.Message }
}
$enabled = [bool]($feature -and $feature.State -eq "Enabled")
Record "Hyper-V Windows feature enabled" $enabled $(if($feature){"State=$($feature.State)"}else{"Requires an elevated feature query"})
$switches = @()
$vms = @()
$cimVms = @()
$vmEnumerationOk = $false
if ($admin -and $enabled) {
  try { $switches = @(Get-VMSwitch -ErrorAction Stop) } catch { Record "Hyper-V switch enumeration" $false $_.Exception.Message }
  try { $vms = @(Get-VM -ErrorAction Stop); $vmEnumerationOk = $true }
  catch { Record "Hyper-V VM enumeration" $false $_.Exception.Message }
  # Independent CIM inventory is critical: Get-VM can fail on corrupt/orphaned
  # registrations, and a failed Get-VM must never imply that the VM is absent.
  try {
    $cimVms = @(Get-CimInstance -Namespace root\virtualization\v2 -ClassName Msvm_ComputerSystem -ErrorAction Stop |
      Where-Object { $_.Caption -eq "Virtual Machine" })
  } catch { Record "Hyper-V CIM inventory" $false $_.Exception.Message }
}
$defaultSwitch = @($switches | Where-Object { $_.Name -eq "Default Switch" })
Record "Hyper-V Default Switch" ($defaultSwitch.Count -eq 1) ("Available: "+(($switches | ForEach-Object Name) -join ", "))
$knownNames = @($cimVms | ForEach-Object ElementName)
$targetRegistered = $knownNames -contains "stackdeck-linux"
Record "Target VM inventory" ($admin -and $enabled -and $vmEnumerationOk) "CIM registered stackdeck-linux=$targetRegistered; CIM VMs=$($knownNames -join ', '); Get-VM succeeded=$vmEnumerationOk"
$disk = Get-PSDrive -Name C -ErrorAction SilentlyContinue
Record "System disk space >= 20 GB" ([bool]($disk -and $disk.Free -ge 20GB)) $(if($disk){"FreeGB=$([math]::Round($disk.Free/1GB,2))"}else{"C drive unavailable"})
$qemu = Get-Command qemu-img.exe -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $qemu) {
  $x86 = [Environment]::GetEnvironmentVariable("ProgramFiles(x86)")
  foreach ($candidate in @("$env:ProgramFiles\qemu\qemu-img.exe","$x86\qemu\qemu-img.exe")) {
    if (Test-Path -LiteralPath $candidate -PathType Leaf) { $qemu = Get-Item -LiteralPath $candidate; break }
  }
}
Record "qemu-img installed" ([bool]$qemu) $(if($qemu){"Found"}else{"Required for pinned Debian QCOW2 image"})
$seven = Get-Command 7z.exe -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $seven) {
  foreach ($candidate in @("$env:ProgramFiles\7-Zip\7z.exe", (Join-Path $env:LOCALAPPDATA "StackDeck\release-tools\7zip\portable\7z.exe"))) {
    if (Test-Path -LiteralPath $candidate -PathType Leaf) { $seven = Get-Item -LiteralPath $candidate; break }
  }
}
Record "7-Zip full extractor installed" ([bool]$seven) $(if($seven){"Found"}else{"Required to inspect MSI/NSIS contents"})
$signTool = Get-Command signtool.exe -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $signTool) {
  $kitbase = [Environment]::GetEnvironmentVariable("ProgramFiles(x86)")
  if (-not $kitbase) { $kitbase = Join-Path $env:SystemDrive "Program Files (x86)" }
  $kitsRoot = Join-Path $kitbase "Windows Kits\10\bin"
  if (Test-Path $kitsRoot) {
    $signTool = Get-ChildItem $kitsRoot -Filter signtool.exe -Recurse -File -ErrorAction SilentlyContinue |
      Where-Object { $_.FullName -match '\\x64\\signtool\.exe$' } |
      Sort-Object FullName -Descending | Select-Object -First 1
  }
}
Record "Microsoft SignTool installed" ([bool]$signTool) $(if($signTool){"Found"}else{"Install Windows SDK Signing Tools"})
$certs = @(Get-ChildItem Cert:\CurrentUser\My -ErrorAction SilentlyContinue | Where-Object {
  $_.HasPrivateKey -and $_.NotAfter -gt [datetime]::UtcNow -and
  @($_.EnhancedKeyUsageList | ForEach-Object { [string]$_.ObjectId }) -contains "1.3.6.1.5.5.7.3.3"
})
Record "Development code-signing identity" ($certs.Count -ge 1) "Usable private-key code-signing cert count=$($certs.Count)"
$issuedCerts = @($certs | Where-Object { $_.Subject -ne $_.Issuer })
Record "Production signing certificate candidate" ($issuedCerts.Count -ge 1) "Non-self-signed candidates=$($issuedCerts.Count); independent clean-machine trust verification still mandatory"
$sha = ""
try { $sha = (& git -C $repo rev-parse HEAD 2>$null | Out-String).Trim() } catch {}
$dirty = @(& git -C $repo status --porcelain=v1 --untracked-files=all 2>$null)
Record "Git source committed" ($dirty.Count -eq 0) "HEAD=$sha; DirtyEntries=$($dirty.Count)"
Record "Windows restart required" (-not $needsRestart) "No automatic reboot performed; RestartNeeded=$needsRestart"
$report = [ordered]@{
  schema = "https://stackdeck.dev/schemas/host-release-preflight-v1.json"
  created_utc = [datetime]::UtcNow.ToString("o")
  host = $env:COMPUTERNAME
  user = $identity.Name
  commit = $sha
  administrator = $admin
  hyperv_restart_required = $needsRestart
  existing_vm_names = $knownNames
  all_passed = ($items.ToArray() | Where-Object { $_.status -eq "BLOCKED" } | Measure-Object).Count -eq 0
  checks = $items.ToArray()
}
$destination = [IO.Path]::GetFullPath($Output)
[IO.Directory]::CreateDirectory((Split-Path -Parent $destination)) | Out-Null
[IO.File]::WriteAllText($destination, ($report | ConvertTo-Json -Depth 8), [Text.UTF8Encoding]::new($false))
$items | Format-Table -AutoSize
Write-Host "Host preflight evidence: $destination"
if (-not $report.all_passed) { Write-Warning "Host is not yet production-qualified; inspect BLOCKED checks in report." }
if ($needsRestart) { Write-Warning "Feature enablement requires a manually approved Windows restart before VM tests." }
