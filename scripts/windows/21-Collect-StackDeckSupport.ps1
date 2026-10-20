#Requires -Version 5.1
<#
.SYNOPSIS
  Generate a privacy-filtered StackDeck support ZIP from an installed Windows PC.
.EXAMPLE
  powershell -NoProfile -ExecutionPolicy Bypass -File .\21-Collect-StackDeckSupport.ps1 -IncludeLogs -IncludeEvents
.DESCRIPTION
  Read-only. Does not require a repository or admin token. Never collects private
  SSH keys, config JSON, environment dumps, VM disks, project files or raw logs.
  Always inspect extracted ZIP contents before sharing.
#>
[CmdletBinding()]
param(
  [string]$OutputDirectory = [Environment]::GetFolderPath('Desktop'),
  [switch]$IncludeLogs,
  [switch]$IncludeEvents,
  [ValidateRange(1,30)][int]$Days = 7,
  [ValidateRange(1,30)][int]$MaxLogFiles = 12
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$base = [IO.Path]::GetFullPath($OutputDirectory)
if (!(Test-Path -LiteralPath $base -PathType Container)) {
  throw 'Output directory does not exist. Choose an existing writable folder.'
}
$ticket = [Guid]::NewGuid().ToString('N').Substring(0,10)
$prefix = 'StackDeck-Support-' + (Get-Date -Format 'yyyyMMdd-HHmmss') + '-' + $ticket
$staging = Join-Path ([IO.Path]::GetTempPath()) $prefix
$zip = Join-Path $base ($prefix + '.zip')
$shaFile = Join-Path $base ($prefix + '.sha256.txt')
foreach ($file in @($staging,$zip,$shaFile)) {
  if (Test-Path -LiteralPath $file) { throw 'Support output collision; no files overwritten.' }
}

function Sanitize([string]$Text) {
  if ($null -eq $Text) { return '' }
  $x = $Text
  # Never include PEM/SSH key blocks or credentials, even if accidentally logged.
  $x = [regex]::Replace($x, '(?is)-----BEGIN [A-Z0-9 ]*(?:KEY|CERTIFICATE)[A-Z0-9 ]*-----.*?-----END [A-Z0-9 ]*-----', '[PEM/KEY/CERTIFICATE REMOVED]')
  $x = [regex]::Replace($x, '(?im)^(?<key>[ \t]*Authorization[ \t]*[:=][ \t]*).+$', '${key}[REDACTED]')
  $x = [regex]::Replace($x, '(?im)(?<key>\b(?:SM_API_KEY|SM_CLIENT_CERT_PASSWORD|PASSWORD|PASSWD|TOKEN|SECRET|API[_-]?KEY|PRIVATE[_-]?KEY|SMB_PASSWORD|CLIENT_SECRET|ACCESS_KEY|CREDENTIAL)\b[ \t]*[:=][ \t]*)(?:"[^"]*"|\x27[^\x27]*\x27|[^\s,;]+)', '${key}[REDACTED]')
  $x = [regex]::Replace($x, '(?i)\bBearer\s+[A-Za-z0-9._~+/-]{8,}=*', 'Bearer [REDACTED]')
  $x = [regex]::Replace($x, '(?i)\b(?:ssh-rsa|ssh-ed25519)\s+[A-Za-z0-9+/]{60,}={0,2}', '[SSH PUBLIC KEY REDACTED]')
  $x = [regex]::Replace($x, '(?i)(https?://)[^/\s:@]+:[^/\s@]+@', '$1[REDACTED]@')
  $x = [regex]::Replace($x, '\b[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\b', '[TOKEN REDACTED]')
  $x = [regex]::Replace($x, '[A-Za-z0-9+/]{160,}={0,2}', '[LONG SECRET REDACTED]')
  $x = [regex]::Replace($x, '(?i)\b(?:[0-9]{1,3}\.){3}[0-9]{1,3}\b', '[IP ADDRESS]')
  $x = [regex]::Replace($x, '(?i)[A-Z]:\\Users\\[^\\\s/"\x27]+', 'C:\Users\[USER]')
  $x = [regex]::Replace($x, '(?i)\bS-1-5-(?:\d+-){1,}\d+\b', '[USER SID]')
  $x = [regex]::Replace($x, '(?i)\b[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}\b', '[EMAIL]')
  return $x
}
function WriteText([string]$Name, [string]$Value) {
  if ($Name -notmatch '^[A-Za-z0-9_.-]+$') { throw 'Unsafe support filename.' }
  $safe = Sanitize $Value
  [IO.File]::WriteAllText((Join-Path $staging $Name), $safe, [Text.UTF8Encoding]::new($false))
}
function SafeResult([scriptblock]$Action) {
  try { return & $Action } catch { return 'Unavailable: ' + $_.Exception.GetType().Name }
}
function FileFact([string]$Name,[string]$Path) {
  if (!(Test-Path -LiteralPath $Path -PathType Leaf)) {
    return [ordered]@{file=$Name;exists=$false}
  }
  $f=Get-Item -LiteralPath $Path -ErrorAction Stop
  $sig=Get-AuthenticodeSignature -LiteralPath $Path -ErrorAction SilentlyContinue
  return [ordered]@{file=$Name;exists=$true;size_bytes=$f.Length;
    sha256=(Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant();
    signature_status=$(if($sig){$sig.Status.ToString()}else{'Unavailable'})}
}
New-Item -ItemType Directory -Path $staging -ErrorAction Stop | Out-Null
try {
  $os=Get-CimInstance Win32_OperatingSystem -ErrorAction Stop
  $machine=[ordered]@{
    bundle_version='1.0'
    captured_utc=[datetime]::UtcNow.ToString('o')
    windows_caption=$os.Caption
    windows_version=$os.Version
    os_architecture=$os.OSArchitecture
    local_admin=[Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
    installed_files=@(
      (FileFact 'stackdeck.exe' 'C:\Program Files\StackDeck\stackdeck.exe'),
      (FileFact 'stackdeck-hive.exe' 'C:\Program Files\StackDeck\stackdeck-hive.exe')
    )
    approved_hyperv_vm='Not queried (elevation may be required)'
    event_log_collection_requested=[bool]$IncludeEvents
    application_log_collection_requested=[bool]$IncludeLogs
    no_config_or_key_contents_collected=$true
  }
  try {
    $vm=Get-VM -Id ([guid]'47F1AD41-9621-4513-AA01-1117E8E47B13') -ErrorAction Stop
    $machine.approved_hyperv_vm=($vm.Name + ': ' + $vm.State.ToString())
  } catch {
    $machine.approved_hyperv_vm='Not available to this Windows account'
  }
  WriteText 'system.json' ($machine|ConvertTo-Json -Depth 7)
  $warnings=@(
    'This ZIP contains sanitized local support details. Redaction is best effort; inspect every extracted file before sending.',
    'Never upload Hyper-V disks, ProgramData\\StackDeck\\config, id_ed25519, .env, browser credentials, API tokens, certificates, or a raw Windows user profile.',
    'Do not post this ZIP publicly if it contains confidential company, customer, or private project details.',
    'The script did NOT collect project configuration JSON, state databases, secrets or guest disks.'
  )
  WriteText 'PRIVACY-REVIEW.txt' ($warnings -join [Environment]::NewLine)
  WriteText 'ISSUE-TEMPLATE.txt' @'
StackDeck version:
Windows edition/build:
What I tried to do:
Expected behavior:
Actual behavior / error text:
Time of issue (include timezone):
Steps to reproduce:
Installer: MSI or NSIS (if applicable):
Does restarting the app help?
Any recent Windows or StackDeck update?
I inspected this support ZIP before sharing: Yes / No
'@
  $cutoff=(Get-Date).AddDays(-$Days)
  $included=@()
  if ($IncludeLogs) {
    $roots=@(
      (Join-Path $env:LOCALAPPDATA 'StackDeck\logs'),
      (Join-Path $env:LOCALAPPDATA 'com.stackdeck.desktop\logs'),
      (Join-Path $env:APPDATA 'com.stackdeck.desktop\logs'),
      (Join-Path $env:PROGRAMDATA 'StackDeck\logs')
    )
    $candidates=@()
    foreach($root in $roots) {
      if(Test-Path -LiteralPath $root -PathType Container) {
        $candidates+=@(Get-ChildItem -LiteralPath $root -File -Recurse -ErrorAction SilentlyContinue|
          Where-Object { $_.Extension -in @('.log','.txt') -and $_.LastWriteTime -ge $cutoff -and $_.Length -le 4MB })
      }
    }
    $tempLogs=Get-ChildItem -Path (Join-Path $env:TEMP 'StackDeck*.log') -File -ErrorAction SilentlyContinue
    $candidates+=@($tempLogs|Where-Object {$_.LastWriteTime -ge $cutoff -and $_.Length -le 4MB})
    $candidates=@($candidates|Sort-Object LastWriteTime -Descending|Select-Object -First $MaxLogFiles)
    $index=0
    foreach($f in $candidates) {
      $index++
      $lines=@(Get-Content -LiteralPath $f.FullName -Tail 200 -ErrorAction Stop)
      WriteText ('app-log-'+$index+'.txt') (($lines -join [Environment]::NewLine) -replace '\x00','')
      $included+=@{name=('app-log-'+$index+'.txt');original_extension=$f.Extension;bytes=$f.Length}
    }
  }
  if ($IncludeEvents) {
    $events=SafeResult {
      @(Get-WinEvent -FilterHashtable @{LogName='Application';StartTime=$cutoff} -MaxEvents 500 -ErrorAction Stop|
        Where-Object {$_.ProviderName -match 'StackDeck|MsiInstaller' -and
          ($_.ProviderName -match 'StackDeck' -or $_.Message -match 'StackDeck')}|
        Select-Object -First 30|ForEach-Object {
          'UTC='+$_.TimeCreated.ToUniversalTime().ToString('o')+' ID='+$_.Id+' SOURCE='+$_.ProviderName+[Environment]::NewLine+$_.Message
        })
    }
    WriteText 'windows-events.txt' ($events|Out-String)
  }
  $manifest=[ordered]@{
    created_utc=[datetime]::UtcNow.ToString('o')
    sanitized_application_logs=$included
    logs_count=@($included).Count
    events_included=[bool]$IncludeEvents
    raw_files_or_config_included=$false
    file_digests=@()
  }
  foreach($file in @(Get-ChildItem -LiteralPath $staging -File |Sort-Object Name)) {
    $manifest.file_digests+=@{name=$file.Name;sha256=(Get-FileHash $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()}
  }
  WriteText 'manifest.json' ($manifest|ConvertTo-Json -Depth 8)
  Add-Type -AssemblyName System.IO.Compression.FileSystem
  [IO.Compression.ZipFile]::CreateFromDirectory($staging,$zip,[IO.Compression.CompressionLevel]::Optimal,$false)
  $archive=[IO.Compression.ZipFile]::OpenRead($zip)
  try {
    $names=@($archive.Entries|ForEach-Object {$_.FullName})
    if (@($names).Count -ne @((Get-ChildItem -LiteralPath $staging -File)).Count) {
      throw 'Support ZIP file count does not match sanitized stage.'
    }
    foreach($entry in $archive.Entries) {
      if($entry.FullName -notmatch '^(?:system\.json|PRIVACY-REVIEW\.txt|ISSUE-TEMPLATE\.txt|manifest\.json|windows-events\.txt|app-log-\d+\.txt)$') {
        throw 'Unexpected file inside support ZIP'
      }
      $stream=$entry.Open()
      try { $buffer=New-Object byte[] 16384;while($stream.Read($buffer,0,$buffer.Length) -gt 0){} }
      finally { $stream.Dispose() }
    }
  } finally { $archive.Dispose() }
  $digest=(Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash.ToLowerInvariant()
  [IO.File]::WriteAllText($shaFile,($digest+'  '+[IO.Path]::GetFileName($zip)),[Text.UTF8Encoding]::new($false))
  Write-Output ('STACKDECK_SUPPORT_ZIP='+$zip)
  Write-Output ('STACKDECK_SUPPORT_ZIP_SHA256='+$digest)
  Write-Output ('SANITIZED_LOG_FILES='+@($included).Count)
  Write-Warning 'REVIEW EACH EXTRACTED FILE BEFORE SHARING. ZIP may still contain sensitive context.'
} finally {
  if(Test-Path -LiteralPath $staging) { Remove-Item -LiteralPath $staging -Recurse -Force -ErrorAction SilentlyContinue }
}
