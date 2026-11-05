# Drives the real windows with a fake firewall: no admin rights, no filters and no UAC prompt, so it
# can't change a block. It reaches Steam for the relay list and pings the server it picks. Its windows
# may appear on screen, but it takes no focus, so a game keeps its input. A real mouse resting over an
# open list moves its selection, and clicking elsewhere closes it, so either can fail a run. Rerun
# before treating a dropdown failure as real.
#   cargo test --no-run
#   powershell -ExecutionPolicy Bypass -File scripts\ui-test.ps1 -TestExe <target\debug\deps\dl_rs-*.exe> -OutDir <dir>
param(
    [Parameter(Mandatory)][string]$TestExe,
    [Parameter(Mandatory)][string]$OutDir
)
$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force $OutDir | Out-Null
$OutDir = (Resolve-Path $OutDir).Path
$results = Join-Path $OutDir 'results.txt'
Set-Content $results "started $(Get-Date)"
function Note($text) { Add-Content $results $text }
$script:failures = 0
function Check($name, [bool]$ok) {
    if (-not $ok) { $script:failures++ }
    Note ("{0}  {1}" -f $(if ($ok) { 'PASS' } else { 'FAIL' }), $name)
}

Add-Type -AssemblyName System.Drawing
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
Add-Type -ReferencedAssemblies System.Drawing -Path (Join-Path $here 'testhelpers.cs')

$CB_GETCOUNT = 0x146; $CB_GETCURSEL = 0x147; $CB_SELECTSTRING = 0x14D; $CB_SETCURSEL = 0x14E
$CB_SHOWDROPDOWN = 0x14F; $CB_GETDROPPEDSTATE = 0x157
$WM_SETTEXT = 0x0C; $WM_CLOSE = 0x10; $WM_KEYDOWN = 0x100; $WM_CHAR = 0x102; $WM_COMMAND = 0x111
$BM_CLICK = 0xF5; $CBN_SELCHANGE = 1
$VK_RETURN = 0x0D; $VK_ESCAPE = 0x1B; $VK_DOWN = 0x28; $VK_F1 = 0x70; $VK_F2 = 0x71; $VK_F3 = 0x72
$ID_DIRECT = 2; $ID_ROUTE = 3; $ID_STATUS = 3; $ID_RERUN = 2; $ID_DEFAULTS = 1; $ID_FIRST_FIELD = 100

function Send($h, $message, $w = 0, $l = 0) { [W]::SendMessage($h, $message, [IntPtr]$w, [IntPtr]$l) }
function Key($h, $code) { [void][W]::PostMessage($h, $WM_KEYDOWN, [IntPtr]$code, [IntPtr]::Zero) }
function TypeText($h, $text) {
    foreach ($c in $text.ToCharArray()) {
        [void][W]::PostMessage($h, $WM_CHAR, [IntPtr][int]$c, [IntPtr]::Zero)
        Start-Sleep -Milliseconds 60
    }
    Start-Sleep -Milliseconds 300
}
function WaitFor([scriptblock]$condition, $seconds = 15) {
    $until = (Get-Date).AddSeconds($seconds)
    while ((Get-Date) -lt $until) {
        if (& $condition) { return $true }
        Start-Sleep -Milliseconds 100
    }
    return $false
}
function Count($combo) { [int](Send $combo $CB_GETCOUNT) }
function IsOpen($combo) { [int](Send $combo $CB_GETDROPPEDSTATE) -ne 0 }
function FindTop($class) { [W]::FindWindow($class, [NullString]::Value) }

# The settings file and any report data stay in the output folder, away from the real ones.
$data = Join-Path $OutDir 'appdata'
Remove-Item -Recurse -Force $data -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force (Join-Path $data 'dl-rs') | Out-Null
$ini = Join-Path $data 'dl-rs\settings.ini'
$env:LOCALAPPDATA = $data

# Without this the test exe has no common controls manifest, so controls would look and act older.
Copy-Item (Join-Path (Split-Path -Parent $here) 'app.manifest') "$TestExe.manifest" -Force

$out = Join-Path $OutDir 'app.txt'
$app = Start-Process $TestExe -ArgumentList 'ui::tests::windows_with_a_fake_firewall --ignored --exact --nocapture --test-threads=1' `
    -RedirectStandardOutput $out -RedirectStandardError "$out.err" -NoNewWindow -PassThru
$null = $app.Handle # Without this, ExitCode stays empty after the process ends.

try {
    Check 'main window opens' (WaitFor { (FindTop 'dl-rs') -ne [IntPtr]::Zero })
    $main = FindTop 'dl-rs'
    $combo = [W]::FindWindowEx($main, [IntPtr]::Zero, 'COMBOBOX', [NullString]::Value)
    $edit = [W]::FindWindowEx($combo, [IntPtr]::Zero, 'Edit', [NullString]::Value)
    Check 'the dropdown has an edit box to type in' ($edit -ne [IntPtr]::Zero)
    Check 'relays load' (WaitFor { [W]::Title($main) -match 'no block' } 30)
    Check 'the list fills' (WaitFor { (Count $combo) -gt 30 })

    Note '--- dropdown'
    $items = 0..((Count $combo) - 1) | ForEach-Object { [W]::Item($combo, $_) }
    Set-Content (Join-Path $OutDir 'rows.txt') $items
    Check 'No block is first' ($items[0] -like 'No block*')
    foreach ($region in 'Africa', 'Asia', 'Europe', 'Middle East', 'North America', 'Oceania', 'South America') {
        Check "section $region" ($items -contains $region)
    }
    Check 'nothing lands in Other' ($items -notcontains 'Other')
    Check 'sections are in order' ([array]::IndexOf($items, 'Africa') -lt [array]::IndexOf($items, 'Europe'))
    Check 'a site sits under its section' ([array]::IndexOf($items, 'Africa') -eq [array]::IndexOf($items, ($items -like 'Johannesburg*')[0]) - 1)
    Check 'the counters start clear' (([W]::Text([W]::GetDlgItem($main, $ID_DIRECT))) -eq 'Direct: pick a server')
    [W]::Shot($main, (Join-Path $OutDir 'main.png'))

    [void](Send $combo $CB_SHOWDROPDOWN 1)
    Start-Sleep -Milliseconds 300
    [W]::Shot((FindTop 'ComboLBox'), (Join-Path $OutDir 'list-full.png'))
    [void](Send $combo $CB_SHOWDROPDOWN 0)
    Start-Sleep -Milliseconds 300

    Note '--- search'
    $before = Count $combo
    TypeText $edit 'frank'
    Check 'typing narrows the list' ((Count $combo) -lt $before -and (Count $combo) -gt 0)
    Check 'typing keeps the text' ([W]::Text($combo) -eq 'frank')
    Check 'typing opens the list' (IsOpen $combo)
    Check 'typing highlights the first match' ([int](Send $combo $CB_GETCURSEL) -eq 1)
    $shown = 0..((Count $combo) - 1) | ForEach-Object { [W]::Item($combo, $_) }
    Note "search rows: $($shown -join ' | ')"
    Check 'only matches and their section are left' (($shown | Where-Object { $_ -notmatch 'Frankfurt|^Europe$' }).Count -eq 0)
    $listHeight = [W]::Rect((FindTop 'ComboLBox'))
    Check 'the list shrinks to what the search left' (($listHeight[3] - $listHeight[1]) -lt 200)
    [W]::Shot((FindTop 'ComboLBox'), (Join-Path $OutDir 'list-search.png'))
    [W]::Shot($main, (Join-Path $OutDir 'main-search.png'))

    Key $edit $VK_ESCAPE
    Check 'Esc closes the search' (WaitFor { -not (IsOpen $combo) } 3)
    Check 'Esc restores the list' (WaitFor { (Count $combo) -eq $before } 3)
    Check 'Esc picks nothing' ([W]::Title($main) -match 'no block')
    Check 'Esc restores the text' ([W]::Text($combo) -like 'No block*')

    TypeText $edit 'sterl'
    Key $edit $VK_RETURN
    Check 'Enter picks the first match' (WaitFor { [W]::Title($main) -match 'Sterling' })
    Check 'Enter closes the list' (WaitFor { -not (IsOpen $combo) } 3)
    Check 'the list comes back in full' (WaitFor { (Count $combo) -eq $before } 3)
    Check 'the pick shows in the box' (WaitFor { [W]::Text($combo) -like 'Sterling*iad' } 3)
    Check 'the direct counter measures' (WaitFor { [W]::Text([W]::GetDlgItem($main, $ID_DIRECT)) -match 'Direct: (\d+ ms to iad|no reply from iad)' } 20)
    Note ('direct: ' + [W]::Text([W]::GetDlgItem($main, $ID_DIRECT)))
    Note ('route: ' + [W]::Text([W]::GetDlgItem($main, $ID_ROUTE)))
    [W]::Shot($main, (Join-Path $OutDir 'main-picked.png'))

    TypeText $edit 'zzzz'
    Check 'a search with no match empties the list' ((Count $combo) -eq 0)
    Key $edit $VK_RETURN
    Start-Sleep -Milliseconds 400
    Check 'Enter with no match changes nothing' ([W]::Title($main) -match 'Sterling')
    Key $edit $VK_ESCAPE
    Check 'and Esc still restores' (WaitFor { (Count $combo) -eq $before } 3)

    Note '--- headers'
    $africa = [array]::IndexOf($items, 'Africa')
    [void](Send $combo $CB_SHOWDROPDOWN 1)
    [void](Send $combo $CB_SETCURSEL ($africa - 1))
    Key $edit $VK_DOWN
    Start-Sleep -Milliseconds 300
    $landed = [int](Send $combo $CB_GETCURSEL)
    Note "arrow from $($africa - 1) landed on $landed $($items[$landed])"
    Check 'the arrow key skips a header' ($landed -eq $africa + 1)
    Key $edit $VK_ESCAPE
    Check 'Esc from an open list closes it' (WaitFor { -not (IsOpen $combo) } 3)
    Start-Sleep -Milliseconds 300
    Check 'and keeps the pick' ([W]::Title($main) -match 'Sterling')

    Note '--- the way the test scripts pick'
    [void](Send $combo $CB_SHOWDROPDOWN 1)
    [void][W]::SendMessage($combo, $CB_SELECTSTRING, [IntPtr](-1), [string]'Johannesburg')
    [void](Send $combo $CB_SHOWDROPDOWN 0)
    Note ("selection after picking: " + [W]::Selected($combo) + ", count " + (Count $combo))
    Check 'CB_SELECTSTRING still picks' (WaitFor { [W]::Title($main) -match 'Johannesburg' })
    Check 'the selection reads back' ([W]::Selected($combo).Contains('jnb'))
    [void](Send $combo $CB_SHOWDROPDOWN 1)
    [void](Send $combo $CB_SETCURSEL 0)
    [void](Send $combo $CB_SHOWDROPDOWN 0)
    Check 'index 0 is still No block' (WaitFor { [W]::Title($main) -match 'no block' })
    Check 'the direct counter clears' (WaitFor { [W]::Text([W]::GetDlgItem($main, $ID_DIRECT)) -eq 'Direct: pick a server' })

    Note '--- F1'
    Key $combo $VK_F1
    Check 'F1 opens help' (WaitFor { (FindTop 'dl-rs-text') -ne [IntPtr]::Zero })
    $help = FindTop 'dl-rs-text'
    $text = [W]::GetDlgItem($help, 1)
    Check 'help has the steps' (WaitFor { [W]::Text($text) -match 'STEP 3: TEST IN A CUSTOM GAME' })
    Check 'help is plain ascii' ([W]::Text($text) -notmatch '[^\x00-\x7F]')
    [W]::Shot($help, (Join-Path $OutDir 'f1.png'))
    Key $text $VK_ESCAPE
    Check 'Esc closes help' (WaitFor { -not [W]::IsWindow($help) } 3)

    Note '--- F2'
    Key $combo $VK_F2
    Check 'F2 opens settings' (WaitFor { (FindTop 'dl-rs-settings') -ne [IntPtr]::Zero })
    $form = FindTop 'dl-rs-settings'
    $status = [W]::GetDlgItem($form, $ID_STATUS)
    # The rows are built once the settings arrive from the session.
    Check 'the rows appear' (WaitFor { [W]::GetDlgItem($form, $ID_FIRST_FIELD + 1) -ne [IntPtr]::Zero })
    $ping = [W]::GetDlgItem($form, $ID_FIRST_FIELD + 1)
    $hold = [W]::GetDlgItem($form, $ID_FIRST_FIELD + 3)
    Check 'the form fills' (WaitFor { [W]::Text($ping) -eq '5' })
    [W]::Shot($form, (Join-Path $OutDir 'f2.png'))

    [void][W]::SendMessage($ping, $WM_SETTEXT, [IntPtr]::Zero, [string]'30')
    Key $ping $VK_RETURN
    Check 'a good value saves' (WaitFor { (Test-Path $ini) -and ((Get-Content $ini -Raw) -match 'ping_every=30') })
    Check 'and says so' (WaitFor { [W]::Text($status) -match 'Saved: ping_every = 30' } 3)

    [void][W]::SendMessage($ping, $WM_SETTEXT, [IntPtr]::Zero, [string]'999')
    Key $ping $VK_RETURN
    Check 'a bad value shows the error' (WaitFor { [W]::Text($status) -match 'Not changed: ping_every must be 0 to 300' })
    Check 'and goes back' (WaitFor { [W]::Text($ping) -eq '30' } 3)
    Check 'and saves nothing' ((Get-Content $ini -Raw) -match 'ping_every=30')
    [W]::Shot($form, (Join-Path $OutDir 'f2-error.png'))

    [void](Send $hold $CB_SETCURSEL 0)
    [void](Send $form $WM_COMMAND (($ID_FIRST_FIELD + 3) -bor ($CBN_SELCHANGE -shl 16)) $hold)
    Check 'a toggle saves' (WaitFor { (Get-Content $ini -Raw) -match 'hold_in_game=on' })

    [void](Send ([W]::GetDlgItem($form, $ID_DEFAULTS)) $BM_CLICK)
    Check 'Defaults resets the file' (WaitFor { (Get-Content $ini -Raw) -match 'ping_every=5' })
    Check 'Defaults resets the form' (WaitFor { [W]::Text($ping) -eq '5' } 3)
    Key $ping $VK_ESCAPE
    Check 'Esc closes settings' (WaitFor { -not [W]::IsWindow($form) } 3)

    Note '--- F3'
    # Starting with no log proves the report explains itself. Then it reads a sample with a path.
    $sample = Join-Path $OutDir 'sample-console.log'
    Set-Content $sample @'
11/05 18:46:13 [CommandLine] -steam -condebug -lognetapi
11/05 18:46:18 [SteamNetSockets] Got SDR network config.  Loaded revision 1793736599 OK
11/05 18:46:31 [SteamNetSockets] Ping measurement completed in 12.1s.  Relays: 1 valid, 0 great, 0 good+, 1 ok+, 0 ignored
11/05 18:46:31 [SteamNetSockets] Ping location: sgp=120+12/121+12,maa2=/152+12
11/05 18:46:43 [SteamNetSockets] [#154 SDR server steamid:76561190000000002 vport 0] Selecting sgp#276 (103.10.124.121:27060) as primary.  (Ping = 121 = 120+1+0 (front+interior+remote).  Rank=1 Score=133)
'@
    Key $combo $VK_F2
    Check 'F2 opens again' (WaitFor { (FindTop 'dl-rs-settings') -ne [IntPtr]::Zero })
    $form = FindTop 'dl-rs-settings'
    Check 'the rows appear again' (WaitFor { [W]::GetDlgItem($form, $ID_FIRST_FIELD + 7) -ne [IntPtr]::Zero })
    $path = [W]::GetDlgItem($form, $ID_FIRST_FIELD + 7)
    Check 'the form fills again' (WaitFor { [W]::Text($path) -eq 'auto' })
    [void][W]::SendMessage($path, $WM_SETTEXT, [IntPtr]::Zero, [string]$sample)
    Key $path $VK_RETURN
    Check 'a path with its case is saved' (WaitFor { (Get-Content $ini -Raw).Contains($sample) })
    Key $path $VK_ESCAPE
    Check 'settings closes' (WaitFor { -not [W]::IsWindow($form) } 3)

    Key $combo $VK_F3
    Check 'F3 opens the report' (WaitFor { (FindTop 'dl-rs-text') -ne [IntPtr]::Zero })
    $report = FindTop 'dl-rs-text'
    $text = [W]::GetDlgItem($report, 1)
    Check 'the report reads the log' (WaitFor { [W]::Text($text) -match 'FINDINGS' })
    $body = [W]::Text($text)
    Set-Content (Join-Path $OutDir 'report.txt') $body
    Check 'the report shows the primary' ($body -match 'sgp#276')
    Check 'the report hides Steam IDs' ($body -notmatch 'steamid|7656')
    [W]::Shot($report, (Join-Path $OutDir 'f3.png'))
    Add-Content $sample '11/05 18:50:00 [SteamNetSockets] [#154 SDR server vport 0] closed by app (1002) NETWORK_DISCONNECT_DISCONNECT_BY_USER'
    [void](Send ([W]::GetDlgItem($report, $ID_RERUN)) $BM_CLICK)
    Check 'Re-run picks up new lines' (WaitFor { [W]::Text($text) -match 'DISCONNECT_BY_USER' })
    Key $text $VK_ESCAPE
    Check 'Esc closes the report' (WaitFor { -not [W]::IsWindow($report) } 3)

    Note '--- close'
    Key $combo $VK_F1
    Check 'help opens once more' (WaitFor { (FindTop 'dl-rs-text') -ne [IntPtr]::Zero })
    [void][W]::PostMessage($main, $WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
    Check 'the X button closes the app' ($app.WaitForExit(5000))
    Check 'the app exits cleanly' ($app.ExitCode -eq 0)
}
catch {
    $script:failures++
    Note "FAIL  script error: $_ at line $($_.InvocationInfo.ScriptLineNumber)"
}
finally {
    if (-not $app.HasExited) { $app.Kill() }
    Remove-Item "$TestExe.manifest" -Force -ErrorAction SilentlyContinue
    Note "finished with $($script:failures) failures"
}
Get-Content $results | Where-Object { $_ -match '^FAIL|^finished' }
exit $script:failures
