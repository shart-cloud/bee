# Implementation Plan: bee Visual Rendering & Design Language (Slice 1 — viz + static render tool + chrome)

**Branch**: `003-visual-render` | **Date**: 2026-07-20 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/003-visual-render/spec.md`

## Summary

Add a project-wide visual design language (`bee_harness::viz` — "honeycomb": palette, glyphs,
status grid) and a `render` tool that lets the agent declare visualizations by writing **Rhai**
scripts against a registered drawing API, rendered through a headless **ratatui** `Buffer` into the
existing rustyline `ExternalPrinter` path. The model sees a text summary, not the ANSI art.

This plan scopes **Slice 1**, covering **US6 (agent-declared visualization) for the static,
buffer-rendered widgets** and **US7 (unified visual chrome)** in full:

- **`viz` foundation** — the honeycomb palette, glyph vocabulary, `status_grid`, and the
  `RenderSpec` → ratatui `Buffer` → ANSI-lines pipeline (`render_to_ansi`).
- **US7 chrome migration** — `terminal.rs` migrates its inline SGR literals and glyph characters to
  `viz` constants (no behavioral change; SC-012), and batch/episode output gains a status grid.
- **US6 render tool (static widgets)** — a Rhai `Engine` with hard resource limits and a registered
  drawing API for the *non-animated* widgets: `bar_chart`, `line_chart`, `sparkline`, `table`,
  `gauge`, `dots`, `text`, `ascii_art`, `separator`, and `vsplit`/`hsplit` layout. The tool returns
  a `ToolResult` carrying an optional `RenderSpec`; `ReplOutput` gains `render_widget`.

**Deferred to Slice 2** (a separate plan): the **sprite + animation subsystem** — FR-028…FR-032,
the half-block truecolor renderer with 256/16-color quantization, the background tokio animator
(the multi-row `\x1b[{N}A` cursor-rewrite analogue of the spinner), and the built-in bee mascot
(`viz::bee`, `bee_sprite()`, `bee_animation()`). Slice 1's `RenderSpec` omits the `Sprite`/
`Animation` variants and is marked `#[non_exhaustive]` so Slice 2 adds them without breaking serde
or match sites.

Technical approach: `RenderSpec` is a **pure serde data type with no ratatui/rhai types in it**, so
it can be recorded in the transcript and re-rendered later, and so ratatui/rhai stay confined to
`bee-harness` (NFR-002, SC-019). The Rhai script builds opaque *builder* custom-types
(`ChartBuilder`, `TableBuilder`, `LayoutBuilder`, …) that accumulate into a shared `RenderContext`;
`render(widget)` commits, and the tool extracts the final `RenderSpec`. The `render` tool does
**no I/O** and ignores the `Sandbox` parameter (FR-022) — Rhai's own sandbox is its confinement,
independent of the kernel sandbox that confines `bash`/file tools.

## Technical Context

**Language/Version**: Rust 2021, stable (rust-version 1.85). All new code is in `bee-harness`
(already async/tokio). No new nightly requirement.

**Primary Dependencies**: `rhai` 1.x (`default-features = false`, `features = ["only_i64", "sync"]`)
for the sandboxed authoring engine; `ratatui` 0.29 (`default-features = false`) for headless
`Buffer`/`Rect`/widget layout math (no `crossterm`, no backend). Terminal-size detection reuses the
existing `libc` dep (`TIOCGWINSZ` ioctl) + `std::io::IsTerminal` — **no new terminal/color crate**
(consistent with the existing hand-written SGR approach). Implementation note: `ratatui` 0.29's exact
`unicode-width =0.2.0` pin conflicts with `rustyline` 18's exact `=0.2.2` pin, so `rustyline` is
pinned to **17** (no `unicode-width` dep, same API) — see research.md D3.

**Storage**: `RenderSpec` is serialized (serde) into the episode transcript as a new optional field
on `ToolResult`. No database; no new on-disk format.

**Testing**: `cargo test` on the host. US6 is validated deterministically with `MockModel` +
the test `Collector` (which overrides `render_widget` to capture the `RenderSpec`) — no network,
no terminal. Rhai limit/rejection paths (op limit → `ErrorTerminated`; unregistered fn →
"function not found") are unit-tested. US7 is validated by the **existing** `TerminalOutput` test
suite passing without assertion changes (SC-012) plus a new `status_grid` unit test (SC-013). The
`Buffer` → ANSI pipeline is unit-tested against a headless buffer (SC-011, SC-014). No BPF-LSM/VM
test is required — this feature never touches the enforcement path.

**Target Platform**: Same as the harness (Linux). Rendering degrades gracefully off-tty (piped/CI):
fall back to 80 columns and suppress color (spec Edge Cases).

**Project Type**: Rust library crate work inside the existing `bee-harness` member + its thin CLI
binaries (`bee-repl`, `bee-episode`). All logic in the library (Constitution V).

**Performance Goals**: NFR-001 — Rhai eval < 50 ms for ≤ 10,000 ops; `Buffer` → ANSI < 10 ms for a
100×30 buffer. Both paths are synchronous and fast; neither blocks the tokio runtime. SC-009 — a
`loop {}` script is terminated within 50 ms.

**Constraints**: NFR-002 — `rhai`/`ratatui` MUST NOT propagate to `bee-core`/`bee-common` (SC-019).
NFR-003 — `render` is opt-in per scenario, **not** in `DEFAULT_TOOLS`. FR-020 — resource limits are
non-negotiable by the script. FR-021 — no I/O/FFI/network/`eval` registered. Existing
`TerminalOutput` output contract is preserved where asserted (SC-012).

**Scale/Scope**: Slice 1 = one new tool, one Rhai engine (built once per session), ~10 static
widget types, the `viz` module, and a mechanical `terminal.rs` migration. Sprites, animation, the
tokio animator, truecolor quantization, and the bee mascot are Slice 2.

## Constitution Check

*GATE: evaluated against Principles I–V and the Security/Platform + Workflow constraints.*

| Principle | Assessment |
|-----------|------------|
| **I. Deny-by-Default & Fail-Closed** | PASS. The Rhai engine registers **only** the drawing functions; every other capability (I/O, FFI, network, `eval`, process control) is denied by omission (FR-021). Resource limits (FR-020) are set at engine construction and cannot be raised by a script. A script that references anything unregistered fails with "function not found" → `ToolResult { is_error: true }` (SC-010). A script that produces no widget is an error, not a silent blank (Edge Cases). |
| **II. Capability Attenuation** | N/A. The `render` tool is not in the enforcement path; it derives no scopes and interacts with no policies. |
| **III. Kernel Enforcement Is Authoritative** | PASS. The `render` tool ignores the `Sandbox` parameter (FR-022) — but this is **not** an enforcement bypass: the tool performs **no I/O of any kind**, so there is nothing for the kernel to enforce. Its confinement is the Rhai sandbox (bounded ops/memory, drawing-only API). The kernel sandbox continues to confine every tool that *does* touch the filesystem/network. The two sandboxes never interact (spec Security Boundaries). Documented rather than tracked as a violation. |
| **IV. Policy-as-Data** | PASS. `render`'s availability is declarative — a scenario opts in via its `tools` list (TOML). The Rhai API surface is fixed at compile time by the `register_fn` calls; a scenario cannot expand it. |
| **V. Library-First, Runtime-Free Core** | PASS — **the load-bearing gate**. `rhai` and `ratatui` are added **only** to `bee-harness`; `bee-core`/`bee-common` gain no dependency (SC-019). `RenderSpec` is a pure serde type with no ratatui/rhai types, so nothing leaks through the transcript. The `viz` module lives in `bee-harness`, not `bee-core`. The CLI binaries stay thin. |
| **Security/Platform** | PASS. Defense-in-depth: a model that is granted `render` cannot use it to read files, exec processes, or exfiltrate data — the worst a hostile script does is burn ≤ 10,000 Rhai ops (~50 ms) and draw a garbled chart. No credentials or env are exposed to the engine. |
| **Workflow / Test-first** | PASS. US7's contract is protected by the **existing** `TerminalOutput` tests passing without assertion changes (SC-012). New behavior (Rhai limits, unregistered-fn rejection, status grid, buffer→ANSI, layout height) has unit tests written against `MockModel`/`Collector`/headless buffers before implementation. No new attenuation logic ⇒ no new property tests. |

**Result: no violations.** Complexity Tracking below is empty. The FR-022 sandbox-bypass is
justified in-line above (no I/O ⇒ nothing to enforce) and needs no Complexity entry.

## Project Structure

### Documentation (this feature)

```text
specs/003-visual-render/
├── plan.md              # This file
├── research.md          # Phase 0 — Rhai engine config, headless ratatui, Buffer→ANSI, width detection, decisions
├── data-model.md        # Phase 1 — RenderSpec (Slice 1 variants) + builders + RenderContext + viz constants
├── quickstart.md        # Phase 1 — validate US6 (mock render) + US7 (chrome migration + status grid)
├── contracts/
│   ├── render-tool.md       # the render tool I/O contract (Rhai script in → RenderSpec + summary out)
│   ├── rhai-api.md          # the agent-facing Rhai drawing API (Slice 1 functions; sprites marked Slice 2)
│   └── honeycomb.md         # the design language: palette, glyphs, status grid, layout rules
└── tasks.md             # Phase 2 (/speckit-tasks — NOT created here)
```

### Source Code (repository root)

```text
bee-harness/
├── Cargo.toml               # ADD: rhai (default-features=false, only_i64, sync), ratatui (default-features=false)
└── src/
    ├── lib.rs               # re-export viz + RenderSpec on the public surface
    ├── render_spec.rs       # NEW — RenderSpec enum (serde, #[non_exhaustive]) + Bar/Series/Point/Row/Dot/Direction
    ├── render_api.rs        # NEW — builder custom-types + RenderContext + all register_fn calls
    ├── tools.rs             # EDIT — ToolResult gains `render_spec: Option<RenderSpec>`; is_known_tool + registry_for learn "render"
    ├── tools/
    │   └── render.rs        # NEW — the RenderTool: holds Arc<Engine>, evaluates the script, builds the summary
    ├── viz.rs               # NEW — module root: re-exports palette/glyph/grid; render_to_ansi (Buffer→Vec<String>)
    ├── viz/
    │   ├── palette.rs       # NEW — honeycomb color constants + paint/bold/is_color_enabled
    │   ├── glyph.rs         # NEW — glyph constants (DOT_PASS, ARROW, CHECK, …)
    │   ├── grid.rs          # NEW — status_grid(results) renderer + Status enum
    │   └── buffer_render.rs # NEW — RenderSpec → ratatui Buffer → ANSI lines; width/tty detection
    ├── repl.rs              # EDIT — ReplOutput gains render_widget (default = ASCII fallback via info()); exchange loop renders render_spec
    └── repl/
        └── terminal.rs      # EDIT — migrate SGR/glyph literals to viz::*; implement render_widget via render_to_ansi
```

**Structure Decision**: No new crate. All work lands in the existing `bee-harness` member —
the only crate permitted to carry `rhai`/`ratatui` (Constitution V, NFR-002). `viz` is a submodule
of `bee-harness`. The design-language constants (`viz::palette`, `viz::glyph`) are the single source
of truth that both the chrome (`terminal.rs`) and the `render_to_ansi` pipeline draw from. The
`render` tool slots into the existing `Tool`/`ToolRegistry`/`registry_for` machinery unchanged in
shape — it is just a `Tool` whose `call` ignores `sandbox`.

**Note on the spec's file list**: the spec's "Modified files" attributes the new `render_spec`
field to `transcript.rs`; the `ToolResult` type actually lives in **`tools.rs`** (transcript.rs only
consumes it). The field is added in `tools.rs`; the transcript inherits it via `RecordedCall.result`
with no change to `transcript.rs`.

## Complexity Tracking

> No Constitution violations — no entries. (The FR-022 sandbox-bypass is justified in the
> Constitution Check above: the render tool does no I/O, so there is nothing for the kernel to
> enforce; it is not a weakening of any enforced guarantee.)
