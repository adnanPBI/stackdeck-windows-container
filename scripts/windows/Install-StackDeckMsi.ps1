[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)]
  [string]$MsiPath,

  [string]$InstallDir = "",
  [string]$VmRoot = "",
  [string]$LogDir = ""
)

$ErrorActionPreference = "Stop"

function Resolve-FullPath {
  param([string]$Path)
  [System.IO.Path]::GetFullPath((Resolve-Path -LiteralPath $Path -ErrorAction Stop).Path)
}

function Select-StackDeckVmRoot {
  param([string]$InitialPath)

  Add-Type -AssemblyName System.Windows.Forms
  $dialog = New-Object System.Windows.Forms.FolderBrowserDialog
  $dialog.Description = "Select the StackDeck Hyper-V VM image/runtime folder. This folder stores stackdeck-linux VHDX, seed ISO, and runtime files."
  $dialog.ShowNewFolderButton = $true
  if ($InitialPath -and (Test-Path -LiteralPath $InitialPath)) {
    $dialog.SelectedPath = $InitialPath
  }
  $result = $dialog.ShowDialog()
  if ($result -eq [System.Windows.Forms.DialogResult]::OK -and $dialog.SelectedPath) {
    return $dialog.SelectedPath
  }
  return $InitialPath
}

$MsiPath = Resolve-FullPath $MsiPath
if (-not $VmRoot) {
  $defaultRoot = if ($InstallDir) {
    Join-Path $InstallDir "VMs\stackdeck-linux"
  } else {
    Join-Path $env:USERPROFILE "StackDeckVMs\stackdeck-linux"
  }
  $VmRoot = Select-StackDeckVmRoot -InitialPath $defaultRoot
}

if (-not $LogDir) {
  $LogDir = Join-Path $env:LOCALAPPDATA "StackDeck\install-logs"
}
New-Item -ItemType Directory -Force -Path $LogDir | Out-Null
$timestamp = Get-Date -Format "yyyyMMdd-HHmmss"
$msiLog = Join-Path $LogDir "stackdeck-msi-$timestamp.log"
$configLog = Join-Path $LogDir "stackdeck-msi-config-$timestamp.log"

$args = @("/i", $MsiPath, "/L*V", $msiLog)
if ($InstallDir) {
  New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
  $args += "INSTALLDIR=$InstallDir"
}

Write-Host "Installing StackDeck MSI..." -ForegroundColor Cyan
Write-Host "MSI:      $MsiPath"
Write-Host "VM root:  $VmRoot"
Write-Host "MSI log:  $msiLog"

$process = Start-Process -FilePath "msiexec.exe" -ArgumentList $args -Wait -PassThru
if ($process.ExitCode -ne 0) {
  throw "msiexec failed with exit code $($process.ExitCode). See $msiLog"
}

$candidateInstallDirs = @()
if ($InstallDir) { $candidateInstallDirs += $InstallDir }
$candidateInstallDirs += @(
  (Join-Path $env:LOCALAPPDATA "Programs\StackDeck"),
  (Join-Path $env:ProgramFiles "StackDeck"),
  (Join-Path ${env:ProgramFiles(x86)} "StackDeck")
) | Where-Object { $_ }

$resolvedInstallDir = $candidateInstallDirs | Where-Object {
  Test-Path -LiteralPath (Join-Path $_ "StackDeck.exe") -PathType Leaf
} | Select-Object -First 1
if (-not $resolvedInstallDir) {
  $resolvedInstallDir = $candidateInstallDirs | Where-Object {
    Test-Path -LiteralPath (Join-Path $_ "stackdeck-hive.exe") -PathType Leaf
  } | Select-Object -First 1
}
if (-not $resolvedInstallDir) {
  throw "StackDeck installed, but the install directory could not be located. MSI log: $msiLog"
}

$configScript = Join-Path $resolvedInstallDir "resources\installer\Configure-StackDeckInstall.ps1"
if (-not (Test-Path -LiteralPath $configScript)) {
  throw "StackDeck installed, but installer configuration script was not found at $configScript"
}

Write-Host "Configuring Hyper-V runtime selection..." -ForegroundColor Cyan
& powershell.exe -NoProfile -ExecutionPolicy Bypass -File $configScript `
  -InstallDir $resolvedInstallDir `
  -VmRoot $VmRoot `
  -LogPath $configLog `
  -InstallerKind "MSI"

if ($LASTEXITCODE -ne 0) {
  throw "StackDeck MSI configuration failed with exit code $LASTEXITCODE. See $configLog"
}

Set-Content -LiteralPath (Join-Path $LogDir "latest-msi-install-log.txt") -Value $msiLog -Encoding UTF8
Set-Content -LiteralPath (Join-Path $LogDir "latest-msi-config-log.txt") -Value $configLog -Encoding UTF8

Write-Host "StackDeck MSI installation and runtime configuration completed." -ForegroundColor Green
Write-Host "MSI log:     $msiLog"
Write-Host "Config log:  $configLog"
