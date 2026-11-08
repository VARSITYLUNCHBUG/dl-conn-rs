# How blocking works

## Relay list

Steam games that use Steam Datagram Relay (SDR) connect through Valve relays. dl-conn-rs fetches
the relay lists from Steam's `GetSDRConfig` API for CS2, Dota 2 and Deadlock, which together
covered every distinct relay set found when sampling popular games, and merges them.

The list refreshes every 5 minutes by default, matching the API's cache lifetime, and once after
each new pick. Failures back off. A failed fetch or bad data never drops a relay or lifts a block.

## Filters

For every relay site except your pick (and backup, if set), dl-conn-rs adds one Windows Filtering
Platform (WFP) filter at the `ALE_AUTH_CONNECT_V4` layer.

- **All apps and protocols.** Any traffic from this PC to a blocked relay address is refused.
  Traffic to other addresses, including ExitLag's own servers, is not touched.
- **No per-packet cost.** WFP checks the filter once per flow. Live flows are re-checked when
  filters change, so a pick also cuts existing connections to blocked relays.
- **Atomic.** Each change is one WFP transaction, so there is never a half-applied state.
- **Quiet.** Filters are only rewritten when the result would differ, because every write makes
  Windows re-check all live flows.
- **IPv4 only.** Steam's relay configs list IPv4 addresses only.

## Until reboot

The filters are not persistent. They last until they are removed or the Windows filtering service
stops, which happens at restart. Nothing goes to the Windows Firewall rule store.

The active pick is stored in WFP itself, not in memory or on disk. When the app opens, it reads the
pick back from its filters, so closing and reopening carries on where you left off.

## Resource use

The app does no work per packet. While there is a pick it sends one ping per `ping_every` seconds
for the Direct counter, and checks for running games and the log size every 5 seconds. The window
only redraws on input or a change. Console diagnostics run only while the console is open.
