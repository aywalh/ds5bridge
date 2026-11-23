# DS5 watcher: shows live when a DualSense connects or disconnects and what Windows logs about it
# (Bluetooth pairing, authentication, driver setup). Read-only; helps diagnose Bluetooth drops.
# Run:  powershell -ExecutionPolicy Bypass -File tools\ds5-watch.ps1
# Also writes tools\ds5-watch.log (ignored by git: it contains your device addresses).
$log = Join-Path $PSScriptRoot 'ds5-watch.log'
$filter = "PNPDeviceID LIKE '%054C%' OR Name LIKE '%DualSense%' OR Name LIKE '%Wireless Controller%'"
$sysProviders = 'BTHUSB', 'HidBth', 'BTHPORT', 'BthEnum', 'BTHMINI', 'bthserv'
$extraLogs = 'Microsoft-Windows-Bluetooth-BthLEPrepairing/Operational', 'Microsoft-Windows-Bluetooth-Policy/Operational', 'Microsoft-Windows-Kernel-PnP/Configuration' |
    Where-Object { Get-WinEvent -ListLog $_ -ErrorAction SilentlyContinue }

function Out-Line($text) {
    $line = "[{0:HH:mm:ss}] {1}" -f (Get-Date), $text
    Write-Output $line
    Add-Content -Path $log -Value $line -Encoding utf8
}
function Get-Bus($id) { if ($id -match '^BTH|00001124-0000-1000-8000-00805F9B34FB') { 'Bluetooth' } elseif ($id -match '^USB|^HID') { 'USB' } else { '?' } }

$before = @{}
Get-CimInstance Win32_PnPEntity -Filter $filter | ForEach-Object { $before[$_.PNPDeviceID] = $_ }
$last = Get-Date
Out-Line ("Watching. Connected now: " + (($before.Values | Where-Object { $_.Status -eq 'OK' } | ForEach-Object { "$($_.Name) ($(Get-Bus $_.PNPDeviceID))" } | Sort-Object -Unique) -join ', '))

while ($true) {
    Start-Sleep -Milliseconds 700
    # 1) devices coming and going
    $now = @{}
    Get-CimInstance Win32_PnPEntity -Filter $filter -ErrorAction SilentlyContinue | ForEach-Object { $now[$_.PNPDeviceID] = $_ }
    foreach ($id in $now.Keys) {
        $d = $now[$id]
        if (-not $before.ContainsKey($id)) { Out-Line "CONNECTED     $($d.Name) [$(Get-Bus $id)] status=$($d.Status) error=$($d.ConfigManagerErrorCode)" }
        elseif ($before[$id].Status -ne $d.Status -or $before[$id].ConfigManagerErrorCode -ne $d.ConfigManagerErrorCode) {
            Out-Line "CHANGED       $($d.Name) [$(Get-Bus $id)] status $($before[$id].Status) -> $($d.Status), error $($d.ConfigManagerErrorCode)"
        }
    }
    foreach ($id in $before.Keys) { if (-not $now.ContainsKey($id)) { Out-Line "DISCONNECTED  $($before[$id].Name) [$(Get-Bus $id)]" } }
    $before = $now

    # 2) what Windows logs meanwhile (BTHUSB 16 = authentication failed, HidBth 4 = first connection failed)
    $since = $last; $last = Get-Date
    $events = @(Get-WinEvent -FilterHashtable @{ LogName = 'System'; ProviderName = $sysProviders; StartTime = $since } -ErrorAction SilentlyContinue)
    foreach ($ln in $extraLogs) {
        $events += Get-WinEvent -FilterHashtable @{ LogName = $ln; StartTime = $since } -ErrorAction SilentlyContinue |
            Where-Object { $_.LogName -notmatch 'Kernel-PnP' -or $_.Message -match '054C' }
    }
    $events | Sort-Object TimeCreated | ForEach-Object {
        $msg = ($_.Message -replace '\s+', ' ').Trim()
        if ($msg.Length -gt 160) { $msg = $msg.Substring(0, 160) + ' ...' }
        Out-Line ("WINDOWS       {0} {1} [{2}]: {3}" -f $_.ProviderName, $_.Id, $_.LevelDisplayName, $msg)
    }
}
