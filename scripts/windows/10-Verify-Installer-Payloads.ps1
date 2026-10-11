#Requires -Version 5.1

[CmdletBinding()]
param(
  [string]$SevenZip = "",
  [string]$ExpectedPayloadManifest = "",
  [switch]$SkipSignature
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Resolve-ApplicationPath {
  param([Parameter(Mandatory = $true)][string[]]$Names)
  foreach ($name in $Names) {
    $cmd = Get-Command -Name $name -CommandType Application -ErrorAction SilentlyContinue |
      Select-Object -First 1
    if ($cmd) { return $cmd.Path }
  }
  return $null
}

function Invoke-NativeChecked {
  param([string]$FilePath, [string[]]$ArgumentList)
  & $FilePath @ArgumentList
  if ($LASTEXITCODE -ne 0) {
    throw "$([IO.Path]::GetFileName($FilePath)) failed with exit code $LASTEXITCODE."
  }
}

$RepoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "..\..")).ProviderPath
if (-not $ExpectedPayloadManifest) {
  $ExpectedPayloadManifest = Join-Path $RepoRoot "target\signed-release\PRE_BUNDLE_PAYLOAD_SHA256.json"
}
if (-not (Test-Path -LiteralPath $ExpectedPayloadManifest -PathType Leaf)) {
  throw "Expected pre-bundle payload manifest was not found: $ExpectedPayloadManifest"
}
$expected = Get-Content -LiteralPath $ExpectedPayloadManifest -Raw | ConvertFrom-Json
if ([string]$expected.schema -ne "https://stackdeck.dev/schemas/windows-prebundle-payloads-v1.json") {
  throw "Unexpected pre-bundle payload manifest schema."
}
$expectedDesktop = ([string]$expected.desktop_sha256).ToLowerInvariant()
$expectedHive = @($expected.hive_sha256 | ForEach-Object { ([string]$_).ToLowerInvariant() } | Sort-Object -Unique)
if ($expectedDesktop -notmatch "^[a-f0-9]{64}$" -or $expectedHive.Count -lt 1 -or @($expectedHive | Where-Object { $_ -notmatch "^[a-f0-9]{64}$" }).Count -gt 0) {
  throw "Pre-bundle payload manifest contains invalid hashes."
}
$RustTarget = "x86_64-pc-windows-msvc"
$BundleRoot = Join-Path $RepoRoot "target\$RustTarget\release\bundle"
$Msi = @(Get-ChildItem -LiteralPath (Join-Path $BundleRoot "msi") -Filter "*.msi" -File -ErrorAction SilentlyContinue)
$Nsis = @(Get-ChildItem -LiteralPath (Join-Path $BundleRoot "nsis") -Filter "*-setup.exe" -File -ErrorAction SilentlyContinue)
if ($Msi.Count -ne 1 -or $Nsis.Count -ne 1) {
  throw "Expected exactly one MSI and one NSIS setup executable under $BundleRoot."
}

if (-not $SevenZip) {
  $SevenZip = Resolve-ApplicationPath @("7z.exe", "7za.exe")
  if (-not $SevenZip) {
    # Some non-interactive Windows agents omit ProgramData/ChocolateyInstall.
    $commonAppData = [Environment]::GetFolderPath([Environment+SpecialFolder]::CommonApplicationData)
    foreach ($candidate in @(
      "$env:ProgramFiles\7-Zip\7z.exe",
      "${env:ProgramFiles(x86)}\7-Zip\7z.exe",
      "$commonAppData\chocolatey\tools\7z.exe",
      "$env:ChocolateyInstall\tools\7z.exe"
    )) {
      if ($candidate -and (Test-Path -LiteralPath $candidate -PathType Leaf)) {
        $SevenZip = $candidate
        break
      }
    }
  }
}
if (-not $SevenZip -or -not (Test-Path -LiteralPath $SevenZip -PathType Leaf)) {
  throw "7-Zip is required to extract and verify signed installer payloads."
}

$signTool = $null
if (-not $SkipSignature) {
  $signTool = Resolve-ApplicationPath @("signtool.exe")
  if (-not $signTool) {
    $kitsRoot = Join-Path ${env:ProgramFiles(x86)} "Windows Kits\10\bin"
    $candidate = Get-ChildItem -LiteralPath $kitsRoot -Filter "signtool.exe" -File -Recurse -ErrorAction SilentlyContinue |
      Where-Object { $_.FullName -match "\\x64\\signtool\.exe$" } |
      Sort-Object FullName -Descending | Select-Object -First 1
    if ($candidate) { $signTool = $candidate.FullName }
  }
  if (-not $signTool) { throw "signtool.exe was not found." }
} else {
  Write-Warning "Authenticode verification is skipped for local unsigned installer payload verification only."
}

$tempRoot = Join-Path ([IO.Path]::GetTempPath()) ("stackdeck-installer-verify-" + [Guid]::NewGuid().ToString("N"))
[IO.Directory]::CreateDirectory($tempRoot) | Out-Null
try {
  foreach ($installer in @($Msi[0], $Nsis[0])) {
    $dest = Join-Path $tempRoot ([IO.Path]::GetFileNameWithoutExtension($installer.Name))
    [IO.Directory]::CreateDirectory($dest) | Out-Null
    Invoke-NativeChecked -FilePath $SevenZip -ArgumentList @("x", "-y", "-o$dest", $installer.FullName)

    if ($installer.Extension -ieq ".msi") {
      # Windows Installer stores its app executables in an embedded CAB.
      # WiX may rename them to opaque File-table keys (e.g. Path, Bin_*).
      # Identify them by exact trusted pre-bundle SHA-256, never by CAB names.
      $cabs = @(Get-ChildItem -LiteralPath $dest -Filter "*.cab" -File -Recurse -ErrorAction SilentlyContinue)
      if ($cabs.Count -ne 1) { throw "Expected exactly one application CAB in $($installer.Name); found $($cabs.Count)." }
      $cabDest = Join-Path $dest "__stackdeck_msi_payload"
      [IO.Directory]::CreateDirectory($cabDest) | Out-Null
      Invoke-NativeChecked -FilePath $SevenZip -ArgumentList @("x", "-y", "-o$cabDest", $cabs[0].FullName)
      $cabFiles = @(Get-ChildItem -LiteralPath $cabDest -File -Recurse -ErrorAction SilentlyContinue)
      if ($cabFiles.Count -ne 2) { throw "Expected exactly two application payload files in MSI CAB; found $($cabFiles.Count)." }
      $hashed = @($cabFiles | ForEach-Object {
        [pscustomobject]@{ File = $_; Sha256 = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant() }
      })
      $desktopMatch = @($hashed | Where-Object { $_.Sha256 -eq $expectedDesktop })
      $hiveMatch = @($hashed | Where-Object { $expectedHive -contains $_.Sha256 })
      if ($desktopMatch.Count -ne 1 -or $hiveMatch.Count -ne 1) {
        throw "MSI CAB payload hash mismatch for $($installer.Name); the embedded desktop or hive is not the exact signed build."
      }
      # Give the extracted signed PE payloads canonical names for signtool and
      # Authenticode, while retaining hash identity with the original CAB bytes.
      $desktopPath = Join-Path $cabDest "stackdeck.exe"
      $hivePath = Join-Path $cabDest "stackdeck-hive.exe"
      Copy-Item -LiteralPath $desktopMatch[0].File.FullName -Destination $desktopPath -Force
      Copy-Item -LiteralPath $hiveMatch[0].File.FullName -Destination $hivePath -Force
      $desktop = @(Get-Item -LiteralPath $desktopPath)
      $hive = @(Get-Item -LiteralPath $hivePath)
    } else {
      $desktop = @(Get-ChildItem -LiteralPath $dest -Filter "stackdeck.exe" -File -Recurse -ErrorAction SilentlyContinue)
      $hive = @(Get-ChildItem -LiteralPath $dest -Filter "stackdeck-hive*.exe" -File -Recurse -ErrorAction SilentlyContinue)
    }
    if ($desktop.Count -ne 1 -or $hive.Count -ne 1) {
      throw "Expected exactly one StackDeck desktop and one hive payload in $($installer.Name)."
    }
    foreach ($payload in $desktop) {
      $actual = (Get-FileHash -LiteralPath $payload.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
      if ($actual -ne $expectedDesktop) {
        throw "Embedded desktop payload hash mismatch in $($installer.Name): $($payload.FullName)"
      }
    }
    foreach ($payload in $hive) {
      $actual = (Get-FileHash -LiteralPath $payload.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
      if ($expectedHive -notcontains $actual) {
        throw "Embedded hive payload hash mismatch in $($installer.Name): $($payload.FullName)"
      }
    }
    $unexpectedStackDeckExe = @(Get-ChildItem -LiteralPath $dest -Filter "stackdeck*.exe" -File -Recurse -ErrorAction SilentlyContinue |
      Where-Object { $_.Name -ne "stackdeck.exe" -and $_.Name -notlike "stackdeck-hive*.exe" })
    if ($unexpectedStackDeckExe.Count -gt 0) {
      throw "Unexpected StackDeck executable payload(s) found in $($installer.Name): $($unexpectedStackDeckExe.Name -join ', ')"
    }

    if (-not $SkipSignature) {
      foreach ($payload in @(@($desktop + $hive) | Sort-Object FullName -Unique)) {
        Invoke-NativeChecked -FilePath $signTool -ArgumentList @("verify", "/pa", "/all", "/v", $payload.FullName)
        $signature = Get-AuthenticodeSignature -LiteralPath $payload.FullName
        if ($signature.Status -ne "Valid") {
          throw "Embedded payload signature is not valid: $($payload.FullName) [$($signature.Status)]"
        }
      }
      Write-Host "Embedded signed payload verification passed: $($installer.Name)"
    } else {
      Write-Host "Embedded unsigned payload hash verification passed: $($installer.Name)"
    }
  }
} finally {
  Remove-Item -LiteralPath $tempRoot -Recurse -Force -ErrorAction SilentlyContinue
}
