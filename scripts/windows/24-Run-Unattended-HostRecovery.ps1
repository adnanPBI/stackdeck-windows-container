#Requires -Version 5.1
#Requires -RunAsAdministrator
# Protected S4U startup entrypoint for RND; only the approved backed-up VM.
# Never creates/deletes a VM and never copies the original user SSH identity.
[CmdletBinding()]
param()
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
# Keep the API child in a Windows Job Object. Stopping this scheduled task
# closes the only job handle and terminates its child before the task exits.
Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
public static class StackDeckRecoveryJobV1 {
 [StructLayout(LayoutKind.Sequential)]
 public struct BasicLimits {
  public long ProcessTime;
  public long JobTime;
  public uint Flags;
  public UIntPtr MinWorkingSet;
  public UIntPtr MaxWorkingSet;
  public uint ActiveProcessLimit;
  public UIntPtr Affinity;
  public uint PriorityClass;
  public uint SchedulingClass;
 }
 [StructLayout(LayoutKind.Sequential)]
 public struct IoCounter {
  public ulong ReadOps, WriteOps, OtherOps, ReadBytes, WriteBytes, OtherBytes;
 }
 [StructLayout(LayoutKind.Sequential)]
 public struct ExtendedLimits {
  public BasicLimits Basic;
  public IoCounter Io;
  public UIntPtr ProcessMemory, JobMemory, PeakProcessMemory, PeakJobMemory;
 }
 [DllImport("kernel32.dll", CharSet=CharSet.Unicode,SetLastError=true)]
 private static extern IntPtr CreateJobObject(IntPtr attributes, string name);
 [DllImport("kernel32.dll", SetLastError=true)]
 private static extern bool SetInformationJobObject(IntPtr job, int infoClass, IntPtr info, uint size);
 [DllImport("kernel32.dll", SetLastError=true)]
 public static extern bool AssignProcessToJobObject(IntPtr job, IntPtr process);
 [DllImport("kernel32.dll", SetLastError=true)]
 public static extern bool CloseHandle(IntPtr handle);
 public static IntPtr CreateKillOnCloseJob() {
  IntPtr job=CreateJobObject(IntPtr.Zero,null);
  if(job==IntPtr.Zero) throw new Win32Exception(Marshal.GetLastWin32Error());
  int size=Marshal.SizeOf(typeof(ExtendedLimits));
  IntPtr buffer=Marshal.AllocHGlobal(size);
  try {
   ExtendedLimits limits=new ExtendedLimits();
   limits.Basic.Flags=0x00002000; // JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
   Marshal.StructureToPtr(limits,buffer,false);
   if(!SetInformationJobObject(job,9,buffer,(uint)size))
     throw new Win32Exception(Marshal.GetLastWin32Error());
   return job;
  } catch {
   CloseHandle(job); throw;
  } finally { Marshal.FreeHGlobal(buffer); }
 }
}
'@ -ErrorAction Stop

$ops=Join-Path $env:ProgramData 'StackDeck\ops'
$cfgRoot=Join-Path $env:ProgramData 'StackDeck\config'
$settingsPath=Join-Path $ops 'recovery-settings.json'
$last=Join-Path $ops 'last-host-recovery.json'
$expectedVM=[guid]'47F1AD41-9621-4513-AA01-1117E8E47B13'
$status=[ordered]@{
 utc=[DateTime]::UtcNow.ToString('o')
 executor=[Security.Principal.WindowsIdentity]::GetCurrent().Name
 approved_vm_guid=$expectedVM.ToString()
 vm_name=''
 vm_state=''
 stage='preflight'
 health_exit=-1
 authenticated_api_launched=$false
 api_listen='127.0.0.1:23750'
 post_real_host_restart=$false
 completed_validation=$false
 guest_dhcp_ip_refreshed=$false
 error=''
}
function SaveStatus {
 $status.utc=[DateTime]::UtcNow.ToString('o')
 $tmp=Join-Path $ops ('last-host-recovery-'+[Guid]::NewGuid().ToString('N')+'.writing')
 [IO.File]::WriteAllText($tmp,($status|ConvertTo-Json -Depth 6),[Text.UTF8Encoding]::new($false))
 Move-Item -LiteralPath $tmp -Destination $last -Force -ErrorAction Stop
}
try {
 if(!(Test-Path -LiteralPath $settingsPath -PathType Leaf)){throw 'Missing privately installed recovery settings'}
 $settings=Get-Content -LiteralPath $settingsPath -Raw -ErrorAction Stop|ConvertFrom-Json
 if($settings.approved_vm_guid -ne $expectedVM.ToString()){throw 'Approved VM ID changed'}
 if([string]$settings.source_script_sha256 -notmatch '^[0-9a-f]{64}$' -or (Get-FileHash -LiteralPath $PSCommandPath -Algorithm SHA256).Hash.ToLowerInvariant() -cne [string]$settings.source_script_sha256){throw 'Protected boot launcher source hash mismatch'}
 if($settings.installed_hive_sha256 -notmatch '^[0-9a-f]{64}$'){throw 'Invalid immutable binary hash'}
 $hive='C:\Program Files\StackDeck\stackdeck-hive.exe'
 if(!(Test-Path -LiteralPath $hive -PathType Leaf)){throw 'Installed Hive missing'}
 if((Get-FileHash $hive -Algorithm SHA256).Hash.ToLowerInvariant() -cne [string]$settings.installed_hive_sha256){throw 'Unqualified installed Hive bytes'}
 $boot=(Get-CimInstance Win32_OperatingSystem -ErrorAction Stop).LastBootUpTime.ToUniversalTime()
 $baseline=[DateTime]::Parse([string]$settings.baseline_boot_utc).ToUniversalTime()
 $status.post_real_host_restart=($boot -gt $baseline.AddSeconds(30))
 $status.stage='waiting_for_only_approved_vm'
 SaveStatus
 $ready=$false
 for($i=1;$i -le 48;$i++){
  $vms=@(Get-VM -ErrorAction Stop)
  if($vms.Count -ne 1 -or $vms[0].Id -ne $expectedVM -or $vms[0].Name -ne 'stackdeck-linux'){
   throw 'Unexpected VM inventory; refusing to operate another VM'
  }
  $vm=$vms[0]
  $status.vm_name=$vm.Name
  $status.vm_state=$vm.State.ToString()
  if($vm.AutomaticStartAction.ToString() -ne 'StartIfRunning' -or $vm.AutomaticStopAction.ToString() -ne 'Save'){
   throw 'Host-recovery VM start/save settings not qualified'
  }
  if($vm.State.ToString() -eq 'Running'){
   # The approved Hyper-V guest can obtain a different IPv4 lease after a
   # physical host restart. Discover only its unambiguous private IPv4, then
   # refresh StackDeck's machine-scoped SSH target through the CLI itself.
   $nic=@(Get-VMNetworkAdapter -VM $vm -ErrorAction Stop)
   if($nic.Count -ne 1){throw 'Approved VM NIC inventory ambiguous'}
   $addresses=@($nic[0].IPAddresses |Where-Object {$_ -match '^\d+\.\d+\.\d+\.\d+$' -and $_ -notmatch '^(?:127\.|169\.254\.)'} |Select-Object -Unique)
   if($addresses.Count -eq 1){
    $current=Get-Content (Join-Path $cfgRoot 'hyperv.json') -Raw -ErrorAction Stop |ConvertFrom-Json
    if($current.vm_name -ne 'stackdeck-linux'){throw 'Machine configuration VM identity mismatch'}
    if([string]$current.ssh_host -ne [string]$addresses[0]){
     $preference=$ErrorActionPreference
     try{
      $ErrorActionPreference='Continue'
      $discovery=(& $hive hyperv discover-ip --timeout 10 2>&1|Out-String)
      $discoveryExit=$LASTEXITCODE
     }finally{$ErrorActionPreference=$preference}
     if($discoveryExit -ne 0){throw 'Approved VM guest IP refresh failed'}
     $refreshed=Get-Content (Join-Path $cfgRoot 'hyperv.json') -Raw -ErrorAction Stop|ConvertFrom-Json
     if($refreshed.vm_name -ne 'stackdeck-linux' -or $refreshed.ssh_host -ne $addresses[0]){throw 'Guest IP refresh did not match approved VM adapter'}
     $status.guest_dhcp_ip_refreshed=$true
     $status.stage='approved_guest_dhcp_refreshed'
     SaveStatus
    }
   }
   $oldPreference=$ErrorActionPreference
   try {
    $ErrorActionPreference='Continue'
    $health=(& $hive hyperv health 2>&1|Out-String).Trim()
    $status.health_exit=$LASTEXITCODE
   }finally{$ErrorActionPreference=$oldPreference}
   if($status.health_exit -eq 0 -and $health.Contains('containerd') -and $health.Contains('buildkit') -and $health.Contains('reachable')){
    $ready=$true
    break
   }
  }
  Start-Sleep -Seconds 5
 }
 if(!$ready){throw 'Approved VM and guest container runtime did not become healthy during bounded startup window'}
 $status.stage='runtime_healthy'
 $status.completed_validation=$true
 SaveStatus
 $tokenPath=Join-Path $cfgRoot 'docker-api-token.txt'
 if(!(Test-Path -LiteralPath $tokenPath -PathType Leaf)){throw 'Protected Docker API token missing'}
 $token=[IO.File]::ReadAllText($tokenPath).Trim()
 if($token.Length -lt 32){throw 'Protected Docker API token invalid'}
 $listener=@(Get-NetTCPConnection -LocalPort 23750 -State Listen -ErrorAction SilentlyContinue)
 if($listener.Count -gt 0){throw 'Docker API port 23750 already has a listener; refusing to replace it'}
 $env:STACKDECK_CONFIG_DIR=$cfgRoot
 $env:STACKDECK_DOCKER_API_REQUIRE_TOKEN='1'
 $env:STACKDECK_DOCKER_API_TOKEN=$token
 $status.stage='authenticated_api_process_started'
 $status.authenticated_api_launched=$true
 SaveStatus
 $job=[StackDeckRecoveryJobV1]::CreateKillOnCloseJob()
 $daemon=New-Object System.Diagnostics.Process
 try {
  $daemon.StartInfo.FileName=$hive
  $daemon.StartInfo.Arguments='daemon serve --host 127.0.0.1 --port 23750'
  $daemon.StartInfo.UseShellExecute=$false
  $daemon.StartInfo.CreateNoWindow=$true
  if(!$daemon.Start()){throw 'Cannot launch authenticated API daemon'}
  if(![StackDeckRecoveryJobV1]::AssignProcessToJobObject($job,$daemon.Handle)){
   try{$daemon.Kill()}catch{}
   throw ('Cannot attach API daemon to kill-on-close Windows Job Object: '+[Runtime.InteropServices.Marshal]::GetLastWin32Error())
  }
  $daemon.WaitForExit()
  $code=$daemon.ExitCode
 }finally{
  # If Task Scheduler terminates this script, the OS closes the job handle.
  # The child cannot survive as an orphan listening on the API port.
  [void][StackDeckRecoveryJobV1]::CloseHandle($job)
  $daemon.Dispose()
 }
 $status.stage='authenticated_api_process_stopped'
 $status.authenticated_api_launched=$false
 $status.error='Daemon exited with code '+$code
 SaveStatus
 exit $(if($code -eq 0){1}else{$code})
}catch{
 $status.error=$_.Exception.Message
 $status.stage='failed'
 try{SaveStatus}catch{}
 Write-Error ('StackDeck unattended host recovery failed at '+$status.stage+'; see protected ops status. Reason: '+$status.error)
 exit 1
}
