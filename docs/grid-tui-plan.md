# Plan: the Bee Grid TUI

A design plan for the next iteration of bee's terminal UI: a **model-owned N×M grid** built on the
existing Rhai visual scripting, and a full-screen layout that **balances a live chat history with
grid panels the model can take over**. Extends the [design system](./design-system.md).

> **Target (decided):** a full-screen, alt-screen ratatui app — persistent chat pane + live
> model-owned grid + input line. **Sequenced** so the grid *spec* ships inline first (zero I/O
> rewrite) and the full-screen driver lands as an **opt-in mode**, leaving today's inline REPL intact
> as the default and the non-tty fallback.

---

## 1. The core insight: reuse the widget layer, rewrite only the driver

The survey of the codebase splits cleanly:

| Layer | File(s) | Verdict |
|---|---|---|
| `RenderSpec` — pure-serde widget tree | `render_spec.rs` | **Reuse** + add a `Grid` variant |
| Rhai drawing API + hard sandbox | `render_api.rs`, `tools/render.rs` | **Reuse** + add a `grid()` builder |
| Headless ratatui → ANSI (`render_into`) | `viz/buffer_render.rs` | **Reuse** — already lays widgets into ratatui cells |
| Terminal driver: rustyline + inline `ExternalPrinter` | `repl.rs`, `repl/terminal.rs`, `viz/animator.rs` | **Rewrite** for anything persistent/live |

`RenderSpec` deliberately holds **no ratatui or rhai types** (`render_spec.rs:5-8`) so specs record
verbatim in the transcript and re-render later. That property is load-bearing and we keep it: the
grid is just more pure-serde data. ratatui is *already* a dependency (`Cargo.toml`), used headlessly
in one file to turn widgets into ANSI strings — so the grid **spec + renderer are a pure extension**;
only the interactive driver (raw mode, event loop, scrollback, live panels) is new.

---

## 2. The Grid spec — same in every phase

Extend the `RenderSpec` enum. A grid is a container of **cells**, each addressed by coordinate, each
holding *any* `RenderSpec` — so the entire component system nests into the grid for free.

```rust
// render_spec.rs
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GridCell {
    pub row: u16,
    pub col: u16,
    #[serde(default = "GridCell::one")] pub row_span: u16, // default 1
    #[serde(default = "GridCell::one")] pub col_span: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,          // optional bordered-cell title
    pub content: Box<RenderSpec>,        // any widget — charts, sprites, nested grids…
}

// new RenderSpec variant:
Grid {
    rows: u16,                           // 1..=MAX_GRID_DIM
    cols: u16,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    col_weights: Vec<u16>,               // optional per-track weights; empty = equal Fill(1)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    row_weights: Vec<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    gap: Option<u16>,                    // cells' inter-track spacing
    cells: Vec<GridCell>,
},
```

**Layout algorithm** (in `render_into`, `buffer_render.rs`): split the area into row-boundary and
col-boundary arrays via ratatui `Layout` with `Constraint::Ratio`/`Fill` from the weights; a cell's
`Rect` is the union of the base cells it spans (`boundary[col]..boundary[col+col_span]` ×
`boundary[row]..boundary[row+row_span]`). Then recurse: render each cell's `content` into its `Rect`.

**Validation** (at the `render()` commit, alongside the existing caps in `render_api.rs:17-24`):
- cells in bounds; spans don't exceed the grid; **no overlap** (reject; last-write-wins is a footgun).
- new caps: `MAX_GRID_DIM` (proposed **12**), `MAX_CELLS` (proposed **64**).
- update `RenderSpec::element_count()` and `nesting_depth()` to descend into cells. A `Grid` counts as
  one nesting level; **raise `MAX_NESTING` from 3 → 4** so a grid-of-vsplits stays legal.

**Fold-in fix:** today a sprite nested in a `Layout` degrades to a `[sprite W×H]` text placeholder
(`buffer_render.rs:319-328`) — only *top-level* sprites get the half-block renderer. In the grid
renderer, composite the sprite's half-blocks into the cell's `Buffer` region so **sprites work inside
cells** (a bee, a status icon, a sparkline-beside-a-gauge, all in one grid).

### Rhai surface (first-class, registered in `render_api.rs`)

```rhai
let g = grid(2, 3);                         // rows, cols
g.col_weights([2, 1, 1]);                   // optional
g.cell(0, 0, gauge("CPU", 0.82));           // row, col, widget
g.cell(0, 1, gauge("MEM", 0.51));
g.cell(0, 2, bee_sprite());                 // sprites in cells now render
g.span(1, 0, 1, 3, line_chart("throughput").series("req/s"));  // row,col,row_span,col_span
render(g);                                  // same commit path as every other widget
```

Registered exactly like the existing builders (`GridBuilder`, `register_type_with_name` +
`register_fn`), coerced through `dynamic_to_spec`. The sandbox is unchanged — no new capabilities,
just more drawing. Add `examples/grid-dashboard.rhai`.

**This is Milestone 1 and it ships in the current inline REPL** as a snapshot block — no driver work.

---

## 3. The full-screen driver (the rewrite, opt-in)

A new front-end — `bee-tui` (binary) / `--tui` (flag) — using the ratatui runtime the codebase does
not have yet. Auto-selected when stdout is a tty and the terminal is capable; **falls back to today's
inline REPL** otherwise (piping, `NO_COLOR`, `TERM=dumb`, tiny size). The inline REPL is *not*
deleted — it becomes the default-for-now and the permanent fallback.

**Architecture — TEA (Elm) over the existing tokio loop:**

```
        crossterm EventStream ─┐
   provider token stream ──────┼─► Message ─► update(&mut Model) ─► view(&Frame)
   tool RenderSpec / panel ────┤                    ▲                    │
        tick (animations) ─────┘                    └────────────────────┘
```

**Persistent layout** (panels never move — spatial stability):

```
┌─ header: model · policy · theme · $cost · tokens ───────────────────────┐
├───────────────────────────────┬─────────────────────────────────────────┤
│ chat transcript (scrollable,   │ model-owned grid panel(s)               │
│ virtualized; messages may hold │ live; replaced/updated across turns     │
│ inline widgets)                │ hidden when empty → chat goes full-width │
├───────────────────────────────┴─────────────────────────────────────────┤
│ > multi-line input                                                       │
├──────────────────────────────────────────────────────────────────────────┤
│ q quit · ? help · Tab focus · PgUp/Dn scroll · gg/G · : command          │
└──────────────────────────────────────────────────────────────────────────┘
```

**What's new, and where the design skills pin the bar:**
- **Runtime:** `ratatui::run()` + `CrosstermBackend`, alt-screen, raw mode, `color-eyre` panic hook
  that restores the terminal *first* (non-negotiable #1/#2).
- **Input:** replace rustyline with a ratatui text input (`tui-textarea`, or a small custom widget) —
  multi-line, history, no reserved keys (`Ctrl+C/Z/S/Q` stay the terminal's).
- **Streaming:** provider tokens stream into the focused assistant message; UI never blocks on I/O
  (already async). A shimmer "thinking…" line on a ~100 ms tick.
- **Scrollback is now ours:** virtualize the transcript; `PgUp/Dn`, wheel, `gg`/`G`. This is the real
  cost of alt-screen.
- **Copy-paste** (the biggest UX risk of leaving inline): `y` yanks the current message via **OSC 52**
  (survives SSH/tmux); document Shift-drag for native selection; the plain fallback mode always
  prints linearly for piping.
- **Resize/suspend:** handle `SIGWINCH` (debounced re-layout) and `SIGTSTP` (leave alt-screen,
  restore, `SIGCONT` redraw).
- **Responsive floor:** ≥120 cols → chat+grid side by side; 80–120 → grid pane on demand; <80 → single
  pane, grid becomes a toggled overlay popup; below min → "terminal too small."
- **Semantic color throughout:** reuse `viz::palette` roles + `viz::theme` (honeycomb default);
  usable under `NO_COLOR`.

---

## 4. Model ownership of grids — how "take over a region" works

This is the heart of the ask. A grid can be **inline** (flows into the chat like any tool result
today) *or* **addressed to a named panel** that persists on the right and updates in place.

- **Panel registry** in the TUI model: `HashMap<PanelId, RenderSpec>`. Rendering a panel id that
  already exists **replaces it → live update**; a fresh id opens a new panel.
- **Model affordance:** extend the `render` tool with an optional `panel` target, plus a Rhai
  `render_to("cpu", g)` (inline `render(g)` stays the default, back-compat). Optionally
  `panel("cpu").update_cell(0, 1, gauge(...))` for cell-granular updates.
- **Transcript:** panel updates record as `(panel_id, RenderSpec)` events — still pure serde, so
  replay reconstructs the panel state. No ratatui/rhai leaks into the transcript (invariant held).
- **Backpressure:** coalesce panel updates per render tick so a chatty model can't thrash the UI.

The model *decides* dimensions and contents at runtime (`grid(4, 3)` → fill cells) — nothing is
templated. "Take over a 3×5" is just `grid(3, 5)` addressed to a panel.

---

## 5. The mascot bug — fix now, independent of all the above

**Root cause** (`repl.rs:885-894`, `terminal.rs:208-212`): the wing-flap is fire-and-forget; the
"interactive session" line + prompt print *between* the sprite draw and its deferred row-reclaim, so
the later `\x1b[8A\r\x1b[0J` measures from the wrong baseline, lands mid-sprite, and erases downward —
leaving the two black antenna rows.

**Recommended fix (minimal, eliminates the whole bug class):** render the inline banner mascot as the
**static** level-wing `FRAME_MID` sprite (no flap, no cursor-reclaim) — which is exactly the resting
pose already shipping. Reserve the wing-flap for **TUI mode**, where a tick-driven redraw of the
mascot's header cell needs no reclaim hacks at all. (Alternative if you want the flap inline: `await`
the animation to completion before emitting the info line — larger change, keeps the fragile
reclaim.) Either way the `animator.rs`/reclaim scheme is **deleted** in TUI mode.

---

## 6. Shared core — the refactor that makes two front-ends sane

Before the TUI, extract the turn engine from `run_repl` into a `SessionEngine` that emits events
(`UserMessage`, `Token`, `ToolCall`, `RenderSpec`, `PanelUpdate`, `Denial`, `TurnDone`) over a channel.
Both front-ends consume the same stream:

- **inline REPL** (existing) — renders events as scrollback lines (fallback + default-for-now).
- **bee-tui** (new) — renders events into panes.

No user-visible change when it lands; it de-risks the TUI by isolating provider/tool/transcript logic
from presentation.

---

## 7. Roadmap

| Milestone | Scope | Rewrite? | Ships |
|---|---|---|---|
| ✅ **M0** | Mascot bug fix (static inline mascot) | none | today |
| ✅ **M1** | `RenderSpec::Grid` + Rhai `grid()` + `render_into` + caps + sprite-in-cell + tests + example | **pure extension** | grids as inline blocks — "model generates N×M grid" works now |
| ✅ **M2** | Extract `SessionEngine` (event-driven core) from `run_repl` | internal refactor | no visible change; unblocks M3 |
| ✅ **M3** | `bee-tui` alt-screen front-end: chat pane + input + footer, streaming, scroll, panic/resize/suspend, plain fallback | **new driver** | full-screen chat; grids inline-in-chat |
| ✅ **M4** | Model-owned **live panels**: named panels, `render_to`/update semantics, transcript panel events | builds on M3 | the full "take over a region" vision |
| ◐ **M5** | Polish: OSC 52 yank, mouse, command palette, layout config, responsive breakpoints, mascot in header, theme-coordinated grid borders | — | production feel |

**Status (008-grid-tui branch):** M0–M4 are landed — the full-screen front-end (`--tui`), model-owned
live panels with lifecycle (`render_to` / `render_to_ttl` / `remove_panel` / `clear_panels`), rich
inline rendering, transcript replay, and honest fallback. M5 is partly done (OSC 52 yank, responsive
breakpoints, panel overlay); mouse, command palette, and layout config remain.

M1 delivers the visible headline feature with zero risk to the current REPL. M3 is the big one and is
cleanly gated behind a flag.

---

## 8. Open questions / risks

1. **Copy-paste in alt-screen** — the top usability risk. OSC 52 yank + a plain fallback is the plan;
   worth prototyping early (M3) before committing.
2. **Cell overlap & nesting caps** — proposed reject-on-overlap, `MAX_GRID_DIM=12`, `MAX_CELLS=64`,
   `MAX_NESTING 3→4`. Tunable; validate against real dashboards.
3. **How the model chooses inline vs live-panel** — a prompt/tool-affordance design question, not a
   code one. Needs a short spec of when the model reaches for a panel.
4. **Testing** — unit-test `SessionEngine.update` as pure functions; snapshot-test grid rendering at a
   **pinned size + color profile** (insta + ratatui `TestBackend`); one PTY smoke test for the TUI.
5. **Transcript growth** — live panels that update every turn: record deltas or last-state? Lean
   last-state per panel id to keep replay simple.

---

## 9. Suggested next step

M0 + M1 are independently valuable and carry no rewrite risk. Recommend landing them on this branch
first (they make the grid real and fix the bee), then run `/speckit-specify` on **M3 (the bee-tui
driver)** to turn Section 3 into a full feature spec before the big build.
