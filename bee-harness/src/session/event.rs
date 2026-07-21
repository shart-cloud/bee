//! [`SessionEvent`] (008-grid-tui, T004 / data-model.md): what the shared `SessionEngine` emits as a
//! turn progresses. Renderer-agnostic — it holds no ratatui/crossterm types — so the inline REPL and
//! the full-screen TUI can each render it their own way.

use crate::render_spec::{RenderSpec, RenderTarget};

/// One thing the conversation core has to say. Front-ends fold these into their view.
#[derive(Debug, Clone, PartialEq)]
pub enum SessionEvent {
    /// The user's submitted line, echoed for display.
    UserEcho(String),
    /// A streamed assistant-token delta to append to the currently-open assistant message.
    Token(String),
    /// A tool call is starting (name + a short arg summary).
    ToolCall { name: String, args: String },
    /// A tool produced a result, possibly carrying a rendered widget and where it should go.
    ToolResult {
        render_spec: Option<RenderSpec>,
        target: RenderTarget,
    },
    /// A model-owned panel was (re)rendered — upsert by `id`, replacing in place (FR-009).
    PanelUpdate { id: String, spec: RenderSpec },
    /// A kernel/policy denial to surface (rendered as a bold `⚠ DENIED` line).
    Denial(String),
    /// The assistant turn began (drives the spinner / "thinking" state).
    TurnStarted,
    /// The assistant turn finished.
    TurnDone,
}
