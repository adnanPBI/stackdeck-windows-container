#Requires -Version 5.1
#Requires -RunAsAdministrator
<#
.SYNOPSIS
Read-only evidence of StackDeck recovery after a real Windows host restart.
Requires the approved RND VM; never starts/stops VMs or changes disk state.
.ProbeOnly
Tests the current boot without calling it a reboot acceptance.
#>
[CmdletBinding()]
param([switch]$ProbeOnly)
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
$ops=Join-Path $env:ProgramData 'StackDeck\ops'
$cfgRoot=Join-Path $env:ProgramData 'StackDeck\config'
$install=Join-Path $ops 'recovery-settings.json'
$qualification=Join-Path $ops 'postboot-qualification.json'
$root='D:\Coding Development Files\StackDeck-HyperV-Local-Container\target\release-evidence'
$scriptPath=$PSCommandPath
$expectedVM=[guid]'47F1AD41-9621-4513-AA01-1117E8E47B13'
$taskName='StackDeck-v060-Unattended-DockerAPI'
$hive='C:\Program Files\StackDeck\stackdeck-hive.exe'
$runId=[Guid]::NewGuid().ToString('N').Substring(0,12)
$path=Join-Path $root ('20261019-postboot-'+$(if($ProbeOnly){'dry-run'}else{'actual'})+'-'+$runId+'.json')
$r=[ordered]@{
 utc=[DateTime]::UtcNow.ToString('o')
 principal=[Security.Principal.WindowsIdentity]::GetCurrent().Name
 verification_mode=$(if($ProbeOnly){'current-boot diagnostic only'}else{'requires actual Windows reboot'})
 approved_vm_guid=$expectedVM.ToString()
 baseline_boot_utc=''
 observed_boot_utc=''
 actual_host_restart_observed=$false
 startup_task_at_boot_s4u=$false
 only_approved_vm_running=$false
 guest_containerd_buildkit_ssh_healthy=$false
 api_authenticated_http_200=$false
 api_unauthenticated_http_401=$false
 api_listener_unique=$false
 expected_http_endpoint=''
 expected_endpoint_http_200=$false
 preapproved_endpoint_test_required=$false
 user_unattended_postboot_readiness=$false
 production_reboot_acceptance_pass=$false
 error=''
}
function HttpCode([string]$Url,[string]$Bearer) {
 try {
  $headers=if($Bearer){@{Authorization='Bearer '+$Bearer}}else{@{}}
  $response=Invoke-WebRequest -Uri $Url -Headers $headers -UseBasicParsing -TimeoutSec 3 -ErrorAction Stop
  return [int]$response.StatusCode
 }catch{
  if($_.Exception.Response){return [int]$_.Exception.Response.StatusCode}
  return -1
 }
}
try{
 if(!(Test-Path -LiteralPath $install) -or !(Test-Path -LiteralPath $qualification)){throw 'Postboot qualification not armed'}
 $seal=Get-Content $install -Raw -ErrorAction Stop|ConvertFrom-Json
 $config=Get-Content $qualification -Raw -ErrorAction Stop|ConvertFrom-Json
 if($seal.approved_vm_guid -ne $expectedVM.ToString() -or $config.approved_vm_guid -ne $expectedVM.ToString()){throw 'Approved VM identity mismatch'}
 if((Get-FileHash $scriptPath -Algorithm SHA256).Hash.ToLowerInvariant() -cne [string]$config.script_sha256){throw 'Postboot verifier source changed'}
 if((Get-FileHash $hive -Algorithm SHA256).Hash.ToLowerInvariant() -cne [string]$seal.installed_hive_sha256){throw 'Installed Hive changed'}
 $baseline=[DateTime]::Parse([string]$config.armed_boot_utc).ToUniversalTime()
 $boot=(Get-CimInstance Win32_OperatingSystem -ErrorAction Stop).LastBootUpTime.ToUniversalTime()
 $r.baseline_boot_utc=$baseline.ToString('o')
 $r.observed_boot_utc=$boot.ToString('o')
 $r.actual_host_restart_observed=($boot -gt $baseline.AddSeconds(30))
 $status=Get-ScheduledTask -TaskName $taskName -ErrorAction Stop
 $startup=@($status.Triggers|Where-Object {$_.CimClass.CimClassName -eq 'MSFT_TaskBootTrigger'})
 $r.startup_task_at_boot_s4u=($status.Principal.LogonType.ToString() -eq 'S4U' -and $startup.Count -ge 1)
 if(!$r.startup_task_at_boot_s4u){throw 'S4U startup task configuration was lost'}
 $r.expected_http_endpoint=[string]$config.expected_http_endpoint
 $r.preapproved_endpoint_test_required=([bool]$config.require_expected_endpoint)
 $tokenPath=Join-Path $cfgRoot 'docker-api-token.txt'
 $token=[IO.File]::ReadAllText($tokenPath).Trim()
 if($token.Length -lt 32){throw 'API token absent'}
 $ready=$false
 $tries=$(if($ProbeOnly){4}else{105})
 for($i=0;$i -lt $tries;$i++){
  try{
   $vms=@(Get-VM -ErrorAction Stop)
   $r.only_approved_vm_running=($vms.Count -eq 1 -and $vms[0].Id -eq $expectedVM -and $vms[0].Name -eq 'stackdeck-linux' -and $vms[0].State.ToString() -eq 'Running')
   if($r.only_approved_vm_running){
    $prior=$ErrorActionPreference
    try{
     $ErrorActionPreference='Continue'
     $health=(& $hive hyperv health 2>&1 |Out-String)
     $exit=$LASTEXITCODE
    }finally{$ErrorActionPreference=$prior}
    $r.guest_containerd_buildkit_ssh_healthy=($exit -eq 0 -and $health.Contains('reachable') -and $health.Contains('containerd') -and $health.Contains('buildkit'))
    $api='http://127.0.0.1:23750/_ping'
    $r.api_authenticated_http_200=((HttpCode $api $token) -eq 200)
    $r.api_unauthenticated_http_401=((HttpCode $api '') -eq 401)
    $r.api_listener_unique=(@(Get-NetTCPConnection -LocalPort 23750 -State Listen -ErrorAction SilentlyContinue).Count -eq 1)
    if($r.preapproved_endpoint_test_required){
     $r.expected_endpoint_http_200=((HttpCode $r.expected_http_endpoint '') -eq 200)
    }else{$r.expected_endpoint_http_200=$true}
    $ready=($r.startup_task_at_boot_s4u -and $r.only_approved_vm_running -and $r.guest_containerd_buildkit_ssh_healthy -and $r.api_authenticated_http_200 -and $r.api_unauthenticated_http_401 -and $r.api_listener_unique -and $r.expected_endpoint_http_200)
    if($ready){break}
   }
  }catch{}
  if($i -lt $tries-1){Start-Sleep -Seconds $(if($ProbeOnly){2}else{5})}
 }
 $r.user_unattended_postboot_readiness=$ready
 $r.production_reboot_acceptance_pass=($ready -and $r.actual_host_restart_observed -and -not $ProbeOnly)
}catch{
 $r.error=$_.Exception.Message
}finally{
 if(!(Test-Path $root -PathType Container)){throw 'Evidence output directory missing'}
 [IO.File]::WriteAllText($path,($r|ConvertTo-Json -Depth 6),[Text.UTF8Encoding]::new($false))
 Write-Output ('POSTBOOT_DIAGNOSTIC_READY='+$r.user_unattended_postboot_readiness)
 Write-Output ('ACTUAL_HOST_RESTART_OBSERVED='+$r.actual_host_restart_observed)
 Write-Output ('PRODUCTION_REBOOT_ACCEPTANCE_PASS='+$r.production_reboot_acceptance_pass)
 Write-Output ('EVIDENCE='+$path)
}
if($ProbeOnly){if(!$r.user_unattended_postboot_readiness){exit 1}}elseif(!$r.production_reboot_acceptance_pass){exit 1}
