[CmdletBinding()]
param(
  [string]$PystackExe = "stackdeck",
  [string]$TaskName = "StackDeck Local Docker API",
  [string]$HostIp = "127.0.0.1",
  [UInt16]$Port = 23750,
  [string]$Token = ""
)
$ErrorActionPreference = "Stop"
$DataRoot = Join-Path $env:ProgramData "StackDeck"
$ConfigRoot = Join-Path $DataRoot "config"
New-Item -ItemType Directory -Force -Path $ConfigRoot | Out-Null
if (-not $Token) {
  $bytes = New-Object byte[] 32
  [Security.Cryptography.RandomNumberGenerator]::Fill($bytes)
  $Token = [Convert]::ToBase64String($bytes)
}
$TokenPath = Join-Path $ConfigRoot "docker-api-token.txt"
Set-Content -Path $TokenPath -Value $Token -Encoding ASCII
$UserGrant = "$($env:USERNAME):F"
icacls $TokenPath /inheritance:r /grant:r $UserGrant "Administrators:F" | Out-Null

$Command = "`$env:STACKDECK_DOCKER_API_TOKEN = Get-Content -Raw '$TokenPath'; `$env:STACKDECK_DOCKER_API_TOKEN = `$env:STACKDECK_DOCKER_API_TOKEN.Trim(); & '$PystackExe' daemon serve --host '$HostIp' --port $Port"
$Action = New-ScheduledTaskAction -Execute "powershell.exe" -Argument "-NoProfile -ExecutionPolicy Bypass -Command `"$Command`""
$Trigger = New-ScheduledTaskTrigger -AtLogOn
$Principal = New-ScheduledTaskPrincipal -UserId "$env:USERDOMAIN\$env:USERNAME" -RunLevel Highest
Register-ScheduledTask -TaskName $TaskName -Action $Action -Trigger $Trigger -Principal $Principal -Description "StackDeck local Docker API shim" -Force | Out-Null
Start-ScheduledTask -TaskName $TaskName
Write-Host "Docker API task installed on http://${HostIp}:$Port" -ForegroundColor Green
Write-Host "Bearer token saved to $TokenPath"
