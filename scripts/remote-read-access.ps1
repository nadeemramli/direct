param(
    [Parameter(Mandatory)][ValidateSet('New', 'Revoke', 'Inspect')][string]$Action,
    [Parameter(Mandatory)][ValidatePattern('^[a-zA-Z0-9-]{1,80}$')][string]$Session,
    [string]$Assigned,
    [string[]]$References = @(),
    [ValidateRange(1,24)][int]$Hours = 4,
    [string]$AccessRoot = (Join-Path $env:USERPROFILE '.direct\remote-read')
)
$ErrorActionPreference = 'Stop'
$accessPath = Join-Path $AccessRoot $Session
$repoPath = [IO.Path]::GetFullPath((Split-Path -Parent $PSScriptRoot)).TrimEnd('\','/')
$resolvedAccess = [IO.Path]::GetFullPath($accessPath)
if ($resolvedAccess.Equals($repoPath,[StringComparison]::OrdinalIgnoreCase) -or $resolvedAccess.StartsWith($repoPath+[IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase)) { throw 'Credentials must be stored outside the repository' }
$policyPath = Join-Path $accessPath 'policy.json'
if ($Action -eq 'New') {
    if ($Assigned -notmatch '^([A-Z][A-Z0-9]{0,15})-[1-9][0-9]{0,9}$') { throw 'Assigned issue key required' }
    $product = $Matches[1]
    $keys = @($Assigned) + $References
    if ($keys.Count -gt 8 -or @($keys | Select-Object -Unique).Count -ne $keys.Count) { throw 'One assigned issue and at most seven distinct reference issues' }
    foreach ($key in $keys) { if ($key -notmatch ('^'+$product+'-[1-9][0-9]{0,9}$')) { throw 'All keys must belong to one product' } }
    if (Test-Path -LiteralPath $accessPath) { throw 'Session already exists; revoke it and choose a new session ID' }
    New-Item -ItemType Directory -Path $accessPath -Force | Out-Null
    # Protect the directory BEFORE writing credentials. No capabilities are read from Direct.
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
    & icacls.exe $accessPath /inheritance:r /grant:r ('*'+$identity+':(OI)(CI)F') '*S-1-5-18:(OI)(CI)F' *> $null
    if ($LASTEXITCODE -ne 0) { throw 'Could not protect access directory; no credential generated' }
    $bytes = New-Object byte[] 32
    $rng = [Security.Cryptography.RandomNumberGenerator]::Create()
    try { $rng.GetBytes($bytes) } finally { $rng.Dispose() }
    $token = -join ($bytes | ForEach-Object { $_.ToString('x2') })
    $sha = [Security.Cryptography.SHA256]::Create()
    try { $hash = -join ($sha.ComputeHash([Text.Encoding]::UTF8.GetBytes($token)) | ForEach-Object { $_.ToString('x2') }) } finally { $sha.Dispose() }
    $issued = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
    $policy = @{ session=$Session; product=$product; assigned=$Assigned; references=@($References); token_sha256=$hash; issued_at=$issued; expires_at=($issued+$Hours*3600); revoked=$false }
    $policy | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $policyPath -Encoding utf8NoBOM
    # For secure environment-secret entry only. Never paste into a prompt, URL or log.
    @{ DIRECT_REMOTE_READ_TOKEN=$token } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $accessPath 'client-secret.json') -Encoding utf8NoBOM
    Write-Output "Created scoped access. Policy: $policyPath; protected client secret: $(Join-Path $accessPath 'client-secret.json')"
} elseif ($Action -eq 'Revoke') {
    $policy = Get-Content -LiteralPath $policyPath -Raw | ConvertFrom-Json
    $policy.revoked = $true
    $temporary = Join-Path $accessPath 'policy.revoked.tmp'
    $policy | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $temporary -Encoding utf8NoBOM
    Move-Item -LiteralPath $temporary -Destination $policyPath -Force
    Write-Output "Revoked $Session. Direct does not need a restart."
} else {
    Get-Content -LiteralPath $policyPath -Raw | ConvertFrom-Json | Select-Object session,product,assigned,references,issued_at,expires_at,revoked
}
