#Requires -Version 5.1

[CmdletBinding()]
param(
  [string]$OutputRoot = "",
  [switch]$SkipTests,
  [switch]$SkipInstallerPayloadVerification
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$RepoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "..\..")).ProviderPath
$RustTarget = "x86_64-pc-windows-msvc"
$Version = [string]((Get-Content -LiteralPath (Join-Path $RepoRoot "desktop\stackdeck\package.json") -Raw | ConvertFrom-Json).version)
if (-not $OutputRoot) {
  $OutputRoot = Join-Path $RepoRoot "target\local-go"
}
$OutputRoot = [IO.Path]::GetFullPath($OutputRoot)
$PackageName = "StackDeck-v$Version-Windows-x64-LOCAL-GO"
$PackageDir = Join-Path $OutputRoot $PackageName
$PackageZip = Join-Path $OutputRoot "$PackageName.zip"
$TargetRoot = Join-Path $RepoRoot "target\$RustTarget\release"
$BundleRoot = Join-Path $TargetRoot "bundle"
$CompatibilityRelease = Join-Path $RepoRoot "target\release"

function Get-Sha256([string]$Path) {
  return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Copy-RequiredFile([string]$Source, [string]$Destination) {
  if (-not (Test-Path -LiteralPath $Source -PathType Leaf)) {
    throw "Required local-GO artifact is missing: $Source"
  }
  $parent = Split-Path -Parent $Destination
  if ($parent) { [IO.Directory]::CreateDirectory($parent) | Out-Null }
  Copy-Item -LiteralPath $Source -Destination $Destination -Force
}

# A source_commit in release provenance is meaningful only for a clean checkout.
# Never package uncommitted local changes under a misleading committed SHA.
$git = Get-Command git.exe -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $git) { throw 'Git is required for release source provenance.' }
$gitCommit = (& $git.Path -C $RepoRoot rev-parse HEAD | Out-String).Trim()
if ($LASTEXITCODE -ne 0 -or $gitCommit -notmatch '^[a-fA-F0-9]{40}$') { throw 'Cannot resolve release source commit.' }
$dirtyPaths = @(& $git.Path -C $RepoRoot status --porcelain=v1 --untracked-files=all)
if ($LASTEXITCODE -ne 0 -or $dirtyPaths.Count -gt 0) {
  throw 'Local GO packaging requires a clean Git checkout. Commit intended source changes and exclude generated artifacts first.'
}

& (Join-Path $PSScriptRoot "13-Verify-Source-Release.ps1")
if (-not $?) { throw "Source release contract failed." }

$buildParams = @{ BundleUnsigned = $true }
if ($SkipTests) { $buildParams.SkipTests = $true }
& (Join-Path $PSScriptRoot "01-Build-Release.ps1") @buildParams
if (-not $?) { throw "Local unsigned desktop build failed." }

$desktop = Join-Path $TargetRoot "stackdeck.exe"
$cli = Join-Path $CompatibilityRelease "stackdeck-cli.exe"
$hiveCompat = Join-Path $CompatibilityRelease "stackdeck-hive.exe"
$hiveTarget = Join-Path $RepoRoot "desktop\stackdeck\src-tauri\bin\stackdeck-hive-x86_64-pc-windows-msvc.exe"
$hiveUnsuffixed = Join-Path $RepoRoot "desktop\stackdeck\src-tauri\bin\stackdeck-hive.exe"
foreach ($path in @($desktop, $cli, $hiveCompat, $hiveTarget, $hiveUnsuffixed)) {
  if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "Expected build artifact is missing: $path" }
}

$msi = @(Get-ChildItem -LiteralPath (Join-Path $BundleRoot "msi") -Filter "*.msi" -File -ErrorAction SilentlyContinue)
$nsis = @(Get-ChildItem -LiteralPath (Join-Path $BundleRoot "nsis") -Filter "*-setup.exe" -File -ErrorAction SilentlyContinue)
if ($msi.Count -ne 1 -or $nsis.Count -ne 1) {
  throw "Expected exactly one MSI and one NSIS installer after local bundling."
}

[IO.Directory]::CreateDirectory($OutputRoot) | Out-Null
$payloadManifest = [ordered]@{
  schema = "https://stackdeck.dev/schemas/windows-prebundle-payloads-v1.json"
  desktop_sha256 = Get-Sha256 $desktop
  hive_sha256 = @((Get-Sha256 $hiveTarget), (Get-Sha256 $hiveUnsuffixed)) | Sort-Object -Unique
}
$payloadManifestPath = Join-Path $OutputRoot "PRE_BUNDLE_PAYLOAD_SHA256.json"
[IO.File]::WriteAllText($payloadManifestPath, ($payloadManifest | ConvertTo-Json -Depth 4), [Text.UTF8Encoding]::new($false))

if (-not $SkipInstallerPayloadVerification) {
  & (Join-Path $PSScriptRoot "10-Verify-Installer-Payloads.ps1") `
    -ExpectedPayloadManifest $payloadManifestPath `
    -SkipSignature
  if (-not $?) { throw "Unsigned installer payload verification failed." }
}

if (Test-Path -LiteralPath $PackageDir) { Remove-Item -LiteralPath $PackageDir -Recurse -Force }
[IO.Directory]::CreateDirectory($PackageDir) | Out-Null
[IO.Directory]::CreateDirectory((Join-Path $PackageDir "installers")) | Out-Null
[IO.Directory]::CreateDirectory((Join-Path $PackageDir "cli")) | Out-Null
[IO.Directory]::CreateDirectory((Join-Path $PackageDir "runtime")) | Out-Null
[IO.Directory]::CreateDirectory((Join-Path $PackageDir "qualification")) | Out-Null

Copy-RequiredFile $msi[0].FullName (Join-Path $PackageDir "installers\$($msi[0].Name)")
Copy-RequiredFile $nsis[0].FullName (Join-Path $PackageDir "installers\$($nsis[0].Name)")
Copy-RequiredFile $cli (Join-Path $PackageDir "cli\stackdeck-cli.exe")
Copy-RequiredFile $hiveCompat (Join-Path $PackageDir "cli\stackdeck-hive.exe")
Copy-RequiredFile (Join-Path $RepoRoot "runtime\windows-runtime-lock.json") (Join-Path $PackageDir "runtime\windows-runtime-lock.json")
Copy-RequiredFile (Join-Path $PSScriptRoot "08-Initialize-Installed-Runtime.ps1") (Join-Path $PackageDir "qualification\08-Initialize-Installed-Runtime.ps1")
Copy-RequiredFile (Join-Path $PSScriptRoot "11-Verify-Installed-Client.ps1") (Join-Path $PackageDir "qualification\11-Verify-Installed-Client.ps1")
Copy-RequiredFile (Join-Path $PSScriptRoot "16-Ensure-Prebuilt-Runtime-Prereqs.ps1") (Join-Path $PackageDir "qualification\16-Ensure-Prebuilt-Runtime-Prereqs.ps1")
Copy-RequiredFile (Join-Path $RepoRoot "LOCAL_GO_README.md") (Join-Path $PackageDir "README.md")

$gitCommit = ""
$git = Get-Command git.exe -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
if ($git) {
  $candidate = (& $git.Path -C $RepoRoot rev-parse HEAD 2>$null | Out-String).Trim()
  if ($LASTEXITCODE -eq 0 -and $candidate -match '^[a-fA-F0-9]{40}$') { $gitCommit = $candidate.ToLowerInvariant() }
}

$packageFiles = @(Get-ChildItem -LiteralPath $PackageDir -File -Recurse | Sort-Object FullName)
$hashLines = foreach ($file in $packageFiles) {
  $rel = $file.FullName.Substring($PackageDir.Length).TrimStart('\').Replace('\','/')
  "$(Get-Sha256 $file.FullName)  $rel"
}
[IO.File]::WriteAllLines((Join-Path $PackageDir "SHA256SUMS.txt"), @($hashLines), [Text.UTF8Encoding]::new($false))

$provenance = [ordered]@{
  schema = "https://stackdeck.dev/schemas/windows-local-go-provenance-v1.json"
  version = $Version
  generated_utc = [DateTime]::UtcNow.ToString("o")
  source_commit = $gitCommit
  rust_target = $RustTarget
  signed = $false
  intended_use = "Local functional deployment and qualification only; not a publishable signed client release."
  source_contract_verified = $true
  installer_payload_hashes_verified = (-not $SkipInstallerPayloadVerification)
  runtime_lock = Get-Content -LiteralPath (Join-Path $RepoRoot "runtime\windows-runtime-lock.json") -Raw | ConvertFrom-Json
}
[IO.File]::WriteAllText((Join-Path $PackageDir "LOCAL_GO_PROVENANCE.json"), ($provenance | ConvertTo-Json -Depth 8), [Text.UTF8Encoding]::new($false))

# Recompute hashes after provenance is added.
$packageFiles = @(Get-ChildItem -LiteralPath $PackageDir -File -Recurse | Where-Object Name -ne 'SHA256SUMS.txt' | Sort-Object FullName)
$hashLines = foreach ($file in $packageFiles) {
  $rel = $file.FullName.Substring($PackageDir.Length).TrimStart('\').Replace('\','/')
  "$(Get-Sha256 $file.FullName)  $rel"
}
[IO.File]::WriteAllLines((Join-Path $PackageDir "SHA256SUMS.txt"), @($hashLines), [Text.UTF8Encoding]::new($false))

Add-Type -AssemblyName System.IO.Compression.FileSystem
if (Test-Path -LiteralPath $PackageZip) { Remove-Item -LiteralPath $PackageZip -Force }
[IO.Compression.ZipFile]::CreateFromDirectory($PackageDir, $PackageZip, [IO.Compression.CompressionLevel]::Optimal, $false)
$zipHash = Get-Sha256 $PackageZip
[IO.File]::WriteAllText("$PackageZip.sha256", "$zipHash  $([IO.Path]::GetFileName($PackageZip))`n", [Text.UTF8Encoding]::new($false))

Write-Host "LOCAL-GO package created." -ForegroundColor Green
Write-Host "Package: $PackageZip"
Write-Host "SHA-256: $zipHash"
Write-Warning "This package is intentionally unsigned. Run 11-Verify-Installed-Client.ps1 -SkipSignature after installing and initializing the runtime."
