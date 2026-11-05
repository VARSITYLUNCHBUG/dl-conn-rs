# Roadmap

Requested and not yet built. Each item follows the rules in `CLAUDE.md`: new behaviour is a
command, and windows only send command lines and display the result. Build them in this order,
since later items depend on earlier ones.

## 1. Settings and config commands

Needed by everything below.

- Commands: `show config`, `set config <key> <value>`.
- `src/settings.rs`: pure parsing and validation, plus load and save for
  `%LOCALAPPDATA%\dl-rs\settings.ini`. This ends the app's "no config files" property, so update
  the README. The active pick still lives only in WFP.
- Only add settings that change real behaviour:

| Key | Values | Effect |
| --- | --- | --- |
| `backup` | `none` or a server | Default backup for every pick. `--backup none` overrides it |
| `ping_every` | `0` to `300` s, default `5` | Counter interval. `0` stops all ping traffic |
| `refresh_minutes` | `0` or `5` to `1440`, default `5` | `0` is manual, which also stops automatic filter repair |
| `report_every` | `1` to `3600` s, default `5` | Console report interval |
| `report_on_open` | `on`, `off`, default `on` | Start the report when the console opens |
| `clear_on_exit` | `on`, `off`, default `off` | Remove the block when dl-rs closes |
| `log_path` | `auto` or a path | Override the game log location |

Be honest with users that no setting lowers ping beyond choosing a route. Don't add placebo tweaks.

## 2. Log analysis (F3)

- Command: `analyze log`. Returns report lines.
- `src/analysis.rs`: pure parser, with tests built from real log lines. Read with shared access.
- Find the log through the `SteamPath` registry value and `libraryfolders.vdf`.
- Report: file and time span, relay config revision and fetch failures, each ping measurement,
  the ping table, each online connection (primary, backups, lost relays, match id, end reason), and
  findings marked OK, CHECK or NOTE.
- Findings should compare against the pick: sites valid against sites open, primary against pick,
  and game ping against direct ping while ExitLag is running.
- Filter to `[SteamNetSockets]` and `[NetSteamConn]`. Loopback and pipe connections are the local
  hideout server, not a match. Only `SDR` connections count.

Real line formats:

```
Ping measurement completed in 12.1s.  Relays: 1 valid, 0 great, 0 good+, 1 ok+, 0 ignored
Ping location: sgp=120+12/121+12,maa2=/152+12,hkg=/153+12
[#154429941 SDR server steamid:<id> vport 0] Selecting sgp#276 (103.10.124.121:27060) as primary.  (Ping = 121 = 120+1+0 (front+interior+remote).  Rank=1 Score=133)
[#1819906075 SDR server steamid:<id> vport 0] Selecting maa2#102 (155.133.225.19:27052) as backup #1 (Ping = 186 = 153+33+0 (front+interior+remote).  Rank=2 Score=201)
[#1819906075 SDR server steamid:<id> vport 0 'server'] Lost backup #1 maa2#102 (155.133.225.19:27052).  consecutive_ping_timeout
SDR network config fetch attempt #1 failed.  HTTP 504.  .  Trying again.
Got SDR network config.  Loaded revision 1793736599 OK
```

## 3. Ping counters under the dropdown

- **Direct:** ICMP to the pick's relays from a dedicated thread, never the session thread. Ping the
  best known relay and sweep only on a miss. Hold the last value for two misses because of the
  fast-failure pattern noted in `CLAUDE.md`.
- **ExitLag route:** the game's own figure from the log, read incrementally by file offset, and
  reset when the file shrinks. Label it with its source and time, for example
  `ExitLag route: 121 ms, in match at 18:46`. It is not live; say so. Show a clear state when
  ExitLag isn't running or there's no recent game data.
- State lives in the session and reaches the UI through `show status`.

## 4. Searchable dropdown with region sections

- `src/regions.rs`: pure, with a table of site codes. See the note on `geo` in `CLAUDE.md`.
- `CBS_DROPDOWN | CBS_OWNERDRAWFIXED | CBS_HASSTRINGS`. Headers are drawn but can't be picked.
- Typing filters on name, code or region. Enter picks the first match. `CB_RESETCONTENT` clears the
  edit text, so restore text and caret and guard against re-entry. `CBN_CLOSEUP` can arrive before
  `CBN_SELCHANGE`, so post the list restore instead of doing it inline.
- Keep `CB_SELECTSTRING` working so the live tests can still drive it.

## 5. Help and settings windows (F1, F2)

- **F1 help:** command `show guide`. Plain text covering setting launch options (`-condebug`,
  `-lognetapi`, and removing them afterwards, since the log grew about 434 KB in 5 minutes), testing
  first in a custom game, and how to read the results. Only state what's been verified.
- **F2 settings:** a form that reads `show config` and runs `set config`, with one line under each
  field saying what it really does.
- **F3 report:** shares the text window with F1, plus a re-run button.
- Text windows: resizable, monospace, read-only. Fetch from the session on a short-lived thread and
  post the result back, because the UI thread never waits on the session.

## Likely additions

- Label console latency as direct pings.
- Fix the `scripts/load-sim.ps1` cleanup bug and collect the filter latency comparison.
