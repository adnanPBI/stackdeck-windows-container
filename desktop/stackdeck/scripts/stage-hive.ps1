$ErrorActionPreference = "Stop"

$TargetTriple = "x86_64-pc-windows-msvc"
$DesktopRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "..")).ProviderPath
$RepoRoot = (Resolve-Path -LiteralPath (Join-Path $DesktopRoot "..\..")).ProviderPath
$BinDir = Join-Path $DesktopRoot "src-tauri\bin"
$SourceExe = Join-Path $RepoRoot "target\$TargetTriple\release\stackdeck-hive.exe"
$SuffixedExe = Join-Path $BinDir "stackdeck-hive-$TargetTriple.exe"
$UnsuffixedExe = Join-Path $BinDir "stackdeck-hive.exe"

if (!(Test-Path $SourceExe)) {
    throw "Missing sidecar source: $SourceExe. Run cargo build -p stackdeck-cli --bin stackdeck-hive --release --locked first."
}
if (!(Test-Path $BinDir)) {
    New-Item -ItemType Directory -Path $BinDir | Out-Null
}

Copy-Item -Path $SourceExe -Destination $SuffixedExe -Force
Copy-Item -Path $SourceExe -Destination $UnsuffixedExe -Force
Write-Host "Sidecar staged: $SuffixedExe and $UnsuffixedExe"
