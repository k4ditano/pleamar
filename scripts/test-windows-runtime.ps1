param([Parameter(Mandatory=$true)][string]$Archive)
$ErrorActionPreference = 'Stop'
$Archive = (Resolve-Path -LiteralPath $Archive).Path
$root = Join-Path ([IO.Path]::GetTempPath()) ('pleamar-runtime-test-' + [Guid]::NewGuid().ToString('N'))
$package = Join-Path $root ('runtime ' + [char]0x00f1 + [char]0x6d77 + ' with spaces')
New-Item -ItemType Directory -Path $package | Out-Null
$prepare = Join-Path $PSScriptRoot 'prepare-windows-runtime.ps1'
function Expect-Failure([scriptblock]$Action) {
    try { & $Action } catch { return }
    throw 'Expected the invalid runtime operation to fail.'
}
try {
    # The preparer does not run the binary; this fixture isolates packaging.
    Set-Content -LiteralPath (Join-Path $package 'pleamar.exe') -Value 'fixture'
    & $prepare -BinaryDirectory $package -Archive $Archive
    $manifest = Get-Content -LiteralPath (Join-Path $package 'dxc-runtime.json') -Raw | ConvertFrom-Json
    foreach ($entry in $manifest.files.PSObject.Properties) {
        if ((Get-FileHash -LiteralPath (Join-Path $package $entry.Name)).Hash -ne $entry.Value) { throw "Incorrect copied hash: $($entry.Name)" }
    }
    $before = (Get-FileHash -LiteralPath (Join-Path $package 'dxcompiler.dll')).Hash
    $badArchive = Join-Path $root 'truncated.zip'
    Set-Content -LiteralPath $badArchive -Value 'invalid package'
    Expect-Failure { & $prepare -BinaryDirectory $package -Archive $badArchive }
    if ((Get-FileHash -LiteralPath (Join-Path $package 'dxcompiler.dll')).Hash -ne $before) { throw 'Rejected archive changed the installed compiler.' }
    # Different old bytes make a partial overwrite observable even when the
    # attempted update uses the same release archive as the positive test.
    Set-Content -LiteralPath (Join-Path $package 'dxcompiler.dll') -Value 'previous compiler fixture'
    $before = (Get-FileHash -LiteralPath (Join-Path $package 'dxcompiler.dll')).Hash
    $manifest.files.'dxcompiler.dll' = $before
    $manifest | ConvertTo-Json -Depth 3 | Set-Content -LiteralPath (Join-Path $package 'dxc-runtime.json') -Encoding UTF8
    $locked = [IO.File]::Open((Join-Path $package 'dxil.dll'), [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
    try { Expect-Failure { & $prepare -BinaryDirectory $package -Archive $Archive } }
    finally { $locked.Dispose() }
    if ((Get-FileHash -LiteralPath (Join-Path $package 'dxcompiler.dll')).Hash -ne $before) { throw 'Locked validator caused a partial compiler update.' }
    Write-Host 'PASS: pinned DXC extraction, licenses/checksums, rejected archive and locked-file preflight.'
} finally {
    $resolvedRoot = [IO.Path]::GetFullPath($root)
    $temporaryRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\') + '\'
    if (-not $resolvedRoot.StartsWith($temporaryRoot, [StringComparison]::OrdinalIgnoreCase) -or
        [IO.Path]::GetFileName($resolvedRoot) -notmatch '^pleamar-runtime-test-[0-9a-f]{32}$') { throw 'Invalid runtime test cleanup path.' }
    Remove-Item -LiteralPath $resolvedRoot -Recurse -Force
}
