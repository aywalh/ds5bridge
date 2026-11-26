param(
    [Parameter(Mandatory)][string] $Destination,
    [switch] $Offline
)

$ErrorActionPreference = 'Stop'
$project = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$destinationPath = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($Destination)
$oldFlags = $env:CARGO_ENCODED_RUSTFLAGS
$profilePath = $env:USERPROFILE
$targetPath = Join-Path $project 'target'
try {
    $remap = @("--remap-path-prefix=$profilePath=/build-user", "--remap-path-prefix=$project=/ds5bridge")
    $env:CARGO_ENCODED_RUSTFLAGS = $remap -join [char]31
    $flags = @('build', '--release', '--locked', '--no-default-features', '--manifest-path', (Join-Path $project 'Cargo.toml'), '--target-dir', $targetPath)
    if ($Offline) { $flags += '--offline' }
    & cargo @flags
    if ($LASTEXITCODE -ne 0) { throw "Cargo failed ($LASTEXITCODE)" }
} finally {
    $env:CARGO_ENCODED_RUSTFLAGS = $oldFlags
}

$built = Join-Path $targetPath 'release/ds5bridge.exe'
$bytes = [IO.File]::ReadAllBytes($built)
$encodings = @(
    [Text.Encoding]::Latin1.GetString($bytes),
    [Text.Encoding]::Unicode.GetString($bytes),
    [Text.Encoding]::Unicode.GetString($bytes, 1, $bytes.Length - 1)
)
$private = '(?i)([a-z]:[/\\]+Users[/\\]+[^/\\\x00\s]+|scratch-workspaces|[a-z0-9._%+\-]+@[a-z0-9.\-]+\.[a-z]{2,})'
foreach ($text in $encodings) {
    if ($text -match $private) { throw 'Executable contains a personal path or email-like string.' }
}
$usesLocalArt = $encodings[0].Contains('PS5-Bridge-controller-halo')
if ($usesLocalArt) { throw 'Local third-party artwork is present in the package.' }

New-Item -ItemType Directory -Path $destinationPath -Force | Out-Null
$artifact = Join-Path $destinationPath 'DS5Bridge.exe'
Copy-Item -LiteralPath $built -Destination $artifact -Force
$digest = (Get-FileHash -LiteralPath $artifact -Algorithm SHA256).Hash.ToLowerInvariant()
"$digest  DS5Bridge.exe" | Set-Content -LiteralPath "$artifact.sha256" -Encoding ascii
Write-Output "Verified $artifact"
