---
name: verify-live
description: Verify dl-conn-rs against real filters, real relays, the real console or a live game. Use after changes to blocking, routing or the UI, or when asked whether blocks and ExitLag routing are working.
---

# Verify dl-conn-rs live

Pick the lightest check that answers the question. Check first whether Deadlock is running, because
that decides what's safe:

```
tasklist | grep -i -E "deadlock|dl-conn-rs|exitlag"
```

| Question | Check | Admin? | Safe in a match? |
| --- | --- | --- | --- |
| Which relay is the game on? What's the route ping? | `scripts/game-log.ps1` | No | Yes |
| Do logic and formatting hold? | `scripts/check.ps1` | No | Yes |
| Do the dropdown, keys and pages behave? | `cargo test` (`src/ui/tests.rs`) | No | Yes |
| Does the real window open, draw and idle properly? | `scripts/window-check.ps1` (fake firewall) | No | Yes |
| Does the block hold in the current match? | `load-sim.ps1 -Scenario probe -Pick <server>` | Yes | Only if `<server>` is the match's server |
| Does a boosted game's traffic get caught? | `load-sim.ps1 -Scenario game -ExeName deadlock.exe` | Yes | Disruptive, ask first |
| Are filters, console and dropdown all correct? | `scripts/live-test.ps1` (moves the mouse) | Yes | No, it switches picks |
| Is block accounting exact under load? | `scripts/load-sim.ps1` | Yes | No, it switches picks |

The guard hook refuses scripts that switch picks while Deadlock is running.

## Running an elevated script

Say first that a UAC prompt is coming, and what the user may see. Each run costs them a prompt, so
fix script bugs before asking again. Run it in the background and read its `results.txt`:

```
Start-Process powershell.exe -Verb RunAs -PassThru -WindowStyle Minimized -ArgumentList @('-NoProfile','-ExecutionPolicy','Bypass','-File','"<script>"', ...)
```

Pass absolute paths. Get the test exe from `cargo test --no-run` and use the `dl_conn_rs-*.exe` path it
prints. If UAC is declined, stop and ask rather than retrying.

## Reading the evidence

Trust these, strongest first:

1. **The game's log:** `Selecting <site> as primary`, `Ping = ...`, `Lost backup`, and
   `Relays: N valid`.
2. **`pktmon` at the network card**, as `load-sim.ps1` does: packets per destination.
3. **Drop lines in the console:** `blocked <app> -> <site> <ip> xN`.
4. **Ping errors:** `General failure` means a local filter refused it. A timeout means little here.

Expected with ExitLag and a strict pick: 0 packets directly to relays, traffic to its nodes, `Relays:
1 valid`, a primary on the pick, and no backup.

## Reporting

- State what ran, on which build, and what it couldn't cover.
- Keep a passing check apart from what it proves. A running process doesn't prove a connected game;
  ask the user.
- Leave the user's setup as you found it. Say if a block is still on, and how to undo it.
- Never stop or close the user's game, ExitLag or running dl-conn-rs without asking.
