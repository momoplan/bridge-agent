param(
    [Parameter(Mandatory = $true)]
    [string]$FilePath
)

$ErrorActionPreference = "Stop"

function Read-MsiRows {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Database,
        [Parameter(Mandatory = $true)]
        [string]$Query,
        [Parameter(Mandatory = $true)]
        [int]$ColumnCount
    )

    $view = $Database.GetType().InvokeMember(
        "OpenView",
        [System.Reflection.BindingFlags]::InvokeMethod,
        $null,
        $Database,
        @($Query)
    )
    try {
        $view.GetType().InvokeMember(
            "Execute",
            [System.Reflection.BindingFlags]::InvokeMethod,
            $null,
            $view,
            $null
        ) | Out-Null

        $rows = @()
        while ($true) {
            $record = $view.GetType().InvokeMember(
                "Fetch",
                [System.Reflection.BindingFlags]::InvokeMethod,
                $null,
                $view,
                $null
            )
            if ($null -eq $record) {
                break
            }

            $row = @()
            for ($index = 1; $index -le $ColumnCount; $index++) {
                $row += $record.GetType().InvokeMember(
                    "StringData",
                    [System.Reflection.BindingFlags]::GetProperty,
                    $null,
                    $record,
                    @($index)
                )
            }
            $rows += [pscustomobject]@{
                Values = $row
            }
        }
        return $rows
    } finally {
        $view.GetType().InvokeMember(
            "Close",
            [System.Reflection.BindingFlags]::InvokeMethod,
            $null,
            $view,
            $null
        ) | Out-Null
    }
}

$resolvedMsi = Resolve-Path -LiteralPath $FilePath
$installer = New-Object -ComObject WindowsInstaller.Installer
$database = $installer.GetType().InvokeMember(
    "OpenDatabase",
    [System.Reflection.BindingFlags]::InvokeMethod,
    $null,
    $installer,
    @($resolvedMsi.Path, 0)
)

$fileRows = Read-MsiRows `
    -Database $database `
    -Query 'SELECT `FileName` FROM `File`' `
    -ColumnCount 1
$serviceFile = $fileRows |
    ForEach-Object { ($_.Values[0] -split '\|')[-1] } |
    Where-Object { $_ -ieq "bridge-agent-service.exe" } |
    Select-Object -First 1
if (-not $serviceFile) {
    throw "MSI does not contain bridge-agent-service.exe"
}

$serviceInstallRows = Read-MsiRows `
    -Database $database `
    -Query 'SELECT `Name`, `ServiceType`, `StartType`, `ErrorControl`, `Arguments` FROM `ServiceInstall`' `
    -ColumnCount 5
$bridgeServiceRow = $serviceInstallRows |
    Where-Object { $_.Values[0] -ieq "BridgeAgent" } |
    Select-Object -First 1
if (-not $bridgeServiceRow) {
    throw "MSI ServiceInstall table does not register BridgeAgent"
}
$bridgeService = $bridgeServiceRow.Values
if ([int]$bridgeService[1] -ne 16) {
    throw "BridgeAgent ServiceType must be own-process (16), got $($bridgeService[1])"
}
if ([int]$bridgeService[2] -ne 2) {
    throw "BridgeAgent StartType must be automatic (2), got $($bridgeService[2])"
}
if ($bridgeService[4] -notlike "*agent-config.json*") {
    throw "BridgeAgent service arguments do not target the shared agent config"
}

$serviceControlRows = Read-MsiRows `
    -Database $database `
    -Query 'SELECT `Name`, `Event`, `Wait` FROM `ServiceControl`' `
    -ColumnCount 3
$bridgeControlRow = $serviceControlRows |
    Where-Object { $_.Values[0] -ieq "BridgeAgent" } |
    Select-Object -First 1
if (-not $bridgeControlRow) {
    throw "MSI ServiceControl table does not manage BridgeAgent"
}
$bridgeControl = $bridgeControlRow.Values
$requiredEvents = 1 + 2 + 32 + 128
if (([int]$bridgeControl[1] -band $requiredEvents) -ne $requiredEvents) {
    throw "BridgeAgent ServiceControl must start on install and stop/remove during servicing"
}
if ([int]$bridgeControl[2] -ne 1) {
    throw "BridgeAgent ServiceControl must wait for transitions"
}

$registryRows = Read-MsiRows `
    -Database $database `
    -Query 'SELECT `Name`, `Value` FROM `Registry`' `
    -ColumnCount 2
$autoStart = $registryRows |
    Where-Object {
        $_.Values[0] -ieq "BaijimuBridgeAgent" -and
        $_.Values[1] -like "*bridge-agent-desktop.exe*"
    } |
    Select-Object -First 1
if ($autoStart) {
    throw "MSI still contains the legacy machine-wide desktop autostart entry"
}

Write-Host "Verified Windows MSI service contract and absence of legacy autostart: $($resolvedMsi.Path)"
