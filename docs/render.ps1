# Re-draws the README images from the app itself (demo values, no screen capture).
# Usage:  cargo build --release; powershell -ExecutionPolicy Bypass -File docs\render.ps1
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$target = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $root 'target' }
$exe = Join-Path $target 'release\ds5bridge.exe'
$jobs = @(
    @('banner', 'en', 'banner'),
    @('main', 'en', 'screenshot-main'),
    @('main', 'en', 'screenshot-settings'),
    @('main', 'en', 'screenshot-splash'),
    @('install', 'en', 'screenshot-install'),
    @('install', 'en', 'screenshot-install-options'),
    @('main', 'de', 'screenshot-main-de'),
    @('main', 'ja', 'screenshot-main-ja')
)
foreach ($j in $jobs) {
    $out = Join-Path $PSScriptRoot ($j[2] + '.png')
    $p = Start-Process -FilePath $exe -ArgumentList @('--render', $j[0], $j[1], "`"$out`"") -PassThru
    if (-not $p.WaitForExit(30000)) { $p.Kill(); throw "render timed out: $($j[2])" }
}
# the app writes uncompressed PNGs; re-save them compressed
Add-Type -AssemblyName System.Drawing
Get-ChildItem (Join-Path $PSScriptRoot '*.png') | ForEach-Object {
    $tmp = "$($_.FullName).tmp"
    $img = [System.Drawing.Image]::FromFile($_.FullName)
    $img.Save($tmp, [System.Drawing.Imaging.ImageFormat]::Png)
    $img.Dispose()
    Move-Item $tmp $_.FullName -Force
    '{0,-26} {1,5:N0} KB' -f $_.Name, ($_.Length / 1KB)
}
