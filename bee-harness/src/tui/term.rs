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

/// RAII guard for the terminal-restore contract (008-grid-tui, T010; contracts/modes-and-cli.md).
///
/// Holding one guarantees the terminal is restored on **every** way out of [`run`](super::run) that
/// isn't a clean explicit restore: an early `?` return, or a panic unwinding through the loop. On
/// the normal quit path we call [`restore`] explicitly and then [`disarm`](Self::disarm) so the
/// guard's `Drop` doesn't restore a second time. The action is injectable so the restore-matrix
/// tests can observe the guarantee deterministically, with no real terminal (which CI lacks).
pub struct RestoreGuard {
    action: Option<Box<dyn FnMut()>>,
}

impl RestoreGuard {
    /// A guard that runs the real terminal [`restore`] on drop.
    pub fn terminal() -> Self {
        Self::with(restore)
    }

    /// A guard that runs `action` on drop instead of the real restore — the seam the matrix test
    /// uses to count restores without touching a terminal.
    pub fn with(action: impl FnMut() + 'static) -> Self {
        RestoreGuard {
            action: Some(Box::new(action)),
        }
    }

    /// Disarm the guard so its `Drop` is a no-op. Call after an explicit [`restore`] on the normal
    /// exit path to avoid restoring twice.
    pub fn disarm(&mut self) {
        self.action = None;
    }
}

impl Drop for RestoreGuard {
    fn drop(&mut self) {
        if let Some(mut action) = self.action.take() {
            action();
        }
    }
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
