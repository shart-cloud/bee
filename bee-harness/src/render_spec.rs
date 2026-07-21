//! [`RenderSpec`] (003-visual-render, US6) — the validated, renderable output of a `render`-tool
//! Rhai script. Produced by the render tool, consumed by [`crate::repl::ReplOutput::render_widget`]
//! and [`crate::viz::render_to_ansi`].
//!
//! **This type deliberately contains NO ratatui or rhai types** (research D5): it is a pure serde
//! value so it can be recorded verbatim in the episode transcript (`ToolResult.render_spec`) and
//! re-rendered later, and so `ratatui`/`rhai` never leak toward `bee-core` through the transcript
//! (NFR-002/SC-019). It is `#[non_exhaustive]` — Slice 2 adds `Sprite`/`Animation` additively.

use serde::{Deserialize, Serialize};

/// One bar of a [`RenderSpec::BarChart`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bar {
    pub label: String,
    pub value: i64,
}

/// A named line-chart series (a sequence of [`Point`]s).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Series {
    pub label: String,
    pub points: Vec<Point>,
}

/// One `(x, y)` data point of a [`Series`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

/// One table row: its cells plus an optional row color (palette or basic-ANSI name).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Row {
    pub cells: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

/// One labelled dot of a [`RenderSpec::DotGrid`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Dot {
    pub label: String,
    pub state: DotState,
}

/// The colored state of a [`Dot`]: green / red / neutral.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DotState {
    Pass,
    Fail,
    Skip,
}

/// A layout direction — maps to a ratatui `Direction` at render time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Vertical,
    Horizontal,
}

/// The validated, renderable output of a Rhai render script. Variants mirror the Rhai drawing API
/// (contracts/rhai-api.md). Slice 1 covers the static widgets; `Sprite`/`Animation` land in Slice 2
/// (hence `#[non_exhaustive]`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum RenderSpec {
    BarChart {
        title: String,
        bars: Vec<Bar>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        x_label: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        y_label: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        color: Option<String>,
    },
    LineChart {
        title: String,
        series: Vec<Series>,
    },
    Sparkline {
        title: String,
        data: Vec<u64>,
    },
    Table {
        title: String,
        headers: Vec<String>,
        rows: Vec<Row>,
    },
    Gauge {
        title: String,
        value: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        color: Option<String>,
    },
    DotGrid {
        title: String,
        dots: Vec<Dot>,
    },
    Text {
        content: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        style: Option<String>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        bold: bool,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        dim: bool,
    },
    AsciiArt {
        lines: Vec<String>,
    },
    Separator,
    Layout {
        direction: Direction,
        children: Vec<RenderSpec>,
    },
}

impl RenderSpec {
    /// The number of drawable data elements this spec (and its children) carries — bars, table rows,
    /// and line-chart points summed. Used by the render API to enforce the 500-element structural cap
    /// (contracts/rhai-api.md Constraints).
    pub fn element_count(&self) -> usize {
        match self {
            RenderSpec::BarChart { bars, .. } => bars.len(),
            RenderSpec::LineChart { series, .. } => series.iter().map(|s| s.points.len()).sum(),
            RenderSpec::Sparkline { data, .. } => data.len(),
            RenderSpec::Table { rows, .. } => rows.len(),
            RenderSpec::DotGrid { dots, .. } => dots.len(),
            RenderSpec::Layout { children, .. } => children.iter().map(|c| c.element_count()).sum(),
            _ => 0,
        }
    }

    /// The maximum layout-nesting depth of this spec (a non-layout is depth 0). Used to enforce the
    /// ≤ 3 nesting cap.
    pub fn nesting_depth(&self) -> usize {
        match self {
            RenderSpec::Layout { children, .. } => {
                1 + children.iter().map(|c| c.nesting_depth()).max().unwrap_or(0)
            }
            _ => 0,
        }
    }

    /// A plain-text (no-ANSI) rendering of this spec — the fallback used by any [`ReplOutput`] that
    /// does not override `render_widget` (FR-024 / US6 AS-5). Always non-blank.
    pub fn to_ascii(&self) -> String {
        match self {
            RenderSpec::BarChart { title, bars, .. } => {
                let mut s = format!("{title}\n");
                let max = bars.iter().map(|b| b.value).max().unwrap_or(1).max(1);
                for b in bars {
                    let filled = ((b.value.max(0) as f64 / max as f64) * 20.0) as usize;
                    s.push_str(&format!("  {:<12} {} {}\n", b.label, "#".repeat(filled), b.value));
                }
                s
            }
            RenderSpec::LineChart { title, series } => {
                let mut s = format!("{title}\n");
                for ser in series {
                    s.push_str(&format!("  {} ({} points)\n", ser.label, ser.points.len()));
                }
                s
            }
            RenderSpec::Sparkline { title, data } => {
                format!("{title}\n  {data:?}\n")
            }
            RenderSpec::Table { title, headers, rows } => {
                let mut s = format!("{title}\n  {}\n", headers.join(" | "));
                for r in rows {
                    s.push_str(&format!("  {}\n", r.cells.join(" | ")));
                }
                s
            }
            RenderSpec::Gauge { title, value, label, .. } => {
                let pct = label.clone().unwrap_or_else(|| format!("{:.0}%", value * 100.0));
                format!("{title}: {pct}\n")
            }
            RenderSpec::DotGrid { title, dots } => {
                let mut s = format!("{title}\n  ");
                for d in dots {
                    s.push(match d.state {
                        DotState::Pass => 'P',
                        DotState::Fail => 'F',
                        DotState::Skip => '-',
                    });
                    s.push(' ');
                }
                s.push('\n');
                s
            }
            RenderSpec::Text { content, .. } => format!("{content}\n"),
            RenderSpec::AsciiArt { lines } => format!("{}\n", lines.join("\n")),
            RenderSpec::Separator => "----\n".to_string(),
            RenderSpec::Layout { children, .. } => {
                children.iter().map(|c| c.to_ascii()).collect::<Vec<_>>().join("\n")
            }
        }
    }

    /// A short human-readable label for the tool summary (contracts/render-tool.md).
    pub fn summary_noun(&self) -> String {
        match self {
            RenderSpec::BarChart { title, bars, .. } => {
                format!("a bar chart {title:?} with {} bars", bars.len())
            }
            RenderSpec::LineChart { title, series } => {
                format!("a line chart {title:?} with {} series", series.len())
            }
            RenderSpec::Sparkline { title, data } => {
                format!("a sparkline {title:?} with {} points", data.len())
            }
            RenderSpec::Table { title, headers, rows } => {
                format!("a {}-column table {title:?} with {} rows", headers.len(), rows.len())
            }
            RenderSpec::Gauge { title, value, .. } => {
                format!("a gauge {title:?} at {:.0}%", value * 100.0)
            }
            RenderSpec::DotGrid { title, dots } => {
                format!("a dot grid {title:?} with {} dots", dots.len())
            }
            RenderSpec::Text { .. } => "a text block".to_string(),
            RenderSpec::AsciiArt { lines } => format!("ASCII art ({} lines)", lines.len()),
            RenderSpec::Separator => "a separator".to_string(),
            RenderSpec::Layout { direction, children } => {
                let dir = match direction {
                    Direction::Vertical => "vsplit",
                    Direction::Horizontal => "hsplit",
                };
                format!("a {dir} layout ({} widgets)", children.len())
            }
        }
    }
}
