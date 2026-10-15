#Requires -Version 5.1
<#
.SYNOPSIS
  Read-only release-host intake for a dedicated Windows 11 Hyper-V acceptance host.
.DESCRIPTION
  Never provisions, stops, resets, repairs, removes, or starts a VM.
  Use -RequireDisposableHost for the isolated runtime acceptance workstation.
  Writes machine-readable evidence even when a check fails.
#>
[CmdletBinding()]
param(
  [string]$Output = "",
  [switch]$RequireDisposableHost
)
Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if (-not $Output) {
  $Output = Join-Path (Get-Location).Path "target\release-evidence\host-qualification-intake.json"
}
$checks = [Collections.Generic.List[object]]::new()
function Add-Check([string]$Name,[bool]$Passed,[string]$Detail) {
  $script:checks.Add([pscustomobject]@{
    name=$Name; status=$(if($Passed){"PASS"}else{"BLOCKED"}); detail=$Detail
  })
}
$principal = [Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent())
$admin = $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
Add-Check "Elevated administrator" $admin "Administrator token=$admin"
$os = Get-CimInstance Win32_OperatingSystem -ErrorAction Stop
$baseline = ([string]$os.Caption -match 'Windows 11') -and
            ([string]$os.Caption -match '(Pro|Enterprise|Education)') -and
            ([string]$os.OSArchitecture -match '64')
Add-Check "Windows 11 x64 Hyper-V-capable edition" $baseline "$($os.Caption), $($os.OSArchitecture)"
$featureEnabled = $false
if ($admin) {
  try {
    $feature = Get-WindowsOptionalFeature -Online -FeatureName Microsoft-Hyper-V-All -ErrorAction Stop
    $featureEnabled = ($feature.State -eq "Enabled")
  } catch { Add-Check "Hyper-V feature interrogation" $false $_.Exception.Message }
}
Add-Check "Hyper-V Windows feature enabled" $featureEnabled "Enabled=$featureEnabled"
$services = @('vmms','vmcompute','hns')
foreach ($svc in $services) {
  $service=Get-Service -Name $svc -ErrorAction SilentlyContinue
  Add-Check "Windows service $svc" ([bool]($service -and $service.Status -eq 'Running')) $(if($service){[string]$service.Status}else{"Unavailable"})
}
$switchOk = $false
try { $switchOk = [bool](Get-VMSwitch -Name 'Default Switch' -ErrorAction Stop) }
catch { Add-Check "Hyper-V switch lookup" $false $_.Exception.Message }
Add-Check "Default Switch" $switchOk "Available=$switchOk"
$vmNames=@()
$vmEnumerationSucceeded=$false
try {
  $vmNames=@(Get-VM -ErrorAction Stop | ForEach-Object Name)
  $vmEnumerationSucceeded=$true
} catch {
  Add-Check "Get-VM enumeration exception" $false $_.Exception.Message
}
Add-Check "Hyper-V VM settings enumeration" $vmEnumerationSucceeded "Enumerated=$vmEnumerationSucceeded; VM names=$($vmNames -join ', ')"
$cimNames=@()
$cimOk=$false
try {
  $cimNames=@(Get-CimInstance -Namespace root\virtualization\v2 -ClassName Msvm_ComputerSystem -ErrorAction Stop |
    Where-Object { $_.Caption -eq "Virtual Machine" } | ForEach-Object ElementName)
  $cimOk=$true
} catch { Add-Check "Hyper-V independent CIM inventory" $false $_.Exception.Message }
Add-Check "Hyper-V inventory agreement" ($vmEnumerationSucceeded -and $cimOk -and
  (Compare-Object -ReferenceObject @($vmNames|Sort-Object) -DifferenceObject @($cimNames|Sort-Object) -SyncWindow 100 | Measure-Object).Count -eq 0) "VMs=$($vmNames -join ', '); CIM=$($cimNames -join ', ')"
if ($RequireDisposableHost) {
  Add-Check "Disposable host contains no registered VMs" ($vmEnumerationSucceeded -and $cimOk -and $vmNames.Count -eq 0 -and $cimNames.Count -eq 0) "Existing VMs are never removed"
}
$qemuPath = Join-Path $env:ProgramFiles "qemu\qemu-img.exe"
$qemu = Get-Command qemu-img.exe -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $qemu -and (Test-Path -LiteralPath $qemuPath -PathType Leaf)) {$qemu=Get-Item $qemuPath}
Add-Check "qemu-img prerequisite" ([bool]$qemu) $(if($qemu){"Available"}else{"Not found"})
$ssh = Get-Command ssh.exe -ErrorAction SilentlyContinue | Select-Object -First 1
Add-Check "OpenSSH client" ([bool]$ssh) $(if($ssh){$ssh.Source}else{"Not found"})
$disk=Get-PSDrive -Name C -ErrorAction SilentlyContinue
Add-Check "System drive free >= 30 GiB" ([bool]($disk -and $disk.Free -ge 30GB)) $(if($disk){"Free GiB="+[math]::Round($disk.Free/1GB,2)}else{"Not available"})
$existingInstall=Test-Path (Join-Path $env:ProgramFiles "StackDeck")
if ($RequireDisposableHost) {
  Add-Check "No existing StackDeck installation" (-not $existingInstall) "A clean qualification host must not have user-installed StackDeck data"
}
$result=[ordered]@{
  schema="https://stackdeck.dev/schemas/host-qualification-intake-v1.json"
  created_utc=[DateTime]::UtcNow.ToString('o')
  machine=$env:COMPUTERNAME
  account=[Security.Principal.WindowsIdentity]::GetCurrent().Name
  read_only=$true
  require_disposable=[bool]$RequireDisposableHost
  passed=($checks.ToArray()|Where-Object status -eq 'BLOCKED'|Measure-Object).Count -eq 0
  checks=$checks.ToArray()
}
$destination=[IO.Path]::GetFullPath($Output)
[IO.Directory]::CreateDirectory((Split-Path -Parent $destination))|Out-Null
[IO.File]::WriteAllText($destination,($result|ConvertTo-Json -Depth 6),[Text.UTF8Encoding]::new($false))
$checks | Format-Table -AutoSize
Write-Output ("HOST_INTAKE_RESULT="+$(if($result.passed){"PASS"}else{"BLOCKED"}))
Write-Output ("HOST_INTAKE_EVIDENCE="+$destination)
if (-not $result.passed) {throw "Host qualification intake BLOCKED; no VMs or disks were modified."}
