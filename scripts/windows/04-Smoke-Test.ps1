[CmdletBinding()]
param(
  [string]$PystackExe = "pystack",
  [string]$ComposeFile = "examples\smoke\docker-compose.yml",
  [switch]$KeepRunning
)
$ErrorActionPreference = "Stop"

function Invoke-Pystack {
  & $PystackExe @args
  if ($LASTEXITCODE -ne 0) {
    throw "pystack command failed with exit code ${LASTEXITCODE}: $($args -join ' ')"
  }
}

Invoke-Pystack hyperv health
Invoke-Pystack compose up -f $ComposeFile --backend hyperv -d
Invoke-Pystack compose status -f $ComposeFile --backend hyperv --json
Invoke-Pystack compose logs -f $ComposeFile --backend hyperv web --tail 80
if (-not $KeepRunning) {
  Invoke-Pystack compose down -f $ComposeFile --backend hyperv -v
}
Write-Host "Smoke test completed." -ForegroundColor Green
