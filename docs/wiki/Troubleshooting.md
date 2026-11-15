# Troubleshooting

**Windows says "Unknown publisher" or "Windows protected your PC".**
The exe isn't code signed. Check the hash as shown in [Installation](Installation), then choose
**More info** and **Run anyway**.

**The window is blank or fails to draw.**
The app falls back to the software renderer if DirectX 12 fails. To force it, set the environment
variable `DL_CONN_RS_RENDERER=software` before starting the app. `dl-conn-rs.log` shows which
renderer was used on its `renderer:` line.

**The game still connects to another server.**
The game measures relay pings only when it starts, so restart the game after picking. Steam can
stay open. The **Connection Hold** chip says when the game has measured your pick. Close
any other server picker. Then follow [Testing your setup](Testing-Your-Setup) and read the
findings in F3.

**I picked a server and nothing changed.**
If you picked during a match, the switch waits for the match to end. **Connection Hold** says so.

**A pick cut my match.**
**Connection Hold** shows `No log` when the game isn't writing its log, and then a pick can't wait
for the match. Keep `-condebug` in the game's launch options.

**The window draws in the wrong place after a fullscreen game starts.**
The app redraws when Windows reports a display change. If part of the window still looks wrong,
move the window or hover over it to redraw it, and report it with `dl-conn-rs.log`.

**A custom game won't connect.**
A private lobby can be hosted on a server you blocked. Leave and create the lobby again, or choose
**No block** for custom games.

**F3 says the log wasn't found or is empty.**
Add `-condebug -lognetapi` to the game's launch options and start the game again. If Steam or the
game is in an unusual folder, set `log_path` in [Settings](Settings).

**The Direct ping doesn't match the game.**
That's expected. Direct is a plain ping from this PC, which can take a different path from the
game, especially through ExitLag. Use the Game chip or the F3 report for the game's figure.

**A single Direct ping fails.**
Single misses are common on some connections and are not a sign of a block. The counter keeps the
last value for two missed pings.

**How do I undo everything?**
Choose **No block**, or restart the PC. Filters never survive a restart.

**Can I see the filters?**
From an admin Command Prompt, run `netsh wfp show state file=wfp.xml` and search `wfp.xml` for
`dl-conn-rs`.

**Where are the settings and log?**
`%LOCALAPPDATA%\dl-conn-rs`. The log is capped at 1 MB.

## Reporting a problem

Open an [issue](https://github.com/VARSITYLUNCHBUG/dl-conn-rs/issues) with your Windows version,
the app version from the exe's Properties, what you expected and what happened. Attach
`dl-conn-rs.log` if you can, after checking it for anything you'd rather not share.
