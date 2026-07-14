[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$InstallDir,

    [Parameter(Mandatory = $true)]
    [string]$VmRoot,

    [Parameter(Mandatory = $true)]
    [string]$LogPath,

    [string]$InstallerKind = "unknown"
)

$ErrorActionPreference = "Stop"

function Write-InstallLog {
    param([string]$Message)
    $stamp = (Get-Date).ToString("yyyy-MM-dd HH:mm:ss.fff zzz")
    Add-Content -LiteralPath $script:LogPath -Value "[$stamp] $Message" -Encoding UTF8
}

function Format-CommandLine {
    param([string]$Exe, [string[]]$Args)
    $quoted = @($Exe) + $Args | ForEach-Object {
        if ($_ -match "\s") { '"' + ($_ -replace '"', '\"') + '"' } else { $_ }
    }
    $quoted -join " "
}

$LogPath = [System.IO.Path]::GetFullPath($LogPath)
$logDir = Split-Path -Parent $LogPath
New-Item -ItemType Directory -Force -Path $logDir | Out-Null
New-Item -ItemType Directory -Force -Path (Split-Path -Parent $VmRoot) | Out-Null

Write-InstallLog "==== StackDeck installer configuration start ===="
Write-InstallLog "installer_kind=$InstallerKind"
Write-InstallLog "install_dir=$InstallDir"
Write-InstallLog "selected_vm_root=$VmRoot"
Write-InstallLog "computer=$env:COMPUTERNAME user=$env:USERNAME domain=$env:USERDOMAIN"
Write-InstallLog "process_arch=$env:PROCESSOR_ARCHITECTURE os_arch=$env:PROCESSOR_ARCHITEW6432"
Write-InstallLog "powershell=$($PSVersionTable.PSVersion)"

$principal = New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())
Write-InstallLog "is_admin=$($principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator))"

try {
    $os = Get-CimInstance Win32_OperatingSystem
    Write-InstallLog "os_caption=$($os.Caption)"
    Write-InstallLog "os_version=$($os.Version)"
    Write-InstallLog "os_build=$($os.BuildNumber)"
} catch {
    Write-InstallLog "os_probe_error=$($_.Exception.Message)"
}

try {
    Get-Volume | Sort-Object DriveLetter | ForEach-Object {
        if ($_.DriveLetter) {
            Write-InstallLog ("volume drive={0}: label={1} fs={2} size_gb={3:N2} free_gb={4:N2} health={5}" -f $_.DriveLetter, $_.FileSystemLabel, $_.FileSystem, ($_.Size / 1GB), ($_.SizeRemaining / 1GB), $_.HealthStatus)
        }
    }
} catch {
    Write-InstallLog "volume_probe_error=$($_.Exception.Message)"
}

try {
    $feature = Get-WindowsOptionalFeature -Online -FeatureName Microsoft-Hyper-V-All -ErrorAction Stop
    Write-InstallLog "hyperv_feature_state=$($feature.State)"
} catch {
    Write-InstallLog "hyperv_feature_probe_error=$($_.Exception.Message)"
}

foreach ($cmd in @("Get-VM", "New-VM", "Resize-VHD", "Get-VMSwitch", "ssh.exe", "powershell.exe")) {
    $found = Get-Command $cmd -ErrorAction SilentlyContinue
    if ($found) {
        Write-InstallLog "command_found $cmd path=$($found.Source)"
    } else {
        Write-InstallLog "command_missing $cmd"
    }
}

$resolvedInstallDir = Resolve-Path -LiteralPath $InstallDir -ErrorAction SilentlyContinue
if ($resolvedInstallDir) {
    Write-InstallLog "resolved_install_dir=$($resolvedInstallDir.Path)"
} else {
    Write-InstallLog "install_dir_not_found=$InstallDir"
}

$candidateCli = @(
    (Join-Path $InstallDir "stackdeck-hive.exe"),
    (Join-Path $InstallDir "bin\stackdeck-hive.exe"),
    (Join-Path $InstallDir "hive.exe"),
    (Join-Path $InstallDir "bin\hive.exe")
) | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1

if (-not $candidateCli) {
    Write-InstallLog "sidecar_not_found"
    throw "Could not find stackdeck-hive.exe or hive.exe under $InstallDir"
}

Write-InstallLog "sidecar=$candidateCli"
$vmRootFull = [System.IO.Path]::GetFullPath($VmRoot)
New-Item -ItemType Directory -Force -Path $vmRootFull | Out-Null
Write-InstallLog "vm_root_created_or_exists=$vmRootFull"

$drive = [System.IO.Path]::GetPathRoot($vmRootFull)
if ($drive) {
    $driveInfo = Get-PSDrive -Name $drive.Substring(0, 1) -ErrorAction SilentlyContinue
    if ($driveInfo) {
        Write-InstallLog ("selected_drive name={0} used_gb={1:N2} free_gb={2:N2}" -f $driveInfo.Name, ($driveInfo.Used / 1GB), ($driveInfo.Free / 1GB))
    }
}

$configureArgs = @("hyperv", "configure", "--vm-name", "stackdeck-linux", "--vm-root", $vmRootFull)
Write-InstallLog "configure_command=$(Format-CommandLine -Exe $candidateCli -Args $configureArgs)"

$output = & $candidateCli @configureArgs 2>&1
$exitCode = $LASTEXITCODE
foreach ($line in $output) {
    Write-InstallLog "configure_output $line"
}
Write-InstallLog "configure_exit_code=$exitCode"
if ($exitCode -ne 0) {
    throw "stackdeck-hive hyperv configure failed with exit code $exitCode"
}

$configPath = Join-Path $env:USERPROFILE ".stackdeck_runner\hyperv.json"
if (Test-Path -LiteralPath $configPath) {
    Write-InstallLog "config_file=$configPath"
    try {
        $cfg = Get-Content -Raw -LiteralPath $configPath | ConvertFrom-Json
        Write-InstallLog "config_vm_name=$($cfg.vm_name)"
        Write-InstallLog "config_vm_root=$($cfg.vm_root)"
        Write-InstallLog "config_memory_mb=$($cfg.vm_memory_mb)"
        Write-InstallLog "config_cpus=$($cfg.vm_cpu_count)"
        Write-InstallLog "config_disk_gb=$($cfg.vm_disk_gb)"
    } catch {
        Write-InstallLog "config_parse_error=$($_.Exception.Message)"
    }
} else {
    Write-InstallLog "config_file_missing=$configPath"
}

$latestPointer = Join-Path $logDir "latest-install-log.txt"
Set-Content -LiteralPath $latestPointer -Value $LogPath -Encoding UTF8
Write-InstallLog "latest_log_pointer=$latestPointer"
Write-InstallLog "==== StackDeck installer configuration complete ===="
