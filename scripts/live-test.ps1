# Elevated end-to-end test: real WFP filters, real relays, and the real app driven through its
# window messages. Run from an admin shell after building:
#   cargo test --no-run; cargo build --release
#   powershell -ExecutionPolicy Bypass -File scripts\live-test.ps1 -TestExe <target\debug\deps\dl_rs-*.exe> -AppExe <target\release\dl-rs.exe> -OutDir <dir>
param(
    [Parameter(Mandatory)][string]$TestExe,
    [Parameter(Mandatory)][string]$OutDir,
    [Parameter(Mandatory)][string]$AppExe
)
$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force $OutDir | Out-Null
$log = Join-Path $OutDir 'results.txt'
Set-Content $log "started $(Get-Date)"
function Note($text) { Add-Content $log $text }
$script:failures = 0
function Check($name, [bool]$ok) {
    if (-not $ok) { $script:failures++ }
    Note ("{0}  {1}" -f $(if ($ok) { 'PASS' } else { 'FAIL' }), $name)
}

Add-Type -AssemblyName System.Drawing
Add-Type -ReferencedAssemblies System.Drawing @"
using System; using System.Text; using System.Drawing; using System.Runtime.InteropServices;
public static class W {
  [DllImport("user32",CharSet=CharSet.Unicode)] public static extern IntPtr FindWindow(string c,string t);
  [DllImport("user32",CharSet=CharSet.Unicode)] public static extern IntPtr FindWindowEx(IntPtr p,IntPtr a,string c,string t);
  [DllImport("user32",CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr h,StringBuilder s,int n);
  [DllImport("user32",CharSet=CharSet.Unicode)] public static extern IntPtr SendMessage(IntPtr h,uint m,IntPtr w,IntPtr l);
  [DllImport("user32",CharSet=CharSet.Unicode)] public static extern IntPtr SendMessage(IntPtr h,uint m,IntPtr w,string l);
  [DllImport("user32",CharSet=CharSet.Unicode)] public static extern IntPtr SendMessage(IntPtr h,uint m,IntPtr w,StringBuilder l);
  [DllImport("user32")] public static extern bool PostMessage(IntPtr h,uint m,IntPtr w,IntPtr l);
  [DllImport("user32")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32")] public static extern bool GetWindowRect(IntPtr h,out RECT r);
  [DllImport("user32")] public static extern bool PrintWindow(IntPtr h,IntPtr dc,uint f);
  [StructLayout(LayoutKind.Sequential)] public struct RECT{public int L,T,R,B;}
  public static string Title(IntPtr h){var s=new StringBuilder(512);GetWindowText(h,s,512);return s.ToString();}
  public static string Text(IntPtr h){int n=(int)SendMessage(h,0x0E,IntPtr.Zero,IntPtr.Zero);var s=new StringBuilder(n+1);SendMessage(h,0x0D,(IntPtr)(n+1),s);return s.ToString();}
  public static void Shot(IntPtr h,string path){RECT r;GetWindowRect(h,out r);using(var b=new Bitmap(r.R-r.L,r.B-r.T))using(var g=Graphics.FromImage(b)){var dc=g.GetHdc();PrintWindow(h,dc,2);g.ReleaseHdc(dc);b.Save(path);}}
}
"@

$CB_GETCOUNT = 0x146; $CB_SELECTSTRING = 0x14D; $CB_SETCURSEL = 0x14E; $CB_SHOWDROPDOWN = 0x14F
$WM_KEYDOWN = 0x100; $VK_OEM_3 = 0xC0

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

# An unconnected UDP send reports success even when dropped; the drop shows up in the console.
function UdpSend($ip) {
    $udp = New-Object Net.Sockets.UdpClient
    try { [void]$udp.Send([byte[]](1, 2, 3), 3, $ip, 27015) } finally { $udp.Close() }
}

function WaitTitle($hwnd, $pattern, $seconds = 20) {
    $until = (Get-Date).AddSeconds($seconds)
    while ((Get-Date) -lt $until) {
        $title = [W]::Title($hwnd)
        if ($title -match $pattern) { return $title }
        Start-Sleep -Milliseconds 200
    }
    return "TIMEOUT, title was: " + [W]::Title($hwnd)
}

function StartApp {
    $proc = Start-Process $AppExe -PassThru
    $until = (Get-Date).AddSeconds(10); $hwnd = [IntPtr]::Zero
    while ($hwnd -eq [IntPtr]::Zero -and (Get-Date) -lt $until) {
        Start-Sleep -Milliseconds 100
        $hwnd = [W]::FindWindow('dl-rs', [NullString]::Value)
    }
    $combo = [W]::FindWindowEx($hwnd, [IntPtr]::Zero, 'COMBOBOX', [NullString]::Value)
    return @{ Proc = $proc; Hwnd = $hwnd; Combo = $combo }
}

# Picks through the same path as a user: open the list, change selection, close it.
function PickItem($app, $prefix) {
    [void][W]::SendMessage($app.Combo, $CB_SHOWDROPDOWN, [IntPtr]1, [IntPtr]::Zero)
    if ($prefix) { [void][W]::SendMessage($app.Combo, $CB_SELECTSTRING, [IntPtr](-1), [string]$prefix) }
    else { [void][W]::SendMessage($app.Combo, $CB_SETCURSEL, [IntPtr]::Zero, [IntPtr]::Zero) }
    [void][W]::SendMessage($app.Combo, $CB_SHOWDROPDOWN, [IntPtr]::Zero, [IntPtr]::Zero)
}

try {
    Note "--- environment"
    Note ((Get-Process | Where-Object { $_.ProcessName -match 'exitlag|deadlock|cs2|dota2' } | ForEach-Object { "$($_.ProcessName)#$($_.Id)" }) -join ' ')
    Note (& netsh wfp show options optionsfor=netevents | Out-String)
    Push-Location $OutDir
    & netsh wfp show filters file=filters.xml | Out-Null
    Pop-Location

    Note "--- filter tests"
    RunTest 'live_tests::blocks_only_the_target_and_clears_cleanly'
    RunTest 'live_tests::reports_dropped_packets'
    RunTest 'live_tests::live_udp_flow_is_cut_and_restored'
    RunTest 'live_tests::survives_exit_part1'
    RunTest 'live_tests::survives_exit_part2'

    Note "--- app"
    # Use relays that answer right now, so another tool's rules can't confuse the result.
    $config = Invoke-RestMethod 'https://api.steampowered.com/ISteamApps/GetSDRConfig/v1/?appid=730'
    $live = @($config.pops.PSObject.Properties | Where-Object { $_.Value.relays } | Where-Object { Ping1 $_.Value.relays[0].ipv4 } | Select-Object -First 2)
    Check 'precondition: two relay sites answer ping' ($live.Count -eq 2)
    $openCode = $live[0].Name; $openDesc = $live[0].Value.desc; $open = $live[0].Value.relays[0].ipv4
    $shutCode = $live[1].Name; $shut = $live[1].Value.relays[0].ipv4
    Note "pick $openCode $open, expect blocked $shutCode $shut"

    $app = StartApp
    Check 'main window opens' ($app.Hwnd -ne [IntPtr]::Zero)
    if ($app.Hwnd -eq [IntPtr]::Zero) { throw 'no main window' }
    Note ('title: ' + (WaitTitle $app.Hwnd 'no block'))
    $count = [int][W]::SendMessage($app.Combo, $CB_GETCOUNT, [IntPtr]::Zero, [IntPtr]::Zero)
    Check "dropdown lists all relay sites ($count items)" ($count -gt 30)
    [void][W]::SendMessage($app.Combo, $CB_SHOWDROPDOWN, [IntPtr]1, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 300
    [W]::Shot([W]::FindWindow('ComboLBox', [NullString]::Value), (Join-Path $OutDir 'dropdown.png'))
    [void][W]::SendMessage($app.Combo, $CB_SHOWDROPDOWN, [IntPtr]::Zero, [IntPtr]::Zero)

    PickItem $app $openDesc
    $title = WaitTitle $app.Hwnd '\d+ blocked'
    Note "title: $title"
    Check 'pick shows in title' ($title.Contains($openDesc) -and $title -match '\d+ blocked')
    Check 'picked relay stays reachable' (Ping1 $open)
    Push-Location $OutDir
    & netsh wfp show state file=state-active.xml | Out-Null
    Pop-Location
    Check 'other relay ping blocked' (-not (Ping1 $shut))
    [W]::Shot($app.Hwnd, (Join-Path $OutDir 'main.png'))

    [void][W]::PostMessage($app.Combo, $WM_KEYDOWN, [IntPtr]$VK_OEM_3, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 500
    $console = [W]::FindWindow('dl-rs-console', [NullString]::Value)
    Check 'backtick opens console' ([W]::IsWindowVisible($console))
    for ($i = 0; $i -lt 20; $i++) { UdpSend $shut; Start-Sleep -Seconds 1 }
    $edit = [W]::FindWindowEx($console, [IntPtr]::Zero, 'EDIT', [NullString]::Value)
    $text = [W]::Text($edit)
    Set-Content (Join-Path $OutDir 'console.txt') $text
    [W]::Shot($console, (Join-Path $OutDir 'console.png'))
    Check 'console shows processes' ($text -match 'processes: .*ExitLag')
    Check 'console shows picking' ($text -match "picking: $openCode")
    Check 'console shows routes' ($text -match 'via .* gw ')
    Check 'console shows UDP drops' ($text -match "blocked .* -> $shutCode")
    [void][W]::PostMessage($app.Combo, $WM_KEYDOWN, [IntPtr]$VK_OEM_3, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 500
    Check 'backtick hides console' (-not [W]::IsWindowVisible($console))

    $app.Proc.Kill(); $app.Proc.WaitForExit()
    Check 'block outlives the app' (-not (Ping1 $shut))

    $app = StartApp
    $title = WaitTitle $app.Hwnd '\d+ blocked'
    Note "reopened title: $title"
    Check 'reopened app recovers the pick from WFP' ($title.Contains($openDesc))

    PickItem $app $null
    $title = WaitTitle $app.Hwnd 'no block'
    Note "title: $title"
    Check 'No block option clears' ($title -match 'no block')
    Check 'relay reachable again' (Ping1 $shut)
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
