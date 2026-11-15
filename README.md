# dl-conn-rs

Pick one Steam relay server and block all the others, so your game can only connect through the
server you chose. The block lasts until you pick again or restart the PC. Works with or without
ExitLag.

dl-conn-rs chooses which relays the game can reach. It does not make any route faster.

Full documentation is in the [wiki](https://github.com/VARSITYLUNCHBUG/dl-conn-rs/wiki).

## Requirements

- Windows 10 or 11, or Windows Server 2016 or later, 64-bit (x64).
- Administrator rights. The app asks for them when it starts.
- DirectX 12. A PC with no usable GPU falls back to the Windows software renderer.

Windows 7, 8.1 and Windows on ARM are not supported.

## Download and verify

1. Open the [latest release](https://github.com/VARSITYLUNCHBUG/dl-conn-rs/releases/latest).
2. Download `dl-conn-rs.exe` and `dl-conn-rs.exe.md5`.
3. Check the file matches. In PowerShell, in the download folder:

   ```
   (Get-FileHash dl-conn-rs.exe -Algorithm MD5).Hash
   Get-Content dl-conn-rs.exe.md5
   ```

   The two values must match, ignoring case. A `.sha256` file is also provided for a stronger check.

There is no installer. Put the exe anywhere and run it.

The exe is not code signed, so Windows may show "Unknown publisher" or a SmartScreen warning. Choose
**More info**, then **Run anyway**, after you have checked the hash.

## Quick start

1. Run `dl-conn-rs.exe` and accept the admin prompt.
2. Choose a server from the dropdown. Type to search by city, code or region, then press Enter.
3. Start the game. If it's already running, the **Connection Hold** chip under the dropdown says
   whether it needs a restart.

The title bar shows the active pick. Choose **No block** to remove it. Closing the app leaves the
block on, and reopening the app picks up where you left off.

## Switching servers

You can pick a new server at any time, and the block changes at once. The game measures relay
pings only when it starts, though, and matchmaking uses that measurement. So the game only sees
a new pick after it restarts. The **Connection Hold** chip says when that's needed, and when the game
has measured your pick. It reads the game's log, so keep `-condebug` in the game's launch options.

- Only the game needs restarting. Steam can stay open, because the game does its own relay
  pinging.
- A pick made during a match waits for the match to end, so the switch doesn't cut it.
  `clear pending` cancels the wait, and `set pick <server> --now` doesn't wait.
- With ExitLag, choose the same server in ExitLag too.

Don't run another server picker at the same time. Its firewall rules still apply and can block the
server you picked.

## Keys

| Key | Page |
| --- | --- |
| `F1` | Help: launch options, testing in a custom game, reading the results |
| `F2` | Settings |
| `F3` | Log report: reads the game's log and checks it against your pick |
| `` ` `` | Console |
| `Esc` | Back to Status, or cancel a search |

Keys work while the window has focus. They are not global hotkeys.

## Status chips

The chips under the dropdown show a short value. Hover one for what it means.

- **Direct:** a ping from this PC to your pick. The game's real path can differ, especially through
  a routing tool, so this is not your in-game ping.
- **Game:** the last figure the game wrote to its log. It is not live. It needs `-condebug` in the
  game's launch options. It reads **ExitLag** while ExitLag is running.
- **Connection Hold:** `In match` while a pick would wait for the match to end, `Ready` once the
  game has measured your pick, `Restart game` when it measured before the pick, and `No log` when
  the game isn't writing its log. Without the log a pick applies at once, even mid-match.

## Backup relay

A strict pick removes the backup relay the game normally keeps, so the connection depends on one
relay. To keep a failover, name a second site: `set pick sgp --backup maa2`, or set a default
`backup` in F2.

## Settings

Press **F2**, or use `set config <key> <value>` in the console.

| Key | Values | Default | Effect |
| --- | --- | --- | --- |
| `backup` | `none` or a server | `none` | Default failover site for each new pick |
| `ping_every` | `0` to `300` s | `5` | Direct counter interval. `0` stops all ping traffic |
| `refresh_minutes` | `0`, or `5` to `1440` | `5` | Relay list refresh. `0` is manual |
| `hold_in_game` | `on`, `off` | `off` | Delay background filter rewrites while a game runs |
| `report_every` | `1` to `3600` s | `5` | Console report interval |
| `report_on_open` | `on`, `off` | `on` | Start the report when the console opens |
| `clear_on_exit` | `on`, `off` | `off` | Remove the block when the app closes |
| `theme` | `dark`, `light`, `system` | `system` | Dark is Dracula, Light is Catppuccin Latte, System follows Windows |
| `log_path` | `auto` or a path | `auto` | Game log to read. `auto` finds Deadlock's |

## Files and removal

Settings and a capped log are stored in `%LOCALAPPDATA%\dl-conn-rs`. Nothing else is written to
disk, and no firewall rules are saved. To remove the app, delete the exe and that folder. Restart
the PC to clear any block.

## How it works

dl-conn-rs fetches Steam's relay lists, then adds one Windows Filtering Platform (WFP) filter for
each relay site except your pick. Filters are checked once per connection, not per packet, and are
not persistent, so Windows drops them at restart. A failed fetch or bad data never lifts a block.
See [How blocking works](https://github.com/VARSITYLUNCHBUG/dl-conn-rs/wiki/How-Blocking-Works).

## Build from source

Needs the Rust toolchain (stable, MSVC target) and the Visual Studio C++ build tools.

```
git clone https://github.com/VARSITYLUNCHBUG/dl-conn-rs.git
cd dl-conn-rs
cargo build --release
```

The exe is `target\release\dl-conn-rs.exe`. Run the tests with `cargo test`. See
[Building from source](https://github.com/VARSITYLUNCHBUG/dl-conn-rs/wiki/Building-from-Source)
for the release build, checks and live tests.

## Licence

MIT. See [LICENSE](LICENSE). Third-party crate licences ship with each release in
`THIRD_PARTY_NOTICES.txt`.
