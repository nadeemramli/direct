param(
    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]]$DirectArgs
)

$repo = Split-Path -Parent $PSScriptRoot
$candidates = [System.Collections.Generic.List[string]]::new()
$candidates.Add((Join-Path $repo "target\debug\direct.exe"))

$commonDir = & git -c "safe.directory=$repo" -C $repo rev-parse --path-format=absolute --git-common-dir 2>$null
if ($LASTEXITCODE -eq 0 -and $commonDir) {
    $primaryRepo = Split-Path -Parent $commonDir.Trim()
    $candidates.Add((Join-Path $primaryRepo "target\debug\direct.exe"))
}

$cli = $candidates | Select-Object -Unique | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
if (-not $cli) {
    throw "Direct CLI not found in this checkout or its primary Git worktree. Build Direct on Windows first with scripts\start.ps1."
}

& $cli @DirectArgs
exit $LASTEXITCODE
