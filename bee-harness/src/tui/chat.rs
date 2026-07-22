//! Chat history model (008-grid-tui, US1 T015).
//!
//! An ordered list of messages; each is either prose (streamed and appended to) or an inline rendered
//! widget. The *view* (T016) virtualizes and draws these — this module is the pure model, so the
//! reducer and its tests need no terminal. Inline widgets will render through
//! `viz::buffer_render::render_into` at draw time.

use crate::render_spec::RenderSpec;

/// Who produced a chat message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// The operator.
    User,
    /// The model's prose.
    Assistant,
    /// Chrome: info / footer / errors / steering acks.
    System,
    /// Tool calls and their results.
    Tool,
}

/// A message's content: prose (appendable while streaming) or a rendered widget.
#[derive(Debug, Clone)]
pub enum Body {
    Text(String),
    Widget(Box<RenderSpec>),
}

/// One entry in the transcript view.
#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub role: Role,
    pub body: Body,
}

impl ChatMessage {
    /// A prose message.
    pub fn text(role: Role, s: impl Into<String>) -> Self {
        ChatMessage {
            role,
            body: Body::Text(s.into()),
        }
    }

    /// An inline widget message (a tool's visualization).
    pub fn widget(spec: RenderSpec) -> Self {
        ChatMessage {
            role: Role::Tool,
            body: Body::Widget(Box::new(spec)),
        }
    }

    /// Append streamed prose. Returns `false` if this message isn't a text body (so the caller starts
    /// a new one).
    pub fn push_str(&mut self, s: &str) -> bool {
        match &mut self.body {
            Body::Text(t) => {
                t.push_str(s);
                true
            }
            Body::Widget(_) => false,
        }
    }

    /// Whether this is an assistant prose message still open for streaming appends.
    pub fn is_open_assistant(&self) -> bool {
        self.role == Role::Assistant && matches!(self.body, Body::Text(_))
    }
}
