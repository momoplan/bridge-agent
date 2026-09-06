$ErrorActionPreference = "Stop"
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
$diagnosticFiles = Get-ChildItem -Path "src-tauri/target/release" -Filter "*.exe" -File -ErrorAction SilentlyContinue
foreach ($file in $diagnosticFiles) {
  $signature = Get-AuthenticodeSignature -LiteralPath $file.FullName
  Write-Host "$($file.FullName): $($signature.Status) (intermediate exe)"
}

$requiredSignedExecutables = @(
  Get-ChildItem -Path "src-tauri/binaries" -Filter "bridge-agent-uninstaller-*.exe" -File -ErrorAction Stop
  Get-Item -LiteralPath "src-tauri/resources/bin/bridge-agent-unified-app-id-migration.exe" -ErrorAction Stop
)
if ($requiredSignedExecutables.Count -ne 2) {
  throw "Expected the signed Windows uninstaller and migration resource, found $($requiredSignedExecutables.Count)"
}
foreach ($file in $requiredSignedExecutables) {
  $signature = Get-AuthenticodeSignature -LiteralPath $file.FullName
  Write-Host "$($file.FullName): $($signature.Status) (required signed sidecar)"
  if ($signature.Status -ne "Valid") {
    throw "Invalid Authenticode signature for $($file.FullName): $($signature.Status)"
  }
}

$files = Get-ChildItem -Path "src-tauri/target/release/bundle/msi" -Filter "*.msi" -File -ErrorAction Stop

if ($files.Count -eq 0) {
  throw "No Windows MSI artifacts found to verify"
}

$signtool = Get-ChildItem `
  -Path "${env:ProgramFiles(x86)}\Windows Kits\10\bin" `
  -Filter "signtool.exe" `
  -File `
  -Recurse `
  -ErrorAction Stop |
  Where-Object { $_.FullName -match "\\x64\\signtool\.exe$" } |
  Sort-Object FullName -Descending |
  Select-Object -First 1
if (-not $signtool) {
  throw "signtool.exe was not found"
}
if (
  [string]::IsNullOrWhiteSpace($env:OSSLSIGNCODE_PATH) -or
  -not (Test-Path -LiteralPath $env:OSSLSIGNCODE_PATH)
) {
  throw "Pinned osslsigncode metadata verifier was not found"
}

foreach ($file in $files) {
  $signature = Get-AuthenticodeSignature -LiteralPath $file.FullName
  Write-Host "$($file.FullName): $($signature.Status)"
  if ($signature.Status -ne "Valid") {
    throw "Invalid Authenticode signature for $($file.FullName): $($signature.Status)"
  }

  $signatureDetails = (& $signtool.FullName verify /pa /v $file.FullName 2>&1) -join "`n"
  if ($LASTEXITCODE -ne 0) {
    Write-Host $signatureDetails
    throw "signtool verification failed for $($file.FullName)"
  }
  Write-Host $signatureDetails

  # signtool validates the Windows trust chain, digest, and timestamp,
  # but its verify output does not expose SpcSpOpusInfo. Use the pinned
  # osslsigncode parser to read that authenticated attribute directly.
  $metadataStartInfo = [System.Diagnostics.ProcessStartInfo]::new()
  $metadataStartInfo.FileName = $env:OSSLSIGNCODE_PATH
  $metadataStartInfo.UseShellExecute = $false
  $metadataStartInfo.CreateNoWindow = $true
  $metadataStartInfo.RedirectStandardOutput = $true
  $metadataStartInfo.RedirectStandardError = $true
  $metadataStartInfo.StandardOutputEncoding = [System.Text.UTF8Encoding]::new($false)
  $metadataStartInfo.StandardErrorEncoding = [System.Text.UTF8Encoding]::new($false)
  $metadataStartInfo.ArgumentList.Add("verify")
  $metadataStartInfo.ArgumentList.Add("-in")
  $metadataStartInfo.ArgumentList.Add($file.FullName)
  $metadataProcess = [System.Diagnostics.Process]::new()
  $metadataProcess.StartInfo = $metadataStartInfo
  if (-not $metadataProcess.Start()) {
    throw "Unable to start osslsigncode metadata verifier"
  }
  $metadataStdout = $metadataProcess.StandardOutput.ReadToEndAsync()
  $metadataStderr = $metadataProcess.StandardError.ReadToEndAsync()
  $metadataProcess.WaitForExit()
  $metadataDetails = (
    $metadataStdout.GetAwaiter().GetResult() + "`n" +
    $metadataStderr.GetAwaiter().GetResult()
  )
  Write-Host $metadataDetails
  Write-Host "osslsigncode metadata verifier exit code: $($metadataProcess.ExitCode)"
  if ($metadataDetails -notmatch '(?m)^\s*Text description:\s*百积木\s*$') {
    throw "Authenticode description must be 百积木 for $($file.FullName)"
  }
}
