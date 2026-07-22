//! `App` (Model) + the `update(&mut App, Message)` reducer (008-grid-tui, US1 T011/T013).
//!
//! Pure, renderer-agnostic state: feed a [`Message`] in, get mutated state out — no terminal, so the
//! whole thing is unit-testable (the tests at the bottom cover the acceptance behaviors). Side effects
//! (sending a submitted line to the model) are surfaced as data via [`App::take_outbox`], not done
//! here. Panels (US2) and the live-panel routing land in later tasks.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::chat::{ChatMessage, Role};
use super::input::InputState;
use super::message::Message;
use super::panels::PanelRegistry;
use crate::session::SessionEvent;

/// Which region has keyboard focus. (Panels focus arrives with US2.)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Input,
    Chat,
}

/// Whether an assistant turn is in flight (drives the spinner / "thinking" line).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnState {
    Idle,
    Streaming,
}

/// The responsive layout tier, derived purely from the terminal size (research D11).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutMode {
    /// ≥ 120 cols: chat + panel column.
    TwoPane,
    /// 80–119 (or short): a single chat pane; panels via overlay.
    SinglePane,
    /// Below the hard floor: only a "terminal too small" message.
    TooSmall,
}

impl LayoutMode {
    /// The tier for a `(cols, rows)` size. Hard floor 40×10 (contracts/modes-and-cli.md).
    pub fn from_size(cols: u16, rows: u16) -> Self {
        if cols < 40 || rows < 10 {
            LayoutMode::TooSmall
        } else if cols >= 120 {
            LayoutMode::TwoPane
        } else {
            LayoutMode::SinglePane
        }
    }
}

/// The full-screen application model.
pub struct App {
    pub chat: Vec<ChatMessage>,
    /// Model-owned panels rendered beside chat (US2). Empty in a US1 session.
    pub panels: PanelRegistry,
    pub input: InputState,
    pub focus: Focus,
    pub turn: TurnState,
    pub layout_mode: LayoutMode,
    pub size: (u16, u16),
    /// Lines scrolled up from the bottom; `0` = pinned to the newest content.
    pub scroll: usize,
    /// Whether new content keeps the view pinned to the bottom.
    pub follow_tail: bool,
    pub help_open: bool,
    pub should_quit: bool,
    /// One-key `gg` state: a `g` was pressed and we're waiting for the second.
    pub pending_g: bool,
    /// A submitted line the event loop must send to the model (drained via [`App::take_outbox`]).
    pub outbox: Option<String>,
}

impl App {
    /// A fresh session sized to the terminal.
    pub fn new(cols: u16, rows: u16) -> Self {
        App {
            chat: Vec::new(),
            panels: PanelRegistry::new(),
            input: InputState::default(),
            focus: Focus::Input,
            turn: TurnState::Idle,
            layout_mode: LayoutMode::from_size(cols, rows),
            size: (cols, rows),
            scroll: 0,
            follow_tail: true,
            help_open: false,
            should_quit: false,
            pending_g: false,
            outbox: None,
        }
    }

    /// Take the pending outgoing user message, if any (the loop sends it to the model).
    pub fn take_outbox(&mut self) -> Option<String> {
        self.outbox.take()
    }

    fn cycle_focus(&mut self) {
        self.focus = match self.focus {
            Focus::Input => Focus::Chat,
            Focus::Chat => Focus::Input,
        };
    }

    fn push_line(&mut self, role: Role, s: impl Into<String>) {
        self.chat.push(ChatMessage::text(role, s));
        self.autoscroll();
    }

    fn autoscroll(&mut self) {
        if self.follow_tail {
            self.scroll = 0;
        }
    }

    fn scroll_up(&mut self, n: usize) {
        self.scroll = self.scroll.saturating_add(n);
        self.follow_tail = false;
    }

    fn scroll_down(&mut self, n: usize) {
        self.scroll = self.scroll.saturating_sub(n);
        if self.scroll == 0 {
            self.follow_tail = true;
        }
    }

    fn scroll_to_top(&mut self) {
        self.scroll = usize::MAX; // the view clamps to the real maximum
        self.follow_tail = false;
    }

    fn scroll_to_bottom(&mut self) {
        self.scroll = 0;
        self.follow_tail = true;
    }

    fn submit(&mut self) {
        if let Some(text) = self.input.take() {
            self.chat.push(ChatMessage::text(Role::User, text.clone()));
            self.outbox = Some(text);
            self.scroll_to_bottom();
        }
    }
}

/// The reducer: apply one [`Message`] to the model. Pure and total.
pub fn update(app: &mut App, msg: Message) {
    match msg {
        Message::Quit => app.should_quit = true,
        Message::Resize(cols, rows) => {
            app.size = (cols, rows);
            app.layout_mode = LayoutMode::from_size(cols, rows);
        }
        Message::Paste(s) => app.input.insert_str(&s),
        Message::Tick | Message::Suspend | Message::Resume => {}
        Message::Key(key) => handle_key(app, key),
        Message::Session(ev) => handle_session(app, *ev),
    }
}

fn handle_key(app: &mut App, key: KeyEvent) {
    // The help overlay swallows input; `?` or Esc closes it.
    if app.help_open {
        if matches!(key.code, KeyCode::Char('?') | KeyCode::Esc) {
            app.help_open = false;
        }
        return;
    }
    // Ctrl-C is a clean quit in any focus (the terminal's SIGINT meaning — reserved, FR-017).
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        app.should_quit = true;
        return;
    }
    if key.code == KeyCode::Tab {
        app.cycle_focus();
        return;
    }
    match app.focus {
        Focus::Input => handle_input_key(app, key),
        Focus::Chat => handle_chat_key(app, key),
    }
}

fn handle_input_key(app: &mut App, key: KeyEvent) {
    let soft_newline =
        key.modifiers.contains(KeyModifiers::SHIFT) || key.modifiers.contains(KeyModifiers::ALT);
    match key.code {
        KeyCode::Enter if soft_newline => app.input.newline(),
        KeyCode::Enter => app.submit(),
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.input.insert_char(c)
        }
        KeyCode::Backspace => app.input.backspace(),
        KeyCode::Delete => app.input.delete(),
        KeyCode::Left => app.input.left(),
        KeyCode::Right => app.input.right(),
        KeyCode::Home => app.input.home(),
        KeyCode::End => app.input.end(),
        KeyCode::Up => app.input.history_prev(),
        KeyCode::Down => app.input.history_next(),
        KeyCode::Esc => app.input.clear(),
        _ => {}
    }
}

fn handle_chat_key(app: &mut App, key: KeyEvent) {
    let was_g_pending = app.pending_g;
    app.pending_g = false;
    match key.code {
        KeyCode::Char('q') => app.should_quit = true,
        KeyCode::Char('?') => app.help_open = true,
        KeyCode::Char('i') | KeyCode::Enter => app.focus = Focus::Input,
        KeyCode::Char('g') => {
            if was_g_pending {
                app.scroll_to_top();
            } else {
                app.pending_g = true;
            }
        }
        KeyCode::Char('G') => app.scroll_to_bottom(),
        KeyCode::Up | KeyCode::Char('k') => app.scroll_up(1),
        KeyCode::Down | KeyCode::Char('j') => app.scroll_down(1),
        KeyCode::PageUp => app.scroll_up(10),
        KeyCode::PageDown => app.scroll_down(10),
        _ => {}
    }
}

fn handle_session(app: &mut App, ev: SessionEvent) {
    match ev {
        SessionEvent::AssistantDelta(s) => {
            // Append to the currently-open assistant message, or start a new one.
            if let Some(last) = app.chat.last_mut() {
                if last.is_open_assistant() {
                    last.push_str(&s);
                    app.autoscroll();
                    return;
                }
            }
            app.chat.push(ChatMessage::text(Role::Assistant, s));
            app.turn = TurnState::Streaming;
            app.autoscroll();
        }
        SessionEvent::AssistantEnd => {}
        SessionEvent::ToolCall { name, arguments } => {
            app.push_line(Role::Tool, format!("▸ {name}{}", compact_args(&arguments)));
        }
        SessionEvent::ToolResult { result, .. } => {
            let mark = if result.is_error { "✗" } else { "✓" };
            let line = result.content.lines().next().unwrap_or_default();
            app.push_line(Role::Tool, format!("{mark} {line}"));
        }
        SessionEvent::RenderWidget { spec } => {
            app.chat.push(ChatMessage::widget(spec));
            app.autoscroll();
        }
        // A targeted render: upsert into the panel column, never the chat flow (FR-008/009). Same id
        // replaces in place; the chat still shows the tool-result summary line separately.
        SessionEvent::PanelUpdate { id, spec } => app.panels.upsert(id, spec),
        // A lifecycle effect: create/replace (with an optional TTL), remove one, or clear them all.
        SessionEvent::PanelOp(op) => app.panels.apply(op),
        SessionEvent::Error(s) => app.push_line(Role::System, format!("error: {s}")),
        SessionEvent::Info(s) | SessionEvent::Footer(s) | SessionEvent::Steering(s) => {
            app.push_line(Role::System, s)
        }
        SessionEvent::TurnStarted => app.turn = TurnState::Streaming,
        SessionEvent::TurnDone => app.turn = TurnState::Idle,
    }
}

/// A compact one-line arg summary for a tool-call line (empty for no/`{}` args).
fn compact_args(v: &serde_json::Value) -> String {
    let s = v.to_string();
    if s == "{}" || s == "null" {
        String::new()
    } else {
        let short: String = s.chars().take(48).collect();
        format!(" {short}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> App {
        App::new(120, 40)
    }

    fn type_str(a: &mut App, s: &str) {
        for c in s.chars() {
            update(a, Message::char(c));
        }
    }

    #[test]
    fn typing_then_enter_submits_a_user_message_and_fills_outbox() {
        let mut a = app();
        type_str(&mut a, "hello");
        assert_eq!(a.input.text(), "hello");
        update(&mut a, Message::key(KeyCode::Enter));
        assert_eq!(a.take_outbox().as_deref(), Some("hello"));
        assert_eq!(a.chat.len(), 1);
        assert!(matches!(a.chat[0].role, Role::User));
        assert!(a.input.is_empty(), "input cleared after submit");
    }

    #[test]
    fn blank_submit_does_nothing() {
        let mut a = app();
        update(&mut a, Message::key(KeyCode::Enter));
        assert!(a.outbox.is_none());
        assert!(a.chat.is_empty());
    }

    #[test]
    fn shift_enter_inserts_a_newline_not_a_submit() {
        let mut a = app();
        type_str(&mut a, "a");
        update(
            &mut a,
            Message::key_mods(KeyCode::Enter, KeyModifiers::SHIFT),
        );
        type_str(&mut a, "b");
        assert_eq!(a.input.text(), "a\nb");
        assert!(a.outbox.is_none());
    }

    #[test]
    fn streaming_deltas_coalesce_into_one_assistant_message() {
        let mut a = app();
        update(
            &mut a,
            Message::session(SessionEvent::AssistantDelta("Hel".into())),
        );
        update(
            &mut a,
            Message::session(SessionEvent::AssistantDelta("lo".into())),
        );
        assert_eq!(a.chat.len(), 1);
        assert!(matches!(a.chat[0].role, Role::Assistant));
        assert_eq!(a.turn, TurnState::Streaming);
        update(&mut a, Message::session(SessionEvent::TurnDone));
        assert_eq!(a.turn, TurnState::Idle);
    }

    #[test]
    fn tab_cycles_focus_and_ctrl_c_quits() {
        let mut a = app();
        assert_eq!(a.focus, Focus::Input);
        update(&mut a, Message::key(KeyCode::Tab));
        assert_eq!(a.focus, Focus::Chat);
        update(&mut a, Message::char_mods('c', KeyModifiers::CONTROL));
        assert!(a.should_quit);
    }

    #[test]
    fn q_quits_from_chat_but_types_in_input() {
        let mut a = app();
        // In input focus, 'q' is text.
        update(&mut a, Message::char('q'));
        assert_eq!(a.input.text(), "q");
        assert!(!a.should_quit);
        // In chat focus, 'q' quits.
        a.input.clear();
        update(&mut a, Message::key(KeyCode::Tab));
        update(&mut a, Message::char('q'));
        assert!(a.should_quit);
    }

    #[test]
    fn help_overlay_toggles_and_swallows_keys() {
        let mut a = app();
        update(&mut a, Message::key(KeyCode::Tab)); // focus Chat
        update(&mut a, Message::char('?'));
        assert!(a.help_open);
        update(&mut a, Message::char('q')); // swallowed by help, not a quit
        assert!(!a.should_quit);
        update(&mut a, Message::key(KeyCode::Esc));
        assert!(!a.help_open);
    }

    #[test]
    fn scrolling_breaks_and_restores_tail_follow() {
        let mut a = app();
        update(&mut a, Message::key(KeyCode::Tab)); // Chat focus
        update(&mut a, Message::key(KeyCode::PageUp));
        assert!(!a.follow_tail);
        assert_eq!(a.scroll, 10);
        // gg jumps to top.
        update(&mut a, Message::char('g'));
        update(&mut a, Message::char('g'));
        assert_eq!(a.scroll, usize::MAX);
        // G returns to the bottom and re-follows.
        update(&mut a, Message::char('G'));
        assert_eq!(a.scroll, 0);
        assert!(a.follow_tail);
    }

    #[test]
    fn resize_updates_layout_mode() {
        let mut a = app();
        assert_eq!(a.layout_mode, LayoutMode::TwoPane);
        update(&mut a, Message::Resize(90, 30));
        assert_eq!(a.layout_mode, LayoutMode::SinglePane);
        update(&mut a, Message::Resize(30, 8));
        assert_eq!(a.layout_mode, LayoutMode::TooSmall);
    }

    // --- US2 (T021): panel routing + upsert through the reducer ---------------------------------

    fn text_spec(s: &str) -> crate::render_spec::RenderSpec {
        crate::render_spec::RenderSpec::Text {
            content: s.into(),
            style: None,
            bold: false,
            dim: false,
        }
    }

    #[test]
    fn panel_update_upserts_into_panels_not_chat() {
        let mut a = app();
        update(
            &mut a,
            Message::session(SessionEvent::PanelUpdate {
                id: "metrics".into(),
                spec: text_spec("v1"),
            }),
        );
        // A targeted render populates the panel column and leaves chat untouched (FR-008).
        assert_eq!(a.panels.len(), 1);
        assert_eq!(a.panels.get("metrics"), Some(&text_spec("v1")));
        assert!(a.chat.is_empty(), "panel render must not append to chat");
    }

    #[test]
    fn re_rendering_a_panel_replaces_in_place_no_duplicate() {
        let mut a = app();
        for v in ["v1", "v2", "v3"] {
            update(
                &mut a,
                Message::session(SessionEvent::PanelUpdate {
                    id: "metrics".into(),
                    spec: text_spec(v),
                }),
            );
        }
        assert_eq!(
            a.panels.len(),
            1,
            "same id replaces — exactly one panel (SC-008)"
        );
        assert_eq!(a.panels.get("metrics"), Some(&text_spec("v3")));
    }

    #[test]
    fn panel_ops_remove_and_clear_through_the_reducer() {
        use crate::render_spec::PanelOp;
        let mut a = app();
        for id in ["a", "b", "c"] {
            update(
                &mut a,
                Message::session(SessionEvent::PanelOp(PanelOp::Upsert {
                    id: id.into(),
                    spec: text_spec("x"),
                    ttl_ms: None,
                })),
            );
        }
        assert_eq!(a.panels.len(), 3);

        update(
            &mut a,
            Message::session(SessionEvent::PanelOp(PanelOp::Remove { id: "b".into() })),
        );
        assert_eq!(a.panels.len(), 2);
        assert!(a.panels.get("b").is_none(), "removed panel is gone");

        update(
            &mut a,
            Message::session(SessionEvent::PanelOp(PanelOp::Clear)),
        );
        assert!(a.panels.is_empty(), "clear reclaims the whole column");
    }

    #[test]
    fn inline_render_goes_to_chat_not_panels() {
        let mut a = app();
        update(
            &mut a,
            Message::session(SessionEvent::RenderWidget {
                spec: text_spec("inline"),
            }),
        );
        assert_eq!(a.chat.len(), 1, "inline render appends a chat widget");
        assert!(a.panels.is_empty(), "inline render never touches panels");
    }
}
