#Requires -Version 5.1

[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)]
  [string]$OutputZip,
  [switch]$IncludeUntracked
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..\..')).ProviderPath
if (-not $IncludeUntracked) {
  $git = Get-Command git.exe -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
  if (-not $git) { throw 'Git is required for clean source handover provenance.' }
  $dirty = @(& $git.Path -C $repoRoot status --porcelain=v1 --untracked-files=all)
  if ($LASTEXITCODE -ne 0 -or $dirty.Count -gt 0) {
    throw 'Source handover requires a clean committed checkout. -IncludeUntracked is diagnostic-only and cannot qualify a release.'
  }
}
& (Join-Path $PSScriptRoot '13-Verify-Source-Release.ps1')
if (-not $?) { throw 'Source release contract failed.' }

function Resolve-GitSourceManifest {
  $git = Get-Command git.exe -CommandType Application -ErrorAction SilentlyContinue |
    Select-Object -First 1
  if (-not $git) { throw 'git.exe is required to build a manifest-driven source handover.' }
  $top = (& $git.Path -C $repoRoot rev-parse --show-toplevel 2>$null | Out-String).Trim()
  if ($LASTEXITCODE -ne 0 -or [IO.Path]::GetFullPath($top).TrimEnd('\') -ine $repoRoot.TrimEnd('\')) {
    throw 'Source handover must be run from a Git checkout rooted at the repository.'
  }
  $args = if ($IncludeUntracked) {
    @('-C', $repoRoot, 'ls-files', '-z', '--cached', '--modified', '--others', '--exclude-standard')
  } else {
    @('-C', $repoRoot, 'ls-files', '-z')
  }
  $raw = & $git.Path @args
  if ($LASTEXITCODE -ne 0) { throw 'git ls-files failed while building source manifest.' }
  $files = @($raw -split "`0" | Where-Object { -not [string]::IsNullOrWhiteSpace($_) } | Sort-Object -Unique)
  if ($files.Count -lt 1) { throw 'Git source manifest is empty.' }
  return $files
}

# Windows PowerShell 5.1 does not reliably load ZipArchive with FileSystem alone.
Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem
# Resolve against the caller's PowerShell location, not the MCP host process CWD.
$outputPath = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($OutputZip)
if (Test-Path -LiteralPath $outputPath) { Remove-Item -LiteralPath $outputPath -Force }
[IO.Directory]::CreateDirectory((Split-Path -Parent $outputPath)) | Out-Null

$relativeFiles = Resolve-GitSourceManifest
foreach ($mustHave in @(
    '.github/workflows/release-windows.yml',
    '.gitignore',
    'LICENSE',
    'runtime/windows-runtime-lock.json',
    'scripts/windows/01-Build-Release.ps1',
    'scripts/windows/09-Build-Signed-Release.ps1',
    'scripts/windows/13-Verify-Source-Release.ps1',
    'scripts/windows/14-Package-Source-Handover.ps1',
    'scripts/windows/04-Smoke-Test.ps1',
    'scripts/windows/08-Initialize-Installed-Runtime.ps1',
    'LOCAL_GO_README.md',
    'examples/smoke/docker-compose.yml',
    'scripts/windows/17-Elevated-Release-Preflight.ps1',
    'docs/WINDOWS_RELEASE_QUALIFICATION_2026-10-13.md'
  )) {
  if ($relativeFiles -notcontains $mustHave) {
    throw "Git source manifest does not contain required release path: $mustHave"
  }
}

$files = foreach ($relative in $relativeFiles) {
  if ($relative -match '^(target|desktop/stackdeck/(node_modules|dist))/' -or
      $relative -match '^desktop/stackdeck/src-tauri/bin/.*\.exe$' -or
      $relative -match '(^|/)\.stackdeck/.*\.(db|db-shm|db-wal)$') {
    throw "Git source manifest contains generated release artifact: $relative"
  }
  $path = Join-Path $repoRoot $relative.Replace('/', '\')
  if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
    throw "Git source manifest entry is missing from disk: $relative"
  }
  Get-Item -LiteralPath $path
}

$manifestPath = "$outputPath.manifest.sha256"
$manifestLines = foreach ($file in $files) {
  $relative = $file.FullName.Substring($repoRoot.Length).TrimStart('\').Replace('\','/')
  $hash = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
  "$hash  $relative"
}
[IO.File]::WriteAllLines($manifestPath, @($manifestLines), [Text.UTF8Encoding]::new($false))

$stream = [IO.File]::Open($outputPath, [IO.FileMode]::CreateNew)
try {
  $zip = [IO.Compression.ZipArchive]::new($stream, [IO.Compression.ZipArchiveMode]::Create, $false)
  try {
    foreach ($file in $files) {
      $relative = $file.FullName.Substring($repoRoot.Length).TrimStart('\').Replace('\','/')
      [IO.Compression.ZipFileExtensions]::CreateEntryFromFile($zip, $file.FullName, $relative, [IO.Compression.CompressionLevel]::Optimal) | Out-Null
    }
  } finally { $zip.Dispose() }
} finally { $stream.Dispose() }

# Re-open the emitted archive and hash every entry against its exact source
# file, not merely the three well-known hidden release paths.
$expectedHashes = @{}
foreach ($file in $files) {
  $relative = $file.FullName.Substring($repoRoot.Length).TrimStart('\').Replace('\','/')
  $expectedHashes[$relative] = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
}
$archive = [IO.Compression.ZipFile]::OpenRead($outputPath)
try {
  $names = @($archive.Entries | ForEach-Object FullName)
  if ($names.Count -ne $expectedHashes.Count -or @($names | Select-Object -Unique).Count -ne $names.Count) {
    throw 'Packaged source entry count differs from the Git manifest (missing or duplicate files).'
  }
  foreach ($mustHave in @('.github/workflows/release-windows.yml', '.gitignore', 'LICENSE', 'runtime/windows-runtime-lock.json')) {
    if ($names -notcontains $mustHave) { throw "Packaged source archive dropped required path: $mustHave" }
  }
  foreach ($entry in $archive.Entries) {
    if (-not $expectedHashes.ContainsKey($entry.FullName)) { throw "Unexpected source archive entry: $($entry.FullName)" }
    $reader = $entry.Open()
    $sha = [Security.Cryptography.SHA256]::Create()
    try { $actual = [BitConverter]::ToString($sha.ComputeHash($reader)).Replace('-','').ToLowerInvariant() }
    finally { $reader.Dispose(); $sha.Dispose() }
    if ($actual -ne $expectedHashes[$entry.FullName]) { throw "Source archive payload hash mismatch: $($entry.FullName)" }
  }
} finally { $archive.Dispose() }

$hash = (Get-FileHash -LiteralPath $outputPath -Algorithm SHA256).Hash.ToLowerInvariant()
[IO.File]::WriteAllText("$outputPath.sha256", "$hash  $([IO.Path]::GetFileName($outputPath))`n", [Text.UTF8Encoding]::new($false))
Write-Host "Source handover archive: $outputPath" -ForegroundColor Green
Write-Host "Source manifest: $manifestPath"
Write-Host "SHA-256: $hash"
