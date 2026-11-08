---
name: add-ui-feature
description: Add or change a dl-conn-rs page, control or hotkey (dropdown, counters, F1/F2/F3 pages, console) as a thin egui wrapper over commands. Use for any dl-conn-rs UI work.
---

# Add a dl-conn-rs UI feature

The UI holds no logic. It sends command lines and displays what comes back.

## Pattern

1. **Make the behaviour a command first** with `/add-command`. A page that can't be replaced by typing
   into the console is a design error.
2. **Acting:** call `session.run("verb noun ...")`. It's echoed to the log, so the console shows what the
   UI did. A control hands back a command line and `App` runs it.
3. **Reading:** call `session.query(...)`, which is silent, and use typed `Output`. Never parse
   rendered text.
4. **Never block the UI thread on the session.** Run the query on a thread and send the result back as
   an `Update`, as `App::fetch` and `view_loop` do, then call `request_repaint`.
5. **Refreshing:** react to the `changed` signal through `view_loop`. No timers, and nothing that asks
   for a repaint every frame.
6. **Keep display logic pure.** Build text and rows in functions like `rows::build_rows` and
   `pages::status_rows`, and unit test those.

## egui notes

- Colours come from `theme::Palette`, never literals, so light and dark can't drift apart.
- Global keys go in `App::hotkeys`, which runs first, so they work whichever control has focus.
- Give every control an accessible name. `kittest` fails a test otherwise, and scripts find controls by
  it through UI Automation.
- Commit dropdown changes when it closes, not while arrowing. Skip a command if nothing changed.
- Only take focus in response to a user's key press: the player may be mid-match.

## Text

- Plain ASCII, no emoji or em dashes. `scripts/check.ps1` enforces this.
- Short, literal labels. Say what's real: for example, label a ping by what it measures.

## Verify

Run `scripts/check.ps1`, which includes the UI tests in `src/ui/tests.rs`. Add a test there for new
behaviour. Run `scripts/window-check.ps1` to look at the real window, then `/verify-live` to check it
with real filters. Look at a screenshot rather than only asserting on text.
