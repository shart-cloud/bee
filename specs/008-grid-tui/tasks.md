---
description: "Task list for Full-Screen TUI with Model-Owned Live Grid Panels"
---

# Tasks: Full-Screen TUI with Model-Owned Live Grid Panels

**Input**: Design documents from `specs/008-grid-tui/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/

**Tests**: Included — the plan's testing strategy (research D12) is a deliverable: a pure-reducer
layer, pinned frame snapshots, a terminal-restore matrix, and a `bee-core` dependency guard. Write the
tests in each story before/with implementation.

**Organization**: grouped by user story (US1 P1 → US2 P2 → US3 P3) for independent delivery.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: parallelizable (different files, no incomplete-task dependency)
- All paths are repo-relative; the feature lives in `bee-harness`.

---

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: dependencies and empty module structure per plan.md.

- [X] T001 [P] In `bee-harness/Cargo.toml`, enable ratatui's `crossterm` backend feature; add `crossterm = "0.28"` (`event-stream`), `color-eyre = "0.6"`, `textwrap`; add `insta` as a dev-dependency; add a `tui` cargo feature that gates the full-screen front-end (research D1/D12).
- [X] T002 [P] Scaffold empty module trees `bee-harness/src/session/{mod.rs,event.rs}` and `bee-harness/src/tui/{mod,app,message,view,panels,chat,input,theme_bridge,term}.rs` with stubs, and register them in `bee-harness/src/lib.rs` (behind the `tui` feature where appropriate).

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: the shared conversation core and rendering helpers every full-screen story needs.

**⚠️ CRITICAL**: no user story can begin until this phase is complete.

- [X] T003 [P] Add pure-serde `RenderTarget` (`Inline` | `Panel(PanelId)`) in `bee-harness/src/render_spec.rs` (data-model.md).
- [X] T004 Define `SessionEvent` enum (`UserEcho`, `Token`, `ToolCall`, `ToolResult{render_spec,target}`, `PanelUpdate`, `Denial`, `TurnStarted`, `TurnDone`) in `bee-harness/src/session/event.rs` (depends on T003).
- [X] T005 Expose the turn loop as an event stream via `session::SessionSink` (a `ReplOutput` that forwards each callback as a `SessionEvent` on a channel) in `bee-harness/src/session/mod.rs`. **Reframed (lower risk):** `repl::run_exchange` already writes exclusively through the `ReplOutput` trait, so it *is* the shared engine — no rewrite of the working loop was needed; the sink is the TUI's consumer. Fidelity unit-tested.
- [X] T006 Inline-REPL parity **by construction**: the inline REPL keeps passing its `TerminalOutput` (a `ReplOutput`) to the unchanged `run_exchange`, so its byte-for-byte output is untouched while the TUI drives the same core via `SessionSink`. No rewrite of `repl.rs` was required (depends on T005).
- [X] T007 [P] Add the theme→ratatui `Style` bridge in `bee-harness/src/tui/theme_bridge.rs`, reusing the color helpers in `viz::buffer_render`; it MUST return an **unstyled** `Style` (no fg/bg) when `NO_COLOR` is set, so the full-screen path stays legible in monochrome exactly like the inline path (research D1; **FR-014, SC-005** — resolves analysis G2).
- [X] T008 [P] Add a truecolor sprite→`Buffer` rasterizer (half-block cells with fg+bg) in `bee-harness/src/viz/buffer_render.rs` so `RenderSpec::Sprite` composites into a sub-rect, plus a unit test that a sprite renders inside a grid cell (research D8; lifts the M1 placeholder).

**Checkpoint**: shared core ready — inline REPL still works via `SessionEngine`; full-screen stories can begin.

---

## Phase 3: User Story 1 — Full-screen session that never corrupts the terminal (Priority: P1) 🎯 MVP

**Goal**: an opt-in full-screen chat surface — streaming replies, scrollable history, persistent input + footer — that restores the terminal on every exit path.

**Independent Test**: run a full session on a capable terminal and exit via quit / Ctrl-C / suspend-resume / panic; each leaves a usable terminal with prior scrollback intact (SC-001), and grids/charts still appear inline in chat.

### Tests for User Story 1

- [X] T009 [P] [US1] Reducer tests (realized as in-crate unit tests in `bee-harness/src/tui/app.rs` — pure, run under `--features tui`): submit, scroll (PgUp/PgDn, `gg`/`G`), focus cycle, quit, resize→`layout_mode` (write to fail first).
- [ ] T010 [P] [US1] Terminal-restore matrix test in `bee-harness/tests/tui_restore.rs`: quit / interrupt / suspend-resume / panic all leave the terminal restored (SC-001, contracts/modes-and-cli.md).

### Implementation for User Story 1

- [X] T011 [P] [US1] `App` model (chat, input, scroll, focus, size, turn, should_quit, theme) in `bee-harness/src/tui/app.rs` (data-model.md).
- [X] T012 [P] [US1] `Message` enum (`Key`, `Paste`, `Resize`, `Suspend`/`Resume`, `Tick`, `Session`, `Quit`) in `bee-harness/src/tui/message.rs`.
- [X] T013 [US1] `update(&mut App, Message)` reducer in `bee-harness/src/tui/app.rs` — key handling, scroll, focus, submit, quit, resize, token append (depends on T011, T012).
- [X] T014 [P] [US1] Minimal multi-line input widget (buffer, cursor, history, multi-line paste) in `bee-harness/src/tui/input.rs` (research D5).
- [X] T015 [P] [US1] Chat model + virtualized visible-slice render + inline-widget rendering via `render_into` in `bee-harness/src/tui/chat.rs` (research D6).
- [X] T016 [US1] `view(&App, &mut Frame)` — header / chat / input / footer hint bar layout in `bee-harness/src/tui/view.rs` (depends on T014, T015; keybindings.md footer).
- [X] T016a [US1] `?` help overlay listing all keybindings, rendered in `bee-harness/src/tui/view.rs` and toggled via a `help_open` flag in the reducer (`bee-harness/src/tui/app.rs`), closable with `?`/`Esc` (**FR-005**; contracts/keybindings.md discoverability ladder — resolves analysis G1) (depends on T013, T016).
- [X] T017 [US1] Terminal lifecycle in `bee-harness/src/tui/term.rs`: alt-screen + raw mode via `ratatui::init`/`restore`, `Drop` guard, `color-eyre` panic hook that restores first, SIGTSTP suspend/SIGCONT redraw (makes T010 pass; contracts/modes-and-cli.md).
- [ ] T018 [US1] Event loop in `bee-harness/src/tui/mod.rs`: `crossterm` `EventStream` + `tokio::select!` over terminal events, the `SessionEvent` channel, resize, and an on-demand tick; redraw once per handled batch (depends on T013, T016, T017; research D3).
- [ ] T019 [US1] Add `--tui`/`--no-tui` flags and launch the full-screen front-end on a capable terminal in `bee-harness/src/bin/bee-repl.rs` (minimal; graceful fallback lands in US3) (depends on T018).
- [X] T020 [P] [US1] Frame snapshot test (header + chat + input + footer) at a pinned size and `NO_COLOR` in `bee-harness/tests/tui_snapshot.rs` (research D12).

**Checkpoint**: US1 is a working full-screen chat MVP, independently testable.

---

## Phase 4: User Story 2 — The model takes over a live panel (Priority: P2)

**Goal**: the model addresses an N×M grid to a named panel that persists beside chat and updates in place across turns; untargeted renders stay inline.

**Independent Test**: render a grid to `render_to("metrics", g)`, then re-render with new values to the same name; exactly one panel exists and shows the latest state while chat is unchanged (SC-002, SC-008); replaying the transcript reconstructs the panel (SC-009).

### Tests for User Story 2

- [ ] T021 [P] [US2] Reducer tests appended to `bee-harness/tests/tui_update.rs`: panel upsert (new vs replace), update coalescing, and inline-vs-panel routing.
- [ ] T022 [P] [US2] Transcript replay test in `bee-harness/tests/panel_replay.rs`: `PanelUpdate` events fold last-writer-wins to each panel's final state (SC-009).

### Implementation for User Story 2

- [ ] T023 [P] [US2] Add pure-serde `PanelUpdate { id, spec }` transcript event in `bee-harness/src/transcript.rs` (data-model.md).
- [ ] T024 [US2] Add `render_to(panel_id, widget)` (with `panel_id` validation) in `bee-harness/src/render_api.rs`, and carry `RenderTarget` through the render tool result in `bee-harness/src/tools/render.rs` (depends on T003; contracts/rhai-panel-api.md).
- [ ] T025 [US2] Route tool results by target in `session::SessionEngine` → emit `ToolResult`/`PanelUpdate` `SessionEvent`s in `bee-harness/src/session/mod.rs` (depends on T024, T005).
- [ ] T026 [P] [US2] Panel registry (`IndexMap<PanelId, RenderSpec>`) with upsert + per-redraw coalescing in `bee-harness/src/tui/panels.rs` (research D7).
- [ ] T027 [US2] Handle `Session(PanelUpdate | ToolResult)` in the reducer → panel upsert or inline chat append in `bee-harness/src/tui/app.rs` (depends on T026, T013).
- [ ] T028 [US2] Render the right-hand panel column (TwoPane) in `bee-harness/src/tui/view.rs`, each panel a bordered block titled with its id, clipped to its area (depends on T027; FR-012).
- [ ] T029 [P] [US2] Add example `specs/003-visual-render/examples/panel-metrics.rhai` using `render_to` (quickstart Scenario B).
- [ ] T030 [P] [US2] Panel snapshot test (a grid in a panel beside chat) in `bee-harness/tests/tui_snapshot.rs`.

**Checkpoint**: US1 + US2 both work independently; the model owns live panels.

---

## Phase 5: User Story 3 — Graceful degradation and honest fallback (Priority: P3)

**Goal**: sensible behavior off a TTY, under `NO_COLOR`, on narrow/tiny terminals, with clipboard copy over SSH.

**Independent Test**: run the same session piped to a file, under `NO_COLOR`, and at 40×10; each yields usable output (linear text / monochrome / "too small" message) rather than a corrupted screen (SC-004, SC-006); `y` copies over a remote session (SC-007).

### Tests for User Story 3

- [ ] T031 [P] [US3] Front-end selection tests in `bee-harness/tests/tui_fallback.rs`: non-tty→inline, `TERM=dumb`→inline, below hard floor→message (contracts/modes-and-cli.md).
- [ ] T032 [P] [US3] Responsive + `NO_COLOR` snapshot tests (narrow single-pane, monochrome) appended to `bee-harness/tests/tui_snapshot.rs`.

### Implementation for User Story 3

- [ ] T033 [US3] Front-end selection + fallback (not-a-tty / `TERM=dumb` / below hard floor) with a one-line stderr note in `bee-harness/src/bin/bee-repl.rs` (extends T019; contracts/modes-and-cli.md).
- [ ] T034 [P] [US3] `LayoutMode` + responsive breakpoints (TwoPane / SinglePane / TooSmall) and the "terminal too small" render in `bee-harness/src/tui/view.rs` (+ recompute `layout_mode` on resize in `tui/app.rs`) (research D11).
- [ ] T035 [P] [US3] OSC-52 yank of the current message bound to `y`, plus reserved-key handling (Ctrl-C→clean quit, Ctrl-Z→suspend) in `bee-harness/src/tui/term.rs` and the reducer (research D9; contracts/keybindings.md).
- [ ] T036 [US3] Single-pane panel overlay toggle (`p`) in `bee-harness/src/tui/view.rs` and the reducer (depends on T034, T028).

**Checkpoint**: all three stories independently functional; feature safe in every environment.

---

## Phase 6: Polish & Cross-Cutting Concerns

- [ ] T037 [P] Add a `bee-core` dependency-guard test (asserts no terminal/async-runtime dep entered the core) in `bee-harness/tests/core_deps_guard.rs` (Constitution V, FR-021).
- [ ] T038 [P] Docs: update milestone status in `docs/grid-tui-plan.md`, note `--tui` in `README.md`, and add the TUI/panel entries to `docs/design-system.md`.
- [ ] T039 [P] Update `specs/003-visual-render/contracts/honeycomb.md` (nesting ≤ 4; note the panel/`RenderTarget` surface).
- [ ] T040 Run the `quickstart.md` scenarios A–D on a capable terminal and record results.
- [ ] T041 Run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and the full `bee-harness` test suite; all green.

---

## Dependencies & Execution Order

### Phase dependencies

- **Setup (P1)**: no dependencies.
- **Foundational (P2)**: after Setup — **blocks all stories**. T003→T004→T005→T006 is a chain; T007, T008 are parallel to it.
- **US1 (P3)**: after Foundational. The MVP.
- **US2 (P4)**: after Foundational; integrates with US1's reducer/view but is independently testable. (T024 needs T003; T025 needs T005.)
- **US3 (P5)**: after Foundational; extends US1's CLI/view. T036 depends on US2's T028.
- **Polish (P6)**: after the desired stories.

### Within a story

- Tests are written first (or alongside) and fail before implementation.
- Model/message (T011,T012) before the reducer (T013); widgets (T014,T015) before the view (T016); lifecycle (T017) before the loop (T018).

### Parallel opportunities

- **Setup**: T001, T002 in parallel.
- **Foundational**: T007 and T008 run parallel to the T003→T006 chain.
- **US1**: T009,T010 (tests) parallel; then T011,T012,T014,T015 parallel; T020 parallel once the view exists.
- **US2**: T021,T022 parallel; T023,T026,T029,T030 parallel where files differ.
- **US3**: T031,T032 parallel; T034,T035 parallel.
- **Polish**: T037,T038,T039 parallel.
- With capacity, **US2 and US3 can proceed in parallel** once US1's reducer/view exist (only T036 crosses into US2).

---

## Parallel Example: User Story 1

```bash
# Tests first (parallel):
Task: "Reducer tests in bee-harness/tests/tui_update.rs"        # T009
Task: "Terminal-restore matrix test in bee-harness/tests/tui_restore.rs"  # T010

# Then the independent building blocks (parallel):
Task: "App model in bee-harness/src/tui/app.rs"                 # T011
Task: "Message enum in bee-harness/src/tui/message.rs"          # T012
Task: "Input widget in bee-harness/src/tui/input.rs"            # T014
Task: "Chat render in bee-harness/src/tui/chat.rs"             # T015
```

---

## Implementation Strategy

### MVP first (US1 only)

1. Phase 1 Setup → 2. Phase 2 Foundational (the `SessionEngine` refactor is the critical blocker) →
3. Phase 3 US1 → **STOP and validate** (quickstart Scenario A + the restore matrix) → demo the
full-screen chat.

### Incremental delivery

- Foundation ready → US1 (full-screen chat MVP) → US2 (live model-owned panels — the headline) →
  US3 (fallback/degradation) → Polish. Each story adds value without breaking the previous.

### Notes

- `[P]` = different files, no incomplete dependency.
- The inline REPL keeps working throughout (US default + fallback); T006 guards parity.
- Commit after each task or logical group; validate at each checkpoint.
