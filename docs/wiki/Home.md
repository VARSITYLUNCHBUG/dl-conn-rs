# dl-conn-rs

dl-conn-rs lets you pick one Steam relay server and blocks all the others, so your game can only
connect through the server you chose. The block lasts until you pick again or restart the PC. It
works with or without ExitLag.

It chooses which relays the game can reach. It does not make any route faster, and no setting
lowers ping beyond choosing a route.

## Start here

1. [Installation](Installation): download, verify the hash, run.
2. [Using dl-conn-rs](Using-dl-conn-rs): pick a server, keys, counters, backup relay.
3. [Testing your setup](Testing-Your-Setup): prove the block works in a custom game.

## Reference

- [Settings](Settings)
- [Console commands](Console-Commands)
- [How blocking works](How-Blocking-Works)
- [ExitLag](ExitLag)
- [Troubleshooting](Troubleshooting)
- [Building from source](Building-from-Source)

## At a glance

| | |
| --- | --- |
| Platform | Windows 10 and 11, Windows Server 2016 and later, x64 |
| Install | None. One exe that asks for admin rights |
| Block lasts | Until you pick again or restart the PC |
| Stored on disk | Settings and a capped log in `%LOCALAPPDATA%\dl-conn-rs` |
| Licence | MIT |
