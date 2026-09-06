$ErrorActionPreference = "Stop"
$msi = Get-ChildItem `
  -Path "src-tauri/target/release/bundle/msi" `
  -Filter "*.msi" `
  -File `
  -ErrorAction Stop | Select-Object -First 1
if (-not $msi) {
  throw "Windows MSI artifact was not found for install smoke test"
}

$legacyDir = Join-Path $env:RUNNER_TEMP "baijimu-legacy-msi"
New-Item -ItemType Directory -Path $legacyDir -Force | Out-Null
& gh release download "bridge-agent-v0.2.22" `
  --repo $env:GITHUB_REPOSITORY `
  --pattern "Baijimu_0.2.22_x64_en-US.msi" `
  --dir $legacyDir `
  --clobber
if ($LASTEXITCODE -ne 0) {
  throw "Failed to download the 0.2.22 service-owned MSI"
}
$legacyMsi = Get-ChildItem -Path $legacyDir -Filter "*.msi" -File |
  Select-Object -First 1
if (-not $legacyMsi) {
  throw "The 0.2.22 service-owned MSI was not downloaded"
}

$legacyInstallLog = Join-Path $env:RUNNER_TEMP "baijimu-msi-legacy-install.log"
$upgradeLog = Join-Path $env:RUNNER_TEMP "baijimu-msi-upgrade.log"
$uninstallLog = Join-Path $env:RUNNER_TEMP "baijimu-msi-uninstall.log"
$legacyInstalled = $false
$upgradeInstalled = $false
function Get-OptionalRegistryValue {
  param(
    [Parameter(Mandatory = $true)]
    [Microsoft.Win32.RegistryKey]$Hive,
    [Parameter(Mandatory = $true)]
    [string]$SubKey,
    [Parameter(Mandatory = $true)]
    [string]$Name
  )

  $key = $Hive.OpenSubKey($SubKey)
  if ($null -eq $key) {
    return $null
  }
  try {
    return $key.GetValue(
      $Name,
      $null,
      [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames
    )
  } finally {
    $key.Dispose()
  }
}

$legacyDesktopExecutable = "C:\Program Files\Baijimu\bridge-agent-desktop.exe"
$desktopExecutable = "C:\Program Files\百积木\bridge-agent-desktop.exe"
$legacyServiceExecutable = "C:\Program Files\Baijimu\bridge-agent-service.exe"
$legacyDesktopBackup = Join-Path $env:RUNNER_TEMP "bridge-agent-desktop-0.2.22.exe"
$configPath = "C:\ProgramData\Baijimu\BridgeAgent\agent-config.json"
$userRunSubKey = "Software\Microsoft\Windows\CurrentVersion\Run"

try {
  $legacyInstall = Start-Process `
    -FilePath "msiexec.exe" `
    -ArgumentList @("/i", "`"$($legacyMsi.FullName)`"", "/qn", "/norestart", "AUTOLAUNCHAPP=False", "/L*v", "`"$legacyInstallLog`"") `
    -Wait `
    -PassThru
  if ($legacyInstall.ExitCode -notin @(0, 3010)) {
    if (Test-Path -LiteralPath $legacyInstallLog) {
      Get-Content -LiteralPath $legacyInstallLog -Tail 180 | Write-Host
    }
    throw "Windows 0.2.22 MSI install failed with exit code $($legacyInstall.ExitCode)"
  }
  $legacyInstalled = $true

  $deadline = (Get-Date).AddSeconds(15)
  do {
    $service = Get-CimInstance Win32_Service -Filter "Name='BridgeAgent'" -ErrorAction SilentlyContinue
    if ($service -and $service.State -eq "Running") {
      break
    }
    Start-Sleep -Milliseconds 500
  } while ((Get-Date) -lt $deadline)

  if (-not $service) {
    throw "The 0.2.22 BridgeAgent service was not installed"
  }
  if ($service.State -ne "Running") {
    throw "The 0.2.22 BridgeAgent service is not running: $($service.State)"
  }

  Stop-Service -Name "BridgeAgent" -Force -ErrorAction Stop
  Get-Process -Name "bridge-agent-desktop" -ErrorAction SilentlyContinue |
    Stop-Process -Force -ErrorAction SilentlyContinue
  if (-not (Test-Path -LiteralPath $legacyDesktopExecutable)) {
    throw "The 0.2.22 desktop executable was not installed"
  }
  Move-Item `
    -LiteralPath $legacyDesktopExecutable `
    -Destination $legacyDesktopBackup `
    -Force

  if (-not (Test-Path -LiteralPath $configPath)) {
    throw "The 0.2.22 service did not initialize the shared config"
  }
  $config = Get-Content -LiteralPath $configPath -Raw | ConvertFrom-Json
  $config.platform.workspace_id = 999999
  $config.relay.url = "ws://127.0.0.1:9/ws/agent"
  $config.relay.token = "windows-upgrade-smoke-token"
  $legacyConnectorId = "com.example.legacy-upgrade"
  $registeredAppId = "registered-upgrade-app"
  $config.local_apps = @(
    [ordered]@{
      connectorId = $legacyConnectorId
      name = "Legacy upgrade fixture"
      version = "1.0.0"
      description = "Verifies offline app ID migration during MSI upgrade"
      enabled = $false
      methods = @()
      events = @(
        [ordered]@{
          name = "upgrade.test"
          description = "Upgrade migration fixture"
          enabled = $true
          payload_schema = [ordered]@{ type = "object" }
        }
      )
    }
  )
  $config | ConvertTo-Json -Depth 100 |
    Set-Content -LiteralPath $configPath -Encoding utf8

  $legacyInstallDir = Join-Path (Split-Path -Parent $configPath) "connectors\$legacyConnectorId"
  $legacyDataDir = Join-Path (Split-Path -Parent $configPath) "connector-data\$legacyConnectorId"
  New-Item -ItemType Directory -Path $legacyInstallDir, $legacyDataDir -Force | Out-Null
  [ordered]@{
    manifest = [ordered]@{
      schemaVersion = "2.0"
      id = $legacyConnectorId
      version = "1.0.0"
    }
    marketAppId = $registeredAppId
  } | ConvertTo-Json -Depth 20 |
    Set-Content -LiteralPath (Join-Path $legacyInstallDir "install.json") -Encoding utf8
  Set-Content `
    -LiteralPath (Join-Path $legacyDataDir "session.json") `
    -Value "preserved-upgrade-session" `
    -Encoding utf8

  Start-Service -Name "BridgeAgent" -ErrorAction Stop

  $deadline = (Get-Date).AddSeconds(30)
  do {
    $legacyListener = Get-NetTCPConnection `
      -LocalAddress "127.0.0.1" `
      -LocalPort 18081 `
      -State Listen `
      -ErrorAction SilentlyContinue |
      Select-Object -First 1
    if ($legacyListener) {
      break
    }
    Start-Sleep -Milliseconds 500
  } while ((Get-Date) -lt $deadline)
  if (-not $legacyListener) {
    throw "The 0.2.22 service did not bind 127.0.0.1:18081"
  }
  $service = Get-CimInstance Win32_Service -Filter "Name='BridgeAgent'"
  if ($legacyListener.OwningProcess -ne $service.ProcessId) {
    throw "Port 18081 is not owned by the legacy service process"
  }
  $legacyServicePid = [int]$service.ProcessId

  $upgrade = Start-Process `
    -FilePath "msiexec.exe" `
    -ArgumentList @("/i", "`"$($msi.FullName)`"", "/qn", "/norestart", "AUTOLAUNCHAPP=True", "/L*v", "`"$upgradeLog`"") `
    -Wait `
    -PassThru
  if ($upgrade.ExitCode -notin @(0, 3010)) {
    if (Test-Path -LiteralPath $upgradeLog) {
      Get-Content -LiteralPath $upgradeLog -Tail 220 | Write-Host
    }
    throw "Windows desktop-owned MSI upgrade failed with exit code $($upgrade.ExitCode)"
  }
  $upgradeInstalled = $true

  $deadline = (Get-Date).AddSeconds(20)
  do {
    $service = Get-CimInstance Win32_Service -Filter "Name='BridgeAgent'" -ErrorAction SilentlyContinue
    $legacyServiceProcess = Get-Process -Id $legacyServicePid -ErrorAction SilentlyContinue
    if (-not $service -and -not $legacyServiceProcess) {
      break
    }
    Start-Sleep -Milliseconds 500
  } while ((Get-Date) -lt $deadline)
  if ($service) {
    throw "The legacy BridgeAgent service remains after the upgrade"
  }
  if ($legacyServiceProcess) {
    throw "The legacy BridgeAgent service process remains after the upgrade"
  }
  if (Test-Path -LiteralPath $legacyServiceExecutable) {
    throw "The legacy service executable remains after the upgrade"
  }
  if (-not (Test-Path -LiteralPath $desktopExecutable)) {
    throw "The upgraded desktop executable was not found"
  }
  $installedUninstaller = "C:\Program Files\百积木\bridge-agent-uninstaller.exe"
  if (-not (Test-Path -LiteralPath $installedUninstaller)) {
    throw "The guided uninstaller was not installed"
  }
  foreach ($installedExecutable in @($desktopExecutable, $installedUninstaller)) {
    $installedSignature = Get-AuthenticodeSignature -LiteralPath $installedExecutable
    if ($installedSignature.Status -ne "Valid") {
      throw "Installed executable has an invalid Authenticode signature: $installedExecutable ($($installedSignature.Status))"
    }
  }
  if (Test-Path -LiteralPath $legacyDesktopExecutable) {
    throw "The legacy English product directory remains after the upgrade"
  }

  $upgradeListener = Get-NetTCPConnection `
    -LocalAddress "127.0.0.1" `
    -LocalPort 18081 `
    -State Listen `
    -ErrorAction SilentlyContinue |
    Select-Object -First 1
  if ($upgradeListener) {
    if ($upgradeListener.OwningProcess -eq $legacyServicePid) {
      throw "Port 18081 is still owned by the retired service process"
    }
    $upgradeOwner = Get-Process -Id $upgradeListener.OwningProcess -ErrorAction SilentlyContinue
    if (-not $upgradeOwner -or $upgradeOwner.ProcessName -ne "bridge-agent-desktop") {
      throw "Port 18081 is owned by an unexpected process after the upgrade"
    }
  }

  $desktopProcess = Get-Process -Name "bridge-agent-desktop" -ErrorAction SilentlyContinue |
    Select-Object -First 1
  if (-not $desktopProcess) {
    $desktopProcess = Start-Process -FilePath $desktopExecutable -PassThru
  }
  $deadline = (Get-Date).AddSeconds(30)
  do {
    $userRunValue = Get-OptionalRegistryValue `
      -Hive ([Microsoft.Win32.Registry]::CurrentUser) `
      -SubKey $userRunSubKey `
      -Name "BaijimuBridgeAgent"
    $desktopListener = Get-NetTCPConnection `
      -LocalAddress "127.0.0.1" `
      -LocalPort 18081 `
      -State Listen `
      -ErrorAction SilentlyContinue |
      Select-Object -First 1
    if ($userRunValue -and $desktopListener) {
      break
    }
    Start-Sleep -Milliseconds 500
  } while ((Get-Date) -lt $deadline)
  if ($userRunValue -notlike "*bridge-agent-desktop.exe*") {
    throw "The current-user desktop login startup entry was not registered: $userRunValue"
  }
  if (-not $desktopListener) {
    throw "The desktop-owned runtime did not bind 127.0.0.1:18081"
  }
  if ($desktopListener.OwningProcess -ne $desktopProcess.Id) {
    throw "Port 18081 is not owned by the desktop process"
  }

  $migratedConfig = Get-Content -LiteralPath $configPath -Raw | ConvertFrom-Json
  if (@($migratedConfig.local_apps).Count -ne 0) {
    throw "Legacy connectorId configuration remained after direct MSI upgrade"
  }
  $migrationRoot = Join-Path `
    (Split-Path -Parent $configPath) `
    "migration-backups\unified-app-id\1.0.1"
  if (-not (Test-Path -LiteralPath (Join-Path $migrationRoot "agent-config.json"))) {
    throw "Legacy configuration backup was not created during direct MSI upgrade"
  }
  if (-not (Test-Path -LiteralPath (Join-Path $migrationRoot "connectors\$legacyConnectorId\install.json"))) {
    throw "Legacy connector package was not archived during direct MSI upgrade"
  }
  $migratedSession = Join-Path `
    (Split-Path -Parent $configPath) `
    "app-data\$registeredAppId\session.json"
  if (-not (Test-Path -LiteralPath $migratedSession)) {
    throw "Legacy connector data was not moved to the registered app ID"
  }

  $desktopCount = @(Get-Process -Name "bridge-agent-desktop" -ErrorAction SilentlyContinue).Count
  if ($desktopCount -ne 1) {
    throw "Expected one desktop runtime owner, found $desktopCount"
  }

  $quitRequest = Start-Process `
    -FilePath $desktopExecutable `
    -ArgumentList "--quit-running-instance" `
    -Wait `
    -PassThru
  if ($quitRequest.ExitCode -ne 0) {
    throw "The desktop quit request failed with exit code $($quitRequest.ExitCode)"
  }
  $deadline = (Get-Date).AddSeconds(30)
  do {
    $desktopStillRunning = Get-Process -Name "bridge-agent-desktop" -ErrorAction SilentlyContinue
    $desktopListener = Get-NetTCPConnection -LocalPort 18081 -State Listen -ErrorAction SilentlyContinue
    if (-not $desktopStillRunning -and -not $desktopListener) {
      break
    }
    Start-Sleep -Milliseconds 500
  } while ((Get-Date) -lt $deadline)
  if ($desktopStillRunning) {
    throw "The desktop process did not exit through the shared tray quit path"
  }
  if ($desktopListener) {
    throw "Port 18081 remains occupied after the desktop exited"
  }
  if (Get-Service -Name "BridgeAgent" -ErrorAction SilentlyContinue) {
    throw "The retired service reappeared after the desktop exited"
  }

  Write-Host "Verified 0.2.22 service-to-desktop upgrade, login startup, single runtime ownership, and graceful exit"
} finally {
  Get-Process -Name "bridge-agent-desktop" -ErrorAction SilentlyContinue |
    Stop-Process -Force -ErrorAction SilentlyContinue
  if ($upgradeInstalled) {
    $uninstall = Start-Process `
      -FilePath "msiexec.exe" `
      -ArgumentList @("/x", "`"$($msi.FullName)`"", "/qn", "/norestart", "/L*v", "`"$uninstallLog`"") `
      -Wait `
      -PassThru
    if ($uninstall.ExitCode -notin @(0, 1605, 3010)) {
      if (Test-Path -LiteralPath $uninstallLog) {
        Get-Content -LiteralPath $uninstallLog -Tail 120 | Write-Host
      }
      throw "Windows MSI uninstall smoke test failed with exit code $($uninstall.ExitCode)"
    }
    $userRunAfterUninstall = Get-OptionalRegistryValue `
      -Hive ([Microsoft.Win32.Registry]::CurrentUser) `
      -SubKey $userRunSubKey `
      -Name "BaijimuBridgeAgent"
    if ($null -ne $userRunAfterUninstall) {
      throw "The current-user desktop login startup entry remained after MSI uninstall"
    }
  } elseif ($legacyInstalled) {
    Start-Process `
      -FilePath "msiexec.exe" `
      -ArgumentList @("/x", "`"$($legacyMsi.FullName)`"", "/qn", "/norestart") `
      -Wait `
      -ErrorAction SilentlyContinue | Out-Null
  }
  $cleanupRunKey = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey($userRunSubKey)
  try {
    $cleanupRunKey.DeleteValue("BaijimuBridgeAgent", $false)
  } finally {
    $cleanupRunKey.Dispose()
  }
  Remove-Item -LiteralPath $legacyDesktopBackup -Force -ErrorAction SilentlyContinue
}

if (Get-Service -Name "BridgeAgent" -ErrorAction SilentlyContinue) {
  throw "BridgeAgent service remained after MSI uninstall"
}
if (Test-Path -LiteralPath "C:\Program Files\Baijimu\bridge-agent-service.exe") {
  throw "BridgeAgent service executable remained after MSI uninstall"
}
if (Test-Path -LiteralPath "C:\Program Files\百积木\bridge-agent-desktop.exe") {
  throw "The 百积木 desktop executable remained after MSI uninstall"
}
if (-not (Test-Path -LiteralPath $configPath)) {
  throw "Standard MSI uninstall removed device configuration that should be preserved"
}
Write-Host "Verified Windows desktop-owned MSI uninstall lifecycle"
