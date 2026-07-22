//! The `render` tool (003-visual-render, US6, FR-019…FR-024): evaluates a Rhai script against the
//! registered drawing API and returns a [`ToolResult`] whose `content` is a **text summary** and
//! whose `render_spec` carries the widget for the REPL to draw. It runs Rhai **in-process** and
//! **ignores the `Sandbox`** (FR-022) — the tool does no I/O, so there is nothing for the kernel to
//! enforce; Rhai's own sandbox (bounded ops/memory, drawing-only API) is its confinement.

use std::sync::Mutex;

use rhai::{Engine, EvalAltResult};
use serde_json::json;

use crate::config::VisualLevel;
use crate::provider::ToolSchema;
use crate::render_api::{register, RenderContext};
use crate::sandbox::Sandbox;
use crate::tools::{Tool, ToolResult};
use crate::visual_gate;

/// Max Rhai operations before the engine terminates a script (FR-020). ~50ms worst case (NFR-001).
const MAX_OPERATIONS: u64 = 10_000;

/// The Rhai primer + function list embedded in the tool's schema so the model has the API surface in
/// its tool definition (spec Assumption).
const SCHEMA_DESC: &str = "\
Render a visualization by writing a short Rhai script that ends with render(widget). Rhai is \
JavaScript-adjacent: `let x = 5;`, `for i in 0..n {}`, `[1,2,3]` arrays, method calls like \
`chart.bar(\"label\", 42)`. Integers are i64, floats f64. No I/O, filesystem, or network is \
available — only these drawing functions:\n\
  bar_chart(title) -> chart;  chart.bar(label, value);  chart.color(name);  chart.x_label(s); chart.y_label(s)\n\
  line_chart(title) -> chart;  let s = chart.series(label);  s.point(x, y)\n\
  sparkline(title, [ints]) -> chart\n\
  table(title) -> t;  t.header([cols]);  t.row([cells]);  t.row_colored([cells], color)\n\
  gauge(title, value_0_to_1) -> g;  g.label(s);  g.color(name)\n\
  dots(title) -> d;  d.pass(label);  d.fail(label);  d.skip(label)\n\
  text(content) -> t;  t.style(name);  t.bold();  t.dim()\n\
  ascii_art([lines]);  separator()\n\
  vsplit() / hsplit() -> layout;  layout.add(widget)   (max nesting depth 3)\n\
  palette() -> p;  p.set(\"K\", \"#1A1A1A\");  p.set(\".\", \"transparent\")\n\
  sprite(w, h, p) -> s;  s.paint([\"..KK..\", ...]);  s.set(x, y, color);  s.fill(color)   (max 32x32)\n\
  animation(ms) -> a;  a.add(sprite);  a.bounce(true);  a.cycles(n)   (max 16 frames, 50-1000ms)\n\
  bee_sprite();  bee_animation()   // the project mascot\n\
  render(widget)               // draw inline in the chat flow\n\
  render_to(panel_id, widget)  // draw to a named, persistent side panel (id: 1-32 of [a-z0-9_-]);\n\
                               // re-rendering the same id replaces that panel in place\n\
  render_to_ttl(panel_id, widget, ttl_ms)  // same, but the panel auto-expires after ttl_ms\n\
  remove_panel(panel_id)       // close one panel and reclaim its space\n\
  clear_panels()               // close every panel\n\
Panel hygiene: panels persist until removed and share one column, so each extra panel shrinks the \
rest. Reuse one id for updates, give short-lived output a TTL, and remove_panel/clear_panels when \
done. You may address several panels in a single script.\n\
Colors: honey, pollen, sting, smoke, royal (or basic ANSI names). Caps: <=500 total elements. \
The model receives a text summary of what was drawn, not the pixels.";

/// The text summary the model gets back: what was drawn and where it went. Never pixels (FR-023).
fn summarize(outcome: &crate::render_api::RenderOutcome) -> String {
    use crate::render_spec::{PanelOp, RenderSpec};

    let mut parts: Vec<String> = Vec::new();
    if let Some(spec) = &outcome.inline {
        let mut s = format!("Rendered {} inline", spec.summary_noun());
        // A fully-transparent sprite renders as blank rows — say so rather than look broken.
        if let RenderSpec::Sprite { spec: sp } = spec {
            if sp.is_fully_transparent() {
                s.push_str(" (note: sprite is fully transparent)");
            }
        }
        parts.push(s);
    }
    for op in &outcome.panel_ops {
        parts.push(match op {
            PanelOp::Upsert {
                id,
                spec,
                ttl_ms: None,
                ..
            } => format!("rendered {} to panel {id:?}", spec.summary_noun()),
            PanelOp::Upsert {
                id,
                spec,
                ttl_ms: Some(ms),
                ..
            } => format!(
                "rendered {} to panel {id:?} (expires in {ms}ms)",
                spec.summary_noun()
            ),
            PanelOp::Remove { id } => format!("removed panel {id:?}"),
            PanelOp::Clear => "cleared all panels".to_string(),
        });
    }

    let mut summary = parts.join("; ");
    summary.push('.');
    // Capitalize when the first clause came from a panel op rather than the inline branch.
    if let Some(first) = summary.get_mut(0..1) {
        first.make_ascii_uppercase();
    }
    if outcome.render_calls > 1 && outcome.inline.is_some() && outcome.panel_ops.is_empty() {
        summary.push_str(&format!(
            " (note: {} earlier render(s) discarded; showing the last)",
            outcome.render_calls - 1
        ));
    }
    summary
}

/// Apply the visual permission gate to everything a script produced (009 US2, T029/T032).
///
/// Runs **before any panel state mutates** — the outcome here is what becomes session events, so a
/// downgrade is not a correction applied later, it is the only decision anything downstream sees.
/// Returns the gated outcome plus the notes to append to the model's summary (FR-009, FR-010).
///
/// At `visual_level = none` the agent owns no regions at all, so panel upserts route inline. Inline
/// holds one widget, so the last upsert wins — the same last-writer-wins rule `render()` has always
/// had. Removes and clears simply drop: there are no panels for them to act on.
fn apply_visual_gate(
    mut outcome: crate::render_api::RenderOutcome,
    level: VisualLevel,
) -> (crate::render_api::RenderOutcome, Vec<&'static str>) {
    use crate::render_spec::{PanelOp, RenderTarget};

    if level != VisualLevel::None || outcome.panel_ops.is_empty() {
        return (outcome, Vec::new());
    }

    let mut notes = Vec::new();
    for op in std::mem::take(&mut outcome.panel_ops) {
        match op {
            PanelOp::Upsert { id, spec, .. } => {
                let gated = visual_gate::gate(level, RenderTarget::Panel { id });
                if let Some(note) = gated.note {
                    if !notes.contains(&note) {
                        notes.push(note);
                    }
                }
                outcome.inline = Some(spec);
            }
            // Nothing was ever created, so nothing is left to remove or clear. Silent: the script
            // asked to tidy up a column it never had, which is not a problem worth a note.
            PanelOp::Remove { .. } | PanelOp::Clear => {}
        }
    }
    (outcome, notes)
}

/// The render tool. The Rhai [`Engine`] is built **once** (per registry/session) with hard resource
/// limits (research D2); per-call cost is just `run`. `eval_guard` serializes concurrent calls so the
/// shared [`RenderContext`] is never raced.
pub struct RenderTool {
    engine: Engine,
    ctx: RenderContext,
    eval_guard: Mutex<()>,
}

impl Default for RenderTool {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderTool {
    /// Build the sandboxed rendering engine: hard limits (FR-020), `eval` disabled, and only the
    /// drawing API registered (FR-021).
    pub fn new() -> Self {
        let ctx = RenderContext::default();
        let mut engine = Engine::new();
        engine.set_max_operations(MAX_OPERATIONS);
        engine.set_max_call_levels(16);
        engine.set_max_expr_depths(32, 16);
        engine.set_max_string_size(64 * 1024);
        engine.set_max_array_size(1_000);
        engine.set_max_map_size(100);
        engine.disable_symbol("eval");
        register(&mut engine, ctx.clone());
        RenderTool {
            engine,
            ctx,
            eval_guard: Mutex::new(()),
        }
    }
}

#[async_trait::async_trait]
impl Tool for RenderTool {
    fn name(&self) -> &'static str {
        "render"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "render".to_string(),
            description: SCHEMA_DESC.to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "script": { "type": "string", "description": "A Rhai script that builds one visualization and ends with render(widget)." }
                },
                "required": ["script"]
            }),
        }
    }

    async fn call(&self, arguments: serde_json::Value, _sandbox: &Sandbox) -> ToolResult {
        // Serialize concurrent calls: the shared RenderContext is reset+read across the eval.
        let _guard = self.eval_guard.lock().expect("render eval guard");

        let script = match arguments.get("script").and_then(|v| v.as_str()) {
            Some(s) => s.to_string(),
            None => return ToolResult::invalid_args("render", "missing string field 'script'"),
        };

        self.ctx.reset();
        match self.engine.run(&script) {
            Ok(()) => {
                let outcome = self.ctx.take();
                if outcome.is_empty() {
                    return ToolResult::error("render: script produced no visualization");
                }
                // The gate runs here, on the way out of the script and before any of this becomes a
                // session event — so nothing downstream ever sees a target above the ceiling.
                let (outcome, notes) = apply_visual_gate(outcome, visual_gate::active().level);
                if outcome.is_empty() {
                    return ToolResult::error("render: script produced no visualization");
                }
                let mut summary = summarize(&outcome);
                for note in &notes {
                    summary.push_str(&format!(" ({note})"));
                }
                ToolResult::rendered_with_ops(summary, outcome.inline, outcome.panel_ops)
            }
            Err(e) => {
                let msg = match &*e {
                    EvalAltResult::ErrorTooManyOperations(_) => format!(
                        "render: script exceeded the operation limit ({MAX_OPERATIONS} ops)"
                    ),
                    other => format!("render: {other}"),
                };
                ToolResult::error(msg)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render_api::RenderOutcome;
    use crate::render_spec::{PanelOp, RenderSpec};

    fn text(s: &str) -> RenderSpec {
        RenderSpec::Text {
            content: s.into(),
            style: None,
            bold: false,
            dim: false,
        }
    }

    fn upsert(id: &str, body: &str) -> PanelOp {
        PanelOp::Upsert {
            id: id.into(),
            spec: text(body),
            ttl_ms: None,
            effect: None,
        }
    }

    /// The gate is a pure function of the level, so every row is exercised without touching the
    /// process-global config that the live tool reads.
    fn gated(level: VisualLevel, ops: Vec<PanelOp>) -> (RenderOutcome, Vec<&'static str>) {
        apply_visual_gate(
            RenderOutcome {
                inline: None,
                panel_ops: ops,
                render_calls: 1,
            },
            level,
        )
    }

    #[test]
    fn panels_survive_at_every_level_that_allows_them() {
        for level in [
            VisualLevel::Panels,
            VisualLevel::PanelsWide,
            VisualLevel::Takeover,
        ] {
            let (out, notes) = gated(level, vec![upsert("metrics", "v1")]);
            assert_eq!(out.panel_ops.len(), 1, "the panel op survives at {level:?}");
            assert!(out.inline.is_none(), "and does not also render inline");
            assert!(notes.is_empty(), "a permitted render says nothing");
        }
    }

    #[test]
    fn level_none_routes_a_panel_inline_and_tells_the_model_why() {
        // FR-009/FR-010: the model gets a working visual plus the note, so it can adapt next turn.
        let (out, notes) = gated(VisualLevel::None, vec![upsert("metrics", "v1")]);
        assert!(out.panel_ops.is_empty(), "no panel state may be mutated");
        assert_eq!(out.inline, Some(text("v1")), "the content still renders");
        assert_eq!(notes, vec![crate::visual_gate::NOTE_TO_INLINE]);
    }

    #[test]
    fn the_note_reaches_the_models_summary_verbatim() {
        // SC-004: what the model reads is the contract's exact string.
        let (out, notes) = gated(VisualLevel::None, vec![upsert("metrics", "v1")]);
        let mut summary = summarize(&out);
        for note in &notes {
            summary.push_str(&format!(" ({note})"));
        }
        assert!(
            summary.contains("downgraded to inline: visual level is none"),
            "got: {summary}"
        );
        assert!(summary.starts_with("Rendered"), "got: {summary}");
    }

    #[test]
    fn several_downgraded_panels_collapse_to_one_widget_and_one_note() {
        // Inline holds a single widget, so the last write wins — the rule `render()` has always
        // had. Repeating the same note once per panel would just pad the model's context.
        let (out, notes) = gated(
            VisualLevel::None,
            vec![upsert("a", "first"), upsert("b", "second")],
        );
        assert_eq!(out.inline, Some(text("second")));
        assert_eq!(notes.len(), 1);
    }

    #[test]
    fn removes_and_clears_drop_silently_at_level_none() {
        // There are no panels to act on, so tidying up a column that never existed is not news.
        let (out, notes) = gated(
            VisualLevel::None,
            vec![PanelOp::Remove { id: "a".into() }, PanelOp::Clear],
        );
        assert!(out.panel_ops.is_empty());
        assert!(out.inline.is_none());
        assert!(notes.is_empty());
    }

    #[test]
    fn an_inline_render_is_untouched_at_every_level() {
        for level in [
            VisualLevel::None,
            VisualLevel::Panels,
            VisualLevel::PanelsWide,
            VisualLevel::Takeover,
        ] {
            let (out, notes) = apply_visual_gate(
                RenderOutcome {
                    inline: Some(text("hi")),
                    panel_ops: Vec::new(),
                    render_calls: 1,
                },
                level,
            );
            assert_eq!(
                out.inline,
                Some(text("hi")),
                "inline is the floor at {level:?}"
            );
            assert!(notes.is_empty());
        }
    }
}
