#Requires -Version 5.1

[CmdletBinding()]
param(
  [string]$CertificateThumbprint = $env:STACKDECK_SIGNING_THUMBPRINT,
  [string]$TimestampUrl = "https://timestamp.digicert.com",
  [ValidatePattern("^[A-Fa-f0-9]{40}$")]
  [string]$SourceCommit = "",
  [switch]$SkipTests,
  [switch]$SkipInstallerPayloadVerification
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if ([string]::IsNullOrWhiteSpace($CertificateThumbprint) -or
    $CertificateThumbprint -notmatch '^[A-Fa-f0-9]{40}$') {
  throw "A 40-character Authenticode certificate thumbprint is required. Pass -CertificateThumbprint or set STACKDECK_SIGNING_THUMBPRINT."
}

$RepoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "..\..")).ProviderPath
$git = Get-Command git.exe -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $git) { throw 'Signed releases require Git source provenance.' }
$resolvedCommit = (& $git.Path -C $RepoRoot rev-parse HEAD | Out-String).Trim()
if ($LASTEXITCODE -ne 0 -or $resolvedCommit -notmatch '^[a-fA-F0-9]{40}$') { throw 'Cannot resolve signed-release Git commit.' }
if ($SourceCommit -and $SourceCommit -ine $resolvedCommit) { throw 'Signed release source commit does not match checked-out HEAD.' }
$uncommittedPaths = @(& $git.Path -C $RepoRoot status --porcelain=v1 --untracked-files=all)
if ($LASTEXITCODE -ne 0 -or $uncommittedPaths.Count -gt 0) {
  throw 'Signed releases require a clean checkout. Commit all source and runtime-lock changes before signing.'
}
$DesktopRoot = Join-Path $RepoRoot "desktop\stackdeck"
$RustTarget = "x86_64-pc-windows-msvc"
$CargoTargetRoot = Join-Path $RepoRoot "target"
$previousCargoTargetDir = [Environment]::GetEnvironmentVariable("CARGO_TARGET_DIR", "Process")
$previousCargoBuildTarget = [Environment]::GetEnvironmentVariable("CARGO_BUILD_TARGET", "Process")

function Write-PreBundlePayloadManifest {
  $desktop = Join-Path $CargoTargetRoot "$RustTarget\release\stackdeck.exe"
  $hiveTarget = Join-Path $RepoRoot "desktop\stackdeck\src-tauri\bin\stackdeck-hive-x86_64-pc-windows-msvc.exe"
  $hiveCompat = Join-Path $RepoRoot "desktop\stackdeck\src-tauri\bin\stackdeck-hive.exe"
  foreach ($path in @($desktop, $hiveTarget, $hiveCompat)) {
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "Canonical signed payload missing before bundle: $path" }
  }
  $manifestRoot = Join-Path $CargoTargetRoot "signed-release"
  [IO.Directory]::CreateDirectory($manifestRoot) | Out-Null
  $hiveHashes = @(
    (Get-FileHash -LiteralPath $hiveTarget -Algorithm SHA256).Hash.ToLowerInvariant()
    (Get-FileHash -LiteralPath $hiveCompat -Algorithm SHA256).Hash.ToLowerInvariant()
  ) | Sort-Object -Unique
  $manifest = [ordered]@{
    schema = "https://stackdeck.dev/schemas/windows-prebundle-payloads-v1.json"
    desktop_sha256 = (Get-FileHash -LiteralPath $desktop -Algorithm SHA256).Hash.ToLowerInvariant()
    hive_sha256 = @($hiveHashes)
  }
  $path = Join-Path $manifestRoot "PRE_BUNDLE_PAYLOAD_SHA256.json"
  [IO.File]::WriteAllText($path, ($manifest | ConvertTo-Json -Depth 4), [Text.UTF8Encoding]::new($false))
  return $path
}

function Invoke-CheckedScript {
  param(
    [Parameter(Mandatory = $true)][string]$Path,
    [hashtable]$Parameters = @{}
  )
  & $Path @Parameters
  if (-not $?) { throw "Release step failed: $Path" }
}

try {
  # Keep Tauri build and bundle on exactly the same canonical output root.
  $env:CARGO_TARGET_DIR = $CargoTargetRoot
  $env:CARGO_BUILD_TARGET = $RustTarget

  $buildParams = @{}
  if ($SkipTests) { $buildParams["SkipTests"] = $true }
  Invoke-CheckedScript -Path (Join-Path $PSScriptRoot "01-Build-Release.ps1") -Parameters $buildParams

  $signScript = Join-Path $PSScriptRoot "07-Sign-Release.ps1"
  Invoke-CheckedScript -Path $signScript -Parameters @{
    Phase = "Sidecars"; CertificateThumbprint = $CertificateThumbprint; TimestampUrl = $TimestampUrl
  }
  Invoke-CheckedScript -Path $signScript -Parameters @{
    Phase = "DesktopPayload"; CertificateThumbprint = $CertificateThumbprint; TimestampUrl = $TimestampUrl
  }
  $preBundlePayloadManifest = Write-PreBundlePayloadManifest

  Push-Location -LiteralPath $DesktopRoot
  try {
    & npm.cmd run bundle:windows
    if ($LASTEXITCODE -ne 0) {
      throw "Tauri MSI/NSIS bundling failed with exit code $LASTEXITCODE."
    }
  } finally {
    Pop-Location
  }

  Invoke-CheckedScript -Path $signScript -Parameters @{
    Phase = "Installers"; CertificateThumbprint = $CertificateThumbprint; TimestampUrl = $TimestampUrl
  }

  if (-not $SkipInstallerPayloadVerification) {
    Invoke-CheckedScript -Path (Join-Path $PSScriptRoot "10-Verify-Installer-Payloads.ps1") -Parameters @{ ExpectedPayloadManifest = $preBundlePayloadManifest }
  }

  $manifestParams = @{ Phase = "Manifest"; TimestampUrl = $TimestampUrl }
  if ($SourceCommit) { $manifestParams["SourceCommit"] = $SourceCommit }
  Invoke-CheckedScript -Path $signScript -Parameters $manifestParams

  Write-Host "Signed Windows release completed successfully." -ForegroundColor Green
  Write-Host "Artifacts: $(Join-Path $CargoTargetRoot "$RustTarget\release\bundle")"
  Write-Host "Manifest/provenance: $(Join-Path $CargoTargetRoot "signed-release")"
} finally {
  if ($null -eq $previousCargoTargetDir) {
    Remove-Item Env:\CARGO_TARGET_DIR -ErrorAction SilentlyContinue
  } else {
    $env:CARGO_TARGET_DIR = $previousCargoTargetDir
  }
  if ($null -eq $previousCargoBuildTarget) {
    Remove-Item Env:\CARGO_BUILD_TARGET -ErrorAction SilentlyContinue
  } else {
    $env:CARGO_BUILD_TARGET = $previousCargoBuildTarget
  }
}
