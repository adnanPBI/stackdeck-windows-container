#Requires -Version 5.1
#Requires -RunAsAdministrator

[CmdletBinding()]
param(
  [string]$InstallRoot = "",
  [string]$Output = ".\stackdeck-client-qualification.json",
  [switch]$AllowWindows10,
  [switch]$SkipSignature
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$results = New-Object System.Collections.Generic.List[object]

if (-not $InstallRoot) {
  $candidates = @(@(
    (Join-Path $env:ProgramFiles "StackDeck"),
    (Join-Path $env:LOCALAPPDATA "Programs\StackDeck")
  ) | Where-Object { $_ -and (Test-Path -LiteralPath $_ -PathType Container) } | Select-Object -Unique)
  if ($candidates.Count -ne 1) {
    throw "Unable to resolve exactly one StackDeck install root. Supply -InstallRoot explicitly. Candidates: $($candidates -join ', ')"
  }
  $InstallRoot = $candidates[0]
}
$InstallRoot = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($InstallRoot)
if ($SkipSignature) {
  Write-Warning "Signature verification is skipped for PRE-SIGNING FUNCTIONAL QUALIFICATION only. This evidence is not final client-release evidence."
}

function Add-Check {
  param([string]$Name, [bool]$Passed, [string]$Detail)
  $results.Add([pscustomobject]@{
    check = $Name
    status = $(if ($Passed) { "PASS" } else { "FAIL" })
    detail = $Detail
  })
}

function Invoke-Captured {
  param([string]$FilePath, [string[]]$Arguments)
  $output = & $FilePath @Arguments 2>&1 | Out-String
  $code = $LASTEXITCODE
  return [pscustomobject]@{ ExitCode = $code; Output = $output.Trim() }
}

$os = Get-CimInstance Win32_OperatingSystem
$caption = [string]$os.Caption
$arch = [string]$os.OSArchitecture
$is11 = $caption -match 'Windows 11'
$is10 = $caption -match 'Windows 10'
$editionOk = $caption -match '(?i)\b(Pro|Professional|Enterprise|Education)\b'
$osOk = ($arch -match '64') -and $editionOk -and ($is11 -or ($AllowWindows10 -and $is10))
Add-Check "Supported Windows baseline" $osOk "$caption; architecture=$arch; AllowWindows10=$AllowWindows10"

$feature = Get-WindowsOptionalFeature -Online -FeatureName Microsoft-Hyper-V-All -ErrorAction SilentlyContinue
$featureState = if ($feature) { [string]$feature.State } else { "unavailable" }
Add-Check "Hyper-V enabled" ([bool]($feature -and $feature.State -eq "Enabled")) "state=$featureState"
$defaultSwitch = Get-VMSwitch -Name "Default Switch" -ErrorAction SilentlyContinue
Add-Check "Hyper-V Default Switch" ([bool]$defaultSwitch) $(if ($defaultSwitch) { $defaultSwitch.Name } else { "not found" })

$qemu = Get-Command qemu-img.exe -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $qemu) { $qemu = Get-Command qemu-img -ErrorAction SilentlyContinue | Select-Object -First 1 }
if (-not $qemu) {
  foreach ($candidate in @("$env:ProgramFiles\qemu\qemu-img.exe", "${env:ProgramFiles(x86)}\qemu\qemu-img.exe", "$env:ChocolateyInstall\bin\qemu-img.exe")) {
    if ($candidate -and (Test-Path -LiteralPath $candidate -PathType Leaf)) {
      $qemu = Get-Item -LiteralPath $candidate
      break
    }
  }
}
$qemuDetail = "required for default Debian QCOW2 first-run path"
if ($qemu) {
  $pathProperty = $qemu.PSObject.Properties["Path"]
  $fullNameProperty = $qemu.PSObject.Properties["FullName"]
  if ($pathProperty -and $pathProperty.Value) { $qemuDetail = [string]$pathProperty.Value }
  elseif ($fullNameProperty -and $fullNameProperty.Value) { $qemuDetail = [string]$fullNameProperty.Value }
  else { $qemuDetail = [string]$qemu }
}
Add-Check "qemu-img available" ([bool]$qemu) $qemuDetail

if (-not (Test-Path -LiteralPath $InstallRoot -PathType Container)) {
  Add-Check "Install root" $false "missing: $InstallRoot"
} else {
  Add-Check "Install root" $true $InstallRoot
}

$desktop = @(Get-ChildItem -LiteralPath $InstallRoot -Filter "stackdeck.exe" -File -Recurse -ErrorAction SilentlyContinue | Select-Object -First 1)
$hive = @(Get-ChildItem -LiteralPath $InstallRoot -Filter "stackdeck-hive*.exe" -File -Recurse -ErrorAction SilentlyContinue | Select-Object -First 1)
Add-Check "Desktop executable installed" ($desktop.Count -eq 1) $(if ($desktop.Count) { $desktop[0].FullName } else { "not found" })
Add-Check "Hive sidecar installed" ($hive.Count -eq 1) $(if ($hive.Count) { $hive[0].FullName } else { "not found" })

if (-not $SkipSignature) {
  foreach ($entry in @($desktop + $hive)) {
    if (-not $entry) { continue }
    $sig = Get-AuthenticodeSignature -LiteralPath $entry.FullName
    $signer = if ($sig.SignerCertificate) { [string]$sig.SignerCertificate.Subject } else { "none" }
    Add-Check "Authenticode: $($entry.Name)" ($sig.Status -eq "Valid") "$($sig.Status); signer=$signer"
  }
} else {
  Add-Check "Authenticode verification" $true "SKIPPED - pre-signing functional qualification only"
}

if ($hive.Count -eq 1) {
  $version = Invoke-Captured -FilePath $hive[0].FullName -Arguments @("--version")
  Add-Check "Hive version command" ($version.ExitCode -eq 0) $version.Output

  $health = Invoke-Captured -FilePath $hive[0].FullName -Arguments @("hyperv", "health")
  Add-Check "Strict Hyper-V runtime health" ($health.ExitCode -eq 0) $health.Output

  $ps = Invoke-Captured -FilePath $hive[0].FullName -Arguments @("hyperv", "ps")
  Add-Check "nerdctl runtime usability" ($ps.ExitCode -eq 0) $ps.Output
}

$machineDataRoot = [Environment]::GetFolderPath([Environment+SpecialFolder]::CommonApplicationData)
$configRoot = Join-Path (Join-Path $machineDataRoot "StackDeck") "config"
$configPath = Join-Path $configRoot "hyperv.json"
$secretPath = Join-Path $configRoot "smb-password.dpapi"
Add-Check "Machine-scoped config root" (Test-Path -LiteralPath $configRoot -PathType Container) $configRoot
if (Test-Path -LiteralPath $configPath -PathType Leaf) {
  $configText = Get-Content -LiteralPath $configPath -Raw
  $hasPlainField = $configText -match '"smb_password"\s*:'
  Add-Check "No plaintext SMB password in JSON" (-not $hasPlainField) $configPath
  if (Test-Path -LiteralPath $secretPath -PathType Leaf) {
    Add-Check "DPAPI SMB credential blob" $true $secretPath
  }
} else {
  Add-Check "Hyper-V config exists" $false $configPath
}

$summary = [ordered]@{
  schema = "https://stackdeck.dev/schemas/windows-client-qualification-v1.json"
  generated_utc = [DateTime]::UtcNow.ToString("o")
  computer = $env:COMPUTERNAME
  user = "$env:USERDOMAIN\$env:USERNAME"
  install_root = $InstallRoot
  signature_checks_skipped = [bool]$SkipSignature
  passed = -not [bool]($results | Where-Object { $_.status -eq "FAIL" })
  checks = @($results)
}

$outputPath = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($Output)
[IO.Directory]::CreateDirectory((Split-Path -Parent $outputPath)) | Out-Null
[IO.File]::WriteAllText($outputPath, ($summary | ConvertTo-Json -Depth 6), [Text.UTF8Encoding]::new($false))
$results | Format-Table -AutoSize
Write-Host "Qualification evidence: $outputPath"

if (-not $summary.passed) {
  throw "Installed client qualification failed. Review the FAIL checks above."
}
Write-Host "Installed client qualification passed." -ForegroundColor Green
