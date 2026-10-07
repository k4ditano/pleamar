param(
    [Parameter(Mandatory=$true)][string]$Binary,
    [Parameter(Mandatory=$true)][string]$Scene,
    [string]$OutputDirectory = 'target/windows-performance',
    [int]$Seconds = 20
)
# Native desktop measurement, not a headless rendering test. Do not compare the
# cycle count with physical display FPS: paintings and present timings are
# recorded separately by PLEAMAR_TIMING in the log.
$ErrorActionPreference = 'Stop'
$Binary = (Resolve-Path -LiteralPath $Binary).Path
$Scene = (Resolve-Path -LiteralPath $Scene).Path
New-Item -ItemType Directory -Force $OutputDirectory | Out-Null
$OutputDirectory = (Resolve-Path -LiteralPath $OutputDirectory).Path
$env:PLEAMAR_SOCKET_DIR = "perf-$PID"
$env:PLEAMAR_TIMING = '1'
$env:PLEAMAR_NO_RELAUNCH = '1'
$outFile = Join-Path $OutputDirectory 'render.log'
$errFile = Join-Path $OutputDirectory 'render.error.log'
$output = [IO.FileStream]::new($outFile, [IO.FileMode]::Create, [IO.FileAccess]::Write, [IO.FileShare]::ReadWrite)
$errors = [IO.FileStream]::new($errFile, [IO.FileMode]::Create, [IO.FileAccess]::Write, [IO.FileShare]::ReadWrite)
$process = [Diagnostics.Process]::new()
$name = [IO.Path]::GetFileNameWithoutExtension($Scene)
try {
    $process.StartInfo.FileName = $Binary
    $process.StartInfo.Arguments = '--scene "' + $Scene + '" --no-hud --stall 0 --record open'
    $process.StartInfo.WorkingDirectory = Split-Path $Scene -Parent
    $process.StartInfo.UseShellExecute = $false
    $process.StartInfo.CreateNoWindow = $true
    $process.StartInfo.RedirectStandardOutput = $true
    $process.StartInfo.RedirectStandardError = $true
    [void]$process.Start()
    $copyOut = $process.StandardOutput.BaseStream.CopyToAsync($output)
    $copyErr = $process.StandardError.BaseStream.CopyToAsync($errors)
    Start-Sleep -Seconds 8
    if ($process.HasExited) { throw "Native scene failed. Read $errFile" }
    $samples = @()
    foreach ($state in @('false', 'true')) {
        & $Binary --say $name "fact open $state" | Out-Null
        if ($LASTEXITCODE -ne 0) { throw 'IPC did not become ready' }
        Start-Sleep -Seconds 2
        $initialOpen = (& $Binary --say $name 'get open').Trim()
        $process.Refresh()
        $cpu = $process.TotalProcessorTime.TotalSeconds
        $watch = [Diagnostics.Stopwatch]::StartNew()
        Start-Sleep -Seconds $Seconds
        $process.Refresh()
        $elapsed = $watch.Elapsed.TotalSeconds
        $oneCore = 100 * ($process.TotalProcessorTime.TotalSeconds - $cpu) / $elapsed
        $finalOpen = (& $Binary --say $name 'get open').Trim()
        $samples += [pscustomobject]@{
            PanelOpen = $state; Seconds = [math]::Round($elapsed, 2)
            ObservedOpenAtStart = $initialOpen; ObservedOpenAtEnd = $finalOpen
            CpuOneCorePercent = [math]::Round($oneCore, 2)
            CpuWholeMachinePercent = [math]::Round($oneCore / [Environment]::ProcessorCount, 2)
            WorkingSetMiB = [math]::Round($process.WorkingSet64 / 1MB, 1)
            Threads = $process.Threads.Count; Handles = $process.HandleCount
        }
        if ($initialOpen -ne $state -or $finalOpen -ne $state) {
            Write-Warning 'The panel changed during measurement (for example, an outside click). Treat CPU as interactive-session data, not a fixed panel-state benchmark.'
        }
    }
    $samples | ConvertTo-Json | Set-Content -Encoding UTF8 (Join-Path $OutputDirectory 'process.json')
    $samples | Format-Table | Out-String | Write-Output
} finally {
    if (-not $process.HasExited) {
        & $Binary --say $name quit | Out-Null
        if (-not $process.WaitForExit(15000)) { $process.Kill(); throw 'Native shutdown timed out' }
    }
    if ($copyOut) { [void]$copyOut.GetAwaiter().GetResult() }
    if ($copyErr) { [void]$copyErr.GetAwaiter().GetResult() }
    $process.Dispose(); $output.Dispose(); $errors.Dispose()
}
$times = @(Get-Content -LiteralPath $outFile | ForEach-Object { if ($_ -match '^(\d+(?:\.\d+)?)\t') { [double]$Matches[1] } })
$deltas = @()
for ($i=1; $i -lt $times.Count; $i++) { if ($times[$i-1] -gt 8000) { $deltas += $times[$i] - $times[$i-1] } }
if ($deltas.Count -lt $Seconds * 40) { throw 'Too few render cycles: native frame pacing regressed' }
$sorted = @($deltas | Sort-Object)
$metrics = [pscustomobject]@{ Cycles=$deltas.Count; MeanMs=($deltas | Measure-Object -Average).Average; P99Ms=$sorted[[int][math]::Floor($sorted.Count*0.99)]; MaxMs=$sorted[-1] }
$metrics | ConvertTo-Json | Set-Content -Encoding UTF8 (Join-Path $OutputDirectory 'cycles.json')
$metrics | Format-List
if ($metrics.P99Ms -gt 50) { throw 'Render cycle p99 exceeded 50 ms; inspect device load and logs' }
