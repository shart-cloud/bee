---
description: "Task list for bee Visual Rendering — Slice 2 (sprites, animation, the bee mascot)"
---

# Tasks: bee Visual Rendering — Slice 2 (sprites & animation)

**Input**: [plan-slice2.md](./plan-slice2.md), [research-slice2.md](./research-slice2.md),
[data-model-slice2.md](./data-model-slice2.md), [contracts/sprite-api.md](./contracts/sprite-api.md),
[spec.md](./spec.md) (sprite/animation sections). Builds on Slice 1 (commit `b519f15`).

**Scope**: Completes feature 003 — the sprite + animation subsystem deferred from Slice 1
(FR-028…FR-032, SC-015…SC-018). No new crate dependency; all work in `bee-harness`.

**No user stories**: sprites/animation are not user stories (US6/US7 shipped in Slice 1). Tasks map to
FR groups and carry capability tags for traceability: **[SPR]** sprite rendering, **[ANI]** animation,
**[BEE]** the mascot. Setup/foundational/polish carry no tag.

**Tests**: the new Rhai caps (sprite ≤ 32×32, ≤ 16 frames, ≤ 32 palette entries) are part of the
fail-closed drawing boundary — their rejection tests are **⛔FAIL-FIRST**. The `reclaim_rows`
generalization has a **regression gate**: the spinner's existing tests must pass unchanged.

## Format: `[ID] [P?] [Tag] Description with file path`
- **[P]**: parallelizable (different files, no incomplete dependency). IDs continue from Slice 1 (T025).

---

## Phase 1: Setup

- [X] T026 Declare the new `viz` submodules in `bee-harness/src/viz.rs` (`pub mod sprite;` for `sprite_render` re-export, `pub mod bee;`, `mod animator;`) and re-export `SpriteSpec`/`AnimationSpec` from `lib.rs`. No new dependency (confirm `Cargo.toml` unchanged; SC-019 still holds)

**Checkpoint**: module scaffold compiles; deps still confined.

---

## Phase 2: Foundational (data types)

- [X] T027 [P] Add `SpriteSpec { width, height, pixels: Vec<Option<(u8,u8,u8)>> }` and `AnimationSpec { frames, interval_ms, bounce, cycles }` (serde, PartialEq) to `bee-harness/src/render_spec.rs` (per data-model-slice2.md §2/§3)
- [X] T028 Add `RenderSpec::Sprite { spec: SpriteSpec }` + `RenderSpec::Animation { spec: AnimationSpec }` to the `#[non_exhaustive]` enum in `bee-harness/src/render_spec.rs`, and **replace the wildcard `_` arms** in `element_count`/`layout_depth`/`ascii_fallback`/`summary_phrase` with explicit `Sprite`/`Animation` arms (data-model-slice2.md §1). Depends on T027

**Checkpoint**: `RenderSpec` grows additively; Slice 1 tests still pass (`cargo test -p bee-harness`).

---

## Phase 3: Sprite rendering (static — FR-028, FR-030; SC-015)

### Tests

- [X] T029 [P] [SPR] Half-block + quantization tests in `bee-harness/tests/sprite_render.rs`: a 16×16, 4-color sprite → 8 rows, each with truecolor escapes; a transparent pixel emits no bg escape for its half (SC-015); `quantize_256`/`quantize_16` map known RGBs to expected indices; `detect_color_mode` honors `COLORTERM`/`TERM`
- [X] T030 [SPR] Rhai sprite cap-rejection tests in `bee-harness/tests/render_sandbox.rs` (extend): `sprite(40, 40, pal)` → `is_error` (max 32×32); palette with > 32 entries → error; bad hex in `pal.set` → error. ⛔FAIL-FIRST

### Implementation

- [X] T031 [P] [SPR] Implement `bee-harness/src/viz/sprite_render.rs`: `ColorMode{TrueColor,Ansi256,Ansi16}`, `detect_color_mode()`, `quantize_256`, `quantize_16`, `render_frame(&SpriteSpec) -> Vec<String>` (half-block glyph selection per research D12; truecolor/quantized/mono SGR per D13). Depends on T027
- [X] T032 [SPR] Extend `bee-harness/src/render_api.rs`: `PaletteBuilder` (`palette()`, `pal.set` parsing `#RRGGBB`/`transparent`, ≤ 32) + `SpriteBuilder` (`sprite(w,h,pal)` ≤ 32×32, `paint`/`set`/`fill`); extend `render()`/`layout.add()` `try_cast` arms for `SpriteBuilder` (per contracts/sprite-api.md). Depends on T027
- [X] T033 [SPR] In `bee-harness/src/repl/terminal.rs`, handle `RenderSpec::Sprite` in `render_widget`: render via `viz::sprite_render::render_frame` (color mode from `detect_color_mode` + tty) and emit the rows inline through `ExternalPrinter`. Depends on T031
- [X] T034 [SPR] Make T029/T030 green: `cargo test -p bee-harness --test sprite_render --test render_sandbox` + clippy clean

**Checkpoint**: the agent can render a static sprite; it appears inline; unsupported terminals degrade.

---

## Phase 4: Animation (FR-029, FR-030, FR-031; SC-016, SC-017)

### Tests

- [X] T035 [ANI] Animator tests in `bee-harness/tests/sprite_anim.rs`, **testing the pure seam, not real time** (research D14): (a) `playback(spec)` returns the expected frame-index order — 3 frames `bounce=true cycles=1` → `[0,1,2,1]`, `bounce=false cycles=2` → `[0,1,2,0,1,2]` (SC-016 ordering); (b) `redraw_block(rows)` starts with `\x1b[{N}A` (SC-016 cursor-up); (c) via a `CapturePrinter`: start an animation, emits frame 0 immediately, stop it, assert `reclaim_rows == N` (SC-016 reclaim — same shape as the existing spinner reclaim test); (d) starting an animation while the spinner runs clears `spinning` (SC-017). No mock clock; the `sleep`-between-frames glue is the trusted spinner pattern
- [X] T036 [ANI] Rhai animation cap tests in `bee-harness/tests/render_sandbox.rs` (extend): a 17th `anim.add` → `is_error` (max 16); frames of mismatched dims → error; interval below 50 / above 1000 is clamped (not an error). ⛔FAIL-FIRST

### Implementation

- [X] T037 [ANI] In `bee-harness/src/repl/terminal.rs`, generalize `reclaim: AtomicBool` → `reclaim_rows: AtomicUsize`; update `emit()` to move up + clear N rows when `N > 0`. **Regression gate**: `N = 1` must reproduce the spinner's exact bytes — the existing `spinner_draws_immediately_then_reclaims_its_row` test passes unchanged
- [X] T038 [ANI] Implement `bee-harness/src/viz/animator.rs` with the **pure testability seam** (research D14): `pub fn playback(&AnimationSpec) -> Vec<usize>` (ordered frame indices applying `bounce` ping-pong + `cycles`; `cycles=0` → one period the task repeats) and `pub fn redraw_block(rows: &[String]) -> String` (`\x1b[{N}A` + N lines) — both pure, no tokio/clock. The background tokio task is thin glue: emit frame 0, then `for idx in playback(spec)[1..] { sleep(interval_ms).await; break if inactive; print(redraw_block(render_frame(&frames[idx]))) }`, sharing the `spinning` flag + `spin_task` handle + printer with the spinner, setting `reclaim_rows = N` on completion. Depends on T037
- [X] T039 [ANI] In `terminal.rs`, handle `RenderSpec::Animation` in `render_widget` via a `start_animation` path; unify the single active slot so `busy_start` (spinner) and `start_animation` each stop the other first (FR-031). Depends on T037, T038
- [X] T040 [ANI] Extend `bee-harness/src/render_api.rs`: `AnimationBuilder` (`animation(ms)` clamp 50–1000, `add` ≤ 16 same-dims, `bounce`, `cycles`); extend `render()`/`layout.add()` `try_cast` for `AnimationBuilder` (first-frame-only inside a layout). Depends on T027
- [X] T041 [ANI] Make T035/T036 green **and** re-run the existing spinner tests unchanged (regression); clippy clean

**Checkpoint**: animations play in place, reclaim on completion, and are mutually exclusive with the spinner.

---

## Phase 5: The bee mascot (FR-032; SC-018)

- [X] T042 [BEE] Implement `bee-harness/src/viz/bee.rs`: `sprite() -> SpriteSpec` (16×16 bee) and `animation() -> AnimationSpec` (3-frame wing-flap, 150 ms, bounce), built once via `OnceLock` from a bitmap-string + palette (no `Date`/random). Depends on T027
- [X] T043 [BEE] Register `bee_sprite()` / `bee_animation()` in `bee-harness/src/render_api.rs` returning the `viz::bee` constants; add a Rhai test (SC-018): both callable → valid `SpriteSpec`/`AnimationSpec`. Depends on T042
- [X] T044 [BEE] Wire the opt-in REPL startup mascot: a `--bee` flag in `bee-harness/src/bin/bee-repl.rs` (and `BEE_MASCOT=1` env) that plays `viz::bee::animation()` once beside the session line, then reclaims. Absent in batch mode. Depends on T039, T042
- [ ] ~~T045 [BEE] Render a static `viz::bee::sprite()` beside the episode-completion status line~~ — **DEFERRED** (analyze M3): the "episode-completion" surface is ambiguous in the REPL (no discrete episode end; batch suppresses the bee). Slice 2 ships the bee via `--bee`/`BEE_MASCOT` startup (T044) + the Rhai `bee_*` functions (T043) only; the episode-completion bee is punted to a later change once the surface is defined

**Checkpoint**: the bee is available (opt-in) at REPL startup and in the Rhai API — never forced. (Episode-completion bee deferred — M3.)

---

## Phase 6: Polish & validation

- [X] T046 [P] Add example scripts `specs/003-visual-render/examples/pixel-bee.rhai` (bee sprite + wing-flap animation) and `status-sprite.rhai` (a sprite whose color changes by pass/fail) — the two examples the spec's Project Structure lists but Slice 1 deferred
- [X] T047 [P] Extend the `render` tool `schema()` function list (in `tools/render.rs`) with the sprite/animation/palette/bee functions from contracts/sprite-api.md, so the model sees them in its tool definition
- [X] T048 [P] Re-confirm SC-019 (`cargo tree -p bee-core|-p bee-common` show no rhai/ratatui — Slice 2 added no dep); author `quickstart-slice2.md` mapping SC-015…SC-018 to their tests and run it
- [X] T049 Full green gate: `cargo test -p bee-harness` (Slice 1 + Slice 2), `cargo clippy -p bee-harness --all-targets`, `cargo build --workspace`; mark all Slice 2 tasks `[X]`

---

## Dependencies & Execution Order

### Phase dependencies
- Setup (T026) → Foundational (T027→T028) blocks everything.
- Sprite (Phase 3) → after Foundational; the MVP of Slice 2 (static sprites, SC-015).
- Animation (Phase 4) → after Foundational; **T037 (reclaim_rows) is the linchpin** — T038/T039 depend on it.
- Bee (Phase 5) → after sprite render (T033) + animator (T039).
- Polish (Phase 6) → last.

### Key task-level dependencies
- T031 (sprite_render) blocks T033, and is used by T042/T038.
- T037 (reclaim_rows) blocks T038, T039 — do it first in Phase 4 and keep the spinner test green.
- T042 (bee) blocks T043, T044, T045.

### Parallel opportunities
- Foundational: T027 alone (T028 follows, same file).
- Sprite: **T029 ∥** (tests) and **T031 ∥ T032** (different files, both need only T027).
- Animation: T037 first, then T038; T040 ∥ (different file).
- Polish: **T046 ∥ T047 ∥ T048**.

---

## Implementation Strategy

### MVP first (static sprites)
1. Setup + Foundational (T026–T028).
2. Phase 3 (T029–T034) → **STOP and validate**: the agent renders a static sprite inline; SC-015 passes.
3. Demo `bee_sprite()` once T042/T043 land.

### Incremental delivery
- Static sprites (Phase 3) → animation (Phase 4) → the bee (Phase 5). Each is an independently testable
  increment; the animator (Phase 4) is the riskiest and is isolated behind the `reclaim_rows` regression gate.

---

## Notes
- No new dependency — sprites are hand-rolled ANSI; the animator reuses the existing tokio + spinner machinery.
- ⛔FAIL-FIRST tests (T030, T036) validate the fail-closed drawing caps before they exist (Constitution I).
- Regression gate (T037/T041): the spinner's byte-exact behavior at `N = 1` must survive the `reclaim_rows` change.
- Completes feature 003 — after Slice 2, all of FR-019…FR-032 and SC-008…SC-019 are delivered.
