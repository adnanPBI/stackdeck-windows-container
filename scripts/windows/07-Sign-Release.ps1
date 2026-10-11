[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)]
  [ValidateSet("Sidecars", "DesktopPayload", "Installers", "Manifest")]
  [string]$Phase,

  [ValidatePattern("^[A-Fa-f0-9]{40}$")]
  [string]$CertificateThumbprint,

  [string]$TimestampUrl = "https://timestamp.digicert.com",

  [ValidatePattern("^[A-Fa-f0-9]{40}$")]
  [string]$SourceCommit = ""
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Resolve-SignTool {
  $command = Get-Command -Name "signtool.exe" -CommandType Application -ErrorAction SilentlyContinue |
    Select-Object -First 1
  if ($command) {
    return $command.Path
  }
  $kitsRoot = Join-Path ${env:ProgramFiles(x86)} "Windows Kits\10\bin"
  $candidate = Get-ChildItem -LiteralPath $kitsRoot -Filter "signtool.exe" -File -Recurse `
    -ErrorAction SilentlyContinue |
    Where-Object { $_.FullName -match "\\x64\\signtool\.exe$" } |
    Sort-Object -Property FullName -Descending |
    Select-Object -First 1
  if (-not $candidate) {
    throw "signtool.exe was not found. Install the Windows 10/11 SDK signing tools."
  }
  return $candidate.FullName
}

function Invoke-SignTool {
  param(
    [Parameter(Mandatory = $true)][string]$SignTool,
    [Parameter(Mandatory = $true)][string[]]$Arguments
  )
  & $SignTool @Arguments
  if ($LASTEXITCODE -ne 0) {
    throw "signtool.exe failed with exit code $LASTEXITCODE."
  }
}

function Sign-And-Verify {
  param(
    [Parameter(Mandatory = $true)][string]$SignTool,
    [Parameter(Mandatory = $true)][IO.FileInfo[]]$Files
  )
  if ($Files.Count -eq 0) {
    throw "No release artifacts matched the requested signing phase."
  }
  foreach ($file in $Files) {
    Invoke-SignTool -SignTool $SignTool -Arguments @(
      "sign", "/sha1", $CertificateThumbprint, "/fd", "SHA256",
      "/tr", $TimestampUrl, "/td", "SHA256", "/v", $file.FullName
    )
    Invoke-SignTool -SignTool $SignTool -Arguments @(
      "verify", "/pa", "/all", "/v", $file.FullName
    )
  }
}

function Require-Files {
  param(
    [Parameter(Mandatory = $true)][string]$Description,
    [Parameter(Mandatory = $true)][string[]]$Paths
  )
  $resolved = foreach ($path in $Paths) {
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
      throw "$Description is missing: $path"
    }
    Get-Item -LiteralPath $path
  }
  return @($resolved)
}

function Get-InstallerFiles {
  param([Parameter(Mandatory = $true)][string]$BundleRoot)

  $msi = @(Get-ChildItem -LiteralPath (Join-Path $BundleRoot "msi") -Filter "*.msi" `
      -File -ErrorAction SilentlyContinue)
  $nsis = @(Get-ChildItem -LiteralPath (Join-Path $BundleRoot "nsis") -Filter "*-setup.exe" `
      -File -ErrorAction SilentlyContinue)
  if ($msi.Count -ne 1 -or $nsis.Count -ne 1) {
    throw "Expected exactly one MSI and one NSIS setup executable under $BundleRoot; found MSI=$($msi.Count), NSIS=$($nsis.Count)."
  }
  return @($msi[0], $nsis[0])
}

function Verify-AuthenticodeFiles {
  param(
    [Parameter(Mandatory = $true)][string]$SignTool,
    [Parameter(Mandatory = $true)][IO.FileInfo[]]$Files
  )
  foreach ($file in $Files) {
    Invoke-SignTool -SignTool $SignTool -Arguments @(
      "verify", "/pa", "/all", "/v", $file.FullName
    )
    $signature = Get-AuthenticodeSignature -LiteralPath $file.FullName
    if ($signature.Status -ne "Valid") {
      throw "Authenticode status for $($file.FullName) is $($signature.Status), expected Valid."
    }
  }
}

$repoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "..\..")).ProviderPath
$signTool = Resolve-SignTool
$rustTarget = "x86_64-pc-windows-msvc"
$releaseRoot = Join-Path $repoRoot "target\release"
$desktopReleaseRoot = Join-Path $repoRoot "target\$rustTarget\release"
$bundleRoot = Join-Path $desktopReleaseRoot "bundle"
$stagedSidecar = Join-Path $repoRoot "desktop\stackdeck\src-tauri\bin\stackdeck-hive-x86_64-pc-windows-msvc.exe"
$stagedSidecarCompat = Join-Path $repoRoot "desktop\stackdeck\src-tauri\bin\stackdeck-hive.exe"
$sidecars = Require-Files -Description "Release CLI payload" -Paths @(
  (Join-Path $releaseRoot "stackdeck-cli.exe"),
  (Join-Path $releaseRoot "stackdeck-hive.exe")
)
$desktopPayload = @()
$installers = @()

if ($Phase -in @("DesktopPayload", "Manifest")) {
  $desktopPayload = Require-Files -Description "Desktop payload" -Paths @(
    (Join-Path $desktopReleaseRoot "stackdeck.exe"),
    $stagedSidecar,
    $stagedSidecarCompat
  )
}
if ($Phase -in @("Installers", "Manifest")) {
  $installers = Get-InstallerFiles -BundleRoot $bundleRoot
}

if ($Phase -ne "Manifest" -and [string]::IsNullOrWhiteSpace($CertificateThumbprint)) {
  throw "-CertificateThumbprint is required for signing phase '$Phase'."
}

$filesToSign = switch ($Phase) {
  "Sidecars" { $sidecars }
  "DesktopPayload" { $desktopPayload }
  "Installers" { $installers }
  default { @() }
}
if ($filesToSign.Count -gt 0) {
  Sign-And-Verify -SignTool $signTool -Files @($filesToSign | Sort-Object FullName -Unique)
}

$manifestFiles = if ($Phase -eq "Manifest") {
  @(@($sidecars + $desktopPayload + $installers) | Sort-Object FullName -Unique)
} else {
  @($filesToSign | Sort-Object FullName -Unique)
}
Verify-AuthenticodeFiles -SignTool $signTool -Files $manifestFiles

$manifestRoot = Join-Path $repoRoot "target\signed-release"
[IO.Directory]::CreateDirectory($manifestRoot) | Out-Null
$manifestPath = Join-Path $manifestRoot "SHA256SUMS.txt"
$artifactRecords = foreach ($file in $manifestFiles) {
  $hash = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
  Push-Location -LiteralPath $repoRoot
  try {
    $relative = (Resolve-Path -LiteralPath $file.FullName -Relative).Replace("\", "/").TrimStart([char[]]"./")
  } finally {
    Pop-Location
  }
  [pscustomobject]@{ path = $relative; sha256 = $hash; bytes = $file.Length }
}
$manifestLines = @($artifactRecords | ForEach-Object { "$($_.sha256)  $($_.path)" })
[IO.File]::WriteAllLines($manifestPath, $manifestLines, [Text.UTF8Encoding]::new($false))

if ($Phase -eq "Manifest") {
  $gitCommit = if ($SourceCommit) {
    $SourceCommit
  } elseif ($env:GITHUB_SHA) {
    $env:GITHUB_SHA
  } else {
    $git = Get-Command git.exe -ErrorAction SilentlyContinue | Select-Object -First 1
    if (-not $git) {
      throw "Unable to resolve source provenance. Supply -SourceCommit or run from a Git checkout."
    }
    $resolved = (& $git.Path -C $repoRoot rev-parse HEAD 2>$null | Out-String).Trim()
    if ($LASTEXITCODE -ne 0 -or $resolved -notmatch '^[A-Fa-f0-9]{40}$') {
      throw "Unable to resolve source provenance. Supply -SourceCommit or run from a Git checkout."
    }
    $resolved
  }
  $runtimeLockPath = Join-Path $repoRoot "runtime\windows-runtime-lock.json"
  if (-not (Test-Path -LiteralPath $runtimeLockPath -PathType Leaf)) { throw "Runtime lock is missing from release source." }
  $runtimeLock = Get-Content -LiteralPath $runtimeLockPath -Raw | ConvertFrom-Json
  $runtimeLockSha256 = (Get-FileHash -LiteralPath $runtimeLockPath -Algorithm SHA256).Hash.ToLowerInvariant()
  $provenance = [ordered]@{
    schema = "https://stackdeck.dev/schemas/windows-release-provenance-v1.json"
    generated_utc = [DateTime]::UtcNow.ToString("o")
    git_commit = $gitCommit
    git_ref = [string]$env:GITHUB_REF
    workflow = [string]$env:GITHUB_WORKFLOW
    workflow_run_id = [string]$env:GITHUB_RUN_ID
    workflow_run_attempt = [string]$env:GITHUB_RUN_ATTEMPT
    rust_toolchain = ((& rustc --version) | Out-String).Trim()
    node = ((& node --version) | Out-String).Trim()
    runtime_lock_sha256 = $runtimeLockSha256
    runtime_lock = $runtimeLock
    artifacts = @($artifactRecords)
  }
  if ($LASTEXITCODE -ne 0) { throw "Unable to record release tool versions." }
  $provenancePath = Join-Path $manifestRoot "provenance.json"
  [IO.File]::WriteAllText(
    $provenancePath,
    ($provenance | ConvertTo-Json -Depth 6),
    [Text.UTF8Encoding]::new($false)
  )
  Write-Host "Release provenance: $provenancePath"
}

Write-Host "Authenticode verification passed for $($manifestFiles.Count) artifact(s)."
Write-Host "SHA-256 manifest: $manifestPath"
