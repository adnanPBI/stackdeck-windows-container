[CmdletBinding()]
param(
  [string]$PystackExe = "target\release\stackdeck.exe",
  [string]$MsiPath = "",
  [string]$NsisPath = "",
  [string]$VmRoot = "",
  [string]$ImageVhdx = "",
  [string]$ImageUrl = "https://cloud-images.ubuntu.com/jammy/current/jammy-server-cloudimg-amd64.img",
  [string]$Sha256 = "",
  [string]$SmbPassword = "",
  [string]$EvidenceDir = ".stackdeck\clean-machine-proof",
  [UInt16]$ApiPort = 23750,
  [switch]$SkipInstall,
  [switch]$SkipInit,
  [switch]$SkipApi
)

$ErrorActionPreference = "Stop"
$RepoRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Set-Location $RepoRoot

function Write-ProofLog {
  param([string]$Message)
  $stamp = Get-Date -Format "yyyy-MM-dd HH:mm:ss.fff zzz"
  Add-Content -LiteralPath $script:ProofLog -Value "[$stamp] $Message" -Encoding UTF8
}

function Invoke-ProofStep {
  param(
    [string]$Name,
    [scriptblock]$Script
  )
  Write-Host "==> $Name" -ForegroundColor Cyan
  Write-ProofLog "STEP_START $Name"
  try {
    & $Script 2>&1 | Tee-Object -FilePath (Join-Path $EvidenceDir "$($Name -replace '[^A-Za-z0-9_.-]', '_').log")
    Write-ProofLog "STEP_OK $Name"
  } catch {
    Write-ProofLog "STEP_FAIL $Name $($_.Exception.Message)"
    throw
  }
}

New-Item -ItemType Directory -Force -Path $EvidenceDir | Out-Null
$ProofLog = Join-Path $EvidenceDir "proof.log"
Set-Content -LiteralPath $ProofLog -Value "" -Encoding UTF8
Write-ProofLog "StackDeck clean-machine proof start"
Write-ProofLog "repo=$RepoRoot"
Write-ProofLog "pystack_exe=$PystackExe"
Write-ProofLog "vm_root=$VmRoot"

Invoke-ProofStep "00-prereqs" {
  .\scripts\windows\00-Ensure-Prereqs.ps1 -EnableFeatures
}

Invoke-ProofStep "01-build" {
  .\scripts\windows\01-Build-Release.ps1 -SkipDesktop
}

if (-not $SkipInstall) {
  if ($MsiPath) {
    Invoke-ProofStep "02-install-msi" {
      $args = @{
        MsiPath = $MsiPath
      }
      if ($VmRoot) { $args.VmRoot = $VmRoot }
      .\scripts\windows\Install-StackDeckMsi.ps1 @args
    }
  } elseif ($NsisPath) {
    Invoke-ProofStep "02-install-nsis" {
      & $NsisPath
    }
  } else {
    Write-ProofLog "install_skipped no installer path supplied"
  }
}

Invoke-ProofStep "03-configure-hyperv" {
  $args = @{
    PystackExe = $PystackExe
  }
  if ($SmbPassword) { $args.SmbPassword = $SmbPassword }
  if ($VmRoot) { $args.VmRoot = $VmRoot }
  .\scripts\windows\02-Configure-HyperV.ps1 @args
}

if (-not $SkipInit) {
  Invoke-ProofStep "04-init-runtime" {
    $args = @{
      PystackExe = $PystackExe
      Timeout = 900
    }
    if ($ImageVhdx) { $args.ImageVhdx = $ImageVhdx }
    if ($ImageUrl) { $args.ImageUrl = $ImageUrl }
    if ($Sha256) { $args.Sha256 = $Sha256 }
    .\scripts\windows\03-Init-HyperV-Runtime.ps1 @args
  }
}

Invoke-ProofStep "05-smoke-compose" {
  .\scripts\windows\04-Smoke-Test.ps1 -PystackExe $PystackExe
}

if (-not $SkipApi) {
  Invoke-ProofStep "06-api-shim" {
    .\scripts\windows\05-Install-Api-Task.ps1 -PystackExe (Resolve-Path $PystackExe) -Port $ApiPort
    $tokenPath = Join-Path $env:ProgramData "StackDeck\config\docker-api-token.txt"
    $token = (Get-Content -Raw -LiteralPath $tokenPath).Trim()
    $headers = @{ Authorization = "Bearer $token" }
    Invoke-RestMethod -Headers $headers -Uri "http://127.0.0.1:$ApiPort/v1.43/_ping" | Out-String
    Invoke-RestMethod -Headers $headers -Uri "http://127.0.0.1:$ApiPort/v1.43/version" | ConvertTo-Json -Depth 10
    Invoke-RestMethod -Headers $headers -Uri "http://127.0.0.1:$ApiPort/v1.43/info" | ConvertTo-Json -Depth 10
    Invoke-RestMethod -Headers $headers -Uri "http://127.0.0.1:$ApiPort/v1.43/containers/json?all=1" | ConvertTo-Json -Depth 10
  }
}

Invoke-ProofStep "07-diagnostics" {
  .\scripts\windows\06-Diagnostics.ps1 -PystackExe $PystackExe -Config stack.json -Output (Join-Path $EvidenceDir "diagnostics") -Tail 200
}

Write-ProofLog "StackDeck clean-machine proof complete"
Write-Host "Proof evidence written to $EvidenceDir" -ForegroundColor Green
