# Elevated end-to-end test: real WFP filters, real relays and the real app. It drives the window through
# posted keys and UI Automation, reads the window title and the app's log file, and uses the real mouse
# near the end. It switches picks, so don't run it during a match. Run from an admin shell after building:
#   cargo test --no-run; cargo build --release
#   powershell -ExecutionPolicy Bypass -File scripts\live-test.ps1 -TestExe <target\debug\deps\dl_rs-*.exe> -AppExe <target\release\dl-rs.exe> -OutDir <dir>
param(
    [Parameter(Mandatory)][string]$TestExe,
    [Parameter(Mandatory)][string]$OutDir,
    [Parameter(Mandatory)][string]$AppExe
)
$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force $OutDir | Out-Null
$OutDir = (Resolve-Path $OutDir).Path
# Settings and the log stay in the output folder, away from the real ones.
$data = Join-Path $OutDir 'data'
Remove-Item -Recurse -Force $data -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $data | Out-Null
$env:DL_RS_DATA_DIR = $data
$log = Join-Path $OutDir 'results.txt'
Set-Content $log "started $(Get-Date)"
function Note($text) { Add-Content $log $text }
$script:failures = 0
function Check($name, [bool]$ok) {
    if (-not $ok) { $script:failures++ }
    Note ("{0}  {1}" -f $(if ($ok) { 'PASS' } else { 'FAIL' }), $name)
}
. (Join-Path (Split-Path -Parent $MyInvocation.MyCommand.Path) 'drive.ps1')

function RunTest($name) {
    $out = Join-Path $OutDir (($name -replace '::', '-') + '.txt')
    $p = Start-Process $TestExe -ArgumentList "$name --ignored --exact --nocapture --test-threads=1" `
        -RedirectStandardOutput $out -RedirectStandardError "$out.err" -NoNewWindow -PassThru
    $null = $p.Handle # Without this, ExitCode stays empty after the process ends.
    $done = $p.WaitForExit(120000)
    if (-not $done) { $p.Kill() }
    Check "test $name" ($done -and $p.ExitCode -eq 0)
    Note (Get-Content $out, "$out.err" -Raw -ErrorAction SilentlyContinue)
}

function Ping1($ip) { (& ping.exe -n 1 -w 1000 $ip | Out-String) -match 'TTL=' }

# An unconnected UDP send reports success even when dropped; the drop shows up in the log.
function UdpSend($ip) {
    $udp = New-Object Net.Sockets.UdpClient
    try { [void]$udp.Send([byte[]](1, 2, 3), 3, $ip, 27015) } finally { $udp.Close() }
}

function StartApp {
    $proc = Start-Process $AppExe -PassThru
    $null = $proc.Handle
    @{ Proc = $proc; Hwnd = (WaitForWindow $proc) }
}

function PickServer($app, $text, $pattern) {
    SearchServers $app.Hwnd $text
    Press $app.Hwnd $VK_RETURN
    WaitTitle $app.Hwnd $pattern
}

function LogHas($pattern) { (LogCount $data $pattern) -gt 0 }

try {
    Note "--- environment"
    Note ((Get-Process | Where-Object { $_.ProcessName -match 'exitlag|deadlock|cs2|dota2' } | ForEach-Object { "$($_.ProcessName)#$($_.Id)" }) -join ' ')
    Note (& netsh wfp show options optionsfor=netevents | Out-String)

    Note "--- filter tests"
    RunTest 'live_tests::blocks_only_the_target_and_clears_cleanly'
    RunTest 'live_tests::reports_dropped_packets'
    RunTest 'live_tests::live_udp_flow_is_cut_and_restored'
    RunTest 'live_tests::survives_exit_part1'
    RunTest 'live_tests::survives_exit_part2'

    Note "--- starting up"
    # Use relays that answer right now, so another tool's rules can't confuse the result.
    $config = Invoke-RestMethod 'https://api.steampowered.com/ISteamApps/GetSDRConfig/v1/?appid=730'
    $live = @($config.pops.PSObject.Properties | Where-Object { $_.Value.relays } | Where-Object { Ping1 $_.Value.relays[0].ipv4 } | Select-Object -First 2)
    Check 'precondition: two relay sites answer ping' ($live.Count -eq 2)
    $openCode = $live[0].Name; $openDesc = $live[0].Value.desc; $open = $live[0].Value.relays[0].ipv4
    $shutCode = $live[1].Name; $shutDesc = $live[1].Value.desc; $shut = $live[1].Value.relays[0].ipv4
    Note "pick $openCode $open, expect blocked $shutCode $shut"

    $clock = [Diagnostics.Stopwatch]::StartNew()
    $app = StartApp
    Note ("window after {0:N2} s" -f $clock.Elapsed.TotalSeconds)
    Check 'main window opens' ($app.Hwnd -ne [IntPtr]::Zero)
    if ($app.Hwnd -eq [IntPtr]::Zero) { throw 'no main window' }
    Note ('title: ' + (WaitTitle $app.Hwnd 'no block'))
    Note ("relays after {0:N2} s" -f $clock.Elapsed.TotalSeconds)
    Check 'it logs which renderer it uses' (LogHas 'renderer: ')
    [W]::Shot($app.Hwnd, (Join-Path $OutDir 'main.png'))
    Start-Sleep -Seconds 3
    $app.Proc.Refresh()
    $before = $app.Proc.TotalProcessorTime
    Start-Sleep -Seconds 20
    $app.Proc.Refresh()
    $used = ($app.Proc.TotalProcessorTime - $before).TotalSeconds
    Note ("idle: {0:N2} s of CPU in 20 s, working set {1:N0} MB, private {2:N0} MB, exe {3:N1} MB" -f $used, ($app.Proc.WorkingSet64 / 1MB), ($app.Proc.PrivateMemorySize64 / 1MB), ((Get-Item $AppExe).Length / 1MB))
    Check 'it uses almost no CPU while idle' ($used -lt 1.0)

    Note "--- a second launch"
    $second = Start-Process $AppExe -PassThru
    $null = $second.Handle
    Check 'a second copy exits' ($second.WaitForExit(10000))
    Check 'and the first keeps running' (-not $app.Proc.HasExited)

    Note "--- dropdown"
    SearchServers $app.Hwnd 'a'
    [W]::Shot($app.Hwnd, (Join-Path $OutDir 'dropdown.png'))
    Check 'typing opens the list with sections' ((UiHas $app.Hwnd 'Africa') -and (UiHas $app.Hwnd 'Europe'))
    Press $app.Hwnd $VK_ESCAPE
    Check 'Esc closes it' (-not (UiHas $app.Hwnd 'Africa' 1))
    Check 'Esc picks nothing' ([W]::Title($app.Hwnd) -match 'no block')

    $title = PickServer $app $openDesc '\d+ blocked'
    Note "title: $title"
    Check 'typing and Enter pick a server' ($title.Contains($openDesc) -and $title -match '\d+ blocked')
    Check 'picked relay stays reachable' (Ping1 $open)
    Push-Location $OutDir
    & netsh wfp show state file=state-active.xml | Out-Null
    Pop-Location
    Check 'other relay ping blocked' (-not (Ping1 $shut))
    Start-Sleep -Seconds 7
    $direct = @(UiNames $app.Hwnd | Where-Object { $_ -like 'Direct:*' })[0]
    Note "direct: $direct"
    Note ('game: ' + @(UiNames $app.Hwnd | Where-Object { $_ -like 'Game:*' -or $_ -like 'ExitLag route:*' })[0])
    Check 'the direct counter reports on the pick' ($direct -match [regex]::Escape($openCode))
    [W]::Shot($app.Hwnd, (Join-Path $OutDir 'picked.png'))

    Note "--- console"
    ToggleConsole $app.Hwnd
    Check 'backtick opens the console' ($null -ne (UiFindEdit $app.Hwnd 'dl-rs>'))
    for ($i = 0; $i -lt 8; $i++) { UdpSend $shut; Start-Sleep -Seconds 1 }
    Check 'the log has history from before it opened' (LogHas 'start: active')
    Check 'a dropdown choice is logged as a command' (LogHas "> set pick $openCode")
    Check 'report shows processes' (LogHas 'processes: \S')
    Check 'report shows picking' (LogHas "picking: $openCode")
    Check 'report shows routes' (LogHas 'via .* gw ')
    Check 'report shows UDP drops' (LogHas "blocked .* -> $shutCode")
    [W]::Shot($app.Hwnd, (Join-Path $OutDir 'console.png'))

    Note "--- console: typing"
    RunCommand $app.Hwnd 'stop report'
    Check 'a typed command runs and echoes' (LogHas '> stop report')
    SetText $app.Hwnd 'dl-rs>' 'list se'
    Press $app.Hwnd $VK_TAB 300
    Check 'tab completes a word' ((PromptText $app.Hwnd) -eq 'list servers ')
    Press $app.Hwnd $VK_ESCAPE 300
    Check 'esc clears the line' ((PromptText $app.Hwnd) -eq '')
    Check 'and stays on the console' ($null -ne (UiFindEdit $app.Hwnd 'dl-rs>'))
    SetText $app.Hwnd 'dl-rs>' 's'
    Press $app.Hwnd $VK_TAB 200
    Press $app.Hwnd $VK_TAB 300
    Check 'second tab lists options with notes' (LogHas 'show\s+status')
    Check 'listing leaves the line intact' ((PromptText $app.Hwnd) -eq 's')
    Press $app.Hwnd $VK_ESCAPE 300
    RunCommand $app.Hwnd 'help set pick'
    Check 'help explains one command' (LogHas 'Block every relay except')
    RunCommand $app.Hwnd 'set pick wibble'
    Check 'bad server gives a useful error' (LogHas "unknown server 'wibble'")
    RunCommand $app.Hwnd 'lst servers'
    Check 'typo gets a suggestion' (LogHas "Did you mean 'list servers'")
    RunCommand $app.Hwnd "list servers --filter $openCode"
    Check 'list servers filters' (LogHas "$openCode\s+\d+ relays")
    RunCommand $app.Hwnd 'show status'
    Check 'show status reports the pick' (LogHas "pick: $openCode ")
    $errors = LogCount $data 'error: '
    RunCommand $app.Hwnd 'show processes'
    RunCommand $app.Hwnd 'show routes'
    RunCommand $app.Hwnd 'ping relays' 1500
    RunCommand $app.Hwnd "list relays $openCode"
    Check 'diagnostic commands run without errors' ((LogCount $data 'error: ') -eq $errors)

    Note "--- one state behind every front-end"
    RunCommand $app.Hwnd "set pick $shutCode" 1500
    $title = WaitTitle $app.Hwnd ([regex]::Escape($shutDesc))
    Note "title: $title"
    Check 'a typed pick updates the title' ($title.Contains($shutDesc))
    Check 'a typed pick moves the block' ((Ping1 $shut) -and -not (Ping1 $open))
    $before = LogCount $data '> set pick'
    SearchServers $app.Hwnd $openCode
    Press $app.Hwnd $VK_ESCAPE 500
    Check 'cancelling the dropdown issues nothing' ((LogCount $data '> set pick') -eq $before)
    $title = PickServer $app $openDesc ([regex]::Escape($openDesc))
    Start-Sleep -Milliseconds 400
    Check 'one dropdown choice logs exactly one command' ((LogCount $data '> set pick') -eq ($before + 1))

    Note "--- console: logging while typing"
    RunCommand $app.Hwnd 'start report --every 1'
    SetText $app.Hwnd 'dl-rs>' 'list se'
    $before = LogCount $data 'processes:'
    Start-Sleep -Seconds 3
    Check 'log keeps going while typing' ((LogCount $data 'processes:') -gt $before)
    Check 'a half-typed command survives log output' ((PromptText $app.Hwnd) -eq 'list se')
    Press $app.Hwnd $VK_ESCAPE 300
    RunCommand $app.Hwnd 'stop report'
    RunCommand $app.Hwnd 'clear screen'

    Note "--- pages"
    ToggleConsole $app.Hwnd
    Check 'backtick closes the console' ($null -eq (UiFindEdit $app.Hwnd 'dl-rs>' 1))
    foreach ($page in @(@($VK_F1, 'CUSTOM GAME', 'f1'), @($VK_F2, 'Hold filter updates in game', 'f2'), @($VK_F3, 'FINDINGS|game log', 'f3'))) {
        Press $app.Hwnd $page[0] 1000
        Check "$($page[2]) opens its page" (@(UiNames $app.Hwnd | Where-Object { $_ -match $page[1] }).Count -gt 0)
        [W]::Shot($app.Hwnd, (Join-Path $OutDir "$($page[2]).png"))
    }
    Press $app.Hwnd $VK_ESCAPE 500
    ToggleConsole $app.Hwnd
    RunCommand $app.Hwnd 'set config theme dark' 600
    [W]::Shot($app.Hwnd, (Join-Path $OutDir 'theme-dark.png'))
    RunCommand $app.Hwnd 'set config theme light' 600
    [W]::Shot($app.Hwnd, (Join-Path $OutDir 'theme-light.png'))
    RunCommand $app.Hwnd 'clear config theme' 300

    Note "--- minimized"
    [void][W]::PostMessage($app.Hwnd, 0x112, [IntPtr]0xF020, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 800
    Check 'it minimizes' ([W]::IsIconic($app.Hwnd))
    Check 'and the block holds' (-not (Ping1 $shut))
    [void][W]::PostMessage($app.Hwnd, 0x112, [IntPtr]0xF120, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 800
    Check 'and it comes back' (-not [W]::IsIconic($app.Hwnd))

    Note "--- quit"
    RunCommand $app.Hwnd 'quit' 300
    Check 'quit closes the app' ($app.Proc.WaitForExit(8000))
    Check 'block outlives the app' (-not (Ping1 $shut))

    Note "--- reopen"
    $app = StartApp
    $title = WaitTitle $app.Hwnd '\d+ blocked'
    Note "reopened title: $title"
    Check 'reopened app recovers the pick from WFP' ($title.Contains($openDesc))
    $title = PickServer $app 'no block' 'no block'
    Note "title: $title"
    Check 'No block option clears' ($title -match 'no block')
    Check 'relay reachable again' (Ping1 $shut)

    Note "--- real mouse"
    [void][W]::SetForegroundWindow($app.Hwnd)
    Start-Sleep -Milliseconds 400
    UiClick $app.Hwnd 'Open the server list'
    [W]::Shot($app.Hwnd, (Join-Path $OutDir 'dropdown-mouse.png'))
    UiClick $app.Hwnd 'Africa'
    Start-Sleep -Milliseconds 300
    Check 'clicking a header picks nothing' ([W]::Title($app.Hwnd) -match 'no block')
    $site = @($config.pops.PSObject.Properties | Where-Object { $_.Name -eq 'jnb' })[0].Value.desc
    UiClick $app.Hwnd "$site - jnb"
    $title = WaitTitle $app.Hwnd ([regex]::Escape($site))
    Note "title: $title"
    Check 'clicking a server picks it' ($title.Contains($site))
    UiClick $app.Hwnd 'Open the server list'
    $hover = UiFind $app.Hwnd "$site - jnb"
    Check 'the list is open again' ($null -ne $hover)
    # Below the list, which always leaves a strip free at the bottom of the window.
    $frame = [W]::Rect($app.Hwnd)
    [W]::Click($frame[0] + 40, $frame[3] - 18)
    Start-Sleep -Milliseconds 500
    Check 'clicking away closes the list' ($null -eq (UiFind $app.Hwnd "$site - jnb" 1))
    Check 'and changes nothing' ([W]::Title($app.Hwnd).Contains($site))

    Note "--- clear_on_exit"
    ToggleConsole $app.Hwnd
    RunCommand $app.Hwnd 'set config clear_on_exit on' 600
    Check 'it saves' ((Get-Content (Join-Path $data 'settings.ini') -Raw) -match 'clear_on_exit=on')
    Check 'the block is on before closing' (-not (Ping1 $shut))
    [void][W]::PostMessage($app.Hwnd, 0x10, [IntPtr]::Zero, [IntPtr]::Zero)
    Check 'the X button closes the app' ($app.Proc.WaitForExit(8000))
    Check 'clear_on_exit removed the block' (Ping1 $shut)
    $app = StartApp
    $title = WaitTitle $app.Hwnd 'no block'
    Check 'a reopened app finds no block' ($title -match 'no block')
    $app.Proc.Kill(); $app.Proc.WaitForExit()
}
catch {
    $script:failures++
    Note "FAIL  script error: $_ at line $($_.InvocationInfo.ScriptLineNumber)"
}
finally {
    Get-Process dl-rs -ErrorAction SilentlyContinue | Stop-Process -Force
    RunTest 'live_tests::clear_any_block'
    Push-Location $OutDir
    & netsh wfp show state file=state-final.xml | Out-Null
    Pop-Location
    Check 'nothing left behind' (-not (Select-String -Path (Join-Path $OutDir 'state-final.xml') -SimpleMatch 'dl-rs' -Quiet))
    Note "finished with $($script:failures) failures"
}
