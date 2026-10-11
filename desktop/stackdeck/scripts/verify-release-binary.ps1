$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$DesktopRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "..")).ProviderPath
$RepoRoot = (Resolve-Path -LiteralPath (Join-Path $DesktopRoot "..\..")).ProviderPath
$TargetTriple = "x86_64-pc-windows-msvc"
$Binary = Join-Path $RepoRoot "target\$TargetTriple\release\stackdeck.exe"
$Index = Join-Path $DesktopRoot "dist\index.html"
$Sidecar = Join-Path $DesktopRoot "src-tauri\bin\stackdeck-hive-x86_64-pc-windows-msvc.exe"
$PackageJson = Join-Path $DesktopRoot "package.json"

foreach ($required in @($Binary, $Index, $Sidecar, $PackageJson)) {
  if (-not (Test-Path -LiteralPath $required -PathType Leaf)) {
    throw "Release verification input is missing: $required"
  }
}

$indexText = Get-Content -LiteralPath $Index -Raw
$assetMatch = [regex]::Match($indexText, 'src="/assets/(?<asset>[^"/]+\.js)"')
if (-not $assetMatch.Success) {
  throw "Could not identify the production JavaScript asset in dist\index.html."
}
$assetName = $assetMatch.Groups["asset"].Value

# Tauri stores its asset table inside the executable. Reading the six-megabyte
# PE as text is deliberate here: a direct `cargo build` contains the dev URL
# but not the hashed Vite asset, which caused packaged releases to open
# 127.0.0.1. The Tauri release build must contain the current asset filename.
$binaryText = [Text.Encoding]::UTF8.GetString([IO.File]::ReadAllBytes($Binary))
if (-not $binaryText.Contains($assetName)) {
  throw "Release binary does not embed '$assetName'. Run 'tauri build --no-bundle --ci'; do not package a direct cargo build."
}
if (-not $binaryText.Contains("runtime_setup_status")) {
  throw "Release binary is missing the first-run runtime setup commands."
}

$package = Get-Content -LiteralPath $PackageJson -Raw | ConvertFrom-Json
$sidecarVersion = (& $Sidecar --version 2>&1 | Out-String).Trim()
if ($LASTEXITCODE -ne 0) {
  throw "The staged hive sidecar failed its version check."
}
if ($sidecarVersion -notmatch "\b$([regex]::Escape([string]$package.version))\b") {
  throw "Staged hive version '$sidecarVersion' does not match desktop version '$($package.version)'."
}

Write-Host "Release binary embeds frontend asset: $assetName"
Write-Host "Release binary includes first-run runtime setup commands."
Write-Host "Staged sidecar version matches desktop: $sidecarVersion"
