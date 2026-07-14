[CmdletBinding()]
param(
  [string]$PystackExe = "stackdeck",
  [string]$ImageVhdx = "",
  [string]$ImageUrl = "https://cloud-images.ubuntu.com/jammy/current/jammy-server-cloudimg-amd64.img",
  [string]$Sha256 = "",
  [UInt32]$Timeout = 900
)
$ErrorActionPreference = "Stop"
& $PystackExe hyperv ensure-key
$args = @("hyperv", "init", "--timeout", "$Timeout")
if ($ImageVhdx) {
  $args += @("--image-vhdx", $ImageVhdx)
} else {
  if (-not $Sha256) {
    throw "Sha256 is required when ImageVhdx is not supplied."
  }
  $args += @("--url", $ImageUrl, "--sha256", $Sha256)
}
& $PystackExe @args
& $PystackExe hyperv health
