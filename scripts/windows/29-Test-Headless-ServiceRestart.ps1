#Requires -Version 5.1
#Requires -RunAsAdministrator
[CmdletBinding()]
param([string]$Output='D:\Coding Development Files\StackDeck-HyperV-Local-Container\target\release-evidence\20261019-headless-api-task-restart.json')
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
$task='StackDeck-v060-Unattended-DockerAPI'
$old='StackDeck Local Docker API'
$vmid=[guid]'47F1AD41-9621-4513-AA01-1117E8E47B13'
if(Test-Path $Output){throw 'Restart evidence exists; do not replay unconditionally'}
$root='C:\ProgramData\StackDeck\config'
$tokenPath=Join-Path $root 'docker-api-token.txt'
$api='http://127.0.0.1:23750/_ping'
$taskItem=Get-ScheduledTask -TaskName $task -ErrorAction Stop
$legacy=Get-ScheduledTask -TaskName $old -ErrorAction Stop
$vm=@(Get-VM -ErrorAction Stop)
if($vm.Count -ne 1 -or $vm[0].Id -ne $vmid -or $vm[0].State -ne 'Running'){throw 'Only approved running VM allowed'}
if($taskItem.State -ne 'Running' -or $taskItem.Principal.UserId -notin @('RND\NHTML','NHTML') -or $taskItem.Principal.LogonType.ToString() -ne 'S4U'){throw 'Original S4U task not running, no test'}
if($legacy.State -ne 'Disabled'){throw 'Legacy logon task not disabled'}
$token=[IO.File]::ReadAllText($tokenPath).Trim()
if($token.Length -lt 32){throw 'API token invalid'}
$cfg='C:\ProgramData\StackDeck\config\hyperv.json'
$cfgHash=(Get-FileHash $cfg -Algorithm SHA256).Hash.ToLowerInvariant()
function ApiStatus([bool]$Authenticated){
 try {
  $headers=if($Authenticated){@{Authorization='Bearer '+$token}}else{@{}}
  $res=Invoke-WebRequest -Uri $api -Headers $headers -UseBasicParsing -TimeoutSec 4 -ErrorAction Stop
  return [int]$res.StatusCode
 }catch{
  if($_.Exception.Response){return [int]$_.Exception.Response.StatusCode}
  return -1
 }
}
$r=[ordered]@{utc=[DateTime]::UtcNow.ToString('o');task=$task;service_mode='AtStartup S4U';before_auth_http200=$false;before_unauth_401=$false;stopped_own_task=$false;api_stopped_with_task=$false;started_own_task_again=$false;after_auth_http200=$false;after_unauth_401=$false;vm_preserved=$false;config_preserved=$false;passed=$false;errors=@()}
try{
 $r.before_auth_http200=((ApiStatus $true) -eq 200)
 $r.before_unauth_401=((ApiStatus $false) -eq 401)
 if(!$r.before_auth_http200 -or !$r.before_unauth_401){throw 'API was not qualified before controlled restart'}
 Stop-ScheduledTask -TaskName $task -ErrorAction Stop
 for($i=0;$i -lt 25;$i++){
  Start-Sleep -Seconds 1
  $state=(Get-ScheduledTask -TaskName $task).State.ToString()
  $port=@(Get-NetTCPConnection -State Listen -LocalPort 23750 -ErrorAction SilentlyContinue)
  if($state -ne 'Running' -and $port.Count -eq 0){$r.stopped_own_task=$true;$r.api_stopped_with_task=$true;break}
 }
 if(!$r.api_stopped_with_task){throw 'Task did not release its API listener, refusing duplicate startup'}
 Start-ScheduledTask -TaskName $task -ErrorAction Stop
 $r.started_own_task_again=$true
 for($i=0;$i -lt 65;$i++){
  Start-Sleep -Seconds 1
  $state=(Get-ScheduledTask -TaskName $task).State.ToString()
  if($state -eq 'Running'){
   $r.after_auth_http200=((ApiStatus $true) -eq 200)
   $r.after_unauth_401=((ApiStatus $false) -eq 401)
   if($r.after_auth_http200 -and $r.after_unauth_401){break}
  }
 }
}catch{$r.errors+= $_.Exception.Message}
finally{
 try{
  $v=@(Get-VM -ErrorAction Stop)
  $r.vm_preserved=($v.Count -eq 1 -and $v[0].Id -eq $vmid -and $v[0].State -eq 'Running')
  $r.config_preserved=((Get-FileHash $cfg -Algorithm SHA256).Hash.ToLowerInvariant() -eq $cfgHash)
 }catch{$r.errors+=('postcheck: '+$_.Exception.Message)}
 $r.passed=[bool]($r.before_auth_http200 -and $r.before_unauth_401 -and $r.stopped_own_task -and $r.api_stopped_with_task -and $r.started_own_task_again -and $r.after_auth_http200 -and $r.after_unauth_401 -and $r.vm_preserved -and $r.config_preserved -and @($r.errors).Count -eq 0)
 [IO.File]::WriteAllText($Output,($r|ConvertTo-Json -Depth 6),[Text.UTF8Encoding]::new($false))
 Write-Output ('HEADLESS_API_TASK_RESTART_PASS='+$r.passed)
}
if(!$r.passed){exit 1}
