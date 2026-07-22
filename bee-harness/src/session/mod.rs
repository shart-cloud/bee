//! The shared conversation engine (008-grid-tui, plan M2 / research D4).
//!
//! **Key realization (T005/T006):** the turn loop [`crate::repl::run_exchange`] already writes
//! *exclusively* through the [`crate::repl::ReplOutput`] trait — so it is already the shared engine,
//! decoupled from any terminal. Both front-ends drive it:
//!
//! * the **inline REPL** passes its `TerminalOutput` (a `ReplOutput`) — unchanged, so its output is
//!   byte-for-byte identical (**T006 parity, by construction**);
//! * the **full-screen TUI** passes a [`SessionSink`], which forwards each callback as a
//!   [`SessionEvent`] on a channel the event loop consumes (**T005**).
//!
//! No rewrite of the working turn loop was needed — the seam already existed.

pub mod event;

pub use event::SessionEvent;

use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

use crate::render_spec::RenderSpec;
use crate::repl::ReplOutput;
use crate::tools::ToolResult;

/// A [`ReplOutput`] that turns the shared turn loop into a stream of [`SessionEvent`]s (008-grid-tui,
/// T005). Drop the sink (or the receiver) to end the stream. `send` never blocks — the queue is
/// unbounded — so it is safe to call from `run_exchange`'s synchronous output callbacks.
pub struct SessionSink {
    tx: UnboundedSender<SessionEvent>,
}

impl SessionSink {
    /// A new sink paired with the receiver the event loop reads.
    pub fn new() -> (Self, UnboundedReceiver<SessionEvent>) {
        let (tx, rx) = unbounded_channel();
        (SessionSink { tx }, rx)
    }

    fn emit(&self, ev: SessionEvent) {
        // A closed receiver just means the front-end went away; dropping the event is correct.
        let _ = self.tx.send(ev);
    }
}

impl ReplOutput for SessionSink {
    fn assistant_delta(&self, chunk: &str) {
        self.emit(SessionEvent::AssistantDelta(chunk.to_string()));
    }
    fn assistant_end(&self) {
        self.emit(SessionEvent::AssistantEnd);
    }
    fn tool_call(&self, name: &str, arguments: &serde_json::Value) {
        self.emit(SessionEvent::ToolCall {
            name: name.to_string(),
            arguments: arguments.clone(),
        });
    }
    fn tool_result(&self, result: &ToolResult, audit: &[bee_core::AuditEvent]) {
        self.emit(SessionEvent::ToolResult {
            result: result.clone(),
            audit: audit.to_vec(),
        });
    }
    fn error(&self, msg: &str) {
        self.emit(SessionEvent::Error(msg.to_string()));
    }
    fn info(&self, msg: &str) {
        self.emit(SessionEvent::Info(msg.to_string()));
    }
    // The following override defaults that would otherwise fold into `info` / no-op / ASCII, so the
    // sink is a *faithful* mirror of the callbacks (not the inline REPL's fallbacks).
    fn footer(&self, msg: &str) {
        self.emit(SessionEvent::Footer(msg.to_string()));
    }
    fn steering(&self, msg: &str) {
        self.emit(SessionEvent::Steering(msg.to_string()));
    }
    fn busy_start(&self) {
        self.emit(SessionEvent::TurnStarted);
    }
    fn busy_stop(&self) {
        self.emit(SessionEvent::TurnDone);
    }
    fn render_widget(&self, spec: &RenderSpec) {
        self.emit(SessionEvent::RenderWidget { spec: spec.clone() });
    }
    fn panel_update(&self, id: &str, spec: &RenderSpec) {
        self.emit(SessionEvent::PanelUpdate {
            id: id.to_string(),
            spec: spec.clone(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sink_forwards_callbacks_as_events_in_order() {
        // T005: each ReplOutput callback becomes the matching SessionEvent, faithfully and in order —
        // including the ones with trait defaults (footer/steering/busy_*/render_widget) that the sink
        // overrides so it mirrors the call, not the inline fallback.
        let (sink, mut rx) = SessionSink::new();
        sink.busy_start();
        sink.assistant_delta("hel");
        sink.assistant_delta("lo");
        sink.assistant_end();
        sink.footer("1 turn");
        sink.busy_stop();
        drop(sink);

        let got: Vec<SessionEvent> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        assert!(matches!(got[0], SessionEvent::TurnStarted));
        assert!(matches!(&got[1], SessionEvent::AssistantDelta(s) if s == "hel"));
        assert!(matches!(&got[2], SessionEvent::AssistantDelta(s) if s == "lo"));
        assert!(matches!(got[3], SessionEvent::AssistantEnd));
        assert!(matches!(&got[4], SessionEvent::Footer(s) if s == "1 turn"));
        assert!(matches!(got[5], SessionEvent::TurnDone));
        assert_eq!(got.len(), 6);
    }
}
