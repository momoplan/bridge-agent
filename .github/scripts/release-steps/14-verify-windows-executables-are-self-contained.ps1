$ErrorActionPreference = "Stop"
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
if (-not (Test-Path -LiteralPath $vswhere)) {
  throw "vswhere.exe was not found"
}
$visualStudioRoot = & $vswhere -latest -products * -property installationPath
$dumpbin = Get-ChildItem `
  -Path "$visualStudioRoot\VC\Tools\MSVC" `
  -Filter "dumpbin.exe" `
  -File `
  -Recurse `
  -ErrorAction Stop | Sort-Object FullName -Descending | Select-Object -First 1
if (-not $dumpbin) {
  throw "dumpbin.exe was not found"
}

$executables = @(
  (Resolve-Path "src-tauri/target/release/bridge-agent-desktop.exe").Path,
  (Resolve-Path "src-tauri/resources/bin/baijimu.exe").Path,
  (Resolve-Path "src-tauri/resources/bin/bridge-agent-unified-app-id-migration.exe").Path
)
foreach ($executable in $executables) {
  $imports = (& $dumpbin.FullName /DEPENDENTS $executable 2>&1) -join "`n"
  Write-Host $imports
  if ($imports -match '(?i)\b(?:VCRUNTIME|MSVCP)\d*(?:_\d+)?\.dll\b') {
    throw "Windows executable requires the Visual C++ runtime: $executable"
  }
}
Write-Host "Verified Windows executables do not require VCRUNTIME/MSVCP"
