# dl-conn-rs

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
- **Docs:** user docs are `README.md` and the wiki pages in `docs/wiki`, held to the same plain ASCII.
  Keep them in step with behaviour. `scripts/publish-wiki.ps1` pushes `docs/wiki` to the GitHub wiki.
- **No contact with the game (VAC):** never open a handle to a game process, read or write its
  memory, inject anything, send it input or console commands, edit its files or launch options, or
  close it. The only contact allowed is the process name list from `CreateToolhelp32Snapshot` and a
  shared read of `console.log`. Everything else happens in the firewall, outside the game.
- **State:** the active pick lives in WFP, not in memory or on disk. It's recovered from the stamp.
  Only settings and a capped log go to disk, in the local app data folder under `dl-conn-rs`.
- **Failure:** bad data or a failed fetch never lifts a block or shrinks the relay set.
- **Overhead:** no per-packet work, and diagnostics only while requested. The one exception is the
  counters: one ping per `ping_every` while there's a pick, and a process check and log size check
  every 5 s. The window draws only on input or a change, so nothing may request a repaint every frame.
- **Git:** `origin` is github.com/VARSITYLUNCHBUG/dl-conn-rs. Commit only when asked.
  `.claude/hooks/guard-commands.js` blocks pushes from Claude sessions, so the user pushes by hand.
  This machine has no git identity, so pass
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
                                                     +--> log.rs --> console page, dl-conn-rs.log
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
| `examples/fullscreen.rs` | Takes the screen in exclusive fullscreen like a game, for tests. Not in the app |

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
fails on any machine path. It builds to `target/dist`, so a running dl-conn-rs elsewhere can't lock it.

`scripts/window-check.ps1` opens the real window with a fake firewall and measures start time and idle
cost on both renderers. It needs no admin rights and can't change a block, so it's safe with a game
running. `scripts/live-test.ps1` uses real filters and needs a UAC prompt. `scripts/fullscreen-repro.ps1`
uses the same fake firewall, takes the screen in exclusive fullscreen for a few seconds, and measures
whether the window's frame moved. It blacks out the screen, so never run it during a match.

## Facts learned the hard way

**Blocking**
- `ALE_AUTH_CONNECT_V4` blocks new and live UDP flows. `ALE_CONNECT_REDIRECT_V4` accepts a block
  filter and then ignores it, measured with a real DNS round trip. Don't use it.
- WFP filters without the persistent flag are the "until reboot" mechanism. `netsh wfp show
  sublayers` isn't a real command; use `netsh wfp show state file=...` and search for `dl-conn-rs`.
- Blocking removes the game's backup relay. Live evidence: `Lost backup #1 maa2#102`, 27 s after
  the block. Strict blocking leaves one relay with no failover, which is why `--backup` exists.

**ExitLag**
- It tunnels boosted traffic through nodes, so no packets reach relays directly. Its `nfextlag`
  driver matched a process by name alone in testing. Blocks still catch that process's traffic to
  blocked relays before the tunnel.
- dl-conn-rs's own ICMP isn't boosted. Example from one PC: direct 356 ms against 136 ms in game. Never
  describe a dl-conn-rs ping as the game's latency. Only the game's log shows the route ping, and only
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
- winit never sends `Occluded` on Windows, and eframe isn't told about display or work-area changes.
  A hidden window in `ui/mod.rs` hears `WM_DISPLAYCHANGE` and `WM_SETTINGCHANGE` and asks for a few
  redraws.
- After a DXGI exclusive fullscreen app changes the display mode and gives it back, Windows shows the
  window's frame at the window's outer corner instead of the client area: up and left by the border
  and title bar (8 and 31 px at 96 DPI), hiding the picker row and leaving a white strip right and
  bottom. Layout, input and new frames are all correct, so a redraw never fixes it. Deadlock does
  this: its `video.txt` had exclusive fullscreen at 1600x900 against a 1920x1080 desktop.
  `scripts/fullscreen-repro.ps1` reproduces it on this PC (GTX 1070 Ti, Windows 10 19045) at
  1600x900, 1280x720 and with only a refresh change (144 to 120 Hz), already on the switch into the
  game's mode. Exclusive fullscreen with no mode change, and a plain `ChangeDisplaySettingsEx` with no
  DirectX, didn't. `WGPU_DX12_PRESENTATION_SYSTEM=Visual` didn't help, though which path wgpu took
  wasn't confirmed. A borderless fullscreen form changes no mode, which is why it never reproduced.
- Only a real move puts the frame back. A same-place `SetWindowPos`, even with `SWP_FRAMECHANGED`,
  doesn't. So on `WM_DISPLAYCHANGE` the listener moves the window 1 px and back at 250 ms, 1 s and
  3 s, without activating it, and logs `display changed to WxH`. Against the last release build the
  repro showed the shift at 1280x720 and 1600x900 and the fixed build didn't, and the fullscreen app
  stayed exclusive through the moves.
- `egui_kittest` renders use egui's default panel colour, not the palette. Check colours in the real
  window.
- Egui takes focus away on Esc before widgets run, unless `set_focus_lock_filter` keeps it.
- A blinking caret and `request_repaint` in a loop both keep the window drawing. Keep `text_cursor.blink`
  off. `an_idle_window_stops_repainting_on_every_page` catches the rest.
- A popup can't extend past its window, which is why there's no compact window.
- While the window is minimized or fully covered, eframe runs no UI pass and drops input, so anything
  that must keep working goes in `App::background`, called from `eframe::App::logic`. Scripts set
  `DL_CONN_RS_DRAW_HIDDEN` so a covered window still takes their input.
- The release build measured 135 MB working set on an NVIDIA driver, against about 5 MB for the old
  Win32 UI, and 7 MB on disk against 590 KB. Much of it is likely the driver. `MemoryHints::MemoryUsage`
  is already set.

**Valve data**
- `geo` is unreliable: Datapacket sites have latitude and longitude swapped. Classify known sites
  from a table of codes, with coordinates only as a fallback.
- `GetSDRConfig` is per app. CS2 (730) currently carries every relay. Apps otherwise share one of
  three sets. Its `max-age` is up to 300 s, so faster polling sees nothing new.

**Game log**
- A game running with `-condebug` went 60 s without a line on 11/15, so a quiet log proves nothing.
  A launch without `-condebug` leaves the last launch's file in place. The monitor counts the log as
  written only once it changes after the game is seen, and as silent after 3 minutes without that.
- `console.log` needs `-condebug`, is rewritten on each launch, and is locked while the game runs,
  so open it with shared read access. `connection_log_27015.txt` covers Steam login servers, not
  relays.
- Without a block, `Ping location` in the log lists sites with direct measurements. With a strict
  block, only the pick has one, and the rest appear as `code=/ms+penalty`.
- `[Networking] Summary of connection` blocks hold each connection's loss, jitter and ping
  percentiles. The first set is this PC's. A second set, "received from remote host", is the far end's.
- A solo hideout is `UDP` to `127.0.0.1` or `pipe`. A hideout lobby uses relays like a match: its
  ticket line ends `hideout_lobby=`, and a match's ends `match_id=`, then an `SDR` connection
  follows. A private lobby ready-up was followed by an `SDR` connection in the one log checked, so
  custom games appear to use relays.
- The game measures relay pings once, at startup. On 11/13 it measured once in a 3 h session. On
  11/14, after a switch from sgp to hkg, it didn't measure again for 11 minutes, through a queue
  and a hideout reload. So a switch reaches matchmaking only after the game restarts.
- Deadlock ships its own `game\bin\win64\steamnetworkingsockets.dll`, so relay pinging happens in
  the game, not in Steam. Restarting Steam never helps.
- Restarting only the game is enough, and ExitLag can stay connected. On 11/15 sgp was picked with
  the game closed. Its startup measurement then had `sgp=121+12` as the only direct entry, boosted as
  before (119 ms the night before), and a real match ran 40 minutes on `sgp#98` at 122 ms.
- Private lobby placement ignores the ping table. On 11/15 under a strict hkg block, with only hkg
  measured, a custom game was hosted in `jnb` and failed twice with `Problems with 6 relays (1 OK)`,
  `LOCALPROBLEM_MANYRELAYS`. The next lobby went to `sgp` and connected through `hkg#345`. So a
  strict block can make a custom game unreachable. hkg measured 360 ms that session, most likely
  because ExitLag was still set to Singapore.

## Open items

- `hold_in_game` is unit tested only. It needs a relay list change while a game runs.
- A mid-match pick waits for the match to end, but no pick has been made in a real match yet.
  Replaying the 11/15 match log through `LogTail` gave `InMatch` at the connect and `LastMatch` at
  the `closed` line, so detection works. Confirm that the match isn't cut and the pick applies then.
- Untested: what a stale ping table costs. Nobody has accepted a match after a switch without a
  restart.
- Under an hkg block on 11/14, a P2P connect to `185.25.182.20:3478` failed with `WSAENETUNREACH`
  and timed out. Check whether blocks break party hideout lobbies.
- The route counter hasn't been seen against a live boosted match yet.
- `No log` on Connection Hold is unit tested only. Check it with a launch without `-condebug`.
- The nudge after a display change fixes the shifted frame in `scripts/fullscreen-repro.ps1`. Confirm
  it with the game, and check a maximized or snapped window, which haven't been tried.
- `--backup` is written and unit tested, but not yet confirmed against the game.
- In a match under a strict Singapore block, the game ran 7 minutes with no backup, then logged
  `Selecting tyo#172 as backup #1` at 18:53:54, 8 s before the player left. Most likely the block
  had just been cleared, which would mean the game picks up a backup mid-connection. Unconfirmed:
  also rule out a leak by repeating it with the block held. `dl-conn-rs.log` now records when picks change.
- `scripts/load-sim.ps1` final cleanup stops early in the `load` and `game` scenarios. Cause unknown.
  The script was rewritten for egui and hasn't been run since.
- Untested: Windows 11, older Windows 10, a no-GPU machine, real reboot, sleep and resume, offline at startup, filters restoring themselves,
  whether a pick steers matchmaking, a pick different from ExitLag's server.
