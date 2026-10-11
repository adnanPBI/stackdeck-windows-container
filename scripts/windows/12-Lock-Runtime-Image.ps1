#Requires -Version 5.1

[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)]
  [string]$ImageUrl,
  [Parameter(Mandatory = $true)]
  [ValidatePattern('^[A-Fa-f0-9]{128}$')]
  [string]$ImageSha512,
  [Parameter(Mandatory = $true)]
  [ValidatePattern('^\d{8}T\d{6}Z$')]
  [string]$DebianSnapshotTimestamp,
  [Parameter(Mandatory = $true)]
  [ValidatePattern('^\d+(?:\.\d+){1,3}$')]
  [string]$NerdctlVersion,
  [Parameter(Mandatory = $true)]
  [ValidatePattern('^[A-Fa-f0-9]{64}$')]
  [string]$NerdctlSha256Amd64,
  [string]$Output = ''
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$uri = $null
if (-not [Uri]::TryCreate($ImageUrl, [UriKind]::Absolute, [ref]$uri) -or $uri.Scheme -ne 'https') {
  throw '-ImageUrl must be an absolute HTTPS URL.'
}
if ($uri.AbsoluteUri -match '/latest/') {
  throw 'Release runtime images must be version-pinned. /latest/ URLs are forbidden.'
}

$repoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..\..')).ProviderPath
if (-not $Output) { $Output = Join-Path $repoRoot 'runtime\windows-runtime-lock.json' }
$outputPath = [IO.Path]::GetFullPath($Output)
[IO.Directory]::CreateDirectory((Split-Path -Parent $outputPath)) | Out-Null

$lock = [ordered]@{
  schema = 'https://stackdeck.dev/schemas/windows-runtime-lock-v1.json'
  image_url = $uri.AbsoluteUri
  image_sha512 = $ImageSha512.ToLowerInvariant()
  debian_snapshot_timestamp = $DebianSnapshotTimestamp
  nerdctl_version = $NerdctlVersion
  nerdctl_sha256_amd64 = $NerdctlSha256Amd64.ToLowerInvariant()
}
[IO.File]::WriteAllText(
  $outputPath,
  ($lock | ConvertTo-Json -Depth 4),
  [Text.UTF8Encoding]::new($false)
)
Write-Host "Immutable Windows runtime lock written: $outputPath" -ForegroundColor Green
Write-Host "Image: $($lock.image_url)"
Write-Host "Debian snapshot: $($lock.debian_snapshot_timestamp)"
Write-Host "nerdctl: $($lock.nerdctl_version)"
