#Requires -Version 5.1
<#
.SYNOPSIS
Synthetic-secret regression test for Windows support collector.
No real customer logs, credentials, VM state or app settings are written.
#>
[CmdletBinding()]
param([string]$Output="")
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
$repo=(Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..\..')).ProviderPath
$collector=Join-Path $PSScriptRoot '21-Collect-StackDeckSupport.ps1'
if(!(Test-Path $collector -PathType Leaf)){throw 'Collector missing'}
$root=Join-Path $env:TEMP ('StackDeck-Support-QA-'+[Guid]::NewGuid().ToString('N'))
$syntheticLog=Join-Path $env:TEMP ('StackDeckQA-'+[Guid]::NewGuid().ToString('N')+'.log')
$secret='SyntheticOnlyNeverPublishS3cr3t_948270'
$record=[ordered]@{
 utc=[DateTime]::UtcNow.ToString('o')
 source_commit=(git -C $repo rev-parse HEAD|Out-String).Trim()
 collector_sha256=(Get-FileHash $collector -Algorithm SHA256).Hash.ToLowerInvariant()
 no_synthetic_secret=$false
 no_private_ipv4=$false
 no_email=$false
 no_unredacted_windows_home=$false
 preserves_useful_error=$false
 archive_integrity=$false
 passed=$false
 error=''
}
New-Item -ItemType Directory -Path $root -ErrorAction Stop|Out-Null
try{
 [IO.File]::WriteAllLines($syntheticLog,@(
   ('SM_API_KEY='+$secret),
   ('Authorization: Bearer '+$secret),
   'C:\Users\SyntheticPerson\AppData\Local\StackDeck',
   '192.168.55.77',
   'synthetic.person@example.com',
   'ordinary containerd error: socket unavailable'
 ),[Text.UTF8Encoding]::new($false))
 & $collector -OutputDirectory $root -IncludeLogs -MaxLogFiles 1 -Days 1 | Out-Null
 if(!$?){throw 'Support collector returned failure'}
 $archives=@(Get-ChildItem -LiteralPath $root -Filter '*.zip' -File)
 if($archives.Count -ne 1){throw 'Expected exactly one support archive'}
 Add-Type -AssemblyName System.IO.Compression.FileSystem
 $archive=[IO.Compression.ZipFile]::OpenRead($archives[0].FullName)
 try{
  $strings=@()
  foreach($entry in $archive.Entries){
   $stream=$entry.Open()
   $reader=[IO.StreamReader]::new($stream)
   try{$strings+= $reader.ReadToEnd()}finally{$reader.Dispose();$stream.Dispose()}
  }
  $all=$strings -join [Environment]::NewLine
  $record.no_synthetic_secret=(!$all.Contains($secret))
  $record.no_private_ipv4=(!$all.Contains('192.168.55.77'))
  $record.no_email=(!$all.Contains('synthetic.person@example.com'))
  $record.no_unredacted_windows_home=(!$all.Contains('C:\Users\SyntheticPerson'))
  $record.preserves_useful_error=$all.Contains('ordinary containerd error: socket unavailable')
  $record.archive_integrity=($archive.Entries.Count -ge 5)
 }finally{$archive.Dispose()}
 $record.passed=($record.no_synthetic_secret -and $record.no_private_ipv4 -and $record.no_email -and
   $record.no_unredacted_windows_home -and $record.preserves_useful_error -and $record.archive_integrity)
}catch{$record.error=$_.Exception.ToString()}
finally{
 if(Test-Path $syntheticLog){Remove-Item -LiteralPath $syntheticLog -Force -ErrorAction SilentlyContinue}
 if(Test-Path $root){Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue}
 if($Output){
  $target=$ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($Output)
  if(Test-Path $target){throw 'QA output exists; refusing overwrite'}
  [IO.File]::WriteAllText($target,($record|ConvertTo-Json -Depth 6),[Text.UTF8Encoding]::new($false))
 }
}
Write-Output ('SUPPORT_COLLECTOR_REDACTION_QA_PASS='+$record.passed)
if(!$record.passed){throw ('Collector QA failed '+$record.error)}
