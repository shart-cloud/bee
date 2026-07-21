# Contract: the Rhai drawing API (agent-facing, Slice 1)

The functions registered on the `render` tool's Rhai `Engine`. This is the surface the model writes
against. Every function here is registered via `register_fn`; **nothing else exists** — no I/O, FFI,
network, process control, or `eval` (FR-021). Types are opaque Rust custom types (see
`data-model.md`).

Slice 1 registers the **static** widget functions below. The **sprite/animation/palette/bee**
functions (`palette()`, `sprite()`, `animation()`, `bee_sprite()`, `bee_animation()`) are **Slice 2**
and are listed at the end as *not yet registered*.

---

## Rhai primer (embedded in the tool schema)

```rhai
let n = 5;                        // variables
let xs = [10, 20, 30];            // arrays
for x in xs { /* … */ }           // loops (bounded by max_operations)
let m = #{ a: 1, b: 2 };          // maps
chart.bar("label", 42);           // method calls (mutate the builder)
render(chart);                    // commit exactly one widget at the end
```

Scripts end with a single `render(widget)`. Integers are `i64`; floats are `f64`.

---

## Charts

| Function | Rhai signature | Description |
|----------|----------------|-------------|
| `bar_chart(title)` | `fn(String) -> ChartBuilder` | Create a bar-chart builder. |
| `chart.bar(label, value)` | `fn(&mut ChartBuilder, String, i64)` | Add a bar. |
| `chart.x_label(s)` | `fn(&mut ChartBuilder, String)` | Set x-axis label. |
| `chart.y_label(s)` | `fn(&mut ChartBuilder, String)` | Set y-axis label. |
| `chart.color(name)` | `fn(&mut ChartBuilder, String)` | Bar color (palette name or ANSI name). |
| `line_chart(title)` | `fn(String) -> ChartBuilder` | Create a line-chart builder. |
| `chart.series(label)` | `fn(&mut ChartBuilder, String) -> SeriesHandle` | Add a named series. |
| `series.point(x, y)` | `fn(&mut SeriesHandle, f64, f64)` | Add a data point. |
| `sparkline(title, data)` | `fn(String, Array) -> ChartBuilder` | Sparkline from an array of ints. |

## Tables

| Function | Rhai signature | Description |
|----------|----------------|-------------|
| `table(title)` | `fn(String) -> TableBuilder` | Create a table builder. |
| `table.header(cols)` | `fn(&mut TableBuilder, Array)` | Set column headers (array of strings). |
| `table.row(cells)` | `fn(&mut TableBuilder, Array)` | Add a row (array of strings). |
| `table.row_colored(cells, color)` | `fn(&mut TableBuilder, Array, String)` | Add a colored row. |

## Status & indicators

| Function | Rhai signature | Description |
|----------|----------------|-------------|
| `gauge(title, value)` | `fn(String, f64) -> GaugeBuilder` | 0.0–1.0 progress gauge (value clamped). |
| `gauge.label(s)` | `fn(&mut GaugeBuilder, String)` | Override the percentage label. |
| `gauge.color(name)` | `fn(&mut GaugeBuilder, String)` | Fill color. |
| `dots(title)` | `fn(String) -> DotGridBuilder` | Pass/fail dot grid. |
| `dots.pass(label)` | `fn(&mut DotGridBuilder, String)` | Add a green dot. |
| `dots.fail(label)` | `fn(&mut DotGridBuilder, String)` | Add a red dot. |
| `dots.skip(label)` | `fn(&mut DotGridBuilder, String)` | Add a neutral dot. |

## Decorative / identity

| Function | Rhai signature | Description |
|----------|----------------|-------------|
| `text(content)` | `fn(String) -> TextBuilder` | Styled text block (ratatui `Paragraph`). |
| `text.style(name)` | `fn(&mut TextBuilder, String)` | Apply a palette color. |
| `text.bold()` | `fn(&mut TextBuilder)` | Bold modifier. |
| `text.dim()` | `fn(&mut TextBuilder)` | Dim modifier. |
| `ascii_art(lines)` | `fn(Array) -> RenderSpec::AsciiArt` | Fixed-width art (no wrapping). |
| `separator()` | `fn() -> RenderSpec::Separator` | Horizontal rule (`─` to width). |

## Layout & composition

| Function | Rhai signature | Description |
|----------|----------------|-------------|
| `vsplit()` | `fn() -> LayoutBuilder` | Vertical stack (top→bottom, blank-line separated). |
| `hsplit()` | `fn() -> LayoutBuilder` | Horizontal columns (side by side, `│`-separated). |
| `layout.add(widget)` | `fn(&mut LayoutBuilder, Dynamic)` | Add any widget to the layout. |
| `render(widget)` | `fn(Dynamic)` | Commit the widget for display (called once). |

---

## Constraints (enforced by the registered functions, not by Rhai's own limits)

| Constraint | Limit | Failure |
|------------|-------|---------|
| Layout nesting depth | ≤ 3 | function returns Rhai error → `ToolResult::error` |
| Total bars + rows + points (whole script) | ≤ 500 | Rhai error → `ToolResult::error` |
| Rendered width | ≤ `min(term_width, 120)` cols | data elided + note in summary |
| Rendered height | ≤ 40 rows | data elided + note in summary |
| Color name | palette (`honey`/`pollen`/`sting`/`smoke`/`royal`) or basic ANSI name | unknown → default color |

## Color names accepted

`honey` (yellow), `pollen` (green), `sting` (red), `smoke` (dim/gray), `royal` (cyan), plus the basic
ANSI names (`red`, `green`, `yellow`, `blue`, `magenta`, `cyan`, `gray`/`grey`, `white`). Unknown
names fall back to the default terminal foreground.

## Example

```rhai
let c = bar_chart("File Sizes");
c.bar("main.rs", 340);
c.bar("lib.rs", 120);
c.bar("tools.rs", 88);
c.color("honey");
render(c);
```

Summary returned to the model: `Rendered a bar chart 'File Sizes' with 3 bars.`

---

## NOT registered in Slice 1 (Slice 2)

These appear in the spec's Rendering API but are **deferred**; a Slice-1 script calling them fails
with "function not found" (which is the correct deny-by-default behavior until Slice 2 lands):

`palette()`, `pal.set(char, color)`, `sprite(w, h, pal)`, `sprite.paint(rows)`, `sprite.set(x, y,
color)`, `sprite.fill(color)`, `animation(ms)`, `anim.add(sprite)`, `anim.bounce(b)`,
`anim.cycles(n)`, `bee_sprite()`, `bee_animation()`.
