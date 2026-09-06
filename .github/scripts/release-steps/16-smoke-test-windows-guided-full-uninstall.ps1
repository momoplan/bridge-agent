$ErrorActionPreference = "Stop"
$msi = Get-ChildItem `
  -Path "src-tauri/target/release/bundle/msi" `
  -Filter "*.msi" `
  -File `
  -ErrorAction Stop | Select-Object -First 1
if (-not $msi) {
  throw "Windows MSI artifact was not found for full uninstall smoke test"
}

$installLog = Join-Path $env:RUNNER_TEMP "baijimu-full-uninstall-install.log"
$install = Start-Process `
  -FilePath "msiexec.exe" `
  -ArgumentList @("/i", "`"$($msi.FullName)`"", "/qn", "/norestart", "AUTOLAUNCHAPP=True", "/L*v", "`"$installLog`"") `
  -Wait `
  -PassThru
if ($install.ExitCode -notin @(0, 3010)) {
  Get-Content -LiteralPath $installLog -Tail 160 -ErrorAction SilentlyContinue | Write-Host
  throw "Windows MSI reinstall failed with exit code $($install.ExitCode)"
}

$desktopExecutable = "C:\Program Files\百积木\bridge-agent-desktop.exe"
$uninstaller = "C:\Program Files\百积木\bridge-agent-uninstaller.exe"
$programData = "C:\ProgramData\Baijimu\BridgeAgent"
$roamingData = Join-Path $env:APPDATA "baijimu\bridge-agent"
$tauriData = Join-Path $env:LOCALAPPDATA "com.baijimu.bridgeagent"
$managedCliRoot = Join-Path $env:LOCALAPPDATA "Baijimu\apps\baijimu-cli"
$managedBin = Join-Path $env:LOCALAPPDATA "Baijimu\bin"
$managedLauncher = Join-Path $managedBin "baijimu.exe"

$deadline = (Get-Date).AddSeconds(45)
do {
  $desktop = Get-Process -Name "bridge-agent-desktop" -ErrorAction SilentlyContinue
  if (
    $desktop -and
    (Test-Path -LiteralPath $uninstaller) -and
    (Test-Path -LiteralPath $managedLauncher)
  ) {
    break
  }
  Start-Sleep -Milliseconds 500
} while ((Get-Date) -lt $deadline)
if (-not $desktop) {
  throw "Desktop did not start before guided full uninstall"
}
if (-not (Test-Path -LiteralPath $uninstaller)) {
  throw "Guided uninstaller was not installed"
}
if (-not (Test-Path -LiteralPath $managedLauncher)) {
  throw "Managed CLI was not installed before full uninstall"
}

New-Item -ItemType Directory -Path $roamingData, $tauriData -Force | Out-Null
Set-Content -LiteralPath (Join-Path $roamingData "full-uninstall-marker.txt") -Value "remove me"
Set-Content -LiteralPath (Join-Path $tauriData "full-uninstall-marker.txt") -Value "remove me"
Set-Content -LiteralPath (Join-Path $programData "full-uninstall-marker.txt") -Value "remove me"

$request = Start-Process `
  -FilePath $uninstaller `
  -ArgumentList @("--full", "--quiet") `
  -Wait `
  -PassThru
if ($request.ExitCode -ne 0) {
  throw "Guided uninstall launcher failed with exit code $($request.ExitCode)"
}

$deadline = (Get-Date).AddSeconds(90)
do {
  if (-not (Test-Path -LiteralPath $desktopExecutable)) {
    break
  }
  Start-Sleep -Milliseconds 500
} while ((Get-Date) -lt $deadline)
if (Test-Path -LiteralPath $desktopExecutable) {
  throw "Desktop executable remained after guided full uninstall"
}
foreach ($path in @($programData, $roamingData, $tauriData, $managedCliRoot, $managedLauncher)) {
  if (Test-Path -LiteralPath $path) {
    throw "Full uninstall left managed data behind: $path"
  }
}
$userRun = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey("Software\Microsoft\Windows\CurrentVersion\Run")
try {
  if ($userRun -and $null -ne $userRun.GetValue("BaijimuBridgeAgent", $null)) {
    throw "Full uninstall left the login startup entry behind"
  }
} finally {
  if ($userRun) { $userRun.Dispose() }
}
$userEnvironment = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey("Environment")
try {
  $userPath = if ($userEnvironment) { [string]$userEnvironment.GetValue("Path", "") } else { "" }
  $managedBinPattern = [Regex]::Escape($managedBin.TrimEnd('\'))
  if ($userPath -match "(?i)(^|;)$managedBinPattern[\\/]*(;|$)") {
    throw "Full uninstall left the Baijimu managed bin directory in the user PATH"
  }
} finally {
  if ($userEnvironment) { $userEnvironment.Dispose() }
}
Write-Host "Verified Windows guided full uninstall and managed data cleanup"
