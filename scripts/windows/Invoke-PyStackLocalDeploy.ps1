[CmdletBinding()]
param(
  [Parameter(Mandatory=$true)][ValidateSet("Prereqs","Build","Configure","Init","Smoke","ApiTask","Diagnostics","All")][string]$Phase,
  [string]$PystackExe = "pystack",
  [string]$Sha256 = "",
  [string]$Sha512 = "",
  [string]$SmbPassword = "",
  [string]$VmName = "pystack-linux",
  [string]$VmRoot = "$env:SystemDrive\StackDeck\VMs\pystack-linux",
  [string]$ImageRoot = "$env:SystemDrive\StackDeck\Images",
  [UInt32]$DiskGB = 10,
  [UInt32]$MemoryMB = 4096,
  [UInt32]$Cpus = 2,
  [string]$SwitchName = "Default Switch",
  [switch]$EnableFeatures,
  [switch]$SkipDesktop
)
$ErrorActionPreference = "Stop"
$Root = $PSScriptRoot

function Stop-ApiTask {
  $task = Get-ScheduledTask -TaskName "PyStack Local Docker API" -ErrorAction SilentlyContinue
  if ($task) {
    Stop-ScheduledTask -TaskName "PyStack Local Docker API" -ErrorAction SilentlyContinue | Out-Null
  }
}

switch ($Phase) {
  "Prereqs" {
    if ($EnableFeatures) {
      & (Join-Path $Root "00-Ensure-Prereqs.ps1") -EnableFeatures -VmRoot $VmRoot -ImageRoot $ImageRoot
    } else {
      & (Join-Path $Root "00-Ensure-Prereqs.ps1") -VmRoot $VmRoot -ImageRoot $ImageRoot
    }
  }
  "Build" {
    Stop-ApiTask
    if ($SkipDesktop) {
      & (Join-Path $Root "01-Build-Release.ps1") -SkipDesktop
    } else {
      & (Join-Path $Root "01-Build-Release.ps1")
    }
  }
  "Configure" {
    & (Join-Path $Root "02-Configure-HyperV.ps1") -PystackExe $PystackExe -VmName $VmName -VmRoot $VmRoot -DiskGB $DiskGB -MemoryMB $MemoryMB -Cpus $Cpus -SwitchName $SwitchName -SmbPassword $SmbPassword
  }
  "Init" {
    $phaseArgs = @{ PystackExe = $PystackExe }
    if ($Sha256) { $phaseArgs["Sha256"] = $Sha256 }
    if ($Sha512) { $phaseArgs["Sha512"] = $Sha512 }
    & (Join-Path $Root "03-Init-HyperV-Runtime.ps1") @phaseArgs
  }
  "Smoke" {
    & (Join-Path $Root "04-Smoke-Test.ps1") -PystackExe $PystackExe
  }
  "ApiTask" {
    & (Join-Path $Root "05-Install-Api-Task.ps1") -PystackExe $PystackExe
  }
  "Diagnostics" {
    & (Join-Path $Root "06-Diagnostics.ps1") -PystackExe $PystackExe
  }
  "All" {
    Stop-ApiTask
    if ($EnableFeatures) {
      & (Join-Path $Root "00-Ensure-Prereqs.ps1") -EnableFeatures -VmRoot $VmRoot -ImageRoot $ImageRoot
    } else {
      & (Join-Path $Root "00-Ensure-Prereqs.ps1") -VmRoot $VmRoot -ImageRoot $ImageRoot
    }
    if ($SkipDesktop) {
      & (Join-Path $Root "01-Build-Release.ps1") -SkipDesktop
    } else {
      & (Join-Path $Root "01-Build-Release.ps1")
    }
    $BuiltPystack = Join-Path (Resolve-Path (Join-Path $Root "..\..")) "target\release\pystack.exe"
    & (Join-Path $Root "02-Configure-HyperV.ps1") -PystackExe $BuiltPystack -VmName $VmName -VmRoot $VmRoot -DiskGB $DiskGB -MemoryMB $MemoryMB -Cpus $Cpus -SwitchName $SwitchName -SmbPassword $SmbPassword
    $initArgs = @{ PystackExe = $BuiltPystack }
    if ($Sha256) { $initArgs["Sha256"] = $Sha256 }
    if ($Sha512) { $initArgs["Sha512"] = $Sha512 }
    & (Join-Path $Root "03-Init-HyperV-Runtime.ps1") @initArgs
    & (Join-Path $Root "04-Smoke-Test.ps1") -PystackExe $BuiltPystack
  }
}
