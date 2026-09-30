param(
    [switch]$SkipBuild,
    [switch]$Browser,
    [string]$DataDir
)
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
Push-Location $repo
try {
    if ($DataDir) { $env:DIRECT_DATA_DIR = [IO.Path]::GetFullPath($DataDir) }
    if (-not $SkipBuild) {
        Push-Location (Join-Path $repo 'app')
        try {
            & npm.cmd ci
            if ($LASTEXITCODE -ne 0) { throw 'npm ci failed' }
            & npm.cmd run check
            if ($LASTEXITCODE -ne 0) { throw 'Frontend checks failed' }
            & npm.cmd run build
            if ($LASTEXITCODE -ne 0) { throw 'Frontend build failed' }
        } finally { Pop-Location }
        & cargo build -p direct -p direct-desktop --features direct-desktop/custom-protocol
        if ($LASTEXITCODE -ne 0) { throw 'Rust build failed. Stop the existing Direct processes before rebuilding on Windows.' }
    }
    $cli = Join-Path $repo 'target\debug\direct.exe'
    $desktop = Join-Path $repo 'target\debug\direct-desktop.exe'
    if ($Browser) {
        & $cli list *> $null
        if ($LASTEXITCODE -ne 0) {
            $assets = Join-Path $repo 'app\dist'
            Start-Process -FilePath $cli -ArgumentList @('serve', '--assets', ('"' + $assets + '"')) -WindowStyle Hidden | Out-Null
            $running = $false
            for ($attempt = 0; $attempt -lt 40; $attempt++) {
                Start-Sleep -Milliseconds 100
                & $cli list *> $null
                if ($LASTEXITCODE -eq 0) { $running = $true; break }
            }
            if (-not $running) { throw 'Service did not start. Run target\debug\direct.exe serve to see diagnostics.' }
        }
        & $cli open
        if ($LASTEXITCODE -ne 0) { throw 'Unable to create browser launch link' }
    } else {
        Start-Process -FilePath $desktop
    }
} finally { Pop-Location }
