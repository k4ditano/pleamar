param(
    [string]$BinaryDirectory = (Join-Path $PSScriptRoot '../target/release'),
    [string]$Archive
)
$ErrorActionPreference = 'Stop'
$version = '1.9.2609'
$url = 'https://github.com/microsoft/DirectXShaderCompiler/releases/download/v1.9.2609/dxc_2026_09_29.zip'
$checksum = 'AD31B1FC8443175D204F77A611FDB3EF2EC42759BDC2F1167368DE24A4A7E7F1'
$BinaryDirectory = [IO.Path]::GetFullPath($BinaryDirectory)
if (-not (Test-Path -LiteralPath (Join-Path $BinaryDirectory 'pleamar.exe') -PathType Leaf)) {
    throw 'Build pleamar first, then select the directory containing pleamar.exe.'
}
# Do not replace a compiler DLL while the executable using it is mapped.
foreach ($process in [Diagnostics.Process]::GetProcessesByName('pleamar')) {
    try {
        $path = $null
        try { $path = $process.MainModule.FileName } catch {}
        if ($path -eq (Join-Path $BinaryDirectory 'pleamar.exe')) { throw 'Stop this pleamar instance before preparing its runtime.' }
    } finally { $process.Dispose() }
}
$stage = Join-Path ([IO.Path]::GetTempPath()) ('pleamar-dxc-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $stage | Out-Null
try {
    if (-not $Archive) {
        $Archive = Join-Path $stage 'dxc.zip'
        Invoke-WebRequest -Uri $url -OutFile $Archive -UseBasicParsing
    }
    $Archive = (Resolve-Path -LiteralPath $Archive).Path
    if ((Get-FileHash -LiteralPath $Archive -Algorithm SHA256).Hash -ne $checksum) {
        throw 'DXC archive checksum does not match the pinned Microsoft release; no runtime files were installed.'
    }
    $unpacked = Join-Path $stage 'unpacked'
    Expand-Archive -LiteralPath $Archive -DestinationPath $unpacked
    $files = [ordered]@{
        'dxcompiler.dll' = 'bin/x64/dxcompiler.dll'
        'dxil.dll' = 'bin/x64/dxil.dll'
        'licenses/dxc/LICENCE-MIT.txt' = 'LICENCE-MIT.txt'
        'licenses/dxc/LICENSE-LLVM.txt' = 'LICENSE-LLVM.txt'
        'licenses/dxc/LICENSE-MS.txt' = 'LICENSE-MS.txt'
    }
    $hashes = [ordered]@{}
    foreach ($relative in $files.Keys) {
        $source = Join-Path $unpacked $files[$relative]
        if (-not (Test-Path -LiteralPath $source -PathType Leaf)) { throw "Missing DXC package input: $relative" }
        $hashes[$relative] = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    [ordered]@{ version = $version; source = $url; archive_sha256 = $checksum.ToLowerInvariant(); files = $hashes } |
        ConvertTo-Json -Depth 3 | Set-Content -LiteralPath (Join-Path $unpacked 'dxc-runtime.json') -Encoding UTF8
    $files['dxc-runtime.json'] = 'dxc-runtime.json'
    foreach ($relative in $files.Keys) {
        $destination = Join-Path $BinaryDirectory $relative
        if (Test-Path -LiteralPath $destination) {
            if (-not (Test-Path -LiteralPath $destination -PathType Leaf)) { throw "Runtime destination is not a file: $relative" }
            $probe = [IO.File]::Open($destination, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
            $probe.Dispose()
        }
    }
    $installed = @()
    try {
        foreach ($relative in $files.Keys) {
            $destination = Join-Path $BinaryDirectory $relative
            if (Test-Path -LiteralPath $destination -PathType Leaf) {
                $saved = Join-Path $stage ('backup/' + $relative)
                New-Item -ItemType Directory -Force (Split-Path $saved -Parent) | Out-Null
                Copy-Item -LiteralPath $destination -Destination $saved
            }
            $installed += $relative
            New-Item -ItemType Directory -Force (Split-Path $destination -Parent) | Out-Null
            Copy-Item -LiteralPath (Join-Path $unpacked $files[$relative]) -Destination $destination -Force
        }
    } catch {
        $failure = $_
        foreach ($relative in $installed) {
            $destination = Join-Path $BinaryDirectory $relative
            $saved = Join-Path $stage ('backup/' + $relative)
            if (Test-Path -LiteralPath $saved -PathType Leaf) { Copy-Item -LiteralPath $saved -Destination $destination -Force }
            elseif (Test-Path -LiteralPath $destination -PathType Leaf) { Remove-Item -LiteralPath $destination }
        }
        throw $failure
    }
    Write-Host "DXC $version prepared beside pleamar.exe, with checksums and license notices."
} finally {
    # Only the unique directory created above is eligible for recursive cleanup.
    $resolvedStage = [IO.Path]::GetFullPath($stage)
    $temporaryRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\') + '\'
    if (-not $resolvedStage.StartsWith($temporaryRoot, [StringComparison]::OrdinalIgnoreCase) -or
        [IO.Path]::GetFileName($resolvedStage) -notmatch '^pleamar-dxc-[0-9a-f]{32}$') { throw 'Invalid DXC staging cleanup path.' }
    Remove-Item -LiteralPath $resolvedStage -Recurse -Force
}
