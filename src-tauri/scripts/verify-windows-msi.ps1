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

function Test-MsiTable {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Database,
        [Parameter(Mandatory = $true)]
        [string]$Name
    )

    $tables = Read-MsiRows `
        -Database $Database `
        -Query 'SELECT `Name` FROM `_Tables`' `
        -ColumnCount 1
    return $null -ne ($tables |
        Where-Object { $_.Values[0] -ieq $Name } |
        Select-Object -First 1)
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

$productNameRow = Read-MsiRows `
    -Database $database `
    -Query "SELECT ``Value`` FROM ``Property`` WHERE ``Property`` = 'ProductName'" `
    -ColumnCount 1 |
    Select-Object -First 1
if (-not $productNameRow -or $productNameRow.Values[0] -cne "百积木") {
    $actualProductName = if ($productNameRow) { $productNameRow.Values[0] } else { "<missing>" }
    throw "MSI ProductName must be 百积木, found: $actualProductName"
}

$upgradeCodeRow = Read-MsiRows `
    -Database $database `
    -Query "SELECT ``Value`` FROM ``Property`` WHERE ``Property`` = 'UpgradeCode'" `
    -ColumnCount 1 |
    Select-Object -First 1
$expectedUpgradeCode = "{94895101-CD67-53B8-BB30-F95026802DF2}"
if (-not $upgradeCodeRow -or $upgradeCodeRow.Values[0] -cne $expectedUpgradeCode) {
    $actualUpgradeCode = if ($upgradeCodeRow) { $upgradeCodeRow.Values[0] } else { "<missing>" }
    throw "MSI UpgradeCode must preserve the existing Windows upgrade identity $expectedUpgradeCode, found: $actualUpgradeCode"
}

$productLanguageRow = Read-MsiRows `
    -Database $database `
    -Query "SELECT ``Value`` FROM ``Property`` WHERE ``Property`` = 'ProductLanguage'" `
    -ColumnCount 1 |
    Select-Object -First 1
$expectedProductLanguage = "2052"
if (-not $productLanguageRow -or $productLanguageRow.Values[0] -cne $expectedProductLanguage) {
    $actualProductLanguage = if ($productLanguageRow) { $productLanguageRow.Values[0] } else { "<missing>" }
    throw "MSI ProductLanguage must be zh-CN ($expectedProductLanguage), found: $actualProductLanguage"
}

$fileRows = Read-MsiRows `
    -Database $database `
    -Query 'SELECT `FileName` FROM `File`' `
    -ColumnCount 1
$serviceFile = $fileRows |
    ForEach-Object { ($_.Values[0] -split '\|')[-1] } |
    Where-Object { $_ -ieq "bridge-agent-service.exe" } |
    Select-Object -First 1
if ($serviceFile) {
    throw "MSI still contains the retired bridge-agent-service.exe"
}

$serviceInstallRows = @()
if (Test-MsiTable -Database $database -Name "ServiceInstall") {
    $serviceInstallRows = Read-MsiRows `
        -Database $database `
        -Query 'SELECT `Name`, `ServiceType`, `StartType`, `ErrorControl`, `Arguments` FROM `ServiceInstall`' `
        -ColumnCount 5
}
$bridgeServiceRow = $serviceInstallRows |
    Where-Object { $_.Values[0] -ieq "BridgeAgent" } |
    Select-Object -First 1
if ($bridgeServiceRow) {
    throw "MSI still registers the retired BridgeAgent Windows service"
}

$serviceControlRows = Read-MsiRows `
    -Database $database `
    -Query 'SELECT `Name`, `Event`, `Wait` FROM `ServiceControl`' `
    -ColumnCount 3
$bridgeControlRow = $serviceControlRows |
    Where-Object { $_.Values[0] -ieq "BridgeAgent" } |
    Select-Object -First 1
if (-not $bridgeControlRow) {
    throw "MSI does not clean up the legacy BridgeAgent service"
}
$bridgeControl = $bridgeControlRow.Values
$requiredEvents = 2 + 8
if (([int]$bridgeControl[1] -band $requiredEvents) -ne $requiredEvents) {
    throw "BridgeAgent ServiceControl must stop and remove the legacy service during install"
}
if (([int]$bridgeControl[1] -band 1) -ne 0) {
    throw "BridgeAgent ServiceControl must not start the retired service"
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

Write-Host "Verified Windows MSI desktop-owned runtime contract and legacy service cleanup: $($resolvedMsi.Path)"
