param(
    [Parameter(Mandatory = $true)]
    [string[]]$FilePath
)

$ErrorActionPreference = "Stop"

Update-MpSignature
$status = Get-MpComputerStatus
if (-not $status.AntivirusEnabled) {
    throw "Microsoft Defender Antivirus must be enabled for the release scan"
}
if (-not $status.AntivirusSignatureVersion) {
    throw "Microsoft Defender Antivirus signatures are unavailable"
}

$resolvedFiles = @(
    $FilePath |
        ForEach-Object { Resolve-Path -LiteralPath $_ }
)
if ($resolvedFiles.Count -eq 0) {
    throw "At least one Windows release file is required"
}

$temporaryRoot = if ([string]::IsNullOrWhiteSpace($env:RUNNER_TEMP)) {
    [IO.Path]::GetTempPath()
} else {
    $env:RUNNER_TEMP
}
$scanRoot = Join-Path `
    $temporaryRoot `
    "baijimu-defender-$([guid]::NewGuid().ToString('N'))"
New-Item -ItemType Directory -Force -Path $scanRoot | Out-Null

try {
    foreach ($resolvedFile in $resolvedFiles) {
        $signature = Get-AuthenticodeSignature -LiteralPath $resolvedFile.Path
        if ($signature.Status -ne "Valid") {
            throw (
                "Invalid Authenticode signature for {0}: {1}" -f
                    $resolvedFile.Path,
                    $signature.Status
            )
        }

        $scanFile = Join-Path `
            $scanRoot `
            ([IO.Path]::GetFileName($resolvedFile.Path))
        Copy-Item -Force -LiteralPath $resolvedFile.Path -Destination $scanFile

        $zoneIdentifier = @"
[ZoneTransfer]
ZoneId=3
HostUrl=https://www.baijimu.com/download/
ReferrerUrl=https://www.baijimu.com/download/
"@
        Set-Content `
            -LiteralPath $scanFile `
            -Stream "Zone.Identifier" `
            -Value $zoneIdentifier `
            -Encoding ASCII

        $startedAt = Get-Date
        Start-MpScan -ScanType CustomScan -ScanPath $scanFile
        Start-Sleep -Seconds 2

        $detections = @(
            Get-MpThreatDetection |
                Where-Object {
                    $_.InitialDetectionTime -ge $startedAt.AddSeconds(-5) -and
                    (
                        $_.Resources |
                            Where-Object {
                                $_ -like "*$([IO.Path]::GetFileName($scanFile))*"
                            }
                    )
                }
        )
        if ($detections.Count -gt 0 -or -not (Test-Path -LiteralPath $scanFile)) {
            $threats = @(
                $detections |
                    ForEach-Object {
                        $catalog = Get-MpThreatCatalog `
                            -ThreatID $_.ThreatID `
                            -ErrorAction SilentlyContinue
                        if ($catalog) {
                            "$($catalog.ThreatName) [$($_.ThreatID)]"
                        } else {
                            "Threat ID $($_.ThreatID)"
                        }
                    }
            ) | Sort-Object -Unique
            if ($threats.Count -eq 0) {
                $threats = @("file removed or quarantined during scan")
            }
            throw "Microsoft Defender rejected $($resolvedFile.Path): $($threats -join ', ')"
        }

        Write-Host (
            "Microsoft Defender accepted {0} with signatures {1}" -f
                $resolvedFile.Path,
                $status.AntivirusSignatureVersion
        )
    }
} finally {
    Remove-Item -Force -Recurse -LiteralPath $scanRoot `
        -ErrorAction SilentlyContinue
}
