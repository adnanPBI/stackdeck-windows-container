#Requires -Version 5.1
#Requires -RunAsAdministrator

[CmdletBinding()]
param(
  [string]$StackDeckExe = "stackdeck-cli",
  [string]$ImageVhdx = "",
  [string]$ImageUrl = "",
  [string]$RuntimeLockPath = "",
  [string]$Sha256 = "",
  [string]$Sha512 = "",
  [ValidateRange(60, 3600)]
  [UInt32]$Timeout = 900
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
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

function Resolve-ApplicationPath {
  param([Parameter(Mandatory = $true)][string]$Value)

  if ([IO.Path]::IsPathRooted($Value) -or $Value -match "[\\/]") {
    if (-not (Test-Path -LiteralPath $Value -PathType Leaf)) {
      throw "StackDeck executable was not found: $Value"
    }
    return (Resolve-Path -LiteralPath $Value).ProviderPath
  }
  $command = Get-Command -Name $Value -CommandType Application -ErrorAction SilentlyContinue |
    Select-Object -First 1
  if (-not $command) {
    throw "StackDeck executable was not found on PATH: $Value"
  }
  return $command.Path
}

function Invoke-StackDeck {
  param([Parameter(Mandatory = $true)][string[]]$ArgumentList)

  & $script:StackDeckPath @ArgumentList
  $exitCode = $LASTEXITCODE
  if ($exitCode -ne 0) {
    throw "stackdeck failed with exit code $exitCode."
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

$StackDeckPath = Resolve-ApplicationPath $StackDeckExe
$env:STACKDECK_CONFIG_DIR = Initialize-StackDeckMachineConfigRoot
$initArgs = @("hyperv", "init")

if ($Sha256 -and $Sha256 -notmatch "^[A-Fa-f0-9]{64}$") {
  throw "-Sha256 must contain exactly 64 hexadecimal characters."
}
if ($Sha512 -and $Sha512 -notmatch "^[A-Fa-f0-9]{128}$") {
  throw "-Sha512 must contain exactly 128 hexadecimal characters."
}

if ($ImageVhdx) {
  if (-not [IO.Path]::IsPathRooted($ImageVhdx) -or -not (Test-Path -LiteralPath $ImageVhdx -PathType Leaf)) {
    throw "-ImageVhdx must be an existing absolute file path."
  }
  if ([IO.Path]::GetExtension($ImageVhdx) -ne ".vhdx") {
    throw "-ImageVhdx must point to a .vhdx file."
  }
  $resolvedVhdx = (Resolve-Path -LiteralPath $ImageVhdx).ProviderPath
  if ($Sha256) {
    $actual = (Get-FileHash -LiteralPath $resolvedVhdx -Algorithm SHA256).Hash
    if ($actual -ine $Sha256) { throw "Local VHDX SHA-256 verification failed." }
  }
  if ($Sha512) {
    $actual = (Get-FileHash -LiteralPath $resolvedVhdx -Algorithm SHA512).Hash
    if ($actual -ine $Sha512) { throw "Local VHDX SHA-512 verification failed." }
  }
  $initArgs += @("--image-vhdx", $resolvedVhdx)
} else {
  if (-not $ImageUrl) {
    $lock = Get-RuntimeLock
    $ImageUrl = [string]$lock.image_url
    if (-not $Sha256 -and -not $Sha512) { $Sha512 = [string]$lock.image_sha512 }
  }
  $parsedImageUri = $null
  if (-not [Uri]::TryCreate($ImageUrl, [UriKind]::Absolute, [ref]$parsedImageUri) -or $parsedImageUri.Scheme -ne "https") {
    throw "-ImageUrl must be an absolute HTTPS URL."
  }
  if (-not $Sha256 -and -not $Sha512) {
    throw "Downloaded images require an explicit trusted digest or the immutable release runtime lock."
  }
  $initArgs += @("--url", $parsedImageUri.AbsoluteUri)
  if ($Sha256) { $initArgs += @("--sha256", $Sha256.ToLowerInvariant()) }
  if ($Sha512) { $initArgs += @("--sha512", $Sha512.ToLowerInvariant()) }
}
$initArgs += @("--timeout", "$Timeout")

Invoke-StackDeck -ArgumentList @("hyperv", "ensure-key")
try {
  Invoke-StackDeck -ArgumentList $initArgs
} catch {
  throw "Hyper-V initialization failed. If a partial VM already exists, inspect it and run 'stackdeck-cli hyperv repair --timeout $Timeout' before retrying. $($_.Exception.Message)"
}
Invoke-StackDeck -ArgumentList @("hyperv", "health")
# Unlike `hyperv health`, `hyperv ps` fails when SSH/containerd/nerdctl is not
# actually usable, so it is the final success gate.
Invoke-StackDeck -ArgumentList @("hyperv", "ps")
