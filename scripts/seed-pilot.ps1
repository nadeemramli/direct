param([string]$DataDir = "$env:LOCALAPPDATA\Direct")
$ErrorActionPreference = 'Stop'
$cli = Join-Path (Split-Path -Parent $PSScriptRoot) 'target\debug\direct.exe'
function Send-Direct($command) {
    $json = $command | ConvertTo-Json -Depth 12 -Compress
    $output = $json | & $cli --data-dir $DataDir call
    if ($LASTEXITCODE -ne 0) { throw "Direct command failed: $($command.op)" }
    return ($output | ConvertFrom-Json)
}
$items = Get-Content -Raw (Join-Path $PSScriptRoot 'pilot-backlog.json') | ConvertFrom-Json
foreach ($item in $items) {
    $created = Send-Direct @{actor='codex-pilot';request_id="pilot-2026-09-29-create-$($item.id)";op='create_issue';product='DIR';title=$item.title;body=$item.body}
    $ctx = Send-Direct @{actor='codex-pilot';op='context';key=$created.key}
    # Reruns must not overwrite work that has moved on since initial capture.
    if ($ctx.issue.version -eq 1) {
        $null = Send-Direct @{actor='codex-pilot';request_id="pilot-2026-09-29-shape-$($item.id)";op='update_issue';key=$created.key;expected_version=1;title=$item.title;body=$item.body;acceptance=$item.acceptance;owner='Nadeem';priority=$item.priority}
    }
    Write-Output "$($created.key) $($item.title)"
}
