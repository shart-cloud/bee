# Contract: modes, CLI, and the terminal-restore guarantee

> **Command names changed.** The consolidation (ADR-0002) replaced `bee-episode`, `bee-repl`, and
> `bee-metrics` with subcommands of the single `bee` executable: `bee run`, `bee repl`, and
> `bee metrics`. The raw process runner moved from `bee run` to `bee exec`, and a session with no
> policy now needs an explicit `--host`. The commands below are recorded as this feature shipped
> them; translate accordingly.

How the operator selects the front-end, how bee auto-falls-back, and the non-negotiable
terminal-restore contract. Satisfies FR-002, FR-013, FR-015, FR-018; SC-001, SC-004, SC-006.

## CLI surface (`bee-repl`)

| Flag | Meaning |
|---|---|
| `--tui` | Request the full-screen front-end (subject to the capability check below). |
| `--no-tui` | Force the inline front-end even on a capable terminal. |
| *(neither)* | **Default: inline** (opt-in TUI for this release — spec Assumptions). |

The flags compose with every existing `bee-repl` flag (`--provider`, `--no-bee`, theme, etc.); session
configuration is shared. No separate `bee-tui` binary (research D10).

## Front-end selection (auto-detect)

```
choose_frontend(flags, env, term):
  if flags.no_tui:                         -> Inline
  if not flags.tui:                        -> Inline           # default
  if not stdout.is_terminal():             -> Inline  (+note)  # piped/redirected (FR-013)
  if env.TERM == "dumb":                   -> Inline  (+note)
  if term.size < HARD_FLOOR (40x10):       -> Inline  (+note: "terminal too small for --tui")
  else:                                    -> FullScreen
```

`(+note)` = a one-line stderr message before the inline session starts, so `--tui` never silently
"does nothing." `NO_COLOR` does **not** force inline — the TUI runs in monochrome (FR-014).

**SC-004 parity**: in the Inline path, output is the exact byte-stream the REPL produces today (no
alt-screen or cursor-control escapes that would corrupt a captured log).

## Terminal-restore contract (SC-001, FR-002)

For the FullScreen front-end, on **every** exit path the terminal MUST be returned to: main screen (not
alt), cooked mode (raw disabled), cursor visible, mouse capture off.

| Exit path | Mechanism |
|---|---|
| Normal quit (`q` / Ctrl-D) | explicit `restore()` before return |
| Early return / `?` error | `Drop` guard on the terminal handle |
| Panic | panic hook restores **before** printing the report (color-eyre) |
| Ctrl-C (SIGINT) | treated as quit; restore then exit cleanly |
| Ctrl-Z (SIGTSTP) | leave alt-screen + restore, then re-raise `SIGTSTP`; on `SIGCONT` re-enter alt-screen and force a full redraw |

**Verification**: an automated matrix exercises each path and asserts the terminal state is restored
(SC-001 = 100%). Prior shell scrollback is never written to (alt-screen only).

## Responsive contract (FR-015, SC-006)

Re-evaluated on every `Resize`:

| Terminal width × height | Layout |
|---|---|
| ≥ 120 cols | `TwoPane` — chat + right-hand panel column |
| 80–119 cols | `SinglePane` — chat; panels via a toggle key (overlay) |
| < 80 cols or < 24 rows | `SinglePane` — chat only; panels overlay-only |
| < 40 cols or < 10 rows (hard floor) | `TooSmall` — render only "terminal too small (min 40×10)" |

No layout may overlap or truncate chrome; multi-pane always has the single-pane fallback.
