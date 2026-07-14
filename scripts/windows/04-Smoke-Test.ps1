[CmdletBinding()]
param(
  [string]$PystackExe = "stackdeck",
  [string]$ComposeFile = "examples\smoke\docker-compose.yml",
  [switch]$KeepRunning
)
$ErrorActionPreference = "Stop"
& $PystackExe hyperv health
& $PystackExe compose up -f $ComposeFile --backend hyperv -d
& $PystackExe compose status -f $ComposeFile --backend hyperv --json
& $PystackExe compose logs -f $ComposeFile --backend hyperv web --tail 80
if (-not $KeepRunning) {
  & $PystackExe compose down -f $ComposeFile --backend hyperv -v
}
Write-Host "Smoke test completed." -ForegroundColor Green
