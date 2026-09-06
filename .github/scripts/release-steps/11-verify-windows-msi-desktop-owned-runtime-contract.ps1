$ErrorActionPreference = "Stop"
& "src-tauri/scripts/verify-windows-app-icon.ps1" `
  -FilePath "src-tauri/target/release/bridge-agent-desktop.exe"
$msiFiles = Get-ChildItem `
  -Path "src-tauri/target/release/bundle/msi" `
  -Filter "*.msi" `
  -File `
  -ErrorAction Stop
if ($msiFiles.Count -ne 1) {
  throw "Expected one Windows MSI artifact, found $($msiFiles.Count)"
}
& "src-tauri/scripts/verify-windows-msi.ps1" -FilePath $msiFiles[0].FullName
