//! [`SessionEvent`] (008-grid-tui, T004 / data-model.md): what the shared session core says as a turn
//! progresses.
//!
//! The turn loop ([`crate::repl::run_exchange`]) already writes everything through the
//! [`crate::repl::ReplOutput`] trait — that trait *is* the engine's output seam. `SessionEvent` is a
//! faithful 1:1 mirror of those callbacks, so a front-end (the TUI) can reconstruct exactly what the
//! inline REPL renders. Renderer-agnostic — no ratatui/rustyline types leak in.

use bee_core::AuditEvent;
use serde_json::Value;

use crate::render_spec::RenderSpec;
use crate::tools::ToolResult;

/// One thing the session core emitted, mirroring one [`crate::repl::ReplOutput`] callback.
#[derive(Debug, Clone)]
pub enum SessionEvent {
    /// A streamed assistant-token delta (`assistant_delta`).
    AssistantDelta(String),
    /// End of an assistant prose block (`assistant_end`).
    AssistantEnd,
    /// A tool call the agent requested, before it runs (`tool_call`).
    ToolCall { name: String, arguments: Value },
    /// A tool result plus its kernel audit events (`tool_result`).
    ToolResult {
        result: ToolResult,
        audit: Vec<AuditEvent>,
    },
    /// A visualization to draw (`render_widget`). The TUI routes it inline or to a panel by target
    /// once `render_to` threads a [`crate::render_spec::RenderTarget`] through (tasks T024/T025).
    RenderWidget { spec: RenderSpec },
    /// An error line the user should see (`error`).
    Error(String),
    /// An informational line — banners, meta-command output (`info`).
    Info(String),
    /// A dim per-exchange summary footer (`footer`).
    Footer(String),
    /// A steering acknowledgement (`steering`).
    Steering(String),
    /// The "working" indicator began (`busy_start`) — the assistant turn is in flight.
    TurnStarted,
    /// The "working" indicator ended (`busy_stop`).
    TurnDone,
}
