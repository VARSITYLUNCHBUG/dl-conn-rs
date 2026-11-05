# dl-rs

Pick one Steam relay region. Everything else is blocked until you pick again or restart the PC.
Built to run alongside ExitLag. Windows 10 or later, x64. One 430 KB exe with no installer, no
config files and no runtime dependencies. It asks for admin rights when it starts.

## Use

Choose a server from the dropdown. The title bar shows what's active. Choose **No block** to
undo it.

Closing the app leaves the block in place. Reopening it picks up where you left off.

Don't run another server picker at the same time. Its firewall rules still apply, so it can
block the server you picked here.

## Console

Press **\`** (backtick) to show or hide the console. It's mainly a live log, with a prompt at the
bottom for commands. While it's open, a report repeats every 5 seconds showing processes, the pick,
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
| `set pick <server>` | Block every relay except this server's |
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
| `quit` | Close the app. Blocks stay on until reboot or the next pick |

| Key | Effect |
| --- | --- |
| `Tab` | Complete a word. A second `Tab` lists the options |
| `?` | Explain what you've typed so far |
| `Up` / `Down` | Command history |
| `Esc` / `Ctrl+C` | Clear the line |
| \` | Hide the console |

Servers complete by code or by name, so `set pick frank<Tab>` offers the Frankfurt sites.

The console's X button is removed because Windows ends the whole app when a console closes. Use
\` to hide it. Show and hide assume a classic console window; with Windows Terminal set as the
default on Windows 11, hiding may not work.

## Design

Commands are the only way to act on the app. The dropdown and console are thin front-ends over one
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
| `src/ui.rs` | The dropdown |
| `src/diag.rs` | Processes, routes and latency |
| `src/wfp.rs` | Windows Filtering Platform filters and drop events |
| `src/sdr.rs` | Fetch and parse relay lists, and work out what to block |

To add a command, add a `Spec` to the table in `command.rs` and handle its `Command` in
`session.rs`. Help and completion follow from the table.

## How blocking works

- **Relay list:** fetched from Steam's `GetSDRConfig` for CS2, Dota 2 and Deadlock, which covered
  every distinct relay set found when sampling popular games. The three lists are merged.
- **Refresh:** every 5 minutes, matching the API's cache lifetime, plus once after a new pick.
  Failures back off, and a failed fetch never drops a relay or lifts a block.
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

Its `nfextlag` driver also hooks UDP at `ALE_CONNECT_REDIRECT`, which runs before the point where
dl-rs blocks. If it rewrites a boosted game's destination there, the block wouldn't see the relay
address. A block filter at that earlier layer isn't an option: Windows accepts it, but in testing
it stopped nothing, for new or live flows.

To check, boost the game in ExitLag, open the console and queue. `blocked <game>.exe -> ...` lines
mean the block is catching its traffic.

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
