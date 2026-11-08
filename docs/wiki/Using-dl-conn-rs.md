# Using dl-conn-rs

## Pick a server

1. Close the game.
2. Open dl-conn-rs.
3. Click the dropdown. Sites are grouped by region. Type to narrow the list by city, code or
   region, then press Enter, or click a site. Esc cancels the search.
4. Start the game.

The title bar shows what's active, for example `dl-conn-rs - Frankfurt (Germany) - 210 blocked`.

Pick before the game starts, so the ping measurement it runs at startup already sees the block. A
block can't move a game that is already connected to a relay you left open.

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

Keys work while the window has focus. They are not global hotkeys. The theme switch at the top
follows Windows, or stays light or dark.

## The two counters

**Direct** is a ping from this PC straight to your pick. The game's real path can differ,
especially through a routing tool, so this is not your in-game ping. The last value holds for two
missed pings, because single misses are common.

**Game** is the last figure the game wrote to its own log, with the time it was written. It is not
live, because the game only logs it when it measures or connects. It needs `-condebug` in the
game's launch options, as set up in [Testing your setup](Testing-Your-Setup). While ExitLag is
running it is labelled **ExitLag route**.

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
