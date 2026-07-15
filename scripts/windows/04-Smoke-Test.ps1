[CmdletBinding()]
param(
  [string]$PystackExe = "stackdeck",
  [string]$ComposeFile = "examples\smoke\docker-compose.yml",
  [switch]$KeepRunning
)
$ErrorActionPreference = "Stop"
function Invoke-Checked {
  param([string[]]$CommandArgs)
  & $PystackExe @CommandArgs
  if ($LASTEXITCODE -ne 0) {
    throw "$PystackExe $($CommandArgs -join ' ') failed with exit code $LASTEXITCODE"
  }
}
Invoke-Checked -CommandArgs @("hyperv", "health")
Invoke-Checked -CommandArgs @("compose", "up", "-f", $ComposeFile, "--backend", "hyperv", "-d")
Invoke-Checked -CommandArgs @("compose", "status", "-f", $ComposeFile, "--backend", "hyperv", "--json")
Invoke-Checked -CommandArgs @("compose", "logs", "-f", $ComposeFile, "--backend", "hyperv", "web", "--tail", "80")
if (-not $KeepRunning) {
  Invoke-Checked -CommandArgs @("compose", "down", "-f", $ComposeFile, "--backend", "hyperv", "-v")
}
Write-Host "Smoke test completed." -ForegroundColor Green
