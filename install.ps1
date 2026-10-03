param(
    [string]$Prefix = (Join-Path $env:LOCALAPPDATA 'pleamar'),
    [switch]$SkipDxc
)
$ErrorActionPreference = 'Stop'
Push-Location $PSScriptRoot
try {
    cargo install --path . --locked --root $Prefix
    if ($LASTEXITCODE -ne 0) { throw 'The MSVC build failed. Install Rust x64 MSVC and Visual Studio C++ build tools.' }
    if (-not $SkipDxc) {
        & (Join-Path $PSScriptRoot 'scripts/prepare-windows-runtime.ps1') -BinaryDirectory (Join-Path $Prefix 'bin')
    }
    $configRoot = if ($env:PLEAMAR_CONFIG) { $env:PLEAMAR_CONFIG } else { Join-Path $env:APPDATA 'pleamar' }
    New-Item -ItemType Directory -Force (Join-Path $configRoot 'shells') | Out-Null
    Write-Host "Installed: $(Join-Path $Prefix 'bin/pleamar.exe')"
    Write-Host "Configuration: $configRoot"
    Write-Host "Add $(Join-Path $Prefix 'bin') to your user PATH, or run that executable by its full path."
} finally {
    Pop-Location
}
