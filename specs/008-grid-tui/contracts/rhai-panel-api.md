# Contract: the panel-addressing surface (`render_to`)

The one new capability the model gains: sending a rendered widget to a **named, persistent panel**
instead of into the chat flow. Extends the existing sandboxed Rhai drawing API (`render_api.rs`);
changes nothing about the sandbox itself. Satisfies FR-008–FR-011; SC-002, SC-008, SC-009.

## Rhai function

```rhai
render(widget);                            // unchanged: commits INLINE (chat flow)
render_to(panel_id, widget);               // create or replace a named panel
render_to_ttl(panel_id, widget, ttl_ms);   // same, but the panel auto-expires
remove_panel(panel_id);                    // close one panel, reclaim its space
clear_panels();                            // close every panel
```

- `render_to` commits the widget with `RenderTarget::Panel(panel_id)`; `render` commits with
  `RenderTarget::Inline` exactly as today (back-compat — FR-008 scenario 3).
- `panel_id` is validated: 1–32 chars, `[a-z0-9_-]`. A bad id is a **script error** (fail-closed,
  matching the API's existing error style). Empty/oversized → rejected.
- Same structural caps as `render` apply to the widget (nesting ≤ 4, ≤ 500 elements, grid ≤ 12×12 /
  ≤ 64 cells). No new engine capability is registered — only this one drawing function.
- Calling `render_to` multiple times in one script commits multiple targeted specs, and inline +
  panel output coexist in one script (the tool returns a text summary to the model, never pixels).
- `remove_panel` on an absent id is a no-op, not an error. `ttl_ms` must be > 0 and is clamped to 24h.

## Fit checks (fail-closed on an undisplayable surface)

The render tool has no screen of its own, so the front-end publishes its real drawable regions
(`viz::viewport`) on startup and every resize. A commit that the surface cannot display is a **script
error**, so the model gets an actionable signal instead of a cheerful "Rendered …" over invisible
output:

| Condition | Result |
|---|---|
| terminal below the hard floor (40×10), full-screen only | every render fails — "ask the operator to enlarge the terminal" |
| widget's minimum width > chat pane | `render` fails, naming the needed vs available columns |
| widget's minimum width > panel interior | `render_to` fails, suggesting `render()` inline where there is more room |
| new panel id when the column has no free slot | `render_to` fails, naming `remove_panel`/`clear_panels` and the reusable live ids |

Updating an **existing** panel never needs a free slot. The inline REPL constrains **width only** (it
scrolls, so height is unbounded), and a panel render there falls back into the chat flow. When the
viewport is *unconstrained* — headless episodes, batch runs, tests, piped output — **no fit check
fires at all**, so non-interactive runs are unaffected.

## Tool-layer surface

The `render` tool result carries the target so the front-end can route it:

```
ToolResult.render_spec: Option<RenderSpec>   // the INLINE widget, if any
ToolResult.panel_ops:   Vec<PanelOp>         // ordered panel effects (pure serde)
                                             //   Upsert { id, spec, ttl_ms } | Remove { id } | Clear
ToolResult.render_target: RenderTarget       // legacy; pre-lifecycle transcripts still replay
```

The model's text summary reads e.g. `rendered a 3×4 grid to panel "metrics"` so the model knows where
it went.

## Panel semantics (front-end)

| Action | Result |
|---|---|
| `render_to("m", g)` with `"m"` absent | a new panel `m` appears (FR-008) |
| `render_to("m", g2)` with `"m"` present | panel `m`'s content is **replaced in place** — no duplicate, no scrollback churn (FR-009, SC-008) |
| rapid `render_to("m", …)` between redraws | **coalesced** to the last spec before the next draw (FR-011) |
| widget exceeds the panel area | clipped to the region with an indication, never drawn outside (FR-012) |
| `remove_panel("m")` / `clear_panels()` | the panel(s) close and the column reflows |
| a panel's `ttl_ms` elapses | it is pruned on the next redraw; the loop wakes so it expires on time |
| more panels than the column can fit | those that fit render at **content height**; a `+N more` note names the escape hatch |

## Persistence (FR-010, SC-009)

Each panel effect is recorded in the transcript as a `PanelOp` (pure serde) on the tool result.
Replay folds them in order — upserts last-writer-wins per id, removes and clears take effect —
reconstructing each panel's final state; no live-only state is required. TTLs are **not** applied on
replay (expiry is live wall-clock state), so a replayed panel shows its last recorded content. This preserves the existing invariant that the transcript holds **no ratatui/rhai types**.

## Non-goals (this contract)

- **Cell-level** in-place updates (`panel("m").set_cell(...)`) — out of scope; whole-panel replace only.
- Panel *animation* APIs — future. (Panel **removal** was originally deferred here; live use showed
  panels accumulating unbounded with no model-side way to reclaim space, so the lifecycle ops above
  were added.)
- Any new non-drawing capability on the engine — explicitly none (Constitution I, FR-020).
