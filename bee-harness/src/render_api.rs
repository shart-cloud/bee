//! The Rhai drawing API (003-visual-render, US6, FR-019/FR-021): the builder custom-types and the
//! [`register`] function that wires every drawing function onto a Rhai [`Engine`]. Scripts build
//! opaque builders that accumulate into a [`RenderContext`]; `render(widget)` commits the final
//! [`RenderSpec`]. Only these functions exist on the engine — everything else is denied by omission
//! (contracts/rhai-api.md). Structural caps (nesting ≤ 3, ≤ 500 elements) are enforced at commit and
//! surfaced as script errors (research D6).

use std::sync::{Arc, Mutex};

use rhai::{Array, Dynamic, Engine, EvalAltResult};

use crate::render_spec::{Bar, Direction, Dot, DotState, Point, RenderSpec, Row, Series};

const MAX_NESTING: usize = 3;
const MAX_ELEMENTS: usize = 500;

/// The `Clone + Send`-able accumulator pushed into a render evaluation. Holds the last committed
/// widget and how many `render()` calls the script made (for the discarded-render note).
#[derive(Clone, Default)]
pub struct RenderContext {
    inner: Arc<Mutex<CtxInner>>,
}

#[derive(Default)]
struct CtxInner {
    committed: Option<RenderSpec>,
    render_calls: u32,
}

impl RenderContext {
    /// Clear the context before a fresh evaluation.
    pub fn reset(&self) {
        *self.inner.lock().expect("render ctx") = CtxInner::default();
    }
    fn commit(&self, spec: RenderSpec) {
        let mut g = self.inner.lock().expect("render ctx");
        g.committed = Some(spec);
        g.render_calls += 1;
    }
    /// Take the committed widget and the number of `render()` calls made.
    pub fn take(&self) -> (Option<RenderSpec>, u32) {
        let mut g = self.inner.lock().expect("render ctx");
        (g.committed.take(), g.render_calls)
    }
}

/// What kind of chart a [`ChartBuilder`] is accumulating.
#[derive(Clone, Copy, PartialEq)]
enum ChartKind {
    Bar,
    Line,
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
            ChartKind::Spark => {
                RenderSpec::Sparkline { title: self.title.clone(), data: self.spark.clone() }
            }
        }
    }
}

/// Handle to one line-chart series; `point` mutates the shared point list also held by the chart.
#[derive(Clone)]
pub struct SeriesHandle {
    points: Arc<Mutex<Vec<Point>>>,
}

/// Builder behind `table`.
#[derive(Clone)]
pub struct TableBuilder {
    title: String,
    headers: Vec<String>,
    rows: Vec<Row>,
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
}

impl DotGridBuilder {
    fn to_spec(&self) -> RenderSpec {
        RenderSpec::DotGrid { title: self.title.clone(), dots: self.dots.clone() }
    }
}

/// Builder behind `text`.
#[derive(Clone)]
pub struct TextBuilder {
    content: String,
    style: Option<String>,
    bold: bool,
    dim: bool,
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
}

impl LayoutBuilder {
    fn to_spec(&self) -> RenderSpec {
        RenderSpec::Layout { direction: self.direction, children: self.children.clone() }
    }
}

/// Convert a Rhai `Array` of ints to `Vec<u64>` (negatives clamped to 0).
fn array_to_u64(a: Array) -> Vec<u64> {
    a.into_iter().map(|d| d.as_int().unwrap_or(0).max(0) as u64).collect()
}

/// Convert a Rhai `Array` to `Vec<String>` (each element stringified).
fn array_to_strings(a: Array) -> Vec<String> {
    a.into_iter().map(|d| d.into_string().unwrap_or_default()).collect()
}

/// Turn any builder/`RenderSpec` `Dynamic` into a [`RenderSpec`], or a script error.
fn dynamic_to_spec(d: Dynamic) -> Result<RenderSpec, Box<EvalAltResult>> {
    if d.is::<RenderSpec>() {
        return Ok(d.cast::<RenderSpec>());
    }
    if d.is::<ChartBuilder>() {
        return Ok(d.cast::<ChartBuilder>().to_spec());
    }
    if d.is::<TableBuilder>() {
        return Ok(d.cast::<TableBuilder>().to_spec());
    }
    if d.is::<GaugeBuilder>() {
        return Ok(d.cast::<GaugeBuilder>().to_spec());
    }
    if d.is::<DotGridBuilder>() {
        return Ok(d.cast::<DotGridBuilder>().to_spec());
    }
    if d.is::<TextBuilder>() {
        return Ok(d.cast::<TextBuilder>().to_spec());
    }
    if d.is::<LayoutBuilder>() {
        return Ok(d.cast::<LayoutBuilder>().to_spec());
    }
    Err("render: value is not a renderable widget".into())
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
    engine.register_type_with_name::<TextBuilder>("Text");
    engine.register_type_with_name::<LayoutBuilder>("Layout");
    engine.register_type_with_name::<RenderSpec>("Widget");

    // --- Charts ---
    engine.register_fn("bar_chart", |title: String| ChartBuilder::new(ChartKind::Bar, title));
    engine.register_fn("line_chart", |title: String| ChartBuilder::new(ChartKind::Line, title));
    engine.register_fn("sparkline", |title: String, data: Array| {
        let mut c = ChartBuilder::new(ChartKind::Spark, title);
        c.spark = array_to_u64(data);
        c
    });
    engine.register_fn("bar", |c: &mut ChartBuilder, label: String, value: i64| {
        c.bars.push(Bar { label, value });
    });
    engine.register_fn("x_label", |c: &mut ChartBuilder, s: String| c.x_label = Some(s));
    engine.register_fn("y_label", |c: &mut ChartBuilder, s: String| c.y_label = Some(s));
    engine.register_fn("color", |c: &mut ChartBuilder, name: String| c.color = Some(name));
    engine.register_fn("series", |c: &mut ChartBuilder, label: String| {
        let pts = Arc::new(Mutex::new(Vec::new()));
        c.series.push((label, pts.clone()));
        SeriesHandle { points: pts }
    });
    engine.register_fn("point", |s: &mut SeriesHandle, x: f64, y: f64| {
        s.points.lock().expect("series").push(Point { x, y });
    });

    // --- Tables ---
    engine.register_fn("table", |title: String| TableBuilder {
        title,
        headers: Vec::new(),
        rows: Vec::new(),
    });
    engine.register_fn("header", |t: &mut TableBuilder, cols: Array| {
        t.headers = array_to_strings(cols);
    });
    engine.register_fn("row", |t: &mut TableBuilder, cells: Array| {
        t.rows.push(Row { cells: array_to_strings(cells), color: None });
    });
    engine.register_fn("row_colored", |t: &mut TableBuilder, cells: Array, color: String| {
        t.rows.push(Row { cells: array_to_strings(cells), color: Some(color) });
    });

    // --- Status & indicators ---
    engine.register_fn("gauge", |title: String, value: f64| GaugeBuilder {
        title,
        value: value.clamp(0.0, 1.0),
        label: None,
        color: None,
    });
    engine.register_fn("label", |g: &mut GaugeBuilder, s: String| g.label = Some(s));
    engine.register_fn("color", |g: &mut GaugeBuilder, name: String| g.color = Some(name));
    engine.register_fn("dots", |title: String| DotGridBuilder { title, dots: Vec::new() });
    engine.register_fn("pass", |d: &mut DotGridBuilder, label: String| {
        d.dots.push(Dot { label, state: DotState::Pass });
    });
    engine.register_fn("fail", |d: &mut DotGridBuilder, label: String| {
        d.dots.push(Dot { label, state: DotState::Fail });
    });
    engine.register_fn("skip", |d: &mut DotGridBuilder, label: String| {
        d.dots.push(Dot { label, state: DotState::Skip });
    });

    // --- Decorative / identity ---
    engine.register_fn("text", |content: String| TextBuilder {
        content,
        style: None,
        bold: false,
        dim: false,
    });
    engine.register_fn("style", |t: &mut TextBuilder, name: String| t.style = Some(name));
    engine.register_fn("bold", |t: &mut TextBuilder| t.bold = true);
    engine.register_fn("dim", |t: &mut TextBuilder| t.dim = true);
    engine.register_fn("ascii_art", |lines: Array| -> RenderSpec {
        RenderSpec::AsciiArt { lines: array_to_strings(lines) }
    });
    engine.register_fn("separator", || -> RenderSpec { RenderSpec::Separator });

    // --- Layout & composition ---
    engine.register_fn("vsplit", || LayoutBuilder {
        direction: Direction::Vertical,
        children: Vec::new(),
    });
    engine.register_fn("hsplit", || LayoutBuilder {
        direction: Direction::Horizontal,
        children: Vec::new(),
    });
    engine.register_fn("add", |l: &mut LayoutBuilder, widget: Dynamic| -> Result<(), Box<EvalAltResult>> {
        l.children.push(dynamic_to_spec(widget)?);
        Ok(())
    });

    // --- Commit ---
    engine.register_fn("render", move |widget: Dynamic| -> Result<(), Box<EvalAltResult>> {
        let spec = dynamic_to_spec(widget)?;
        validate(&spec)?;
        ctx.commit(spec);
        Ok(())
    });
}
