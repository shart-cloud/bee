# Phase 0 Research: Full-Screen TUI with Model-Owned Live Grid Panels

Consolidated decisions. Inputs: the codebase architecture survey (inline REPL, headless ratatui,
Rhai render API), the `ratatui-tui` + `tui-design` skills, and `docs/grid-tui-plan.md`. No open
`NEEDS CLARIFICATION` remained from the spec; the items below resolve the technical unknowns.

---

## D1 — TUI framework & backend

**Decision**: Stay on **`ratatui = "0.29"`**, enabling its `crossterm` backend feature; add
`crossterm = "0.28"` with `event-stream`. Drive the terminal with `ratatui::init()` / `ratatui::restore()`
plus an explicit panic hook.

**Rationale**: ratatui is already a workspace dependency (used headlessly for `buffer_render`);
enabling the backend reuses every existing widget mapping with zero content-layer churn. crossterm
0.28 is the version ratatui 0.29 pins. Staying on 0.29 avoids the `unicode-width` pin cascade
documented in `bee-harness/Cargo.toml:50` (rustyline 17 ↔ ratatui 0.29 ↔ unicode-width 0.2.0).

**Alternatives considered**: Bump to ratatui 0.30.1 for `ratatui::run()` (built-in panic-restore) —
**deferred**: it re-opens the unicode-width/rustyline pin and isn't needed to ship. Textual/Bubble
Tea/Ink — wrong language. A raw crossterm app without ratatui — throws away the widget layer.

## D2 — Terminal lifecycle & panic safety (SC-001, FR-002)

**Decision**: A `term.rs` guard owns alt-screen + raw mode via crossterm. Install `color-eyre` and a
panic hook that **restores the terminal first**, then prints the report. A `Drop` guard covers normal
and `?`-early-return paths; the panic hook covers panics.

**Rationale**: The `tui-design` non-negotiable #2. The current inline path has no raw mode to restore,
so this is net-new and must be tested across quit / Ctrl-C / panic / suspend (SC-001).

**Alternatives**: Rely only on `Drop` — misses panics. Rely only on the panic hook — misses early
returns. Both together is the idiom.

## D3 — Event loop & integration with the async provider

**Decision**: TEA loop using `crossterm::event::EventStream` + `tokio::select!` over: (a) terminal
events (key/mouse/resize), (b) a `SessionEvent` channel from the `SessionEngine` (streamed tokens,
tool results, panel updates, denials, turn-done), (c) an **on-demand** animation tick (only armed when
something is animating — no idle redraw, SC-003). Redraw once per handled batch of messages.

**Rationale**: The harness is already tokio-based and streams from the provider; a channel of
`SessionEvent` cleanly decouples the conversation core from the view. No fixed-timer redraw keeps idle
CPU at ~0 (SC-003).

**Alternatives**: A blocking `event::read()` in a thread — fights the async provider. A fixed 60 fps
tick — violates SC-003.

## D4 — Shared session core (plan M2 prerequisite)

**Decision**: Extract the provider/tool/transcript turn logic out of `repl.rs::run_repl` into
`session::SessionEngine`, which emits `SessionEvent`s over a channel. Both front-ends consume it: the
inline REPL renders events as lines; the TUI renders them into regions.

**Rationale**: Without this, the two front-ends duplicate the turn loop and drift. Isolating it also
lets the reducer be unit-tested without a terminal. This is sequenced **first** in tasks.

**Alternatives**: Copy the loop into the TUI — rejected (drift, double-maintenance, violates the
"thin wrapper" spirit of Constitution V).

## D5 — Input widget

**Decision**: A **custom minimal multi-line input** in `input.rs` (buffer + cursor + history +
multi-line paste), rendered as a ratatui `Paragraph` in the input region. `rustyline` stays for the
inline front-end only.

**Rationale**: Full keybinding control, no new dependency, and we only need a small subset. Avoids
pulling `tui-textarea` (and its own version constraints) for v1.

**Alternatives**: `tui-textarea` — richer (undo, selection) but a new dep with its own ratatui pin;
revisit if editing needs grow. Reuse rustyline inside alt-screen — rustyline owns its own line
discipline and fights raw-mode ratatui.

## D6 — Chat history: rendering & virtualization (FR-006, edge: long conversations)

**Decision**: Chat is a `Vec<ChatMessage>`; each message is pre-wrapped to the pane width with
`textwrap` into `Line`s. Maintain a scroll offset and render **only the visible slice**. Inline widgets
(a message carrying a `RenderSpec`) render through the existing `viz::buffer_render::render_into` into
the pane sub-rect.

**Rationale**: Virtualization keeps long histories responsive (edge case). Reusing `render_into` means
charts/tables/grids in the chat flow look identical to the inline path.

**Alternatives**: Render all messages every frame — O(history) per frame, janky. A third-party log
widget — unnecessary given the existing renderer.

## D7 — Panel registry, addressing & coalescing (FR-008–FR-011, SC-002/008)

**Decision**: `App` holds `panels: IndexMap<PanelId, RenderSpec>` (insertion-ordered). A new Rhai
`render_to(name, widget)` (and an optional `panel` field on the render tool) routes a spec to a panel;
plain `render(widget)` stays **inline** (back-compat, FR-008/scenario 3). Re-rendering an existing id
**replaces** the spec in place (FR-009). The event loop **coalesces**: multiple `PanelUpdate`s for the
same id between redraws collapse to the last (FR-011). Panels lay out in a right-hand column, each a
bordered block titled with its id.

**Rationale**: A keyed map gives "take over a region" with in-place replacement and no duplicates
(SC-008). Whole-panel replacement is the simplest correct semantics (cell-level updates are out of
scope per spec Assumptions).

**Alternatives**: A list of panels without ids — can't address/replace. Cell-level diffing — deferred.

## D8 — Sprites inside cells/panels (truecolor)

**Decision**: In the TUI's real backend, add a **sprite → `Buffer` rasterizer**: set each cell's symbol
to a half-block (`▀▄█`) with truecolor `fg`/`bg`, so `RenderSpec::Sprite` renders properly **inside**
grid cells and panels (unlike the inline basic-ANSI path, where nested sprites show a placeholder).

**Rationale**: The full-screen backend carries per-cell `fg`+`bg` natively, so the half-block technique
composites into a `Buffer` region. This lifts the M1 limitation (sprite-in-cell placeholder) for the
TUI and lets a bee/status icon live in a panel.

**Alternatives**: Keep the placeholder — worse UX. Route sprites through the separate `sprite_render`
ANSI path — can't composite into a sub-region of a shared frame.

## D9 — Copy / clipboard over SSH (FR-016, SC-007)

**Decision**: Emit **OSC 52** directly (base64 of the selection) — no crate — bound to `y` (yank
current message). Document the tmux `set-clipboard on` passthrough. Mouse selection note: alt-screen
capture is bypassable with Shift in most emulators; document it.

**Rationale**: OSC 52 is interpreted by the local emulator, so it works over SSH where `pbcopy`/`xclip`
can't. It's a one-line escape; a clipboard crate (`arboard`) adds deps and fails over SSH.

**Alternatives**: `arboard`/`copypasta` — local-only, heavier. No copy — fails FR-016.

## D10 — Mode selection & fallback (FR-013, FR-018)

**Decision**: `bee-repl` gains `--tui` / `--no-tui`. Default = **inline** (opt-in TUI). Even with
`--tui`, fall back to inline when stdout is not a TTY, `TERM=dumb`, or the terminal is below the
minimum size at startup (with a message). Keep the single `bee-repl` binary rather than a `bee-tui`
binary so config + fallback live together.

**Rationale**: Opt-in first (spec Assumptions); one binary keeps session setup and the fallback in one
place. Auto-detection prevents a broken screen in non-interactive contexts (FR-013).

**Alternatives**: Separate `bee-tui` binary — duplicates arg parsing/config. TUI-by-default — premature
(spec defers it).

## D11 — Responsive layout (FR-015, SC-006)

**Decision**: Breakpoints on terminal width — **≥ 120 cols**: chat + panel column side by side;
**80–119**: single chat pane, panels reachable via a toggle key (overlay); **< 80** or **height < 24**:
single pane, panels overlay-only; **below a hard floor (e.g. 40×10)**: render only a "terminal too
small" message. Re-evaluated on every resize.

**Rationale**: The `tui-design` "pressure-test the floor" reflex. A grid pane needs width; collapsing
to a toggle/overlay degrades gracefully instead of truncating.

**Alternatives**: Fixed two-pane — breaks on narrow terminals. No floor message — a broken layout.

## D12 — Testing strategy

**Decision**: Three layers. (1) **Pure reducer tests** (`tui_update.rs`): feed synthetic `Message`s,
assert `App` state (scroll, focus, panel replace, coalesce, quit). (2) **Frame snapshots**
(`tui_snapshot.rs`) via ratatui `TestBackend` at a **pinned size and `NO_COLOR`**, asserting the buffer
(inline expected-cells or `insta`). (3) One optional **PTY smoke** test. Keep the existing
`render_sandbox` and `grid` suites green. Add a guard test that `bee-core`'s dependency set is
unchanged (Constitution V).

**Rationale**: The reducer layer is the cheapest and catches the "Tab silently broke" class; pinned
snapshots avoid CI flake; a bee-core dep guard enforces FR-021 mechanically.

**Alternatives**: PTY-only e2e — slow, flaky. No snapshots — regressions in layout go unnoticed.

---

## Dependency delta (summary)

| Crate | Change | Why |
|---|---|---|
| `ratatui` | enable `crossterm` feature (was headless) | real terminal backend |
| `crossterm` | **add** `0.28` (`event-stream`) | async input, alt-screen, raw mode |
| `color-eyre` | **add** `0.6` | panic hook that restores the terminal |
| `textwrap` | **add** (small) | chat wrapping to pane width |
| `insta` | **add** dev-dep (optional) | pinned frame snapshots |
| `bee-core` | **no change** | Constitution V — runtime-free core stays clean |

All additions live in `bee-harness` only. No new crate; stable toolchain preserved.
