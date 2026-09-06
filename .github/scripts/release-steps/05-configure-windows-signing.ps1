$ErrorActionPreference = "Stop"
$required = @(
  "SSL_COM_USERNAME",
  "SSL_COM_PASSWORD",
  "SSL_COM_CREDENTIAL_ID",
  "SSL_COM_TOTP_SECRET"
)
$missing = @()
foreach ($name in $required) {
  if ([string]::IsNullOrWhiteSpace([Environment]::GetEnvironmentVariable($name))) {
    $missing += $name
  }
}
if ($missing.Count -gt 0) {
  throw "Windows release requires code signing. Missing SSL.com secrets: $($missing -join ', ')"
}
"WINDOWS_SIGNING_ENABLED=true" | Out-File -FilePath $env:GITHUB_ENV -Append -Encoding utf8
