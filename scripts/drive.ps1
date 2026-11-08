# Shared by the test scripts: drive the egui window and read what it did. Dot-source this file.
#
# Keys are posted to the window and text is set through Windows UI Automation, the interface screen
# readers use, so nothing takes focus and a game keeps its input. Results come from the window title,
# the app's log file and the same accessibility tree. Real mouse clicks are the one thing here that
# moves the user's pointer.
# A window that's minimized or covered runs no UI pass and drops its input, unless it's told otherwise.
$env:DL_CONN_RS_DRAW_HIDDEN = '1'
Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type -ReferencedAssemblies System.Drawing -Path (Join-Path $PSScriptRoot 'testhelpers.cs')

$VK_RETURN = 0x0D; $VK_ESCAPE = 0x1B; $VK_DOWN = 0x28; $VK_F1 = 0x70; $VK_F2 = 0x71; $VK_F3 = 0x72
$VK_TAB = 0x09

function WaitForWindow($process, $seconds = 20) {
    $until = (Get-Date).AddSeconds($seconds)
    while ((Get-Date) -lt $until) {
        if ($process.HasExited) { return [IntPtr]::Zero }
        $hwnd = [W]::TitledWindow($process.Id)
        if ($hwnd -ne [IntPtr]::Zero) { return $hwnd }
        Start-Sleep -Milliseconds 50
    }
    return [IntPtr]::Zero
}

# Returns the title once it matches, or a line saying what it was instead.
function WaitTitle($hwnd, $pattern, $seconds = 20) {
    $until = (Get-Date).AddSeconds($seconds)
    while ((Get-Date) -lt $until) {
        $title = [W]::Title($hwnd)
        if ($title -match $pattern) { return $title }
        Start-Sleep -Milliseconds 100
    }
    return "TIMEOUT, title was: " + [W]::Title($hwnd)
}

function Press($hwnd, $key, $settleMs = 150) {
    [W]::PostKey($hwnd, [uint32]$key)
    Start-Sleep -Milliseconds $settleMs
}

# Sets a text box's contents, as dictation or a screen reader would. No focus is needed.
function SetText($hwnd, $name, $text, $settleMs = 150) {
    $element = UiFindEdit $hwnd $name
    if (-not $element) { throw "no text box named $name" }
    $pattern = $element.GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern)
    $pattern.SetValue($text)
    Start-Sleep -Milliseconds $settleMs
}

# Runs a command through the in-app console, which must already be open.
function RunCommand($hwnd, $line, $settleMs = 400) {
    SetText $hwnd 'dl-conn-rs>' $line
    Press $hwnd $VK_RETURN $settleMs
}

# Types into the dropdown's search box, which opens its list.
function SearchServers($hwnd, $text) {
    SetText $hwnd 'Server' $text 300
}

function ToggleConsole($hwnd, $settleMs = 500) {
    Press $hwnd 0xC0 $settleMs
}

# The app's log. It's held open for appending, so it's read with shared access.
function LogLines($dataDir) {
    $path = Join-Path $dataDir 'dl-conn-rs.log'
    if (-not (Test-Path $path)) { return @() }
    $stream = [IO.File]::Open($path, 'Open', 'Read', 'ReadWrite,Delete')
    $reader = New-Object IO.StreamReader($stream)
    try { return ($reader.ReadToEnd() -split "`r?`n") } finally { $reader.Close() }
}

function LogCount($dataDir, $pattern) {
    @(LogLines $dataDir | Where-Object { $_ -match $pattern }).Count
}

function WaitLog($dataDir, $pattern, $seconds = 15) {
    $until = (Get-Date).AddSeconds($seconds)
    while ((Get-Date) -lt $until) {
        if ((LogCount $dataDir $pattern) -gt 0) { return $true }
        Start-Sleep -Milliseconds 100
    }
    return $false
}

function UiRoot($hwnd) { [Windows.Automation.AutomationElement]::FromHandle($hwnd) }

# Finds a control by its accessible name, waiting because the tree appears after the first query.
function UiFind($hwnd, $name, $seconds = 5) {
    $condition = New-Object Windows.Automation.PropertyCondition([Windows.Automation.AutomationElement]::NameProperty, $name)
    $until = (Get-Date).AddSeconds($seconds)
    while ((Get-Date) -lt $until) {
        $found = (UiRoot $hwnd).FindFirst([Windows.Automation.TreeScope]::Descendants, $condition)
        if ($found) { return $found }
        Start-Sleep -Milliseconds 100
    }
    return $null
}

# A label and the box it names share a name, so this looks for the box.
function UiFindEdit($hwnd, $name, $seconds = 5) {
    $condition = New-Object Windows.Automation.AndCondition(
        (New-Object Windows.Automation.PropertyCondition([Windows.Automation.AutomationElement]::NameProperty, $name)),
        (New-Object Windows.Automation.PropertyCondition([Windows.Automation.AutomationElement]::ControlTypeProperty, [Windows.Automation.ControlType]::Edit)))
    $until = (Get-Date).AddSeconds($seconds)
    while ((Get-Date) -lt $until) {
        $found = (UiRoot $hwnd).FindFirst([Windows.Automation.TreeScope]::Descendants, $condition)
        if ($found) { return $found }
        Start-Sleep -Milliseconds 100
    }
    return $null
}

# The names of everything the window shows, which is how to read its text.
function UiNames($hwnd) {
    (UiRoot $hwnd).FindAll([Windows.Automation.TreeScope]::Descendants, [Windows.Automation.Condition]::TrueCondition) |
        ForEach-Object { $_.Current.Name } | Where-Object { $_ }
}

function PromptText($hwnd) {
    $element = UiFindEdit $hwnd 'dl-conn-rs>'
    $value = $element.GetCurrentPropertyValue([Windows.Automation.ValuePattern]::ValueProperty)
    # An empty box reports no value at all rather than an empty string.
    if ($value -is [string]) { $value } else { '' }
}

function UiHas($hwnd, $name, $seconds = 5) { $null -ne (UiFind $hwnd $name $seconds) }

# Clicks a control with the real mouse, so this needs the window in front and moves the pointer.
function UiClick($hwnd, $name) {
    $element = UiFind $hwnd $name
    if (-not $element) { throw "no control named $name" }
    $box = $element.Current.BoundingRectangle
    [W]::Click([int]($box.X + $box.Width / 2), [int]($box.Y + $box.Height / 2))
    Start-Sleep -Milliseconds 300
}

# Clicks away from a dropdown on the title bar, because an open list covers the window below it.
function ClickTitleBar($hwnd) {
    $frame = [W]::Rect($hwnd)
    [W]::Click([int](($frame[0] + $frame[2]) / 2), $frame[1] + 10)
    Start-Sleep -Milliseconds 300
}
