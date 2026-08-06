$ErrorActionPreference = "Stop"

. "$PSScriptRoot/update-defender-signatures.ps1"

function Assert-Equal {
    param(
        [Parameter(Mandatory = $true)]
        $Expected,

        [Parameter(Mandatory = $true)]
        $Actual,

        [Parameter(Mandatory = $true)]
        [string]$Message
    )

    if ($Expected -ne $Actual) {
        throw "$Message. Expected '$Expected', got '$Actual'"
    }
}

$global:DefenderUpdateCalls = 0
$global:DefenderSleepCalls = 0
$global:DefenderSleepSeconds = @()
function global:Update-MpSignature {
    [CmdletBinding()]
    param()

    $global:DefenderUpdateCalls++
    if ($global:DefenderUpdateCalls -lt 3) {
        throw "transient Defender RPC failure"
    }
}
function global:Start-Sleep {
    [CmdletBinding()]
    param(
        [int]$Seconds
    )

    $global:DefenderSleepCalls++
    $global:DefenderSleepSeconds += $Seconds
}

Update-DefenderSignaturesWithRetry -MaxAttempts 3 -RetrySeconds 1
Assert-Equal 3 $global:DefenderUpdateCalls "Transient failure was not retried"
Assert-Equal 2 $global:DefenderSleepCalls "Retry delay count was incorrect"
Assert-Equal "1,1" ($global:DefenderSleepSeconds -join ",") "Retry delay was not forwarded"

$global:DefenderUpdateCalls = 0
$global:DefenderSleepCalls = 0
$global:DefenderSleepSeconds = @()
function global:Update-MpSignature {
    [CmdletBinding()]
    param()

    $global:DefenderUpdateCalls++
    throw "persistent Defender RPC failure"
}

$terminalFailure = $null
try {
    Update-DefenderSignaturesWithRetry -MaxAttempts 3 -RetrySeconds 1
} catch {
    $terminalFailure = $_.Exception.Message
}

Assert-Equal 3 $global:DefenderUpdateCalls "Persistent failure did not exhaust retries"
Assert-Equal 2 $global:DefenderSleepCalls "Persistent failure retry delay count was incorrect"
if ($terminalFailure -notlike "*failed after 3 attempts*persistent Defender RPC failure*") {
    throw "Persistent failure did not produce the expected terminal error: $terminalFailure"
}

Write-Host "Defender signature retry tests passed"
