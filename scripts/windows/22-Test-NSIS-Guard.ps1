#Requires -Version 5.1
<#
.SYNOPSIS
Non-destructive NSIS legacy-install guard test. Compiles the REAL hook in
a tiny temporary installer. Does not run a StackDeck MSI/NSIS or touch a VM.
#>
[CmdletBinding()]
param([string]$Output="")
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
$repo=(Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..\..')).ProviderPath
$sha=(git -C $repo rev-parse HEAD |Out-String).Trim()
$hook=Join-Path $repo 'desktop\stackdeck\src-tauri\windows\hooks.nsh'
if(!(Test-Path $hook -PathType Leaf)){throw 'Legacy guard source missing'}
$makensis=Get-Command makensis.exe -CommandType Application -ErrorAction SilentlyContinue|Select-Object -First 1
if(!$makensis){
 $p=Join-Path $env:LOCALAPPDATA 'tauri\NSIS\makensis.exe'
 if(Test-Path $p -PathType Leaf){$makensis=Get-Item $p}
}
if(!$makensis){throw 'NSIS compiler unavailable; build the bundle before running this test'}
$key='HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\com.stackdeck.desktop'
if(Test-Path -LiteralPath $key){throw 'Real historical registration exists; test cannot replace it'}
$folder=Join-Path ([IO.Path]::GetTempPath()) ('StackDeck-NSIS-GuardQA-'+[Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $folder -ErrorAction Stop|Out-Null
$nsi=Join-Path $folder 'test-legacy-guard.nsi'
$exe=Join-Path $folder 'test-legacy-guard.exe'
$marker=Join-Path $folder 'allowed.txt'
$token=[Guid]::NewGuid().ToString('N')
$record=[ordered]@{
 utc=[datetime]::UtcNow.ToString('o');source=$sha
 hook_sha256=(Get-FileHash $hook -Algorithm SHA256).Hash.ToLowerInvariant()
 registered_fake_current_user_install=$false
 simulated_legacy_rejected=$false
 clean_current_user_allowed=$false
 original_registry_preserved=$false
 touched_vm=$false
 touched_installed_stackdeck=$false
 compile_exit=-1;blocked_exit=-1;clean_exit=-1;passed=$false;error=''
}
$snippet=@'
Unicode true
Name "StackDeck-NSIS-Hook-Only-QA"
OutFile "__OUTPUT__"
InstallDir "$TEMP"
RequestExecutionLevel user
!include "__HOOK__"
Section
  !insertmacro NSIS_HOOK_PREINSTALL
  FileOpen $R1 "__MARKER__" w
  FileWrite $R1 "guard-allowed"
  FileClose $R1
SectionEnd
'@
$content=$snippet.Replace('__HOOK__',$hook).Replace('__OUTPUT__',$exe).Replace('__MARKER__',$marker)
[IO.File]::WriteAllText($nsi,$content,[Text.UTF8Encoding]::new($false))
function RunHarness {
 param([string]$Path)
 $p=Start-Process -FilePath $Path -ArgumentList '/S' -PassThru -ErrorAction Stop
 if(!$p.WaitForExit(20000)) {
  try{$p.Kill()}catch{}
  throw 'NSIS QA harness hung'
 }
 $p.Refresh()
 return [int]$p.ExitCode
}
try{
 & $makensis.FullName /V2 $nsi |Out-Null
 $record.compile_exit=$LASTEXITCODE
 if($record.compile_exit -ne 0 -or !(Test-Path $exe)){throw 'Guard harness cannot compile'}
 $null=New-Item -Path $key -ErrorAction Stop
 $null=New-ItemProperty -LiteralPath $key -Name StackDeckGuardQAMarker -Value $token -PropertyType String
 $null=New-ItemProperty -LiteralPath $key -Name DisplayName -Value 'StackDeck' -PropertyType String
 $record.registered_fake_current_user_install=$true
 $record.blocked_exit=RunHarness $exe
 $record.simulated_legacy_rejected=($record.blocked_exit -ne 0 -and !(Test-Path $marker))
 $saved=Get-ItemProperty -LiteralPath $key -ErrorAction Stop
 if($saved.StackDeckGuardQAMarker -ne $token){throw 'QA marker changed; leave registry key for review'}
 Remove-Item -LiteralPath $key -ErrorAction Stop
 $record.clean_exit=RunHarness $exe
 $record.clean_current_user_allowed=($record.clean_exit -eq 0 -and (Test-Path $marker))
}catch{$record.error=$_.Exception.ToString()}
finally{
 try{
  if(Test-Path -LiteralPath $key){
   $check=Get-ItemProperty -LiteralPath $key -ErrorAction Stop
   if($check.StackDeckGuardQAMarker -eq $token){
    Remove-Item -LiteralPath $key -ErrorAction Stop
   }else{throw 'Unexpected registry change, refusing delete'}
  }
  $record.original_registry_preserved=!(Test-Path $key)
 }catch{$record.error+=(' CLEANUP='+$_.Exception.Message)}
 $record.passed=($record.compile_exit -eq 0 -and $record.simulated_legacy_rejected -and
   $record.clean_current_user_allowed -and $record.original_registry_preserved -and !$record.error)
 if($Output){
  $full=[IO.Path]::GetFullPath($Output)
  if(Test-Path -LiteralPath $full){throw 'QA evidence destination exists'}
  [IO.File]::WriteAllText($full,($record|ConvertTo-Json -Depth 6),[Text.UTF8Encoding]::new($false))
 }
 if(Test-Path -LiteralPath $folder){Remove-Item -LiteralPath $folder -Recurse -Force -ErrorAction SilentlyContinue}
}
Write-Output ('NSIS_GUARD_UNIT_TEST_PASS='+$record.passed+' DENY_EXIT='+$record.blocked_exit+' ALLOW_EXIT='+$record.clean_exit)
if(!$record.passed){throw ('NSIS guard test failed: '+$record.error)}
