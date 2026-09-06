$ErrorActionPreference = "Stop"
$zipPath = Join-Path $env:RUNNER_TEMP "CodeSignTool.zip"
$extractPath = Join-Path $env:RUNNER_TEMP "CodeSignTool"

Invoke-WebRequest `
  -Uri "https://ssl.com/wp-content/uploads/2024/10/CodeSignTool-v1.3.1-windows.zip" `
  -UserAgent "Mozilla/5.0 (Windows NT 10.0; Win64; x64) GitHub-Actions" `
  -MaximumRetryCount 3 `
  -RetryIntervalSec 5 `
  -OutFile $zipPath
$expectedSha256 = "e45a9e6c2aac4cae16c114eb590a2196406681357eb587507c65cd3646b5330d"
$actualSha256 = (Get-FileHash -Algorithm SHA256 -Path $zipPath).Hash.ToLowerInvariant()
if ($actualSha256 -ne $expectedSha256) {
  throw "CodeSignTool archive checksum mismatch: $actualSha256"
}
Expand-Archive -Force -Path $zipPath -DestinationPath $extractPath

$codeSignTool = Get-ChildItem `
  -Path $extractPath `
  -Filter "CodeSignTool.bat" `
  -Recurse `
  -File |
  Select-Object -First 1

if (-not $codeSignTool) {
  throw "Unable to locate CodeSignTool.bat"
}
"CODESIGN_TOOL_PATH=$($codeSignTool.FullName)" | Out-File -FilePath $env:GITHUB_ENV -Append -Encoding utf8

Push-Location $codeSignTool.DirectoryName
try {
  & ".\CodeSignTool.bat" --version
} finally {
  Pop-Location
}

$metadataVerifierZipPath = Join-Path $env:RUNNER_TEMP "osslsigncode.zip"
$metadataVerifierExtractPath = Join-Path $env:RUNNER_TEMP "osslsigncode"
Invoke-WebRequest `
  -Uri "https://github.com/mtrojnar/osslsigncode/releases/download/2.14/osslsigncode-2.14-windows-x64-mingw.zip" `
  -UserAgent "Mozilla/5.0 (Windows NT 10.0; Win64; x64) GitHub-Actions" `
  -MaximumRetryCount 3 `
  -RetryIntervalSec 5 `
  -OutFile $metadataVerifierZipPath
$expectedMetadataVerifierSha256 = "9a1722aaf62a27852c4eb9c35749a0248065052d0ae0a93d4ed6bb49def027f2"
$actualMetadataVerifierSha256 = (
  Get-FileHash -Algorithm SHA256 -Path $metadataVerifierZipPath
).Hash.ToLowerInvariant()
if ($actualMetadataVerifierSha256 -ne $expectedMetadataVerifierSha256) {
  throw "osslsigncode archive checksum mismatch: $actualMetadataVerifierSha256"
}
Expand-Archive `
  -Force `
  -Path $metadataVerifierZipPath `
  -DestinationPath $metadataVerifierExtractPath
$metadataVerifier = Get-ChildItem `
  -Path $metadataVerifierExtractPath `
  -Filter "osslsigncode.exe" `
  -Recurse `
  -File | Select-Object -First 1
if (-not $metadataVerifier) {
  throw "Unable to locate osslsigncode.exe"
}
"OSSLSIGNCODE_PATH=$($metadataVerifier.FullName)" |
  Out-File -FilePath $env:GITHUB_ENV -Append -Encoding utf8
& $metadataVerifier.FullName --version
if ($LASTEXITCODE -ne 0) {
  throw "osslsigncode version check failed with exit code $LASTEXITCODE"
}
