#Requires -Version 5.1

[CmdletBinding()]
param(
  [switch]$AllowUnlockedRuntimeLock
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..\..')).ProviderPath

$required = @(
  '.github\workflows\release-windows.yml',
  '.gitignore',
  'LICENSE',
  'runtime\windows-runtime-lock.json',
  'desktop\stackdeck\src-tauri\tauri.conf.json',
  'scripts\windows\09-Build-Signed-Release.ps1',
  'scripts\windows\10-Verify-Installer-Payloads.ps1',
  'scripts\windows\11-Verify-Installed-Client.ps1',
  'scripts\windows\15-Build-Local-Go-Package.ps1',
  'scripts\windows\16-Ensure-Prebuilt-Runtime-Prereqs.ps1'
)
foreach ($relative in $required) {
  $path = Join-Path $repoRoot $relative
  if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
    throw "Required release source file is missing: $relative"
  }
}

$tauri = Get-Content -LiteralPath (Join-Path $repoRoot 'desktop\stackdeck\src-tauri\tauri.conf.json') -Raw | ConvertFrom-Json
if ([string]$tauri.bundle.windows.nsis.installMode -ne 'perMachine') {
  throw 'NSIS installMode must be perMachine so installed-client tooling and Program Files layout agree.'
}
# Explicit Tauri target is mandatory: CARGO_BUILD_TARGET does not guarantee
# where `tauri bundle` writes installers when called as a separate command.
$npmPackage = Get-Content -LiteralPath (Join-Path $repoRoot 'desktop\stackdeck\package.json') -Raw | ConvertFrom-Json
foreach ($scriptName in @('build:hive','build:desktop-release','bundle:windows')) {
  if ([string]$npmPackage.scripts.$scriptName -notmatch '--target\s+x86_64-pc-windows-msvc(?:\s|$)') {
    throw "Tauri script $scriptName must explicitly select the canonical Windows MSVC target."
  }
}

$lockPath = Join-Path $repoRoot 'runtime\windows-runtime-lock.json'
$lock = Get-Content -LiteralPath $lockPath -Raw | ConvertFrom-Json
if ([string]$lock.schema -ne 'https://stackdeck.dev/schemas/windows-runtime-lock-v1.json') {
  throw 'Unsupported Windows runtime lock schema.'
}
$isUnlocked = ([string]$lock.image_url -eq 'UNLOCKED' -or
  [string]$lock.image_sha512 -eq 'UNLOCKED' -or
  [string]$lock.debian_snapshot_timestamp -eq 'UNLOCKED' -or
  [string]$lock.nerdctl_version -eq 'UNLOCKED' -or
  [string]$lock.nerdctl_sha256_amd64 -eq 'UNLOCKED')
if ($isUnlocked -and $AllowUnlockedRuntimeLock) {
  Write-Warning 'Runtime lock is UNLOCKED. Release builds remain blocked until 12-Lock-Runtime-Image.ps1 writes qualified values.'
} else {
  $imageUri = $null
  if (-not [Uri]::TryCreate([string]$lock.image_url, [UriKind]::Absolute, [ref]$imageUri) -or $imageUri.Scheme -ne 'https') {
    throw 'Runtime lock image_url must be absolute HTTPS.'
  }
  if ($imageUri.AbsoluteUri -match '/latest/' -or [string]$lock.image_url -eq 'UNLOCKED') {
    throw 'Runtime lock is mutable/unlocked. Run 12-Lock-Runtime-Image.ps1 before release build.'
  }
  if ([string]$lock.image_sha512 -notmatch '^[A-Fa-f0-9]{128}$') { throw 'Runtime image SHA-512 lock is invalid.' }
  if ([string]$lock.debian_snapshot_timestamp -notmatch '^\d{8}T\d{6}Z$') { throw 'Debian snapshot timestamp lock is invalid.' }
  if ([string]$lock.nerdctl_version -notmatch '^\d+(?:\.\d+){1,3}$') { throw 'nerdctl version lock is invalid.' }
  if ([string]$lock.nerdctl_sha256_amd64 -notmatch '^[A-Fa-f0-9]{64}$') { throw 'nerdctl SHA-256 lock is invalid.' }
}

$cliText = Get-Content -LiteralPath (Join-Path $repoRoot 'crates\stackdeck-cli\src\main.rs') -Raw
if ($cliText -match '(?m)^\s*Gui\s*\{' -or $cliText -match 'stackdeck_gui::run_server') {
  throw 'Legacy HTTP GUI command is still reachable from the production CLI/hive.'
}
$workspaceText = Get-Content -LiteralPath (Join-Path $repoRoot 'Cargo.toml') -Raw
$cliCargoText = Get-Content -LiteralPath (Join-Path $repoRoot 'crates\stackdeck-cli\Cargo.toml') -Raw
if ($workspaceText -match 'stackdeck-gui' -or $cliCargoText -match 'stackdeck-gui') {
  throw 'Legacy HTTP GUI crate is still wired into the production workspace or CLI dependency graph.'
}
foreach ($retired in @('crates\stackdeck-gui', 'crates\stackdeck-desktop')) {
  if (Test-Path -LiteralPath (Join-Path $repoRoot $retired)) {
    throw "Retired disconnected UI source is still present in the release source: $retired"
  }
}

$typesText = Get-Content -LiteralPath (Join-Path $repoRoot 'crates\stackdeck-types\src\lib.rs') -Raw
if ($typesText -notmatch 'STACKDECK_CONFIG_DIR' -or $typesText -notmatch 'ProgramData' -or $typesText -notmatch 'StackDeck.+config') {
  throw 'Shared config root must default to ProgramData\StackDeck\config with STACKDECK_CONFIG_DIR override.'
}
$hypervText = Get-Content -LiteralPath (Join-Path $repoRoot 'crates\stackdeck-hyperv\src\lib.rs') -Raw
if ($hypervText -notmatch 'DataProtectionScope\]::LocalMachine') {
  throw 'SMB DPAPI credential storage must use LocalMachine scope for the machine config root.'
}

$forbidden = @(
  'desktop\stackdeck\src-tauri\bin\*.exe',
  '**\.stackdeck\state.db'
)
# Build output is allowed in the ignored local working tree, but never in
# tracked release source or an unpacked handover ZIP.
$git = Get-Command git.exe -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
$hasGitCheckout = $false
if ($git) {
  $top = (& $git.Path -C $repoRoot rev-parse --show-toplevel 2>$null | Out-String).Trim()
  $hasGitCheckout = ($LASTEXITCODE -eq 0 -and $top -and
    [IO.Path]::GetFullPath($top).TrimEnd('\') -ieq $repoRoot.TrimEnd('\'))
}
if ($hasGitCheckout) {
  $tracked = @(& $git.Path -C $repoRoot ls-files)
  if ($LASTEXITCODE -ne 0) { throw 'Could not inspect tracked release files.' }
  $generatedTracked = @($tracked | Where-Object {
    $_ -match '^desktop/stackdeck/src-tauri/bin/[^/]+\.exe$' -or
    $_ -match '(^|/)\.stackdeck/state\.db$'
  })
  if ($generatedTracked.Count -gt 0) { throw "Generated artifacts are tracked in Git: $($generatedTracked -join ', ')" }
} else {
  $embeddedExe = @(Get-ChildItem -LiteralPath (Join-Path $repoRoot 'desktop\stackdeck\src-tauri\bin') -Filter '*.exe' -File -ErrorAction SilentlyContinue)
  if ($embeddedExe.Count -gt 0) { throw 'Generated sidecars are present in unpacked source.' }
  $stateDb = @(Get-ChildItem -LiteralPath $repoRoot -Filter 'state.db' -File -Recurse -ErrorAction SilentlyContinue |
    Where-Object { $_.FullName -match '[\\/]\.stackdeck[\\/]' })
  if ($stateDb.Count -gt 0) { throw 'Generated .stackdeck/state.db is present in unpacked source.' }
}

Write-Host 'Source release contract passed.' -ForegroundColor Green
