# Building from source

## Requirements

- Windows 10 or 11, x64.
- [Rust](https://rustup.rs) stable, with the `x86_64-pc-windows-msvc` target, which is the default
  on Windows.
- Visual Studio Build Tools with the "Desktop development with C++" workload, for the MSVC linker
  and resource compiler.

## Build and run

```
git clone https://github.com/VARSITYLUNCHBUG/dl-conn-rs.git
cd dl-conn-rs
cargo build --release
```

The exe is `target\release\dl-conn-rs.exe`. It's statically linked, so it runs on its own.

## Test

```
cargo test
```

This runs the unit tests and drives the real UI code in-process with a fake firewall and no network.
It needs no admin rights and changes no filters.

## All checks

```
powershell -ExecutionPolicy Bypass -File scripts\check.ps1
```

Runs formatting, the plain ASCII rule for UI text, the commit message rule, clippy, all tests and a
release build that fails if the exe contains a path from your PC.

## Release build

```
powershell -ExecutionPolicy Bypass -File scripts\release.ps1
```

Writes to `dist`:

- `dl-conn-rs.exe`, with build paths removed and Control Flow Guard on.
- `dl-conn-rs.exe.md5` and `dl-conn-rs.exe.sha256`.
- `LICENSE.txt` and `THIRD_PARTY_NOTICES.txt`.
- `dl-conn-rs-<version>-source.zip`, made from the committed tree, so commit first.

To sign the exe, set `DL_CONN_RS_SIGN_THUMBPRINT` to the thumbprint of a code signing certificate
in your Windows certificate store. Never put a key or password in the repo.

`docs/release-checklist.md` lists what a person has to check before publishing.

## Window and live tests

`scripts/window-check.ps1` opens the real window with a fake firewall and measures start time and
idle cost on both renderers. It needs no admin rights and can't change a block.

`scripts/live-test.ps1` needs admin rights, uses real filters, briefly blocks real relays and one
public DNS resolver, and moves the mouse near the end. Close the game first.

`scripts/load-sim.ps1` sends rate-capped, game-shaped UDP from `examples/loadsim.rs` and counts what
reaches the network card with `pktmon`. See the top of each script for its arguments. The test exe
they take is the `dl_conn_rs-*.exe` printed by `cargo test --no-run`.

## Design

Commands are the only way to act on the app. The window and its console are thin front-ends that
send `verb noun [options]` lines to one session, which runs them one at a time.

```
window --- command lines --> command.rs parses --> session.rs --> wfp / sdr / diag / analysis
  ^                                                    |
  +----------- "changed", then it queries again -------+--> log.rs --> console page, log file
```

| Path | Purpose |
| --- | --- |
| `src/command.rs` | The command table, which drives parsing, help and completion |
| `src/session.rs` | Runs commands, and owns the relays, pick and refresh loop |
| `src/wfp.rs` | Windows Filtering Platform filters and drop events |
| `src/sdr.rs` | Fetches and parses relay lists, and works out what to block |
| `src/diag.rs` | Processes, routes, latency and finding the game's log |
| `src/analysis.rs` | The game log parser and report |
| `src/settings.rs` | Setting definitions, validation and the settings file |
| `src/monitor.rs` | The ping counters' thread |
| `src/regions.rs` | Which region each relay site belongs to |
| `src/log.rs` | The log buffer and its capped file |
| `src/editor.rs` | The console prompt's history and completion |
| `src/guide.rs` | The F1 help text |
| `src/ui/` | The egui window: dropdown, counters, pages and themes |

Pure logic lives in modules with no Windows calls, so it can be unit tested. Windows code stays
thin.

To add a command, add a `Spec` to the table in `command.rs` and handle its `Command` in
`session.rs`. Help and completion follow from the table.

## Contributing

Run `git config core.hooksPath .githooks` once after cloning, so the commit message hook runs.
`CLAUDE.md` holds the design rules and lessons learned from testing, and `docs/roadmap.md` lists
planned work. Allowed crates are `windows-sys`, `miniserde`, `eframe` and `wgpu`. UI text must be
plain ASCII.
