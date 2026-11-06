# dl-rs

Picks one Steam relay site and blocks the rest until reboot, with or without ExitLag. Rust, egui, one
admin exe for Windows 10 and 11, and Server 2016 and later, x64. See `README.md` for what it does,
`docs/roadmap.md` for what's next and `docs/release-checklist.md` before shipping.

## Rules

- **Commands are the only way to act on the app.** Every action is a `verb noun [options]` line
  handled by the session. Front-ends send lines and read typed `Output`. They hold no logic.
- **Crates:** `windows-sys`, `miniserde`, `eframe` and `wgpu`, plus `embed-resource` to build and
  `egui_kittest` and `winit` to test. Ask before adding one.
- **Nothing about this PC in the app:** no fixed paths, and no assumption that ExitLag, Steam or a GPU
  exists. Find folders through the registry and known folders. `scripts/release.ps1` fails if the exe
  holds a machine path. Numbers measured here don't belong in user text.
- **UI text:** plain ASCII. No emoji or em dashes. `scripts/check.ps1` enforces this across `src/`.
- **State:** the active pick lives in WFP, not in memory or on disk. It's recovered from the stamp.
  Only settings and a capped log go to disk, in the local app data folder under `dl-rs`.
- **Failure:** bad data or a failed fetch never lifts a block or shrinks the relay set.
- **Overhead:** no per-packet work, and diagnostics only while requested. The one exception is the
  counters: one ping per `ping_every` while there's a pick, and a process check and log size check
  every 5 s. The window draws only on input or a change, so nothing may request a repaint every frame.
- **Git:** local only, no remote. Commit only when asked. This machine has no git identity, so pass
  `-c user.name=VARSITYLUNCHBUG -c user.email=100867870+VARSITYLUNCHBUG@users.noreply.github.com`.
- **No Claude signature in any commit**, in the message or the identity. Never use `--no-verify` to
  get past this. The `.githooks/commit-msg` hook enforces it, and `scripts/check.ps1` scans all
  history. A fresh clone needs `git config core.hooksPath .githooks` once. Patterns live in
  `.githooks/attribution-patterns.txt`. Naming a file such as `CLAUDE.md` in a message is fine.

## Architecture

```
window --- command lines --> command.rs parses --> session.rs --> wfp / sdr / diag / analysis
  ^                                                  |   ^
  +------- "changed", then it queries again ---------+   +-- counters <-- monitor.rs thread
                                                     +--> log.rs --> console page, dl-rs.log
```

| File | Purpose |
| --- | --- |
| `src/command.rs` | The command table, which drives parsing, help and completion. Pure |
| `src/session.rs` | Runs commands, owns relays, pick and refresh loop. Tested with fakes |
| `src/log.rs` | Capped log buffer and capped file |
| `src/editor.rs` | The console prompt's history and completion, as pure state |
| `src/ui/mod.rs` | Starts the window, renderer choice, single instance, startup errors |
| `src/ui/app.rs` | The window: header, pages, hotkeys, and results from the session |
| `src/ui/picker.rs` | The searchable dropdown |
| `src/ui/pages.rs` | Status, Help, Settings, Report and Console |
| `src/ui/rows.rs` | What the dropdown lists. Pure |
| `src/ui/theme.rs` | Light and dark colours. Every colour comes from a `Palette` |
| `src/ui/tests.rs` | Drives the UI in-process with `egui_kittest`, a fake firewall and no network |
| `src/settings.rs` | Setting definitions and validation, plus load and save |
| `src/analysis.rs` | Log parser, findings and report. Pure |
| `src/monitor.rs` | The counters' thread. Its hold logic and labels are pure |
| `src/regions.rs` | Site code to region table. Pure |
| `src/guide.rs` | The F1 text |
| `src/diag.rs` | Processes, routes, latency, data folder and finding the game's log |
| `src/wfp.rs` | Filters and drop events |
| `src/sdr.rs` | Relay lists, and working out what to block |
| `scripts/drive.ps1` | How scripts drive the window without taking focus |
| `examples/loadsim.rs` | Rate-capped, game-shaped UDP for tests. Not in the app |

Pure logic goes in a module with no Windows calls so it can be unit tested. Windows code stays thin.

## Skills

- `/add-command` adds or changes a command.
- `/add-ui-feature` adds a page or control that wraps commands.
- `/verify-live` picks and runs the right live test, and reads the game's log.

## Commands

The session's PATH may not include cargo. Use `$HOME/.cargo/bin`, or the script below, which handles it.

```
powershell -ExecutionPolicy Bypass -File scripts/check.ps1
powershell -ExecutionPolicy Bypass -File scripts/game-log.ps1
```

`check.ps1` runs format, ASCII, the commit rule, clippy, unit and UI tests, and a release build that
fails on any machine path. It builds to `target/dist`, so a running dl-rs elsewhere can't lock it.

`scripts/window-check.ps1` opens the real window with a fake firewall and measures start time and idle
cost on both renderers. It needs no admin rights and can't change a block, so it's safe with a game
running. `scripts/live-test.ps1` uses real filters and needs a UAC prompt.

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
- dl-rs's own ICMP isn't boosted. Example from one PC: direct 356 ms against 136 ms in game. Never
  describe a dl-rs ping as the game's latency. Only the game's log shows the route ping, and only
  when it measures or connects, so a route figure can't be a true live counter.

**Testing**
- Unconnected UDP `send_to` reports success even when blocked. Use drop events, or `pktmon`
  counters at the network card. Relays ignore stray packets, so they can't show a round trip; use
  a DNS resolver for that.
- On the development connection about one ICMP round in three fails fast with `IP_REQ_TIMED_OUT`, whatever
  the call pattern. Cause unknown. Don't treat a single timeout as a block; `IP_GENERAL_FAILURE`
  means a local filter refused it.
- `netsh wfp show netevents` returned zero drops for `deadlock.exe` while blocks were clearly
  active, so don't rely on that parser without validating it.
- A test exe has no manifest, so it gets no DPI awareness. `window-check.ps1` puts a copy of
  `app.manifest` beside it.
- `kittest` fails a test if a control has no accessible name, so give every control a label.
- A window that isn't in front still handles posted key-downs, which is how scripts avoid taking focus
  in a match. Posted characters don't work: a character after a key-down can type twice, and one alone
  is ignored. Set text through UI Automation's `ValuePattern` instead.
- PowerShell: `Echo` is an alias that shadows a function of that name, and `$null` passed to a
  `[string]` P/Invoke becomes `""`. Use `[NullString]::Value`.

**egui**
- Egui takes focus away on Esc before widgets run, unless `set_focus_lock_filter` keeps it.
- A blinking caret and `request_repaint` in a loop both keep the window drawing. Keep `text_cursor.blink`
  off. `an_idle_window_stops_repainting_on_every_page` catches the rest.
- A popup can't extend past its window, which is why there's no compact window.
- While the window is minimized or fully covered, eframe runs no UI pass and drops input, so anything
  that must keep working goes in `App::background`, called from `eframe::App::logic`. Scripts set
  `DL_RS_DRAW_HIDDEN` so a covered window still takes their input.
- The release build measured 135 MB working set on an NVIDIA driver, against about 5 MB for the old
  Win32 UI, and 7 MB on disk against 590 KB. Much of it is likely the driver. `MemoryHints::MemoryUsage`
  is already set.

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
- `[Networking] Summary of connection` blocks hold each connection's loss, jitter and ping
  percentiles. The first set is this PC's. A second set, "received from remote host", is the far end's.
- Hideout connections are `UDP` to `127.0.0.1` or `pipe`. A private lobby ready-up was followed by an
  `SDR` connection in the one log checked, so custom games appear to use relays.

## Open items

- `hold_in_game` is unit tested only. It needs a relay list change while a game runs.
- The route counter hasn't been seen against a live boosted match yet.
- `--backup` is written and unit tested, but not yet confirmed against the game.
- In a match under a strict Singapore block, the game ran 7 minutes with no backup, then logged
  `Selecting tyo#172 as backup #1` at 18:53:54, 8 s before the player left. Most likely the block
  had just been cleared, which would mean the game picks up a backup mid-connection. Unconfirmed:
  also rule out a leak by repeating it with the block held. `dl-rs.log` now records when picks change.
- `scripts/load-sim.ps1` final cleanup stops early in the `load` and `game` scenarios. Cause unknown.
  The script was rewritten for egui and hasn't been run since.
- The repo has no licence file. Distributing it should have one.
- Untested: Windows 11, older Windows 10, a no-GPU machine, real reboot, sleep and resume, offline at startup, filters restoring themselves,
  whether a pick steers matchmaking, a pick different from ExitLag's server.
