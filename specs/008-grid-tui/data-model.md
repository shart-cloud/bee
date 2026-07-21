# Phase 1 Data Model: Full-Screen TUI with Model-Owned Live Grid Panels

The TUI state and message types (The Elm Architecture), the panel/chat entities, and the transcript
event added for replay. Shapes are advisory — field names may shift in implementation — but the
relationships and invariants are the contract. Everything visual reuses the existing pure-serde
`RenderSpec`; nothing here re-styles content.

---

## App (Model)

The single source of truth the `view` renders and `update` mutates. Owned by the event loop; never
shared across threads (messages flow in over a channel).

| Field | Type | Notes |
|---|---|---|
| `chat` | `Vec<ChatMessage>` | ordered conversation history; virtualized on render |
| `panels` | `IndexMap<PanelId, RenderSpec>` | model-owned regions, insertion-ordered (D7) |
| `input` | `InputState` | multi-line buffer + cursor + history (D5) |
| `scroll` | `ScrollState` | chat scroll offset + follow-tail flag |
| `focus` | `Focus` | which region has focus: `Chat` \| `Input` \| `Panels` |
| `layout_mode` | `LayoutMode` | derived from size: `TwoPane` \| `SinglePane` \| `TooSmall` (D11) |
| `size` | `(u16, u16)` | last known `(cols, rows)`; updated on resize |
| `turn` | `TurnState` | `Idle` \| `Streaming { since }` (drives the spinner/shimmer) |
| `theme` | `Theme` | the active `viz::theme` (reused; not re-defined here) |
| `panels_visible` | `bool` | toggle for the overlay in single-pane mode |
| `help_open` | `bool` | whether the `?` keybinding-help overlay is shown |
| `should_quit` | `bool` | set by `update`; the loop exits when true |

**Invariants**
- `panels` never contains two entries with the same `PanelId` (replacement, not duplication — FR-009).
- `layout_mode` is a pure function of `size` and the breakpoints (D11); it is recomputed on `Resize`.
- No field holds a ratatui or crossterm type — the model is renderer-agnostic and unit-testable (D12).

## Message

Everything that can change the model. `update(&mut App, Message)` is pure and total (unit-tested, D12).

| Variant | Payload | Effect |
|---|---|---|
| `Key(KeyEvent)` | key + mods | edits input, scrolls, focuses, submits, quits, yanks (see keybindings contract) |
| `Paste(String)` | pasted text | inserts into input (multi-line safe) |
| `Resize(u16, u16)` | new size | updates `size`, recomputes `layout_mode` |
| `Suspend` / `Resume` | — | leave/re-enter alt-screen; force full redraw on resume |
| `Tick` | — | advance any active animation (armed only while animating) |
| `Session(SessionEvent)` | see below | the conversation core speaking (tokens, tool results, panels) |
| `Quit` | — | set `should_quit` |

## SessionEvent (from the shared `SessionEngine`, D4)

Emitted by the conversation core; consumed by **both** front-ends. Renderer-agnostic.

| Variant | Payload | TUI handling |
|---|---|---|
| `UserEcho(String)` | submitted text | append a `ChatMessage{role: User}` |
| `Token(String)` | streamed delta | append to the open assistant message |
| `ToolCall { name, args }` | | append a tool-call line to chat |
| `ToolResult { render_spec: Option<RenderSpec>, target: RenderTarget }` | | inline → chat message; panel → `panels` upsert |
| `PanelUpdate { id: PanelId, spec: RenderSpec }` | | upsert `panels[id] = spec` (coalesced, FR-011) |
| `Denial(String)` | | append a bold `⚠ DENIED` chat line |
| `TurnStarted` / `TurnDone` | | set `turn` Streaming/Idle |

## ChatMessage

One entry in the transcript view.

| Field | Type | Notes |
|---|---|---|
| `role` | `Role` | `User` \| `Assistant` \| `System` \| `Tool` |
| `body` | `MessageBody` | `Text(String)` or `Widget(RenderSpec)` (inline rendered content) |
| `ts` | `i64` | monotonic order key (display only) |

`Widget(RenderSpec)` renders through the existing `render_into` into the pane width (D6) — the same
path the inline REPL uses, so charts/tables/grids look identical in chat.

## Panel

A named, persistent, model-owned region.

| Field | Type | Notes |
|---|---|---|
| `id` | `PanelId` (`String`, validated) | address; also the border title |
| `spec` | `RenderSpec` | usually a `Grid`; may be any widget |

**Rules**
- `id` is 1–32 chars, `[a-z0-9_-]` (rejected otherwise — fail-closed, matches the API's style).
- Upsert semantics: same `id` replaces `spec`; new `id` appends a panel (FR-008/009).
- A panel whose `spec` exceeds its rendered area is clipped to the region with an indication (FR-012).
- Panels render in insertion order down the right-hand column (TwoPane) or in the overlay (SinglePane).

## RenderTarget

Where a rendered spec goes — the one new concept the model can express.

```
RenderTarget = Inline            // default: flows into chat (today's behavior)
             | Panel(PanelId)    // persistent, replace-in-place
```

## Transcript event (persistence, FR-010 / SC-009)

The episode transcript gains one pure-serde variant so replay reconstructs panel state:

```
PanelUpdate { id: PanelId, spec: RenderSpec }   // last-writer-wins per id on replay
```

**Rationale**: recording last-state-per-id (not deltas) keeps replay a simple fold — apply each
`PanelUpdate` in order, keyed by id — and matches the whole-panel replacement semantics (D7). No
ratatui/rhai types enter the transcript (invariant preserved from `render_spec.rs`).

## Reused, unchanged

- `RenderSpec` (incl. `Grid`/`GridCell` from M1), `viz::palette`/`theme`/`glyph`, `sprite_render`.
- The Rhai drawing engine + sandbox (`render_api`, `tools::render`) — extended only by `render_to`.
- The provider/tool/transcript machinery — relocated behind `SessionEngine`, not rewritten.
