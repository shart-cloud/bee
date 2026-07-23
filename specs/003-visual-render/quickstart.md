# Quickstart: bee Visual Rendering (Slice 1)

> **Command names changed.** The consolidation (ADR-0002) replaced `bee-episode`, `bee-repl`, and
> `bee-metrics` with subcommands of the single `bee` executable: `bee run`, `bee repl`, and
> `bee metrics`. The raw process runner moved from `bee run` to `bee exec`, and a session with no
> policy now needs an explicit `--host`. The commands below are recorded as this feature shipped
> them; translate accordingly.

Validation guide for Slice 1 — the `viz` foundation, US7 chrome migration, and the US6 `render` tool
for static widgets. All checks run on the host with `cargo test` (no network, no keys, no VM). See
`contracts/` for the I/O details and `data-model.md` for the types.

## Prerequisites

- The workspace builds: `cargo build -p bee-harness`.
- New deps present in `bee-harness/Cargo.toml`:
  `rhai = { version = "1", default-features = false, features = ["only_i64", "sync"] }` and
  `ratatui = { version = "0.29", default-features = false }`.

## 1. US7 — chrome migration is behavior-preserving (SC-012)

The existing `TerminalOutput` tests must pass **without assertion changes** after `terminal.rs`
migrates to `viz::palette`/`viz::glyph`:

```bash
cargo test -p bee-harness --lib repl::            # existing TerminalOutput suite, unchanged
```

Expected: all pre-existing assertions on rendered glyphs/colors pass — the constants produce the same
bytes the inline literals did.

## 2. US7 — status grid (SC-013)

```bash
cargo test -p bee-harness --lib viz::grid
```

Expected: given 4 results (3 `Pass`, 1 `Fail`), the rendered string contains **3 green `●`** and
**1 red `●`** (assert on the `\x1b[32m…●` and `\x1b[31m…●` SGR+glyph sequences), names left-aligned,
`3/4 pass` right-aligned. With `NO_COLOR=1`, the same call yields the dots with **no** SGR codes
(AS-3).

## 3. US6 — the render tool via MockModel (SC-008)

The headline end-to-end check: a scripted model turn calls `render`, and the test `Collector`
captures the `RenderSpec`.

```bash
cargo test -p bee-harness --lib render            # render tool + api + spec tests
```

Covers:

- **SC-008** — `MockModel` emits a `render` call whose script builds a 5-bar chart. Assert: (a) the
  `Collector` received `render_widget` with `RenderSpec::BarChart { bars, .. }` of length 5 and the
  expected labels/values; (b) the `ToolResult.content` returned to the model is the text summary
  (`Rendered a bar chart 'File Sizes' with 5 bars.`) and contains **no** ANSI escape; (c)
  `result.render_spec.is_some()`.
- **SC-009** — a script of `loop {}` returns `ToolResult { is_error: true }` whose `content` mentions
  the operation limit, and the eval returns within 50 ms.
- **SC-010** — a script calling an unregistered function (`import "std";` or `std::fs::read(".")`)
  returns `ToolResult { is_error: true }` with a Rhai "function not found" message.
- **Edge** — a script that commits no `render()` → `is_error: true`, "script produced no
  visualization"; a script calling `render()` twice → last widget wins, summary notes the discard.

## 4. The `Buffer` → ANSI pipeline (SC-011, SC-014)

```bash
cargo test -p bee-harness --lib viz::buffer_render
```

Covers:

- **SC-011** — a `RenderSpec::BarChart` renders to inline ANSI lines (a `Vec<String>`) that contain
  box-drawing characters and colored bars, produced **without** entering alt-screen or raw mode
  (headless `Buffer` only).
- **SC-014** — a `vsplit` layout containing a gauge and a table renders to a buffer whose height is
  the sum of the children's heights plus separators, and whose width fills the available columns.

## 5. Dependency confinement (SC-019)

```bash
cargo tree -p bee-core   | grep -E 'rhai|ratatui' && echo "LEAK" || echo "clean"
cargo tree -p bee-common | grep -E 'rhai|ratatui' && echo "LEAK" || echo "clean"
```

Expected: both print `clean` — `rhai`/`ratatui` appear only in `bee-harness`.

## 6. Try it live (optional)

With a `render`-enabled scenario/REPL config (add `"render"` to the `tools` list — it is opt-in,
NFR-003), start the REPL and prompt the model to visualize something it gathered:

```bash
cargo run -p bee-harness --bin bee-repl -- --config <cfg-with-render-tool>.toml
```

The chart appears inline in the scrollback; the model's context shows only the one-line summary.
Piping the output (non-tty) falls back to 80 columns and drops color.

---

## Success-criteria coverage (Slice 1)

| SC | Where validated |
|----|-----------------|
| SC-008 | §3 render tool via MockModel |
| SC-009 | §3 op-limit termination |
| SC-010 | §3 unregistered-function rejection |
| SC-011 | §4 BarChart → inline ANSI, no alt-screen |
| SC-012 | §1 chrome migration, existing tests unchanged |
| SC-013 | §2 status grid 3 pass / 1 fail |
| SC-014 | §4 vsplit height = children + separators |
| SC-019 | §5 rhai/ratatui absent from bee-core/bee-common |

**Deferred to Slice 2**: SC-015 (sprite half-block), SC-016 (animation redraw + reclaim), SC-017
(animation stops spinner), SC-018 (`bee_sprite()`/`bee_animation()`).
