---
description: "Task list for Terminal Effects & Agent Visual Permissions"
---

# Tasks: Terminal Effects & Agent Visual Permissions

**Input**: Design documents from `specs/009-tachyonfx-effects/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/, quickstart.md

**Tests**: Included — every contract in `contracts/` ends with a "Test obligations" table, and the
spec carries 12 measurable success criteria (SC-001…SC-012). These are deliverables, not optional
extras. Write each story's tests with its implementation.

**Organization**: grouped by user story (US1 P1 → US2 P2 → US3 P2 → US4 P3 → US5 P3) for independent
delivery. Motion control (FR-006a–d) is **not** a user story — its config plumbing and resolver
chokepoint live in Foundational, because every later phase must route through them, and retrofitting
a kill switch across established call sites is the exact failure mode FR-006c exists to prevent.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: parallelizable (different files, no incomplete-task dependency)
- All paths are repo-relative; the feature lives entirely in `bee-harness`.

---

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: the dependency bump (plan "Phase 0") and empty module structure.

- [X] T001 In `bee-harness/Cargo.toml`, bump `ratatui` `0.29` → `0.30` and `crossterm` `0.28` → `0.29`; leave `rustyline` pinned at `17`; leave the `tui` feature list unchanged (`"ratatui/crossterm"`, **no** `crossterm_0_28` — it cannot hold the version, research R2/D1). Manifest-only: no source file changes are expected.
- [X] T002 Verify the Phase 0 gate: `cargo check -p bee-harness --features tui` (0 errors, 0 warnings), `cargo check -p bee-harness` (headless), `cargo test -p bee-harness --features tui` (**291 passed, 0 failed, 28 suites**), and `cargo tree -p bee-harness --features tui -d | grep crossterm` returning **no output**. Any behavior change means Phase 0 grows before Phase 2 starts (depends on T001).
- [X] T003 [P] Add `tachyonfx = { version = "0.25", default-features = false, optional = true }` to `bee-harness/Cargo.toml` and add `"dep:tachyonfx"` to the `tui` feature list, keeping it out of the headless build (SC-009, Constitution V) (depends on T002).
- [~] T004 [P] Scaffold empty modules with stubs and register them: `bee-harness/src/render_spec/effect_spec.rs` and `bee-harness/src/render_api/effect_api.rs` (both **ungated**, headless-compiling), plus `bee-harness/src/tui/{effects,overlay,visual_gate}.rs` (all behind the `tui` feature), wired into `render_spec.rs`, `render_api.rs`, and `tui/mod.rs`. **PARTIAL**: `render_spec/effect_spec.rs` and `tui/effects.rs` exist and are registered. `render_api/effect_api.rs`, `tui/overlay.rs`, `tui/visual_gate.rs` are created in their own phases rather than stubbed up front — an empty module that compiles is not obviously better than no module.

**Checkpoint**: ratatui 0.30 + crossterm 0.29 green on the existing suite; tachyonfx available behind `tui`; module skeleton in place.

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: the serde layer, the configuration axes, and the single resolver chokepoint that every
user story routes through.

**⚠️ CRITICAL**: no user story can begin until this phase is complete.

- [X] T005 [P] Define `EffectSpec` (12 variants, internally tagged on `kind`, snake_case) and `Direction` in `bee-harness/src/render_spec/effect_spec.rs`. Pure serde, **no tachyonfx import**, compiles in the headless build (data-model.md, contracts/effect-spec.md).
- [X] T006 [P] Add the `Renderable { spec: RenderSpec, effect: Option<EffectSpec> }` wrapper in `bee-harness/src/render_spec.rs`, keeping `RenderSpec`'s existing serde shape byte-compatible so 003-visual-render transcripts still deserialize unchanged (data-model.md).
- [X] T007 [P] Extend `RenderTarget` in `bee-harness/src/render_spec.rs` with `Overlay { ttl_ms: Option<u32> }` alongside the existing `Inline` and `Panel` variants (FR-013a, data-model.md).
- [X] T008 [P] Unit-test the `EffectSpec` serde contract in `bee-harness/src/render_spec/effect_spec.rs`: round-trip every variant, assert `ms` clamps to `100..=2000` on **deserialize** as well as construction, assert unknown `kind` is a deserialization **error** (not a silent no-op), and assert an absent `effect` field means "default transition", not "no animation" (contracts/effect-spec.md).
- [X] T009 [P] Add `VisualLevel` (`None` | `Panels` | `PanelsWide` | `Takeover`, `#[derive(Ord)]` with variant order = permission order, `Default` = `Panels`) to `bee-harness/src/config.rs`, parsed from `[harness] visual_level`, `--visual-level`, and `BEE_VISUAL_LEVEL` with CLI > env > scenario > default precedence. An unrecognized value is a **hard startup error**, never a fallback (FR-007, FR-008, Constitution I).
- [X] T010 [P] Add `MotionSetting` to `bee-harness/src/config.rs` from `[harness] animations`, `--no-animation`, and `BEE_NO_ANIMATION` (presence-is-truth, mirroring `viz/palette.rs:28`'s `NO_COLOR` handling), same precedence order, default enabled (FR-006a).
- [X] T011 [P] Add `TakeoverConfig { ttl_secs }` to `bee-harness/src/config.rs` from `[harness] takeover_ttl_secs`, default 30. A value above the 120s maximum is a **hard startup error**, not a clamp — unlike an agent request, which clamps silently (FR-017, FR-021).
- [X] T012 Unit-test config precedence and fail-closed behavior in `bee-harness/src/config.rs`: CLI beats env beats scenario for all three axes; unknown `visual_level` errors; `takeover_ttl_secs = 600` errors; defaults are `panels` / animations-on / 30s (FR-007, FR-008, FR-006a, US2 §5) (depends on T009, T010, T011).
- [X] T013 Establish the resolver chokepoint `fn resolve(spec: &EffectSpec, ctx: &ResolveCtx) -> Option<tachyonfx::Effect>` in `bee-harness/src/tui/effects.rs`, with `ResolveCtx` carrying the active theme, target `Rect`, and the three presentation axes. Return `None` (meaning "render final content, register nothing") when animations are disabled, when the area is 0×0, or when `visual_level == None`; per-variant `None` under `NO_COLOR` for the four color variants. Variant→tachyonfx mapping arrives in T016 (FR-006b, FR-006d, FR-024, research R5) (depends on T003, T005, T010).
- [X] T014 Own a `tachyonfx::EffectManager<String>` in `bee-harness/src/tui/effects.rs` and expose `is_running()`, `add(key, fx)` routed through `unique(key, fx)`, and `process(dt, buf, area)` over `process_effects` (FR-001, FR-005, research R4) (depends on T013).

**Checkpoint**: serde layer compiles headless with no tachyonfx; all three config axes parse and fail closed; the one chokepoint every effect must pass through exists. ✅ **REACHED** — 319 tui / 242 headless tests, clippy clean, fmt clean, `tachyonfx` absent from `bee-core`, `bee-common`, and the headless `bee-harness` build.

> **Implementation stopped here.** Phases 1–2 are complete, tested, and committed
> (`1d34c95`, `566f48c`). Phase 3 begins the integration work described below — it changes
> `view()`'s signature to take `&mut App`, reworks the fixed 120ms tick in `tui/mod.rs` into the
> three-state scheduler, and threads `Effects` through `App`. That is a coherent chunk of work best
> started fresh rather than half-wired.

---

## Phase 3: User Story 1 — Panels fade in and data updates sweep across (Priority: P1) 🎯 MVP

**Goal**: panel creation fades in and panel replacement dissolves-then-coalesces, automatically, with
no Rhai API change and no model involvement. Includes the motion kill switch, which must be proven
in the same increment that first registers effects.

**Independent Test**: render a `RenderSpec::BarChart` to a new panel, advance the `EffectManager` by
simulated ticks, and assert the buffer holds intermediate values at t=150ms and final values at
t=300ms; then upsert new content and assert the dissolve/coalesce character mutations.

### Tests for User Story 1

- [ ] T015 [P] [US1] Add a simulated-clock test harness in `bee-harness/src/tui/effects.rs` that advances `process(dt, buf, area)` in fixed steps and captures `Buffer` snapshots at named offsets, so SC-001/SC-002/SC-008 assert against deterministic frames (tachyonfx ships a seeded `SimpleRng` and no `rand` dep, research R4).

### Implementation for User Story 1

- [ ] T016 [US1] Implement the 12 `EffectSpec` → tachyonfx mappings in `bee-harness/src/tui/effects.rs`: 10 direct (`fade_from`/`fade_to`, `coalesce`/`dissolve`, `slide_in`/`slide_out`, `sweep_in`/`sweep_out`, `evolve_into`/`evolve_from` with `EvolveSymbolSet::BlocksHorizontal`) and 2 **composites** — `Pulse` as `sequence([fade_to_fg, fade_from_fg])` and `Glow` as `ping_pong(lighten)`. Map `Direction` → `tachyonfx::Motion`; supply house values for the `gradient_length` and `randomness` args the Rhai surface does not expose (research R3, contracts/rhai-effect-api.md) (depends on T013).
- [ ] T017 [US1] Add `EffectSlot { prev: Option<Buffer>, started: Option<Instant> }` to `Panel` in `bee-harness/src/tui/panels.rs`. It holds only the outgoing-content snapshot — cancellation belongs to `EffectManager::unique`, keyed by panel name, not to a handle stored here (data-model.md, research R4) (depends on T014).
- [ ] T018 [US1] On first upsert of a new panel name in `bee-harness/src/tui/panels.rs`, register the default entrance effect (fade-in, 200–400ms, fading from the theme's `info` role) (FR-003) (depends on T016, T017).
- [ ] T019 [US1] On upsert to an existing panel name in `bee-harness/src/tui/panels.rs`, snapshot the outgoing buffer into `EffectSlot::prev` and register the update transition as `parallel([dissolve(prev), coalesce(new)])`, 300–500ms (FR-004) (depends on T018).
- [ ] T020 [US1] Call `EffectManager::process` in `bee-harness/src/tui/view.rs` **after** widgets render to the `Buffer` and **before** the buffer is flushed to the screen (FR-001) (depends on T014).
- [ ] T021 [US1] Implement the three-state scheduler in the `select!` loop in `bee-harness/src/tui/mod.rs`: `is_running()` → 16ms; else overlay active → 1s; else `None` (pure event-driven). The overlay arm is unreachable until US3 but the shape lands here (FR-002).
- [ ] T022 [P] [US1] Snapshot-test the panel entrance in `bee-harness/src/tui/effects.rs`: at t=0 the region matches the background, at t=150ms cells hold intermediate colors, at t=300ms cells match the widget's final colors (SC-001) (depends on T018, T015).
- [ ] T023 [P] [US1] Snapshot-test the panel update in `bee-harness/src/tui/effects.rs`: t=0 old content, t=200ms some cells are substituted glyphs, t=400ms new content (SC-002) (depends on T019, T015).
- [ ] T024 [P] [US1] Test transition cancellation in `bee-harness/src/tui/panels.rs`: two upserts 50ms apart to the same panel leave exactly one active effect, starting from the current buffer state — no visual stacking (FR-005, US1 §3) (depends on T019).
- [ ] T025 [P] [US1] Test `NO_COLOR` degradation in `bee-harness/src/tui/effects.rs`: character-based effects still mutate cells while `.fg`/`.bg` are untouched and no ANSI color escape is emitted (FR-006, SC-010, US1 §4) (depends on T016).
- [ ] T026 [P] [US1] Test idle scheduling in `bee-harness/src/tui/mod.rs`: a session with no active effects and no overlay performs **zero** periodic redraws, reusing 008's redraw counter (SC-003) (depends on T021).
- [ ] T027 [P] [US1] Test the motion kill switch in `bee-harness/src/tui/effects.rs`: with `BEE_NO_ANIMATION=1`, panel create and panel replace perform zero periodic redraws, every snapshot at t=0 already shows final content, and the 60fps state is never entered (FR-006b, FR-006c, SC-011) (depends on T013, T019, T021).
- [ ] T028 [P] [US1] Test axis composition in `bee-harness/src/tui/effects.rs`: `NO_COLOR=1` + animations on + `visual_level = "none"` yields chrome text effects that still mutate cells, no color escapes, and no panels — one test asserting all three (FR-006d, SC-012) (depends on T025, T027).

**Checkpoint**: the TUI is visibly alive on panel create/replace, degrades correctly under `NO_COLOR`, and goes completely still under `BEE_NO_ANIMATION` without ever ticking. Shippable on its own.

---

## Phase 4: User Story 2 — The operator configures how much screen the agent can use (Priority: P2)

**Goal**: `visual_level` bounds what the agent may claim, downgrading rather than failing, with a note
the model can read and adapt to.

**Independent Test**: set `visual_level = "panels"`, have the model attempt a full-screen render, and
verify it lands as a panel with a downgrade note in the tool summary.

### Implementation for User Story 2

- [ ] T029 [US2] Implement the downgrade matrix in `bee-harness/src/tui/visual_gate.rs` over `(VisualLevel, RenderTarget)`, running in the session event handler **before** any panel or overlay state mutates. Emit the exact note strings from the contract (contracts/visual-levels.md, FR-009) (depends on T009, T007).
- [ ] T030 [US2] Enforce the width caps in `bee-harness/src/tui/visual_gate.rs`: ⌊w/3⌋ at `panels`, ⌊w/2⌋ at `panels-wide`, taking `min(level_cap, existing_008_panel_w_cap)`. 008's "needs at least N columns" script error (`render_api.rs:144`) still applies afterward (FR-011, FR-012) (depends on T029).
- [ ] T031 [US2] Strip effects entirely at `visual_level = "none"` via the `ResolveCtx` axis already threaded through `resolve` in `bee-harness/src/tui/effects.rs`; content renders immediately with no animation (FR-024) (depends on T013, T029).
- [ ] T032 [US2] Thread the downgrade note into `ToolResult.content` in `bee-harness/src/tools/render.rs` so the model sees what happened and can adjust next turn (FR-009, FR-010) (depends on T029).
- [ ] T033 [P] [US2] Test each downgrade matrix row in `bee-harness/src/tui/visual_gate.rs`, asserting the **exact** note text per row (SC-004, US2 §1/§3/§7) (depends on T029, T032).
- [ ] T034 [P] [US2] Test the width caps in `bee-harness/src/tui/visual_gate.rs` by measuring the resulting panel `Rect` at a known terminal size: ≤ ⌊w/3⌋ at `panels`, ≤ ⌊w/2⌋ at `panels-wide` (SC-005, US2 §2) (depends on T030).
- [ ] T035 [P] [US2] Test that `visual_level = "none"` routes panel **and** overlay requests inline while leaving harness chrome effects untouched (FR-006d, FR-010, US2 §7) (depends on T031, T032).

**Checkpoint**: the operator's ceiling is enforced, downgrades are silent-but-reported, and the agent never escalates past its tier.

---

## Phase 5: User Story 3 — Full-screen takeover with TTL and dismiss (Priority: P2)

**Goal**: a single full-screen overlay the operator can always escape, that always expires on its own,
and that never blocks input.

**Independent Test**: trigger an overlay, assert the dismiss hint occupies the overlay's last row,
advance past the TTL, and assert the overlay is gone and the chat area restored.

### Implementation for User Story 3

- [ ] T036 [US3] Define `Overlay { spec, created, ttl, dismissing }` and its `Entering → Showing → Dismissing` state machine in `bee-harness/src/tui/overlay.rs`. Resolve `ttl` **once** at construction as `min(requested, config_max)` so a mid-session config change cannot extend a live overlay (FR-017, FR-021, data-model.md) (depends on T011, T007).
- [ ] T037 [US3] Add `overlay: Option<Overlay>` to `App` in `bee-harness/src/tui/app.rs` — the `Option` structurally enforces FR-020's at-most-one (depends on T036).
- [ ] T038 [US3] Lay the overlay over the chat area only, never the input line, in `bee-harness/src/tui/view.rs`; the operator keeps typing and submitting throughout (FR-014) (depends on T037).
- [ ] T039 [US3] Render the dismiss hint in the overlay's bottom row in `bee-harness/src/tui/overlay.rs`: ``Esc`` to dismiss · auto-dismiss in {N}s, `{N} = ceil(remaining)`, themed `dim`, unstyled but still present under `NO_COLOR`. If the area is too short to spare a row, the hint still wins — it must never be the thing that gets clipped (FR-015) (depends on T038).
- [ ] T040 [US3] Insert the `Esc` dismiss check in `handle_key` in `bee-harness/src/tui/app.rs`, **after** the `help_open` swallow (line 205-210) and **before** the `KeyModifiers::CONTROL` block (line 213), so it outranks focus dispatch and works from either focus. Do **not** bind `q` — it keeps meaning quit (FR-016, contracts/overlay-lifecycle.md, research R6) (depends on T037).
- [ ] T041 [US3] Wire the remaining three dismiss triggers in `bee-harness/src/tui/mod.rs` and `overlay.rs` — TTL expiry, assistant reply, and replacement (cross-fade, replace in place) — all producing the same 200ms fade-out and restoring the chat with its scroll position preserved (FR-018, FR-019, FR-020) (depends on T036, T037).
- [ ] T042 [US3] Activate the 1Hz countdown arm of the scheduler in `bee-harness/src/tui/mod.rs` for `Showing`, keeping 60fps for `Entering`/`Dismissing` and idle when no overlay and no effects (FR-002) (depends on T021, T036).
- [ ] T043 [US3] Handle the overlay edge cases in `bee-harness/src/tui/overlay.rs`: dismissal mid-`Entering` cancels the entrance and fades from the current buffer state; terminal resize cancels effects and re-renders at the new size; a 0×0 area is a no-op; a dismiss followed immediately by a new overlay completes its fade-out before the new `Entering` begins (spec Edge Cases) (depends on T041).
- [ ] T044 [P] [US3] Test the hint and countdown in `bee-harness/src/tui/overlay.rs`: present in the last row, `{N}` decrements once per second, and still advances with `BEE_NO_ANIMATION=1` — the countdown is information, not motion (FR-015) (depends on T039, T042).
- [ ] T045 [P] [US3] Test `Esc` precedence in `bee-harness/src/tui/app.rs`: from input focus with text typed, `Esc` dismisses the overlay and **leaves the text intact**; a second `Esc` clears it. With no overlay, `Esc` still clears input and still hides the panel column — a regression guard on 008 (FR-016, US3 §2) (depends on T040).
- [ ] T046 [P] [US3] Test dismissal latency and TTL in `bee-harness/src/tui/overlay.rs`: gone within one frame of the keypress with chat scroll unchanged (SC-006); absent from the buffer after `ttl + 0.2s` (SC-007) (depends on T041).
- [ ] T047 [P] [US3] Test TTL resolution in `bee-harness/src/tui/overlay.rs`: a 90s request against a 20s config yields 20s; a 5s request yields 5s; `ttl_ms = 0` is a script error (FR-021, US2 §6) (depends on T036).
- [ ] T048 [P] [US3] Test the wakeup budget in `bee-harness/src/tui/mod.rs`: an N-second `Showing` overlay costs ~N redraws, not 60N — the counter after a 10s effect-free overlay reads ≤ 15 (SC-003) (depends on T042).
- [ ] T049 [P] [US3] Test replacement in `bee-harness/src/tui/overlay.rs`: a second overlay request replaces the first in place with a cross-fade and never stacks (FR-020, US3 §5) (depends on T041).

**Checkpoint**: takeover is safe to enable — bounded in time, always escapable, never input-blocking.

---

## Phase 6: User Story 4 — The agent requests specific effects via Rhai (Priority: P3)

**Goal**: the model can express directional intent instead of accepting the defaults, through a
curated surface that never exposes a tachyonfx type to the sandbox.

**Independent Test**: run a Rhai script attaching `slide_in("left", 400)` to a bar chart, render to a
panel, advance 200ms, and verify partially-slid content.

### Implementation for User Story 4

- [ ] T050 [US4] Register the 12 effect constructors on the existing Rhai `Engine` in `bee-harness/src/render_api/effect_api.rs` (same engine as 003-visual-render, no second engine), each producing an `EffectSpec`. Clamp `ms` to 100–2000 **at construction** so the transcript records the effective value; resolve unknown directions to `"left"` at construction too (FR-022, FR-023, FR-025) (depends on T005).
- [ ] T051 [US4] Register `widget.effect(e)` in `bee-harness/src/render_api/effect_api.rs`, attaching an `EffectSpec` to any widget type via the `Renderable` wrapper (FR-022) (depends on T050, T006).
- [ ] T052 [US4] Register the takeover commit verbs `render_fullscreen(w)` and `render_fullscreen_ttl(w, ttl_ms)` in `bee-harness/src/render_api.rs`, producing `RenderTarget::Overlay`. Reject `ttl_ms <= 0` as a script error, matching `render_to_ttl` (`render_api.rs:912`) (FR-013a) (depends on T007).
- [ ] T053 [US4] Reserve the `"takeover"` panel id in `bee-harness/src/render_api.rs`: the `Overlay`→`Panel` downgrade upserts it, and a script calling `render_to("takeover", w)` directly is a script error. The existing `[a-z0-9_-]` validator (`render_api.rs:182`) needs no change (research R7, contracts/visual-levels.md) (depends on T029, T052).
- [ ] T054 [US4] Carry `EffectSpec` and `RenderTarget` from the Rhai result through to `ToolResult` in `bee-harness/src/tools/render.rs` (FR-025) (depends on T051, T052).
- [ ] T055 [P] [US4] Snapshot-test an agent-requested effect in `bee-harness/src/render_api/effect_api.rs`: `chart.effect(slide_in("left", 400))` produces `EffectSpec::SlideIn`, resolves to a live tachyonfx slide, and is active for 400ms — frames at t=0, t=200ms, t=400ms (SC-008, US4 §1) (depends on T050, T016, T015).
- [ ] T056 [P] [US4] Test argument handling in `bee-harness/src/render_api/effect_api.rs`: `ms = 5000` clamps to 2000, `ms = 10` clamps to 100, `dir = "sideways"` falls back to `left`, and `pulse("sting", 200)` flashes the theme's error role and returns to normal (FR-023, US4 §2/§3) (depends on T050, T016).
- [ ] T057 [P] [US4] Test silent degradation in `bee-harness/src/render_api/effect_api.rs`: at `visual_level = "none"` an attached effect is dropped and content renders immediately — no tool-call failure (FR-024, US4 §4) (depends on T031, T050).
- [ ] T058 [P] [US4] Test the takeover verbs in `bee-harness/src/render_api.rs`: `render_fullscreen` yields `Overlay`, `render_fullscreen_ttl(w, 0)` is a script error, and `render_to("takeover", w)` is a script error (FR-013a, R7) (depends on T052, T053).

**Checkpoint**: the model can author directional visuals and request takeovers; the sandbox still never sees a tachyonfx type.

---

## Phase 7: User Story 5 — Honeycomb effect presets for bee's own chrome (Priority: P3)

**Goal**: bee's own UI moves with the same vocabulary it gives the agent — and the agent cannot
override it.

**Independent Test**: start a session and verify the header fade-in plays (background-colored cells at
t=0, final header colors at t=300ms). Each preset is independently testable as a buffer snapshot at
known offsets.

### Implementation for User Story 5

- [ ] T059 [P] [US5] Add the session-start chrome presets in `bee-harness/src/tui/effects.rs`: header fades from background to the `info` role over 300ms; footer slides up from the bottom. Registered via `add_effect` (unkeyed), never through the agent path (FR-026, FR-028, US5 §1) (depends on T016).
- [ ] T060 [P] [US5] Add the tool-call → result pulse (the tool-call line pulses `accent` when its result arrives) and the episode pass/fail pulse (`success`/`error`, 500ms) in `bee-harness/src/tui/effects.rs` (FR-026) (depends on T016).
- [ ] T061 [P] [US5] Add the panel-column appearance effect in `bee-harness/src/tui/panels.rs`: the column expands from zero width via tachyonfx `stretch` when the first panel is created (FR-026) (depends on T017).
- [ ] T062 [P] [US5] Make the bee mascot evolve in via `evolve_into(BlocksHorizontal, 600)` instead of the current pop-in, gated on the existing `BEE_MASCOT` opt-in, in `bee-harness/src/tui/effects.rs` (US5 §2) (depends on T016).
- [ ] T063 [US5] Assert chrome effects are not agent-overridable in `bee-harness/src/tui/effects.rs` — chrome uses unkeyed `add_effect`, so no agent panel key can cancel or replace it (FR-028) (depends on T059, T060).
- [ ] T064 [P] [US5] Snapshot-test the chrome presets in `bee-harness/src/tui/effects.rs` at known offsets, and assert the two axis behaviors: at `visual_level = "none"` chrome still animates in full, and under `BEE_NO_ANIMATION=1` no chrome effect plays and the 60fps state is never entered (FR-006d, US5 §3/§4/§5) (depends on T059, T062, T027).

**Checkpoint**: all five stories independently functional.

---

## Phase 8: Polish & Cross-Cutting Concerns

- [ ] T065 [P] Write the four example scripts in `specs/009-tachyonfx-effects/examples/`: `panel-fade-in.rhai`, `slide-dashboard.rhai`, `status-pulse.rhai`, `fullscreen-chart.rhai`, and assert each parses and runs in a test (spec Project Structure).
- [ ] T066 [P] Extend the dependency-boundary guard in `bee-harness/tests/core_deps_guard.rs` to assert `tachyonfx` is absent from `bee-core` and `bee-common` trees and from the headless `bee-harness` build (SC-009, Constitution V).
- [ ] T067 [P] Document the three presentation axes (`NO_COLOR`, `BEE_NO_ANIMATION`, `BEE_VISUAL_LEVEL`) and the takeover keys in the user-facing docs and `--help` text, including the deliberate `q`-is-not-dismiss decision (contracts/motion-control.md).
- [ ] T068 Run the full `quickstart.md` sweep: `cargo test --workspace`, `cargo test -p bee-harness --features tui`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check`, plus the interactive overlay walkthrough (type → `Esc` → text survives → `Esc` → clears → `q` quits).

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: no dependencies. T002's gate blocks everything downstream.
- **Foundational (Phase 2)**: depends on Setup — **blocks all user stories**.
- **US1 (Phase 3)**: depends on Foundational. No dependency on other stories.
- **US2 (Phase 4)**: depends on Foundational. Independent of US1 — the gate can be tested with static content.
- **US3 (Phase 5)**: depends on Foundational **and** US2 (T029 provides the gate that admits `Overlay` at level `takeover`; T021 provides the scheduler shape).
- **US4 (Phase 6)**: depends on Foundational, US1 (T016's mappings), and US2 (T029 for the downgrade, T031 for level-`none` stripping).
- **US5 (Phase 7)**: depends on US1 (T016) only. Can run parallel to US2/US3/US4.
- **Polish (Phase 8)**: depends on all desired stories.

### The one cross-story constraint worth naming

US3 is the only story with a hard dependency on another (US2). Takeover without the gate would let the
agent claim the whole screen regardless of the operator's ceiling — shipping US3 before US2 would
briefly invert this feature's entire point.

### Parallel Opportunities

- Phase 1: T003 and T004 in parallel after T002.
- Phase 2: T005, T006, T007, T008 in parallel; T009, T010, T011 in parallel; then T012, T013, T014.
- Phase 3: T022–T028 all parallel once their implementation deps land.
- Phase 4: T033, T034, T035 in parallel.
- Phase 5: T044–T049 in parallel.
- Phase 6: T055–T058 in parallel.
- Phase 7: T059, T060, T061, T062 in parallel.
- Across stories: once Phase 2 closes, US1 / US2 / US5 can proceed on three tracks; US3 joins after US2, US4 after US1+US2.

---

## Parallel Example: User Story 1

```bash
# After T016–T021 land, run the whole US1 assertion set together:
Task: "Snapshot panel entrance at t=0/150/300ms in bee-harness/src/tui/effects.rs"     # T022
Task: "Snapshot panel update at t=0/200/400ms in bee-harness/src/tui/effects.rs"       # T023
Task: "Test transition cancellation in bee-harness/src/tui/panels.rs"                  # T024
Task: "Test NO_COLOR degradation in bee-harness/src/tui/effects.rs"                    # T025
Task: "Test zero idle redraws in bee-harness/src/tui/mod.rs"                           # T026
Task: "Test BEE_NO_ANIMATION kill switch in bee-harness/src/tui/effects.rs"            # T027
```

---

## Implementation Strategy

### MVP (Phases 1–3)

1. Phase 1 Setup — the dependency bump, gated on 291 tests staying green.
2. Phase 2 Foundational — serde layer, config axes, resolver chokepoint.
3. Phase 3 US1 — automatic panel transitions plus the motion kill switch.
4. **STOP and VALIDATE**: the TUI is visibly alive, degrades under `NO_COLOR`, goes still under `BEE_NO_ANIMATION`. Zero Rhai API change, zero agent involvement.

That is a complete, shippable increment: the single biggest visual improvement for the least surface
area, and it needs none of the permission machinery.

### Incremental Delivery

1. MVP → demo
2. +US2 → the operator's ceiling is enforced → demo
3. +US3 → takeover, now safe because US2 bounds it → demo
4. +US4 → the agent gains directional intent → demo
5. +US5 → bee's own chrome joins in → demo

### Sequencing note

Motion control is deliberately **not** deferred to a polish phase. Its chokepoint (T013) is
Foundational and its assertions (T027, T028) ship with the MVP, because FR-006c's "never enters the
60fps state" is a structural guarantee — cheap to establish at the one place effects are created,
expensive and unreliable to retrofit across a dozen call sites later.

---

## Notes

- `[P]` = different files, no incomplete-task dependency.
- Every task names its file; every user-story task carries its story label.
- Requirement IDs (FR-###, SC-###) and research findings (R#) are cited so each task traces back.
- Phase 1's gate is already measured (research R1/R2) — T002 is a reproduction, not a discovery.
- Commit after each task or logical group; stop at any checkpoint to validate a story independently.
