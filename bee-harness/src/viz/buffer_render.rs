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

use crate::render_spec::{Direction, DotState, RenderSpec};
use crate::viz::grid::{dot_cell, Status};
use crate::viz::palette;

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
    if let Some(cols) = std::env::var("COLUMNS").ok().and_then(|s| s.trim().parse::<u16>().ok()) {
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
        RenderSpec::Layout { direction, children } => match direction {
            Direction::Vertical => {
                let sum: u16 = children.iter().map(|c| spec_height(c, width)).sum();
                sum + children.len().saturating_sub(1) as u16 // 1 blank separator between children
            }
            Direction::Horizontal => {
                children.iter().map(|c| spec_height(c, width)).max().unwrap_or(1)
            }
        },
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

/// Map a palette/basic-ANSI color name to a ratatui [`Color`]. Unknown names → default foreground.
fn color_of(name: &str) -> Color {
    match name.to_ascii_lowercase().as_str() {
        "honey" | "yellow" => Color::Yellow,
        "pollen" | "green" => Color::Green,
        "sting" | "red" => Color::Red,
        "royal" | "cyan" => Color::Cyan,
        "smoke" | "gray" | "grey" => Color::DarkGray,
        "blue" => Color::Blue,
        "magenta" => Color::Magenta,
        "white" => Color::White,
        _ => Color::Reset,
    }
}

/// Recursively render `spec` into `area` of `buf`.
fn render_into(spec: &RenderSpec, area: Rect, buf: &mut Buffer) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    match spec {
        RenderSpec::Separator => {
            for x in area.left()..area.right() {
                buf[(x, area.top())].set_symbol(crate::viz::glyph::HLINE);
            }
        }
        RenderSpec::Text { content, style, bold, .. } => {
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
        RenderSpec::Gauge { title, value, label, color } => {
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
        RenderSpec::BarChart { title, bars, color, .. } => {
            let rbars: Vec<RBar<'_>> = bars
                .iter()
                .map(|b| {
                    RBar::default().label(Line::from(b.label.clone())).value(b.value.max(0) as u64)
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
        RenderSpec::Table { title, headers, rows } => {
            let ncols = headers.len().max(1);
            let widths: Vec<Constraint> =
                (0..ncols).map(|_| Constraint::Percentage((100 / ncols) as u16)).collect();
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
        RenderSpec::Layout { direction, children } => {
            if children.is_empty() {
                return;
            }
            match direction {
                Direction::Vertical => {
                    // Stack children with a 1-row blank separator between them.
                    let mut y = area.top();
                    let n = children.len();
                    for (i, child) in children.iter().enumerate() {
                        let ch = spec_height(child, area.width).min(area.bottom().saturating_sub(y));
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

/// A ratatui foreground [`Color`] → basic-ANSI SGR fg code, or `None` for the default fg.
fn color_to_sgr(c: Color) -> Option<&'static str> {
    Some(match c {
        Color::Black => "30",
        Color::Red => "31",
        Color::Green => "32",
        Color::Yellow => "33",
        Color::Blue => "34",
        Color::Magenta => "35",
        Color::Cyan => "36",
        Color::Gray => "37",
        Color::DarkGray => "90",
        Color::LightRed => "91",
        Color::LightGreen => "92",
        Color::LightYellow => "93",
        Color::LightBlue => "94",
        Color::LightMagenta => "95",
        Color::LightCyan => "96",
        Color::White => "97",
        _ => return None,
    })
}

/// Serialize a headless [`Buffer`] to one ANSI string per row. When `color` is on, foreground SGR is
/// emitted with run-length batching (a span per run of equal color) and reset at each run's end;
/// when off, symbols only — no escapes (AS-3). Basic ANSI only (no truecolor, Slice 1).
fn buffer_to_ansi(buf: &Buffer, color: bool) -> Vec<String> {
    let area = buf.area;
    let mut out = Vec::with_capacity(area.height as usize);
    for y in area.top()..area.bottom() {
        let mut line = String::new();
        let mut run = String::new();
        let mut run_code: Option<&'static str> = None;
        let flush = |line: &mut String, run: &mut String, code: Option<&'static str>| {
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
            let code = if color { color_to_sgr(cell.fg) } else { None };
            if code != run_code {
                flush(&mut line, &mut run, run_code);
                run_code = code;
            }
            run.push_str(cell.symbol());
        }
        flush(&mut line, &mut run, run_code);
        // Trim trailing spaces (never inside a colored run at the right edge for our widgets).
        while line.ends_with(' ') {
            line.pop();
        }
        out.push(line);
    }
    out
}
