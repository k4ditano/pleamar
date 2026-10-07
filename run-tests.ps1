param([string]$Binary)
$ErrorActionPreference = 'Stop'
$arguments = @((Join-Path $PSScriptRoot 'scripts/run-tests.py'))
if ($Binary) { $arguments += @('--binary', $Binary) }
python @arguments
exit $LASTEXITCODE
