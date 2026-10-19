#Requires -Version 5.1
#Requires -RunAsAdministrator
[CmdletBinding()]
param()
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
$repo=(Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..\..')).ProviderPath
$e=Join-Path $repo 'target\release-evidence'
$report=Join-Path $e '20261019-unattended-host-recovery-s4u-install.json'
$taskName='StackDeck-v060-Unattended-DockerAPI'
$legacyTask='StackDeck Local Docker API'
$ops=Join-Path $env:ProgramData 'StackDeck\ops'
$scriptTarget=Join-Path $ops 'Unattended-HostRecovery.ps1'
$settingsPath=Join-Path $ops 'recovery-settings.json'
$tokenPath=Join-Path $env:ProgramData 'StackDeck\config\docker-api-token.txt'
$vmGuid=[guid]'47F1AD41-9621-4513-AA01-1117E8E47B13'
$hive='C:\Program Files\StackDeck\stackdeck-hive.exe'
$hiveSha='a7dd15e2cb9a361cdddf80a188af88e47ae54374e048c6223a367ba409fad35d'
$source=Join-Path $PSScriptRoot '24-Run-Unattended-HostRecovery.ps1'
if(Test-Path $report){throw 'Acceptance already attempted, refusing replay'}
if(Get-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue){throw 'Unattended task already registered; no overwrite'}
if((Get-FileHash $hive -Algorithm SHA256).Hash.ToLowerInvariant() -ne $hiveSha){throw 'Installed binary hash drift'}
$v=@(Get-VM -ErrorAction Stop)
if($v.Count -ne 1 -or $v[0].Id -ne $vmGuid -or $v[0].Name -ne 'stackdeck-linux' -or $v[0].State -ne 'Running'){throw 'Approved VM-only guard'}
if($v[0].AutomaticStartAction.ToString() -ne 'StartIfRunning' -or $v[0].AutomaticStopAction.ToString() -ne 'Save'){throw 'Host auto-start/save settings not eligible'}
if(@(Get-NetTCPConnection -LocalPort 23750 -State Listen -ErrorAction SilentlyContinue).Count){throw 'Existing 23750 listener, will not replace'}
$oldTask=Get-ScheduledTask -TaskName $legacyTask -ErrorAction SilentlyContinue
if(!$oldTask){throw 'Legacy logon task missing; preserve baseline before modification'}
if($oldTask.State -eq 'Running'){throw 'Legacy API task active; stop it separately and collect its state before cutover'}
if(Test-Path $scriptTarget){
 if((Get-FileHash $scriptTarget -Algorithm SHA256).Hash -ne (Get-FileHash $source -Algorithm SHA256).Hash){throw 'Protected existing startup script differs from reviewed source'}
}
if(Test-Path $settingsPath){
 $existing=Get-Content $settingsPath -Raw|ConvertFrom-Json
 if($existing.approved_vm_guid -ne $vmGuid.ToString() -or $existing.installed_hive_sha256 -ne $hiveSha){throw 'Existing protected startup settings conflict'}
}
$t=$null;$errs=$null
[Management.Automation.Language.Parser]::ParseFile($source,[ref]$t,[ref]$errs)|Out-Null
if($errs.Count){throw 'Runner script syntax error'}
$createdToken=$false;$registered=$false;$passed=$false;$legacyDisabled=$false
$result=[ordered]@{
 utc=[DateTime]::UtcNow.ToString('o')
 task=$taskName
 service_principal='RND\NHTML'
 trigger='AtStartup';logon_type='S4U (no interactive sign-in required)'
 security='loopback 127.0.0.1:23750, bearer-token mandatory'
 vm_guid=$vmGuid.ToString()
 vm_preboot_state='Running'
 automatic_start='StartIfRunning'
 automatic_stop='Save'
 prior_logon_task=$legacyTask
 installed_hive_sha256=$hiveSha
 protected_script_sha256=''
 system_task_running=$false
 system_runtime_health_pass=$false
 authenticated_ping_200=$false
 unauthorized_ping_401=$false
 real_host_restart_observed=$false
 legacy_logon_task_disabled=$false
 success=$false
 error=''
 rollback_error=''
}
function Set-PrivateOpsACL([string]$Path) {
 $a=New-Object Security.AccessControl.DirectorySecurity
 $a.SetAccessRuleProtection($true,$false)
 foreach($sidText in @('S-1-5-18','S-1-5-32-544')){
  $sid=New-Object Security.Principal.SecurityIdentifier($sidText)
  $rule=New-Object Security.AccessControl.FileSystemAccessRule($sid,'FullControl','ContainerInherit, ObjectInherit','None','Allow')
  $a.AddAccessRule($rule)
 }
 [IO.Directory]::SetAccessControl($Path,$a)
}
function HttpStatus([string]$Token){
 try{
  $header=if($Token){@{Authorization='Bearer '+$Token}}else{@{}}
  $resp=Invoke-WebRequest -Uri 'http://127.0.0.1:23750/_ping' -Headers $header -UseBasicParsing -TimeoutSec 4 -ErrorAction Stop
  return [int]$resp.StatusCode
 }catch{
  if($_.Exception.Response){return [int]$_.Exception.Response.StatusCode}
  return -1
 }
}
try {
 [IO.Directory]::CreateDirectory($ops)|Out-Null
 Set-PrivateOpsACL $ops
 $rollback=Join-Path $ops 'legacy-api-task-pre-unattended.xml'
 if(!(Test-Path $rollback)){
  $xml=Export-ScheduledTask -TaskName $legacyTask -ErrorAction Stop
  [IO.File]::WriteAllText($rollback,$xml,[Text.UTF8Encoding]::new($false))
 }
 if(!(Test-Path $scriptTarget)){Copy-Item -LiteralPath $source -Destination $scriptTarget -ErrorAction Stop}
 $result.protected_script_sha256=(Get-FileHash $scriptTarget -Algorithm SHA256).Hash.ToLowerInvariant()
 if(!(Test-Path $tokenPath -PathType Leaf)){
  $bytes=New-Object byte[] 32
  $rng=[Security.Cryptography.RandomNumberGenerator]::Create()
  try{$rng.GetBytes($bytes)}finally{$rng.Dispose()}
  $token=[Convert]::ToBase64String($bytes)
  [IO.File]::WriteAllText($tokenPath,$token,[Text.Encoding]::ASCII)
  $createdToken=$true
 }else{$token=[IO.File]::ReadAllText($tokenPath).Trim()}
 if($token.Length -lt 32){throw 'Existing token invalid'}
 $a=New-Object Security.AccessControl.FileSecurity
 $a.SetAccessRuleProtection($true,$false)
 foreach($sidText in @('S-1-5-18','S-1-5-32-544')){
  $sid=New-Object Security.Principal.SecurityIdentifier($sidText)
  [void]$a.AddAccessRule((New-Object Security.AccessControl.FileSystemAccessRule($sid,'FullControl','Allow')))
 }
 [IO.File]::SetAccessControl($tokenPath,$a)
 $boot=(Get-CimInstance Win32_OperatingSystem).LastBootUpTime.ToUniversalTime()
 $cfg=[ordered]@{approved_vm_guid=$vmGuid.ToString();installed_hive_sha256=$hiveSha;baseline_boot_utc=$boot.ToString('o');installed_utc=[DateTime]::UtcNow.ToString('o');source_script_sha256=$result.protected_script_sha256}
 if(!(Test-Path $settingsPath)){[IO.File]::WriteAllText($settingsPath,($cfg|ConvertTo-Json -Depth 5),[Text.UTF8Encoding]::new($false))}
 $action=New-ScheduledTaskAction -Execute 'powershell.exe' -Argument ('-NoProfile -NonInteractive -ExecutionPolicy Bypass -File "'+$scriptTarget+'"') -WorkingDirectory 'C:\Program Files\StackDeck'
 $trigger=New-ScheduledTaskTrigger -AtStartup
 $principal=New-ScheduledTaskPrincipal -UserId 'RND\NHTML' -LogonType S4U -RunLevel Highest
 $settings=New-ScheduledTaskSettingsSet -ExecutionTimeLimit ([TimeSpan]::Zero) -RestartCount 3 -RestartInterval (New-TimeSpan -Minutes 1) -StartWhenAvailable -MultipleInstances IgnoreNew -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries
 Register-ScheduledTask -TaskName $taskName -Action $action -Trigger $trigger -Principal $principal -Settings $settings -Description 'StackDeck approved-VM guarded localhost Docker API startup without user login' -ErrorAction Stop|Out-Null
 $registered=$true
 Start-ScheduledTask -TaskName $taskName -ErrorAction Stop
 $lastPath=Join-Path $ops 'last-host-recovery.json'
 for($i=0;$i -lt 80;$i++){
  Start-Sleep -Seconds 1
  $task=Get-ScheduledTask -TaskName $taskName -ErrorAction Stop
  if(Test-Path $lastPath){
   $last=Get-Content $lastPath -Raw|ConvertFrom-Json
   if($last.executor -eq 'RND\NHTML' -and $last.completed_validation){$result.system_runtime_health_pass=$true}
  }
  if($task.State -eq 'Running' -and $result.system_runtime_health_pass){
   $result.unauthorized_ping_401=((HttpStatus '') -eq 401)
   $result.authenticated_ping_200=((HttpStatus $token) -eq 200)
   if($result.unauthorized_ping_401 -and $result.authenticated_ping_200){$result.system_task_running=$true;break}
  }
 }
 if(!$result.system_task_running){throw 'S4U startup task did not pass authenticated API and VM health qualification'}
 Disable-ScheduledTask -TaskName $legacyTask -ErrorAction Stop|Out-Null
 $legacyDisabled=$true
 $result.legacy_logon_task_disabled=$true
 $result.success=$true
 $passed=$true
}catch{
 $result.error=$_.Exception.ToString()
 if($registered){
  try{Stop-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue}catch{}
  try{Unregister-ScheduledTask -TaskName $taskName -Confirm:$false -ErrorAction Stop}catch{$result.rollback_error+=' task:'+ $_.Exception.Message}
 }
 if($legacyDisabled){
  try{Enable-ScheduledTask -TaskName $legacyTask -ErrorAction Stop|Out-Null}catch{$result.rollback_error+=' legacy:'+ $_.Exception.Message}
 }
 if($createdToken){
  try{Remove-Item -LiteralPath $tokenPath -Force -ErrorAction Stop}catch{$result.rollback_error+=' token:'+ $_.Exception.Message}
 }
}finally{
 $lastPath=Join-Path $ops 'last-host-recovery.json'
 if(Test-Path $lastPath){
  $s=Get-Content $lastPath -Raw|ConvertFrom-Json
  $result.real_host_restart_observed=[bool]$s.post_real_host_restart
 }
 [IO.File]::WriteAllText($report,($result|ConvertTo-Json -Depth 8),[Text.UTF8Encoding]::new($false))
 Write-Output ('UNATTENDED_SYSTEM_API_STARTUP_SIMULATION_PASS='+$passed)
 Write-Output ('ACTUAL_HOST_RESTART_OBSERVED='+$result.real_host_restart_observed)
 Write-Output ('QUALIFICATION_REPORT='+$report)
}
if(!$passed){exit 1}
