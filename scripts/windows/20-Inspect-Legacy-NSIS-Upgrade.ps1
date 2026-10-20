#Requires -Version 5.1
<# .SYNOPSIS
Read-only migration guard: historical per-user NSIS 0.5.0 -> machine-wide 0.6.0.
Never uninstall, delete, relocate or create VM resources.
#>
[CmdletBinding()]
param(
  [string]$Output = "",
  [switch]$FailIfLegacy
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$me = [Security.Principal.WindowsIdentity]::GetCurrent()
$legacyKeys = @(
 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\StackDeck',
 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\com.stackdeck.desktop'
)
$perUserRoot = Join-Path $env:LOCALAPPDATA 'StackDeck'
$legacyExe = Join-Path $perUserRoot 'uninstall.exe'
$rows = @()
foreach($key in $legacyKeys) {
 if(Test-Path -LiteralPath $key) {
  $entry=Get-ItemProperty -LiteralPath $key -ErrorAction Stop
  $rows+=@{
   key=$key
   display_name=[string]$entry.DisplayName
   display_version=[string]$entry.DisplayVersion
   uninstall_registration_present=$true
  }
 }
}
$oldUninstaller = Test-Path -LiteralPath $legacyExe -PathType Leaf
$legacyPresent = @($rows).Count -gt 0 -or $oldUninstaller
$machineKey = 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\{F50F68F7-10D0-4F00-842F-F975DAF11656}'
$machinePresent = Test-Path -LiteralPath $machineKey
$machineVersion = ''
if ($machinePresent) {
 $machineVersion = [string](Get-ItemProperty -LiteralPath $machineKey -ErrorAction Stop).DisplayVersion
}
$report=[ordered]@{
 utc=[datetime]::UtcNow.ToString('o')
 current_user=$me.Name
 historical_scope='currentUser/HKCU/LocalAppData'
 target_scope='perMachine/HKLM/Program Files'
 peruser_uninstaller_present=$oldUninstaller
 legacy_registrations=$rows
 legacy_present_for_current_user=[bool]$legacyPresent
 machine_wide_installer_registered=$machinePresent
 machine_version=$machineVersion
 other_user_hives_inspected=$false
 safe_for_same_account_nsis_install=[bool](!$legacyPresent)
 cross_account_upgrade_automatically_supported=$false
 vm_or_configuration_modified=$false
 verdict=$(if($legacyPresent){'REQUIRES_PRESERVATION_AND_LEGACY_UNINSTALL'}else{'NO_CURRENT_USER_LEGACY_CONFLICT'})
}
$serialized=$report|ConvertTo-Json -Depth 6
if($Output) {
 $path=$ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($Output)
 if(Test-Path -LiteralPath $path){throw 'Output already exists; refusing overwrite'}
 [IO.File]::WriteAllText($path,$serialized,[Text.UTF8Encoding]::new($false))
 Write-Output ('MIGRATION_EVIDENCE_FILE='+$path)
}
Write-Output ('MIGRATION_VERDICT='+$report.verdict)
if($FailIfLegacy -and $legacyPresent) { exit 3 }
