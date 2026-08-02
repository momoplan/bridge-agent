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
$normalizedFilePath = $resolvedFile.Path -replace '/', '\'
if (
    $normalizedFilePath -match "\\target\\release\\wix\\" -and
    $resolvedFile.Path.EndsWith(".dll", [StringComparison]::OrdinalIgnoreCase)
) {
    Write-Host "Skipping WiX tool DLL signing: $resolvedFile"
    exit 0
}

$codeSignTool = if ([string]::IsNullOrWhiteSpace($env:CODESIGN_TOOL_PATH)) {
    (Get-Command CodeSignTool.bat -ErrorAction Stop).Source
} else {
    $env:CODESIGN_TOOL_PATH
}
$codeSignToolDirectory = Split-Path -Parent $codeSignTool
$javaExecutable = Get-ChildItem `
    -Path $codeSignToolDirectory `
    -Filter "java.exe" `
    -File `
    -Recurse `
    -ErrorAction Stop |
    Where-Object { $_.FullName -match "\\jdk-[^\\]+\\bin\\java\.exe$" } |
    Select-Object -First 1
$codeSignToolJar = Get-ChildItem `
    -Path (Join-Path $codeSignToolDirectory "jar") `
    -Filter "code_sign_tool-*.jar" `
    -File `
    -ErrorAction Stop |
    Select-Object -First 1
if (-not $javaExecutable) {
    throw "Unable to locate the Java runtime bundled with CodeSignTool"
}
if (-not $codeSignToolJar) {
    throw "Unable to locate the CodeSignTool JAR"
}

$brandProgramName = ([string][char]0x767E) + ([char]0x79EF) + ([char]0x6728)
$programName = if ([string]::IsNullOrWhiteSpace($env:WINDOWS_SIGNING_DESCRIPTION)) {
    $brandProgramName
} else {
    $env:WINDOWS_SIGNING_DESCRIPTION
}

Write-Host "Signing Windows artifact: $resolvedFile"
$outputDirectory = Join-Path ([System.IO.Path]::GetTempPath()) "codesign-$([guid]::NewGuid().ToString('N'))"
New-Item -ItemType Directory -Force -Path $outputDirectory | Out-Null

$arguments = @(
    "sign",
    "-username=$env:SSL_COM_USERNAME",
    "-password=$env:SSL_COM_PASSWORD",
    "-credential_id=$env:SSL_COM_CREDENTIAL_ID",
    "-totp_secret=$env:SSL_COM_TOTP_SECRET",
    "-input_file_path=$($resolvedFile.Path)",
    "-output_dir_path=$outputDirectory"
)

if ($resolvedFile.Path.EndsWith(".msi", [StringComparison]::OrdinalIgnoreCase)) {
    if ($programName -ne $brandProgramName) {
        throw "Windows MSI signing description must match the product brand"
    }
    $arguments += "-program_name=$programName"
}

# CodeSignTool.bat forwards arguments through cmd.exe. On an English Windows
# code page that replaces the Chinese MSI program name with question marks
# before Java can encode it as an Authenticode BMPString. Invoke the bundled
# Java runtime directly so PowerShell passes the Unicode command line intact.
# The tool still needs its installation directory as the working directory to
# resolve the bundled configuration files.
$codeSignToolExitCode = 0
Push-Location $codeSignToolDirectory
try {
    & $javaExecutable.FullName `
        "-Dfile.encoding=UTF-8" `
        "-jar" `
        $codeSignToolJar.FullName `
        @arguments
    $codeSignToolExitCode = $LASTEXITCODE
} finally {
    Pop-Location
}

if ($codeSignToolExitCode -ne 0) {
    throw "CodeSignTool failed with exit code $codeSignToolExitCode"
}

$signedFile = Join-Path $outputDirectory (Split-Path -Leaf $resolvedFile.Path)
if (-not (Test-Path -LiteralPath $signedFile)) {
    throw "CodeSignTool did not produce expected signed file: $signedFile"
}

Move-Item -Force -LiteralPath $signedFile -Destination $resolvedFile.Path
Remove-Item -Force -Recurse -LiteralPath $outputDirectory
