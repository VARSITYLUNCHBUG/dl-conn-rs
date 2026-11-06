# Opens the real window with a fake firewall, with no admin rights, no UAC prompt and no real filters,
# so it can't change a block. It measures start time, idle cost and how it closes, on the normal
# renderer and on the Windows software renderer, and drives it with posted keys and UI Automation. It
# takes no focus and doesn't move the mouse, so a game keeps its input.
#   cargo test --no-run
#   powershell -ExecutionPolicy Bypass -File scripts\window-check.ps1 -TestExe <target\debug\deps\dl_rs-*.exe> -OutDir <dir>
param(
    [Parameter(Mandatory)][string]$TestExe,
    [Parameter(Mandatory)][string]$OutDir
)
$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force $OutDir | Out-Null
$OutDir = (Resolve-Path $OutDir).Path
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
. (Join-Path $here 'drive.ps1')
# Without this the test exe gets no DPI awareness.
Copy-Item (Join-Path (Split-Path -Parent $here) 'app.manifest') "$TestExe.manifest" -Force
$failures = 0

function Check($name, [bool]$ok) {
    if (-not $ok) { $script:failures++ }
    "{0}  {1}" -f $(if ($ok) { 'PASS' } else { 'FAIL' }), $name
}

try {
    foreach ($renderer in 'normal', 'software') {
        "--- $renderer renderer"
        $data = Join-Path $OutDir "data-$renderer"
        Remove-Item -Recurse -Force $data -ErrorAction SilentlyContinue
        New-Item -ItemType Directory -Force $data | Out-Null
        $env:DL_RS_DATA_DIR = $data
        $env:DL_RS_RENDERER = $(if ($renderer -eq 'software') { 'software' } else { '' })
        # The second round runs as a user would, with no help when hidden.
        $env:DL_RS_DRAW_HIDDEN = $(if ($renderer -eq 'software') { '' } else { '1' })
        $out = Join-Path $OutDir "app-$renderer.txt"
        $clock = [Diagnostics.Stopwatch]::StartNew()
        $app = Start-Process $TestExe -ArgumentList 'ui::adapter_tests::window_with_a_fake_firewall --ignored --exact --nocapture --test-threads=1' `
            -RedirectStandardOutput $out -RedirectStandardError "$out.err" -NoNewWindow -PassThru
        $null = $app.Handle
        $hwnd = WaitForWindow $app
        if ($hwnd -eq [IntPtr]::Zero) { throw "no window. See $out.err" }
        "window after {0:N2} s" -f $clock.Elapsed.TotalSeconds
        $title = WaitTitle $hwnd 'no block' 30
        "relays after {0:N2} s, title: {1}" -f $clock.Elapsed.TotalSeconds, $title
        Check 'the title follows the session' ($title -match 'no block')
        Start-Sleep -Seconds 2
        [W]::Shot($hwnd, (Join-Path $OutDir "window-$renderer.png"))

        $app.Refresh()
        $before = $app.TotalProcessorTime
        Start-Sleep -Seconds 20
        $app.Refresh()
        $used = ($app.TotalProcessorTime - $before).TotalSeconds
        "idle: {0:N2} s of CPU in 20 s ({1:N2}% of one core), memory {2:N0} MB" -f $used, ($used / 20 * 100), ($app.WorkingSet64 / 1MB)
        if ($renderer -eq 'normal') { Check 'it uses almost no CPU while idle' ($used -lt 1.0) }

        $rendererLine = @(LogLines $data | Where-Object { $_ -match 'renderer:' })[-1]
        "log: $rendererLine"
        if ($renderer -eq 'software') { Check 'the software renderer is in use' ($rendererLine -match 'Cpu') }

        if ($renderer -eq 'normal') {
            '--- driving it without focus'
            Check 'UI Automation sees the tabs' (UiHas $hwnd 'Settings  F2')
            ToggleConsole $hwnd
            Check 'a posted backtick opens the console' (WaitLog $data '> toggle console')
            RunCommand $hwnd 'set pick iad'
            $title = WaitTitle $hwnd 'Sterling'
            "title: $title"
            Check 'a posted command picks a server' ($title -match 'Sterling')
            Check 'the log has the command' ((LogCount $data '> set pick iad') -eq 1)
            SetText $hwnd 'dl-rs>' 'list se'
            Press $hwnd $VK_ESCAPE 300
            Check 'a posted Esc clears the line' ((PromptText $hwnd) -eq '')
            $before = LogCount $data '> '
            Press $hwnd $VK_RETURN 300
            Check 'so Enter has nothing to run' ((LogCount $data '> ') -eq $before)
            Press $hwnd $VK_F1
            Check 'a posted F1 opens help' (UiHas $hwnd 'Help  F1')
            [W]::Shot($hwnd, (Join-Path $OutDir 'help.png'))
            Press $hwnd $VK_F3 600
            [W]::Shot($hwnd, (Join-Path $OutDir 'report.png'))
            Press $hwnd $VK_ESCAPE
            ToggleConsole $hwnd
            RunCommand $hwnd 'clear pick'
            Check 'clear pick works' ((WaitTitle $hwnd 'no block') -match 'no block')
            RunCommand $hwnd 'set config theme dark' 500
            [W]::Shot($hwnd, (Join-Path $OutDir 'dark.png'))
            RunCommand $hwnd 'set config theme light' 500
            [W]::Shot($hwnd, (Join-Path $OutDir 'light.png'))
            Check 'the theme setting saves' ((Get-Content (Join-Path $data 'settings.ini') -Raw) -match 'theme=light')
        }

        if ($renderer -eq 'software') {
            [void][W]::PostMessage($hwnd, 0x112, [IntPtr]0xF020, [IntPtr]::Zero)
            Start-Sleep -Seconds 1
            Check 'it minimizes' ([W]::IsIconic($hwnd))
        }
        [void][W]::PostMessage($hwnd, 0x10, [IntPtr]::Zero, [IntPtr]::Zero)
        Check 'the X button closes it, minimized or not' ($app.WaitForExit(8000))
        if (-not $app.HasExited) { $app.Kill() } else { Check 'it exits cleanly' ($app.ExitCode -eq 0) }
        "log: " + @(LogLines $data | Where-Object { $_ -match 'frames' })[-1]
    }
}
finally {
    if ($app -and -not $app.HasExited) { $app.Kill() }
    Remove-Item "$TestExe.manifest" -Force -ErrorAction SilentlyContinue
}
"finished with $failures failures"
exit $failures
