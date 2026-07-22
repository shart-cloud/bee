# Implementation Plan: Full-Screen TUI with Model-Owned Live Grid Panels

**Branch**: `008-grid-tui` | **Date**: 2026-07-21 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `specs/008-grid-tui/spec.md`

> **Branch note (analysis I1)**: `008-grid-tui` is the Spec Kit *feature-directory* name and is
> intentionally independent of the git branch. This work currently sits on `008-bee-design-system`
> (alongside the shipped bee redesign + grid M0/M1). At `/speckit-implement` time, decide whether to
> continue on that branch or cut a dedicated `008-grid-tui` branch — the feature-directory name stays
> the same either way.

## Summary

Add an **opt-in full-screen terminal front-end** to the harness that renders a persistent, scrollable
chat history beside **model-owned grid panels** that update live across turns, while the existing
inline REPL remains the default and the automatic fallback. The content layer already exists — the
`RenderSpec` widget/grid model, the sandboxed Rhai `grid()` API, the semantic themes — so this feature
is a **new interactive driver** (alt-screen, raw mode, an event loop, panel ownership) plus a small
**Rhai/tool surface for addressing panels**. A prerequisite refactor extracts a shared
`SessionEngine` so the inline and full-screen front-ends drive the same conversation core.

Technical approach: a ratatui + crossterm application in `bee-harness` following The Elm Architecture
(Model → Message → update → view) over the harness's existing tokio loop; a `HashMap<PanelId,
RenderSpec>` panel registry mutated by a new `render_to(panel, widget)` surface; a truecolor buffer
renderer that composites the existing widgets (and, newly, sprites) into cell regions; and a hard
terminal-restore contract enforced on every exit path.

## Technical Context

**Language/Version**: Rust, edition 2021, workspace `rust-version = 1.85`, **stable** toolchain
(Constitution V — user-space crates build on stable).

**Primary Dependencies**:
- `ratatui = "0.29"` — **enable the `crossterm` backend feature** (currently `default-features = false`
  for headless use; the TUI needs the real backend). Reuse the existing `viz::buffer_render` widget
  mappings.
- `crossterm = "0.28"` (matches ratatui 0.29) with the `event-stream` feature for async input.
- `tokio` (already present) — the event loop and provider streaming.
- `color-eyre = "0.6"` — panic hook that restores the terminal before printing the trace.
- Existing: `viz` (palette/theme/glyph/sprite_render), `render_spec`, `render_api`, provider, transcript.
- New (small, evaluated in research): `textwrap` for chat wrapping; a **custom** minimal multi-line
  input (no `tui-textarea` dependency for v1). `rustyline` (17) stays for the inline front-end.

**Storage**: No new store. Panel state is added to the **episode transcript** as pure-serde
`PanelUpdate { id, spec }` events (reuses the `RenderSpec`-in-transcript design; no ratatui/rhai leaks).

**Testing**: `cargo test` (stable). Unit-test the `SessionEngine`/`App::update` reducer as pure
functions (synthetic message in → state out). Frame snapshots via ratatui `TestBackend` at a **pinned
size + color profile** (add `insta` as a dev-dependency, or inline expected-buffer assertions). One
optional PTY smoke test. Existing `render_sandbox`/`grid` suites must stay green.

**Target Platform**: Linux terminal (the project's platform). Full-screen requires an interactive TTY
and a capable terminal; otherwise the inline path runs (FR-013).

**Project Type**: CLI / terminal application — a harness **front-end**, not a new crate.

**Performance Goals**: Event-driven — **no fixed-timer redraw**; an idle session does zero work
(SC-003). A panel update is visible within **200 ms** (SC-002). Any animation is capped at ≤ 60 fps and
only runs while active.

**Constraints**:
- **Terminal restored on 100% of exit paths** — quit, Ctrl-C, Ctrl-Z/resume, panic (SC-001, FR-002).
- **No async-runtime or terminal dependency in `bee-core`** — the driver lives only in `bee-harness`
  (Constitution V, FR-021).
- Reuse the existing visual vocabulary; honor `NO_COLOR`; handle resize/suspend; min-size message.
- Do not weaken the render sandbox or any enforcement behavior (FR-020).

**Scale/Scope**: One local or remote operator; chat history virtualized (render only the visible
slice); grids ≤ 12×12 / ≤ 64 cells (existing caps); a handful of concurrent panels.

## Constitution Check

*GATE: must pass before Phase 0. Re-checked after Phase 1 (below).*

| Principle | Assessment |
|---|---|
| **I. Deny-by-Default & Fail-Closed** | Not touched. The render script sandbox and policy enforcement are unchanged (FR-020). The TUI renders already-produced, already-validated `RenderSpec` data; it grants no new capability. **PASS.** |
| **II. Capability Attenuation** | Not touched — no policy derivation in a presentation layer. **PASS.** |
| **III. Kernel Enforcement Is Authoritative** | Not touched — no enforcement moves to user space. **PASS.** |
| **IV. Policy-as-Data** | Not touched. Panel/transcript state is declarative pure-serde data, diffable and replayable (FR-010). **PASS.** |
| **V. Library-First, Runtime-Free Core** | **Directly relevant.** The driver and panel logic live in `bee-harness` (already the async layer); `bee-core` gains **no** terminal or async-runtime dependency (FR-021). The `--tui` front-end is a thin wrapper over the shared `SessionEngine`, holding no logic the library lacks. Guarded by a test asserting `bee-core` has no new deps. **PASS.** |

**Development Workflow gates**: no security-boundary behavior changes, so the "test-first for
enforcement" gate is N/A here; the existing enforcement tests must remain green. Frame snapshots are
pinned to size + color profile. Terminal-restore is a hard, tested requirement (SC-001).

**Result: PASS — no violations. Complexity Tracking is empty.**

## Project Structure

### Documentation (this feature)

```text
specs/008-grid-tui/
├── plan.md              # This file
├── research.md          # Phase 0 — decisions (framework, event loop, panels, copy, testing)
├── data-model.md        # Phase 1 — App/Model, Message, Panel, ChatMessage, transcript events
├── quickstart.md        # Phase 1 — runnable validation scenarios
├── contracts/           # Phase 1 — the surfaces this feature exposes
│   ├── modes-and-cli.md #   the --tui flag, auto-detect, fallback + terminal-restore contract
│   ├── rhai-panel-api.md#   render_to(panel, widget) + panel semantics + transcript event
│   └── keybindings.md   #   the key map + reserved-key contract
└── checklists/
    └── requirements.md  # from /speckit-specify
```

### Source Code (repository root)

This is a Rust workspace; the feature is contained in **`bee-harness`** and reuses existing modules.

```text
bee-harness/
├── src/
│   ├── session/            # NEW (plan M2 prerequisite): shared conversation engine
│   │   ├── mod.rs          #   SessionEngine — drives provider/tools/transcript, emits SessionEvent
│   │   └── event.rs        #   SessionEvent enum (Token, ToolResult, PanelUpdate, Denial, TurnDone…)
│   ├── tui/                # NEW: the full-screen front-end (M3–M4)
│   │   ├── mod.rs          #   entry: init/restore terminal, run the event loop
│   │   ├── app.rs          #   App (Model) + update(Message) reducer (pure, unit-tested)
│   │   ├── message.rs      #   Message enum (Key, Resize, Suspend, Tick, Session(SessionEvent)…)
│   │   ├── view.rs         #   view(&App, &mut Frame): layout + render regions
│   │   ├── panels.rs       #   panel registry + addressing + coalescing
│   │   ├── chat.rs         #   chat history model + virtualized scroll + inline-widget rendering
│   │   ├── input.rs        #   minimal multi-line input widget
│   │   ├── theme_bridge.rs #   viz::theme roles → ratatui Style (reuse buffer_render helpers)
│   │   └── term.rs         #   alt-screen/raw-mode lifecycle, panic hook, suspend, OSC-52 copy
│   ├── repl.rs             # MODIFIED: inline front-end now consumes SessionEngine (fallback path)
│   ├── viz/
│   │   ├── buffer_render.rs# REUSED + extended: sprite→Buffer rasterizer (truecolor cells)
│   │   └── …               # palette/theme/glyph/sprite_render unchanged
│   ├── render_spec.rs      # REUSED: RenderSpec::Grid (M1) already present
│   ├── render_api.rs       # MODIFIED: add render_to(panel, widget) (inline render() unchanged)
│   ├── bin/
│   │   └── bee-repl.rs     # MODIFIED: add --tui / --no-tui; auto-detect tty → pick front-end
│   └── transcript.rs       # MODIFIED: record PanelUpdate events (pure serde)
└── tests/
    ├── tui_update.rs       # NEW: pure reducer tests (key handling, scroll, panel replace/coalesce)
    ├── tui_snapshot.rs     # NEW: TestBackend frame snapshots at pinned size + NO_COLOR
    └── grid.rs, render_sandbox.rs, … # existing, must stay green
```

**Structure Decision**: Single workspace, feature isolated in `bee-harness`. Two new module trees
(`session/`, `tui/`) plus small, additive edits to `repl.rs`, `render_api.rs`, `buffer_render.rs`,
`transcript.rs`, and the `bee-repl` binary. No new crate; no change to `bee-core`/`bee-ebpf`/policy
crates. The `--tui` flag on the existing `bee-repl` binary is preferred over a separate `bee-tui`
binary so session configuration and the fallback live in one place (revisited in research).

## Post-Design Constitution Re-Check

*Re-evaluated after Phase 1 (research + data-model + contracts).*

The design keeps every security-relevant concern out of the new surface:

- **Principle V (runtime-free core)** — held and now *mechanically guarded*: the `session/` and `tui/`
  trees live only in `bee-harness`; the plan adds a **dependency-guard test** asserting `bee-core`'s
  dep set is unchanged (research D12, quickstart automated checks). The `render_to` addition registers
  **one** drawing function and no new engine capability (contracts/rhai-panel-api.md).
- **Principle I (fail-closed)** — the panel API validates `panel_id` and reuses the existing structural
  caps; a bad id or oversized widget is a script error, not a silent pass. The sandbox is untouched.
- **Principle IV (policy-as-data)** — panel state is recorded as pure-serde `PanelUpdate` events;
  replay is a deterministic fold, no ratatui/rhai types enter the transcript (data-model.md).
- **Principles II & III** — unaffected; no policy derivation, no enforcement in user space.

Design added no new dependency to any policy/kernel crate; all additions are `bee-harness`-local
(research "dependency delta"). **Re-check result: PASS — Complexity Tracking remains empty.**

## Complexity Tracking

*No Constitution violations — section intentionally empty.*
