# Elevated end-to-end test: real WFP filters, real relays, and the real app. The dropdown is driven
# through window messages, and the console through real keystrokes with its screen read back.
# Run from an admin shell after building:
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
using System; using System.Text; using System.Drawing; using System.Runtime.InteropServices; using Microsoft.Win32.SafeHandles;
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
  [DllImport("user32")] public static extern IntPtr GetSystemMenu(IntPtr h,bool revert);
  [DllImport("user32")] public static extern int GetMenuState(IntPtr m,uint id,uint flags);
  [StructLayout(LayoutKind.Sequential)] public struct RECT{public int L,T,R,B;}
  public static string Title(IntPtr h){var s=new StringBuilder(512);GetWindowText(h,s,512);return s.ToString();}
  public static string Selected(IntPtr c){var i=SendMessage(c,0x147,IntPtr.Zero,IntPtr.Zero);var s=new StringBuilder(256);SendMessage(c,0x148,i,s);return s.ToString();}
  public static void Shot(IntPtr h,string path){RECT r;GetWindowRect(h,out r);using(var b=new Bitmap(r.R-r.L,r.B-r.T))using(var g=Graphics.FromImage(b)){var dc=g.GetHdc();PrintWindow(h,dc,2);g.ReleaseHdc(dc);b.Save(path);}}
}
public static class Con {
  [DllImport("kernel32")] static extern bool FreeConsole();
  [DllImport("kernel32")] static extern bool AllocConsole();
  [DllImport("kernel32")] static extern bool AttachConsole(uint pid);
  [DllImport("kernel32")] public static extern IntPtr GetConsoleWindow();
  [DllImport("user32")] static extern bool ShowWindow(IntPtr h,int cmd);
  [DllImport("kernel32",CharSet=CharSet.Unicode,SetLastError=true)] static extern SafeFileHandle CreateFile(string n,uint a,uint s,IntPtr sa,uint d,uint f,IntPtr t);
  [DllImport("kernel32",EntryPoint="WriteConsoleInputW")] static extern bool WriteInput(SafeFileHandle h,Rec[] r,uint n,out uint w);
  [DllImport("kernel32",EntryPoint="ReadConsoleOutputCharacterW",CharSet=CharSet.Unicode)] static extern bool ReadChars(SafeFileHandle h,StringBuilder s,uint n,Coord c,out uint r);
  [DllImport("kernel32",EntryPoint="GetConsoleScreenBufferInfo")] static extern bool Info(SafeFileHandle h,out Buf b);
  [StructLayout(LayoutKind.Sequential)] struct Coord{public short X,Y;}
  [StructLayout(LayoutKind.Sequential)] struct Buf{public Coord Size,Cursor;public ushort Attr;public short L,T,R,B;public Coord Max;}
  [StructLayout(LayoutKind.Explicit,Size=20)] struct Rec{[FieldOffset(0)]public ushort Type;[FieldOffset(4)]public int Down;[FieldOffset(8)]public ushort Repeat;[FieldOffset(10)]public ushort Vk;[FieldOffset(12)]public ushort Scan;[FieldOffset(14)]public ushort Ch;[FieldOffset(16)]public uint Ctrl;}
  static SafeFileHandle Open(string n){var h=CreateFile(n,0xC0000000,3,IntPtr.Zero,3,0,IntPtr.Zero);if(h.IsInvalid)throw new Exception(n+" error "+Marshal.GetLastWin32Error());return h;}
  public static bool Attach(uint pid){FreeConsole();return AttachConsole(pid);}
  public static void Detach(){FreeConsole();AllocConsole();ShowWindow(GetConsoleWindow(),0);}
  public static void Type(string text){using(var h=Open("CONIN$")){foreach(char c in text){ushort vk=(ushort)(c=='\t'?9:c=='\r'?13:c==(char)27?27:0);var recs=new Rec[2];for(int i=0;i<2;i++){recs[i].Type=1;recs[i].Down=i==0?1:0;recs[i].Repeat=1;recs[i].Vk=vk;recs[i].Ch=(ushort)c;}uint w;WriteInput(h,recs,2,out w);}}}
  public static string[] Rows(){using(var h=Open("CONOUT$")){Buf b;Info(h,out b);var rows=new string[b.Cursor.Y+1];for(int y=0;y<=b.Cursor.Y;y++){var s=new StringBuilder(b.Size.X+1);uint r;var c=new Coord();c.Y=(short)y;ReadChars(h,s,(uint)b.Size.X,c,out r);rows[y]=s.ToString(0,(int)r).TrimEnd();}return rows;}}
}
"@

$CB_GETCOUNT = 0x146; $CB_SELECTSTRING = 0x14D; $CB_SETCURSEL = 0x14E; $CB_SHOWDROPDOWN = 0x14F
$WM_KEYDOWN = 0x100; $VK_OEM_3 = 0xC0; $SC_CLOSE = 0xF060; $ESC = [string][char]27
$script:attached = $false

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

function Backtick($app) { [void][W]::PostMessage($app.Combo, $WM_KEYDOWN, [IntPtr]$VK_OEM_3, [IntPtr]::Zero) }

# Types into the console, then gives the app a moment to react.
function Keys($text, $settleMs = 400) { [Con]::Type($text); Start-Sleep -Milliseconds $settleMs }
function Screen { [Con]::Rows() -join "`n" }
function PromptLine { [Con]::Rows() | Select-Object -Last 1 }
function CountOf($pattern) { @([Con]::Rows() | Where-Object { $_ -match $pattern }).Count }

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

    Note "--- dropdown"
    # Use relays that answer right now, so another tool's rules can't confuse the result.
    $config = Invoke-RestMethod 'https://api.steampowered.com/ISteamApps/GetSDRConfig/v1/?appid=730'
    $live = @($config.pops.PSObject.Properties | Where-Object { $_.Value.relays } | Where-Object { Ping1 $_.Value.relays[0].ipv4 } | Select-Object -First 2)
    Check 'precondition: two relay sites answer ping' ($live.Count -eq 2)
    $openCode = $live[0].Name; $openDesc = $live[0].Value.desc; $open = $live[0].Value.relays[0].ipv4
    $shutCode = $live[1].Name; $shutDesc = $live[1].Value.desc; $shut = $live[1].Value.relays[0].ipv4
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

    Note "--- console: the logger"
    Backtick $app
    Start-Sleep -Milliseconds 800
    $script:attached = [Con]::Attach([uint32]$app.Proc.Id)
    Check 'backtick opens a real console' $script:attached
    $console = [Con]::GetConsoleWindow()
    Check 'console window is visible' ([W]::IsWindowVisible($console))
    Check 'console X button is removed' ([W]::GetMenuState([W]::GetSystemMenu($console, $false), $SC_CLOSE, 0) -eq -1)
    for ($i = 0; $i -lt 20; $i++) { UdpSend $shut; Start-Sleep -Seconds 1 }
    $text = Screen
    Set-Content (Join-Path $OutDir 'console-log.txt') $text
    Check 'history replayed from before it opened' ($text -match 'start: active pick')
    Check 'dropdown choice was logged as a command' ($text -match "> set pick $openCode")
    Check 'report shows processes' ($text -match 'processes: .*ExitLag')
    Check 'report shows picking' ($text -match "picking: $openCode")
    Check 'report shows routes' ($text -match 'via .* gw ')
    Check 'report shows UDP drops' ($text -match "blocked .* -> $shutCode")
    Check 'prompt is the last line' ((PromptLine) -eq 'dl-rs>')
    [W]::Shot($console, (Join-Path $OutDir 'console.png'))

    Note "--- console: typing"
    Keys "stop report`r"
    Check 'typed command runs and echoes' ((Screen) -match '> stop report')
    Keys 'list se'; Keys "`t"
    Check 'tab completes a word' ((PromptLine) -eq 'dl-rs> list servers')
    Keys $ESC
    Check 'esc clears the line' ((PromptLine) -eq 'dl-rs>')
    Keys 's'; Keys "`t"; Keys "`t"
    Check 'second tab lists options with notes' ((Screen) -match 'show\s+status, routes')
    Check 'listing leaves the line intact' ((PromptLine) -eq 'dl-rs> s')
    Keys $ESC
    Keys "set pick $($openCode.Substring(0, 2))"; Keys "`t"
    Check 'tab completes a server code' ((PromptLine) -match "^dl-rs> set pick $($openCode.Substring(0, 2))")
    Keys $ESC
    Keys 'set pick ?'
    Check 'question mark explains the partial line' ((Screen) -match '<server>\s+A code from')
    Check 'question mark leaves the line intact' ((PromptLine) -eq 'dl-rs> set pick')
    Keys $ESC
    Keys "help set pick`r"
    Check 'help explains one command' ((Screen) -match 'Block every relay except')
    Keys "help`r"
    Check 'help lists all commands' ((Screen) -match 'refresh relays' -and (Screen) -match 'toggle console')
    Keys "set pick wibble`r"
    Check 'bad server gives a useful error' ((Screen) -match "unknown server 'wibble'")
    Keys "lst servers`r"
    Check 'typo gets a suggestion' ((Screen) -match "Did you mean 'list servers'")
    Keys "list servers --filter $openCode`r"
    Check 'list servers filters' ((Screen) -match "$openCode\s+\d+ relays")
    Keys "show status`r"
    Check 'show status reports the pick' ((Screen) -match "pick: $openCode ")
    $errors = CountOf 'error: '
    Keys "show processes`r"; Keys "show routes`r"; Keys "ping relays`r" 1500; Keys "list relays $openCode`r"
    Check 'diagnostic commands run without errors' ((CountOf 'error: ') -eq $errors)
    Check 'show routes prints a route' ((Screen) -match 'via .* gw ')

    Note "--- console: one state behind both front-ends"
    Keys "set pick $shutCode`r" 1500
    $title = WaitTitle $app.Hwnd ([regex]::Escape($shutDesc))
    Note "title: $title"
    Check 'typed pick updates the title' ($title.Contains($shutDesc))
    Check 'typed pick updates the dropdown' ([W]::Selected($app.Combo).Contains($shutCode))
    Check 'typed pick moves the block' ((Ping1 $shut) -and -not (Ping1 $open))
    $before = CountOf '> set pick'
    [void][W]::SendMessage($app.Combo, $CB_SHOWDROPDOWN, [IntPtr]1, [IntPtr]::Zero)
    [void][W]::SendMessage($app.Combo, $CB_SHOWDROPDOWN, [IntPtr]::Zero, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 500
    Check 'closing the dropdown unchanged issues nothing' ((CountOf '> set pick') -eq $before)
    PickItem $app $openDesc
    $title = WaitTitle $app.Hwnd ([regex]::Escape($openDesc))
    Start-Sleep -Milliseconds 400
    Check 'one dropdown choice logs exactly one command' ((CountOf '> set pick') -eq ($before + 1))

    Note "--- console: logging while typing"
    Keys "start report --every 1`r"
    Keys 'list se' 200
    $before = CountOf 'processes:'
    Start-Sleep -Seconds 3
    Check 'log keeps scrolling while typing' ((CountOf 'processes:') -gt $before)
    Check 'half-typed command survives log output' ((PromptLine) -eq 'dl-rs> list se')
    Keys "`t"
    Check 'and still completes afterwards' ((PromptLine) -eq 'dl-rs> list servers')
    Keys $ESC
    Keys "stop report`r"
    Keys "clear screen`r" 800
    Check 'clear screen clears' (-not ((Screen) -match 'start: active pick'))
    Check 'prompt remains after clearing' ((PromptLine) -eq 'dl-rs>')

    Note "--- console: show, hide, quit"
    Keys '`' 800
    Check 'backtick in the console hides it' (-not [W]::IsWindowVisible($console))
    Backtick $app
    Start-Sleep -Milliseconds 800
    Check 'backtick in the UI shows it again' ([W]::IsWindowVisible($console))
    Keys "quit`r" 300
    Check 'quit closes the app' ($app.Proc.WaitForExit(5000))
    [Con]::Detach(); $script:attached = $false
    Check 'block outlives the app' (-not (Ping1 $shut))

    Note "--- reopen"
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
    if ($script:attached) { [Con]::Detach() }
    Get-Process dl-rs -ErrorAction SilentlyContinue | Stop-Process -Force
    RunTest 'live_tests::clear_any_block'
    Push-Location $OutDir
    & netsh wfp show state file=state-final.xml | Out-Null
    Pop-Location
    Check 'nothing left behind' (-not (Select-String -Path (Join-Path $OutDir 'state-final.xml') -SimpleMatch 'dl-rs' -Quiet))
    Note "finished with $($script:failures) failures"
}
