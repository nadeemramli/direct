param(
    [string]$BackupDirectory = (Join-Path $env:USERPROFILE ".direct\backups"),
    [ValidateRange(1, 3650)]
    [int]$Retain = 14
)

$repo = Split-Path -Parent $PSScriptRoot
& (Join-Path $PSScriptRoot "direct.ps1") backup $BackupDirectory --retain $Retain
exit $LASTEXITCODE
