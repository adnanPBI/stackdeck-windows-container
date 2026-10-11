#Requires -Version 5.1
#Requires -RunAsAdministrator

[CmdletBinding()]
param(
  [string]$StackDeckExe = "stackdeck-cli",
  [string]$ComposeFile = "examples\smoke\docker-compose.yml",
  [switch]$KeepRunning,
  [switch]$RemoveVolumes,
  [ValidateNotNullOrEmpty()]
  [string]$ServiceName = "web",
  [Uri]$HealthUri = "http://127.0.0.1:18080/",
  [ValidateRange(10, 600)]
  [UInt32]$StartupTimeout = 90
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Resolve-ApplicationPath {
  param([Parameter(Mandatory = $true)][string]$Value)

  if ([IO.Path]::IsPathRooted($Value) -or $Value -match "[\\/]") {
    if (-not (Test-Path -LiteralPath $Value -PathType Leaf)) {
      throw "StackDeck executable was not found: $Value"
    }
    return (Resolve-Path -LiteralPath $Value).ProviderPath
  }
  $command = Get-Command -Name $Value -CommandType Application -ErrorAction SilentlyContinue |
    Select-Object -First 1
  if (-not $command) {
    throw "StackDeck executable was not found on PATH: $Value"
  }
  return $command.Path
}

function Invoke-StackDeck {
  param([Parameter(Mandatory = $true)][string[]]$ArgumentList)

  & $script:StackDeckPath @ArgumentList
  $exitCode = $LASTEXITCODE
  if ($exitCode -ne 0) {
    throw "stackdeck failed with exit code $exitCode."
  }
}

function Invoke-StackDeckCapture {
  param([Parameter(Mandatory = $true)][string[]]$ArgumentList)

  $output = & $script:StackDeckPath @ArgumentList
  $exitCode = $LASTEXITCODE
  if ($exitCode -ne 0) {
    throw "stackdeck failed with exit code $exitCode."
  }
  return ($output | Out-String).Trim()
}

function Wait-SmokeEndpoint {
  $deadline = [DateTime]::UtcNow.AddSeconds($StartupTimeout)
  $lastError = $null
  while ([DateTime]::UtcNow -lt $deadline) {
    try {
      $response = Invoke-WebRequest -Uri $HealthUri -UseBasicParsing -TimeoutSec 5
      if ($response.StatusCode -eq 200) {
        return
      }
      $lastError = "HTTP status $($response.StatusCode)"
    } catch {
      $lastError = $_.Exception.Message
    }
    Start-Sleep -Seconds 1
  }
  throw "Smoke endpoint did not return HTTP 200 within $StartupTimeout seconds: $HealthUri. Last error: $lastError"
}

if (-not (Test-Path -LiteralPath $ComposeFile -PathType Leaf)) {
  throw "Compose file was not found: $ComposeFile"
}
$ComposePath = (Resolve-Path -LiteralPath $ComposeFile).ProviderPath
$StackDeckPath = Resolve-ApplicationPath $StackDeckExe

if ($HealthUri.Scheme -notin @("http", "https")) {
  throw "-HealthUri must use HTTP or HTTPS."
}
if ($KeepRunning -and $RemoveVolumes) {
  throw "-KeepRunning and -RemoveVolumes cannot be used together."
}

$cleanupNeeded = $false
$primaryError = $null
try {
  Invoke-StackDeck -ArgumentList @("hyperv", "health")
  $cleanupNeeded = $true
  Invoke-StackDeck -ArgumentList @("compose", "up", "-f", $ComposePath, "--backend", "hyperv", "-d")
  $statusText = Invoke-StackDeckCapture -ArgumentList @(
    "compose", "status", "-f", $ComposePath, "--backend", "hyperv", "--json"
  )
  $status = $statusText | ConvertFrom-Json
  $containers = @($status.PSObject.Properties)
  if ($containers.Count -eq 0) {
    throw "Smoke status did not contain any containers."
  }
  foreach ($container in $containers) {
    if (-not $container.Value.running -or
        $container.Value.status -notin @("running", "healthy")) {
      throw "Smoke container '$($container.Name)' is not running and healthy: $($container.Value.status)"
    }
  }
  Write-Host $statusText
  Invoke-StackDeck -ArgumentList @("compose", "logs", "-f", $ComposePath, "--backend", "hyperv", $ServiceName, "--tail", "80")
  Wait-SmokeEndpoint
} catch {
  $primaryError = $_
} finally {
  if ($cleanupNeeded -and -not $KeepRunning) {
    $downArgs = @("compose", "down", "-f", $ComposePath, "--backend", "hyperv")
    if ($RemoveVolumes) {
      $downArgs += "-v"
    }
    try {
      Invoke-StackDeck -ArgumentList $downArgs
    } catch {
      if ($primaryError) {
        Write-Warning "Smoke cleanup also failed: $($_.Exception.Message)"
      } else {
        $primaryError = $_
      }
    }
  }
}

if ($primaryError) {
  throw $primaryError
}
Write-Host "Smoke test completed with HTTP 200 from $HealthUri" -ForegroundColor Green
