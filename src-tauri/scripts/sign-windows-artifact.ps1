param(
    [Parameter(Mandatory = $true)]
    [string]$FilePath
)

$ErrorActionPreference = "Stop"

if ($env:WINDOWS_SIGNING_ENABLED -ne "true") {
    Write-Host "Windows signing is disabled; leaving artifact unsigned: $FilePath"
    exit 0
}

$requiredVariables = @(
    "SSL_COM_USERNAME",
    "SSL_COM_PASSWORD",
    "SSL_COM_CREDENTIAL_ID",
    "SSL_COM_TOTP_SECRET"
)

$missingVariables = @()
foreach ($name in $requiredVariables) {
    if ([string]::IsNullOrWhiteSpace([Environment]::GetEnvironmentVariable($name))) {
        $missingVariables += $name
    }
}

if ($missingVariables.Count -gt 0) {
    throw "Missing required Windows signing environment variables: $($missingVariables -join ', ')"
}

$resolvedFile = Resolve-Path -LiteralPath $FilePath
$codeSignTool = if ([string]::IsNullOrWhiteSpace($env:CODESIGN_TOOL_PATH)) {
    (Get-Command CodeSignTool.bat -ErrorAction Stop).Source
} else {
    $env:CODESIGN_TOOL_PATH
}
$programName = if ([string]::IsNullOrWhiteSpace($env:WINDOWS_SIGNING_DESCRIPTION)) {
    "百积木"
} else {
    $env:WINDOWS_SIGNING_DESCRIPTION
}

Write-Host "Signing Windows artifact: $resolvedFile"
$arguments = @(
    "sign",
    "-username=$env:SSL_COM_USERNAME",
    "-password=$env:SSL_COM_PASSWORD",
    "-credential_id=$env:SSL_COM_CREDENTIAL_ID",
    "-totp_secret=$env:SSL_COM_TOTP_SECRET",
    "-input_file_path=$($resolvedFile.Path)",
    "-override=true"
)

if ($resolvedFile.Path.EndsWith(".msi", [StringComparison]::OrdinalIgnoreCase)) {
    $arguments += "-program_name=$programName"
}

& $codeSignTool @arguments

if ($LASTEXITCODE -ne 0) {
    throw "CodeSignTool failed with exit code $LASTEXITCODE"
}
