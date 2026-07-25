//! Public-API checks for the US7 status grid (003-visual-render, SC-013/AS-3) and the `render_widget`
//! default ASCII fallback (analysis finding C1 / US6 AS-5).

use bee::render_spec::{Bar, RenderSpec};
use bee::repl::ReplOutput;
use bee::viz::{status_grid, Status};

#[test]
fn status_grid_colors_then_no_color() {
    // Serial (NO_COLOR is process-global). SC-013: color on.
    std::env::remove_var("NO_COLOR");
    let rows = vec![
        (
            "read-denied".to_string(),
            vec![Status::Pass, Status::Pass, Status::Pass, Status::Fail],
        ),
        (
            "write-allowed".to_string(),
            vec![Status::Pass, Status::Pass],
        ),
    ];
    let colored = status_grid(&rows);
    assert_eq!(
        colored.matches("\x1b[32m").count(),
        5,
        "5 green dots: {colored:?}"
    );
    assert_eq!(
        colored.matches("\x1b[31m").count(),
        1,
        "1 red dot: {colored:?}"
    );
    assert!(colored.contains("3/4 pass") && colored.contains("2/2 pass"));

    // AS-3: color off.
    std::env::set_var("NO_COLOR", "1");
    let plain = status_grid(&rows);
    std::env::remove_var("NO_COLOR");
    assert!(
        !plain.contains('\u{1b}'),
        "no SGR under NO_COLOR: {plain:?}"
    );
    assert!(plain.contains('\u{25CF}'), "dot glyph still present");
}

/// A [`ReplOutput`] that captures `info` lines — used to prove the DEFAULT `render_widget` emits a
/// non-blank ASCII fallback (does not override `render_widget`).
#[derive(Default)]
struct InfoCapture {
    lines: std::sync::Mutex<Vec<String>>,
}

impl ReplOutput for InfoCapture {
    fn assistant_delta(&self, _chunk: &str) {}
    fn assistant_end(&self) {}
    fn tool_call(&self, _name: &str, _arguments: &serde_json::Value) {}
    fn tool_result(&self, _result: &bee::ToolResult, _audit: &[bee_core::AuditEvent]) {}
    fn error(&self, _msg: &str) {}
    fn info(&self, msg: &str) {
        self.lines.lock().unwrap().push(msg.to_string());
    }
    // NOTE: deliberately does NOT override render_widget — exercises the default fallback (AS-5/C1).
}

#[test]
fn default_render_widget_emits_non_blank_ascii() {
    let spec = RenderSpec::BarChart {
        title: "Sizes".into(),
        bars: vec![
            Bar {
                label: "a".into(),
                value: 3,
            },
            Bar {
                label: "b".into(),
                value: 7,
            },
        ],
        x_label: None,
        y_label: None,
        color: None,
    };
    let out = InfoCapture::default();
    out.render_widget(&spec, None);
    let lines = out.lines.lock().unwrap();
    assert!(!lines.is_empty(), "fallback must emit lines, not a blank");
    let joined = lines.join("\n");
    assert!(joined.contains("Sizes") && joined.contains('a') && joined.contains('b'));
    assert!(
        !joined.contains('\u{1b}'),
        "fallback is plain text (no ANSI)"
    );
}
