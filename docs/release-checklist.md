# Release checklist

Automated checks can't reach everything. Run these before giving a build to other people, and write
down what you ran it on.

## Build

- [ ] `scripts/check.ps1` passes.
- [ ] `scripts/release.ps1` builds, and reports no machine paths.
- [ ] `dist\dl-conn-rs.exe` shows the right version and icon under Properties.
- [ ] Signed, or you've accepted that users will see "Unknown publisher" and SmartScreen warnings.
- [ ] `dist` holds `THIRD_PARTY_NOTICES.txt`, `LICENSE.txt`, the `.md5` and `.sha256` files and the
      source zip, and all of them are attached to the GitHub release.

## Windows versions

The target is Windows 10 and 11, and Server 2016 and later, x64 only. On each one you can reach, open
the app, pick a server, check the block with `ping`, open each page, and restart to confirm the block
is gone.

- [ ] Windows 11
- [ ] Early Windows 10 (1607 or 1809 LTSC)
- [ ] Windows Server 2016 or later
- [ ] A non-English Windows, then read a real game log with F3. The log format is unconfirmed there.
- [ ] Windows on ARM isn't a target. Note what happens if you have one.

## Machines unlike this one

- [ ] A clean PC or Windows Sandbox: no Steam, no ExitLag, no game. It should open, pick and block, and
      F3 should explain that Steam wasn't found.
- [ ] Steam in a non-default folder, and a game in a second library.
- [ ] A user name with non-English letters.
- [ ] A standard user elevating with an administrator's password. Settings then belong to that
      administrator, and Steam should still be found.
- [ ] No usable GPU: a VM with no 3D, or Remote Desktop. The software renderer should take over, which
      `dl-conn-rs.log` shows on its `renderer:` line. `DL_CONN_RS_RENDERER=software` forces it for a quick check.
- [ ] 100%, 150% and 200% display scaling, and a window moved between monitors.
- [ ] Light and dark mode, with the theme set to `system`, changed while the app is open.

## Things only time or a real game can prove

- [ ] A real reboot clears the block.
- [ ] Sleep and resume, with a pick on.
- [ ] Starting offline, then going online.
- [ ] Filters removed by hand come back at the next refresh.
- [ ] A custom game: `Relays: 1 valid` and a primary on the pick in F3.
- [ ] A real match, with and without ExitLag.
- [ ] The second counter during a boosted match says `ExitLag route`, and without ExitLag says `Game`.
- [ ] `--backup` and the default backup setting keep a failover relay in the game.
- [ ] `hold_in_game` during a real relay list change.
- [ ] No visible effect on the game's frame rate with the window open beside it.
