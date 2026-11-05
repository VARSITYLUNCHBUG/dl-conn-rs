# Prints the relay and connection lines from the game's console.log, with Steam IDs removed. It's
# read-only and uses shared access, so it's safe while the game is running and needs no admin.
# Needs the -condebug launch option.
#   powershell -ExecutionPolicy Bypass -File scripts\game-log.ps1 [-Path <console.log>] [-Last 40]
param(
    [string]$Path,
    [int]$Last = 40
)
$ErrorActionPreference = 'Stop'

function Find-Log {
    $relative = 'steamapps\common\Deadlock\game\citadel\console.log'
    $steam = (Get-ItemProperty 'HKCU:\Software\Valve\Steam' -ErrorAction SilentlyContinue).SteamPath
    if (-not $steam) { $steam = 'C:\Program Files (x86)\Steam' }
    $steam = $steam -replace '/', '\'
    $libraries = @($steam)
    $folders = Join-Path $steam 'steamapps\libraryfolders.vdf'
    if (Test-Path $folders) {
        foreach ($line in Get-Content $folders) {
            if ($line -match '"path"\s+"([^"]+)"') { $libraries += $Matches[1] -replace '\\\\', '\' }
        }
    }
    foreach ($library in $libraries) {
        $candidate = Join-Path $library $relative
        if (Test-Path $candidate) { return $candidate }
    }
    throw "console.log not found under: $($libraries -join ', '). Add -condebug to Deadlock's launch options and run the game once."
}

if (-not $Path) { $Path = Find-Log }
$item = Get-Item $Path
"file: $Path"
"size: {0:N0} KB, last write {1}" -f ($item.Length / 1KB), $item.LastWriteTime
foreach ($name in 'deadlock', 'project8', 'ExitLag', 'dl-rs') {
    foreach ($p in Get-Process $name -ErrorAction SilentlyContinue) { "running: $($p.ProcessName)#$($p.Id)" }
}

$stream = [IO.File]::Open($Path, 'Open', 'Read', 'ReadWrite,Delete')
$reader = New-Object IO.StreamReader($stream)
$lines = $reader.ReadToEnd() -split "`r?`n"
$reader.Close(); $stream.Close()
"lines: $($lines.Count)"

$relay = 'Ping measurement|Ping location|Got SDR network config|SDR network config fetch|Requesting session|Selecting |Lost (primary|backup)|Switch(ed|ing) .*(primary|relay)|Disconnecting from server|Received Steam datagram ticket|for Match \d+ created'
$matched = $lines | Where-Object {
    ($_ -match $relay) -or ($_ -match '\[SteamNetSockets\] \[#\d+ SDR .*\] (connected|closed)')
}
"--- relay and connection events (last $Last of $(@($matched).Count))"
$matched | Select-Object -Last $Last | ForEach-Object {
    $_ -replace 'steamid:\d+', 'steamid:<id>' -replace '7656\d{13}', '<steamid>' -replace '\[[AUGI]:\d:\d+(:\d+)?\]', '<steamid>' -replace 'server_steam_id: \d+', 'server_steam_id: <id>'
}
