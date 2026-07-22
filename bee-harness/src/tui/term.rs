//! Terminal lifecycle + the event→message mapping (008-grid-tui, US1 T017 / part of T018).
//!
//! [`init`] enters the alternate screen + raw mode and installs a panic hook that restores the
//! terminal *before* the trace prints; [`restore`] undoes it on the way out. Together they satisfy the
//! hard rule that every exit path — quit, `?`-early-return, panic — leaves a usable terminal (SC-001,
//! FR-002). OSC-52 copy (T035) and SIGTSTP suspend are layered on in US3.

use std::io::{self, Stdout};

use crossterm::event::{Event as CtEvent, KeyEventKind};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use super::message::Message;

/// The concrete terminal the TUI draws on.
pub type Tui = Terminal<CrosstermBackend<Stdout>>;

/// Enter the alternate screen + raw mode and return the ratatui terminal. `ratatui::init` also
/// installs a panic hook that restores the terminal first (SC-001) — install `color-eyre` before this
/// (in `main`) so the restored terminal then shows a pretty report.
pub fn init() -> Tui {
    ratatui::init()
}

/// Leave the alternate screen, disable raw mode, show the cursor. Safe to call more than once.
pub fn restore() {
    ratatui::restore();
}

/// Copy `text` to the system clipboard via **OSC 52** (008-grid-tui, US3 T035). Works over SSH/tmux
/// where the *local* emulator interprets the escape (with tmux `set-clipboard on`). Best-effort.
pub fn osc52_copy(text: &str) -> io::Result<()> {
    use std::io::Write;
    let payload = base64_encode(text.as_bytes()); // tiny inline encoder below — no new dependency
    let mut out = io::stdout();
    write!(out, "\x1b]52;c;{payload}\x07")?;
    out.flush()
}

/// Translate a crossterm terminal event into a reducer [`Message`] — the pure, unit-testable part of
/// the event loop (T018). Key *releases* and events the app ignores map to `None`.
pub fn message_from_event(ev: CtEvent) -> Option<Message> {
    match ev {
        // Some terminals (Kitty protocol) send Release/Repeat too; only act on presses + repeats.
        CtEvent::Key(k) if k.kind != KeyEventKind::Release => Some(Message::Key(k)),
        CtEvent::Resize(cols, rows) => Some(Message::Resize(cols, rows)),
        CtEvent::Paste(s) => Some(Message::Paste(s)),
        _ => None,
    }
}

/// Minimal standard-base64 encoder (OSC 52 payloads only) — avoids pulling a base64 crate.
fn base64_encode(input: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | (b[2] as u32);
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            T[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    #[test]
    fn key_press_maps_to_message_release_is_ignored() {
        let press = CtEvent::Key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        assert!(matches!(message_from_event(press), Some(Message::Key(_))));

        let mut rel = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE);
        rel.kind = KeyEventKind::Release;
        assert!(message_from_event(CtEvent::Key(rel)).is_none());
    }

    #[test]
    fn resize_and_paste_map_through() {
        assert!(matches!(
            message_from_event(CtEvent::Resize(80, 24)),
            Some(Message::Resize(80, 24))
        ));
        assert!(matches!(
            message_from_event(CtEvent::Paste("hi".into())),
            Some(Message::Paste(s)) if s == "hi"
        ));
    }

    #[test]
    fn base64_matches_known_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"hello"), "aGVsbG8=");
    }
}
