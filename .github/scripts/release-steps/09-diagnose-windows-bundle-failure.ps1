$ErrorActionPreference = "Continue"
$signingLog = Join-Path $env:RUNNER_TEMP "windows-signing.log"
if (Test-Path -LiteralPath $signingLog) {
  Write-Host "Windows signing diagnostics:"
  Get-Content -LiteralPath $signingLog
} else {
  Write-Host "No Windows signing diagnostic log was produced"
}

$msiDirectory = "src-tauri/target/release/bundle/msi"
if (Test-Path -LiteralPath $msiDirectory) {
  Get-ChildItem -LiteralPath $msiDirectory -File |
    Select-Object Name, Length, LastWriteTime |
    Format-Table -AutoSize
}

$candle = Get-ChildItem `
  -Path "src-tauri/target/release/wix" `
  -Filter "candle.exe" `
  -File `
  -Recurse `
  -ErrorAction SilentlyContinue | Select-Object -First 1
$utilExtension = if ($candle) {
  Get-ChildItem `
    -Path $candle.Directory.FullName `
    -Filter "WixUtilExtension.dll" `
    -File `
    -ErrorAction SilentlyContinue | Select-Object -First 1
}
if (-not $candle -or -not $utilExtension) {
  Write-Host "WiX compiler or WixUtilExtension was not found after bundle failure"
} else {
  & $candle.FullName `
    -nologo `
    -arch x64 `
    -ext $utilExtension.FullName `
    "src-tauri/wix/bridge-agent-windows-runtime.wxs"
  Write-Host "WiX fragment diagnostic exit code: $LASTEXITCODE"

  $wixToolDirectory = $candle.Directory.FullName
  $wixBuildDirectory = Split-Path -Parent $wixToolDirectory
  $light = Join-Path $wixToolDirectory "light.exe"
  $uiExtension = Join-Path $wixToolDirectory "WixUIExtension.dll"
  $locale = Join-Path $wixBuildDirectory "locale.wxl"
  $wixObjects = @(
    Get-ChildItem `
      -LiteralPath $wixBuildDirectory `
      -Filter "*.wixobj" `
      -File `
      -ErrorAction SilentlyContinue
  )
  if (
    (Test-Path -LiteralPath $light) -and
    (Test-Path -LiteralPath $uiExtension) -and
    (Test-Path -LiteralPath $locale) -and
    $wixObjects.Count -gt 0
  ) {
    $diagnosticMsi = Join-Path $wixBuildDirectory "diagnostic-output.msi"
    $lightOutput = & $light `
      -nologo `
      -v `
      "-cultures:zh-cn;en-US" `
      -loc $locale `
      -ext $uiExtension `
      -ext $utilExtension.FullName `
      -out $diagnosticMsi `
      @($wixObjects.FullName) 2>&1
    $lightExitCode = $LASTEXITCODE
    $lightOutput | ForEach-Object { Write-Host $_ }
    Write-Host "WiX linker diagnostic exit code: $lightExitCode"
  } else {
    Write-Host "WiX linker diagnostic inputs were incomplete"
  }
}
