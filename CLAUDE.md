# dl-rs

Picks one Steam relay site and blocks the rest until reboot, alongside ExitLag. Rust, Win32, one
admin exe, target under 500 KB. See `README.md` for what it does and `docs/roadmap.md` for what's
next.

## Rules

- **Commands are the only way to act on the app.** Every action is a `verb noun [options]` line
  handled by the session. Front-ends send lines and read typed `Output`. They hold no logic.
- **Crates:** `windows-sys` and `miniserde` only. Ask before adding one.
- **UI text:** plain ASCII. No emoji or em dashes. `scripts/check.ps1` enforces this across `src/`.
- **State:** the active pick lives in WFP, not in memory or on disk. It's recovered from the stamp.
- **Failure:** bad data or a failed fetch never lifts a block or shrinks the relay set.
- **Overhead:** no per-packet work, no polling while idle, and diagnostics only while requested.
- **Git:** local only, no remote. No Claude attribution in commits. Commit only when asked. This
  machine has no git identity, so pass `-c user.name=VARSITYLUNCHBUG -c user.email=100867870+VARSITYLUNCHBUG@users.noreply.github.com`.

## Architecture

```
dropdown --+  command lines
           +--> command.rs parses --> session.rs --> wfp / sdr / diag
console ---+                               |
    ^             log.rs <---- lines ------+
    +------ reads --+                      +-- "changed" --> dropdown queries again
```

| File | Purpose |
| --- | --- |
| `src/command.rs` | The command table, which drives parsing, help and completion. Pure |
| `src/session.rs` | Runs commands, owns relays, pick and refresh loop. Tested with fakes |
| `src/log.rs` | Capped log buffer that wakes the console |
| `src/editor.rs` | Console input line as pure state |
| `src/console.rs` | Real console and its single I/O thread |
| `src/ui.rs` | Dropdown only. `build_view` and `title` are pure |
| `src/diag.rs` | Processes, routes and latency, returned as lines |
| `src/wfp.rs` | Filters and drop events |
| `src/sdr.rs` | Relay lists, and working out what to block |
| `examples/loadsim.rs` | Rate-capped, game-shaped UDP for tests. Not in the app |

Pure logic goes in a module with no Windows calls so it can be unit tested. Windows code stays thin.

## Skills

- `/add-command` adds or changes a command.
- `/add-ui-feature` adds a window or control that wraps commands.
- `/verify-live` picks and runs the right live test, and reads the game's log.

## Commands

The session's PATH may not include cargo. Use `$HOME/.cargo/bin`, or the script below, which handles it.

```
powershell -ExecutionPolicy Bypass -File scripts/check.ps1
powershell -ExecutionPolicy Bypass -File scripts/game-log.ps1
```

`check.ps1` runs format, ASCII, clippy, unit tests and a release build. A running dl-rs locks
`target/release/dl-rs.exe`, so it builds to `target/next` instead, and it doesn't close the app.

## Facts learned the hard way

**Blocking**
- `ALE_AUTH_CONNECT_V4` blocks new and live UDP flows. `ALE_CONNECT_REDIRECT_V4` accepts a block
  filter and then ignores it, measured with a real DNS round trip. Don't use it.
- WFP filters without the persistent flag are the "until reboot" mechanism. `netsh wfp show
  sublayers` isn't a real command; use `netsh wfp show state file=...` and search for `dl-rs`.
- Blocking removes the game's backup relay. Live evidence: `Lost backup #1 maa2#102`, 27 s after
  the block. Strict blocking leaves one relay with no failover, which is why `--backup` exists.

**ExitLag**
- It tunnels boosted traffic through nodes, so no packets reach relays directly. Its `nfextlag`
  driver matched a process by name alone in testing. Blocks still catch that process's traffic to
  blocked relays before the tunnel.
- dl-rs's own ICMP isn't boosted. Live example: direct 356 ms against 136 ms in game. Never
  describe a dl-rs ping as the game's latency. Only the game's log shows the route ping, and only
  when it measures or connects, so a route figure can't be a true live counter.

**Testing**
- Unconnected UDP `send_to` reports success even when blocked. Use drop events, or `pktmon`
  counters at the network card. Relays ignore stray packets, so they can't show a round trip; use
  a DNS resolver for that.
- On this connection about one ICMP round in three fails fast with `IP_REQ_TIMED_OUT`, whatever
  the call pattern. Cause unknown. Don't treat a single timeout as a block; `IP_GENERAL_FAILURE`
  means a local filter refused it.
- `netsh wfp show netevents` returned zero drops for `deadlock.exe` while blocks were clearly
  active, so don't rely on that parser without validating it.
- PowerShell: `Echo` is an alias that shadows a function of that name, and `$null` passed to a
  `[string]` P/Invoke becomes `""`. Use `[NullString]::Value`.

**Valve data**
- `geo` is unreliable: Datapacket sites have latitude and longitude swapped. Classify known sites
  from a table of codes, with coordinates only as a fallback.
- `GetSDRConfig` is per app. CS2 (730) currently carries every relay. Apps otherwise share one of
  three sets. Its `max-age` is up to 300 s, so faster polling sees nothing new.

**Game log**
- `console.log` needs `-condebug`, is rewritten on each launch, and is locked while the game runs,
  so open it with shared read access. `connection_log_27015.txt` covers Steam login servers, not
  relays.
- Without a block, `Ping location` in the log lists sites with direct measurements. With a strict
  block, only the pick has one, and the rest appear as `code=/ms+penalty`.

## Open items

- `--backup` is written and unit tested, but not yet confirmed against the game.
- In a match under a strict Singapore block, the game ran 7 minutes with no backup, then logged
  `Selecting tyo#172 as backup #1` at 18:53:54, 8 s before the player left. Most likely the block
  had just been cleared, which would mean the game picks up a backup mid-connection. Unconfirmed:
  dl-rs keeps no log on disk, so also rule out a leak by repeating it with the block held.
- `scripts/load-sim.ps1` final cleanup stops early in the `load` and `game` scenarios. Cause unknown.
- Untested: real reboot, sleep and resume, offline at startup, filters restoring themselves,
  whether a pick steers matchmaking, a pick different from ExitLag's server.
