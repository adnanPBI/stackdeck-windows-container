[CmdletBinding()]
param(
  [switch]$SkipTests,
  [switch]$SkipDesktop
)
$ErrorActionPreference = "Stop"
$RepoRoot = Resolve-Path (Join-Path $PSScriptRoot "..\..")
Set-Location $RepoRoot

cargo fmt --check
if (-not $SkipTests) { cargo test --workspace --locked }
cargo build --workspace --locked
cargo build -p pystack-cli --bin pystack --release --locked
cargo build -p pystack-cli --bin hive --release --locked

if (-not $SkipDesktop) {
  Push-Location "desktop\stackdeck"
  npm ci
  npm run build
  npm run stage:hive
  npm run package:windows
  Pop-Location
}

Write-Host "Build complete." -ForegroundColor Green
Write-Host "CLI: $RepoRoot\target\release\pystack.exe"
Write-Host "Hive sidecar: $RepoRoot\target\release\hive.exe"
