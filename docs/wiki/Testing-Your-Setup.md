# Testing your setup

The log report (F3) reads the game's own log and checks it against your pick. This page shows how
to prove the block works in a custom game before you rely on it in a real match. The same steps
are in the app under F1.

The steps use Deadlock, whose log is found automatically. Other games are untested. For another
game, set `log_path` in [Settings](Settings).

## 1. Turn on logging

1. In Steam, open your Library.
2. Right-click Deadlock and choose **Properties**.
3. On the **General** tab, find **Launch Options**.
4. Add: `-condebug -lognetapi`
5. Close the window.

The log is written to `steamapps\common\Deadlock\game\citadel\console.log` in your Steam library.
The game rewrites it each time it starts, so copy it first if you want to keep one.

## 2. Pick your server first

1. Close the game.
2. Choose your server in the dl-conn-rs dropdown.
3. Start the game.

## 3. Test in a custom game

1. Create a private lobby and start the game.
2. Once you're in the match, stay connected for at least 2 minutes.
3. Leave the match.

The hideout doesn't count. It runs on your own PC, so it uses no relay.

## 4. Read the report

Press **F3**, or run `analyze log` in the console. The report reads the log with shared access, so
it's safe while the game runs. Steam IDs and player names are left out.

Findings at the top are marked:

| Mark | Meaning |
| --- | --- |
| `OK` | Checked and looks right |
| `CHECK` | Doesn't match what you asked for. Read the advice with it |
| `NOTE` | Worth knowing. Nothing is wrong |

A working strict pick shows:

| Finding | Expected |
| --- | --- |
| Relays | `1 valid`: the game could reach only your pick |
| Primary | On the site you picked |
| Backups | None, when no backup is set |

Below the findings are the relay config, ping measurements, the ping table, and each relay
connection with its loss and ping figures.

## What each CHECK usually means

**More relays valid than sites open.** The block wasn't on when the game measured. Pick first,
then restart the game. Close any other server picker, since its rules still apply.

**Primary isn't your pick.** The game connected somewhere else. Check that the log is from after
your pick.

**Backup outside your open sites.** The game selected a relay that should have been blocked. If you
cleared the block during that match, this is expected. If not, repeat the test with the block held.

**No relay valid.** The game could reach no relay. Check that your pick has relays online, and that
no other tool is blocking them.

If the report says there's no SDR connection, that session didn't use a relay and proves nothing
about the route.

## 5. Move to a real match

When the custom game passes, play a real match and press F3 afterwards.

## 6. Turn logging off

Remove `-condebug -lognetapi` from Launch Options when you're done. The log grows by several
hundred KB every few minutes. Blocking works without these options. Only the report and the Game
counter need them.

## Checking the block directly

- Open the console with the backtick key. While it's open, a report lists traffic stopped by the
  block, such as `blocked deadlock.exe -> waw`.
- From an admin Command Prompt, `netsh wfp show state file=wfp.xml` writes the active filters to
  `wfp.xml`. Search it for `dl-conn-rs`.
