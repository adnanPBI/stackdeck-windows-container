[CmdletBinding()]
param(
  [string]$PystackExe = "pystack",
  [string]$VmName = "pystack-linux",
  [string]$VmRoot = "D:\StackDeck\VMs\pystack-linux",
  [UInt32]$DiskGB = 10,
  [UInt32]$MemoryMB = 4096,
  [UInt32]$Cpus = 2,
  [string]$SwitchName = "Default Switch",
  [string]$WindowsHost = $env:COMPUTERNAME,
  [string]$SmbUser = "$env:USERDOMAIN\$env:USERNAME",
  [string]$SmbPassword = ""
)
$ErrorActionPreference = "Stop"
New-Item -ItemType Directory -Force -Path $VmRoot | Out-Null

$args = @(
  "hyperv", "configure",
  "--vm-name", $VmName,
  "--vm-root", $VmRoot,
  "--disk-gb", "$DiskGB",
  "--memory-mb", "$MemoryMB",
  "--cpus", "$Cpus",
  "--switch-name", $SwitchName,
  "--portproxy", "true",
  "--windows-host", $WindowsHost,
  "--smb-user", $SmbUser
)
if ($SmbPassword) { $args += @("--smb-password", $SmbPassword) }
& $PystackExe @args
& $PystackExe hyperv ensure-key
& $PystackExe hyperv doctor
