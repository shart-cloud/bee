//! Preview a model-style N×M grid through the real render pipeline.
//! `cargo run --example grid_preview`.
use bee::render_spec::{GridCell, RenderSpec, Row};
use bee::viz::render_to_ansi;

fn gauge(title: &str, value: f64) -> RenderSpec {
    RenderSpec::Gauge {
        title: title.into(),
        value,
        label: None,
        color: Some("honey".into()),
    }
}

fn cell(row: u16, col: u16, content: RenderSpec) -> GridCell {
    GridCell {
        row,
        col,
        row_span: 1,
        col_span: 1,
        title: None,
        content,
    }
}

fn main() {
    let table = RenderSpec::Table {
        title: "recent".into(),
        headers: vec!["host".into(), "status".into()],
        rows: vec![
            Row {
                cells: vec!["web-1".into(), "ok".into()],
                color: Some("pollen".into()),
            },
            Row {
                cells: vec!["web-2".into(), "down".into()],
                color: Some("sting".into()),
            },
        ],
    };

    // 2×3: three gauges across the top, a full-width table spanning the bottom row.
    let grid = RenderSpec::Grid {
        rows: 2,
        cols: 3,
        col_weights: Vec::new(),
        row_weights: Vec::new(),
        gap: Some(0),
        cells: vec![
            cell(0, 0, gauge("CPU", 0.82)),
            cell(0, 1, gauge("MEM", 0.51)),
            cell(0, 2, gauge("NET", 0.13)),
            GridCell {
                row: 1,
                col: 0,
                row_span: 1,
                col_span: 3,
                title: None,
                content: table,
            },
        ],
    };

    for line in render_to_ansi(&grid, 72, 40) {
        println!("{line}");
    }
}
