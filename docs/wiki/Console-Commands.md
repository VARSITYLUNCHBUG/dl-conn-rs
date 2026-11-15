# Console commands

Press the backtick key to open or close the console. It's mainly a live log, with a prompt at the
bottom. Everything the app does is a `verb noun [options]` command, and the dropdown runs the same
commands, so its choices show up in the log too.

While the console is open, a report repeats every 5 seconds by default. It shows processes, the
pick, relay latency and routes, and any traffic stopped by the block. Use `stop report` for quiet.

## Commands

| Command | Effect |
| --- | --- |
| `help [verb [noun]]` | List commands, or explain one |
| `list servers [--filter <text>]` | List relay sites you can pick |
| `list relays [<server>]` | List relay addresses, for all sites or one |
| `show status` | Show the pick, what's blocked and connection state |
| `set pick <server> [--backup <server>] [--now]` | Block every relay except this server's. Mid-match, waits for the match to end |
| `clear pick` | Remove the block so all servers are open |
| `clear pending` | Drop a pick that's waiting for the match to end |
| `refresh relays` | Fetch relay lists from Steam now |
| `show routes [<server>]` | Show the interface and gateway to each relay |
| `ping relays [<server>]` | Measure latency to each relay |
| `show processes` | Show running game, Steam and ExitLag processes |
| `show report` | Print processes, pick, relays and blocked traffic once |
| `start report [--every <secs>]` | Repeat the report and collect blocked traffic |
| `stop report` | Stop repeating the report |
| `toggle console` | Show or hide the console, and its report with it |
| `clear screen` | Clear the console |
| `show config` | Show every setting and what it does |
| `set config <key> <value>` | Change a setting and save it |
| `clear config [<key>]` | Put one setting, or all of them, back to the default |
| `analyze log` | Read the game's log and report on its relay connections |
| `show guide` | Show how to test a connection, step by step |
| `quit` | Close the app. Blocks stay on until reboot or the next pick |

## Prompt keys

| Key | Effect |
| --- | --- |
| `Tab` | Complete a word. A second `Tab` lists the options |
| `?` | Explain what you've typed so far |
| `Up` / `Down` | Command history |
| `Esc` | Clear the line, then go back to Status |
| `` ` `` | Close the console |

Servers complete by code or by name, so `set pick frank` then `Tab` offers the Frankfurt sites.

## Examples

```
list servers --filter japan
set pick fra
set pick sgp --backup maa2
ping relays fra
clear pick
```
