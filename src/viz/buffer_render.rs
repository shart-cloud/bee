//! [`render_to_ansi`] (003-visual-render, FR-025): turn a [`RenderSpec`] into inline ANSI lines via a
//! **headless** ratatui `Buffer` — no terminal backend, no alt-screen, no raw mode (research D3/D4).
//! Widgets render into an in-memory cell grid; each row is then serialized to an ANSI string with
//! basic-ANSI (3/4-bit) SGR only (no truecolor in Slice 1) and handed to the caller's printer.

use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Constraint, Direction as LayoutDir, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::Marker;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{
    Axis, Bar as RBar, BarChart, BarGroup, Block, Cell, Chart, Dataset, Gauge, GraphType,
    Paragraph, Row as RRow, Sparkline, Table, Widget,
};

use crate::render_spec::{Direction, DotState, RenderSpec, SpriteSpec};
use crate::viz::grid::{dot_cell, Status};
use crate::viz::palette;
use crate::viz::sprite_render::{detect_color_mode, ColorMode};
use crate::viz::theme::{self, ThemeColor};

/// Hard caps on the rendered surface (contracts/honeycomb.md). Width is additionally clamped to the
/// terminal width by the caller.
const MAX_WIDTH: u16 = 120;
const MAX_HEIGHT: u16 = 40;

/// Detect `(terminal_width, is_tty)`. Width comes from `TIOCGWINSZ` (via the already-present `libc`),
/// honoring `$COLUMNS`; falls back to `(80, false)` off a tty or on ioctl failure (research D9 — no
/// new crate).
pub fn terminal_dims() -> (u16, bool) {
    use std::io::IsTerminal;
    let is_tty = std::io::stdout().is_terminal();
    if let Some(cols) = std::env::var("COLUMNS")
        .ok()
        .and_then(|s| s.trim().parse::<u16>().ok())
    {
        if cols > 0 {
            return (cols, is_tty);
        }
    }
    if is_tty {
        // SAFETY: `winsize` is POD; `ioctl(TIOCGWINSZ)` only writes into it and returns 0 on success.
        unsafe {
            let mut ws: libc::winsize = std::mem::zeroed();
            if libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) == 0 && ws.ws_col > 0 {
                return (ws.ws_col, true);
            }
        }
    }
    (80, false)
}

/// The rows a markdown block occupies at `width`, styled where a renderer exists.
///
/// With `tui` this is the real styled rendering, so height and drawing can never disagree. Headless
/// it is the source split into lines: the markdown parser is `tui`-gated on purpose (a batch run has
/// no chat pane and should not carry one), and markdown source is designed to read as plain text —
/// so the fallback shows the block as written rather than as nothing (010).
#[cfg(feature = "tui")]
fn markdown_rows(content: &str, width: u16) -> Vec<Line<'static>> {
    crate::tui::markdown::render(content, width)
}

#[cfg(not(feature = "tui"))]
fn markdown_rows(content: &str, _width: u16) -> Vec<Line<'static>> {
    content.lines().map(|l| Line::raw(l.to_string())).collect()
}

/// The deterministic terminal-row height a spec occupies at `width` (before the [`MAX_HEIGHT`] clamp).
/// Bordered widgets include their 2 frame rows. Exposed so tests can assert composed-layout heights
/// (SC-014). `width` is threaded through for width-dependent wrapping (future) and nested layouts.
#[allow(clippy::only_used_in_recursion)]
pub fn spec_height(spec: &RenderSpec, width: u16) -> u16 {
    let h = match spec {
        RenderSpec::Separator => 1,
        RenderSpec::Text { content, .. } => (content.lines().count().max(1)) as u16,
        // Height must equal what `render_into` will actually draw, so both go through the same
        // function — a markdown block's rendered row count is not its source's line count (a fenced
        // block gains its fences, a long line wraps).
        RenderSpec::Markdown { content } => markdown_rows(content, width).len().max(1) as u16,
        RenderSpec::AsciiArt { lines } => lines.len().max(1) as u16,
        RenderSpec::Gauge { .. } => 3,
        RenderSpec::Sparkline { .. } => 3,
        RenderSpec::DotGrid { .. } => 3,
        RenderSpec::BarChart { .. } => 10,
        RenderSpec::LineChart { .. } => 10,
        RenderSpec::ScatterPlot { .. } => 10,
        RenderSpec::AreaChart { .. } => 10,
        RenderSpec::Heatmap { rows, .. } => 2 + rows.len().max(1) as u16,
        // The tail's height is its window, not its history: border(2) + up to max_rows of content.
        RenderSpec::LogTail {
            lines, max_rows, ..
        } => 2 + (lines.len() as u16).clamp(1, max_rows.unwrap_or(8).max(1)),
        RenderSpec::Table { rows, .. } => 3 + rows.len() as u16, // border(2)+header(1)+rows
        RenderSpec::Layout {
            direction,
            children,
        } => match direction {
            Direction::Vertical => {
                let sum: u16 = children.iter().map(|c| spec_height(c, width)).sum();
                sum + children.len().saturating_sub(1) as u16 // 1 blank separator between children
            }
            Direction::Horizontal => children
                .iter()
                .map(|c| spec_height(c, width))
                .max()
                .unwrap_or(1),
        },
        RenderSpec::Sprite { spec } => spec.height.div_ceil(2),
        RenderSpec::Animation { spec } => spec
            .frames
            .first()
            .map(|f| f.height.div_ceil(2))
            .unwrap_or(1),
        RenderSpec::Grid {
            rows,
            cols,
            gap,
            cells,
            ..
        } => {
            // Uniform-height rows: the row height is the tallest cell content (normalized by its
            // row-span). Simple and deterministic for the inline snapshot; the full-screen renderer
            // will honor row_weights for proportional sizing (docs/grid-tui-plan.md §2).
            let gap = gap.unwrap_or(0);
            let cell_w = (width / (*cols).max(1)).max(1);
            let unit = cells
                .iter()
                .map(|c| spec_height(&c.content, cell_w).div_ceil(c.row_span.max(1)))
                .max()
                .unwrap_or(3)
                .max(1);
            rows.saturating_mul(unit)
                .saturating_add(rows.saturating_sub(1).saturating_mul(gap))
        }
    };
    h.max(1)
}

/// The narrowest terminal width at which `spec` is still *legible* (008-grid-tui). Used to fail a
/// render early when the surface can't show it, instead of drawing an unreadable smear.
///
/// Deliberately forgiving — it answers "is this hopeless?", not "is this pretty?" — so a legitimate
/// render is never rejected. Grids dominate: N columns each need a few cells of content plus gaps.
pub fn min_width(spec: &RenderSpec) -> u16 {
    /// Narrowest column that can still show something in a grid cell.
    const MIN_CELL_COLS: u16 = 8;
    /// Narrowest column of a table.
    const MIN_TABLE_COL: u16 = 6;
    /// Floor for everything that wraps or scales freely (text, gauges, charts…).
    const MIN_ANY: u16 = 8;

    match spec {
        RenderSpec::Grid {
            cols, gap, cells, ..
        } => {
            let cols = (*cols).max(1);
            let gaps = gap.unwrap_or(0).saturating_mul(cols.saturating_sub(1));
            // Each track must fit the widest thing placed in it, floored at MIN_CELL_COLS.
            let per_cell = cells
                .iter()
                .map(|c| min_width(&c.content).div_ceil(c.col_span.max(1)))
                .max()
                .unwrap_or(MIN_CELL_COLS)
                .max(MIN_CELL_COLS);
            cols.saturating_mul(per_cell).saturating_add(gaps)
        }
        RenderSpec::Table { headers, .. } => {
            (headers.len() as u16).max(1).saturating_mul(MIN_TABLE_COL)
        }
        RenderSpec::Layout {
            direction,
            children,
        } => match direction {
            // Side-by-side children each need their own width; stacked ones share it.
            Direction::Horizontal => children.iter().map(min_width).sum::<u16>().max(MIN_ANY),
            Direction::Vertical => children.iter().map(min_width).max().unwrap_or(MIN_ANY),
        },
        // A sprite is one terminal column per pixel column (half-block packs rows, not columns).
        RenderSpec::Sprite { spec } => spec.width.max(1),
        RenderSpec::Animation { spec } => spec.frames.first().map(|f| f.width).unwrap_or(1).max(1),
        _ => MIN_ANY,
    }
}

/// Render a [`RenderSpec`] to inline ANSI lines. `max_width`/`max_height` bound the surface (each is
/// itself clamped to [`MAX_WIDTH`]/[`MAX_HEIGHT`]).
pub fn render_to_ansi(spec: &RenderSpec, max_width: u16, max_height: u16) -> Vec<String> {
    let width = max_width.clamp(8, MAX_WIDTH);
    let height = spec_height(spec, width).clamp(1, max_height.clamp(1, MAX_HEIGHT));
    let area = Rect::new(0, 0, width, height);
    let mut buf = Buffer::empty(area);
    render_into(spec, area, &mut buf);
    buffer_to_ansi(&buf, palette::is_color_enabled())
}

/// Resolve a color *name* through the active theme (005-themes, FR-051) to a ratatui [`Color`]:
/// semantic roles, extended-palette entries, bare `#RRGGBB` hex, and basic color words all resolve;
/// an unknown name falls back to the theme's `text` color. With `honeycomb` active every name maps to
/// a basic-ANSI [`Color`] exactly as it did pre-theming.
fn color_of(name: &str) -> Color {
    theme_to_ratatui_color(&theme::resolve_color_name(theme::active_theme(), name))
}

/// A foreground [`Style`] for a semantic role — unstyled under `NO_COLOR`, so every widget below
/// stays legible in monochrome without each arm re-checking.
fn role_fg(role: theme::Role) -> Style {
    if !palette::is_color_enabled() {
        return Style::default();
    }
    Style::default().fg(theme_to_ratatui_color(theme::active_theme().get(role)))
}

/// A caller-named color as a fg style, or the theme role when the caller didn't pick one.
fn named_or_role(name: Option<&str>, role: theme::Role) -> Style {
    match name {
        Some(n) if palette::is_color_enabled() => Style::default().fg(color_of(n)),
        _ => role_fg(role),
    }
}

/// The one bordered frame every widget wears: dim single-line border, bold title in the text color.
/// Chrome recedes; the title and the data carry the weight.
fn titled_block(title: &str) -> Block<'static> {
    Block::bordered()
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(role_fg(theme::Role::Dim))
        .title(Span::styled(
            format!(" {title} "),
            role_fg(theme::Role::Text).add_modifier(Modifier::BOLD),
        ))
}

/// Categorical series colors in **fixed assignment order** (identity, not rank): accent first, then
/// theme-coordinated hues from the extended palette where the theme has one, else the basic-ANSI
/// spread `honeycomb` has always used. Series index → color; the order never re-shuffles when a
/// series is added or removed.
fn series_palette() -> Vec<Color> {
    let t = theme::active_theme();
    let mut out = vec![theme_to_ratatui_color(t.get(theme::Role::Accent))];
    // Extended hues chosen for pairwise hue separation (blue → yellow → green → purple → orange).
    let extended = ["peach", "orange", "teal", "mauve", "purple", "pink"];
    for name in extended {
        if let Some(c) = t.extended(name) {
            let c = theme_to_ratatui_color(c);
            if !out.contains(&c) {
                out.push(c);
            }
        }
    }
    for fallback in [
        theme_to_ratatui_color(t.get(theme::Role::Info)),
        theme_to_ratatui_color(t.get(theme::Role::Success)),
        Color::Magenta,
        Color::LightRed,
    ] {
        if out.len() >= 5 {
            break;
        }
        if !out.contains(&fallback) {
            out.push(fallback);
        }
    }
    out
}

/// The shared chart frame: data bounds → dim axes with min/max labels. Every chart used to derive
/// this inline, three times, with undimmed labels.
fn chart_axes(
    series: &[crate::render_spec::Series],
    zero_y_floor: bool,
) -> (Axis<'static>, Axis<'static>) {
    let (x_min, x_max, y_min, y_max) = series.iter().flat_map(|s| &s.points).fold(
        (f64::MAX, f64::MIN, f64::MAX, f64::MIN),
        |(xn, xx, yn, yx), p| (xn.min(p.x), xx.max(p.x), yn.min(p.y), yx.max(p.y)),
    );
    let (x_min, x_max) = if x_min >= x_max {
        (x_min - 1.0, x_max + 1.0)
    } else {
        (x_min, x_max)
    };
    let (y_min, y_max) = if zero_y_floor {
        // Areas fill down to the axis, so the axis must sit at zero or the fill lies about magnitude.
        if y_min >= y_max {
            (0.0, y_max + 1.0)
        } else {
            (0.0, y_max)
        }
    } else if y_min >= y_max {
        (y_min - 1.0, y_max + 1.0)
    } else {
        (y_min, y_max)
    };
    let dim = role_fg(theme::Role::Dim);
    let label = |v: f64| Span::styled(format!("{v:.0}"), dim);
    (
        Axis::default()
            .style(dim)
            .bounds([x_min, x_max])
            .labels(vec![label(x_min), label(x_max)]),
        Axis::default()
            .style(dim)
            .bounds([y_min, y_max])
            .labels(vec![label(y_min), label(y_max)]),
    )
}

/// Map a [`ThemeColor`] to a ratatui [`Color`] (005-themes): `Rgb` → [`Color::Rgb`] (serialized
/// truecolor/quantized by [`buffer_to_ansi`]); `Ansi` → the matching basic [`Color`].
pub fn theme_to_ratatui_color(c: &ThemeColor) -> Color {
    match c {
        ThemeColor::Rgb { r, g, b } => Color::Rgb(*r, *g, *b),
        ThemeColor::Ansi(code) => ansi_code_to_ratatui(code),
    }
}

/// A basic-ANSI SGR fg code → the corresponding ratatui [`Color`]. `"2"` (dim) and `"0"` (reset) have
/// no direct color, so they approximate to dark gray / default fg.
fn ansi_code_to_ratatui(code: &str) -> Color {
    match code {
        "0" => Color::Reset,
        "1" | "2" => Color::DarkGray, // bold/dim intensity ≈ dark gray in the headless buffer
        "30" => Color::Black,
        "31" => Color::Red,
        "32" => Color::Green,
        "33" => Color::Yellow,
        "34" => Color::Blue,
        "35" => Color::Magenta,
        "36" => Color::Cyan,
        "37" => Color::Gray,
        "90" => Color::DarkGray,
        "91" => Color::LightRed,
        "92" => Color::LightGreen,
        "93" => Color::LightYellow,
        "94" => Color::LightBlue,
        "95" => Color::LightMagenta,
        "96" => Color::LightCyan,
        "97" => Color::White,
        _ => Color::Reset,
    }
}

/// Recursively render `spec` into `area` of `buf`. Public so the full-screen TUI can draw a panel's
/// widget directly into its (bordered) sub-rect (008-grid-tui, US2 T028); content is clipped to
/// `area` by construction — sub-renders receive sub-rects and never write outside them.
pub fn render_into(spec: &RenderSpec, area: Rect, buf: &mut Buffer) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    match spec {
        RenderSpec::Separator => {
            for x in area.left()..area.right() {
                buf[(x, area.top())].set_symbol(crate::viz::glyph::HLINE);
            }
        }
        RenderSpec::Text {
            content,
            style,
            bold,
            ..
        } => {
            let mut st = Style::default();
            if let Some(c) = style {
                st = st.fg(color_of(c));
            }
            if *bold {
                st = st.add_modifier(ratatui::style::Modifier::BOLD);
            }
            Paragraph::new(content.as_str()).style(st).render(area, buf);
        }
        RenderSpec::Markdown { content } => {
            // Already wrapped to `area.width` by `markdown_rows`, so no `Wrap` here — that would
            // re-flow rows the height calculation has already committed to.
            Paragraph::new(ratatui::text::Text::from(markdown_rows(
                content, area.width,
            )))
            .render(area, buf);
        }
        RenderSpec::AsciiArt { lines } => {
            let text = lines.join("\n");
            Paragraph::new(text).render(area, buf);
        }
        RenderSpec::Gauge {
            title,
            value,
            label,
            color,
        } => {
            // `use_unicode` fills the bar's last cell with an eighth-block, so the edge lands where
            // the value is instead of snapping to the nearest whole cell.
            let mut g = Gauge::default()
                .block(titled_block(title))
                .gauge_style(named_or_role(color.as_deref(), theme::Role::Accent))
                .use_unicode(true)
                .ratio(value.clamp(0.0, 1.0));
            if let Some(l) = label {
                // The label sits centered, so past ~55% the fill is underneath it: flip its ink to
                // dark so "73%" doesn't wash out light-on-light. Below that it sits on the empty
                // (dark) half and keeps the text color. Unstyled under NO_COLOR either way.
                let ink = if *value > 0.55 && palette::is_color_enabled() {
                    Style::default().fg(Color::Black)
                } else {
                    role_fg(theme::Role::Text)
                };
                g = g.label(Span::styled(l.clone(), ink.add_modifier(Modifier::BOLD)));
            }
            g.render(area, buf);
        }
        RenderSpec::Sparkline { title, data } => {
            let block = titled_block(title);
            let inner = block.inner(area);
            Sparkline::default()
                .block(block)
                .style(role_fg(theme::Role::Info))
                .data(data)
                .render(area, buf);
            // Honey gradient: each bar's brightness follows its height, so magnitude is encoded
            // twice — height for reading, brightness for scanning. Only where the theme gives an
            // RGB honey to scale; ANSI themes keep the flat role color, NO_COLOR keeps none.
            if palette::is_color_enabled() {
                if let ThemeColor::Rgb { r, g, b } = theme::active_theme().get(theme::Role::Info) {
                    const LEVELS: [&str; 8] = ["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];
                    for y in inner.top()..inner.bottom() {
                        for x in inner.left()..inner.right() {
                            let cell = &mut buf[(x, y)];
                            if let Some(i) = LEVELS.iter().position(|l| *l == cell.symbol()) {
                                let t = 0.45 + 0.55 * (i as f64 / (LEVELS.len() - 1) as f64);
                                let scale = |c: u8| (f64::from(c) * t) as u8;
                                cell.set_fg(Color::Rgb(scale(*r), scale(*g), scale(*b)));
                            }
                        }
                    }
                }
            }
        }
        RenderSpec::LineChart { title, series } => {
            let colors = series_palette();
            let datasets: Vec<Vec<(f64, f64)>> = series
                .iter()
                .map(|s| s.points.iter().map(|p| (p.x, p.y)).collect())
                .collect();
            // `GraphType::Line` is the whole point: the default `Scatter` drew a line chart as a
            // constellation of disconnected braille dots.
            let ds: Vec<Dataset<'_>> = series
                .iter()
                .zip(datasets.iter())
                .enumerate()
                .map(|(i, (s, pts))| {
                    let d = Dataset::default()
                        .marker(Marker::Braille)
                        .graph_type(GraphType::Line)
                        .style(Style::default().fg(colors[i % colors.len()]))
                        .data(pts);
                    // A name summons the legend. One series needs none — the title names it — and
                    // for two or more the legend is mandatory: two colored lines with no key is
                    // color-alone signaling.
                    if series.len() > 1 {
                        d.name(s.label.clone())
                    } else {
                        d
                    }
                })
                .collect();
            let (x_axis, y_axis) = chart_axes(series, false);
            Chart::new(ds)
                .block(titled_block(title))
                .x_axis(x_axis)
                .y_axis(y_axis)
                // The default hides the legend once it exceeds a quarter of the graph, which a
                // 10-row chart with 2 series always trips. Let it use the full height and half the
                // width — hiding identity beats obscuring a corner of the data only when the series
                // count is absurd, and at that point the width cap still applies.
                .hidden_legend_constraints((
                    Constraint::Percentage(50),
                    Constraint::Percentage(100),
                ))
                .render(area, buf);
        }
        RenderSpec::ScatterPlot { title, series } => {
            let colors = series_palette();
            let datasets: Vec<Vec<(f64, f64)>> = series
                .iter()
                .map(|s| s.points.iter().map(|p| (p.x, p.y)).collect())
                .collect();
            let ds: Vec<Dataset<'_>> = series
                .iter()
                .zip(datasets.iter())
                .enumerate()
                .map(|(i, (s, pts))| {
                    let d = Dataset::default()
                        .marker(Marker::Dot)
                        .graph_type(GraphType::Scatter)
                        .style(Style::default().fg(colors[i % colors.len()]))
                        .data(pts);
                    if series.len() > 1 {
                        d.name(s.label.clone())
                    } else {
                        d
                    }
                })
                .collect();
            let (x_axis, y_axis) = chart_axes(series, false);
            Chart::new(ds)
                .block(titled_block(title))
                .x_axis(x_axis)
                .y_axis(y_axis)
                .hidden_legend_constraints((
                    Constraint::Percentage(50),
                    Constraint::Percentage(100),
                ))
                .render(area, buf);
        }
        RenderSpec::AreaChart {
            title,
            series,
            color,
        } => {
            let colors = match color.as_deref() {
                // The caller picked the hue; extra series still get distinct ones after it.
                Some(c) => {
                    let base = color_of(c);
                    let mut v = vec![base];
                    v.extend(series_palette().into_iter().filter(|x| *x != base));
                    v
                }
                None => series_palette(),
            };
            let datasets: Vec<Vec<(f64, f64)>> = series
                .iter()
                .map(|s| s.points.iter().map(|p| (p.x, p.y)).collect())
                .collect();
            // `GraphType::Area` interpolates between points like `Line` and fills down to
            // `fill_to_y` — a real surface. Half-block markers keep the fill solid rather than a
            // braille stipple.
            let ds: Vec<Dataset<'_>> = series
                .iter()
                .zip(datasets.iter())
                .enumerate()
                .map(|(i, (s, pts))| {
                    let d = Dataset::default()
                        .marker(Marker::HalfBlock)
                        .graph_type(GraphType::Area)
                        .fill_to_y(0.0)
                        .style(Style::default().fg(colors[i % colors.len()]))
                        .data(pts);
                    if series.len() > 1 {
                        d.name(s.label.clone())
                    } else {
                        d
                    }
                })
                .collect();
            let (x_axis, y_axis) = chart_axes(series, true);
            Chart::new(ds)
                .block(titled_block(title))
                .x_axis(x_axis)
                .y_axis(y_axis)
                .hidden_legend_constraints((
                    Constraint::Percentage(50),
                    Constraint::Percentage(100),
                ))
                .render(area, buf);
        }
        RenderSpec::Heatmap {
            title, rows, color, ..
        } => {
            let block = titled_block(title);
            let inner = block.inner(area);
            block.render(area, buf);
            if rows.is_empty() || inner.width == 0 || inner.height == 0 {
                return;
            }
            // One hue, light→dark in *saturation of that hue* — never to black. The old ramp
            // multiplied the channel by t, so low values were near-black cells that read as dead
            // pixels rather than as "small".
            let (hr, hg, hb) = match color.as_deref().map(color_of) {
                Some(Color::Rgb(r, g, b)) => (r, g, b),
                Some(Color::Green) => (64, 220, 64),
                Some(Color::Red) => (230, 70, 70),
                Some(Color::Yellow) => (235, 200, 60),
                Some(Color::Magenta) => (210, 90, 210),
                _ => match theme_to_ratatui_color(theme::active_theme().get(theme::Role::Accent)) {
                    Color::Rgb(r, g, b) => (r, g, b),
                    _ => (60, 170, 230),
                },
            };
            // The floor tints the darkest cell with ~22% of the hue: still clearly "low", never void.
            let ramp = |t: f64| {
                let lerp = |c: u8| (f64::from(c) * (0.22 + 0.78 * t)) as u8;
                Color::Rgb(lerp(hr), lerp(hg), lerp(hb))
            };
            let ncols = rows
                .iter()
                .map(|r| r.values.len())
                .max()
                .unwrap_or(1)
                .max(1);
            let nrows = rows.len();
            let label_w = rows
                .iter()
                .map(|r| r.label.len())
                .max()
                .unwrap_or(0)
                .min(12) as u16;
            let data_w = inner.width.saturating_sub(label_w + 1);
            let cell_w = (data_w / ncols as u16).max(1);
            let cell_h = ((inner.height) / nrows as u16).max(1);
            let (vmin, vmax) = rows
                .iter()
                .flat_map(|r| &r.values)
                .fold((f64::MAX, f64::MIN), |(mn, mx), &v| (mn.min(v), mx.max(v)));
            let range = if (vmax - vmin).abs() < f64::EPSILON {
                1.0
            } else {
                vmax - vmin
            };
            let dim_fg = theme_to_ratatui_color(theme::active_theme().get(theme::Role::Dim));
            for (ri, row) in rows
                .iter()
                .enumerate()
                .take(inner.height as usize / cell_h.max(1) as usize)
            {
                let y = inner.y + ri as u16 * cell_h;
                let label: String = row.label.chars().take(label_w as usize).collect();
                for (ci, ch) in label.chars().enumerate() {
                    if inner.x + ci as u16 <= inner.right() && y < inner.bottom() {
                        buf[(inner.x + ci as u16, y)].set_char(ch).set_fg(dim_fg);
                    }
                }
                for (ci, &val) in row.values.iter().enumerate() {
                    let t = ((val - vmin) / range).clamp(0.0, 1.0);
                    let bg = ramp(t);
                    // Ink flips against the cell's own luma so the value stays readable at both ends.
                    let luma =
                        0.299 * f64::from(hr) + 0.587 * f64::from(hg) + 0.114 * f64::from(hb);
                    let fg = if t * luma > 110.0 {
                        Color::Black
                    } else {
                        Color::White
                    };
                    let cx = inner.x + label_w + 1 + ci as u16 * cell_w;
                    // The value annotates the cell only where it fits with a cell of padding —
                    // cramming "0.4" into a 3-wide cell left no color visible to compare.
                    let label_str = format!("{val:.1}");
                    let annotate = cell_w as usize >= label_str.len() + 2;
                    for dy in 0..cell_h {
                        for dx in 0..cell_w {
                            let px = cx + dx;
                            let py = y + dy;
                            // The rightmost column of each cell stays the surface color: the 1-cell
                            // spacer that keeps adjacent fills readable as separate cells.
                            if px < inner.right() && py < inner.bottom() {
                                if dx + 1 == cell_w && cell_w > 1 {
                                    continue;
                                }
                                let ch = if annotate && dy == 0 && (dx as usize) < label_str.len() {
                                    label_str.as_bytes()[dx as usize] as char
                                } else {
                                    ' '
                                };
                                buf[(px, py)].set_char(ch).set_fg(fg).set_bg(bg);
                            }
                        }
                    }
                }
            }
        }
        RenderSpec::BarChart {
            title, bars, color, ..
        } => {
            let label_style = role_fg(theme::Role::Dim);
            let rbars: Vec<RBar<'_>> = bars
                .iter()
                .map(|b| {
                    RBar::default()
                        .label(Line::styled(b.label.clone(), label_style))
                        .value(b.value.max(0) as u64)
                })
                .collect();
            // Bars share the inner width instead of a fixed 7 cells: n bars + (n-1) gaps, clamped so
            // a two-bar chart doesn't become two slabs and a twelve-bar one doesn't vanish.
            let n = bars.len().max(1) as u16;
            let inner_w = area.width.saturating_sub(2);
            let bar_w = (inner_w.saturating_sub(n - 1) / n).clamp(3, 9);
            let fill = named_or_role(color.as_deref(), theme::Role::Info);
            BarChart::default()
                .block(titled_block(title))
                .data(BarGroup::default().bars(&rbars))
                .bar_width(bar_w)
                .bar_gap(1)
                .bar_style(fill)
                // The value prints in the bar's bottom cell; reversed ink keeps it legible on the
                // fill without inventing a second color.
                .value_style(fill.add_modifier(Modifier::REVERSED | Modifier::BOLD))
                .render(area, buf);
        }
        RenderSpec::LogTail { title, lines, .. } => {
            let block = titled_block(title);
            let inner = block.inner(area);
            block.render(area, buf);
            if inner.width == 0 || inner.height == 0 {
                return;
            }
            // Bottom-anchored: the newest lines that fit, newest on the last row — a tail, not a
            // page. Lines truncate rather than wrap so the stream keeps its column alignment.
            let visible = lines
                .iter()
                .rev()
                .take(inner.height as usize)
                .collect::<Vec<_>>();
            for (i, line) in visible.iter().rev().enumerate() {
                // Letter + color per level: the letter is what survives monochrome (never
                // color-alone), the color is what makes an error findable at a glance.
                let (mark, style) = match line.level.as_deref() {
                    Some("error" | "fatal") => ("E", role_fg(theme::Role::Error)),
                    Some("warn" | "warning") => ("W", role_fg(theme::Role::Info)),
                    Some("info") => ("I", role_fg(theme::Role::Text)),
                    Some("debug" | "trace") => ("D", role_fg(theme::Role::Dim)),
                    _ => (" ", role_fg(theme::Role::Text)),
                };
                // Errors carry their color through the text (they are what the eye hunts for);
                // debug/trace recede whole; everything else keeps quiet text with a marked gutter.
                let text_style = match line.level.as_deref() {
                    Some("error" | "fatal") => role_fg(theme::Role::Error),
                    Some("debug" | "trace") => role_fg(theme::Role::Dim),
                    _ => role_fg(theme::Role::Text),
                };
                let y = inner.y + i as u16;
                let row = Rect::new(inner.x, y, inner.width, 1);
                let content: String = line
                    .text
                    .chars()
                    .take(inner.width.saturating_sub(2) as usize)
                    .collect();
                Paragraph::new(Line::from(vec![
                    Span::styled(mark.to_string(), style.add_modifier(Modifier::BOLD)),
                    Span::raw(" "),
                    Span::styled(content, text_style),
                ]))
                .render(row, buf);
            }
        }
        RenderSpec::Table {
            title,
            headers,
            rows,
        } => {
            let ncols = headers.len().max(1);
            let widths: Vec<Constraint> = (0..ncols)
                .map(|_| Constraint::Percentage((100 / ncols) as u16))
                .collect();
            // Numeric columns read right-aligned (ones under ones); a column is numeric when every
            // populated cell in it leads with a digit/sign — "14.8s" and "88ms" count.
            let numeric_col: Vec<bool> = (0..ncols)
                .map(|i| {
                    let mut any = false;
                    let all = rows.iter().all(|r| match r.cells.get(i) {
                        Some(c) if !c.is_empty() => {
                            any = true;
                            c.starts_with(|ch: char| ch.is_ascii_digit() || ch == '-' || ch == '+')
                        }
                        _ => true,
                    });
                    any && all
                })
                .collect();
            let align = |i: usize| {
                if numeric_col.get(i).copied().unwrap_or(false) {
                    Alignment::Right
                } else {
                    Alignment::Left
                }
            };
            let header = RRow::new(headers.iter().enumerate().map(|(i, h)| {
                Cell::from(Text::from(h.clone()).alignment(align(i))).style(
                    role_fg(theme::Role::Dim).add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
                )
            }));
            let body: Vec<RRow<'_>> =
                rows.iter()
                    .map(|r| {
                        let mut row =
                            RRow::new(r.cells.iter().enumerate().map(|(i, c)| {
                                Cell::from(Text::from(c.clone()).alignment(align(i)))
                            }));
                        if let Some(c) = &r.color {
                            row = row.style(Style::default().fg(color_of(c)));
                        }
                        row
                    })
                    .collect();
            Table::new(body, widths)
                .header(header)
                .block(titled_block(title))
                .render(area, buf);
        }
        RenderSpec::DotGrid { title, dots } => {
            let mut spans: Vec<Span<'_>> = Vec::new();
            for (i, d) in dots.iter().enumerate() {
                if i > 0 {
                    spans.push(Span::raw(" "));
                }
                let status = match d.state {
                    DotState::Pass => Status::Pass,
                    DotState::Fail => Status::Fail,
                    DotState::Skip => Status::Skip,
                };
                let (glyph, code) = dot_cell(status);
                spans.push(Span::styled(glyph, Style::default().fg(sgr_to_color(code))));
            }
            Paragraph::new(Line::from(spans))
                .block(titled_block(title))
                .render(area, buf);
        }
        RenderSpec::Layout {
            direction,
            children,
        } => {
            if children.is_empty() {
                return;
            }
            match direction {
                Direction::Vertical => {
                    // Stack children with a 1-row blank separator between them.
                    let mut y = area.top();
                    let n = children.len();
                    for (i, child) in children.iter().enumerate() {
                        let ch =
                            spec_height(child, area.width).min(area.bottom().saturating_sub(y));
                        if ch == 0 {
                            break;
                        }
                        let sub = Rect::new(area.left(), y, area.width, ch);
                        render_into(child, sub, buf);
                        y += ch;
                        if i + 1 < n {
                            y += 1; // separator row
                        }
                        if y >= area.bottom() {
                            break;
                        }
                    }
                }
                Direction::Horizontal => {
                    let cols = Layout::default()
                        .direction(LayoutDir::Horizontal)
                        .constraints(
                            children
                                .iter()
                                .map(|_| Constraint::Ratio(1, children.len() as u32))
                                .collect::<Vec<_>>(),
                        )
                        .split(area);
                    for (child, col) in children.iter().zip(cols.iter()) {
                        render_into(child, *col, buf);
                    }
                }
            }
        }
        // Sprites rasterize into the buffer as truecolor half-blocks (T008), wherever they appear —
        // nested in a layout/grid cell, or inline in the full-screen chat flow. (This used to be a
        // `[sprite W×H]` placeholder, which is why the mascot and every inline sprite showed up as
        // literal text.) An animation is one static render here, so it draws its first frame;
        // playback is `TerminalOutput`'s job via `viz::animator` (contracts/sprite-api.md).
        RenderSpec::Sprite { spec } => rasterize_sprite_into(spec, area, buf),
        RenderSpec::Animation { spec } => {
            if let Some(frame) = spec.frames.first() {
                rasterize_sprite_into(frame, area, buf);
            }
        }
        RenderSpec::Grid {
            rows,
            cols,
            col_weights,
            row_weights,
            gap,
            cells,
        } => {
            if *rows == 0 || *cols == 0 {
                return;
            }
            let gap = gap.unwrap_or(0);
            // Track boundaries: split the whole area into `cols` columns and `rows` rows (weighted by
            // *_weights, default equal Fill). A spanning cell's Rect is the union of the base tracks
            // it covers, so spans "just work" (docs/grid-tui-plan.md §2).
            let col_c: Vec<Constraint> = (0..*cols)
                .map(|c| Constraint::Fill(col_weights.get(c as usize).copied().unwrap_or(1).max(1)))
                .collect();
            let row_c: Vec<Constraint> = (0..*rows)
                .map(|r| Constraint::Fill(row_weights.get(r as usize).copied().unwrap_or(1).max(1)))
                .collect();
            let col_rects = Layout::default()
                .direction(LayoutDir::Horizontal)
                .spacing(gap)
                .constraints(col_c)
                .split(area);
            let row_rects = Layout::default()
                .direction(LayoutDir::Vertical)
                .spacing(gap)
                .constraints(row_c)
                .split(area);
            for cell in cells {
                let (c0, r0) = (cell.col as usize, cell.row as usize);
                let c1 = (cell.col + cell.col_span - 1) as usize;
                let r1 = (cell.row + cell.row_span - 1) as usize;
                if r1 >= row_rects.len() || c1 >= col_rects.len() {
                    continue; // out of bounds — validated away at build, guarded here for safety
                }
                let x = col_rects[c0].x;
                let y = row_rects[r0].y;
                let w = col_rects[c1].right().saturating_sub(x);
                let h = row_rects[r1].bottom().saturating_sub(y);
                let rect = Rect::new(x, y, w, h);
                // An optional cell title wraps the content in a bordered block; otherwise the inner
                // widget draws directly (most widgets carry their own titled block).
                let inner = match &cell.title {
                    Some(t) => {
                        let block = titled_block(t);
                        let inner = block.inner(rect);
                        block.render(rect, buf);
                        inner
                    }
                    None => rect,
                };
                render_into(&cell.content, inner, buf);
            }
        }
    }
}

/// Composite a [`SpriteSpec`] into `area` of a truecolor `Buffer` using vertical half-blocks
/// (008-grid-tui, T008 / research D8): two stacked pixels per cell — `fg` = the lower pixel, `bg` =
/// the upper pixel (`▄`), or `▀` when only the top is opaque.
///
/// This is the path the **full-screen TUI** uses to draw sprites *inside* grid cells and panels,
/// where the real backend carries a per-cell background. It deliberately is **not** wired into the
/// inline `render_into` placeholder: that path serializes through [`buffer_to_ansi`], which emits only
/// a foreground SGR, so a background would be lost. A transparent half leaves the buffer's existing
/// content in that cell.
pub fn rasterize_sprite_into(spec: &SpriteSpec, area: Rect, buf: &mut Buffer) {
    let rows = spec.height.div_ceil(2);
    for r in 0..rows {
        let y = area.top() + r;
        if y >= area.bottom() {
            break;
        }
        for x in 0..spec.width {
            let cx = area.left() + x;
            if cx >= area.right() {
                break;
            }
            let top = spec.pixel(x, 2 * r);
            let bottom = spec.pixel(x, 2 * r + 1);
            let cell = &mut buf[(cx, y)];
            match (top, bottom) {
                (None, None) => {}
                (Some(t), Some(b)) => {
                    cell.set_symbol("▄")
                        .set_fg(Color::Rgb(b.0, b.1, b.2))
                        .set_bg(Color::Rgb(t.0, t.1, t.2));
                }
                (Some(t), None) => {
                    cell.set_symbol("▀").set_fg(Color::Rgb(t.0, t.1, t.2));
                }
                (None, Some(b)) => {
                    cell.set_symbol("▄").set_fg(Color::Rgb(b.0, b.1, b.2));
                }
            }
        }
    }
}

/// Map an SGR fg code back to a ratatui [`Color`] (for the shared `dot_cell` codes).
fn sgr_to_color(code: &str) -> Color {
    match code {
        palette::POLLEN => Color::Green,
        palette::STING => Color::Red,
        palette::SMOKE => Color::DarkGray,
        palette::HONEY => Color::Yellow,
        palette::ROYAL => Color::Cyan,
        _ => Color::Reset,
    }
}

/// A ratatui foreground [`Color`] → SGR fg parameters, or `None` for the default fg. Truecolor
/// [`Color::Rgb`] (from an RGB theme) is emitted verbatim on a truecolor terminal, else quantized to
/// the xterm-256 cube / 16 colors per `mode` (005-themes, FR-049).
fn color_to_sgr(c: Color, mode: ColorMode) -> Option<String> {
    Some(match c {
        Color::Black => "30".into(),
        Color::Red => "31".into(),
        Color::Green => "32".into(),
        Color::Yellow => "33".into(),
        Color::Blue => "34".into(),
        Color::Magenta => "35".into(),
        Color::Cyan => "36".into(),
        Color::Gray => "37".into(),
        Color::DarkGray => "90".into(),
        Color::LightRed => "91".into(),
        Color::LightGreen => "92".into(),
        Color::LightYellow => "93".into(),
        Color::LightBlue => "94".into(),
        Color::LightMagenta => "95".into(),
        Color::LightCyan => "96".into(),
        Color::White => "97".into(),
        Color::Rgb(r, g, b) => ThemeColor::Rgb { r, g, b }.sgr_params(mode),
        Color::Indexed(i) => format!("38;5;{i}"),
        _ => return None,
    })
}

/// Serialize a headless [`Buffer`] to one ANSI string per row. When `color` is on, foreground SGR is
/// emitted with run-length batching (a span per run of equal color) and reset at each run's end;
/// when off, symbols only — no escapes (AS-3). Basic ANSI only (no truecolor, Slice 1).
fn buffer_to_ansi(buf: &Buffer, color: bool) -> Vec<String> {
    let area = buf.area;
    let mode = detect_color_mode();
    let mut out = Vec::with_capacity(area.height as usize);
    for y in area.top()..area.bottom() {
        let mut line = String::new();
        let mut run = String::new();
        let mut run_code: Option<String> = None;
        let flush = |line: &mut String, run: &mut String, code: &Option<String>| {
            if run.is_empty() {
                return;
            }
            match code {
                Some(c) if color => {
                    line.push_str(&format!("\x1b[{c}m{run}\x1b[0m"));
                }
                _ => line.push_str(run),
            }
            run.clear();
        };
        for x in area.left()..area.right() {
            let cell = &buf[(x, y)];
            let code = if color {
                color_to_sgr(cell.fg, mode)
            } else {
                None
            };
            if code != run_code {
                flush(&mut line, &mut run, &run_code);
                run_code = code;
            }
            run.push_str(cell.symbol());
        }
        flush(&mut line, &mut run, &run_code);
        // Trim trailing spaces (never inside a colored run at the right edge for our widgets).
        while line.ends_with(' ') {
            line.pop();
        }
        out.push(line);
    }
    out
}

#[cfg(test)]
mod log_tail_tests {
    use super::*;
    use crate::render_spec::LogLine;

    fn tail(n: usize, max_rows: Option<u16>) -> RenderSpec {
        RenderSpec::LogTail {
            title: "audit".into(),
            lines: (0..n)
                .map(|i| LogLine {
                    text: format!("line-{i}"),
                    level: if i % 2 == 0 {
                        Some("error".into())
                    } else {
                        None
                    },
                })
                .collect(),
            max_rows,
        }
    }

    fn symbols(spec: &RenderSpec, w: u16, h: u16) -> String {
        let area = Rect::new(0, 0, w, h);
        let mut buf = Buffer::empty(area);
        render_into(spec, area, &mut buf);
        let mut s = String::new();
        for y in 0..h {
            for x in 0..w {
                s.push_str(buf[(x, y)].symbol());
            }
            s.push('\n');
        }
        s
    }

    #[test]
    fn the_tail_keeps_the_newest_lines_and_drops_the_oldest() {
        let spec = tail(20, Some(5));
        assert_eq!(spec_height(&spec, 40), 7, "border(2) + max_rows(5)");
        let out = symbols(&spec, 40, 7);
        assert!(out.contains("line-19"), "newest line shown:\n{out}");
        assert!(
            out.contains("line-15"),
            "window reaches back 5 lines:\n{out}"
        );
        assert!(!out.contains("line-14"), "older lines scrolled off:\n{out}");
        // Bottom-anchored: the newest line sits on the last content row.
        let rows: Vec<&str> = out.lines().collect();
        assert!(rows[5].contains("line-19"), "newest at the bottom:\n{out}");
    }

    #[test]
    fn levels_mark_the_gutter_with_letters_that_survive_monochrome() {
        let out = symbols(&tail(2, None), 40, 4);
        assert!(out.contains("E line-0"), "error marked with E:\n{out}");
        assert!(out.contains("  line-1"), "unleveled lines unmarked:\n{out}");
    }
}
