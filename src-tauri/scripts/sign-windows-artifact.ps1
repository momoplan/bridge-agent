param(
    [Parameter(Mandatory = $true)]
    [string]$FilePath
)

$ErrorActionPreference = "Stop"

function Protect-DiagnosticText {
    param([string]$Text)

    $protectedText = $Text
    foreach ($name in @("SSL_COM_USERNAME", "SSL_COM_PASSWORD", "SSL_COM_CREDENTIAL_ID", "SSL_COM_TOTP_SECRET")) {
        $secret = [Environment]::GetEnvironmentVariable($name)
        if (-not [string]::IsNullOrEmpty($secret)) {
            $protectedText = $protectedText.Replace($secret, "***")
        }
    }
    return $protectedText
}

function Write-SigningDiagnostic {
    param([string]$Message)

    if ([string]::IsNullOrWhiteSpace($env:WINDOWS_SIGNING_LOG_PATH)) {
        return
    }
    $diagnosticDirectory = Split-Path -Parent $env:WINDOWS_SIGNING_LOG_PATH
    if (-not [string]::IsNullOrWhiteSpace($diagnosticDirectory)) {
        New-Item -ItemType Directory -Force -Path $diagnosticDirectory | Out-Null
    }
    $safeMessage = Protect-DiagnosticText -Text $Message
    Add-Content `
        -LiteralPath $env:WINDOWS_SIGNING_LOG_PATH `
        -Value "$(Get-Date -Format o) $safeMessage" `
        -Encoding UTF8
}

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
$workingDirectory = Join-Path ([System.IO.Path]::GetTempPath()) "codesign-$([guid]::NewGuid().ToString('N'))"
$inputDirectory = Join-Path $workingDirectory "input"
$outputDirectory = Join-Path $workingDirectory "output"
New-Item -ItemType Directory -Force -Path $inputDirectory, $outputDirectory | Out-Null

# CodeSignTool's local file handling is not Unicode-safe even though its
# Authenticode program name supports Unicode. Stage every artifact under a
# deterministic ASCII-only leaf name, then move the signed bytes back to the
# original path. This keeps the public Chinese MSI name and metadata intact.
$stagedInputFile = Join-Path `
    $inputDirectory `
    "bridge-agent-signing-input$([System.IO.Path]::GetExtension($resolvedFile.Path))"
$replacementFile = "$($resolvedFile.Path).signed.tmp"
Copy-Item -Force -LiteralPath $resolvedFile.Path -Destination $stagedInputFile
Write-SigningDiagnostic "start extension=$([System.IO.Path]::GetExtension($resolvedFile.Path)) ascii_staging=true"

$arguments = @(
    "sign",
    "-username=$env:SSL_COM_USERNAME",
    "-password=$env:SSL_COM_PASSWORD",
    "-credential_id=$env:SSL_COM_CREDENTIAL_ID",
    "-totp_secret=$env:SSL_COM_TOTP_SECRET",
    "-input_file_path=$stagedInputFile",
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
try {
    Push-Location $codeSignToolDirectory
    try {
        $codeSignToolOutput = & $javaExecutable.FullName `
            "-Dfile.encoding=UTF-8" `
            "-jar" `
            $codeSignToolJar.FullName `
            @arguments 2>&1
        $codeSignToolExitCode = $LASTEXITCODE
    } finally {
        Pop-Location
    }

    foreach ($line in $codeSignToolOutput) {
        $safeLine = Protect-DiagnosticText -Text $line.ToString()
        Write-Host $safeLine
        Write-SigningDiagnostic "tool $safeLine"
    }
    Write-SigningDiagnostic "tool_exit_code=$codeSignToolExitCode"

    if ($codeSignToolExitCode -ne 0) {
        throw "CodeSignTool failed with exit code $codeSignToolExitCode"
    }

    $signedFile = Join-Path $outputDirectory (Split-Path -Leaf $stagedInputFile)
    if (-not (Test-Path -LiteralPath $signedFile)) {
        throw "CodeSignTool did not produce the expected signed artifact"
    }

    Copy-Item -Force -LiteralPath $signedFile -Destination $replacementFile
    [System.IO.File]::Replace($replacementFile, $resolvedFile.Path, $null)
    Write-SigningDiagnostic "complete"
} catch {
    Write-SigningDiagnostic "failure type=$($_.Exception.GetType().FullName) message=$($_.Exception.Message)"
    throw
} finally {
    if (Test-Path -LiteralPath $workingDirectory) {
        Remove-Item -Force -Recurse -LiteralPath $workingDirectory
    }
    if (Test-Path -LiteralPath $replacementFile) {
        Remove-Item -Force -LiteralPath $replacementFile
    }
}
