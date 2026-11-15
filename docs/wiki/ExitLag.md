# ExitLag

You don't need ExitLag to use dl-conn-rs. If you use it, the two work together.

## Choose the same server in both

Pick the same server in dl-conn-rs as the one ExitLag routes to. A block can't change where ExitLag
routes, and ExitLag's route choice is separate from dl-conn-rs.

## Blocks still apply

ExitLag tunnels boosted traffic through its own nodes. dl-conn-rs only matches Valve relay
addresses, so ExitLag's traffic to its own servers isn't touched.

In a live Deadlock session routed through ExitLag, the game's traffic left the PC only as tunnels to
ExitLag's nodes, yet its packets to blocked relays were still dropped before entering the tunnel.

To check your own setup, boost the game, open the console and look for lines such as
`blocked deadlock.exe -> waw`.

## Status chips with ExitLag

- **Direct** pings are not boosted, so they can differ a lot from the game's real path.
- The **Game** chip reads **ExitLag** while ExitLag is running. It shows the figure the game
  logged. A running ExitLag doesn't prove the game was boosted at that moment.
- If the log report says the route looks slower than direct, try another route or server in
  ExitLag.
