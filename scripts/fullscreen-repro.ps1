# Reproduces the shifted window after a fullscreen game, without the game. It opens the real window
# with a fake firewall (no admin, no real filters), has examples\fullscreen.rs take the screen in
# exclusive fullscreen and give it back, then measures how far the window's picture moved. A healthy
# window reports 0, 0. The screen goes black for a few seconds per round, so don't run it in a match.
#   powershell -ExecutionPolicy Bypass -File scripts\fullscreen-repro.ps1 [-Mode dxgi|display] [-Secs 5] [-Size 1280x720] [-Hz <rate>] [-TestExe <exe>] [-OutDir <dir>]
param(
    [ValidateSet('dxgi', 'display')][string]$Mode = 'dxgi',
    [int]$Secs = 5,
    [string]$Size = '',
    [int]$Hz = 0,
    # Another build's test exe, such as one from before a fix, to compare against.
    [string]$TestExe = '',
    [string]$OutDir = (Join-Path $env:TEMP 'dl-conn-rs-fullscreen-repro')
)
$ErrorActionPreference = 'Stop'
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$root = Split-Path -Parent $here
. (Join-Path $here 'drive.ps1')
New-Item -ItemType Directory -Force $OutDir | Out-Null
$OutDir = (Resolve-Path $OutDir).Path

Push-Location $root
try {
    cargo build --example fullscreen --quiet
    if ($LASTEXITCODE) { throw 'building examples\fullscreen.rs failed' }
    if (-not $TestExe) {
        $built = cargo test --no-run --message-format=json --quiet 2>$null | ConvertFrom-Json |
            Where-Object { $_.executable -and $_.target.kind -contains 'lib' }
        if ($LASTEXITCODE -or -not $built) { throw 'building the test exe failed' }
        $TestExe = @($built)[-1].executable
    }
}
finally { Pop-Location }
$fullscreen = Join-Path $root 'target\debug\examples\fullscreen.exe'
# Without this the test exe gets no DPI awareness.
Copy-Item (Join-Path $root 'app.manifest') "$testExe.manifest" -Force

function MeasureShift($name, [switch]$Taken) {
    $path = Join-Path $OutDir "$name.png"
    if (-not $Taken) { [W]::Shot($hwnd, $path) }
    $shift = [W]::Shift((Join-Path $OutDir 'before.png'), $path, [W]::Client($hwnd), 16, 48)
    Write-Host ("{0,-14} shift {1}, {2} px, {3:P1} of pixels differ" -f $name, $shift[0], $shift[1], $shift[2])
    return $shift
}

$data = Join-Path $OutDir 'data'
Remove-Item -Recurse -Force $data -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $data | Out-Null
$env:DL_CONN_RS_DATA_DIR = $data
$env:DL_CONN_RS_RENDERER = ''
$app = $null
try {
    $app = Start-Process $testExe -ArgumentList 'ui::adapter_tests::window_with_a_fake_firewall --ignored --exact --nocapture --test-threads=1' `
        -RedirectStandardOutput (Join-Path $OutDir 'app.txt') -RedirectStandardError (Join-Path $OutDir 'app.err') -NoNewWindow -PassThru
    $null = $app.Handle
    $hwnd = WaitForWindow $app
    if ($hwnd -eq [IntPtr]::Zero) { throw "no window. See $OutDir\app.err" }
    "title: " + (WaitTitle $hwnd 'no block' 30)
    Start-Sleep -Seconds 2
    [W]::Shot($hwnd, (Join-Path $OutDir 'before.png'))
    $client = [W]::Client($hwnd)
    "client area at {0},{1} in the window, {2}x{3}" -f $client[0], $client[1], $client[2], $client[3]

    "--- $Mode fullscreen for $Secs s $Size"
    $arguments = @('--mode', $Mode, '--secs', $Secs)
    if ($Size) { $arguments += @('--size', $Size) }
    if ($Hz) { $arguments += @('--hz', $Hz) }
    & $fullscreen @arguments
    # At once, before the app's own fix has had a chance to run.
    [W]::Shot($hwnd, (Join-Path $OutDir 'at-exit.png'))
    Start-Sleep -Seconds 4

    $atExit = MeasureShift 'at-exit' -Taken
    $after = MeasureShift 'after'
    # A new frame, to tell a stale picture from one that keeps landing in the wrong place.
    $centre = [IntPtr](($client[3] / 2 -shl 16) -bor ($client[2] / 2))
    [void][W]::PostMessage($hwnd, 0x200, [IntPtr]::Zero, $centre)
    Start-Sleep -Milliseconds 800
    $redrawn = MeasureShift 'after-redraw'

    $shifted = ($after[0] -ne 0) -or ($after[1] -ne 0)
    $wasShifted = ($atExit[0] -ne 0) -or ($atExit[1] -ne 0)
    if ($shifted) { "REPRODUCED: the picture moved {0}, {1} px. Screenshots are in $OutDir" -f $after[0], $after[1] }
    elseif ($wasShifted) { "FIXED: the picture moved {0}, {1} px when fullscreen ended, then went back" -f $atExit[0], $atExit[1] }
    else { "not reproduced: the picture stayed in place" }
    if ($shifted -and (($redrawn[0] -ne 0) -or ($redrawn[1] -ne 0))) { 'and a new frame lands in the same wrong place' }
}
finally {
    if ($app -and -not $app.HasExited) {
        [void][W]::PostMessage($hwnd, 0x10, [IntPtr]::Zero, [IntPtr]::Zero)
        if (-not $app.WaitForExit(8000)) { $app.Kill() }
    }
    Remove-Item "$testExe.manifest" -Force -ErrorAction SilentlyContinue
}
