param(
    [Parameter(Mandatory = $true)]
    [string]$PackageDir,
    [string]$ReportPath,
    [switch]$KeepArtifacts
)

$ErrorActionPreference = 'Stop'
$PackageDir = [IO.Path]::GetFullPath($PackageDir)
$smokeRoot = Join-Path ([IO.Path]::GetTempPath()) ('direct-package-smoke-' + [Guid]::NewGuid().ToString('N'))
$installDir = Join-Path $smokeRoot 'app'
$dataDir = Join-Path $smokeRoot 'data'
$desktopProcess = $null
$checks = [System.Collections.Generic.List[string]]::new()
$started = [DateTime]::UtcNow

function Wait-ForService([string]$Cli, [string]$Workspace, [int]$Seconds = 20) {
    $deadline = [DateTime]::UtcNow.AddSeconds($Seconds)
    do {
        & $Cli --data-dir $Workspace list *> $null
        if ($LASTEXITCODE -eq 0) { return }
        Start-Sleep -Milliseconds 150
    } while ([DateTime]::UtcNow -lt $deadline)
    throw 'The packaged service did not become ready'
}

function Wait-ForWindow([Diagnostics.Process]$Process, [int]$Seconds = 20) {
    $deadline = [DateTime]::UtcNow.AddSeconds($Seconds)
    do {
        if ($Process.HasExited) { throw "The native desktop exited with code $($Process.ExitCode)" }
        $Process.Refresh()
        if ($Process.MainWindowHandle -ne 0) { return }
        Start-Sleep -Milliseconds 150
    } while ([DateTime]::UtcNow -lt $deadline)
    throw 'The native desktop did not expose a Windows window handle'
}

function Start-NativeDesktop([string]$Desktop, [string]$Workspace) {
    $prior = $env:DIRECT_DATA_DIR
    try {
        $env:DIRECT_DATA_DIR = $Workspace
        return Start-Process -FilePath $Desktop -PassThru
    } finally {
        $env:DIRECT_DATA_DIR = $prior
    }
}

function Close-NativeDesktop([Diagnostics.Process]$Process) {
    if (-not $Process -or $Process.HasExited) { return }
    $null = $Process.CloseMainWindow()
    if (-not $Process.WaitForExit(5000)) {
        Stop-Process -Id $Process.Id -Force
        $Process.WaitForExit()
    }
}

function Stop-Service([string]$Cli, [string]$Workspace) {
    & $Cli --data-dir $Workspace stop *> $null
    if ($LASTEXITCODE -ne 0) { throw 'The packaged service did not accept a graceful stop' }
    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    do {
        if (-not (Test-Path -LiteralPath (Join-Path $Workspace 'endpoint.json'))) { return }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)
    throw 'The packaged service did not remove its endpoint after graceful shutdown'
}

try {
    New-Item -ItemType Directory -Path $smokeRoot | Out-Null
    $installer = Join-Path $PackageDir 'Install-Direct.ps1'
    & $installer -PackageDir $PackageDir -InstallDir $installDir -DataDir $dataDir -NoShortcut -NoLaunch | Out-Null
    $checks.Add('manifest checksums and isolated installation passed')

    $cli = Join-Path $installDir 'direct.exe'
    $desktop = Join-Path $installDir 'direct-desktop.exe'
    $desktopProcess = Start-NativeDesktop $desktop $dataDir
    Wait-ForService $cli $dataDir
    Wait-ForWindow $desktopProcess
    $checks.Add('native desktop opened a Windows window and started the packaged service')

    $createdText = & $cli --data-dir $dataDir --actor package-smoke create 'Packaged desktop smoke' --body 'Synthetic isolated fixture'
    if ($LASTEXITCODE -ne 0) { throw 'Could not create the isolated smoke issue' }
    $created = ($createdText -join [Environment]::NewLine) | ConvertFrom-Json
    $key = $created.key
    if (-not $key) { throw 'The isolated smoke issue did not return a key' }
    $checks.Add("isolated fixture edit persisted as $key")

    Close-NativeDesktop $desktopProcess
    $desktopProcess = $null
    $contextBeforeRestartText = & $cli --data-dir $dataDir context $key
    $contextBeforeRestart = ($contextBeforeRestartText -join [Environment]::NewLine) | ConvertFrom-Json
    if ($contextBeforeRestart.issue.key -ne $key) { throw 'Agent access failed after closing the desktop' }
    $checks.Add('agent access remained available after the desktop closed')

    Stop-Service $cli $dataDir
    $checks.Add('service accepted graceful shutdown and removed its endpoint')

    $desktopProcess = Start-NativeDesktop $desktop $dataDir
    Wait-ForService $cli $dataDir
    Wait-ForWindow $desktopProcess
    $contextAfterRestartText = & $cli --data-dir $dataDir context $key
    $contextAfterRestart = ($contextAfterRestartText -join [Environment]::NewLine) | ConvertFrom-Json
    if ($contextAfterRestart.issue.key -ne $key) { throw 'Fixture data did not survive service restart' }
    $checks.Add('native relaunch restarted the service and preserved fixture data')

    Close-NativeDesktop $desktopProcess
    $desktopProcess = $null
    & $cli --data-dir $dataDir list *> $null
    if ($LASTEXITCODE -ne 0) { throw 'Agent access failed after the relaunched desktop closed' }
    $checks.Add('second desktop close again left agent access available')

    & $installer -PackageDir $PackageDir -InstallDir $installDir -DataDir $dataDir -NoShortcut -NoLaunch | Out-Null
    if (Test-Path -LiteralPath (Join-Path $dataDir 'endpoint.json')) {
        throw 'The staged update did not stop the installed service'
    }
    if (@(Get-ChildItem -LiteralPath $smokeRoot -Directory -Filter 'app.previous-*').Count -ne 1) {
        throw 'The staged update did not retain exactly one prior application directory'
    }
    $checks.Add('staged update gracefully stopped the installed service and retained the prior application')

    $desktopProcess = Start-NativeDesktop $desktop $dataDir
    Wait-ForService $cli $dataDir
    Wait-ForWindow $desktopProcess
    $contextAfterUpdateText = & $cli --data-dir $dataDir context $key
    $contextAfterUpdate = ($contextAfterUpdateText -join [Environment]::NewLine) | ConvertFrom-Json
    if ($contextAfterUpdate.issue.key -ne $key) { throw 'Fixture data did not survive the staged application update' }
    Close-NativeDesktop $desktopProcess
    $desktopProcess = $null
    Stop-Service $cli $dataDir
    $checks.Add('updated native package relaunched against the unchanged persistent workspace')

    $manifest = Get-Content -LiteralPath (Join-Path $PackageDir 'manifest.json') -Raw | ConvertFrom-Json
    $report = [ordered]@{
        passed = $true
        package = $PackageDir
        version = $manifest.version
        commit = $manifest.commit
        dirty = $manifest.dirty
        started_at_utc = $started.ToString('o')
        completed_at_utc = [DateTime]::UtcNow.ToString('o')
        workspace = if ($KeepArtifacts) { $smokeRoot } else { 'removed after successful isolated smoke' }
        checks = @($checks)
        owner_acceptance = 'not recorded; manual verification remains required'
    }
    $json = $report | ConvertTo-Json -Depth 4
    if ($ReportPath) {
        $reportFull = [IO.Path]::GetFullPath($ReportPath)
        $reportParent = Split-Path -Parent $reportFull
        if ($reportParent) { New-Item -ItemType Directory -Force -Path $reportParent | Out-Null }
        $json | Set-Content -LiteralPath $reportFull -Encoding utf8
    }
    $json
} finally {
    if ($desktopProcess) { Close-NativeDesktop $desktopProcess }
    $cli = Join-Path $installDir 'direct.exe'
    if ((Test-Path -LiteralPath $cli) -and (Test-Path -LiteralPath (Join-Path $dataDir 'endpoint.json'))) {
        try { Stop-Service $cli $dataDir } catch { }
    }
    if (-not $KeepArtifacts -and (Test-Path -LiteralPath $smokeRoot)) {
        $tempPrefix = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\') + '\direct-package-smoke-'
        if ($smokeRoot.StartsWith($tempPrefix, [StringComparison]::OrdinalIgnoreCase)) {
            Remove-Item -LiteralPath $smokeRoot -Recurse -Force
        }
    }
}
