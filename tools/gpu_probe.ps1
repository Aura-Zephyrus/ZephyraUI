# Measure a process's GPU usage -- two readings: per engine, and per presented
# frame.
#
# WHY TWO READINGS
#
#   * "GPU Engine" performance counters are attributed PER PROCESS: the GPU%
#     column in Task Manager is exactly this. Split by engine (3D / Copy /
#     Video) so a 4 MB texture upload (Copy) is not confused with rasterizing
#     (3D) -- they have completely different fixes.
#
#   * "GPU 5%" is meaningless without knowing how many frames were presented:
#     5% at 16 fps is 3 ms/frame, 5% at 120 fps is 0.4 ms/frame. Optimizing
#     the wrong reading yields "optimized but the usage did not drop" -- because
#     the frame rate grows back on its own (a retained UI draws as soon as
#     something happens).
#
# USAGE
#
#   powershell -ExecutionPolicy Bypass -File tools\gpu_probe.ps1 -TargetPid 12345
#   powershell -ExecutionPolicy Bypass -File tools\gpu_probe.ps1 -Port 7878 -TargetPid 12345
#
# -Port reads the app's own frame counter over the debug bridge, which is what
# turns the percentage into "ms per presented frame". Without it you only get
# the percentage.
#
# ASCII-only on purpose: this file is read by PowerShell 5.1, which decodes
# .ps1 as ANSI unless there is a BOM -- so non-ASCII here becomes mojibake (and
# sometimes eats a newline, which silently breaks the param block).

param(
    # Debug bridge port (used to read `frames` from `stats`); 0 = skip.
    [int]$Port = 0,
    # Target process id; 0 = look it up by -Name.
    [int]$TargetPid = 0,
    # Target process name (used when -TargetPid is 0).
    [string]$Name = "gallery",
    # Sampling window in seconds.
    [int]$Seconds = 10,
    # Path to the zephyra CLI (for the bridge round trip).
    [string]$Cli = ""
)

if ($Cli -eq "") {
    $guess = Join-Path $PSScriptRoot "..\target\release\zephyra.exe"
    $Cli = if (Test-Path $guess) { (Resolve-Path $guess).Path } else { "zephyra" }
}

if ($TargetPid -eq 0) {
    $p = Get-Process -Name $Name -ErrorAction SilentlyContinue | Select-Object -First 1
    if (-not $p) { Write-Output "process $Name not running"; exit 1 }
    $TargetPid = $p.Id
}
Write-Output "pid = $TargetPid"

function Get-Frames {
    if ($Port -le 0) { return -1 }
    $s = & $Cli bridge --port $Port "stats" 2>$null | Select-Object -Last 1
    if ($s -match "frames=(\d+)") { return [int]$matches[1] }
    return -1
}

$f0 = Get-Frames
$samples = @()
$sw = [System.Diagnostics.Stopwatch]::StartNew()
while ($sw.Elapsed.TotalSeconds -lt $Seconds) {
    $c = Get-Counter "\GPU Engine(*)\Utilization Percentage" -ErrorAction SilentlyContinue
    if ($c) {
        $total = 0.0
        $byEngine = @{}
        foreach ($s in $c.CounterSamples) {
            if ($s.InstanceName -notmatch "pid_(\d+)_") { continue }
            if ([int]$matches[1] -ne $TargetPid) { continue }
            if ($s.CookedValue -le 0) { continue }
            $total += $s.CookedValue
            $eng = "?"
            if ($s.InstanceName -match "engtype_([A-Za-z0-9]+)") { $eng = $matches[1] }
            if ($byEngine.ContainsKey($eng)) { $byEngine[$eng] += $s.CookedValue }
            else { $byEngine[$eng] = $s.CookedValue }
        }
        $samples += [pscustomobject]@{ t = [math]::Round($sw.Elapsed.TotalSeconds, 2); v = $total; e = $byEngine }
    }
    Start-Sleep -Milliseconds 400
}
$elapsed = $sw.Elapsed.TotalSeconds
$f1 = Get-Frames

if ($samples.Count -eq 0) { Write-Output "no samples"; exit 2 }
$vals = @($samples | ForEach-Object { $_.v })
$avg = ($vals | Measure-Object -Average).Average
$max = ($vals | Measure-Object -Maximum).Maximum
Write-Output ""
Write-Output ("GPU  avg={0:N2}%  max={1:N2}%" -f $avg, $max)
Write-Output ""
Write-Output "per engine (avg / max):"
$acc = @{}
foreach ($s in $samples) {
    foreach ($k in $s.e.Keys) {
        if ($acc.ContainsKey($k)) { $acc[$k] += $s.e[$k] } else { $acc[$k] = $s.e[$k] }
    }
}
foreach ($k in @($acc.Keys | Sort-Object { -$acc[$_] })) {
    $per = @($samples | ForEach-Object { if ($_.e.ContainsKey($k)) { $_.e[$k] } else { 0 } })
    Write-Output ("  {0,-10} avg={1:N2}  max={2:N2}" -f $k, ($acc[$k] / $samples.Count), (($per | Measure-Object -Maximum).Maximum))
}
if ($f0 -ge 0 -and $f1 -ge 0) {
    $df = $f1 - $f0
    $fps = if ($df -gt 0) { $df / $elapsed } else { 0 }
    Write-Output ""
    Write-Output ("frames {0} -> {1}  (+{2} in {3:N1}s => {4:N1} fps)" -f $f0, $f1, $df, $elapsed, $fps)
    if ($fps -gt 0) {
        Write-Output ("GPU per presented frame: {0:N3} ms" -f ($avg / 100 * 1000) / $fps)
    }
}
Write-Output ""
Write-Output "timeline: " + (($samples | ForEach-Object { "{0:N1}" -f $_.v }) -join " ")
