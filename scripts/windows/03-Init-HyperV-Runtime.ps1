[CmdletBinding()]
param(
  [string]$PystackExe = "pystack",
  [string]$ImageUrl = "https://cloud-images.ubuntu.com/jammy/current/jammy-server-cloudimg-amd64.img",
  [Parameter(Mandatory=$true)][string]$Sha256,
  [UInt32]$Timeout = 900
)
$ErrorActionPreference = "Stop"
& $PystackExe hyperv ensure-key
& $PystackExe hyperv init --url $ImageUrl --sha256 $Sha256 --timeout $Timeout
& $PystackExe hyperv health
