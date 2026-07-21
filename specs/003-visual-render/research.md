# Research: bee Visual Rendering & Design Language (Slice 1)

Phase 0 decisions for the `viz` module, the `render` tool's Rhai engine, and the headless ratatui
`Buffer` → ANSI pipeline. Slice 1 = static widgets + chrome migration; sprites/animation are Slice 2
(their research — half-block truecolor, 256/16 quantization, the tokio animator — is deferred).

All version/feature claims below were verified with `cargo add --dry-run` against the crates.io index
on 2026-07-20.

---

## D1 — Authoring surface: Rhai, not a JSON widget enum

**Decision**: The `render` tool takes a **Rhai script** (`{ "script": string }`) that calls a
registered drawing API, not a fixed `{ "kind": "...", "data": [...] }` JSON schema.

**Rationale**: A JSON enum makes the agent a slot-filler and turns every new visualization into a
Rust enum variant + schema change + recompile. Rhai gives the agent an *authoring* surface with a
controlled blast radius: it can sort, filter, compute derived values, and compose layouts, but the
only side effects it can produce are calls into the drawing API. Rhai's safety properties make this
viable — no I/O/FFI/network unless `register_fn` exposes it, a don't-panic guarantee, hard resource
limits enforced at the engine level, and pure safe Rust (no C deps).

**Alternatives considered**: (a) JSON widget enum — rejected: fixed menu, no derived logic, schema
churn. (b) A full scripting language with I/O (Lua via `mlua`, Python via `pyo3`) — rejected: pulls
a C runtime and an I/O surface we would then have to fence off; Rhai starts closed. (c) A DSL of our
own — rejected: we would reinvent parsing, limits, and error reporting that Rhai already ships.

---

## D2 — `rhai` crate configuration

**Decision**: `rhai = { version = "1", default-features = false, features = ["only_i64", "sync"] }`
(resolves to **v1.25.1**).

- `default-features = false` — drops `std` conveniences we do not want and keeps the surface minimal.
  Critically, it does **not** enable `unchecked` (which would *disable* the safety limits — never
  use it).
- `only_i64` — disables the extra integer types; Rhai's number type is `i64` (bar values, cycles,
  dimensions). Cuts code size. Float (`f64`) is still available for line-chart points and gauges.
- `sync` — makes Rhai's shared/`Engine` types `Send + Sync` (shared cells become `Arc`/`RwLock`).
  **Required**, because `Tool: Send + Sync` and the `RenderTool` holds the `Engine` in an
  `Arc<Engine>` built once per session (spec Edge Case: "the `Engine` is built once per REPL session,
  not per tool call"). Without `sync`, `Engine` is `!Sync` and cannot live behind `Arc` in a
  `Box<dyn Tool>`.

**Registered/disabled surface** (FR-021): register only the drawing functions from
`contracts/rhai-api.md`. Call `engine.disable_symbol("eval")` to remove the `eval` keyword. Register
**no** `print`/`debug` sinks that touch I/O (Rhai's `on_print`/`on_debug` default to no-ops unless we
wire them). No module resolver is installed, so `import` resolves nothing → error (SC-010).

**Resource limits (FR-020), set once at construction, non-negotiable by scripts**:

| Limit | Rhai setter | Default |
|-------|-------------|---------|
| max operations | `set_max_operations(10_000)` | 10,000 |
| max call depth | `set_max_call_levels(16)` | 16 |
| max expr depth (global / fn) | `set_max_expr_depths(32, 16)` | 32 / 16 |
| max string size | `set_max_string_size(64 * 1024)` | 64 KB |
| max array size | `set_max_array_size(1_000)` | 1,000 |
| max map size | `set_max_map_size(100)` | 100 |

`set_max_operations` is what terminates `loop {}` — Rhai raises `EvalAltResult::ErrorTooManyOperations`
after the budget, which the tool maps to `ToolResult::error` mentioning the operation limit (SC-009).
The budget of 10,000 ops is reached well under 50 ms (NFR-001).

**Alternatives**: build the engine per `call()` — rejected: the spec mandates once-per-session, and
`register_fn` for ~25 functions on every render call is needless work. A `thread_local!` engine —
rejected: `call` is async on a multi-threaded runtime, so thread affinity is unreliable; `sync` +
`Arc<Engine>` is simpler and correct.

---

## D3 — Headless ratatui rendering

**Decision**: `ratatui = { version = "0.29", default-features = false }` (resolves to **v0.29.0**).
Render every widget to an in-memory `Buffer`, never to a terminal backend.

- `default-features = false` drops the `crossterm` backend (and `termion`/`termwiz`). We do **not**
  need a backend: `Buffer::empty(Rect { x:0, y:0, width, height })` gives a headless cell grid, and
  `Widget::render(area, &mut buf)` fills it. No alt-screen, no raw mode, no `crossterm` — no fight
  with rustyline (which owns the real terminal).
- The standard widgets we use — `BarChart`, `Table`, `Sparkline`, `Gauge`, `Chart` (for line
  charts), `Paragraph` (for `text`/`ascii_art`), and `Block` (borders/titles) — are **core**, not
  behind the `all-widgets`/`widget-calendar` features. Verified: the only feature-gated widget in
  0.29 is the calendar. `unicode-width` is a hard dependency of ratatui, so CJK/emoji label widths
  are handled correctly (spec Edge Case) for free.
- `Layout` with `Direction::Vertical`/`Horizontal` + `Constraint` does the `vsplit`/`hsplit` math.

**Rationale**: ratatui widgets are mature and correct about Unicode width, color, and layout —
re-deriving that in hand-written ANSI is a "mistake factory" (spec). Rendering to a `Buffer` gives us
a clean seam: the widget tree is ratatui's problem; turning cells into ANSI lines is ours (D4).

**Alternatives**: hand-rolled ANSI charts — rejected (correctness). `tui-rs` (unmaintained
predecessor) — rejected. Enabling the `serde` feature on ratatui to serialize its `Buffer` —
rejected: we serialize our own `RenderSpec`, never ratatui types (keeps ratatui out of the data
model, D5).

**Dependency-resolution note (discovered at implementation)**: `ratatui` 0.29 pins
`unicode-width =0.2.0` (exact), while the incumbent `rustyline` **18** pins `unicode-width =0.2.2`
(exact) — an unresolvable conflict (two exact pins on the same crate). `rustyline` **17.0.2** carries
no `unicode-width` dependency and exposes the same `DefaultEditor`/`ExternalPrinter` API the REPL
uses, so `bee-harness` is pinned to `rustyline = "17"`. This is the only dependency deviation from the
original plan; all existing REPL tests pass unchanged under 17.

---

## D4 — `Buffer` → ANSI lines (`viz::render_to_ansi` / `buffer_render.rs`)

**Decision**: After a widget tree renders into a `Buffer`, walk it row by row and emit one ANSI
string per row, then hand the `Vec<String>` to the existing `ExternalPrinter` path.

Algorithm per row: iterate cells left→right; track the current `(fg, bg, modifier)`; when a cell's
style differs from the active one, emit the SGR sequence for the new style (run-length style batching
so we do not re-emit an escape per cell); append the cell's `symbol()`; at end of row emit `\x1b[0m`
and terminate the line (no trailing newline — the printer adds it, matching `emit`).

**Color mapping (Slice 1)**: widget colors come from the honeycomb palette, which is **basic
3/4-bit ANSI** (`honey`=33, `pollen`=32, `sting`=31, `royal`=36, `smoke`=2). Map ratatui `Color`
variants (`Green`, `Red`, `Yellow`, `Cyan`, `DarkGray`, `Reset`) to the corresponding SGR code —
the same codes `terminal.rs::paint` already emits. **No truecolor** (`38;2;…`) is produced in
Slice 1; truecolor is introduced only by the sprite renderer in Slice 2. This keeps the whole Slice 1
color surface identical in kind to what the REPL already emits, so `ExternalPrinter` (which already
handles SGR + box-drawing) needs no changes (spec Assumption).

**`NO_COLOR` / off-tty**: when color is disabled (D9), the row walk emits **symbols only**, no SGR —
so box-drawing glyphs and bars are still present, colors are not (SC / spec AS-3 for `viz`).

**Width/height caps**: the buffer's `Rect` is sized to `min(terminal_width, 120)` × `min(spec
height, 40)` (Rendering-API constraints). Data exceeding the visual capacity is elided by the widget
(ratatui clips) with a note appended to the tool summary (Edge Case: 200 bars at width 80).

**Rationale**: row-major cell walk with style batching is the standard headless-render idiom; it is
O(width×height) and comfortably inside the 10 ms budget for 100×30 (NFR-001).

---

## D5 — `RenderSpec`: a ratatui/rhai-free serde data type

**Decision**: `RenderSpec` (and its `Bar`/`Series`/`Point`/`Row`/`Dot`/`Direction` members) is a
plain serde enum with **no ratatui or rhai types inside it**. The Rhai builders produce a
`RenderSpec`; `render_to_ansi` consumes one and *then* constructs ratatui widgets transiently.

**Rationale**: three payoffs. (1) `RenderSpec` is the `ToolResult.render_spec` field recorded in the
transcript — it must serialize without dragging ratatui into the serde graph. (2) NFR-002/SC-019 —
keeping ratatui/rhai types out of `RenderSpec` guarantees nothing leaks toward `bee-core` through the
transcript types. (3) A future web transcript viewer can re-render from the spec without ratatui
(spec Assumption).

**`#[non_exhaustive]`**: the enum is marked `#[non_exhaustive]` and Slice 1 omits the `Sprite` and
`Animation` variants. Slice 2 adds them additively; external match sites already must have a
wildcard arm, so no break. `serde(tag = "kind", rename_all = "snake_case")` gives stable,
human-diffable JSON.

---

## D6 — Rhai builders + `RenderContext` accumulation

**Decision**: Each drawing function returns an opaque builder **custom type** registered on the
engine (`ChartBuilder`, `TableBuilder`, `GaugeBuilder`, `DotGridBuilder`, `TextBuilder`,
`LayoutBuilder`, `SeriesHandle`). Methods (`.bar(...)`, `.row(...)`, `.add(...)`) mutate the builder.
`render(widget)` commits the builder's `RenderSpec` into a shared `RenderContext`; after
`eval_with_scope`, the tool calls `ctx.take()` to extract it.

- `RenderContext` is `Clone + Send` (an `Arc<Mutex<Option<RenderSpec>>>` interior, aligned with the
  `sync` feature). It is pushed into the Rhai `Scope` (or captured by the registered closures) before
  eval and accumulates the committed widget.
- **Multiple `render()` calls** (Edge Case): last write wins; `ctx` records that earlier renders were
  discarded so the tool can note it in the summary.
- **No `render()` call** (Edge Case): `ctx.take()` is `None` → `ToolResult::error("script produced no
  visualization")`.
- **API-enforced constraints** (separate from Rhai limits): builder methods return a Rhai error when
  a cap is exceeded — layout nesting > 3, total bars/rows/points > 500 across the script, computed
  width > 120 or height > 40. Rhai surfaces the returned `Err` as a script error → `ToolResult::error`
  (spec "Constraints (enforced by the API, not Rhai limits)").

**Rationale**: builders keep the Rhai-facing surface fluent and familiar (JavaScript-adjacent), while
the accumulator gives a single, testable extraction point. Registered closures capturing the
`Arc<Mutex<…>>` is the idiomatic Rhai pattern for "functions with shared state".

**Alternatives**: have each function return a fully-built `RenderSpec` `Dynamic` and make `render`
the only stateful call — workable, but layouts (`layout.add(widget)`) need to accept and nest other
widgets, which is cleaner with typed builders than with untyped `Dynamic` juggling.

---

## D7 — The `render` tool ignores the `Sandbox` (FR-022) — and why that is safe

**Decision**: `RenderTool::call(&self, args, _sandbox)` never touches `sandbox`. It evaluates Rhai
in-process and returns.

**Rationale / constitution**: this looks like it grazes Principle III (kernel enforcement is
authoritative), so it is justified explicitly: the render tool performs **no I/O** — no file, no
process, no socket. There is nothing for the kernel LSM to mediate, so bypassing the cgroup sandbox
removes no guarantee. Confinement is provided by Rhai's own sandbox (bounded ops/memory, drawing-only
API). This is defense-in-depth atop the existing sandbox, two independent threat models that never
interact (spec Security Boundaries). Recorded in the plan's Constitution Check as PASS, not as a
tracked violation.

---

## D8 — `ReplOutput::render_widget` default = ASCII fallback via `info()`

**Decision**: Add `fn render_widget(&self, spec: &RenderSpec)` to `ReplOutput` with a **default impl**
that formats the spec as plain-text ASCII (a text table / labelled rows) and routes it through the
already-required `self.info(&text)`. `TerminalOutput` overrides it to call `render_to_ansi` and emit
the ANSI lines through the `ExternalPrinter`.

**Reconciling US6's two acceptance criteria**: AS-5 says a `ReplOutput` that does *not* override
`render_widget` (e.g. a plain `Collector`) must still emit an ASCII fallback (not a blank) — the
`info()`-routed default satisfies this. The US6 Independent Test wants the test to *capture* the
`RenderSpec` — so the **test `Collector` overrides** `render_widget` to record the spec into its
buffer for assertion. Both hold: the default provides the fallback; the test double opts into
capture. `ReplOutput` is a **plain (non-async) trait** today, so this is a straightforward defaulted
method (mirrors the existing `busy_start`/`footer` defaults) and leaves the six required methods
untouched.

**Exchange-loop wiring**: in `run_exchange`, immediately after the `output.tool_result(&result,
&audit)` call, add `if let Some(spec) = &result.render_spec { output.render_widget(spec); }`. The
`ToolResult.content` sent back to the model stays the text summary (FR-023) — the render never
pollutes the model's context.

---

## D9 — Terminal width & tty detection (new capability, no new crate)

**Decision**: Add a small `terminal_dims()` helper in `buffer_render.rs` that returns
`(width: u16, is_tty: bool)`:

- tty check: `std::io::stdout().is_terminal()` (`std::io::IsTerminal`, stable since Rust 1.70; our
  MSRV is 1.85).
- width: a `TIOCGWINSZ` `ioctl` via the **already-present `libc`** dependency (`libc::ioctl(1,
  libc::TIOCGWINSZ, &mut winsize)`), reading `ws_col`. Honor `COLUMNS` env if set (CI convenience).
- fallback: when not a tty or the ioctl fails, **80 columns and color suppressed** (as if `NO_COLOR`
  — spec Edge Case "Terminal width unknown").

**Rationale**: the recon confirmed the harness has *no* width/tty detection today (only a hardcoded
`WRAP_WIDTH = 88` for prose). Rather than add `crossterm`/`terminal_size`/`is-terminal`, reuse
`libc` (already a dependency) + `std::io::IsTerminal` — zero new crates, consistent with the
"no color crate" posture (spec "What is NOT added"). This helper is also what caps render width to
`min(width, 120)` (D4).

---

## D10 — `viz` module + chrome migration (US7)

**Decision**: Create `bee_harness::viz` as the single source of truth for palette/glyph/status-grid,
then migrate `terminal.rs` to reference it — a **mechanical, behavior-preserving** refactor.

- `viz::palette` — `HONEY="33"`, `POLLEN="32"`, `STING="31"`, `SMOKE="2"`, `ROYAL="36"` constants +
  `paint(code, text)`, `bold(code, text)`, `is_color_enabled()`. These reproduce exactly what
  `terminal.rs::paint` already does, so the migration swaps `self.paint("32", …)` call sites for
  `viz::palette::paint(viz::palette::POLLEN, …)` with identical output bytes (SC-012).
- `viz::glyph` — the existing `▸ ✓ ✗ ⚠` plus the new `● ○ ⬡ ─` constants. `terminal.rs` swaps its
  glyph literals for these constants; the characters are unchanged, so assertions on rendered text
  pass without edits (SC-012 AS-1).
- `viz::status_grid(results: &[(String, Status)]) -> String` — renders the pass/fail dot grid used by
  batch/episode output and available to the Rhai `dots()` widget path. New unit test: 3 pass + 1 fail
  → 3 green `●` + 1 red `●`, verified by SGR-code presence (SC-013, AS-2). Scenario names
  left-aligned, `N/M pass` fraction right-aligned.

**`NO_COLOR` consistency**: `is_color_enabled()` reads `NO_COLOR` exactly as `terminal.rs` does today
(any value ⇒ off), so migrated call sites keep their current `NO_COLOR` behavior (spec AS-3).

**Rationale**: doing US7 in Slice 1 (before/with the render tool) means both the chrome and the
`render_to_ansi` pipeline consume one palette/glyph vocabulary from day one — no second migration
later. It is low-risk because it is assertion-preserving and can be done one call site at a time.

---

## D11 — Slice boundary: what Slice 2 inherits

**Decision**: Slice 1 deliberately builds the seams Slice 2 needs, then stops:

- `RenderSpec` is `#[non_exhaustive]` → Slice 2 adds `Sprite`/`Animation` variants additively (D5).
- `render_to_ansi` dispatches on `RenderSpec` variants → Slice 2 adds sprite/animation arms; Slice 1
  arms are untouched.
- The spinner's reclaim mechanism (single-row `\x1b[1A` + `reclaim: AtomicBool`) is **not** modified
  in Slice 1 — static widgets emit inline with no cursor rewrite. Slice 2 generalizes it to N rows
  (`\x1b[{N}A` + a row-count) for the animator, reusing the exact `spinning`/`spin_task`/`reclaim`
  pattern (spec FR-030/031). Keeping Slice 1 off that machinery is why the two slices are cleanly
  separable.
- Rhai `sync` + `Arc<Engine>` (D2) is already in place, so Slice 2's `palette()`/`sprite()`/
  `animation()`/`bee_sprite()`/`bee_animation()` registrations are additive `register_fn` calls.

**Open items for Slice 2 research (not resolved here)**: half-block `▄` rendering with per-cell
truecolor fg/bg; `quantize_256`/`quantize_16` nearest-color in the xterm cube; `detect_color_mode()`
from `COLORTERM`; the animator's interaction with the spinner (only one animated element at a time,
FR-031) and `CapturePrinter`-based tests for cursor-up sequences (SC-016, SC-017).

---

## Summary of decisions

| # | Decision | Key rationale |
|---|----------|---------------|
| D1 | Rhai script authoring surface | authoring > slot-filling; starts closed |
| D2 | `rhai` 1.25 `default-features=false` + `only_i64` + `sync`; limits at construction | Engine must be `Send+Sync` for `Arc<Engine>`; limits non-negotiable |
| D3 | `ratatui` 0.29 `default-features=false`, headless `Buffer` | mature widget/Unicode math, no backend, no crossterm |
| D4 | Row-major `Buffer`→ANSI with style batching; basic ANSI only | matches existing `paint` SGR; `ExternalPrinter` unchanged |
| D5 | `RenderSpec` is ratatui/rhai-free serde, `#[non_exhaustive]` | transcript-serializable; NFR-002; Slice 2 grows it additively |
| D6 | Builder custom-types + `RenderContext` accumulator | fluent API; single extraction point; API-level caps |
| D7 | render tool ignores `Sandbox` (no I/O ⇒ nothing to enforce) | Constitution III justified, not violated |
| D8 | `render_widget` default = ASCII fallback via `info()`; test `Collector` overrides | satisfies AS-5 fallback + Independent-Test capture |
| D9 | `IsTerminal` + `libc` `TIOCGWINSZ`, fallback 80/no-color | no new crate; matches "no color crate" posture |
| D10 | `viz` single source of truth; mechanical `terminal.rs` migration | one vocabulary for chrome + render; SC-012 preserving |
| D11 | Slice-1 seams (`#[non_exhaustive]`, untouched spinner) for Slice-2 sprites | clean, additive slice boundary |
