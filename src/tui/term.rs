//! Terminal lifecycle + the event→message mapping (008-grid-tui, US1 T017 / part of T018).
//!
//! [`init`] enters the alternate screen + raw mode and installs a panic hook that restores the
//! terminal *before* the trace prints; [`restore`] undoes it on the way out. Together they satisfy the
//! hard rule that every exit path — quit, `?`-early-return, panic — leaves a usable terminal (SC-001,
//! FR-002). OSC-52 copy (T035) and SIGTSTP suspend are layered on in US3.

use std::io::{self, Stdout};
use std::sync::Once;

use crossterm::event::{
    DisableMouseCapture, EnableMouseCapture, Event as CtEvent, KeyEventKind, MouseEventKind,
};
use crossterm::execute;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use super::message::Message;

/// Rows a single wheel notch scrolls (010). Three is the conventional terminal step — one row per
/// notch feels broken, and a full page overshoots.
const WHEEL_ROWS: u16 = 3;

/// The concrete terminal the TUI draws on.
pub type Tui = Terminal<CrosstermBackend<Stdout>>;

static MOUSE_PANIC_HOOK: Once = Once::new();

/// Enter the alternate screen + raw mode and return the ratatui terminal. `ratatui::init` also
/// installs a panic hook that restores the terminal first (SC-001) — install `color-eyre` before this
/// (in `main`) so the restored terminal then shows a pretty report.
///
/// Mouse reporting is enabled here so the wheel scrolls the chat (010). The trade is that
/// drag-to-select becomes the *application's* gesture rather than the terminal's; most emulators
/// still give it back under `Shift`, and `y` yanks the newest message via OSC 52 regardless.
pub fn init() -> Tui {
    let terminal = ratatui::init();
    let _ = execute!(io::stdout(), EnableMouseCapture);
    // ratatui's own hook restores the screen but knows nothing about mouse mode, so a panic would
    // leave the terminal reporting clicks as escape garbage. Chain ours in front of it — once, since
    // `init` runs again on every Ctrl-Z resume and stacked hooks would each re-run the whole chain.
    MOUSE_PANIC_HOOK.call_once(|| {
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = execute!(io::stdout(), DisableMouseCapture);
            prev(info);
        }));
    });
    terminal
}

/// Leave the alternate screen, disable raw mode, show the cursor. Safe to call more than once.
pub fn restore() {
    // Before the screen goes back: leaving mouse mode on would make the shell the user lands in
    // print escape sequences on every click.
    let _ = execute!(io::stdout(), DisableMouseCapture);
    ratatui::restore();
}

/// Suspend the process the way `Ctrl-Z` does on a normal terminal (008-grid-tui, US3 T035).
///
/// Raw mode swallows the real SIGTSTP, so the front-end recreates it: leave the alt-screen and
/// restore cooked mode **first** (so the user lands in a usable shell), re-raise `SIGTSTP` to
/// actually stop, and — once `fg` delivers `SIGCONT` and `raise` returns — re-enter the alt-screen.
/// The caller must force a full redraw afterwards, since the screen was handed back to the shell.
pub fn suspend_and_resume() -> Tui {
    restore();
    // SAFETY: `raise` only signals the calling process; SIGTSTP stops us until SIGCONT resumes.
    unsafe {
        libc::raise(libc::SIGTSTP);
    }
    init()
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
        // Only the wheel means anything to this UI (010). Clicks, drags and moves are deliberately
        // ignored: binding them would take gestures away from the terminal for no gain.
        CtEvent::Mouse(m) => match m.kind {
            MouseEventKind::ScrollUp => Some(Message::ScrollUp(WHEEL_ROWS)),
            MouseEventKind::ScrollDown => Some(Message::ScrollDown(WHEEL_ROWS)),
            _ => None,
        },
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
    fn the_wheel_maps_to_scroll_and_every_other_mouse_gesture_is_ignored() {
        use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
        let ev = |kind| {
            CtEvent::Mouse(MouseEvent {
                kind,
                column: 0,
                row: 0,
                modifiers: KeyModifiers::NONE,
            })
        };
        assert!(matches!(
            message_from_event(ev(MouseEventKind::ScrollUp)),
            Some(Message::ScrollUp(n)) if n == WHEEL_ROWS
        ));
        assert!(matches!(
            message_from_event(ev(MouseEventKind::ScrollDown)),
            Some(Message::ScrollDown(n)) if n == WHEEL_ROWS
        ));
        // Clicks and moves stay the terminal's business (010).
        assert!(message_from_event(ev(MouseEventKind::Down(MouseButton::Left))).is_none());
        assert!(message_from_event(ev(MouseEventKind::Moved)).is_none());
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
