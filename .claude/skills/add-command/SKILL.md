---
name: add-command
description: Add or change a dl-rs verb-noun command, including its parsing, help, completion, session handling and tests. Use for any new behaviour, setting or diagnostic in dl-rs, because all behaviour goes through commands.
---

# Add a dl-rs command

Commands are the only way to act on the app, so new behaviour starts here. One table drives parsing,
help and completion.

## Steps

1. **`src/command.rs`:** add a variant to `Command`, then a `Spec` to `COMMANDS`.
   - Name it `verb noun`, reusing an existing verb where one fits: `list`, `show`, `set`, `clear`,
     `start`, `stop`, `refresh`, `ping`, `toggle`. One-word commands are for `help` and `quit` only.
   - Declare `params`. An `Option` whose value is `SERVER_VALUE` completes to server codes.
   - Do validation that needs no app state in `build`, and return a clear error.
2. **`src/session.rs`:** handle the variant in `execute`. The match is exhaustive, so the compiler
   points you there.
   - Return typed `Output`, adding a variant with a `render` arm if a UI needs the data.
   - Validate servers with `known`. Change firewall state only through `reconcile`.
   - Anything slower than about a second runs off the session thread, because commands run one at
     a time.
3. **Windows code**, if any, goes in `src/diag.rs` or `src/wfp.rs`, returning data or lines. If it
   needs the front-end, extend the `Host` trait and the fakes.
4. **Front-ends** need nothing unless they use the command. See `/add-ui-feature`.
5. **README:** add a row to the command table.

## Tests

Add to the existing `tests` modules:

- `command.rs`: parsing, each error, and completion at each position. The table tests already cover
  help. If you add a required parameter type, update the `example` helper.
- `session.rs`: behaviour through the `run` helper with `FakeFirewall` and `FakeHost`. Cover the
  failure path, and assert filter writes with `writes` if the command touches the firewall.

## Rules to hold

- Unchanged results must not rewrite filters, because every write makes Windows re-check live flows.
- Bad data or errors must never lift a block.
- User-facing text is plain ASCII, with no emoji or em dashes.
- Output must not claim more than was measured. See "Facts learned the hard way" in `CLAUDE.md`.

## Finish

Run `powershell -ExecutionPolicy Bypass -File scripts/check.ps1`. If the change affects real blocking
or the console, follow with `/verify-live`.
