//! `cargo xtask viz-gallery` — render every widget type to the terminal (ANSI), to SVG files, or
//! both. Uses bee's own `buffer_render` so what you see here is exactly what the TUI draws.

use std::path::{Path, PathBuf};

use anyhow::Result;
use bee::render_spec::*;
use bee::viz::buffer_render::{render_into, spec_height};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};

const WIDTH: u16 = 60;
const MAX_HEIGHT: u16 = 20;

struct Exhibit {
    name: &'static str,
    spec: RenderSpec,
}

fn gallery() -> Vec<Exhibit> {
    vec![
        Exhibit {
            name: "bar_chart",
            spec: RenderSpec::BarChart {
                title: "Quarterly Revenue".into(),
                bars: vec![
                    Bar {
                        label: "Q1".into(),
                        value: 120,
                    },
                    Bar {
                        label: "Q2".into(),
                        value: 185,
                    },
                    Bar {
                        label: "Q3".into(),
                        value: 95,
                    },
                    Bar {
                        label: "Q4".into(),
                        value: 210,
                    },
                ],
                x_label: Some("Quarter".into()),
                y_label: Some("$K".into()),
                color: Some("accent".into()),
            },
        },
        Exhibit {
            name: "line_chart",
            spec: RenderSpec::LineChart {
                title: "CPU Usage".into(),
                series: vec![Series {
                    label: "node-1".into(),
                    points: vec![
                        Point { x: 0.0, y: 12.0 },
                        Point { x: 1.0, y: 45.0 },
                        Point { x: 2.0, y: 30.0 },
                        Point { x: 3.0, y: 78.0 },
                        Point { x: 4.0, y: 55.0 },
                        Point { x: 5.0, y: 92.0 },
                        Point { x: 6.0, y: 41.0 },
                        Point { x: 7.0, y: 63.0 },
                    ],
                }],
            },
        },
        Exhibit {
            name: "line_chart_multi",
            spec: RenderSpec::LineChart {
                title: "Latency (ms)".into(),
                series: vec![
                    Series {
                        label: "p50".into(),
                        points: vec![
                            Point { x: 0.0, y: 5.0 },
                            Point { x: 1.0, y: 8.0 },
                            Point { x: 2.0, y: 6.0 },
                            Point { x: 3.0, y: 12.0 },
                            Point { x: 4.0, y: 7.0 },
                        ],
                    },
                    Series {
                        label: "p99".into(),
                        points: vec![
                            Point { x: 0.0, y: 25.0 },
                            Point { x: 1.0, y: 40.0 },
                            Point { x: 2.0, y: 30.0 },
                            Point { x: 3.0, y: 55.0 },
                            Point { x: 4.0, y: 35.0 },
                        ],
                    },
                ],
            },
        },
        Exhibit {
            name: "scatter_plot",
            spec: RenderSpec::ScatterPlot {
                title: "Clusters".into(),
                series: vec![
                    Series {
                        label: "group-A".into(),
                        points: vec![
                            Point { x: 1.0, y: 2.0 },
                            Point { x: 2.0, y: 3.5 },
                            Point { x: 1.5, y: 4.0 },
                            Point { x: 3.0, y: 2.5 },
                            Point { x: 2.5, y: 5.0 },
                        ],
                    },
                    Series {
                        label: "group-B".into(),
                        points: vec![
                            Point { x: 6.0, y: 7.0 },
                            Point { x: 7.0, y: 8.5 },
                            Point { x: 6.5, y: 9.0 },
                            Point { x: 8.0, y: 7.5 },
                            Point { x: 7.5, y: 6.0 },
                        ],
                    },
                ],
            },
        },
        Exhibit {
            name: "area_chart",
            spec: RenderSpec::AreaChart {
                title: "Memory Usage (GB)".into(),
                series: vec![Series {
                    label: "heap".into(),
                    points: vec![
                        Point { x: 0.0, y: 2.1 },
                        Point { x: 1.0, y: 3.4 },
                        Point { x: 2.0, y: 2.8 },
                        Point { x: 3.0, y: 5.1 },
                        Point { x: 4.0, y: 4.2 },
                        Point { x: 5.0, y: 6.3 },
                        Point { x: 6.0, y: 5.5 },
                    ],
                }],
                color: Some("accent".into()),
            },
        },
        Exhibit {
            name: "heatmap",
            spec: RenderSpec::Heatmap {
                title: "Weekly Activity".into(),
                rows: vec![
                    HeatRow {
                        label: "Mon".into(),
                        values: vec![0.1, 0.4, 0.8, 0.6, 0.2, 0.9, 0.3],
                    },
                    HeatRow {
                        label: "Tue".into(),
                        values: vec![0.5, 0.7, 0.3, 0.9, 0.4, 0.1, 0.6],
                    },
                    HeatRow {
                        label: "Wed".into(),
                        values: vec![0.8, 0.2, 0.6, 0.4, 0.7, 0.5, 0.9],
                    },
                    HeatRow {
                        label: "Thu".into(),
                        values: vec![0.3, 0.9, 0.1, 0.7, 0.5, 0.8, 0.2],
                    },
                    HeatRow {
                        label: "Fri".into(),
                        values: vec![0.6, 0.3, 0.9, 0.2, 0.8, 0.4, 0.7],
                    },
                ],
                color: None,
            },
        },
        Exhibit {
            name: "log_tail",
            spec: RenderSpec::LogTail {
                title: "sandbox denials".into(),
                lines: vec![
                    LogLine { text: "policy cargo-test loaded (14 rules)".into(), level: Some("info".into()) },
                    LogLine { text: "exec /usr/bin/cc allowed".into(), level: Some("debug".into()) },
                    LogLine { text: "connect 140.82.112.3:443 outside egress scope".into(), level: Some("warn".into()) },
                    LogLine { text: "open /etc/shadow blocked (file_open)".into(), level: Some("error".into()) },
                    LogLine { text: "exec /usr/bin/curl blocked (bprm_check)".into(), level: Some("error".into()) },
                    LogLine { text: "episode continuing under policy".into(), level: Some("info".into()) },
                ],
                max_rows: Some(6),
            },
        },
        Exhibit {
            name: "sparkline",
            spec: RenderSpec::Sparkline {
                title: "Requests/sec".into(),
                data: vec![4, 7, 12, 8, 15, 9, 3, 11, 6, 14, 10, 5, 13, 8, 7],
            },
        },
        Exhibit {
            name: "table",
            spec: RenderSpec::Table {
                title: "Test Results".into(),
                headers: vec![
                    "Suite".into(),
                    "Passed".into(),
                    "Failed".into(),
                    "Time".into(),
                ],
                rows: vec![
                    Row {
                        cells: vec!["unit".into(), "142".into(), "0".into(), "1.2s".into()],
                        color: None,
                    },
                    Row {
                        cells: vec![
                            "integration".into(),
                            "38".into(),
                            "2".into(),
                            "14.8s".into(),
                        ],
                        color: Some("error".into()),
                    },
                    Row {
                        cells: vec!["e2e".into(), "15".into(), "0".into(), "42.1s".into()],
                        color: None,
                    },
                ],
            },
        },
        Exhibit {
            name: "gauge",
            spec: RenderSpec::Gauge {
                title: "Disk Usage".into(),
                value: 0.73,
                label: Some("73% (58.4 / 80 GB)".into()),
                color: Some("accent".into()),
            },
        },
        Exhibit {
            name: "dot_grid",
            spec: RenderSpec::DotGrid {
                title: "Test Matrix".into(),
                dots: vec![
                    Dot {
                        label: "auth-login".into(),
                        state: DotState::Pass,
                    },
                    Dot {
                        label: "auth-logout".into(),
                        state: DotState::Pass,
                    },
                    Dot {
                        label: "auth-refresh".into(),
                        state: DotState::Pass,
                    },
                    Dot {
                        label: "api-read".into(),
                        state: DotState::Pass,
                    },
                    Dot {
                        label: "api-write".into(),
                        state: DotState::Fail,
                    },
                    Dot {
                        label: "api-delete".into(),
                        state: DotState::Pass,
                    },
                    Dot {
                        label: "ws-connect".into(),
                        state: DotState::Skip,
                    },
                    Dot {
                        label: "ws-stream".into(),
                        state: DotState::Pass,
                    },
                ],
            },
        },
        Exhibit {
            name: "text_styled",
            spec: RenderSpec::Text {
                content: "Deploy completed successfully to production-us-east-1".into(),
                style: Some("success".into()),
                bold: true,
                dim: false,
            },
        },
        Exhibit {
            name: "sprite",
            spec: RenderSpec::Sprite {
                spec: bee::viz::bee::sprite(),
            },
        },
        Exhibit {
            name: "layout_vsplit",
            spec: RenderSpec::Layout {
                direction: Direction::Horizontal,
                children: vec![
                    RenderSpec::Gauge {
                        title: "CPU".into(),
                        value: 0.42,
                        label: Some("42%".into()),
                        color: Some("success".into()),
                    },
                    RenderSpec::Gauge {
                        title: "Memory".into(),
                        value: 0.87,
                        label: Some("87%".into()),
                        color: Some("error".into()),
                    },
                ],
            },
        },
    ]
}

pub fn run(format: &str, out_dir: Option<&Path>) -> Result<()> {
    let exhibits = gallery();

    match format {
        "ansi" | "all" => {
            for ex in &exhibits {
                let height = spec_height(&ex.spec, WIDTH).clamp(1, MAX_HEIGHT);
                let area = Rect::new(0, 0, WIDTH, height);
                let mut buf = Buffer::empty(area);
                render_into(&ex.spec, area, &mut buf);

                println!("\n\x1b[1;36m── {} ──\x1b[0m", ex.name);
                for y in area.y..area.bottom() {
                    let mut line = String::new();
                    for x in area.x..area.right() {
                        let cell = &buf[(x, y)];
                        let sym = cell.symbol();
                        let (fg, bg) = (cell.fg, cell.bg);
                        if fg != Color::Reset || bg != Color::Reset {
                            if let Some(fg_code) = color_to_ansi_fg(fg) {
                                line.push_str(&fg_code);
                            }
                            if let Some(bg_code) = color_to_ansi_bg(bg) {
                                line.push_str(&bg_code);
                            }
                            line.push_str(sym);
                            line.push_str("\x1b[0m");
                        } else {
                            line.push_str(sym);
                        }
                    }
                    println!("{line}");
                }
            }
        }
        _ => {}
    }

    if matches!(format, "svg" | "all") {
        let dir = out_dir
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("xtask/gallery"));
        std::fs::create_dir_all(&dir)?;

        for ex in &exhibits {
            let height = spec_height(&ex.spec, WIDTH).clamp(1, MAX_HEIGHT);
            let area = Rect::new(0, 0, WIDTH, height);
            let mut buf = Buffer::empty(area);
            render_into(&ex.spec, area, &mut buf);

            let svg = buffer_to_svg(&buf, ex.name);
            let path = dir.join(format!("{}.svg", ex.name));
            std::fs::write(&path, &svg)?;
            println!("wrote {}", path.display());
        }
    }

    Ok(())
}

pub(crate) fn buffer_to_svg(buf: &Buffer, title: &str) -> String {
    let cell_w: f32 = 8.0;
    let cell_h: f32 = 16.0;
    let width = buf.area.width as f32 * cell_w;
    let height = buf.area.height as f32 * cell_h;
    let pad = 20.0;
    let title_h = 24.0;
    let total_w = width + pad * 2.0;
    let total_h = height + pad * 2.0 + title_h;

    let mut svg = format!(
        concat!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {total_w} {total_h}\"",
            // Mono first for the grid; DejaVu Sans / Noto Symbols2 cover the braille block
            // (U+2800) charts draw with, which no common *mono* font carries — braille as tofu made
            // every line-chart export look broken when the terminal rendering was fine. Cells are
            // absolutely positioned, so a proportional fallback can't skew the grid.
            " font-family=\"DejaVu Sans Mono, DejaVu Sans, Noto Sans Symbols2, monospace\"",
            " font-size=\"13\">\n",
            "<rect width=\"{total_w}\" height=\"{total_h}\" fill=\"#1a1a2e\" rx=\"8\"/>\n",
            "<text x=\"{pad}\" y=\"{title_y}\" fill=\"#e5a100\" font-weight=\"bold\"",
            " font-size=\"14\">{title}</text>\n",
        ),
        total_w = total_w,
        total_h = total_h,
        pad = pad,
        title_y = pad + 14.0,
        title = title,
    );

    let base_y = pad + title_h;
    for y in buf.area.y..buf.area.bottom() {
        for x in buf.area.x..buf.area.right() {
            let cell = &buf[(x, y)];
            let sym = cell.symbol();
            if sym == " " && cell.bg == Color::Reset {
                continue;
            }

            let px = pad + (x as f32) * cell_w;
            let py = base_y + (y as f32) * cell_h;

            // Reverse video is how the TUI marks selection; dropping it here made a focused panel
            // export identically to an unfocused one. Swap the pair, defaulting the missing side to
            // the page colors.
            let reversed = cell.modifier.contains(Modifier::REVERSED);
            let (fg, bg) = if reversed {
                (
                    if cell.bg == Color::Reset { Color::Rgb(0x1a, 0x1a, 0x2e) } else { cell.bg },
                    if cell.fg == Color::Reset { Color::Rgb(0xcc, 0xcc, 0xcc) } else { cell.fg },
                )
            } else {
                (cell.fg, cell.bg)
            };

            if bg != Color::Reset {
                if let Some(hex) = color_to_hex(bg) {
                    svg.push_str(&format!(
                        r#"<rect x="{px}" y="{}" width="{cell_w}" height="{cell_h}" fill="{hex}"/>"#,
                        py - cell_h + 3.0,
                    ));
                }
            }

            if sym != " " {
                let fg_hex = color_to_hex(fg).unwrap_or_else(|| "#cccccc".into());
                let weight = if cell.modifier.contains(Modifier::BOLD) {
                    r#" font-weight="bold""#
                } else {
                    ""
                };
                let escaped = match sym {
                    "<" => "&lt;",
                    ">" => "&gt;",
                    "&" => "&amp;",
                    "\"" => "&quot;",
                    _ => sym,
                };
                // Braille (U+2800..U+28FF) and the symbol block holding bee's hexagons
                // (U+2B00..U+2BFF) get an explicit non-mono face: renderers with cairo's
                // toy font API (cairosvg) pick ONE family per element and never fall back per
                // glyph, so those ranges under the mono default are tofu. Per-element override is
                // the only fallback such renderers honor; the cell is absolutely positioned anyway.
                let needs_face = sym.chars().all(|c| {
                    ('\u{2800}'..='\u{28ff}').contains(&c) || ('\u{2b00}'..='\u{2bff}').contains(&c)
                });
                let face = if needs_face {
                    r#" font-family="DejaVu Sans, Noto Sans Symbols2""#
                } else {
                    ""
                };
                svg.push_str(&format!(
                    r#"<text x="{px}" y="{py}" fill="{fg_hex}"{weight}{face}>{escaped}</text>"#,
                ));
            }
        }
        svg.push('\n');
    }

    svg.push_str("</svg>\n");
    svg
}

fn color_to_hex(c: Color) -> Option<String> {
    match c {
        Color::Rgb(r, g, b) => Some(format!("#{r:02x}{g:02x}{b:02x}")),
        Color::Black => Some("#000000".into()),
        Color::Red => Some("#cc0000".into()),
        Color::Green => Some("#00cc00".into()),
        Color::Yellow => Some("#cccc00".into()),
        Color::Blue => Some("#0000cc".into()),
        Color::Magenta => Some("#cc00cc".into()),
        Color::Cyan => Some("#00cccc".into()),
        Color::Gray => Some("#aaaaaa".into()),
        Color::DarkGray => Some("#555555".into()),
        Color::LightRed => Some("#ff5555".into()),
        Color::LightGreen => Some("#55ff55".into()),
        Color::LightYellow => Some("#ffff55".into()),
        Color::LightBlue => Some("#5555ff".into()),
        Color::LightMagenta => Some("#ff55ff".into()),
        Color::LightCyan => Some("#55ffff".into()),
        Color::White => Some("#ffffff".into()),
        Color::Reset => None,
        _ => None,
    }
}

fn color_to_ansi_fg(c: Color) -> Option<String> {
    match c {
        Color::Rgb(r, g, b) => Some(format!("\x1b[38;2;{r};{g};{b}m")),
        Color::Black => Some("\x1b[30m".into()),
        Color::Red => Some("\x1b[31m".into()),
        Color::Green => Some("\x1b[32m".into()),
        Color::Yellow => Some("\x1b[33m".into()),
        Color::Blue => Some("\x1b[34m".into()),
        Color::Magenta => Some("\x1b[35m".into()),
        Color::Cyan => Some("\x1b[36m".into()),
        Color::Gray => Some("\x1b[37m".into()),
        Color::DarkGray => Some("\x1b[90m".into()),
        Color::LightRed => Some("\x1b[91m".into()),
        Color::LightGreen => Some("\x1b[92m".into()),
        Color::LightYellow => Some("\x1b[93m".into()),
        Color::LightBlue => Some("\x1b[94m".into()),
        Color::LightMagenta => Some("\x1b[95m".into()),
        Color::LightCyan => Some("\x1b[96m".into()),
        Color::White => Some("\x1b[97m".into()),
        Color::Reset => None,
        _ => None,
    }
}

fn color_to_ansi_bg(c: Color) -> Option<String> {
    match c {
        Color::Rgb(r, g, b) => Some(format!("\x1b[48;2;{r};{g};{b}m")),
        Color::Black => Some("\x1b[40m".into()),
        Color::Red => Some("\x1b[41m".into()),
        Color::Green => Some("\x1b[42m".into()),
        Color::Yellow => Some("\x1b[43m".into()),
        Color::Blue => Some("\x1b[44m".into()),
        Color::Magenta => Some("\x1b[45m".into()),
        Color::Cyan => Some("\x1b[46m".into()),
        Color::Gray => Some("\x1b[47m".into()),
        Color::Reset => None,
        _ => None,
    }
}
