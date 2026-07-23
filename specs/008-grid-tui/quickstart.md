# Quickstart: validating the Full-Screen TUI + live panels

> **Command names changed.** The consolidation (ADR-0002) replaced `bee-episode`, `bee-repl`, and
> `bee-metrics` with subcommands of the single `bee` executable: `bee run`, `bee repl`, and
> `bee metrics`. The raw process runner moved from `bee run` to `bee exec`, and a session with no
> policy now needs an explicit `--host`. The commands below are recorded as this feature shipped
> them; translate accordingly.

Runnable scenarios that prove the feature end to end. Assumes the implementation from `plan.md` (a
`--tui` front-end in `bee-harness`, the `render_to` surface, the `SessionEngine`). References
[data-model.md](./data-model.md) and [contracts/](./contracts/) for shapes; no implementation code
here.

## Prerequisites

- A capable interactive terminal (truecolor recommended; `COLORTERM=truecolor`).
- A provider config (e.g. `specs/002-llm-harness/examples/anthropic.toml`).
- Build: `cargo build -p bee-harness --bin bee-repl` (with the TUI backend feature enabled).

## Scenario A — full-screen session & terminal restore (US1 / SC-001)

1. `./target/debug/bee-repl --provider <cfg> --tui`
2. Expect: alt-screen with a chat area, an input line, and a footer hint bar; shell scrollback untouched.
3. Send a message; expect the reply to **stream** in; input stays responsive.
4. Scroll with `PgUp`/`PgDn` and `gg`/`G`.
5. Exit each way and confirm the terminal is usable and prior scrollback is intact:
   - `q` (normal), `Ctrl-C` (interrupt), `Ctrl-Z` then `fg` (suspend/resume).
   - Panic path: run the panic-injection test (below) — the trace prints on a restored terminal.

**Pass**: every exit leaves a normal-screen, cooked-mode, cursor-visible terminal (SC-001).

## Scenario B — model takes over a live panel (US2 / SC-002, SC-008)

Drive the model (or use a mock turn) to run, via the render tool:

```rhai
let g = grid(2, 3);
g.cell(0, 0, gauge("CPU", 0.82));
g.cell(0, 1, gauge("MEM", 0.51));
g.cell(0, 2, gauge("NET", 0.13));
g.span(1, 0, 1, 3, table("recent"));
render_to("metrics", g);        // -> a persistent panel named "metrics"
```

1. Expect a panel titled `metrics` beside the chat, showing the grid.
2. On a later turn, re-render with new values to `render_to("metrics", …)`.
3. Expect the **same** panel to update in place — no second panel, chat history unchanged.
4. Repeat ≥ 20 times; the panel always shows the latest state (SC-008).
5. `render(g)` (no target) → the grid appears **inline** in the chat instead (FR-008 scenario 3).

**Pass**: exactly one `metrics` panel, always current; inline vs panel routing correct.

## Scenario C — fallback & degradation (US3 / SC-004, SC-006)

1. **Pipe**: `./target/debug/bee-repl --provider <cfg> --tui | cat` → runs the **inline** path;
   output is linear, capturable text (SC-004). A one-line note explains the fallback.
2. **NO_COLOR**: `NO_COLOR=1 … --tui` → TUI runs in monochrome; every state still distinguishable.
3. **Narrow**: resize to ~60 cols → single-pane layout; press `p` to overlay the panel.
4. **Too small**: resize below 40×10 → only a "terminal too small (min 40×10)" message.

**Pass**: no broken screen in any environment; each degrades as specified.

## Scenario D — replay reconstructs panels (SC-009)

1. Run a session that creates and updates a panel (Scenario B), then end it.
2. Replay the episode transcript.
3. Expect each panel's **final** state reconstructed from `PanelUpdate` events — identical to live.

**Pass**: replayed panel state matches the live final state (SC-009).

## Automated checks (map to CI)

| Check | Asserts |
|---|---|
| `cargo test -p bee-harness --test tui_update` | reducer: submit, scroll, focus, panel replace, coalesce, quit |
| `cargo test -p bee-harness --test tui_snapshot` | frame snapshots at pinned size + `NO_COLOR` |
| terminal-restore matrix test | SC-001 across quit/interrupt/suspend/panic |
| `cargo test -p bee-harness --test grid` / `render_sandbox` | existing behavior still green |
| bee-core dependency-guard test | no terminal/runtime dep leaked into the core (FR-021) |
| `cargo fmt --all --check` · `cargo clippy --workspace --all-targets -- -D warnings` | repo gates |
