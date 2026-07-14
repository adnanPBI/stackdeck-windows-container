[CmdletBinding()]
param(
  [Parameter(Mandatory=$true)][ValidateSet("Prereqs","Build","Configure","Init","Smoke","ApiTask","Diagnostics","All")][string]$Phase,
  [string]$PystackExe = "stackdeck",
  [string]$Sha256 = "",
  [string]$SmbPassword = "",
  [switch]$EnableFeatures,
  [switch]$SkipDesktop
)
$ErrorActionPreference = "Stop"
$Root = $PSScriptRoot
function Run($Script, $ArgsList) { & (Join-Path $Root $Script) @ArgsList }

switch ($Phase) {
  "Prereqs" { Run "00-Ensure-Prereqs.ps1" @($(if($EnableFeatures){"-EnableFeatures"})) }
  "Build" { Run "01-Build-Release.ps1" @($(if($SkipDesktop){"-SkipDesktop"})) }
  "Configure" { Run "02-Configure-HyperV.ps1" @("-PystackExe", $PystackExe, "-SmbPassword", $SmbPassword) }
  "Init" {
    if (-not $Sha256) { throw "-Sha256 is required for Init." }
    Run "03-Init-HyperV-Runtime.ps1" @("-PystackExe", $PystackExe, "-Sha256", $Sha256)
  }
  "Smoke" { Run "04-Smoke-Test.ps1" @("-PystackExe", $PystackExe) }
  "ApiTask" { Run "05-Install-Api-Task.ps1" @("-PystackExe", $PystackExe) }
  "Diagnostics" { Run "06-Diagnostics.ps1" @("-PystackExe", $PystackExe) }
  "All" {
    if (-not $Sha256) { throw "-Sha256 is required for All." }
    Run "00-Ensure-Prereqs.ps1" @($(if($EnableFeatures){"-EnableFeatures"}))
    Run "01-Build-Release.ps1" @($(if($SkipDesktop){"-SkipDesktop"}))
    $BuiltPystack = Join-Path (Resolve-Path (Join-Path $Root "..\..")) "target\release\stackdeck.exe"
    Run "02-Configure-HyperV.ps1" @("-PystackExe", $BuiltPystack, "-SmbPassword", $SmbPassword)
    Run "03-Init-HyperV-Runtime.ps1" @("-PystackExe", $BuiltPystack, "-Sha256", $Sha256)
    Run "04-Smoke-Test.ps1" @("-PystackExe", $BuiltPystack)
  }
}
