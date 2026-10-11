#Requires -Version 5.1
#Requires -RunAsAdministrator

[CmdletBinding()]
param(
  [ValidateNotNullOrEmpty()]
  [string]$HiveExe = "$env:ProgramFiles\StackDeck\stackdeck-hive.exe",
  [ValidatePattern("^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$")]
  [string]$VmName = "stackdeck-linux",
  [ValidateNotNullOrEmpty()]
  [string]$VmRoot = "$env:SystemDrive\StackDeck\VMs\stackdeck-linux",
  [ValidateRange(8, 2048)]
  [UInt32]$DiskGB = 8,
  [ValidateRange(1024, 1048576)]
  [UInt32]$MemoryMB = 4096,
  [ValidateRange(1, 256)]
  [UInt32]$Cpus = 2,
  [ValidateNotNullOrEmpty()]
  [string]$SwitchName = "Default Switch",
  [ValidateRange(60, 3600)]
  [UInt32]$Timeout = 900,
  [string]$ImageUrl = "",
  [ValidatePattern("^[A-Fa-f0-9]{128}$")]
  [string]$ImageSha512 = "",
  [string]$RuntimeLockPath = ""
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"
if (-not $RuntimeLockPath) {
  $RuntimeLockPath = Join-Path (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "..\..")).ProviderPath "runtime\windows-runtime-lock.json"
}
function Get-RuntimeLock {
  if (-not (Test-Path -LiteralPath $RuntimeLockPath -PathType Leaf)) { throw "Runtime lock was not found: $RuntimeLockPath" }
  $lock = Get-Content -LiteralPath $RuntimeLockPath -Raw | ConvertFrom-Json
  if ([string]$lock.image_url -eq "UNLOCKED" -or [string]$lock.image_url -match "/latest/") { throw "Runtime image lock is not release-pinned." }
  if ([string]$lock.image_sha512 -notmatch "^[A-Fa-f0-9]{128}$") { throw "Runtime image SHA-512 lock is invalid." }
  return $lock
}

function Invoke-Hive {
  param([Parameter(Mandatory = $true)][string[]]$ArgumentList)

  & $script:ResolvedHive @ArgumentList
  if ($LASTEXITCODE -ne 0) {
    throw "stackdeck-hive failed with exit code $LASTEXITCODE."
  }
}

function Set-StackDeckPrivateDirectoryAcl {
  param([Parameter(Mandatory = $true)][string]$Path)

  $acl = New-Object Security.AccessControl.DirectorySecurity
  $acl.SetAccessRuleProtection($true, $false)
  $inheritance = [Security.AccessControl.InheritanceFlags]"ContainerInherit, ObjectInherit"
  $propagation = [Security.AccessControl.PropagationFlags]::None
  $access = [Security.AccessControl.FileSystemRights]::FullControl
  $allow = [Security.AccessControl.AccessControlType]::Allow
  $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
  $sids = @(
    $identity.User,
    [Security.Principal.SecurityIdentifier]::new("S-1-5-18"),
    [Security.Principal.SecurityIdentifier]::new("S-1-5-32-544")
  )
  foreach ($sid in $sids) {
    $rule = [Security.AccessControl.FileSystemAccessRule]::new(
      $sid, $access, $inheritance, $propagation, $allow
    )
    [void]$acl.AddAccessRule($rule)
  }
  [IO.Directory]::SetAccessControl($Path, $acl)
}

function Initialize-StackDeckMachineConfigRoot {
  $dataRoot = Join-Path $env:ProgramData "StackDeck"
  $configRoot = Join-Path $dataRoot "config"
  foreach ($path in @($dataRoot, $configRoot)) {
    if (Test-Path -LiteralPath $path) {
      $item = Get-Item -LiteralPath $path -Force
      if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "Refusing to use a reparse point for StackDeck machine config: $path"
      }
    }
    [IO.Directory]::CreateDirectory($path) | Out-Null
    Set-StackDeckPrivateDirectoryAcl -Path $path
  }
  return $configRoot
}

if (-not (Test-Path -LiteralPath $HiveExe -PathType Leaf)) {
  throw "Installed StackDeck sidecar was not found: $HiveExe"
}
$ResolvedHive = (Resolve-Path -LiteralPath $HiveExe).ProviderPath
$env:STACKDECK_CONFIG_DIR = Initialize-StackDeckMachineConfigRoot

if (-not [IO.Path]::IsPathRooted($VmRoot) -or $VmRoot.StartsWith("\\")) {
  throw "VM root must be an absolute local-drive path: $VmRoot"
}
$VmRoot = [IO.Path]::GetFullPath($VmRoot)
[IO.Directory]::CreateDirectory($VmRoot) | Out-Null

if (-not (Get-VMSwitch -Name $SwitchName -ErrorAction SilentlyContinue)) {
  throw "Hyper-V switch '$SwitchName' was not found."
}

$existingVm = Get-VM -Name $VmName -ErrorAction SilentlyContinue
if ($existingVm) {
  $diskPaths = @(Get-VMHardDiskDrive -VMName $VmName -ErrorAction SilentlyContinue |
      ForEach-Object Path |
      Where-Object { $_ })
  $missingDisk = $diskPaths.Count -eq 0 -or
    @($diskPaths | Where-Object { -not (Test-Path -LiteralPath $_ -PathType Leaf) }).Count -gt 0
  if (-not $missingDisk) {
    throw "VM '$VmName' already exists with an attached disk. Refusing to overwrite it."
  }
  Write-Host "Removing stale VM '$VmName' because its attached VHDX is missing..."
  Stop-VM -Name $VmName -TurnOff -Force -ErrorAction SilentlyContinue
  Remove-VM -Name $VmName -Force
}

if (-not $ImageUrl) {
  $lock = Get-RuntimeLock
  $ImageUrl = [string]$lock.image_url
  if (-not $ImageSha512) { $ImageSha512 = [string]$lock.image_sha512 }
}
$parsedImageUri = $null
if (-not [Uri]::TryCreate($ImageUrl, [UriKind]::Absolute, [ref]$parsedImageUri) -or $parsedImageUri.Scheme -ne "https") {
  throw "-ImageUrl must be an absolute HTTPS URL."
}
if (-not $ImageSha512) { throw "-ImageSha512 or a release runtime lock is required." }
$sha512 = $ImageSha512.ToLowerInvariant()

Invoke-Hive -ArgumentList @(
  "hyperv", "configure",
  "--vm-name", $VmName,
  "--vm-root", $VmRoot,
  "--disk-gb", "$DiskGB",
  "--memory-mb", "$MemoryMB",
  "--cpus", "$Cpus",
  "--switch-name", $SwitchName,
  "--portproxy", "true"
)
Invoke-Hive -ArgumentList @("hyperv", "ensure-key")
Invoke-Hive -ArgumentList @(
  "hyperv", "init",
  "--url", $ImageUrl,
  "--sha512", $sha512,
  "--timeout", "$Timeout"
)
Invoke-Hive -ArgumentList @("hyperv", "health")
Invoke-Hive -ArgumentList @("hyperv", "ps")

$createdVhdx = Get-ChildItem -LiteralPath $VmRoot -Recurse -Filter *.vhdx -File |
  Select-Object -First 1
if (-not $createdVhdx) {
  throw "Initialization returned success but no VHDX was found under $VmRoot."
}
$vhd = Get-VHD -LiteralPath $createdVhdx.FullName
if ($vhd.VhdType -ne "Dynamic") {
  throw "Created VHDX is not dynamic: $($createdVhdx.FullName)"
}

Write-Host ""
Write-Host "StackDeck runtime is ready." -ForegroundColor Green
Write-Host "VM:   $VmName"
Write-Host "VHDX: $($createdVhdx.FullName)"
Write-Host "Type: $($vhd.VhdType)"
Write-Host "Max:  $([math]::Round($vhd.Size / 1GB, 2)) GB"
Write-Host "SMB:  disabled (no Windows password required)"
