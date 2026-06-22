param(
    [Parameter(Mandatory = $true)]
    [string]$FilePath
)

$ErrorActionPreference = "Stop"

$requiredVariables = @(
    "AZURE_CLIENT_ID",
    "AZURE_CLIENT_SECRET",
    "AZURE_TENANT_ID",
    "AZURE_ARTIFACT_SIGNING_ENDPOINT",
    "AZURE_ARTIFACT_SIGNING_ACCOUNT",
    "AZURE_ARTIFACT_SIGNING_CERTIFICATE_PROFILE"
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
$artifactSigningCli = (Get-Command artifact-signing-cli -ErrorAction Stop).Source
$description = if ([string]::IsNullOrWhiteSpace($env:WINDOWS_SIGNING_DESCRIPTION)) {
    "Bridge Agent"
} else {
    $env:WINDOWS_SIGNING_DESCRIPTION
}

Write-Host "Signing Windows artifact: $resolvedFile"
& $artifactSigningCli `
    --endpoint $env:AZURE_ARTIFACT_SIGNING_ENDPOINT `
    --account $env:AZURE_ARTIFACT_SIGNING_ACCOUNT `
    --certificate $env:AZURE_ARTIFACT_SIGNING_CERTIFICATE_PROFILE `
    --description $description `
    $resolvedFile

if ($LASTEXITCODE -ne 0) {
    throw "artifact-signing-cli failed with exit code $LASTEXITCODE"
}
