function Update-DefenderSignaturesWithRetry {
    param(
        [Parameter(Mandatory = $true)]
        [ValidateRange(1, 10)]
        [int]$MaxAttempts,

        [Parameter(Mandatory = $true)]
        [ValidateRange(0, 300)]
        [int]$RetrySeconds
    )

    for ($attempt = 1; $attempt -le $MaxAttempts; $attempt++) {
        try {
            Write-Host (
                "Updating Microsoft Defender signatures (attempt {0}/{1})" -f
                    $attempt,
                    $MaxAttempts
            )
            Update-MpSignature -ErrorAction Stop
            return
        } catch {
            $failureMessage = $_.Exception.Message
            if ($attempt -eq $MaxAttempts) {
                throw (
                    "Microsoft Defender signature update failed after {0} attempts: {1}" -f
                        $MaxAttempts,
                        $failureMessage
                )
            }

            Write-Warning (
                "Microsoft Defender signature update attempt {0}/{1} failed: {2}. Retrying in {3} seconds." -f
                    $attempt,
                    $MaxAttempts,
                    $failureMessage,
                    $RetrySeconds
            )
            if ($RetrySeconds -gt 0) {
                Start-Sleep -Seconds $RetrySeconds
            }
        }
    }
}
