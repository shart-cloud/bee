# Data Model: bee Visual Rendering (Slice 1)

Entities introduced by Slice 1. All live in `bee-harness` (Constitution V / NFR-002). Types are
grouped by role: the **serde data model** (`RenderSpec` and members — recorded in the transcript),
the **Rhai builder types** (opaque custom types the script manipulates), and the **`viz` module**
constants/functions. Sprite/animation entities (`SpriteSpec`, `AnimationSpec`, and the `Sprite`/
`Animation` `RenderSpec` variants) are **Slice 2** and are intentionally absent here.

---

## 1. `RenderSpec` (enum, serde) — `src/render_spec.rs`

The validated, renderable output of a Rhai script. Produced by the `render` tool, consumed by
`ReplOutput::render_widget` and `viz::render_to_ansi`. **No ratatui/rhai types appear in it** (D5).

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]                       // Slice 2 adds Sprite / Animation additively
pub enum RenderSpec {
    BarChart  { title: String, bars: Vec<Bar>, x_label: Option<String>,
                y_label: Option<String>, color: Option<String> },
    LineChart { title: String, series: Vec<Series> },
    Sparkline { title: String, data: Vec<u64> },
    Table     { title: String, headers: Vec<String>, rows: Vec<Row> },
    Gauge     { title: String, value: f64, label: Option<String>, color: Option<String> },
    DotGrid   { title: String, dots: Vec<Dot> },
    Text      { content: String, style: Option<String>, bold: bool, dim: bool },
    AsciiArt  { lines: Vec<String> },
    Separator,
    Layout    { direction: Direction, children: Vec<RenderSpec> },
}
```

### Members

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bar { pub label: String, pub value: i64 }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Series { pub label: String, pub points: Vec<Point> }

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Point { pub x: f64, pub y: f64 }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Row { pub cells: Vec<String>, pub color: Option<String> }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Dot { pub label: String, pub state: DotState }

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DotState { Pass, Fail, Skip }

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction { Vertical, Horizontal }   // → ratatui Direction at render time
```

### Validation rules (enforced by the builders, D6 — not at deserialize time)

| Rule | Value | Source |
|------|-------|--------|
| Layout nesting depth | ≤ 3 | Rendering-API Constraints |
| Total bars + rows + points across the script | ≤ 500 | Rendering-API Constraints |
| Rendered width | ≤ `min(term_width, 120)` cols | Rendering-API Constraints |
| Rendered height | ≤ 40 rows | Rendering-API Constraints |
| `Gauge.value` | clamped to `0.0..=1.0` | `gauge(title, value)` |
| `color` strings | palette name (`honey`…) or basic ANSI name; unknown → default | `chart.color`, `row_colored` |

A violated builder-level rule returns a Rhai error → `ToolResult::error` (never a panic).

### Serialized form (transcript)

`RenderSpec` is the payload of the new `ToolResult.render_spec` field. Example JSON:

```json
{ "kind": "bar_chart", "title": "File Sizes",
  "bars": [{"label": "a.rs", "value": 120}, {"label": "b.rs", "value": 340}],
  "x_label": null, "y_label": null, "color": "honey" }
```

---

## 2. `ToolResult` change — `src/tools.rs` (EDIT)

One additive field on the existing struct (kept `Option`, `skip_serializing_if` so every existing
result and transcript stays byte-identical when absent):

```rust
pub struct ToolResult {
    pub content: String,                 // (unchanged) the text summary sent to the model
    // … existing fields (exit_code, is_error, truncated, original_len, terminal) …
    /// The visualization to render, when this result came from the `render` tool (FR-023).
    /// Never sent to the model — `content` carries the human-readable summary instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub render_spec: Option<RenderSpec>,
}
```

The existing constructors (`ok`, `error`, `terminal_ok`, `invalid_args`) set `render_spec: None`. A
new `ToolResult::rendered(summary, spec)` constructor sets `is_error: false` + `render_spec:
Some(spec)`.

---

## 3. Rhai builder types — `src/render_api.rs`

Opaque custom types registered on the engine; the agent never sees their internals, only the methods
in `contracts/rhai-api.md`. Each accumulates toward one `RenderSpec` variant.

| Builder | Rhai constructor | Accumulates into |
|---------|------------------|------------------|
| `ChartBuilder` | `bar_chart(title)` / `line_chart(title)` / `sparkline(title, data)` | `BarChart` / `LineChart` / `Sparkline` |
| `SeriesHandle` | `chart.series(label)` | a `Series` inside a `LineChart` |
| `TableBuilder` | `table(title)` | `Table` |
| `GaugeBuilder` | `gauge(title, value)` | `Gauge` |
| `DotGridBuilder` | `dots(title)` | `DotGrid` |
| `TextBuilder` | `text(content)` | `Text` |
| `LayoutBuilder` | `vsplit()` / `hsplit()` | `Layout { direction, children }` |
| (value) | `ascii_art(lines)` / `separator()` | `AsciiArt` / `Separator` |

- Builders are `Clone` (Rhai custom types are cloned on assignment); interior data is plain `Vec`s.
- `LayoutBuilder.add(widget)` accepts any builder as a Rhai `Dynamic`, converts it to its
  `RenderSpec`, and pushes it as a child — enforcing the depth ≤ 3 and total-element ≤ 500 caps.
- `render(widget)` (free fn) converts the passed builder to a `RenderSpec` and commits it into the
  `RenderContext`.

---

## 4. `RenderContext` — `src/render_api.rs`

The `Clone + Send` accumulator pushed into the Rhai `Scope` before evaluation (D6).

```rust
#[derive(Clone, Default)]
pub struct RenderContext {
    inner: Arc<Mutex<CtxInner>>,          // Arc/Mutex aligns with rhai "sync" feature
}
struct CtxInner { committed: Option<RenderSpec>, render_calls: u32, element_count: u32 }
```

- `commit(spec)` — sets `committed = Some(spec)`, increments `render_calls` (last-write-wins).
- `add_elements(n)` — bumps `element_count`; errors past 500.
- `take() -> Option<(RenderSpec, RenderCtxStats)>` — extracts the committed spec + stats
  (`render_calls`, whether earlier renders were discarded) for the tool summary.

State transition of one evaluation: `empty → (builders mutate) → commit(...) → take()`.
`render_calls == 0` at `take()` ⇒ "script produced no visualization" error.

---

## 5. `viz` module entities — `src/viz.rs` + `src/viz/*`

### `viz::palette` (constants + helpers) — `viz/palette.rs`

```rust
pub const HONEY:  &str = "33";   // yellow  — info, banners, identity
pub const POLLEN: &str = "32";   // green   — success / pass / allowed
pub const STING:  &str = "31";   // red     — error / fail / denied
pub const SMOKE:  &str = "2";    // dim     — footers, metadata, tool args
pub const ROYAL:  &str = "36";   // cyan    — steering, highlights, links
// `comb` (default fg) = reset; no constant needed beyond "0".

pub fn paint(code: &str, text: &str) -> String;   // wraps in SGR iff color enabled
pub fn bold(code: &str, text: &str) -> String;    // "1;{code}" variant
pub fn is_color_enabled() -> bool;                // false when NO_COLOR is set (any value)
```

### `viz::glyph` (constants) — `viz/glyph.rs`

| Const | Char | Meaning |
|-------|------|---------|
| `DOT_PASS` | `●` U+25CF | pass (colored `POLLEN`) |
| `DOT_FAIL` | `●` U+25CF | fail (colored `STING`) |
| `DOT_SKIP` | `○` U+25CB | skip/pending (colored `SMOKE`) |
| `ARROW` | `▸` U+25B8 | tool call (existing) |
| `CHECK` | `✓` U+2713 | success result (existing) |
| `CROSS` | `✗` U+2717 | error result (existing) |
| `WARN` | `⚠` U+26A0 | kernel denial (existing) |
| `HEX` | `⬡` U+2B21 | bee identity |
| `HLINE` | `─` U+2500 | horizontal rule |
| `VLINE` | `│` U+2502 | vertical separator |

### `viz::grid` — `viz/grid.rs`

```rust
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Status { Pass, Fail, Skip }

/// Render the pass/fail dot grid: name left-aligned, colored dots, "N/M pass" right-aligned.
pub fn status_grid(results: &[(String, Status)]) -> String;
```

`Status` maps from `EpisodeStatus` for batch output: `Completed`/`Captured` → `Pass`,
`Timeout`/`ApiError`/`InfraError`/`NotCaptured` → `Fail`, `NoToolCalls` → `Skip` (mapping documented
in `contracts/honeycomb.md`).

### `viz` (module root) — `viz.rs`

```rust
pub fn render_to_ansi(spec: &RenderSpec, max_width: u16, max_height: u16) -> Vec<String>;
```

Builds transient ratatui widgets from `spec`, renders into a headless `Buffer`, and returns ANSI
lines (D4). Slice 2 adds `viz::sprite` and `viz::bee` submodules here.

---

## Entity relationships

```text
Rhai script ──calls──▶ builder custom-types ──render()──▶ RenderContext
                                                              │ take()
                                                              ▼
                                                          RenderSpec ──serde──▶ ToolResult.render_spec ──▶ transcript
                                                              │
                                        ┌─────────────────────┴───────────────────────┐
                                        ▼                                              ▼
                       ReplOutput::render_widget default              TerminalOutput::render_widget
                        → ASCII fallback via info()                    → viz::render_to_ansi → Buffer
                                                                          → ANSI lines → ExternalPrinter
```
