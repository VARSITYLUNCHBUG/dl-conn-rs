# Runs game-shaped traffic through dl-conn-rs and counts what really reaches the network card with the
# built-in pktmon, so the result doesn't depend on dl-conn-rs's own reports. Run from an admin shell:
#   cargo build --release --example loadsim; cargo build --release
#   powershell -ExecutionPolicy Bypass -File scripts\load-sim.ps1 -SimExe <target\release\examples\loadsim.exe> -AppExe <target\release\dl-conn-rs.exe> -TestExe <target\debug\deps\dl_conn_rs-*.exe> -OutDir <dir>
# To test ExitLag with a real game, boost and run it first, stay out of a match, then add
#   -Scenario game -ExeName deadlock.exe -Pick sgp
# That scenario only ever picks the server ExitLag is routing to, so it can't cut the game off.
param(
    [ValidateSet('load', 'game', 'probe')][string]$Scenario = 'load',
    [string]$Pick = 'sgp',
    [Parameter(Mandatory)][string]$SimExe,
    [Parameter(Mandatory)][string]$AppExe,
    [Parameter(Mandatory)][string]$TestExe,
    [Parameter(Mandatory)][string]$OutDir,
    [string]$ExeName = 'loadsim.exe',
    [int]$Pps = 256,
    [int]$Size = 1200,
    [int]$Warmup = 0
)
$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force $OutDir | Out-Null
$OutDir = (Resolve-Path $OutDir).Path
# Settings and the log stay in the output folder, away from the real ones.
$data = Join-Path $OutDir 'data'
New-Item -ItemType Directory -Force $data | Out-Null
$env:DL_CONN_RS_DATA_DIR = $data
$log = Join-Path $OutDir 'results.txt'
Set-Content $log "started $(Get-Date), process $ExeName, $Pps pps x $Size bytes"
function Note($text) { Add-Content $log $text }

. (Join-Path (Split-Path -Parent $MyInvocation.MyCommand.Path) 'drive.ps1')

$simDir = Join-Path $OutDir 'bin'
New-Item -ItemType Directory -Force $simDir | Out-Null
$sim = Join-Path $simDir $ExeName
Copy-Item $SimExe $sim -Force

function StartSim($secs, $open, $probes, [int]$warmup = 0) {
    $out = Join-Path $OutDir "sim-$([guid]::NewGuid().ToString('N')).txt"
    $arguments = "--open $open --probe $($probes -join ',') --secs $secs --pps $Pps --size $Size --warmup $warmup"
    $p = Start-Process $sim -ArgumentList $arguments -RedirectStandardOutput $out -NoNewWindow -PassThru
    $null = $p.Handle
    @{ Proc = $p; Out = $out }
}

function FinishSim($run) {
    if (-not $run.Proc.WaitForExit(180000)) { $run.Proc.Kill(); return 'sim hung' }
    (Get-Content $run.Out -Raw).Trim()
}

function LocalEcho($secs) {
    $out = Join-Path $OutDir 'echo.txt'
    $p = Start-Process $sim -ArgumentList "--echo $secs" -RedirectStandardOutput $out -NoNewWindow -PassThru
    $null = $p.Handle
    [void]$p.WaitForExit(($secs + 20) * 1000)
    (Get-Content $out -Raw).Trim()
}

function CaptureStart($name) {
    & pktmon filter remove | Out-Null
    & pktmon filter add dlrs-udp -t UDP | Out-Null
    & pktmon start -c --comp nics --pkt-size 64 -s 256 -f (Join-Path $OutDir "$name.etl") | Out-Null
}

# Stops the capture and returns packets per destination. The raw capture includes all of this PC's
# UDP traffic, so only the per-destination summary is kept.
function CaptureStop($name) {
    & pktmon stop | Out-Null
    $etl = Join-Path $OutDir "$name.etl"
    $txt = Join-Path $OutDir "$name.txt"
    & pktmon etl2txt $etl -o $txt | Out-Null
    $counts = @{}
    $pattern = [regex]'(\d+\.\d+\.\d+\.\d+)\.(\d+) > (\d+\.\d+\.\d+\.\d+)\.(\d+)'
    foreach ($line in [IO.File]::ReadLines($txt)) {
        $m = $pattern.Match($line)
        if ($m.Success) { $counts[$m.Groups[3].Value] = 1 + [int]$counts[$m.Groups[3].Value] }
    }
    if ($counts.Count -eq 0) { Get-Content $txt -TotalCount 25 | Set-Content (Join-Path $OutDir "$name-sample.txt") }
    Remove-Item $etl, $txt -ErrorAction SilentlyContinue
    $counts
}

function NoteCounts($label, $counts, $relays) {
    $named = foreach ($r in $relays) { "$($r.Code) $($r.Ip)=$([int]$counts[$r.Ip])" }
    Note "$label wire: $($named -join ', ')"
    $known = @($relays | ForEach-Object { $_.Ip })
    $others = $counts.GetEnumerator() | Where-Object { $known -notcontains $_.Key } | Sort-Object Value -Descending | Select-Object -First 5
    Note "$label top other destinations: $(($others | ForEach-Object { "$($_.Key)=$($_.Value)" }) -join ', ')"
}

function StartApp {
    $proc = Start-Process $AppExe -PassThru
    $null = $proc.Handle
    $hwnd = WaitForWindow $proc 10
    if ($hwnd -eq [IntPtr]::Zero) { throw 'no main window' }
    @{ Proc = $proc; Hwnd = $hwnd }
}

# Picks through the dropdown's search, or chooses No block with no text.
function PickItem($app, $text) {
    $query = if ($text) { $text } else { 'no block' }
    SearchServers $app.Hwnd $query
    Press $app.Hwnd $VK_RETURN
}

# The log file holds what the console page shows.
function ConsoleRows($app) { LogLines $data }

function ConsoleType($app, $text) { RunCommand $app.Hwnd $text }

# Totals the console's "blocked <app> -> <site> <ip> xN" lines by destination, for one app.
function DropsByIp($rows) {
    $totals = @{}
    foreach ($row in $rows) {
        if ($row -match "blocked $([regex]::Escape($ExeName)) -> (\S+) (\S+) x(\d+)") {
            $totals[$Matches[2]] = [int]$Matches[3] + [int]$totals[$Matches[2]]
        }
    }
    $totals
}

# Everything that left the network card: relays by whether they're open, then the busiest others.
function NoteWire($label, $counts, $config, $pickCode) {
    $openIps = @($config.pops.$pickCode.relays | ForEach-Object { $_.ipv4 })
    $relayIps = @($config.pops.PSObject.Properties | ForEach-Object { $_.Value.relays } | Where-Object { $_ } | ForEach-Object { $_.ipv4 })
    $toOpen = 0; $toBlocked = 0
    foreach ($ip in $relayIps) {
        if ($openIps -contains $ip) { $toOpen += [int]$counts[$ip] } else { $toBlocked += [int]$counts[$ip] }
    }
    Note "$label wire: to picked relays=$toOpen, to blocked relays=$toBlocked"
    $others = $counts.GetEnumerator() | Where-Object { $relayIps -notcontains $_.Key } | Sort-Object Value -Descending | Select-Object -First 8
    Note "$label busiest other destinations: $(($others | ForEach-Object { "$($_.Key)=$($_.Value)" }) -join ', ')"
}

# Applies the block during a real match and measures its effect and dl-conn-rs's own cost. Sends no test
# traffic, posts its input so it can't take focus, and leaves dl-conn-rs running with the block on.
function ProbeScenario($config) {
    $pickRelay = $config.pops.$Pick
    Note "--- 1. the match with no block, 15 s"
    CaptureStart 'probe-before'
    Start-Sleep -Seconds 15
    NoteWire 'before' (CaptureStop 'probe-before') $config $Pick

    Note "--- 2. block applied during the match, 60 s"
    $app = StartApp
    Note "title: $(WaitTitle $app.Hwnd 'no block')"
    PickItem $app $pickRelay.desc
    Note "title: $(WaitTitle $app.Hwnd 'blocked')"
    # Minimized only now, because a minimized window doesn't draw or take input.
    [void][W]::PostMessage($app.Hwnd, 0x112, [IntPtr]0xF020, [IntPtr]::Zero)
    Note "block applied at $(Get-Date -Format HH:mm:ss)"
    $proc = Get-Process -Id $app.Proc.Id
    $cpuBefore = $proc.TotalProcessorTime
    CaptureStart 'probe-block'
    Start-Sleep -Seconds 60
    NoteWire 'during' (CaptureStop 'probe-block') $config $Pick
    $proc.Refresh()
    $cpu = ($proc.TotalProcessorTime - $cpuBefore).TotalMilliseconds
    Note ("dl-conn-rs over 60 s: cpu {0:N0} ms, working set {1:N1} MB, private {2:N1} MB, threads {3}, handles {4}" -f $cpu, ($proc.WorkingSet64 / 1MB), ($proc.PrivateMemorySize64 / 1MB), $proc.Threads.Count, $proc.HandleCount)
    Note "dl-conn-rs left running with $Pick picked"
}

function GameScenario($config) {
    $pickRelay = @{ Code = $Pick; Ip = $config.pops.$Pick.relays[0].ipv4; Desc = $config.pops.$Pick.desc }
    $probes = @('fra', 'ams', 'lhr', 'waw', 'jnb', 'dfw', 'iad') | Where-Object { $_ -ne $Pick } | ForEach-Object { $config.pops.$_.relays[0].ipv4 }
    Note "pick $Pick $($pickRelay.Ip); probe addresses $($probes -join ' ')"
    foreach ($name in 'deadlock', 'project8', 'ExitLag') {
        foreach ($p in Get-Process $name -ErrorAction SilentlyContinue) {
            Note "$name#$($p.Id): $(@(Get-NetUDPEndpoint -OwningProcess $p.Id -ErrorAction SilentlyContinue).Count) UDP sockets"
        }
    }

    Note "--- 1. the real game through ExitLag, no block, 20 s"
    CaptureStart 'game-before'
    Start-Sleep -Seconds 20
    NoteWire 'before' (CaptureStop 'game-before') $config $Pick

    Note "--- 2. the real game with only $Pick open, 60 s"
    $app = StartApp
    Note "title: $(WaitTitle $app.Hwnd 'no block')"
    PickItem $app $pickRelay.Desc
    Note "title: $(WaitTitle $app.Hwnd 'blocked')"
    ToggleConsole $app.Hwnd
    CaptureStart 'game-block'
    Start-Sleep -Seconds 60
    NoteWire 'game' (CaptureStop 'game-block') $config $Pick
    Start-Sleep -Seconds 6
    $rows = ConsoleRows $app
    $gameDrops = DropsByIp $rows
    Note "game drops in the console: $($gameDrops.Count) addresses, $(($gameDrops.Values | Measure-Object -Sum).Sum) packets"
    Note "sample: $((@($rows | Where-Object { $_ -match 'blocked ' }) | Select-Object -Last 6) -join ' | ')"

    Note "--- 3. the simulator under the game's process name, 30 s"
    CaptureStart 'game-sim'
    $run = StartSim 30 $pickRelay.Ip $probes
    $result = FinishSim $run
    $simWire = CaptureStop 'game-sim'
    Note "sim: $result"
    Note "sim wire to the picked address: $([int]$simWire[$pickRelay.Ip])"
    NoteWire 'sim' $simWire $config $Pick
    Start-Sleep -Seconds 6
    $rows = ConsoleRows $app
    $after = DropsByIp $rows
    foreach ($ip in $probes) {
        Note "probe address $($ip): dropped $([int]$after[$ip] - [int]$gameDrops[$ip]) during the sim, wire $([int]$simWire[$ip])"
    }
    Set-Content (Join-Path $OutDir 'console-log.txt') $rows

    Note "--- 4. back to no block"
    PickItem $app $null
    Note "title: $(WaitTitle $app.Hwnd 'no block')"
    $app.Proc.Kill(); $app.Proc.WaitForExit()
}

# Totals the console's "blocked <app> -> <site> <ip> xN" lines for the simulator.
function DropTotals($rows) {
    $totals = @{}
    foreach ($row in $rows) {
        if ($row -match "blocked $([regex]::Escape($ExeName)) -> (\S+) \S+ x(\d+)") {
            $totals[$Matches[1]] = [int]$Matches[2] + [int]$totals[$Matches[1]]
        }
    }
    if ($totals.Count -eq 0) { return 'none' }
    ($totals.GetEnumerator() | Sort-Object Name | ForEach-Object { "$($_.Key)=$($_.Value)" }) -join ', '
}

try {
    Note "--- environment"
    Note ((Get-Process | Where-Object { $_.ProcessName -match 'exitlag|deadlock|cs2|dota2' } | ForEach-Object { "$($_.ProcessName)#$($_.Id)" }) -join ' ')

    $config = Invoke-RestMethod 'https://api.steampowered.com/ISteamApps/GetSDRConfig/v1/?appid=730'
    if ($Scenario -eq 'probe') { ProbeScenario $config; return }
    if ($Scenario -eq 'game') { GameScenario $config; return }
    $relay = { param($code) @{ Code = $code; Ip = $config.pops.$code.relays[0].ipv4; Desc = $config.pops.$code.desc } }
    $openRelay = & $relay 'jnb'
    $probeRelays = @('fra', 'ams', 'lhr', 'waw', 'sgp', 'dfw', 'iad') | ForEach-Object { & $relay $_ }
    $all = @($openRelay) + $probeRelays
    $probeIps = @($probeRelays | ForEach-Object { $_.Ip })
    Note "open $($openRelay.Code) $($openRelay.Ip); probes $(($probeRelays | ForEach-Object { $_.Code }) -join ' ')"

    Note "--- 1. baseline, no block: proves the counter sees this traffic"
    Note "echo, no block: $(LocalEcho 8)"
    CaptureStart 'baseline'
    $run = StartSim 12 $openRelay.Ip $probeIps $Warmup
    $result = FinishSim $run
    NoteCounts 'baseline' (CaptureStop 'baseline') $all
    Note "sim: $result"

    $app = StartApp
    Note "title: $(WaitTitle $app.Hwnd 'no block')"

    Note "--- 2. block under sustained load, with a live relay refresh mid-run"
    PickItem $app $openRelay.Desc
    Note "title: $(WaitTitle $app.Hwnd 'blocked')"
    ToggleConsole $app.Hwnd
    Note "echo, $($openRelay.Code) picked: $(LocalEcho 8)"
    CaptureStart 'block'
    $run = StartSim 30 $openRelay.Ip $probeIps $Warmup
    Start-Sleep -Seconds 15
    ConsoleType $app 'refresh relays'
    $result = FinishSim $run
    NoteCounts 'block' (CaptureStop 'block') $all
    Note "sim: $result"
    Start-Sleep -Seconds 6
    $rows = ConsoleRows $app
    Note "console drops for $($ExeName): $(DropTotals $rows)"
    Note "console filter writes during the run: $(@($rows | Where-Object { $_ -match ' apply: ' }).Count) total since start"

    Note "--- 3. switching the pick mid-flow"
    $other = $probeRelays[0]
    $run = StartSim 30 $openRelay.Ip $probeIps $Warmup
    Start-Sleep -Seconds ([Math]::Max(1, $Warmup + 1))
    CaptureStart 'phase-a'
    Start-Sleep -Seconds 6
    NoteCounts "phase A, picked $($openRelay.Code)" (CaptureStop 'phase-a') @($openRelay, $other)
    PickItem $app $other.Desc
    Note "title: $(WaitTitle $app.Hwnd $other.Desc)"
    CaptureStart 'phase-b'
    Start-Sleep -Seconds 6
    NoteCounts "phase B, picked $($other.Code)" (CaptureStop 'phase-b') @($openRelay, $other)
    PickItem $app $openRelay.Desc
    Note "title: $(WaitTitle $app.Hwnd $openRelay.Desc)"
    CaptureStart 'phase-c'
    Start-Sleep -Seconds 6
    NoteCounts "phase C, picked $($openRelay.Code)" (CaptureStop 'phase-c') @($openRelay, $other)
    Note "sim: $(FinishSim $run)"

    Note "--- 4. No block restores everything"
    PickItem $app $null
    Note "title: $(WaitTitle $app.Hwnd 'no block')"
    CaptureStart 'restored'
    $run = StartSim 8 $openRelay.Ip $probeIps $Warmup
    $result = FinishSim $run
    NoteCounts 'restored' (CaptureStop 'restored') $all
    Note "sim: $result"
    $rows = ConsoleRows $app
    Set-Content (Join-Path $OutDir 'console-log.txt') $rows
    $app.Proc.Kill(); $app.Proc.WaitForExit()
}
catch {
    Note "FAIL  script error: $_ at line $($_.InvocationInfo.ScriptLineNumber)"
}
finally {
    # Cleanup must run to the end, and pktmon reports an error if it was already stopped.
    $ErrorActionPreference = 'Continue'
    & cmd /c "pktmon stop >nul 2>&1"
    & cmd /c "pktmon filter remove >nul 2>&1"
    Get-ChildItem $OutDir -Filter *.etl -ErrorAction SilentlyContinue | Remove-Item -ErrorAction SilentlyContinue
    # The probe deliberately leaves the block in place.
    if ($Scenario -eq 'probe') { Note 'finished'; exit }
    Get-Process dl-conn-rs -ErrorAction SilentlyContinue | Stop-Process -Force
    # Matches on the full path, so a real game with the same process name is never touched.
    Get-Process ([IO.Path]::GetFileNameWithoutExtension($ExeName)) -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $sim } | Stop-Process -Force
    $p = Start-Process $TestExe -ArgumentList 'live_tests::clear_any_block --ignored --exact' -NoNewWindow -PassThru -RedirectStandardOutput (Join-Path $OutDir 'clear.txt')
    [void]$p.WaitForExit(30000)
    Push-Location $OutDir
    & netsh wfp show state file=state-final.xml | Out-Null
    Pop-Location
    Note ("dl-conn-rs filters left behind: {0}" -f (Select-String -Path (Join-Path $OutDir 'state-final.xml') -SimpleMatch 'dl-conn-rs' -Quiet))
    Remove-Item (Join-Path $OutDir 'state-final.xml') -ErrorAction SilentlyContinue
    Note "finished"
}
