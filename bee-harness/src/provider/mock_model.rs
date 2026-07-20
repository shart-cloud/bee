//! [`MockModel`] — the offline, deterministic backend every host test uses (research H11).
//!
//! Constructed from a script (`Vec<Turn>`) or a closure `Fn(&Conversation) -> Turn`. It performs no
//! network I/O, so the loop, tools, truncation, malformed-arg handling, and `no_tool_calls` are all
//! testable without a provider key or a kernel.

use std::sync::Mutex;

use super::{Conversation, Model, ModelError, ToolSchema, Turn};

enum Script {
    /// Successive scripted turns; when exhausted, a graceful text-only turn ends the episode.
    Steps(Mutex<std::collections::VecDeque<Turn>>),
    /// A pure function of the current conversation.
    Closure(Box<dyn Fn(&Conversation) -> Turn + Send + Sync>),
}

/// A scripted [`Model`] for deterministic, offline tests.
pub struct MockModel {
    id: String,
    script: Script,
}

impl MockModel {
    /// Build from a fixed sequence of turns. Each `complete` pops the next; once empty, returns a
    /// text-only turn so the loop terminates as `no_tool_calls` rather than hanging.
    pub fn scripted(turns: Vec<Turn>) -> Self {
        MockModel {
            id: "mock/scripted".to_string(),
            script: Script::Steps(Mutex::new(turns.into())),
        }
    }

    /// Build from a closure that computes each turn from the conversation so far.
    pub fn from_fn<F>(f: F) -> Self
    where
        F: Fn(&Conversation) -> Turn + Send + Sync + 'static,
    {
        MockModel {
            id: "mock/closure".to_string(),
            script: Script::Closure(Box::new(f)),
        }
    }

    /// Override the reported id (which appears in the transcript).
    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = id.into();
        self
    }
}

#[async_trait::async_trait]
impl Model for MockModel {
    fn id(&self) -> &str {
        &self.id
    }

    async fn complete(
        &self,
        convo: &Conversation,
        _tools: &[ToolSchema],
    ) -> Result<Turn, ModelError> {
        match &self.script {
            Script::Steps(q) => Ok(q
                .lock()
                .expect("mock script mutex")
                .pop_front()
                .unwrap_or_else(|| Turn::text("[mock] script exhausted"))),
            Script::Closure(f) => Ok(f(convo)),
        }
    }
}
