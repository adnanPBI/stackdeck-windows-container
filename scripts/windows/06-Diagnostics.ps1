[CmdletBinding()]
param(
  [string]$PystackExe = "stackdeck",
  [string]$Config = "stack.json",
  [string]$Output = ".stackdeck\diagnostics",
  [UInt32]$Tail = 200,
  [string]$Backend = "hyperv"
)
$ErrorActionPreference = "Stop"
& $PystackExe --config $Config --backend $Backend diagnostics --output $Output --tail $Tail
if ($LASTEXITCODE -ne 0) {
  throw "$PystackExe diagnostics failed with exit code $LASTEXITCODE"
}
Write-Host "Diagnostics written to $Output" -ForegroundColor Green
