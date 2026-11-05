---
name: add-ui-feature
description: Add or change a dl-rs window, dialog, control or hotkey (dropdown, counters, F1/F2/F3 windows) as a thin Win32 wrapper over commands. Use for any dl-rs UI work.
---

# Add a dl-rs UI feature

The UI holds no logic. It sends command lines and displays what comes back. `docs/roadmap.md` has
design notes for the planned windows, so check it first.

## Pattern

1. **Make the behaviour a command first** with `/add-command`. A window that can't be replaced by
   typing into the console is a design error.
2. **Acting:** call `session.run("verb noun ...")`. It's echoed to the log, so the console shows
   what the UI did.
3. **Reading:** call `session.query(...)`, which is silent, and use typed `Output`. Never parse
   rendered text.
4. **Never block the UI thread on the session.** Run the query on a thread and hand the result back
   with `PostMessageW`, the way `view_loop` and `publish` do. Filling a window at startup or from a
   hotkey works the same way.
5. **Refreshing:** react to the `changed` signal through `view_loop`. Don't add timers to the UI.
6. **Keep display logic pure.** Build text and rows in functions like `build_view` and `title`, and
   unit test those. Win32 calls stay in thin wrappers.

## Win32 notes

- Filter hotkeys in the message loop in `ui::run` so they work whichever control has focus. If the
  key has a meaning to a focused control, it won't reach it, as with backtick and the dropdown.
- Don't hold a `Mutex` across `SendMessageW`, because it can re-enter a window procedure.
- Commit dropdown changes when it closes, not while arrowing through an open list. Skip a command if
  the selection didn't change, so nothing extra reaches the log.
- A visible window must not take focus unprompted: the player may be mid-match. Only foreground a
  window in response to a user's key press.
- Scale sizes with `scale`. Use the system message font, and Consolas for logs and reports.

## Text

- Plain ASCII, no emoji or em dashes. `scripts/check.ps1` enforces this.
- Short, literal labels. Say what's real: for example, label a ping by what it measures.

## Verify

Run `scripts/check.ps1`, then `/verify-live` to check it in the real app. Extend
`scripts/live-test.ps1` with any new behaviour, and look at a screenshot rather than only asserting
on text. Keep `CB_SELECTSTRING` working, since the test scripts drive the dropdown with it.
