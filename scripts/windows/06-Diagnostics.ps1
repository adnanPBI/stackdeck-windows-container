#Requires -Version 5.1
#Requires -RunAsAdministrator

[CmdletBinding()]
param(
  [string]$StackDeckExe = "stackdeck-cli",
  [string]$Config = "stack.json",
  [string]$Output = ".stackdeck\diagnostics",
  [ValidateRange(1, 100000)]
  [UInt32]$Tail = 200
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Resolve-ApplicationPath {
  param([Parameter(Mandatory = $true)][string]$Value)

  if ([IO.Path]::IsPathRooted($Value) -or $Value -match "[\\/]") {
    if (-not (Test-Path -LiteralPath $Value -PathType Leaf)) {
      throw "StackDeck executable was not found: $Value"
    }
    return (Resolve-Path -LiteralPath $Value).ProviderPath
  }
  $command = Get-Command -Name $Value -CommandType Application -ErrorAction SilentlyContinue |
    Select-Object -First 1
  if (-not $command) {
    throw "StackDeck executable was not found on PATH: $Value"
  }
  return $command.Path
}

if (-not (Test-Path -LiteralPath $Config -PathType Leaf)) {
  throw "Configuration file was not found: $Config"
}
$ConfigPath = (Resolve-Path -LiteralPath $Config).ProviderPath
$StackDeckPath = Resolve-ApplicationPath $StackDeckExe
$OutputPath = if ([IO.Path]::IsPathRooted($Output)) {
  [IO.Path]::GetFullPath($Output)
} else {
  [IO.Path]::GetFullPath((Join-Path (Get-Location).Path $Output))
}
if (Test-Path -LiteralPath $OutputPath) {
  throw "Diagnostics output already exists. Choose a new path to avoid retaining stale logs: $OutputPath"
}

$PartialPath = "$OutputPath.partial.$([Guid]::NewGuid().ToString('N'))"
$primaryError = $null
try {
  & $StackDeckPath --config $ConfigPath diagnostics --output $PartialPath --tail $Tail
  $exitCode = $LASTEXITCODE
  if ($exitCode -ne 0) {
    throw "stackdeck-cli diagnostics failed with exit code $exitCode."
  }
  if (-not (Test-Path -LiteralPath $PartialPath -PathType Container)) {
    throw "stackdeck reported success without creating the diagnostics directory."
  }
  [IO.Directory]::Move($PartialPath, $OutputPath)
} catch {
  $primaryError = $_
} finally {
  if (Test-Path -LiteralPath $PartialPath) {
    try {
      Remove-Item -LiteralPath $PartialPath -Recurse -Force
    } catch {
      if ($primaryError) {
        Write-Warning "Diagnostics cleanup also failed: $($_.Exception.Message)"
      } else {
        $primaryError = $_
      }
    }
  }
}
if ($primaryError) {
  throw $primaryError
}

Write-Host "Diagnostics written to $OutputPath" -ForegroundColor Green
Write-Warning "Redaction is best-effort. Inspect every diagnostics file before sharing it."
