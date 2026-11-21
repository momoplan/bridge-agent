$ErrorActionPreference = "Stop"
if ($env:GITHUB_ACTIONS -ne "true") { throw "Installer regression test is CI-only" }
# Immutable regression fixtures: 0.6.18 first exposed the white-screen failure.
# 0.20.0 wrote the legacy Connector records checked during the upgrade.
$fixtureVersions = @("0.6.18", "0.20.0")
$directory = Join-Path $env:RUNNER_TEMP "white-screen-repair"
New-Item -ItemType Directory -Path $directory -Force | Out-Null
$current = Get-ChildItem "src-tauri/target/release/bundle/msi" -Filter "*.msi" -File | Select-Object -First 1
if (-not $current) { throw "Current signed MSI was not found" }
function Invoke-Installer([string]$operation, [string]$package, [string]$phase) {
  $log = Join-Path $directory "$phase.log"
  $process = Start-Process msiexec.exe -ArgumentList @($operation, "`"$package`"", "/qn", "/norestart", "AUTOLAUNCHAPP=False", "/L*v", "`"$log`"") -Wait -PassThru
  if ($process.ExitCode -notin @(0, 3010)) {
    Get-Content $log -Tail 100 | Write-Host
    throw "$phase failed: $($process.ExitCode)"
  }
}
$installed = Join-Path $env:ProgramFiles "百积木\bridge-agent-desktop.exe"
# Tauri restores the unsigned intermediate EXE after bundling. The signed MSI payload,
# not target/release/bridge-agent-desktop.exe, is the authoritative installation identity.
Invoke-Installer "/i" $current.FullName "install-current-msi-baseline"
$signature = Get-AuthenticodeSignature -LiteralPath $installed
if ($signature.Status -ne "Valid") { throw "Current MSI payload signature is invalid: $($signature.Status)" }
$expected = (Get-FileHash $installed).Hash
Write-Host "Current signed MSI payload: SHA256=$expected ProductVersion=$((Get-Item $installed).VersionInfo.ProductVersion)"
Invoke-Installer "/x" $current.FullName "remove-current-msi-baseline"
$configDirectory = Join-Path $env:ProgramData "Baijimu\BridgeAgent"
$configPath = Join-Path $configDirectory "agent-config.json"
$dataDirectory = Join-Path $configDirectory "app-data\repair-fixture"

foreach ($fixtureVersion in $fixtureVersions) {
  $fixtureTag = "bridge-agent-v$fixtureVersion"
  $fixtureAsset = "Baijimu_${fixtureVersion}_x64_zh-CN.msi"
  & gh release download $fixtureTag --repo $env:GITHUB_REPOSITORY --pattern $fixtureAsset --dir $directory
  if ($LASTEXITCODE -ne 0) { throw "Failed to download immutable Windows MSI fixture $fixtureTag" }
  $previous = Join-Path $directory $fixtureAsset
  Invoke-Installer "/i" $previous "install-affected-version-$fixtureVersion"
  New-Item -ItemType Directory -Path $dataDirectory -Force | Out-Null
  # An unreadable business config must not be a prerequisite for external package repair.
  Set-Content $configPath -Value "invalid-business-config-preserve-verbatim" -Encoding utf8
  $marker = Join-Path $dataDirectory "session.txt"
  Set-Content $marker -Value "preserve-user-data" -Encoding utf8
  $configHash = (Get-FileHash $configPath).Hash
  $dataHash = (Get-FileHash $marker).Hash
  Invoke-Installer "/i" $current.FullName "repair-with-new-version-$fixtureVersion"
  if ((Get-FileHash $configPath).Hash -ne $configHash) { throw "Repair overwrote business configuration" }
  if ((Get-FileHash $marker).Hash -ne $dataHash) { throw "Repair lost application data" }
  if ((Get-FileHash $installed).Hash -ne $expected) { throw "Repair did not install the current signed executable" }
  $productVersion = (Get-Item -LiteralPath $installed).VersionInfo.ProductVersion
  Write-Host "Windows MSI upgrade passed: $fixtureVersion -> $productVersion; config and data preserved verbatim"
  Invoke-Installer "/x" $current.FullName "remove-ci-installation-$fixtureVersion"
}
