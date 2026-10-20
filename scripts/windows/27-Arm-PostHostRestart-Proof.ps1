#Requires -Version 5.1
#Requires -RunAsAdministrator
<#
.SYNOPSIS
Arm read-only postboot acceptance before a separately authorized host restart.
It NEVER restarts Windows, changes the VM, or logs any API bearer token.
#>
[CmdletBinding()]
param()
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
$r='D:\Coding Development Files\StackDeck-HyperV-Local-Container'
$e=Join-Path $r 'target\release-evidence'
$ops=Join-Path $env:ProgramData 'StackDeck\ops'
$target=Join-Path $ops 'PostBoot-Readiness.ps1'
$source=Join-Path $r 'scripts\windows\26-Verify-PostHostRestart.ps1'
$settings=Join-Path $ops 'postboot-qualification.json'
$report=Join-Path $e '20261019-postboot-evidence-task-armed.json'
$name='StackDeck-v060-PostBoot-Readiness'
$apiTaskName='StackDeck-v060-Unattended-DockerAPI'
$guid=[guid]'47F1AD41-9621-4513-AA01-1117E8E47B13'
if(Test-Path $report){throw 'Postboot arming report already exists'}
if((Test-Path $target) -or (Test-Path $settings)){throw 'Postboot settings already exist; do not overwrite'}
if(Get-ScheduledTask -TaskName $name -ErrorAction SilentlyContinue){throw 'Postboot task name already registered'}
$v=@(Get-VM -ErrorAction Stop)
if($v.Count -ne 1 -or $v[0].Id -ne $guid -or $v[0].Name -ne 'stackdeck-linux' -or $v[0].State -ne 'Running'){throw 'Approved VM-only guard'}
$apiTask=Get-ScheduledTask -TaskName $apiTaskName -ErrorAction Stop
if($apiTask.State -ne 'Running' -or $apiTask.Principal.LogonType.ToString() -ne 'S4U'){throw 'Existing S4U startup task must be active'}
$code=$null;$errors=$null
[Management.Automation.Language.Parser]::ParseFile($source,[ref]$code,[ref]$errors)|Out-Null
if($errors.Count){throw 'Postboot source has syntax errors'}
$boot=(Get-CimInstance Win32_OperatingSystem -ErrorAction Stop).LastBootUpTime.ToUniversalTime()
$hash=(Get-FileHash $source -Algorithm SHA256).Hash.ToLowerInvariant()
$httpEndpoint='http://127.0.0.1:18080/'
$httpOk=$false
try{$resp=Invoke-WebRequest -Uri $httpEndpoint -UseBasicParsing -TimeoutSec 3 -ErrorAction Stop;$httpOk=($resp.StatusCode -eq 200)}catch{}
$registration=[ordered]@{
 utc=[DateTime]::UtcNow.ToString('o')
 task_name=$name
 approved_vm_guid=$guid.ToString()
 task_principal='RND\NHTML'
 task_logon_type='S4U'
 trigger='AtStartup'
 armed_boot_utc=$boot.ToString('o')
 script_sha256=$hash
 require_expected_endpoint=$httpOk
 expected_http_endpoint=$(if($httpOk){$httpEndpoint}else{''})
 actual_host_restart_observed=$false
 verified_until_reboot=$false
 task_registration_pass=$false
 failure=''
}
$hasTask=$false;$hasScript=$false;$hasSettings=$false
try{
 Copy-Item -LiteralPath $source -Destination $target -ErrorAction Stop
 $hasScript=$true
 $cfg=[ordered]@{approved_vm_guid=$guid.ToString();armed_boot_utc=$boot.ToString('o');script_sha256=$hash;require_expected_endpoint=$httpOk;expected_http_endpoint=$(if($httpOk){$httpEndpoint}else{''});source_commit=(git -C $r rev-parse HEAD).Trim()}
 [IO.File]::WriteAllText($settings,($cfg|ConvertTo-Json -Depth 5),[Text.UTF8Encoding]::new($false))
 $hasSettings=$true
 $a=New-ScheduledTaskAction -Execute 'powershell.exe' -Argument ('-NoProfile -NonInteractive -ExecutionPolicy Bypass -File "'+$target+'"')
 $t=New-ScheduledTaskTrigger -AtStartup
 $p=New-ScheduledTaskPrincipal -UserId 'RND\NHTML' -LogonType S4U -RunLevel Highest
 $s=New-ScheduledTaskSettingsSet -ExecutionTimeLimit (New-TimeSpan -Minutes 15) -RestartCount 1 -RestartInterval (New-TimeSpan -Minutes 2) -StartWhenAvailable -MultipleInstances IgnoreNew
 Register-ScheduledTask -TaskName $name -Action $a -Trigger $t -Principal $p -Settings $s -Description 'StackDeck RND post-host-restart read-only verification; NO automatic reboot' -ErrorAction Stop|Out-Null
 $hasTask=$true
 $t=Get-ScheduledTask -TaskName $name -ErrorAction Stop
 $registration.task_registration_pass=($t.Principal.LogonType.ToString() -eq 'S4U' -and @($t.Triggers|Where-Object {$_.CimClass.CimClassName -eq 'MSFT_TaskBootTrigger'}).Count -eq 1 -and (Get-FileHash $target -Algorithm SHA256).Hash.ToLowerInvariant() -eq $hash)
 if(!$registration.task_registration_pass){throw 'Registered startup task failed integrity check'}
 $registration.verified_until_reboot=$true
}catch{
 $registration.failure=$_.Exception.Message
 if($hasTask){try{Unregister-ScheduledTask -TaskName $name -Confirm:$false -ErrorAction Stop}catch{}}
 if($hasSettings){try{Remove-Item -LiteralPath $settings -Force -ErrorAction Stop}catch{}}
 if($hasScript){try{Remove-Item -LiteralPath $target -Force -ErrorAction Stop}catch{}}
}finally{
 [IO.File]::WriteAllText($report,($registration|ConvertTo-Json -Depth 6),[Text.UTF8Encoding]::new($false))
 Write-Output ('POSTBOOT_VERIFIER_ARMED='+$registration.task_registration_pass)
 Write-Output ('HOST_RESTART_NOT_PERFORMED=True')
 Write-Output ('EVIDENCE='+$report)
}
if(!$registration.task_registration_pass){exit 1}
