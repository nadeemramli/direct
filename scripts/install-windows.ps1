param(
    [string]$PackageDir = $PSScriptRoot,
    [string]$InstallDir,
    [string]$DataDir,
    [switch]$NoShortcut,
    [switch]$NoLaunch,
    [switch]$ForceStop
)

$ErrorActionPreference = 'Stop'
if (-not $InstallDir) {
    if (-not $env:USERPROFILE) { throw 'USERPROFILE is required unless -InstallDir is supplied' }
    $InstallDir = Join-Path $env:USERPROFILE '.direct\app'
}
$PackageDir = [IO.Path]::GetFullPath($PackageDir)
$InstallDir = [IO.Path]::GetFullPath($InstallDir)
if ($DataDir) { $DataDir = [IO.Path]::GetFullPath($DataDir) }
$manifestPath = Join-Path $PackageDir 'manifest.json'
if (-not (Test-Path -LiteralPath $manifestPath)) { throw "Package manifest not found: $manifestPath" }
$manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
if ($manifest.format -ne 1 -or $manifest.product -ne 'Direct') { throw 'Unsupported Direct package manifest' }

$packagePrefix = $PackageDir.TrimEnd('\') + '\'
foreach ($entry in $manifest.files) {
    $source = [IO.Path]::GetFullPath((Join-Path $PackageDir $entry.path))
    if (-not $source.StartsWith($packagePrefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Package path escapes its root: $($entry.path)"
    }
    if (-not (Test-Path -LiteralPath $source -PathType Leaf)) { throw "Package file is missing: $($entry.path)" }
    if ((Get-Item -LiteralPath $source).Length -ne $entry.length) { throw "Package file length mismatch: $($entry.path)" }
    $hash = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($hash -ne $entry.sha256) { throw "Package checksum mismatch: $($entry.path)" }
}

function Get-ExactProcess([string]$Name, [string]$ExpectedPath) {
    @(Get-Process -Name $Name -ErrorAction SilentlyContinue | Where-Object {
        try { [IO.Path]::GetFullPath($_.Path) -eq [IO.Path]::GetFullPath($ExpectedPath) } catch { $false }
    })
}

function Wait-ExactProcessExit([string]$Name, [string]$ExpectedPath, [int]$Seconds) {
    $deadline = [DateTime]::UtcNow.AddSeconds($Seconds)
    do {
        $running = @(Get-ExactProcess $Name $ExpectedPath)
        if ($running.Count -eq 0) { return $true }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)
    return $false
}

$parent = Split-Path -Parent $InstallDir
New-Item -ItemType Directory -Force -Path $parent | Out-Null
$staging = Join-Path $parent ('.direct-update-' + [Guid]::NewGuid().ToString('N'))
$backup = $null
try {
    New-Item -ItemType Directory -Path $staging | Out-Null
    foreach ($entry in $manifest.files) {
        $source = Join-Path $PackageDir $entry.path
        $destination = Join-Path $staging $entry.path
        $destinationParent = Split-Path -Parent $destination
        if ($destinationParent) { New-Item -ItemType Directory -Force -Path $destinationParent | Out-Null }
        Copy-Item -LiteralPath $source -Destination $destination
    }
    Copy-Item -LiteralPath $manifestPath -Destination (Join-Path $staging 'manifest.json')

    if (Test-Path -LiteralPath $InstallDir) {
        $desktopPath = Join-Path $InstallDir 'direct-desktop.exe'
        foreach ($process in @(Get-ExactProcess 'direct-desktop' $desktopPath)) {
            $null = $process.CloseMainWindow()
        }
        if (-not (Wait-ExactProcessExit 'direct-desktop' $desktopPath 5)) {
            foreach ($process in @(Get-ExactProcess 'direct-desktop' $desktopPath)) {
                Stop-Process -Id $process.Id -Force
            }
        }

        $cliPath = Join-Path $InstallDir 'direct.exe'
        $serviceProcesses = @(Get-ExactProcess 'direct' $cliPath)
        if ($serviceProcesses.Count -gt 0) {
            $stopArgs = @()
            if ($DataDir) { $stopArgs += @('--data-dir', $DataDir) }
            $stopArgs += 'stop'
            & $cliPath @stopArgs
            $stopSucceeded = $LASTEXITCODE -eq 0
            if (-not $stopSucceeded -and -not $ForceStop) {
                throw 'The installed Direct service did not accept a graceful stop. Retry with -ForceStop only after confirming no write is active.'
            }
            if (-not (Wait-ExactProcessExit 'direct' $cliPath 10)) {
                if (-not $ForceStop) { throw 'The installed Direct service did not stop within 10 seconds' }
                foreach ($process in @(Get-ExactProcess 'direct' $cliPath)) {
                    Stop-Process -Id $process.Id -Force
                }
            }
        }

        $backup = "$InstallDir.previous-$([DateTime]::UtcNow.ToString('yyyyMMddHHmmss'))"
        Move-Item -LiteralPath $InstallDir -Destination $backup
    }

    try {
        Move-Item -LiteralPath $staging -Destination $InstallDir
    } catch {
        if ($backup -and -not (Test-Path -LiteralPath $InstallDir) -and (Test-Path -LiteralPath $backup)) {
            Move-Item -LiteralPath $backup -Destination $InstallDir
        }
        throw
    }

    if (-not $NoShortcut) {
        $startMenu = [Environment]::GetFolderPath('StartMenu')
        if (-not $startMenu) { throw 'Could not resolve the current user Start Menu' }
        $programs = Join-Path $startMenu 'Programs'
        New-Item -ItemType Directory -Force -Path $programs | Out-Null
        $shortcutPath = Join-Path $programs 'Direct.lnk'
        $shell = New-Object -ComObject WScript.Shell
        $shortcut = $shell.CreateShortcut($shortcutPath)
        $shortcut.TargetPath = Join-Path $InstallDir 'direct-desktop.exe'
        $shortcut.WorkingDirectory = $InstallDir
        $shortcut.IconLocation = (Join-Path $InstallDir 'direct-desktop.exe') + ',0'
        $shortcut.Description = 'Direct local work tracking'
        $shortcut.Save()
    }

    if (-not $NoLaunch) {
        $priorDataDir = $env:DIRECT_DATA_DIR
        try {
            if ($DataDir) { $env:DIRECT_DATA_DIR = $DataDir }
            Start-Process -FilePath (Join-Path $InstallDir 'direct-desktop.exe') | Out-Null
        } finally {
            $env:DIRECT_DATA_DIR = $priorDataDir
        }
    }

    [ordered]@{
        installed = $true
        version = $manifest.version
        commit = $manifest.commit
        install_dir = $InstallDir
        previous_install = $backup
        shortcut_created = -not $NoShortcut
        launched = -not $NoLaunch
    } | ConvertTo-Json
} finally {
    if (Test-Path -LiteralPath $staging) {
        $stagingPrefix = $parent.TrimEnd('\') + '\.direct-update-'
        if ($staging.StartsWith($stagingPrefix, [StringComparison]::OrdinalIgnoreCase)) {
            Remove-Item -LiteralPath $staging -Recurse -Force
        }
    }
}
