[CmdletBinding()]
param(
  [switch]$SkipTests,
  [switch]$SkipDesktop,
  [switch]$BundleUnsigned
)

# Windows PowerShell does not turn native-process failures into terminating
# errors, so every Cargo/npm invocation must be checked explicitly.
Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Resolve-ApplicationPath {
  param([Parameter(Mandatory = $true)][string]$Name)

  $command = Get-Command -Name $Name -CommandType Application -ErrorAction SilentlyContinue |
    Select-Object -First 1
  if (-not $command) {
    throw "Required command was not found: $Name"
  }
  return $command.Path
}

function Invoke-NativeChecked {
  param(
    [Parameter(Mandatory = $true)][string]$FilePath,
    [string[]]$ArgumentList = @()
  )

  & $FilePath @ArgumentList
  $exitCode = $LASTEXITCODE
  if ($exitCode -ne 0) {
    throw "$([IO.Path]::GetFileName($FilePath)) failed with exit code $exitCode."
  }
}

$RepoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "..\..")).ProviderPath
$CargoPath = Resolve-ApplicationPath "cargo.exe"
$RustTarget = "x86_64-pc-windows-msvc"
$CargoTargetRoot = Join-Path $RepoRoot "target"
$TargetReleaseRoot = Join-Path $CargoTargetRoot "$RustTarget\release"
$CompatibilityReleaseRoot = Join-Path $CargoTargetRoot "release"
$StackDeckCliOutput = Join-Path $RepoRoot "target\release\stackdeck-cli.exe"
$StackDeckHiveOutput = Join-Path $RepoRoot "target\release\stackdeck-hive.exe"
$StackDeckDesktopOutput = Join-Path $TargetReleaseRoot "stackdeck.exe"
$previousCargoTargetDir = [Environment]::GetEnvironmentVariable("CARGO_TARGET_DIR", "Process")
$previousCargoBuildTarget = [Environment]::GetEnvironmentVariable("CARGO_BUILD_TARGET", "Process")

& (Join-Path $PSScriptRoot "13-Verify-Source-Release.ps1")
if (-not $?) { throw "Source release contract failed." }

Push-Location -LiteralPath $RepoRoot
try {
  # Force a deterministic target location/architecture even when the caller has
  # global Cargo environment or config overrides.
  $env:CARGO_TARGET_DIR = $CargoTargetRoot
  $env:CARGO_BUILD_TARGET = $RustTarget

  Invoke-NativeChecked -FilePath $CargoPath -ArgumentList @("fmt", "--check")

  if (-not $SkipTests) {
    Invoke-NativeChecked -FilePath $CargoPath -ArgumentList @(
      "test",
      "-p", "stackdeck-cli",
      "-p", "stackdeck-api",
      "-p", "stackdeck-hyperv",
      "-p", "stackdeck-compose",
      "-p", "stackdeck-types",
      "-p", "stackdeck-state",
      "-p", "stackdeck-process",
      "--target", $RustTarget,
      "--locked"
    )
  }

  Invoke-NativeChecked -FilePath $CargoPath -ArgumentList @(
    "build", "-p", "stackdeck-cli", "--bin", "stackdeck-cli", "--release",
    "--target", $RustTarget, "--locked"
  )
  Invoke-NativeChecked -FilePath $CargoPath -ArgumentList @(
    "build", "-p", "stackdeck-cli", "--bin", "stackdeck-hive", "--release",
    "--target", $RustTarget, "--locked"
  )

  [IO.Directory]::CreateDirectory($CompatibilityReleaseRoot) | Out-Null
  Copy-Item -LiteralPath (Join-Path $TargetReleaseRoot "stackdeck-cli.exe") -Destination $StackDeckCliOutput -Force
  Copy-Item -LiteralPath (Join-Path $TargetReleaseRoot "stackdeck-hive.exe") -Destination $StackDeckHiveOutput -Force

  if (-not (Test-Path -LiteralPath $StackDeckCliOutput -PathType Leaf)) {
    throw "Release build did not create $StackDeckCliOutput"
  }
  if (-not (Test-Path -LiteralPath $StackDeckHiveOutput -PathType Leaf)) {
    throw "Release build did not create $StackDeckHiveOutput"
  }

  if (-not $SkipDesktop) {
    $NpmPath = Resolve-ApplicationPath "npm.cmd"
    $DesktopRoot = Join-Path $RepoRoot "desktop\stackdeck"
    $BundleRoot = Join-Path $TargetReleaseRoot "bundle"
    foreach ($format in @("msi", "nsis")) {
      $oldBundle = Join-Path $BundleRoot $format
      if (Test-Path -LiteralPath $oldBundle -PathType Container) {
        Remove-Item -LiteralPath $oldBundle -Recurse -Force
      }
    }

    Push-Location -LiteralPath $DesktopRoot
    try {
      Invoke-NativeChecked -FilePath $NpmPath -ArgumentList @("ci")
      Invoke-NativeChecked -FilePath $NpmPath -ArgumentList @("run", "stage:hive")
      # Build the desktop executable without bundling. Public release installers
      # must not be created until the executable and staged sidecar are signed.
      Invoke-NativeChecked -FilePath $NpmPath -ArgumentList @("run", "build:desktop-release")
      Invoke-NativeChecked -FilePath $NpmPath -ArgumentList @("run", "verify:desktop-release")
      if ($BundleUnsigned) {
        Write-Warning "Creating UNSIGNED local MSI/NSIS packages. These are test artifacts and must never be published."
        Invoke-NativeChecked -FilePath $NpmPath -ArgumentList @("run", "bundle:windows")
      }
    } finally {
      Pop-Location
    }

    if (-not (Test-Path -LiteralPath $StackDeckDesktopOutput -PathType Leaf)) {
      throw "Desktop release build did not create $StackDeckDesktopOutput"
    }
    if ($BundleUnsigned) {
      $Msi = Get-ChildItem -LiteralPath (Join-Path $BundleRoot "msi") `
        -Filter "*.msi" -File -ErrorAction SilentlyContinue | Select-Object -First 1
      $Nsis = Get-ChildItem -LiteralPath (Join-Path $BundleRoot "nsis") `
        -Filter "*.exe" -File -ErrorAction SilentlyContinue | Select-Object -First 1
      if (-not $Msi -or -not $Nsis) {
        throw "Unsigned desktop bundling completed without producing both MSI and NSIS installers."
      }
    }
  }
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
  Pop-Location
}

Write-Host "Build complete." -ForegroundColor Green
Write-Host "CLI: $StackDeckCliOutput"
Write-Host "Hive sidecar: $StackDeckHiveOutput"
if (-not $SkipDesktop) {
  Write-Host "Desktop payload: $StackDeckDesktopOutput"
  if (-not $BundleUnsigned) {
    Write-Host "Installers were intentionally not bundled. Use 09-Build-Signed-Release.ps1 for a publishable signed release."
  }
}
