# dl-rs

Pick one Steam relay region. Everything else is blocked until you pick again or restart the PC.
Built to run alongside ExitLag. Windows 10 or later, x64. One 365 KB exe with no installer, no
config files and no runtime dependencies. It asks for admin rights when it starts.

## Use

Choose a server from the dropdown. The title bar shows what's active. Choose **No block** to
undo it.

Closing the app leaves the block in place. Reopening it picks up where you left off.

Don't run another server picker at the same time. Its firewall rules still apply, so it can
block the server you picked here.

Press **\`** (backtick) in the window to toggle the hidden console. It shows:

- **processes:** which game, Steam and ExitLag processes are running
- **picking:** the current pick, its relays by latency, and the route to each
- **blocked:** packets stopped, by app and relay site, such as `blocked deadlock.exe -> waw`
- fetches, filter changes and errors

## How it works

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

`cargo test` runs the unit tests. The live tests need admin rights and use real filters. They
briefly block real relays and one public DNS resolver:

```
powershell -ExecutionPolicy Bypass -File scripts\live-test.ps1 -TestExe <target\debug\deps\dl_rs-*.exe> -AppExe <target\release\dl-rs.exe> -OutDir <dir>
```

## Layout

| File | Purpose |
| --- | --- |
| `src/sdr.rs` | Fetch and parse relay lists, and work out what to block |
| `src/wfp.rs` | WFP filters and drop events |
| `src/worker.rs` | Refresh loop and applying the pick |
| `src/diag.rs` | Console reports |
| `src/ui.rs` | Window, dropdown and console |
