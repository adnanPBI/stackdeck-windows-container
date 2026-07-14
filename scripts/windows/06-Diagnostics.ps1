[CmdletBinding()]
param(
  [string]$PystackExe = "stackdeck",
  [string]$Config = "stack.json",
  [string]$Output = ".stackdeck\diagnostics",
  [UInt32]$Tail = 200
)
$ErrorActionPreference = "Stop"
& $PystackExe --config $Config diagnostics --output $Output --tail $Tail
Write-Host "Diagnostics written to $Output" -ForegroundColor Green
