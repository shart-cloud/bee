//! The `render` tool (003-visual-render, US6, FR-019…FR-024): evaluates a Rhai script against the
//! registered drawing API and returns a [`ToolResult`] whose `content` is a **text summary** and
//! whose `render_spec` carries the widget for the REPL to draw. It runs Rhai **in-process** and
//! **ignores the `Sandbox`** (FR-022) — the tool does no I/O, so there is nothing for the kernel to
//! enforce; Rhai's own sandbox (bounded ops/memory, drawing-only API) is its confinement.

use std::sync::Mutex;

use rhai::{Engine, EvalAltResult};
use serde_json::json;

use crate::provider::ToolSchema;
use crate::render_api::{register, RenderContext};
use crate::sandbox::Sandbox;
use crate::tools::{Tool, ToolResult};

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
  render(widget)               // commit one widget inline in the chat flow\n\
  render_to(panel_id, widget)  // commit to a named, persistent side panel (id: 1-32 of [a-z0-9_-]);\n\
                               // re-rendering the same id replaces that panel in place\n\
Colors: honey, pollen, sting, smoke, royal (or basic ANSI names). Caps: <=500 total elements. \
The model receives a text summary of what was drawn, not the pixels.";

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
                let (committed, render_calls) = self.ctx.take();
                match committed {
                    None => ToolResult::error("render: script produced no visualization"),
                    Some((spec, target)) => {
                        let where_to = match &target {
                            crate::render_spec::RenderTarget::Inline => String::new(),
                            crate::render_spec::RenderTarget::Panel { id } => {
                                format!(" to panel {id:?}")
                            }
                        };
                        let mut summary = format!("Rendered {}{where_to}.", spec.summary_noun());
                        // M1: a fully-transparent sprite renders as blank rows — say so.
                        if let crate::render_spec::RenderSpec::Sprite { spec: s } = &spec {
                            if s.is_fully_transparent() {
                                summary.push_str(" (note: sprite is fully transparent)");
                            }
                        }
                        if render_calls > 1 {
                            summary.push_str(&format!(
                                " (note: {} earlier render(s) discarded; showing the last)",
                                render_calls - 1
                            ));
                        }
                        ToolResult::rendered_to(summary, spec, target)
                    }
                }
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
