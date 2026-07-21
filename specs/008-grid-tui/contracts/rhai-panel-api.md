# Contract: the panel-addressing surface (`render_to`)

The one new capability the model gains: sending a rendered widget to a **named, persistent panel**
instead of into the chat flow. Extends the existing sandboxed Rhai drawing API (`render_api.rs`);
changes nothing about the sandbox itself. Satisfies FR-008–FR-011; SC-002, SC-008, SC-009.

## Rhai function

```rhai
render_to(panel_id, widget);   // panel_id: String, widget: any renderable (incl. grid())
render(widget);                // unchanged: commits to the INLINE target (chat flow)
```

- `render_to` commits the widget with `RenderTarget::Panel(panel_id)`; `render` commits with
  `RenderTarget::Inline` exactly as today (back-compat — FR-008 scenario 3).
- `panel_id` is validated: 1–32 chars, `[a-z0-9_-]`. A bad id is a **script error** (fail-closed,
  matching the API's existing error style). Empty/oversized → rejected.
- Same structural caps as `render` apply to the widget (nesting ≤ 4, ≤ 500 elements, grid ≤ 12×12 /
  ≤ 64 cells). No new engine capability is registered — only this one drawing function.
- Calling `render_to` multiple times in one script commits multiple targeted specs (the render tool
  already tracks committed specs; it returns a text summary to the model, never pixels).

## Tool-layer surface

The `render` tool result carries the target so the front-end can route it:

```
ToolResult.render_target: RenderTarget   // Inline | Panel(PanelId)   (pure serde)
ToolResult.render_spec:   Option<RenderSpec>
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

## Persistence (FR-010, SC-009)

Each targeted commit records a transcript event `PanelUpdate { id, spec }` (pure serde). Replay folds
these in order, last-writer-wins per id, reconstructing each panel's final state — no live-only state
is required. This preserves the existing invariant that the transcript holds **no ratatui/rhai types**.

## Non-goals (this contract)

- **Cell-level** in-place updates (`panel("m").set_cell(...)`) — out of scope; whole-panel replace only.
- Panel removal/animation APIs — future; a panel persists for the session (or until replaced).
- Any new non-drawing capability on the engine — explicitly none (Constitution I, FR-020).
