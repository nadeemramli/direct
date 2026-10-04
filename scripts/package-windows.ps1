param(
    [string]$OutputRoot,
    [switch]$SkipChecks,
    [switch]$Replace
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
if (-not $OutputRoot) { $OutputRoot = Join-Path $repo 'target\packages' }
$OutputRoot = [IO.Path]::GetFullPath($OutputRoot)

function Assert-LastExitCode([string]$Step) {
    if ($LASTEXITCODE -ne 0) { throw "$Step failed with exit code $LASTEXITCODE" }
}

Push-Location $repo
try {
    if (-not $SkipChecks) {
        & cargo test -p direct-core -p direct
        Assert-LastExitCode 'Rust tests'
        & cargo clippy --workspace --all-targets -- -D warnings
        Assert-LastExitCode 'Clippy'
        & cargo fmt --all --check
        Assert-LastExitCode 'Rust formatting'
    }

    Push-Location (Join-Path $repo 'app')
    try {
        & npm.cmd ci
        Assert-LastExitCode 'npm ci'
        & npm.cmd run check
        Assert-LastExitCode 'Frontend checks'
        & npm.cmd run build
        Assert-LastExitCode 'Frontend build'
    } finally {
        Pop-Location
    }

    & cargo build --release -p direct -p direct-desktop --features direct-desktop/custom-protocol
    Assert-LastExitCode 'Windows release build'

    $metadataText = & cargo metadata --no-deps --format-version 1
    Assert-LastExitCode 'Cargo metadata'
    $metadata = $metadataText | ConvertFrom-Json
    $version = ($metadata.packages | Where-Object name -eq 'direct' | Select-Object -First 1).version
    if (-not $version) { throw 'Could not determine the Direct package version' }

    $commit = (& git -c "safe.directory=$repo" -C $repo rev-parse HEAD).Trim()
    Assert-LastExitCode 'Git revision lookup'
    $dirty = [bool](& git -c "safe.directory=$repo" -C $repo status --porcelain)
    Assert-LastExitCode 'Git status lookup'
    $suffix = if ($dirty) { '-dirty' } else { '' }
    $packageName = "Direct-$version-windows-x64-$($commit.Substring(0, 12))$suffix"
    $packageDir = Join-Path $OutputRoot $packageName
    $archivePath = "$packageDir.zip"

    New-Item -ItemType Directory -Force -Path $OutputRoot | Out-Null
    if ((Test-Path -LiteralPath $packageDir) -or (Test-Path -LiteralPath $archivePath)) {
        if (-not $Replace) {
            throw "Package output already exists: $packageDir. Use -Replace to rebuild it."
        }
        $rootPrefix = $OutputRoot.TrimEnd('\') + '\'
        foreach ($target in @($packageDir, $archivePath)) {
            $resolved = [IO.Path]::GetFullPath($target)
            if (-not $resolved.StartsWith($rootPrefix, [StringComparison]::OrdinalIgnoreCase)) {
                throw "Refusing to replace output outside $OutputRoot"
            }
            if (Test-Path -LiteralPath $resolved) { Remove-Item -LiteralPath $resolved -Recurse -Force }
        }
    }

    New-Item -ItemType Directory -Path (Join-Path $packageDir 'web') -Force | Out-Null
    Copy-Item -LiteralPath (Join-Path $repo 'target\release\direct.exe') -Destination $packageDir
    Copy-Item -LiteralPath (Join-Path $repo 'target\release\direct-desktop.exe') -Destination $packageDir
    Copy-Item -Path (Join-Path $repo 'app\dist\*') -Destination (Join-Path $packageDir 'web') -Recurse
    Copy-Item -LiteralPath (Join-Path $repo 'scripts\install-windows.ps1') -Destination (Join-Path $packageDir 'Install-Direct.ps1')
    Copy-Item -LiteralPath (Join-Path $repo 'scripts\smoke-windows-package.ps1') -Destination (Join-Path $packageDir 'Test-DirectPackage.ps1')
    Copy-Item -LiteralPath (Join-Path $repo 'packaging\windows\README.txt') -Destination (Join-Path $packageDir 'README.txt')

    $files = @(Get-ChildItem -LiteralPath $packageDir -Recurse -File | ForEach-Object {
        [ordered]@{
            path = [IO.Path]::GetRelativePath($packageDir, $_.FullName).Replace('\', '/')
            length = $_.Length
            sha256 = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
        }
    })
    $manifest = [ordered]@{
        format = 1
        product = 'Direct'
        version = $version
        platform = 'windows-x64'
        commit = $commit
        dirty = $dirty
        built_at_utc = [DateTime]::UtcNow.ToString('o')
        files = $files
    }
    $manifest | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $packageDir 'manifest.json') -Encoding utf8
    Compress-Archive -Path (Join-Path $packageDir '*') -DestinationPath $archivePath

    [ordered]@{
        package = $packageDir
        archive = $archivePath
        version = $version
        commit = $commit
        dirty = $dirty
        file_count = $files.Count
    } | ConvertTo-Json
} finally {
    Pop-Location
}
