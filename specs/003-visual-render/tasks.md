---
description: "Task list for bee Visual Rendering & Design Language — Slice 1 (viz + US6 static render tool + US7 chrome)"
---

# Tasks: bee Visual Rendering & Design Language (Slice 1)

**Input**: Design documents in `/specs/003-visual-render/` (plan.md, research.md, data-model.md, contracts/, quickstart.md)

**Scope**: This list delivers **Slice 1** — the `viz` honeycomb foundation, **US6 (agent-declared
visualization)** for the *static* buffer-rendered widgets, and **US7 (unified visual chrome)** in
full. The **sprite + animation subsystem** (FR-028…FR-032: half-block truecolor rendering, 256/16
quantization, the tokio animator, the bee mascot) is **Slice 2** — captured in the **Deferred**
section, not built here. All work is in `bee-harness`; nothing touches the enforcement path.

**Tests**: The `render` tool's Rhai sandbox is a **fail-closed, deny-by-default boundary** (FR-021 —
only drawing functions registered; everything else denied by omission). Per the constitution
("every security-relevant behavior is a testable requirement"), the limit/rejection tests are written
to **fail first** (marked ⛔FAIL-FIRST) before their implementation. Other tests (rendering shape,
chrome preservation) are written alongside their tasks.

## Format: `[ID] [P?] [Story] Description with file path`
- **[P]**: parallelizable (different files, no incomplete dependency)
- **[US6]/[US7]**: belongs to that user story (setup/foundational/polish carry no story label)
- Paths are relative to repo root `/home/jg/git/bee/`

---

## Phase 1: Setup (Shared Infrastructure)

- [X] T001 Add dependencies to `bee-harness/Cargo.toml`: `rhai = { version = "1", default-features = false, features = ["only_i64", "sync"] }` and `ratatui = { version = "0.29", default-features = false }`. Do **not** enable rhai `unchecked` (disables limits) or the ratatui `crossterm`/backend features. Confirm `cargo tree -p bee-core`/`-p bee-common` still show neither (NFR-002 gate, re-checked in T023)
- [X] T002 Scaffold the `viz` module: create `bee-harness/src/viz.rs` (module root with `pub mod palette; pub mod glyph; pub mod grid; mod buffer_render;`) and the empty `bee-harness/src/viz/` files; declare `pub mod viz;` and re-export `viz` + (later) `RenderSpec` in `bee-harness/src/lib.rs`
- [X] T003 [P] Confirm `cargo build -p bee-harness` compiles the empty scaffold and `cargo clippy -p bee-harness` is clean (deny-warnings posture matching the repo)

**Checkpoint**: crate builds with rhai + ratatui present; `viz` module exists; deps confined to `bee-harness`.

---

## Phase 2: Foundational (Blocking Prerequisites — the honeycomb vocabulary)

**Purpose**: The palette + glyph constants both user stories consume. No story label.

- [X] T004 [P] Implement `viz::palette` in `bee-harness/src/viz/palette.rs`: constants `HONEY="33"`, `POLLEN="32"`, `STING="31"`, `SMOKE="2"`, `ROYAL="36"`; `paint(code, text)` (SGR-wrap iff color enabled), `bold(code, text)` (`1;{code}`), `is_color_enabled()` (false when `NO_COLOR` set to any value — matching current `terminal.rs`). Byte-for-byte compatible with the existing `terminal.rs::paint` (per contracts/honeycomb.md migration invariant)
- [X] T005 [P] Implement `viz::glyph` in `bee-harness/src/viz/glyph.rs`: constants `DOT_PASS`/`DOT_FAIL`=`●`, `DOT_SKIP`=`○`, `ARROW`=`▸`, `CHECK`=`✓`, `CROSS`=`✗`, `WARN`=`⚠`, `HEX`=`⬡`, `HLINE`=`─`, `VLINE`=`│` (per data-model.md §5)

**Checkpoint**: `viz::palette`/`viz::glyph` compile and unit-test; both stories can now build against them.

---

## Phase 3: User Story 6 — Agent-Declared Visualization (Priority: P2) 🎯 MVP

**Goal**: The agent calls the `render` tool with a Rhai script that builds a static widget; the
harness evaluates it in the Rhai sandbox (in-process, sandbox param ignored), produces a `RenderSpec`,
returns a **text summary** to the model, and renders the widget inline via `render_widget`.

**Independent Test**: A `MockModel` exchange whose model turn calls `render` with a 5-bar chart
script → the test `Collector` receives `render_widget` with a `RenderSpec::BarChart` (5 correct
bars); `ToolResult.content` is the summary, not ANSI; a `loop {}` script is rejected at the op limit;
an unregistered-function script is rejected.

### Tests (write first)

- [X] T006 [P] [US6] Rhai deny-by-default / limit tests in `bee-harness/tests/render_sandbox.rs`: `loop {}` → `is_error` mentioning the operation limit, within 50 ms (SC-009); `import "std";` / `std::fs::read(".")` / any unregistered call → `is_error` with a "function not found" message (SC-010); script with no `render()` → "script produced no visualization"; two `render()` calls → last wins + summary notes the discard (Edge Cases). ⛔FAIL-FIRST
- [X] T007 [P] [US6] MockModel render-exchange test in `bee-harness/tests/render_exchange.rs`: scripted turn calls `render` with a 5-bar `bar_chart` script; assert (a) the test `Collector` (overriding `render_widget`) captured `RenderSpec::BarChart` of len 5 with correct labels/values, (b) `ToolResult.content` is the text summary and contains no `\x1b[`, (c) `result.render_spec.is_some()` (SC-008)
- [X] T008 [P] [US6] Buffer→ANSI pipeline tests in `bee-harness/tests/render_ansi.rs`: a `RenderSpec::BarChart` → `Vec<String>` containing box-drawing chars + colored bars, produced without alt-screen/raw-mode (SC-011); a `vsplit` of a gauge + a table → buffer height = sum of children heights + separators, full width (SC-014)

### Implementation

- [X] T009 [P] [US6] Define `RenderSpec` (serde, `#[serde(tag="kind", rename_all="snake_case")]`, `#[non_exhaustive]`) + `Bar`/`Series`/`Point`/`Row`/`Dot`/`DotState`/`Direction` in `bee-harness/src/render_spec.rs` — Slice 1 variants only (no `Sprite`/`Animation`); no ratatui/rhai types (per data-model.md §1). Re-export from `lib.rs`
- [X] T010 [US6] Add `render_spec: Option<RenderSpec>` to `ToolResult` (`#[serde(default, skip_serializing_if="Option::is_none")]`) in `bee-harness/src/tools.rs`; set `None` in existing constructors; add `ToolResult::rendered(summary, spec)` (per data-model.md §2). Depends on T009
- [X] T011 [P] [US6] Implement the Rhai builders + `RenderContext` + all `register_fn` calls in `bee-harness/src/render_api.rs`: `ChartBuilder`/`SeriesHandle`/`TableBuilder`/`GaugeBuilder`/`DotGridBuilder`/`TextBuilder`/`LayoutBuilder`, `render(widget)`, and API-level caps (nesting ≤ 3, ≤ 500 elements, gauge clamp) returning Rhai errors (per contracts/rhai-api.md + data-model.md §3/§4). Depends on T009
- [X] T012 [US6] Implement `RenderTool` in `bee-harness/src/tools/render.rs`: holds `Arc<rhai::Engine>` built once with limits (`set_max_operations(10_000)`, call levels 16, expr depths 32/16, string 64 KB, array 1 000, map 100) and `disable_symbol("eval")`; `call()` ignores `sandbox`, runs `eval_with_scope`, maps errors to `ToolResult::error` and success to `ToolResult::rendered(summary, spec)` with the summary format from contracts/render-tool.md. Then wire `bee-harness/src/tools.rs`: `is_known_tool("render") == true` (NOT added to `DEFAULT_TOOLS`, NFR-003) and a `"render"` arm in `registry_for`. Depends on T010, T011
- [X] T013 [P] [US6] Implement the render pipeline in `bee-harness/src/viz/buffer_render.rs` + `viz::render_to_ansi(spec, max_width, max_height)`: `terminal_dims()` (via `std::io::IsTerminal` + `libc` `TIOCGWINSZ`, honoring `COLUMNS`, fallback 80/no-color); build transient ratatui widgets from `RenderSpec`; render into a headless `Buffer::empty(Rect)` sized `min(width,120)×min(height,40)`; walk rows → ANSI lines with style batching, basic-ANSI SGR only, symbols-only when color disabled (per research D4/D9). Depends on T009
- [X] T014 [US6] Add `fn render_widget(&self, spec: &RenderSpec)` to the `ReplOutput` trait in `bee-harness/src/repl.rs` with a **default impl** that formats an ASCII fallback and routes it through `self.info(...)` (AS-5); wire the exchange loop: after `output.tool_result(&result, &audit)` add `if let Some(spec) = &result.render_spec { output.render_widget(spec); }`; give the test `Collector` an override that captures the spec (per research D8). Depends on T009
- [X] T015 [US6] Implement `TerminalOutput::render_widget` in `bee-harness/src/repl/terminal.rs` by calling `viz::render_to_ansi` and emitting each line through the existing `ExternalPrinter` (`emit` path), no alt-screen/raw-mode (SC-011). Depends on T013, T014

### Make tests pass

- [X] T016 [US6] Make T006/T007/T008 green: `cargo test -p bee-harness --test render_sandbox --test render_exchange --test render_ansi` + `cargo clippy -p bee-harness` clean

**Checkpoint**: US6 is fully functional and independently testable — the agent can render static widgets; the model sees only summaries. **MVP deliverable.**

---

## Phase 4: User Story 7 — Unified Visual Chrome (Priority: P3)

**Goal**: The REPL's existing terminal output migrates from ad-hoc inline SGR/glyph literals to
`viz::palette`/`viz::glyph` (no behavioral change), and batch/episode output gains a status grid.

**Independent Test**: The existing `TerminalOutput` test suite passes without assertion changes
(SC-012); the new `status_grid` test given 3 pass + 1 fail produces 3 green `●` + 1 red `●`
(SC-013); with `NO_COLOR=1`, glyphs remain but SGR codes are gone (AS-3).

### Tests (write first)

- [X] T017 [P] [US7] `status_grid` tests in `bee-harness/tests/status_grid.rs` (or `viz::grid` unit tests): 3 `Pass` + 1 `Fail` → 3 `\x1b[32m…●` + 1 `\x1b[31m…●`, names left-aligned, `3/4 pass` right-aligned (SC-013); with `NO_COLOR=1` the dots render with no SGR (AS-3)

### Implementation

- [X] T018 [P] [US7] Implement `viz::grid` in `bee-harness/src/viz/grid.rs`: `Status { Pass, Fail, Skip }`, `status_grid(results: &[(String, Status)]) -> String` (colored dots via `viz::glyph` + `viz::palette`, `N/M pass` right-aligned), and an `EpisodeStatus → Status` mapping helper (per contracts/honeycomb.md table). Depends on T004, T005
- [X] T019 [US7] Migrate `bee-harness/src/repl/terminal.rs`: replace inline SGR literals (`"2"`,`"31"`,`"32"`,`"33"`,`"36"`,`"1;31"`) with `viz::palette::{SMOKE,STING,POLLEN,HONEY,ROYAL}` + `paint`/`bold`, and glyph literals (`▸ ✓ ✗ ⚠`) with `viz::glyph::*`. Byte-identical output — existing assertions unchanged (SC-012). Note: this file also gains `render_widget` in T015; land whichever story is second onto the other's edits. Depends on T004, T005
- [X] T020 [P] [US7] Add the episode/batch status grid to `bee-harness/src/batch.rs` (episode-summary output): render each episode's `EpisodeStatus` via `viz::status_grid` (per contracts/honeycomb.md AS-2). Depends on T018
- [X] T021 [US7] Make T017 green and re-run the **existing** `TerminalOutput` suite unchanged (`cargo test -p bee-harness --lib repl::`) to confirm SC-012; `cargo clippy -p bee-harness` clean

**Checkpoint**: chrome speaks one vocabulary; batch output has a status grid; no existing assertion changed.

---

## Phase 5: Polish & Cross-Cutting Concerns

- [X] T022 [P] Add example agent scripts under `specs/003-visual-render/examples/`: `bar-chart.rhai`, `dashboard.rhai` (a `vsplit` of a gauge + table), `dot-grid.rhai` (referenced by quickstart.md). (`pixel-bee.rhai`/`status-sprite.rhai` are Slice 2.)
- [X] T023 [P] Dependency-confinement gate (SC-019): `cargo tree -p bee-core | grep -E 'rhai|ratatui'` and `-p bee-common` both empty; record in quickstart.md §5
- [X] T024 Finalize the `render` tool `schema()` description in `bee-harness/src/tools/render.rs`: embed the Rhai primer + the full Slice-1 function list (per contracts/render-tool.md) so the model has the API in its tool definition
- [X] T025 Run the quickstart.md validation end-to-end (§1–§5) and update any doc drift; confirm SC-008…SC-014 + SC-019 coverage

---

## Dependencies & Execution Order

### Phase dependencies
- **Setup (P1)** → no deps.
- **Foundational (P2)** → after Setup; **blocks both stories** (palette/glyph are shared).
- **US6 (P3 phase, priority P2)** → after Foundational. The MVP.
- **US7 (P4 phase, priority P3)** → after Foundational. Independent of US6 except the shared
  `terminal.rs` file (T015 adds `render_widget`; T019 migrates literals — different lines, second one
  rebases onto the first).
- **Polish (P5)** → after the stories it validates.

### Key task-level dependencies
- T009 (`RenderSpec`) blocks T010, T011, T013, T014 (all consume the type).
- T012 (`RenderTool` + registry) depends on T010 + T011.
- T015 (`TerminalOutput::render_widget`) depends on T013 + T014.
- T018 (`grid`) depends on T004 + T005; T020 depends on T018.
- T016/T021 are the per-story green gates.

### Parallel opportunities
- Setup: T003 after T001/T002.
- Foundational: **T004 ∥ T005** (different files).
- US6 tests: **T006 ∥ T007 ∥ T008** (different test files, write-first).
- US6 impl: **T009**, then **T011 ∥ T013** (different files, both need only T009); T014 ∥ T011/T013.
- US7: **T017 ∥** (test) and **T018 ∥ T020-after-T018**; T019 independent file-wise except the T015 overlap.
- Cross-story: once Foundational lands, **US6 and US7 can be built in parallel by two people** (mind the `terminal.rs` overlap).

---

## Parallel Example: User Story 6

```bash
# Write the US6 tests together (fail first):
Task: "Rhai deny-by-default/limit tests in bee-harness/tests/render_sandbox.rs"     # T006
Task: "MockModel render-exchange test in bee-harness/tests/render_exchange.rs"      # T007
Task: "Buffer→ANSI pipeline tests in bee-harness/tests/render_ansi.rs"              # T008

# After T009 (RenderSpec), build these together:
Task: "Rhai builders + RenderContext + register_fn in bee-harness/src/render_api.rs"  # T011
Task: "render_to_ansi + buffer_render.rs in bee-harness/src/viz/buffer_render.rs"     # T013
```

---

## Implementation Strategy

### MVP first (US6)
1. Setup (T001–T003) → Foundational (T004–T005).
2. US6 (T006–T016) → **STOP and validate**: the agent renders static widgets; model sees summaries.
3. Demo the render tool with `specs/003-visual-render/examples/bar-chart.rhai`.

### Incremental delivery
- US6 (MVP) → then US7 (chrome + status grid) → then Polish. Each is independently testable; US7 is a
  behavior-preserving refactor and could equally ship first if preferred (it is the lower-risk half).

---

## Deferred to Slice 2 (separate plan — NOT built here)

The sprite + animation subsystem: `SpriteSpec`/`AnimationSpec` + the `RenderSpec::Sprite`/`Animation`
variants; `viz::sprite` half-block truecolor renderer with `quantize_256`/`quantize_16`/
`detect_color_mode` (FR-028, FR-030); the background tokio animator generalizing the spinner's
single-row reclaim to N rows (`\x1b[{N}A`, FR-030/031); `viz::bee` mascot + `bee_sprite()`/
`bee_animation()` Rhai fns (FR-032); the `--bee`/`BEE_MASCOT=1` startup art. Covers SC-015…SC-018.
Slice 1's `#[non_exhaustive]` `RenderSpec` and untouched spinner machinery are the seams it builds on.

---

## Notes
- [P] = different files, no incomplete dependency.
- The `render` tool never touches the enforcement path — no VM/BPF-LSM test is needed (contrast 002).
- ⛔FAIL-FIRST tests (T006) validate the fail-closed Rhai boundary (Constitution I) before it exists.
- Commit after each task or logical group; stop at any checkpoint to validate a story independently.
