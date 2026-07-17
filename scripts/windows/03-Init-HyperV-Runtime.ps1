[CmdletBinding()]
param(
  [string]$PystackExe = "pystack",
  [string]$ImageUrl = "https://cloud.debian.org/images/cloud/bookworm/latest/debian-12-genericcloud-amd64.qcow2",
  [string]$Sha256 = "",
  [UInt32]$Timeout = 900
)
$ErrorActionPreference = "Stop"
& $PystackExe hyperv ensure-key
$initArgs = @("hyperv", "init", "--url", $ImageUrl, "--timeout", $Timeout)
if ($Sha256) { $initArgs += @("--sha256", $Sha256) }
& $PystackExe @initArgs
& $PystackExe hyperv health
