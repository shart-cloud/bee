# Live TUI runbook (008 — US1 full-screen chat + US2 model-owned panels)

## 1. Set your key
```bash
export ANTHROPIC_API_KEY=sk-ant-...
```

## 2. Launch the full-screen TUI
Release binary (built for you — snappier):
```bash
./target/release/bee-repl --provider .scratch/tui-demo.toml --tui
```
Or straight from cargo:
```bash
cargo run --release -p bee-harness --features tui --bin bee-repl -- \
  --provider .scratch/tui-demo.toml --tui
```

`render` is in the default toolset, so the model can draw straight away. Drop the
mascot with `--no-bee` if you want a cleaner first screen.

## 3. What to exercise

### US1 — full-screen chat + terminal safety
- Type a message, `Enter` to send; watch tokens stream into the chat pane.
- `Tab` cycles focus (input ↔ chat). In **chat** focus: `↑/↓ j/k` scroll,
  `PgUp/PgDn` page, `gg`/`G` top/bottom, `i`/`Enter` back to input.
- `?` opens the keybindings overlay (`?`/`Esc` closes).
- **The safety contract** — after any of these, your prompt + scrollback must be
  intact (no raw-mode/alt-screen corruption):
  - `q` (from chat focus) or `Ctrl-C` → clean quit
  - `Ctrl-Z` → suspend, then `fg` → resume (redraw)  *(note: full SIGTSTP redraw
    is US3/T035 — today it may not repaint until the next key; the terminal must
    still be usable)*

### US2 — model-owned live panels (the new thing)
Ask the model to put something in a **named panel** and then update it. Good prompts:

> Use the render tool to draw a 2×2 grid of gauges (cpu, mem, net, disk) and send
> it to a panel named "metrics" with `render_to("metrics", g)`.

Then, in a follow-up turn:

> Now update the "metrics" panel with new values.

**Expected:** a bordered `metrics` panel appears in the right-hand column (needs a
terminal **≥120 cols** for TwoPane), updates **in place** on the second turn (no
duplicate panel, no chat scrollback churn), while the chat still shows the tool
summary line (`✓ Rendered a 2×2 grid to panel "metrics".`). Untargeted `render(g)`
still lands inline in chat.

You can hand the model the example script to adapt:
`specs/003-visual-render/examples/panel-metrics.rhai`.

## 4. If something breaks
- Paste the last chat/tool line and what you did — reducer/view logic is unit- and
  snapshot-tested, so a live break is most likely in the event loop (T018) or the
  panel column layout (T028).
- Panels only show in **TwoPane** (≥120 cols). Below that they're overlay-only,
  which is US3 (T036) and not wired yet — so on a narrow terminal you'll see the
  tool summary in chat but no visible panel. Widen to ≥120 to see the column.
