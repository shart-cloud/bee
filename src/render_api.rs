//! The Rhai drawing API (003-visual-render, US6, FR-019/FR-021): the builder custom-types and the
//! [`register`] function that wires every drawing function onto a Rhai [`Engine`]. Scripts build
//! opaque builders that accumulate into a [`RenderContext`]; `render(widget)` commits the final
//! [`RenderSpec`]. Only these functions exist on the engine — everything else is denied by omission
//! (contracts/rhai-api.md). Structural caps (nesting ≤ 4, ≤ 500 elements; grids ≤ 12×12, ≤ 64 cells)
//! are enforced at commit / cell-placement and surfaced as script errors (research D6).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use rhai::{Array, Dynamic, Engine, EvalAltResult};

use crate::render_spec::{
    AnimationSpec, Bar, Direction, Dot, DotState, EffectSpec, GridCell, HeatRow, PanelOp, Point,
    RenderSpec, Renderable, Row, Series, SpriteSpec,
};
use crate::visual_gate;

pub mod effect_api;

/// Max layout-nesting depth. A `Grid` counts as one level (like a vsplit/hsplit), so 4 keeps a
/// grid-of-splits legal (grid-tui, M1 — was 3 pre-grid).
const MAX_NESTING: usize = 4;
const MAX_ELEMENTS: usize = 500;
/// Max sprite edge in pixels (Slice 2, FR-028).
const MAX_SPRITE_DIM: i64 = 32;
/// Max frames per animation (Slice 2, FR-029).
const MAX_FRAMES: usize = 16;
/// Max palette entries (Slice 2, contracts/sprite-api.md).
const MAX_PALETTE: usize = 32;
/// Max grid edge in tracks (grid-tui, M1).
const MAX_GRID_DIM: i64 = 12;
/// Max cells in one grid (grid-tui, M1).
const MAX_CELLS: usize = 64;

/// The `Clone + Send`-able accumulator pushed into a render evaluation. Holds the last committed
/// widget and how many `render()` calls the script made (for the discarded-render note).
#[derive(Clone, Default)]
pub struct RenderContext {
    inner: Arc<Mutex<CtxInner>>,
}

#[derive(Default)]
struct CtxInner {
    /// The last `render()` widget — the inline commit (last writer wins, as it always has).
    inline: Option<RenderSpec>,
    /// The transition the agent attached to that widget (009 FR-022), if any.
    inline_effect: Option<EffectSpec>,
    /// A full-screen takeover request (009 FR-013a). Carried separately from `inline` because the
    /// visual gate has to see the *target* before any state mutates — inferring it from a widget
    /// property or a magic panel id would put the decision after the fact.
    overlay: Option<(RenderSpec, Option<u32>, Option<EffectSpec>)>,
    /// Ordered panel effects from `render_to` / `remove_panel` / `clear_panels`. A script may address
    /// several panels in one call, so this is a list, not a single commit.
    panel_ops: Vec<PanelOp>,
    render_calls: u32,
}

/// What one render script produced: an optional inline widget plus the panel effects it requested.
#[derive(Debug, Default, Clone)]
pub struct RenderOutcome {
    pub inline: Option<RenderSpec>,
    /// The transition attached to the inline widget (009 FR-022).
    pub inline_effect: Option<EffectSpec>,
    /// A full-screen takeover: the widget, the requested TTL, and its transition (009 FR-013a).
    pub overlay: Option<(RenderSpec, Option<u32>, Option<EffectSpec>)>,
    pub panel_ops: Vec<PanelOp>,
    /// Total commit calls (`render`/`render_to`), for the "earlier render discarded" note.
    pub render_calls: u32,
}

impl RenderOutcome {
    /// True when the script drew nothing at all (no inline widget, no panel effect).
    pub fn is_empty(&self) -> bool {
        self.inline.is_none() && self.panel_ops.is_empty() && self.overlay.is_none()
    }
}

impl RenderContext {
    /// Clear the context before a fresh evaluation.
    pub fn reset(&self) {
        *self.inner.lock().expect("render ctx") = CtxInner::default();
    }
    fn commit_inline(&self, r: Renderable) {
        let mut g = self.inner.lock().expect("render ctx");
        g.inline = Some(r.spec);
        g.inline_effect = r.effect;
        g.render_calls += 1;
    }
    fn commit_overlay(&self, r: Renderable, ttl_ms: Option<u32>) {
        let mut g = self.inner.lock().expect("render ctx");
        g.overlay = Some((r.spec, ttl_ms, r.effect));
        g.render_calls += 1;
    }
    fn push_op(&self, op: PanelOp) {
        let mut g = self.inner.lock().expect("render ctx");
        if matches!(op, PanelOp::Upsert { .. }) {
            g.render_calls += 1;
        }
        g.panel_ops.push(op);
    }
    /// Take everything the script produced.
    pub fn take(&self) -> RenderOutcome {
        let mut g = self.inner.lock().expect("render ctx");
        RenderOutcome {
            inline: g.inline.take(),
            inline_effect: g.inline_effect.take(),
            overlay: g.overlay.take(),
            panel_ops: std::mem::take(&mut g.panel_ops),
            render_calls: g.render_calls,
        }
    }
}

/// The hard floor below which the full-screen front-end renders nothing but "terminal too small"
/// (contracts/modes-and-cli.md). Mirrors `tui::app::LayoutMode`'s floor.
const HARD_FLOOR: (u16, u16) = (40, 10);

/// Fail the script when the active surface cannot display `spec` (008-grid-tui).
///
/// The render tool has no screen of its own, so without this a script could commit a 10-column grid
/// into a 48-column panel — or anything at all into a 30×8 terminal — and the model would be told
/// "Rendered …" while nothing legible appeared. Every message names the concrete next action so the
/// agent can adapt rather than repeat itself.
///
/// Skipped entirely when the viewport is unconstrained (headless episodes, batch runs, tests), so a
/// non-interactive run never fails a render.
fn check_fits(spec: &RenderSpec, panel_id: Option<&str>) -> Result<(), Box<EvalAltResult>> {
    let vp = crate::viz::viewport::get();
    if !vp.constrained {
        return Ok(());
    }

    // 1. Nothing is displayable below the hard floor. Full-screen only: the inline REPL scrolls, so
    // its height is never a constraint — only width can make output unreadable there.
    if vp.full_screen && (vp.cols < HARD_FLOOR.0 || vp.rows < HARD_FLOOR.1) {
        return Err(format!(
            "render: the terminal is too small to display anything ({}×{}; {}×{} minimum). \
             Ask the operator to enlarge the terminal before rendering.",
            vp.cols, vp.rows, HARD_FLOOR.0, HARD_FLOOR.1
        )
        .into());
    }

    let need = crate::viz::buffer_render::min_width(spec);

    match panel_id {
        // 2. An inline render must fit the chat pane.
        None => {
            if vp.inline_cols > 0 && need > vp.inline_cols {
                return Err(format!(
                    "render: this widget needs at least {need} columns but the chat pane is only {}. \
                     Simplify it (fewer grid columns / table columns) or split it across turns.",
                    vp.inline_cols
                )
                .into());
            }
        }
        Some(id) => {
            // In the inline REPL a panel render falls back into the chat flow, so the panel column's
            // limits don't apply — check it against the inline width instead.
            if !vp.full_screen {
                if vp.inline_cols > 0 && need > vp.inline_cols {
                    return Err(format!(
                        "render_to: this widget needs at least {need} columns but the surface is \
                         only {}. Simplify it or render fewer columns.",
                        vp.inline_cols
                    )
                    .into());
                }
                return Ok(());
            }
            // 3. A *new* panel needs a free slot; updating an existing one always fits.
            if !vp.has_panel(id) && vp.panel_slots_free == 0 {
                return Err(format!(
                    "render_to: the panel column is full ({} live, no room for {id:?}). \
                     Call remove_panel(id) or clear_panels() to reclaim space, or reuse one of these \
                     ids: {}.",
                    vp.live_panels.len(),
                    vp.live_panels.join(", ")
                )
                .into());
            }
            // 4. And it must fit the column's width.
            if vp.panel_cols > 0 && need > vp.panel_cols {
                return Err(format!(
                    "render_to: this widget needs at least {need} columns but a panel is only {} \
                     wide. Use fewer grid/table columns, or render() it inline where there is more \
                     room ({} columns).",
                    vp.panel_cols, vp.inline_cols
                )
                .into());
            }
        }
    }
    Ok(())
}

/// Validate a panel id (contracts/rhai-panel-api.md): 1–32 chars of `[a-z0-9_-]`. A bad id is a
/// fail-closed script error, matching the drawing API's error style.
fn validate_panel_id(id: &str) -> Result<(), Box<EvalAltResult>> {
    // `takeover` is the id an over-ambitious `render_fullscreen` is downgraded onto (research R7).
    // A script that writes to it directly would be able to forge or clobber a downgraded takeover,
    // so it is reserved — the error names the verb the script actually wanted.
    if id == visual_gate::TAKEOVER_PANEL_ID {
        return Err(format!(
            "render_to: panel id {id:?} is reserved for downgraded full-screen renders. \
             Use render_fullscreen(widget) to request a takeover, or pick another id."
        )
        .into());
    }
    if id.is_empty() || id.len() > 32 {
        return Err(format!("render_to: panel id must be 1–32 chars (got {})", id.len()).into());
    }
    if let Some(bad) = id
        .chars()
        .find(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '_' || *c == '-'))
    {
        return Err(format!(
            "render_to: panel id {id:?} has an invalid char {bad:?} (want [a-z0-9_-])"
        )
        .into());
    }
    Ok(())
}

/// What kind of chart a [`ChartBuilder`] is accumulating.
#[derive(Clone, Copy, PartialEq)]
enum ChartKind {
    Bar,
    Line,
    Scatter,
    Area,
    Spark,
}

/// Fluent builder behind `bar_chart` / `line_chart` / `sparkline`.
#[derive(Clone)]
pub struct ChartBuilder {
    kind: ChartKind,
    title: String,
    bars: Vec<Bar>,
    series: Vec<(String, Arc<Mutex<Vec<Point>>>)>,
    spark: Vec<u64>,
    x_label: Option<String>,
    y_label: Option<String>,
    color: Option<String>,
    /// The transition the agent attached with `widget.effect(e)` (009 FR-022).
    /// `None` means the default transition for the target, never "no animation".
    effect: Option<EffectSpec>,
}

impl ChartBuilder {
    fn new(kind: ChartKind, title: String) -> Self {
        ChartBuilder {
            kind,
            title,
            bars: Vec::new(),
            series: Vec::new(),
            spark: Vec::new(),
            x_label: None,
            y_label: None,
            color: None,
            effect: None,
        }
    }
    fn to_spec(&self) -> RenderSpec {
        match self.kind {
            ChartKind::Bar => RenderSpec::BarChart {
                title: self.title.clone(),
                bars: self.bars.clone(),
                x_label: self.x_label.clone(),
                y_label: self.y_label.clone(),
                color: self.color.clone(),
            },
            ChartKind::Line => RenderSpec::LineChart {
                title: self.title.clone(),
                series: self
                    .series
                    .iter()
                    .map(|(label, pts)| Series {
                        label: label.clone(),
                        points: pts.lock().expect("series").clone(),
                    })
                    .collect(),
            },
            ChartKind::Scatter => RenderSpec::ScatterPlot {
                title: self.title.clone(),
                series: self
                    .series
                    .iter()
                    .map(|(label, pts)| Series {
                        label: label.clone(),
                        points: pts.lock().expect("series").clone(),
                    })
                    .collect(),
            },
            ChartKind::Area => RenderSpec::AreaChart {
                title: self.title.clone(),
                series: self
                    .series
                    .iter()
                    .map(|(label, pts)| Series {
                        label: label.clone(),
                        points: pts.lock().expect("series").clone(),
                    })
                    .collect(),
                color: self.color.clone(),
            },
            ChartKind::Spark => RenderSpec::Sparkline {
                title: self.title.clone(),
                data: self.spark.clone(),
            },
        }
    }
}

/// Handle to one line-chart series; `point` mutates the shared point list also held by the chart.
#[derive(Clone)]
pub struct SeriesHandle {
    points: Arc<Mutex<Vec<Point>>>,
}

/// Builder behind `heatmap`.
#[derive(Clone)]
pub struct HeatmapBuilder {
    title: String,
    rows: Vec<HeatRow>,
    color: Option<String>,
    effect: Option<EffectSpec>,
}

impl HeatmapBuilder {
    fn to_spec(&self) -> RenderSpec {
        RenderSpec::Heatmap {
            title: self.title.clone(),
            rows: self.rows.clone(),
            color: self.color.clone(),
        }
    }
}

/// Builder behind `table`.
#[derive(Clone)]
pub struct TableBuilder {
    title: String,
    headers: Vec<String>,
    rows: Vec<Row>,
    /// The transition the agent attached with `widget.effect(e)` (009 FR-022).
    /// `None` means the default transition for the target, never "no animation".
    effect: Option<EffectSpec>,
}

impl TableBuilder {
    fn to_spec(&self) -> RenderSpec {
        RenderSpec::Table {
            title: self.title.clone(),
            headers: self.headers.clone(),
            rows: self.rows.clone(),
        }
    }
}

/// Builder behind `gauge`.
#[derive(Clone)]
pub struct GaugeBuilder {
    title: String,
    value: f64,
    label: Option<String>,
    color: Option<String>,
    /// The transition the agent attached with `widget.effect(e)` (009 FR-022).
    /// `None` means the default transition for the target, never "no animation".
    effect: Option<EffectSpec>,
}

impl GaugeBuilder {
    fn to_spec(&self) -> RenderSpec {
        RenderSpec::Gauge {
            title: self.title.clone(),
            value: self.value,
            label: self.label.clone(),
            color: self.color.clone(),
        }
    }
}

/// Builder behind `dots`.
#[derive(Clone)]
pub struct DotGridBuilder {
    title: String,
    dots: Vec<Dot>,
    /// The transition the agent attached with `widget.effect(e)` (009 FR-022).
    /// `None` means the default transition for the target, never "no animation".
    effect: Option<EffectSpec>,
}

impl DotGridBuilder {
    fn to_spec(&self) -> RenderSpec {
        RenderSpec::DotGrid {
            title: self.title.clone(),
            dots: self.dots.clone(),
        }
    }
}

/// Builder behind `log_tail`.
#[derive(Clone)]
pub struct LogTailBuilder {
    title: String,
    lines: Vec<crate::render_spec::LogLine>,
    max_rows: Option<u16>,
    /// The transition the agent attached with `widget.effect(e)` (009 FR-022).
    /// `None` means the default transition for the target, never "no animation".
    effect: Option<EffectSpec>,
}

impl LogTailBuilder {
    fn to_spec(&self) -> RenderSpec {
        RenderSpec::LogTail {
            title: self.title.clone(),
            lines: self.lines.clone(),
            max_rows: self.max_rows,
        }
    }
}

/// Builder behind `text`.
#[derive(Clone)]
pub struct TextBuilder {
    content: String,
    style: Option<String>,
    bold: bool,
    dim: bool,
    /// The transition the agent attached with `widget.effect(e)` (009 FR-022).
    /// `None` means the default transition for the target, never "no animation".
    effect: Option<EffectSpec>,
}

impl TextBuilder {
    fn to_spec(&self) -> RenderSpec {
        RenderSpec::Text {
            content: self.content.clone(),
            style: self.style.clone(),
            bold: self.bold,
            dim: self.dim,
        }
    }
}

/// Builder behind `vsplit` / `hsplit`.
#[derive(Clone)]
pub struct LayoutBuilder {
    direction: Direction,
    children: Vec<RenderSpec>,
    /// The transition the agent attached with `widget.effect(e)` (009 FR-022).
    /// `None` means the default transition for the target, never "no animation".
    effect: Option<EffectSpec>,
}

impl LayoutBuilder {
    fn to_spec(&self) -> RenderSpec {
        RenderSpec::Layout {
            direction: self.direction,
            children: self.children.clone(),
        }
    }
}

/// Builder behind `grid` (grid-tui, M1): an N×M grid the script fills cell by cell. Cells are
/// validated in-bounds and non-overlapping as they're added, so a committed grid is always valid.
#[derive(Clone)]
pub struct GridBuilder {
    rows: u16,
    cols: u16,
    col_weights: Vec<u16>,
    row_weights: Vec<u16>,
    gap: Option<u16>,
    cells: Vec<GridCell>,
    /// The transition the agent attached with `widget.effect(e)` (009 FR-022).
    /// `None` means the default transition for the target, never "no animation".
    effect: Option<EffectSpec>,
}

impl GridBuilder {
    fn to_spec(&self) -> RenderSpec {
        RenderSpec::Grid {
            rows: self.rows,
            cols: self.cols,
            col_weights: self.col_weights.clone(),
            row_weights: self.row_weights.clone(),
            gap: self.gap,
            cells: self.cells.clone(),
        }
    }
}

/// Whether a proposed `(row, col, row_span, col_span)` rectangle overlaps an already-placed cell.
fn cell_overlaps(c: &GridCell, row: u16, col: u16, rs: u16, cs: u16) -> bool {
    let (a_r0, a_r1, a_c0, a_c1) = (c.row, c.row + c.row_span, c.col, c.col + c.col_span);
    let (b_r0, b_r1, b_c0, b_c1) = (row, row + rs, col, col + cs);
    a_r0 < b_r1 && b_r0 < a_r1 && a_c0 < b_c1 && b_c0 < a_c1
}

/// Place a cell into `g`, enforcing the cell cap, non-negative coords, spans ≥ 1, in-bounds, and
/// non-overlap. A violation is surfaced as a script error (contracts/rhai-api.md style).
fn push_cell(
    g: &mut GridBuilder,
    row: i64,
    col: i64,
    row_span: i64,
    col_span: i64,
    content: RenderSpec,
) -> Result<(), Box<EvalAltResult>> {
    if g.cells.len() >= MAX_CELLS {
        return Err(format!("render: grid exceeds {MAX_CELLS} cells").into());
    }
    if row < 0 || col < 0 || row_span < 1 || col_span < 1 {
        return Err("render: grid cell needs row/col ≥ 0 and spans ≥ 1".into());
    }
    let (row, col, rs, cs) = (row as u16, col as u16, row_span as u16, col_span as u16);
    if row + rs > g.rows || col + cs > g.cols {
        return Err(format!(
            "render: cell ({row},{col}) span {rs}×{cs} exceeds the {}×{} grid",
            g.rows, g.cols
        )
        .into());
    }
    if let Some(hit) = g.cells.iter().find(|c| cell_overlaps(c, row, col, rs, cs)) {
        return Err(format!(
            "render: cell ({row},{col}) overlaps the cell at ({},{})",
            hit.row, hit.col
        )
        .into());
    }
    g.cells.push(GridCell {
        row,
        col,
        row_span: rs,
        col_span: cs,
        title: None,
        content,
    });
    Ok(())
}

/// Builder behind `palette` (Slice 2, FR-028): a char→color map (`None` = transparent).
#[derive(Clone, Default)]
pub struct PaletteBuilder {
    map: HashMap<char, Option<(u8, u8, u8)>>,
}

/// Builder behind `sprite` (Slice 2, FR-028): a `width × height` pixel canvas painted from a palette.
#[derive(Clone)]
pub struct SpriteBuilder {
    width: u16,
    height: u16,
    palette: HashMap<char, Option<(u8, u8, u8)>>,
    pixels: Vec<Option<(u8, u8, u8)>>,
    /// The transition the agent attached with `widget.effect(e)` (009 FR-022).
    /// `None` means the default transition for the target, never "no animation".
    effect: Option<EffectSpec>,
}

impl SpriteBuilder {
    fn to_spec(&self) -> SpriteSpec {
        SpriteSpec {
            width: self.width,
            height: self.height,
            pixels: self.pixels.clone(),
        }
    }
}

/// Builder behind `animation` (Slice 2, FR-029).
#[derive(Clone)]
pub struct AnimationBuilder {
    interval_ms: u64,
    frames: Vec<SpriteSpec>,
    bounce: bool,
    cycles: u32,
    /// The transition the agent attached with `widget.effect(e)` (009 FR-022).
    /// `None` means the default transition for the target, never "no animation".
    effect: Option<EffectSpec>,
}

impl AnimationBuilder {
    fn to_spec(&self) -> AnimationSpec {
        AnimationSpec {
            frames: self.frames.clone(),
            interval_ms: self.interval_ms,
            bounce: self.bounce,
            cycles: self.cycles,
        }
    }
}

/// Parse `"#RRGGBB"` or `"transparent"` to an optional RGB pixel; a bad value is a script error.
fn parse_color(s: &str) -> Result<Option<(u8, u8, u8)>, Box<EvalAltResult>> {
    let s = s.trim();
    if s.eq_ignore_ascii_case("transparent") {
        return Ok(None);
    }
    let h = s.strip_prefix('#').unwrap_or(s);
    if h.len() == 6 {
        if let (Ok(r), Ok(g), Ok(b)) = (
            u8::from_str_radix(&h[0..2], 16),
            u8::from_str_radix(&h[2..4], 16),
            u8::from_str_radix(&h[4..6], 16),
        ) {
            return Ok(Some((r, g, b)));
        }
    }
    Err(format!("render: bad color {s:?} (want #RRGGBB or \"transparent\")").into())
}

/// Convert a Rhai `Array` of ints to `Vec<u64>` (negatives clamped to 0).
fn array_to_u64(a: Array) -> Vec<u64> {
    a.into_iter()
        .map(|d| d.as_int().unwrap_or(0).max(0) as u64)
        .collect()
}

/// Convert a Rhai `Array` to `Vec<String>` (each element stringified).
fn array_to_strings(a: Array) -> Vec<String> {
    a.into_iter()
        .map(|d| d.into_string().unwrap_or_default())
        .collect()
}

/// Convert a Rhai `Array` of ints to `Vec<u16>` track weights (each clamped to 1..=255).
fn array_to_u16(a: Array) -> Vec<u16> {
    a.into_iter()
        .map(|d| d.as_int().unwrap_or(1).clamp(1, 255) as u16)
        .collect()
}

/// Turn any builder/`RenderSpec` `Dynamic` into a [`RenderSpec`], or a script error.
/// Convert a Rhai value into its spec, discarding any attached transition. For the commit verbs,
/// which need both, use [`dynamic_to_renderable`].
fn dynamic_to_spec(d: Dynamic) -> Result<RenderSpec, Box<EvalAltResult>> {
    dynamic_to_renderable(d).map(|r| r.spec)
}

/// Convert a Rhai value into the widget **and** the transition the agent attached to it (FR-022).
///
/// One dispatch for both, so a widget type can never be renderable but effect-blind — adding a
/// builder means adding it here once, not in two places that can disagree.
fn dynamic_to_renderable(d: Dynamic) -> Result<Renderable, Box<EvalAltResult>> {
    // A bare `RenderSpec` (`separator()`, `ascii_art(...)`) has nowhere to hold an effect, so
    // `effect()` wraps it into a `Renderable` instead — handled first, above the builders.
    if d.is::<Renderable>() {
        return Ok(d.cast::<Renderable>());
    }
    if d.is::<RenderSpec>() {
        return Ok(Renderable::plain(d.cast::<RenderSpec>()));
    }
    if d.is::<ChartBuilder>() {
        let b = d.cast::<ChartBuilder>();
        return Ok(Renderable {
            spec: b.to_spec(),
            effect: b.effect,
        });
    }
    if d.is::<HeatmapBuilder>() {
        let b = d.cast::<HeatmapBuilder>();
        return Ok(Renderable {
            spec: b.to_spec(),
            effect: b.effect,
        });
    }
    if d.is::<TableBuilder>() {
        let b = d.cast::<TableBuilder>();
        return Ok(Renderable {
            spec: b.to_spec(),
            effect: b.effect,
        });
    }
    if d.is::<GaugeBuilder>() {
        let b = d.cast::<GaugeBuilder>();
        return Ok(Renderable {
            spec: b.to_spec(),
            effect: b.effect,
        });
    }
    if d.is::<DotGridBuilder>() {
        let b = d.cast::<DotGridBuilder>();
        return Ok(Renderable {
            spec: b.to_spec(),
            effect: b.effect,
        });
    }
    if d.is::<LogTailBuilder>() {
        let b = d.cast::<LogTailBuilder>();
        return Ok(Renderable {
            spec: b.to_spec(),
            effect: b.effect,
        });
    }
    if d.is::<TextBuilder>() {
        let b = d.cast::<TextBuilder>();
        return Ok(Renderable {
            spec: b.to_spec(),
            effect: b.effect,
        });
    }
    if d.is::<LayoutBuilder>() {
        let b = d.cast::<LayoutBuilder>();
        return Ok(Renderable {
            spec: b.to_spec(),
            effect: b.effect,
        });
    }
    if d.is::<GridBuilder>() {
        let b = d.cast::<GridBuilder>();
        return Ok(Renderable {
            spec: b.to_spec(),
            effect: b.effect,
        });
    }
    if d.is::<SpriteBuilder>() {
        let b = d.cast::<SpriteBuilder>();
        return Ok(Renderable {
            spec: RenderSpec::Sprite { spec: b.to_spec() },
            effect: b.effect,
        });
    }
    if d.is::<AnimationBuilder>() {
        let b = d.cast::<AnimationBuilder>();
        return Ok(Renderable {
            spec: RenderSpec::Animation { spec: b.to_spec() },
            effect: b.effect,
        });
    }
    Err("render: value is not a renderable widget".into())
}

/// A layout is a single static render, so an animation added to one collapses to its first frame
/// (contracts/sprite-api.md).
fn flatten_for_layout(spec: RenderSpec) -> RenderSpec {
    match spec {
        RenderSpec::Animation { spec: a } => match a.frames.into_iter().next() {
            Some(frame) => RenderSpec::Sprite { spec: frame },
            None => RenderSpec::Separator,
        },
        other => other,
    }
}

/// Validate the structural caps (research D6): nesting ≤ 3, total elements ≤ 500.
fn validate(spec: &RenderSpec) -> Result<(), Box<EvalAltResult>> {
    if spec.nesting_depth() > MAX_NESTING {
        return Err(format!("render: layout nesting exceeds max depth {MAX_NESTING}").into());
    }
    if spec.element_count() > MAX_ELEMENTS {
        return Err(format!("render: total elements exceed {MAX_ELEMENTS}").into());
    }
    Ok(())
}

/// Register every drawing function (contracts/rhai-api.md) onto `engine`. `ctx` is captured by the
/// `render` closure so committed widgets survive the evaluation.
pub fn register(engine: &mut Engine, ctx: RenderContext) {
    engine.register_type_with_name::<ChartBuilder>("Chart");
    engine.register_type_with_name::<SeriesHandle>("Series");
    engine.register_type_with_name::<TableBuilder>("Table");
    engine.register_type_with_name::<GaugeBuilder>("Gauge");
    engine.register_type_with_name::<DotGridBuilder>("DotGrid");
    engine.register_type_with_name::<LogTailBuilder>("LogTail");
    engine.register_type_with_name::<TextBuilder>("Text");
    engine.register_type_with_name::<LayoutBuilder>("Layout");
    engine.register_type_with_name::<RenderSpec>("Widget");
    engine.register_type_with_name::<Renderable>("Widget");

    // --- Charts ---
    engine.register_fn("bar_chart", |title: String| {
        ChartBuilder::new(ChartKind::Bar, title)
    });
    engine.register_fn("line_chart", |title: String| {
        ChartBuilder::new(ChartKind::Line, title)
    });
    engine.register_fn("scatter", |title: String| {
        ChartBuilder::new(ChartKind::Scatter, title)
    });
    engine.register_fn("area_chart", |title: String| {
        ChartBuilder::new(ChartKind::Area, title)
    });
    engine.register_fn("sparkline", |title: String, data: Array| {
        let mut c = ChartBuilder::new(ChartKind::Spark, title);
        c.spark = array_to_u64(data);
        c
    });
    engine.register_fn("bar", |c: &mut ChartBuilder, label: String, value: i64| {
        c.bars.push(Bar { label, value });
    });
    engine.register_fn("x_label", |c: &mut ChartBuilder, s: String| {
        c.x_label = Some(s)
    });
    engine.register_fn("y_label", |c: &mut ChartBuilder, s: String| {
        c.y_label = Some(s)
    });
    engine.register_fn("color", |c: &mut ChartBuilder, name: String| {
        c.color = Some(name)
    });
    engine.register_fn("series", |c: &mut ChartBuilder, label: String| {
        let pts = Arc::new(Mutex::new(Vec::new()));
        c.series.push((label, pts.clone()));
        SeriesHandle { points: pts }
    });
    engine.register_fn("point", |s: &mut SeriesHandle, x: f64, y: f64| {
        s.points.lock().expect("series").push(Point { x, y });
    });
    engine.register_fn("point", |s: &mut SeriesHandle, x: i64, y: i64| {
        s.points.lock().expect("series").push(Point {
            x: x as f64,
            y: y as f64,
        });
    });

    // --- Heatmaps ---
    engine.register_type_with_name::<HeatmapBuilder>("Heatmap");
    engine.register_fn("heatmap", |title: String| HeatmapBuilder {
        title,
        rows: Vec::new(),
        color: None,
        effect: None,
    });
    engine.register_fn(
        "row",
        |h: &mut HeatmapBuilder, label: String, values: Array| {
            h.rows.push(HeatRow {
                label,
                values: values
                    .into_iter()
                    .map(|v| {
                        if let Ok(f) = v.as_float() {
                            f
                        } else {
                            v.as_int().unwrap_or(0) as f64
                        }
                    })
                    .collect(),
            });
        },
    );
    engine.register_fn("color", |h: &mut HeatmapBuilder, name: String| {
        h.color = Some(name);
    });

    // --- Tables ---
    engine.register_fn("table", |title: String| TableBuilder {
        title,
        headers: Vec::new(),
        rows: Vec::new(),
        effect: None,
    });
    engine.register_fn("header", |t: &mut TableBuilder, cols: Array| {
        t.headers = array_to_strings(cols);
    });
    engine.register_fn("row", |t: &mut TableBuilder, cells: Array| {
        t.rows.push(Row {
            cells: array_to_strings(cells),
            color: None,
        });
    });
    engine.register_fn(
        "row_colored",
        |t: &mut TableBuilder, cells: Array, color: String| {
            t.rows.push(Row {
                cells: array_to_strings(cells),
                color: Some(color),
            });
        },
    );

    // --- Status & indicators ---
    engine.register_fn("gauge", |title: String, value: f64| GaugeBuilder {
        title,
        value: value.clamp(0.0, 1.0),
        label: None,
        color: None,
        effect: None,
    });
    engine.register_fn("label", |g: &mut GaugeBuilder, s: String| g.label = Some(s));
    engine.register_fn("color", |g: &mut GaugeBuilder, name: String| {
        g.color = Some(name)
    });
    engine.register_fn("log_tail", |title: String| LogTailBuilder {
        title,
        lines: Vec::new(),
        max_rows: None,
        effect: None,
    });
    engine.register_fn("line", |l: &mut LogTailBuilder, text: String| {
        l.lines
            .push(crate::render_spec::LogLine { text, level: None });
    });
    engine.register_fn(
        "line",
        |l: &mut LogTailBuilder, text: String, level: String| {
            l.lines.push(crate::render_spec::LogLine {
                text,
                level: Some(level),
            });
        },
    );
    engine.register_fn("max_rows", |l: &mut LogTailBuilder, n: i64| {
        l.max_rows = Some(n.clamp(1, 40) as u16);
    });

    engine.register_fn("dots", |title: String| DotGridBuilder {
        title,
        dots: Vec::new(),
        effect: None,
    });
    engine.register_fn("pass", |d: &mut DotGridBuilder, label: String| {
        d.dots.push(Dot {
            label,
            state: DotState::Pass,
        });
    });
    engine.register_fn("fail", |d: &mut DotGridBuilder, label: String| {
        d.dots.push(Dot {
            label,
            state: DotState::Fail,
        });
    });
    engine.register_fn("skip", |d: &mut DotGridBuilder, label: String| {
        d.dots.push(Dot {
            label,
            state: DotState::Skip,
        });
    });

    // --- Decorative / identity ---
    engine.register_fn("text", |content: String| TextBuilder {
        content,
        style: None,
        bold: false,
        dim: false,
        effect: None,
    });
    engine.register_fn("style", |t: &mut TextBuilder, name: String| {
        t.style = Some(name)
    });
    engine.register_fn("color", |t: &mut TextBuilder, name: String| {
        t.style = Some(name)
    });
    engine.register_fn("bold", |t: &mut TextBuilder| t.bold = true);
    engine.register_fn("dim", |t: &mut TextBuilder| t.dim = true);
    // Markdown the agent writes as source and bee renders through the active theme (010). One verb,
    // no builder: there is nothing to configure — the look belongs to the operator's theme, not to
    // the script.
    engine.register_fn("markdown", |content: String| -> Renderable {
        RenderSpec::Markdown { content }.into()
    });
    engine.register_fn("ascii_art", |lines: Array| -> Renderable {
        RenderSpec::AsciiArt {
            lines: array_to_strings(lines),
        }
        .into()
    });
    engine.register_fn("separator", || -> Renderable {
        Renderable::plain(RenderSpec::Separator)
    });

    // --- Layout & composition ---
    engine.register_fn("vsplit", || LayoutBuilder {
        direction: Direction::Vertical,
        children: Vec::new(),
        effect: None,
    });
    engine.register_fn("hsplit", || LayoutBuilder {
        direction: Direction::Horizontal,
        children: Vec::new(),
        effect: None,
    });
    engine.register_fn(
        "add",
        |l: &mut LayoutBuilder, widget: Dynamic| -> Result<(), Box<EvalAltResult>> {
            l.children
                .push(flatten_for_layout(dynamic_to_spec(widget)?));
            Ok(())
        },
    );

    // --- Grid (grid-tui, M1): a model-defined N×M grid; cells hold any widget ---
    engine.register_type_with_name::<GridBuilder>("Grid");
    engine.register_fn(
        "grid",
        |rows: i64, cols: i64| -> Result<GridBuilder, Box<EvalAltResult>> {
            if !(1..=MAX_GRID_DIM).contains(&rows) || !(1..=MAX_GRID_DIM).contains(&cols) {
                return Err(format!("render: grid dimensions must be 1..={MAX_GRID_DIM}").into());
            }
            Ok(GridBuilder {
                rows: rows as u16,
                cols: cols as u16,
                col_weights: Vec::new(),
                row_weights: Vec::new(),
                gap: None,
                cells: Vec::new(),
                effect: None,
            })
        },
    );
    engine.register_fn(
        "cell",
        |g: &mut GridBuilder,
         row: i64,
         col: i64,
         widget: Dynamic|
         -> Result<(), Box<EvalAltResult>> {
            let spec = dynamic_to_spec(widget)?;
            push_cell(g, row, col, 1, 1, flatten_for_layout(spec))
        },
    );
    engine.register_fn(
        "span",
        |g: &mut GridBuilder,
         row: i64,
         col: i64,
         row_span: i64,
         col_span: i64,
         widget: Dynamic|
         -> Result<(), Box<EvalAltResult>> {
            let spec = dynamic_to_spec(widget)?;
            push_cell(g, row, col, row_span, col_span, flatten_for_layout(spec))
        },
    );
    engine.register_fn("col_weights", |g: &mut GridBuilder, w: Array| {
        g.col_weights = array_to_u16(w);
    });
    engine.register_fn("row_weights", |g: &mut GridBuilder, w: Array| {
        g.row_weights = array_to_u16(w);
    });
    engine.register_fn("gap", |g: &mut GridBuilder, n: i64| {
        g.gap = Some(n.clamp(0, 8) as u16);
    });

    // --- Sprites & animation (Slice 2) ---
    engine.register_type_with_name::<PaletteBuilder>("Palette");
    engine.register_type_with_name::<SpriteBuilder>("Sprite");
    engine.register_type_with_name::<AnimationBuilder>("Animation");

    engine.register_fn("palette", PaletteBuilder::default);
    engine.register_fn(
        "set",
        |p: &mut PaletteBuilder, key: String, color: String| -> Result<(), Box<EvalAltResult>> {
            let c = key
                .chars()
                .next()
                .ok_or::<Box<EvalAltResult>>("render: empty palette key".into())?;
            if p.map.len() >= MAX_PALETTE && !p.map.contains_key(&c) {
                return Err(format!("render: palette exceeds {MAX_PALETTE} entries").into());
            }
            p.map.insert(c, parse_color(&color)?);
            Ok(())
        },
    );
    engine.register_fn(
        "sprite",
        |w: i64, h: i64, pal: PaletteBuilder| -> Result<SpriteBuilder, Box<EvalAltResult>> {
            if !(1..=MAX_SPRITE_DIM).contains(&w) || !(1..=MAX_SPRITE_DIM).contains(&h) {
                return Err(
                    format!("render: sprite dimensions must be 1..={MAX_SPRITE_DIM}").into(),
                );
            }
            Ok(SpriteBuilder {
                width: w as u16,
                height: h as u16,
                palette: pal.map,
                pixels: vec![None; (w * h) as usize],
                effect: None,
            })
        },
    );
    engine.register_fn("paint", |s: &mut SpriteBuilder, rows: Array| {
        for (y, row) in rows.into_iter().enumerate() {
            if y >= s.height as usize {
                break;
            }
            let line = row.into_string().unwrap_or_default();
            for (x, ch) in line.chars().enumerate() {
                if x >= s.width as usize {
                    break;
                }
                s.pixels[y * s.width as usize + x] = s.palette.get(&ch).copied().flatten();
            }
        }
    });
    engine.register_fn(
        "set",
        |s: &mut SpriteBuilder, x: i64, y: i64, color: String| -> Result<(), Box<EvalAltResult>> {
            if x >= 0 && y >= 0 && (x as u16) < s.width && (y as u16) < s.height {
                s.pixels[y as usize * s.width as usize + x as usize] = parse_color(&color)?;
            }
            Ok(())
        },
    );
    engine.register_fn(
        "fill",
        |s: &mut SpriteBuilder, color: String| -> Result<(), Box<EvalAltResult>> {
            let c = parse_color(&color)?;
            s.pixels.iter_mut().for_each(|p| *p = c);
            Ok(())
        },
    );
    engine.register_fn("animation", |ms: i64| AnimationBuilder {
        interval_ms: ms.clamp(50, 1000) as u64,
        frames: Vec::new(),
        bounce: false,
        cycles: 1,
        effect: None,
    });
    engine.register_fn(
        "add",
        |a: &mut AnimationBuilder, sprite: SpriteBuilder| -> Result<(), Box<EvalAltResult>> {
            if a.frames.len() >= MAX_FRAMES {
                return Err(format!("render: animation exceeds {MAX_FRAMES} frames").into());
            }
            let spec = sprite.to_spec();
            if let Some(f0) = a.frames.first() {
                if f0.width != spec.width || f0.height != spec.height {
                    return Err("render: animation frames must share dimensions".into());
                }
            }
            a.frames.push(spec);
            Ok(())
        },
    );
    engine.register_fn("bounce", |a: &mut AnimationBuilder, b: bool| a.bounce = b);
    engine.register_fn("cycles", |a: &mut AnimationBuilder, n: i64| {
        a.cycles = n.max(0) as u32
    });
    engine.register_fn("bee_sprite", || -> Renderable {
        RenderSpec::Sprite {
            spec: crate::viz::bee::sprite(),
        }
        .into()
    });
    engine.register_fn("bee_animation", || -> Renderable {
        RenderSpec::Animation {
            spec: crate::viz::bee::animation(),
        }
        .into()
    });

    // --- Commit + panel lifecycle ---
    // `render(widget)` commits inline into the chat flow (unchanged). `render_to(id, widget)` creates
    // or replaces a named panel; `render_to_ttl` gives it an expiry; `remove_panel`/`clear_panels`
    // reclaim space (008-grid-tui, FR-008; contracts/rhai-panel-api.md). Same structural caps apply to
    // every widget — no new non-drawing engine capability is registered (Constitution I, FR-020).
    let inline_ctx = ctx.clone();
    engine.register_fn(
        "render",
        move |widget: Dynamic| -> Result<(), Box<EvalAltResult>> {
            let r = dynamic_to_renderable(widget)?;
            validate(&r.spec)?;
            check_fits(&r.spec, None)?;
            inline_ctx.commit_inline(r);
            Ok(())
        },
    );
    let to_ctx = ctx.clone();
    engine.register_fn(
        "render_to",
        move |panel_id: String, widget: Dynamic| -> Result<(), Box<EvalAltResult>> {
            validate_panel_id(&panel_id)?;
            let r = dynamic_to_renderable(widget)?;
            validate(&r.spec)?;
            check_fits(&r.spec, Some(&panel_id))?;
            to_ctx.push_op(PanelOp::Upsert {
                id: panel_id,
                spec: r.spec,
                ttl_ms: None,
                effect: r.effect,
            });
            Ok(())
        },
    );
    let ttl_ctx = ctx.clone();
    engine.register_fn(
        "render_to_ttl",
        move |panel_id: String, widget: Dynamic, ttl_ms: i64| -> Result<(), Box<EvalAltResult>> {
            validate_panel_id(&panel_id)?;
            let r = dynamic_to_renderable(widget)?;
            validate(&r.spec)?;
            if ttl_ms <= 0 {
                return Err("render_to_ttl: ttl_ms must be > 0".into());
            }
            check_fits(&r.spec, Some(&panel_id))?;
            ttl_ctx.push_op(PanelOp::Upsert {
                id: panel_id,
                spec: r.spec,
                // Clamp to a day so a typo can't pin a panel effectively forever.
                ttl_ms: Some((ttl_ms as u64).min(24 * 60 * 60 * 1000)),
                effect: r.effect,
            });
            Ok(())
        },
    );
    let overlay_ctx = ctx.clone();
    let rm_ctx = ctx.clone();
    engine.register_fn(
        "remove_panel",
        move |panel_id: String| -> Result<(), Box<EvalAltResult>> {
            validate_panel_id(&panel_id)?;
            rm_ctx.push_op(PanelOp::Remove { id: panel_id });
            Ok(())
        },
    );
    engine.register_fn("clear_panels", move || {
        ctx.push_op(PanelOp::Clear);
    });

    // --- 009 US4: transitions and the takeover verbs ------------------------------------------
    effect_api::register(engine);

    // `widget.effect(e)` per builder type: Rhai dispatches method syntax on the receiver's concrete
    // type, so this is one registration each rather than one generic one. Every widget type is
    // listed, because "any widget" is the contract (FR-022).
    macro_rules! attach_effect {
        ($($t:ty),+ $(,)?) => {
            $(engine.register_fn("effect", |b: &mut $t, e: EffectSpec| {
                b.effect = Some(e);
            });)+
        };
    }
    attach_effect!(
        ChartBuilder,
        HeatmapBuilder,
        TableBuilder,
        GaugeBuilder,
        LogTailBuilder,
        DotGridBuilder,
        TextBuilder,
        LayoutBuilder,
        GridBuilder,
        SpriteBuilder,
        AnimationBuilder,
    );
    // `separator()`, `ascii_art()` and the mascot helpers have no builder, so they return a
    // `Renderable` — which is exactly a spec plus an effect slot. That is why every widget-producing
    // function on this surface returns something `effect()` can attach to.
    engine.register_fn("effect", |w: &mut Renderable, e: EffectSpec| {
        w.effect = Some(e);
    });

    let fs_ctx = overlay_ctx.clone();
    engine.register_fn(
        "render_fullscreen",
        move |widget: Dynamic| -> Result<(), Box<EvalAltResult>> {
            let r = dynamic_to_renderable(widget)?;
            validate(&r.spec)?;
            // Deliberately no `check_fits`: a takeover gets the whole chat area, which is the
            // roomiest surface there is. If it does not fit there, nothing would have helped.
            fs_ctx.commit_overlay(r, None);
            Ok(())
        },
    );
    engine.register_fn(
        "render_fullscreen_ttl",
        move |widget: Dynamic, ttl_ms: i64| -> Result<(), Box<EvalAltResult>> {
            let r = dynamic_to_renderable(widget)?;
            validate(&r.spec)?;
            // A non-positive lifetime is a script error, matching `render_to_ttl`. An over-long one
            // is not: the configured ceiling clamps it silently, because that is the operator's
            // decision rather than the script's mistake (FR-021).
            if ttl_ms <= 0 {
                return Err("render_fullscreen_ttl: ttl_ms must be > 0".into());
            }
            overlay_ctx.commit_overlay(r, Some(ttl_ms.min(u32::MAX as i64) as u32));
            Ok(())
        },
    );
}

/// Serializes tests that publish the process-global viewport. Shared with `effect_api`'s tests,
/// which run scripts through the same fit check against the same global.
#[cfg(test)]
pub(crate) static VP_LOCK: Mutex<()> = Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    /// Build an engine wired to a fresh context and run `script`, returning what it produced.
    /// Does **not** touch the viewport lock — callers below hold it.
    fn run_inner(script: &str) -> Result<RenderOutcome, String> {
        let ctx = RenderContext::default();
        let mut engine = Engine::new();
        register(&mut engine, ctx.clone());
        engine.run(script).map_err(|e| e.to_string())?;
        Ok(ctx.take())
    }

    /// Run `script` against an **unconstrained** viewport (the headless default), serialized so a
    /// concurrent fit-guard test can't leak its viewport into this one.
    fn run(script: &str) -> Result<RenderOutcome, String> {
        let _g = VP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::viz::viewport::reset();
        run_inner(script)
    }

    #[test]
    fn markdown_commits_its_source_not_a_rendering() {
        // 010: the spec carries source, so the transcript stays renderer-free and a replay
        // re-renders at the width the replaying terminal has.
        // `r##` because the markdown content contains `"#`, which would close an `r#` literal.
        let out = run(r##"render(markdown("# Title\n\nbody **here**"));"##).unwrap();
        let spec = out.inline.expect("committed inline");
        match spec {
            RenderSpec::Markdown { content } => {
                assert!(content.contains("# Title"));
                assert!(content.contains("**here**"), "source, verbatim");
            }
            other => panic!("expected a markdown spec, got {other:?}"),
        }
    }

    #[test]
    fn a_markdown_block_takes_an_effect_and_a_panel_like_any_other_widget() {
        // `effect` mutates the widget rather than chaining, so this is the two-statement form the
        // rest of the surface uses.
        let out =
            run(r##"let w = markdown("# Notes"); w.effect(fade_in(300)); render_to("notes", w);"##)
                .unwrap();
        assert_eq!(out.panel_ops.len(), 1);
        assert!(matches!(
            &out.panel_ops[0],
            PanelOp::Upsert {
                effect: Some(EffectSpec::FadeIn { ms: 300 }),
                ..
            }
        ));
    }

    #[test]
    fn log_tail_builds_lines_with_and_without_levels() {
        let out = run(concat!(
            r#"let l = log_tail("denials");"#,
            r#" l.line("policy loaded");"#,
            r#" l.line("open /etc/shadow blocked", "error");"#,
            r#" l.max_rows(4);"#,
            r#" render_to("audit", l);"#,
        ))
        .unwrap();
        let [PanelOp::Upsert { id, spec, .. }] = &out.panel_ops[..] else {
            panic!("expected one upsert, got {:?}", out.panel_ops);
        };
        assert_eq!(id, "audit");
        match spec {
            RenderSpec::LogTail {
                title,
                lines,
                max_rows,
            } => {
                assert_eq!(title, "denials");
                assert_eq!(max_rows, &Some(4));
                assert_eq!(lines.len(), 2);
                assert_eq!(lines[0].level, None);
                assert_eq!(lines[1].level.as_deref(), Some("error"));
            }
            other => panic!("expected a log tail, got {other:?}"),
        }
    }

    #[test]
    fn render_commits_inline_and_no_panel_ops() {
        let out = run(r#"render(text("hi"));"#).unwrap();
        assert!(out.inline.is_some());
        assert!(out.panel_ops.is_empty());
    }

    #[test]
    fn render_to_emits_an_upsert() {
        let out = run(r#"render_to("metrics", text("hi"));"#).unwrap();
        assert!(
            out.inline.is_none(),
            "a panel render does not also go inline"
        );
        assert!(matches!(
            &out.panel_ops[..],
            [PanelOp::Upsert { id, ttl_ms: None, .. }] if id == "metrics"
        ));
    }

    #[test]
    fn inline_and_panels_coexist_in_one_script() {
        // The old single-commit model let a later render_to clobber an earlier inline render.
        let out =
            run(r#"render(text("chat")); render_to("a", text("A")); render_to("b", text("B"));"#)
                .unwrap();
        assert!(out.inline.is_some(), "the inline render survives");
        assert_eq!(out.panel_ops.len(), 2, "both panels are addressed");
    }

    #[test]
    fn render_to_ttl_carries_the_expiry_and_rejects_non_positive() {
        let out = run(r#"render_to_ttl("flash", text("x"), 500);"#).unwrap();
        assert!(matches!(
            &out.panel_ops[..],
            [PanelOp::Upsert {
                ttl_ms: Some(500),
                ..
            }]
        ));
        assert!(run(r#"render_to_ttl("flash", text("x"), 0);"#).is_err());
        assert!(run(r#"render_to_ttl("flash", text("x"), -5);"#).is_err());
    }

    #[test]
    fn ttl_is_clamped_to_a_day() {
        let out = run(r#"render_to_ttl("f", text("x"), 999999999);"#).unwrap();
        assert!(matches!(
            &out.panel_ops[..],
            [PanelOp::Upsert { ttl_ms: Some(ms), .. }] if *ms == 24 * 60 * 60 * 1000
        ));
    }

    #[test]
    fn remove_panel_and_clear_panels_emit_ops() {
        let out = run(r#"remove_panel("old");"#).unwrap();
        assert!(matches!(&out.panel_ops[..], [PanelOp::Remove { id }] if id == "old"));

        let out = run(r#"clear_panels();"#).unwrap();
        assert!(matches!(&out.panel_ops[..], [PanelOp::Clear]));
    }

    #[test]
    fn ops_keep_script_order() {
        let out = run(r#"clear_panels(); render_to("a", text("A")); remove_panel("b");"#).unwrap();
        assert!(matches!(out.panel_ops[0], PanelOp::Clear));
        assert!(matches!(out.panel_ops[1], PanelOp::Upsert { .. }));
        assert!(matches!(out.panel_ops[2], PanelOp::Remove { .. }));
    }

    // --- Fit guard: fail the script when the surface can't display the widget -------------------
    //
    // These publish process-global viewport state, so they share one mutex and reset afterwards.
    use crate::viz::viewport::{self, Viewport};

    /// Run `script` with `vp` published, always restoring the unconstrained default.
    fn run_with_viewport(vp: Viewport, script: &str) -> Result<RenderOutcome, String> {
        let _g = VP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        viewport::set(vp);
        let out = run_inner(script);
        viewport::reset();
        out
    }

    /// A roomy full-screen viewport: 120×40, 70-col chat, 48-col panels, slots to spare.
    fn roomy() -> Viewport {
        Viewport {
            cols: 120,
            rows: 40,
            inline_cols: 70,
            panel_cols: 48,
            panel_rows: 1,
            panel_slots_free: 5,
            live_panels: vec!["existing".into()],
            full_screen: true,
            constrained: true,
        }
    }

    #[test]
    fn headless_never_fails_a_render() {
        // The default (unconstrained) viewport must not reject anything — episodes and batch runs
        // have no screen at all.
        let _g = VP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        viewport::reset();
        assert!(run_inner(r#"render(grid(2, 12));"#).is_ok());
    }

    #[test]
    fn a_terminal_below_the_hard_floor_fails_every_render() {
        let vp = Viewport {
            cols: 30,
            rows: 8,
            ..roomy()
        };
        let err = run_with_viewport(vp, r#"render(text("hi"));"#).unwrap_err();
        assert!(err.contains("too small"), "got: {err}");
        assert!(err.contains("40×10"), "message names the minimum: {err}");
    }

    #[test]
    fn a_widget_wider_than_the_chat_pane_fails_inline() {
        // A 12-column grid needs ≥96 columns; the chat pane is 70.
        let err = run_with_viewport(roomy(), r#"render(grid(1, 12));"#).unwrap_err();
        assert!(err.contains("at least"), "got: {err}");
        assert!(err.contains("70"), "message names the actual width: {err}");
    }

    #[test]
    fn a_widget_wider_than_a_panel_fails_and_suggests_inline() {
        // 8 columns ≥ 64 > the 48-col panel, but still fits the 70-col chat pane.
        let err = run_with_viewport(roomy(), r#"render_to("m", grid(1, 8));"#).unwrap_err();
        assert!(err.contains("panel is only 48"), "got: {err}");
        assert!(
            err.contains("inline"),
            "should point at the roomier surface: {err}"
        );
    }

    #[test]
    fn a_new_panel_fails_when_the_column_is_full_but_an_update_still_works() {
        let full = Viewport {
            panel_slots_free: 0,
            ..roomy()
        };
        let err =
            run_with_viewport(full.clone(), r#"render_to("brand-new", text("x"));"#).unwrap_err();
        assert!(err.contains("panel column is full"), "got: {err}");
        assert!(
            err.contains("remove_panel") && err.contains("clear_panels"),
            "message names the escape hatch: {err}"
        );
        assert!(err.contains("existing"), "and lists reusable ids: {err}");

        // Updating a panel that already exists needs no free slot.
        assert!(run_with_viewport(full, r#"render_to("existing", text("x"));"#).is_ok());
    }

    #[test]
    fn ttl_renders_are_fit_checked_too() {
        let full = Viewport {
            panel_slots_free: 0,
            ..roomy()
        };
        assert!(run_with_viewport(full, r#"render_to_ttl("new", text("x"), 500);"#).is_err());
    }

    #[test]
    fn the_inline_repl_only_constrains_width_not_height() {
        // Height is unbounded there (it scrolls), so a short terminal must not fail a render.
        let inline = Viewport {
            cols: 50,
            rows: 0,
            inline_cols: 50,
            full_screen: false,
            constrained: true,
            ..Viewport::unconstrained()
        };
        assert!(run_with_viewport(inline.clone(), r#"render(text("hi"));"#).is_ok());
        // A panel render falls back inline there, so it's checked against the inline width.
        assert!(run_with_viewport(inline, r#"render_to("m", grid(1, 12));"#).is_err());
    }

    #[test]
    fn render_to_accepts_the_full_legal_charset() {
        assert!(run(r#"render_to("ab_9-z", text("x"));"#).is_ok());
    }

    #[test]
    fn panel_ids_are_validated_on_every_entry_point() {
        // Empty, oversized, and out-of-charset ids all fail closed (contracts/rhai-panel-api.md).
        assert!(run(r#"render_to("", text("x"));"#).is_err());
        let long = "a".repeat(33);
        assert!(run(&format!(r#"render_to("{long}", text("x"));"#)).is_err());
        assert!(
            run(r#"render_to("Metrics", text("x"));"#).is_err(),
            "uppercase rejected"
        );
        assert!(
            run(r#"render_to("a b", text("x"));"#).is_err(),
            "space rejected"
        );
        assert!(
            run(r#"render_to("pan/el", text("x"));"#).is_err(),
            "slash rejected"
        );
        assert!(
            run(r#"remove_panel("Nope");"#).is_err(),
            "remove validates too"
        );
        assert!(
            run(r#"render_to_ttl("Nope", text("x"), 10);"#).is_err(),
            "ttl validates too"
        );
    }
}
