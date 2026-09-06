$ErrorActionPreference = "Stop"
$cli = Resolve-Path "src-tauri/resources/bin/baijimu.exe"
$migration = Resolve-Path "src-tauri/resources/bin/bridge-agent-unified-app-id-migration.exe"
$msi = Get-ChildItem `
  -Path "src-tauri/target/release/bundle/msi" `
  -Filter "*.msi" `
  -File `
  -ErrorAction Stop |
  Select-Object -First 1
if (-not $msi) {
  throw "Windows MSI artifact was not found for Defender scan"
}
$files = @($cli.Path, $migration.Path, $msi.FullName)
& "src-tauri/scripts/verify-windows-defender.ps1" `
  -FilePath $files
