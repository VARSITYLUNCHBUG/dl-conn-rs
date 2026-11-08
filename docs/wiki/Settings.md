# Settings

Press **F2** to change settings in the app. In the console, use `show config` to list them,
`set config <key> <value>` to change one and `clear config [<key>]` to reset one or all.

Settings are saved to `%LOCALAPPDATA%\dl-conn-rs\settings.ini`. A damaged file falls back to
defaults and never changes a block.

| Key | Values | Default | Effect |
| --- | --- | --- | --- |
| `backup` | `none` or a server | `none` | Default failover site for each new pick. `--backup none` skips it for one pick |
| `ping_every` | `0` to `300` s | `5` | Direct counter interval. `0` stops all ping traffic |
| `refresh_minutes` | `0`, or `5` to `1440` | `5` | Relay list refresh. `0` is manual, which also stops automatic filter repair |
| `hold_in_game` | `on`, `off` | `off` | Delays background filter rewrites while a game runs. New relays stay unblocked until it closes |
| `report_every` | `1` to `3600` s | `5` | Console report interval |
| `report_on_open` | `on`, `off` | `on` | Start the report when the console opens |
| `clear_on_exit` | `on`, `off` | `off` | Remove the block when the app closes |
| `theme` | `system`, `light`, `dark` | `system` | How the window looks |
| `log_path` | `auto` or a path | `auto` | The game log to read. `auto` finds Deadlock's. Other games are untested |

No setting lowers ping beyond choosing a route.

## Examples

```
set config backup maa2
set config ping_every 0
set config clear_on_exit on
clear config
```
