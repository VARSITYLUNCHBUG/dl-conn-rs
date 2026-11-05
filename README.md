# dl-rs

Pick one Steam relay region. Everything else is blocked until you pick again or restart the PC.
Built to run alongside ExitLag. Windows 10 or later, x64. One exe of about 590 KB with no installer
and no runtime dependencies. It asks for admin rights when it starts. Settings you change are saved
in `%LOCALAPPDATA%\dl-rs\settings.ini`. Nothing else goes to disk.

## Use

Choose a server from the dropdown. Sites are grouped by region, and typing narrows the list by
name, code or region. Enter picks the highlighted site, and Esc cancels the search. The title bar
shows what's active. Choose **No block** to undo it.

Closing the app leaves the block in place. Reopening it picks up where you left off.

Don't run another server picker at the same time. Its firewall rules still apply, so it can
block the server you picked here.

| Key | Window |
| --- | --- |
| `F1` | Help: how to set launch options, test in a custom game, and read the results |
| `F2` | Settings |
| `F3` | Log report: reads the game's log and checks it against your pick |
| \` | Console |

These work while dl-rs has focus. They aren't global hotkeys.

## Ping counters

Two lines under the dropdown:

- **Direct:** a ping from this PC to your pick. It doesn't go through ExitLag, so it can differ a lot
  from the game's real path. The last value holds for two missed pings, because single misses are
  common.
- **ExitLag route:** the figure the game wrote to its log, with its time. It's the last known value,
  not a live reading, because the game only logs it when it measures or connects. It needs
  `-condebug` in the game's launch options. It shows only while ExitLag is running, but a running
  ExitLag doesn't prove the game was boosted at that moment.

## Settings

Press **F2**, or use `show config` and `set config <key> <value>`. No setting lowers ping beyond
choosing a route.

| Key | Values | Default | Effect |
| --- | --- | --- | --- |
| `backup` | `none` or a server | `none` | Default failover site for each new pick. `--backup none` skips it for one pick |
| `ping_every` | `0` to `300` s | `5` | Direct counter interval. `0` stops all ping traffic |
| `refresh_minutes` | `0`, or `5` to `1440` | `5` | Relay list refresh. `0` is manual, which also stops automatic filter repair |
| `hold_in_game` | `on`, `off` | `off` | Delays background filter rewrites while a game runs. New relays stay unblocked until it closes |
| `report_every` | `1` to `3600` s | `5` | Console report interval |
| `report_on_open` | `on`, `off` | `on` | Start the report when the console opens |
| `clear_on_exit` | `on`, `off` | `off` | Remove the block when dl-rs closes |
| `log_path` | `auto` or a path | `auto` | The log to read. `auto` finds Deadlock's. Other games are untested |

A damaged settings file falls back to defaults and never changes a block.

## Log report

Press **F3**, or run `analyze log`. It reads `console.log` with shared access, so it's safe while the
game runs, and the game needs `-condebug` to write one. The report covers the file, relay config, ping
measurements, the ping table and each relay connection with its loss and ping figures. Findings at the
top are marked `OK`, `CHECK` or `NOTE`. Steam IDs and player names are left out.

## Console

Press **\`** (backtick) to show or hide the console. It's mainly a live log, with a prompt at the
bottom for commands. While it's open, a report repeats every 5 seconds by default, showing processes, the pick,
relay latency and routes, and any traffic stopped, such as `blocked deadlock.exe -> waw`. Use
`stop report` for quiet.

Everything is a `verb noun [options]` command, and dropdown choices run the same ones, so they show
up in the log.

| Command | Effect |
| --- | --- |
| `help [verb [noun]]` | List commands, or explain one |
| `list servers [--filter <text>]` | List relay sites you can pick |
| `list relays [<server>]` | List relay addresses, for all sites or one |
| `show status` | Show the pick, what's blocked and connection state |
| `set pick <server> [--backup <server>]` | Block every relay except this server's |
| `clear pick` | Remove the block so all servers are open |
| `refresh relays` | Fetch relay lists from Steam now |
| `show routes [<server>]` | Show the interface and gateway to each relay |
| `ping relays [<server>]` | Measure latency to each relay |
| `show processes` | Show running game, Steam and ExitLag processes |
| `show report` | Print processes, pick, relays and blocked traffic once |
| `start report [--every <secs>]` | Repeat the report and collect blocked traffic |
| `stop report` | Stop repeating the report |
| `toggle console` | Show or hide the console, and its report with it |
| `clear screen` | Clear the console |
| `show config` | Show every setting and what it does |
| `set config <key> <value>` | Change a setting and save it |
| `clear config [<key>]` | Put one setting, or all of them, back to the default |
| `analyze log` | Read the game's log and report on its relay connections |
| `show guide` | Show how to test a connection, step by step |
| `quit` | Close the app. Blocks stay on until reboot or the next pick |

| Key | Effect |
| --- | --- |
| `Tab` | Complete a word. A second `Tab` lists the options |
| `?` | Explain what you've typed so far |
| `Up` / `Down` | Command history |
| `Esc` / `Ctrl+C` | Clear the line |
| \` | Hide the console |

Servers complete by code or by name, so `set pick frank<Tab>` offers the Frankfurt sites.

A strict pick also removes the backup relay the game normally keeps, so the connection depends on
one relay. To keep a failover route, name a second site: `set pick sgp --backup maa2`. Both stay
open and everything else is blocked. A later pick without `--backup`, including any dropdown choice,
goes back to the `backup` setting, which is strict blocking by default.

The console's X button is removed because Windows ends the whole app when a console closes. Use
\` to hide it. Show and hide assume a classic console window; with Windows Terminal set as the
default on Windows 11, hiding may not work.

## Design

Commands are the only way to act on the app. The windows and console are thin front-ends over one
session that runs commands one at a time.

```
dropdown ─┐  command lines
          ├───────────────> command.rs parses ──> session.rs ──> wfp / sdr / diag
console ──┘                                            │
    ^                 log.rs <──────── lines ──────────┤
    └───────── reads ──┘                               └─ "changed" ─> dropdown queries again
```

| File | Purpose |
| --- | --- |
| `src/command.rs` | The command table, which drives parsing, help and completion |
| `src/session.rs` | Runs commands, and owns the relays, pick and refresh loop |
| `src/log.rs` | The log buffer |
| `src/editor.rs` | The console's input line, as pure state |
| `src/console.rs` | The real console and its I/O thread |
| `src/ui/` | The main window, dropdown, and the F1, F2 and F3 windows |
| `src/settings.rs` | What each setting means, validation and the settings file |
| `src/analysis.rs` | The log parser and report |
| `src/monitor.rs` | The ping counters' thread |
| `src/regions.rs` | Which region each relay site belongs to |
| `src/guide.rs` | The help text |
| `src/diag.rs` | Processes, routes, latency and finding the game's log |
| `src/wfp.rs` | Windows Filtering Platform filters and drop events |
| `src/sdr.rs` | Fetch and parse relay lists, and work out what to block |

To add a command, add a `Spec` to the table in `command.rs` and handle its `Command` in
`session.rs`. Help and completion follow from the table.

## How blocking works

- **Relay list:** fetched from Steam's `GetSDRConfig` for CS2, Dota 2 and Deadlock, which covered
  every distinct relay set found when sampling popular games. The three lists are merged.
- **Refresh:** every 5 minutes by default, matching the API's cache lifetime, plus once after a new
  pick. Failures back off, and a failed fetch never drops a relay or lifts a block.
- **Blocking:** one Windows Filtering Platform (WFP) filter per relay site, for every other site,
  at `ALE_AUTH_CONNECT_V4`. It applies to all apps and protocols. It's checked once per flow, so
  it adds no per-packet cost, and live flows are re-checked when filters change.
- **Atomic and quiet:** each change is one transaction, so there's never a half-applied state.
  Filters are only rewritten when the result would differ, because every write makes Windows
  re-check all live flows.
- **Until reboot:** filters are WFP's default "static" objects, which last until deleted or until
  the filtering service stops. Nothing goes to the firewall rule store or to disk, and the active
  pick is read back from WFP itself.

## ExitLag

ExitLag's traffic to its own servers isn't touched, because only Valve relay addresses are matched.

Blocks work on a game boosted through ExitLag. In a live Deadlock session routed to Singapore, the
game's traffic reached the network only as tunnels to ExitLag's nodes, yet its packets to other
relays were still dropped before entering the tunnel. A simulator under the game's process name was
tunnelled too, and every relay it probed showed at least as many drops as probes sent. Those counts
include some of the game's own drops, since both share a process name.

To check your own setup, boost the game, open the console and look for
`blocked <game>.exe -> ...` lines. Latency in the console and the Direct counter comes from unboosted
pings, so it can differ a lot from the game's real path.

Match your pick to the server ExitLag routes to. A block can't move a game that's already on a
relay you've left open, and ExitLag's route choice is separate from dl-rs.

## Development

`CLAUDE.md` has the design rules and lessons learned, and `docs/roadmap.md` has planned features.
`scripts/check.ps1` runs every check that doesn't need admin rights.

## Build and test

```
cargo build --release
cargo test
```

`cargo test` runs the unit tests. The live tests need admin rights and use real filters and real
keystrokes. They briefly block real relays and one public DNS resolver, and open the app's windows:

```
powershell -ExecutionPolicy Bypass -File scripts\live-test.ps1 -TestExe <target\debug\deps\dl_rs-*.exe> -AppExe <target\release\dl-rs.exe> -OutDir <dir>
```

`scripts/ui-test.ps1` drives the real windows with a fake firewall. It needs no admin rights and can't
change a block, so it works with a game running:

```
powershell -ExecutionPolicy Bypass -File scripts\ui-test.ps1 -TestExe <target\debug\deps\dl_rs-*.exe> -OutDir <dir>
```

`scripts/load-sim.ps1` sends game-shaped UDP from `examples/loadsim.rs` and counts what reaches the
network card with `pktmon`. Its rates are capped because it reaches real relays. Its `game` scenario
tests a real game boosted through ExitLag, and only ever picks the server you name. See the top of
the script for how to run it.
