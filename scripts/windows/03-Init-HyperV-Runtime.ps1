[CmdletBinding()]
param(
  [string]$PystackExe = "pystack",
  [string]$ImageUrl = "https://cloud.debian.org/images/cloud/bookworm/latest/debian-12-genericcloud-amd64.qcow2",
  [string]$Sha256 = "",
  [string]$Sha512 = "",
  [UInt32]$Timeout = 900
)
$ErrorActionPreference = "Stop"

function Invoke-Pystack {
  & $PystackExe @args
  if ($LASTEXITCODE -ne 0) {
    throw "pystack command failed with exit code ${LASTEXITCODE}: $($args -join ' ')"
  }
}

Invoke-Pystack hyperv ensure-key
$initArgs = @("hyperv", "init", "--url", $ImageUrl, "--timeout", $Timeout)
if ($Sha256) { $initArgs += @("--sha256", $Sha256) }
if ($Sha512) { $initArgs += @("--sha512", $Sha512) }
Invoke-Pystack @initArgs
Invoke-Pystack hyperv health
