# Using dl-conn-rs

## Pick a server

1. Open dl-conn-rs.
2. Click the dropdown. Sites are grouped by region. Type to narrow the list by city, code or
   region, then press Enter, or click a site. Esc cancels the search.
3. Start the game. If it's already running, check the **Connection Hold** chip under the dropdown.

The title bar shows what's active, for example `dl-conn-rs - Frankfurt (Germany) - 210 blocked`.

## Switch servers

You can pick a new server at any time, and the block changes at once. The game measures relay
pings only when it starts, and matchmaking uses that measurement, so the game only sees a new pick
after it restarts. The **Connection Hold** chip, which reads the game's log, tells you what's left.
Hover it for the details.

| Connection Hold | Meaning |
| --- | --- |
| `In match` | A pick that closes this match's server waits for the match to end |
| `<server> after match` | You picked during a match. The switch happens when it ends |
| `Restart game` | The game measured before this pick. Restart only the game |
| `Next launch` | The game will measure your pick when it starts |
| `Ready` | The game measured your pick and no blocked server |
| `No log` | The game isn't writing its log, so a pick applies at once, even mid-match |

Keep `-condebug` in the game's launch options so the hold can see matches. The game starts its log
again on each launch, so the file only holds the current session.

Steam can stay open. The game does its own relay pinging, so restarting Steam doesn't help.

A pick made during a match waits, because switching would cut the match's relay. `clear pending`
cancels the wait, and `set pick <server> --now` switches anyway. A solo hideout isn't a match, so a
pick there applies at once.

## Remove the block

Choose **No block** at the top of the dropdown, run `clear pick` in the console, or restart the PC.

## Closing and reopening

Closing the window leaves the block on, unless `clear_on_exit` is set in [Settings](Settings).
Reopening the app reads the active pick back from Windows and carries on.

## Keys

| Key | Page |
| --- | --- |
| `F1` | Help: launch options, testing in a custom game, reading the results |
| `F2` | Settings |
| `F3` | Log report |
| `` ` `` (backtick) | Console |
| `Esc` | Back to Status, or cancel a search |

Keys work while the window has focus. They are not global hotkeys. The theme is set in Settings
(F2): Dark (Dracula), Light (Catppuccin Latte), or System to follow Windows.

## Status chips

The chips under the dropdown show a short value. Hover one for what it means.

**Direct** is a ping from this PC straight to your pick. The game's real path can differ,
especially through a routing tool, so this is not your in-game ping. The last value holds for two
missed pings, because single misses are common.

**Game** is the last figure the game wrote to its own log, with the time it was written. It is not
live, because the game only logs it when it measures or connects. It needs `-condebug` in the
game's launch options, as set up in [Testing your setup](Testing-Your-Setup). While ExitLag is
running it is labelled **ExitLag**.

**Connection Hold** says whether a pick waits for a match and what a switch still needs. See
[Switch servers](#switch-servers) above.

## Backup relay

Normally the game keeps a backup relay ready in case the main one fails. A strict pick blocks it,
so the connection depends on one relay.

To keep a failover route, name a second site. Both stay open and everything else is blocked:

```
set pick sgp --backup maa2
```

To use a backup with every pick, including dropdown choices, set `backup` in F2. Use
`--backup none` to skip it for one pick.

## Other server pickers

Don't run another server picker at the same time. Its firewall rules still apply, so it can block
the server you picked here.
