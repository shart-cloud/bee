//! [`render_to_ansi`] (003-visual-render, FR-025): turn a [`RenderSpec`] into inline ANSI lines via a
//! **headless** ratatui `Buffer` — no terminal backend, no alt-screen, no raw mode (research D3/D4).
//! Widgets render into an in-memory cell grid; each row is then serialized to an ANSI string with
//! basic-ANSI (3/4-bit) SGR only (no truecolor in Slice 1) and handed to the caller's printer.

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction as LayoutDir, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Bar as RBar, BarChart, BarGroup, Block, Cell, Gauge, Paragraph, Row as RRow, Sparkline, Table,
    Widget,
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

/// The deterministic terminal-row height a spec occupies at `width` (before the [`MAX_HEIGHT`] clamp).
/// Bordered widgets include their 2 frame rows. Exposed so tests can assert composed-layout heights
/// (SC-014). `width` is threaded through for width-dependent wrapping (future) and nested layouts.
#[allow(clippy::only_used_in_recursion)]
pub fn spec_height(spec: &RenderSpec, width: u16) -> u16 {
    let h = match spec {
        RenderSpec::Separator => 1,
        RenderSpec::Text { content, .. } => (content.lines().count().max(1)) as u16,
        RenderSpec::AsciiArt { lines } => lines.len().max(1) as u16,
        RenderSpec::Gauge { .. } => 3,
        RenderSpec::Sparkline { .. } => 3,
        RenderSpec::DotGrid { .. } => 3,
        RenderSpec::BarChart { .. } => 10,
        RenderSpec::LineChart { .. } => 10,
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
            let mut g = Gauge::default()
                .block(Block::bordered().title(title.clone()))
                .ratio(value.clamp(0.0, 1.0));
            if let Some(c) = color {
                g = g.gauge_style(Style::default().fg(color_of(c)));
            }
            if let Some(l) = label {
                g = g.label(l.clone());
            }
            g.render(area, buf);
        }
        RenderSpec::Sparkline { title, data } => {
            Sparkline::default()
                .block(Block::bordered().title(title.clone()))
                .data(data)
                .render(area, buf);
        }
        RenderSpec::LineChart { title, series } => {
            // Slice 1: render the first series' y-values as a sparkline inside a titled frame — a
            // faithful ratatui Chart with axes is deferred (only the bar chart is exercised by SCs).
            let data: Vec<u64> = series
                .first()
                .map(|s| s.points.iter().map(|p| p.y.max(0.0) as u64).collect())
                .unwrap_or_default();
            Sparkline::default()
                .block(Block::bordered().title(title.clone()))
                .data(&data)
                .render(area, buf);
        }
        RenderSpec::BarChart {
            title, bars, color, ..
        } => {
            let rbars: Vec<RBar<'_>> = bars
                .iter()
                .map(|b| {
                    RBar::default()
                        .label(Line::from(b.label.clone()))
                        .value(b.value.max(0) as u64)
                })
                .collect();
            let mut bc = BarChart::default()
                .block(Block::bordered().title(title.clone()))
                .data(BarGroup::default().bars(&rbars))
                .bar_width(7)
                .bar_gap(1);
            let fill = color.as_deref().map(color_of).unwrap_or(Color::Yellow);
            bc = bc.bar_style(Style::default().fg(fill));
            bc.render(area, buf);
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
            let header = RRow::new(headers.iter().map(|h| Cell::from(h.clone())))
                .style(Style::default().add_modifier(ratatui::style::Modifier::BOLD));
            let body: Vec<RRow<'_>> = rows
                .iter()
                .map(|r| {
                    let mut row = RRow::new(r.cells.iter().map(|c| Cell::from(c.clone())));
                    if let Some(c) = &r.color {
                        row = row.style(Style::default().fg(color_of(c)));
                    }
                    row
                })
                .collect();
            Table::new(body, widths)
                .header(header)
                .block(Block::bordered().title(title.clone()))
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
                .block(Block::bordered().title(title.clone()))
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
                        let block = Block::bordered().title(t.clone());
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
