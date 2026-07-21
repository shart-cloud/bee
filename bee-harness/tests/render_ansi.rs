//! The headless `RenderSpec` → ANSI-lines pipeline (003-visual-render, US6, FR-025): a bar chart
//! renders to inline ANSI with box-drawing + color and no terminal takeover (SC-011); a composed
//! vsplit's height is the sum of its children plus separators (SC-014).

use bee_harness::render_spec::{Bar, Direction, RenderSpec, Row};
use bee_harness::viz::{render_to_ansi, spec_height};

fn bar_chart() -> RenderSpec {
    RenderSpec::BarChart {
        title: "Sizes".into(),
        bars: vec![
            Bar {
                label: "a".into(),
                value: 10,
            },
            Bar {
                label: "b".into(),
                value: 25,
            },
            Bar {
                label: "c".into(),
                value: 40,
            },
        ],
        x_label: None,
        y_label: None,
        color: Some("honey".into()),
    }
}

#[test]
fn bar_chart_renders_inline_ansi_box_color_and_no_color() {
    // Serial (NO_COLOR is process-global): color-on then color-off in one test to avoid racing a
    // parallel test in this binary.
    std::env::remove_var("NO_COLOR");
    let lines = render_to_ansi(&bar_chart(), 60, 40);
    assert!(!lines.is_empty());
    let joined = lines.join("\n");
    // Box-drawing from the bordered Block (SC-011).
    assert!(
        joined.contains('\u{2500}') || joined.contains('\u{2502}') || joined.contains('\u{250C}'),
        "expected box-drawing characters: {joined:?}"
    );
    // Colored content — an SGR sequence is present (bars/title styled).
    assert!(joined.contains('\u{1b}'), "expected ANSI color: {joined:?}");
    // Headless: nothing entered an alternate screen / raw mode (no such escape emitted).
    assert!(
        !joined.contains("\x1b[?1049"),
        "must not switch to alt-screen"
    );
    assert!(!joined.contains("\x1b[?25l"), "must not hide the cursor");

    // AS-3: color off drops all escapes, keeps glyphs.
    std::env::set_var("NO_COLOR", "1");
    let plain = render_to_ansi(&bar_chart(), 60, 40).join("\n");
    std::env::remove_var("NO_COLOR");
    assert!(
        !plain.contains('\u{1b}'),
        "no SGR under NO_COLOR: {plain:?}"
    );
}

#[test]
fn vsplit_height_is_sum_of_children_plus_separators() {
    let gauge = RenderSpec::Gauge {
        title: "progress".into(),
        value: 0.5,
        label: None,
        color: None,
    };
    let table = RenderSpec::Table {
        title: "results".into(),
        headers: vec!["name".into(), "status".into()],
        rows: vec![
            Row {
                cells: vec!["a".into(), "ok".into()],
                color: None,
            },
            Row {
                cells: vec!["b".into(), "ok".into()],
                color: None,
            },
        ],
    };
    let layout = RenderSpec::Layout {
        direction: Direction::Vertical,
        children: vec![gauge.clone(), table.clone()],
    };

    let width = 60;
    let gh = spec_height(&gauge, width);
    let th = spec_height(&table, width);
    // Two children ⇒ one separator row between them (SC-014).
    assert_eq!(spec_height(&layout, width), gh + 1 + th);

    let lines = render_to_ansi(&layout, width, 40);
    assert_eq!(
        lines.len() as u16,
        gh + 1 + th,
        "rendered height matches spec_height"
    );
}
