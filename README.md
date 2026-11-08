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

1. Close the game.
2. Run `dl-conn-rs.exe` and accept the admin prompt.
3. Choose a server from the dropdown. Type to search by city, code or region, then press Enter.
4. Start the game.

The title bar shows the active pick. Choose **No block** to remove it. Closing the app leaves the
block on, and reopening the app picks up where you left off.

Pick before the game starts. A block can't move a game that is already connected to a relay you
left open.

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

## Counters

- **Direct:** a ping from this PC to your pick. The game's real path can differ, especially through
  a routing tool, so this is not your in-game ping.
- **Game:** the last figure the game wrote to its log, with its time. It is not live. It needs
  `-condebug` in the game's launch options. It reads **ExitLag route** while ExitLag is running.

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
| `theme` | `system`, `light`, `dark` | `system` | Window theme |
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
