//! The default [`ReplOutput`](super::ReplOutput): renders the session through a rustyline
//! [`ExternalPrinter`](rustyline::ExternalPrinter) so agent output scrolls *above* the live input
//! line without corrupting whatever the user is typing (which is what lets streaming output and
//! concurrent steering input coexist).
//!
//! Color is inline escapes (no color crate) and is suppressed when `NO_COLOR` is set (per
//! <https://no-color.org>). Assistant prose streams in, word-wrapped a line at a time as it
//! arrives; tool calls and results are indented and tagged; audit denials are called out in bold
//! red; a dim footer summarizes each exchange.

use std::sync::Mutex;

use bee_core::AuditEvent;
use rustyline::ExternalPrinter;

use super::ReplOutput;
use crate::tools::ToolResult;

/// Width to word-wrap assistant prose to.
const WRAP_WIDTH: usize = 88;
/// Max lines of a single tool result to print before eliding the rest.
const MAX_RESULT_LINES: usize = 40;
/// Max characters of a tool call's argument JSON to show on the call line.
const MAX_ARG_CHARS: usize = 100;

/// The mutable state of the in-progress streamed assistant paragraph. Deltas arrive a few tokens at
/// a time; we buffer the current visual line and emit it (wrapped) as soon as it fills or a newline
/// lands, so prose appears line-by-line as the model generates it.
#[derive(Default)]
struct StreamState {
    /// Whether the leading blank separator line for this assistant block has been printed.
    started: bool,
    /// The current, not-yet-emitted visual line (never contains `\n`, kept within `WRAP_WIDTH`).
    line: String,
}

/// Writes the REPL through a rustyline external printer (with ANSI color unless `NO_COLOR` is set).
pub struct TerminalOutput {
    color: bool,
    printer: Mutex<Box<dyn ExternalPrinter + Send>>,
    stream: Mutex<StreamState>,
}

impl TerminalOutput {
    /// Build a terminal output over an external printer taken from the active editor. Honors
    /// `NO_COLOR` (any value ⇒ no escapes).
    pub fn new(printer: Box<dyn ExternalPrinter + Send>) -> Self {
        TerminalOutput {
            color: std::env::var_os("NO_COLOR").is_none(),
            printer: Mutex::new(printer),
            stream: Mutex::new(StreamState::default()),
        }
    }

    /// Wrap `text` in an ANSI SGR sequence, or return it unchanged when color is disabled.
    fn paint(&self, code: &str, text: &str) -> String {
        if self.color {
            format!("\x1b[{code}m{text}\x1b[0m")
        } else {
            text.to_string()
        }
    }

    /// Print one line above the input line. A trailing newline is added; the external printer
    /// redraws the prompt beneath it. Printer errors are swallowed — a REPL should not abort a
    /// session because one line failed to render.
    fn emit(&self, line: &str) {
        if let Ok(mut p) = self.printer.lock() {
            let _ = p.print(format!("{line}\n"));
        }
    }

    /// Emit the current buffered assistant line and clear it.
    fn flush_line(&self, st: &mut StreamState) {
        let line = std::mem::take(&mut st.line);
        self.emit(&line);
    }
}

/// Display width of a string in columns (char count; good enough for wrap decisions on prose).
fn width(s: &str) -> usize {
    s.chars().count()
}

impl ReplOutput for TerminalOutput {
    fn assistant_delta(&self, chunk: &str) {
        let mut st = self.stream.lock().expect("stream state");
        if !st.started {
            self.emit(""); // blank separator before the assistant block
            st.started = true;
        }
        for c in chunk.chars() {
            if c == '\n' {
                self.flush_line(&mut st);
                continue;
            }
            st.line.push(c);
            if width(&st.line) > WRAP_WIDTH {
                // Greedy wrap: break at the last space, else hard-break an over-long word.
                if let Some(sp) = st.line.rfind(' ') {
                    let tail = st.line[sp + 1..].to_string();
                    st.line.truncate(sp);
                    self.flush_line(&mut st);
                    st.line = tail;
                } else {
                    self.flush_line(&mut st);
                }
            }
        }
    }

    fn assistant_end(&self) {
        let mut st = self.stream.lock().expect("stream state");
        if !st.line.is_empty() {
            self.flush_line(&mut st);
        }
        st.started = false;
    }

    fn tool_call(&self, name: &str, arguments: &serde_json::Value) {
        let args = clip(&arguments.to_string(), MAX_ARG_CHARS);
        let line = format!("  ▸ {name} {args}");
        self.emit(&self.paint("2", &line)); // dim
    }

    fn tool_result(&self, result: &ToolResult, audit: &[AuditEvent]) {
        let (glyph, code) = if result.is_error { ("✗", "31") } else { ("✓", "32") };
        let content = if result.content.trim().is_empty() {
            "(no output)".to_string()
        } else {
            result.content.clone()
        };

        let lines: Vec<&str> = content.lines().collect();
        let shown = lines.len().min(MAX_RESULT_LINES);
        for (i, line) in lines.iter().take(shown).enumerate() {
            let prefixed = if i == 0 {
                format!("  {glyph} {line}")
            } else {
                format!("    {line}")
            };
            self.emit(&self.paint(code, &prefixed));
        }
        if lines.len() > shown {
            let more = lines.len() - shown;
            self.emit(&self.paint("2", &format!("    … {more} more line(s)")));
        }

        // Kernel denials stand out in bold red regardless of the result glyph above.
        for e in audit {
            if e.decision == "denied" {
                let line = format!("  ⚠ DENIED {} {}", e.op, e.target);
                self.emit(&self.paint("1;31", &line)); // bold red
            }
        }
    }

    fn error(&self, msg: &str) {
        self.emit(&self.paint("31", &format!("error: {msg}"))); // red
    }

    fn info(&self, msg: &str) {
        self.emit(&self.paint("33", msg)); // yellow
    }

    fn footer(&self, msg: &str) {
        self.emit(&self.paint("2", msg)); // dim
    }

    fn steering(&self, msg: &str) {
        self.emit(&self.paint("36", msg)); // cyan — user's steering nudge
    }

    fn thinking(&self, msg: &str) {
        self.emit(&self.paint("2", msg)); // dim heartbeat
    }
}

/// The first `max` chars of `s`'s single-line form, with an ellipsis when clipped.
fn clip(s: &str, max: usize) -> String {
    let one_line: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() > max {
        let kept: String = one_line.chars().take(max).collect();
        format!("{kept}…")
    } else {
        one_line
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// An [`ExternalPrinter`] that captures printed messages into a shared buffer.
    #[derive(Clone, Default)]
    struct CapturePrinter(Arc<Mutex<String>>);

    impl ExternalPrinter for CapturePrinter {
        fn print(&mut self, msg: String) -> rustyline::Result<()> {
            self.0.lock().unwrap().push_str(&msg);
            Ok(())
        }
    }

    fn term() -> (TerminalOutput, Arc<Mutex<String>>) {
        let buf = Arc::new(Mutex::new(String::new()));
        let printer = CapturePrinter(buf.clone());
        // Force color off so assertions match raw text.
        let mut t = TerminalOutput::new(Box::new(printer));
        t.color = false;
        (t, buf)
    }

    #[test]
    fn streamed_deltas_reassemble_into_wrapped_lines() {
        let (t, buf) = term();
        // Feed a paragraph in arbitrary chunks; the emitted text (minus wrap newlines) must contain
        // the original words in order.
        for chunk in ["Hello ", "there, ", "how can ", "I help", " you today?"] {
            t.assistant_delta(chunk);
        }
        t.assistant_end();
        let out = buf.lock().unwrap().clone();
        let collapsed: String = out.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(collapsed.contains("Hello there, how can I help you today?"), "got: {out:?}");
    }

    #[test]
    fn long_line_wraps_at_width() {
        let (t, buf) = term();
        let words = "lorem ipsum ".repeat(30); // well over WRAP_WIDTH
        t.assistant_delta(&words);
        t.assistant_end();
        let out = buf.lock().unwrap().clone();
        for line in out.lines().filter(|l| !l.is_empty()) {
            assert!(width(line) <= WRAP_WIDTH, "line exceeds width: {line:?}");
        }
    }

    #[test]
    fn no_color_emits_no_escapes() {
        let (t, buf) = term();
        t.info("hi");
        assert!(!buf.lock().unwrap().contains("\x1b["));
    }

    #[test]
    fn clip_collapses_and_truncates() {
        assert_eq!(clip("a  b\n c", 100), "a b c");
        let long = clip(&"x".repeat(200), 10);
        assert!(long.ends_with('…'));
        assert_eq!(long.chars().count(), 11);
    }
}
