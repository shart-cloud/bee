# Implementation Plan: bee Visual Rendering — Slice 2 (sprites, animation, the bee)

**Branch**: `003-visual-render` | **Date**: 2026-07-20 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/003-visual-render/spec.md` (the sprite/animation
sections) + [plan.md](./plan.md) (Slice 1, shipped in commit `b519f15`).

## Summary

Slice 2 completes the 003 feature: the **sprite + animation subsystem** deferred from Slice 1. It adds
pixel-art sprites and frame animations to the honeycomb design language and the `render` tool, plus the
built-in bee mascot. Concretely (FR-028…FR-032, SC-015…SC-018):

- **`SpriteSpec` / `AnimationSpec`** data types + additive `RenderSpec::Sprite` / `RenderSpec::Animation`
  variants (the `#[non_exhaustive]` enum grows without breaking Slice 1).
- **`viz::sprite`** — the half-block renderer (`▄`/`▀` U+2584/2580 with truecolor SGR), with
  `quantize_256` / `quantize_16` / `detect_color_mode` graceful degradation.
- **`viz::animator`** — a background tokio task that redraws frames in place, generalizing the
  spinner's single-row reclaim to `N` rows and sharing one "active animation" slot with the spinner
  (FR-031: only one animated element at a time).
- **Rhai API** — `palette()`/`pal.set`, `sprite()`/`paint`/`set`/`fill`, `animation()`/`add`/`bounce`/
  `cycles`, and `bee_sprite()`/`bee_animation()` — additive registrations on Slice 1's engine.
- **`viz::bee`** — the 16×16 bee sprite + 3-frame wing-flap animation, wired into an opt-in REPL
  startup banner (`--bee` / `BEE_MASCOT=1`) and the episode-completion line.

Technical approach: sprite rendering is **hand-rolled ANSI, no ratatui and no new crate** — the data
model stays ratatui/rhai-free (NFR-002). The animator **reuses the spinner's exact mechanism**
(`spinning`/`reclaim`/`spin_task`), generalized from 1 row to `⌈H/2⌉` rows via `reclaim_rows:
AtomicUsize`; the single shared flag makes mutual exclusion (FR-031) structural. See
[research-slice2.md](./research-slice2.md) (D12–D18).

## Technical Context

**Language/Version**: Rust 2021, stable (1.85). All work in `bee-harness`. No new nightly, no new
dependency (sprites are ANSI strings; the animator uses the tokio already present).

**Primary Dependencies**: unchanged from Slice 1 — `rhai` 1.25 (`only_i64`, `sync`), `ratatui` 0.29
(static widgets only; **not** used for sprites), `libc`, `tokio`, `rustyline` 17.

**Storage**: `SpriteSpec`/`AnimationSpec` are serde types nested in `RenderSpec`, recorded in the
transcript like every other widget. No new on-disk format.

**Testing**: `cargo test` on the host. Sprite rendering, quantization, and color-mode detection are
pure functions — unit-tested directly (SC-015). The animator is factored into a pure
seam — `playback(spec)` (frame-index order) and `redraw_block(rows)` (`\x1b[{N}A` + N lines) — so
SC-016 is tested as data, not through real time (research D14); a `CapturePrinter` covers the
synchronous first-paint + reclaim, and the mutual-exclusion invariant (starting an animation clears
`spinning` — SC-017) is a synchronous assertion.
`bee_sprite()`/`bee_animation()` are exercised from a Rhai script (SC-018). No terminal, no network.

**Target Platform**: same as the harness. Degrades on non-truecolor terminals (256→16→mono) and
off-tty (monochrome block chars) — never breaks (Edge Cases).

**Project Type**: library work in `bee-harness` + its REPL binary.

**Performance Goals**: frame intervals clamped 50–1000 ms (FR-029). The animator sleeps between frames
and holds the printer lock only for one redraw — no busy-loop, no runtime block. Half-block render of a
16×16 sprite is ~128 cells → well under a frame budget.

**Constraints**: sprite ≤ 32×32 px (16×16 default/recommended), ≤ 16 frames/animation, ≤ 32 palette
entries. Only one animated element (spinner **or** sprite animation) active at once (FR-031). Truecolor
required for full fidelity; quantize otherwise. The animator must not regress the spinner (its tests
pass unchanged — SC-012 for the spinner path).

**Scale/Scope**: Slice 2 = 3 new `viz` submodules (`sprite_render`, `animator`, `bee`), 2 additive
`RenderSpec` variants, ~10 additive Rhai functions, and the REPL mascot wiring. Completes feature 003.

## Constitution Check

*GATE: evaluated against Principles I–V + Security/Platform + Workflow.*

| Principle | Assessment |
|-----------|------------|
| **I. Deny-by-Default & Fail-Closed** | PASS. The new Rhai functions are still drawing-only; caps (32×32, 16 frames, 32 palette entries, 50–1000 ms) are enforced by the registered functions and surface as script errors. No new capability is exposed — sprites produce ANSI strings, nothing more. |
| **II. Capability Attenuation** | N/A. Not in the enforcement path. |
| **III. Kernel Enforcement Is Authoritative** | PASS. Still no I/O in the `render` tool. The animator writes to the terminal via the same `ExternalPrinter` the spinner uses — a UI side effect in the harness process, not a sandboxed-tool capability. The `Sandbox` parameter remains ignored (FR-022). |
| **IV. Policy-as-Data** | PASS. `render` availability is still declarative (scenario/REPL `tools`); the mascot is opt-in via a flag/env var, not policy. |
| **V. Library-First, Runtime-Free Core** | PASS — **load-bearing**. No new dependency; `bee-core`/`bee-common` remain untouched (SC-019 still holds). Sprite rendering adds **no** crate — it is hand-rolled ANSI. The animator uses the tokio already confined to `bee-harness`. |
| **Security/Platform** | PASS. A malicious script can now also burn ops building a 32×32 sprite or a 16-frame animation — still bounded by the Rhai op limit and the API caps; the worst outcome is a garbled or annoying animation, reclaimed on the next output. No new attack surface. |
| **Workflow / Test-first** | PASS. Sprite/quantization/color-mode are pure and unit-tested; the animator's redraw/reclaim/mutual-exclusion invariants are `CapturePrinter`-tested. The spinner's existing tests must pass unchanged after the `reclaim_rows` generalization (regression gate). |

**Result: no violations.** Complexity Tracking empty.

## Project Structure

### Documentation (this slice)

```text
specs/003-visual-render/
├── plan-slice2.md         # This file
├── research-slice2.md     # Phase 0 — D12–D18 (half-block, quantization, animator, bee)
├── data-model-slice2.md   # Phase 1 — SpriteSpec/AnimationSpec/ColorMode + additive variants + builders
├── contracts/
│   └── sprite-api.md       # the sprite/animation/palette/bee Rhai API + half-block + animator contract
└── tasks-slice2.md        # Phase 2 (authored here; /speckit-tasks would clobber Slice 1's tasks.md)
```

### Source Code (repository root)

```text
bee-harness/src/
├── render_spec.rs          # EDIT — add SpriteSpec, AnimationSpec, RenderSpec::Sprite/Animation;
│                           #        replace the wildcard helper arms with explicit Sprite/Animation arms
├── render_api.rs           # EDIT — PaletteBuilder/SpriteBuilder/AnimationBuilder custom-types +
│                           #        register palette()/sprite()/animation()/bee_sprite()/bee_animation();
│                           #        extend render()/layout.add() try_cast arms
├── viz.rs                  # EDIT — declare `pub mod sprite; pub mod bee; mod animator;`
├── viz/
│   ├── sprite_render.rs    # NEW — SpriteSpec → half-block ANSI lines; quantize_256/16; detect_color_mode; ColorMode
│   ├── animator.rs         # NEW — background tokio animation task (N-row generalization of the spinner)
│   └── bee.rs              # NEW — the 16×16 bee sprite + 3-frame wing-flap animation (OnceLock consts)
├── repl.rs                 # EDIT — `--bee`/BEE_MASCOT startup play; episode-completion static bee
└── repl/
    └── terminal.rs         # EDIT — reclaim: AtomicBool → reclaim_rows: AtomicUsize; emit() N-row reclaim;
                            #        render_widget handles Sprite (inline) + Animation (start_animation);
                            #        unify spinner + animation under one active slot (FR-031)
```

**Structure Decision**: still no new crate. Slice 2 is additive to the Slice 1 files plus three new
`viz` submodules. The one non-additive change is generalizing `TerminalOutput`'s reclaim state from a
bool to a count — carefully, so the spinner's existing tests pass unchanged (the regression gate).

## Complexity Tracking

> No Constitution violations — no entries. The `reclaim_rows` generalization is a refactor of existing
> state, not new complexity: it removes the special-case single-row assumption rather than adding a
> parallel mechanism.
