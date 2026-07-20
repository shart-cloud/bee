//! The default [`ReplOutput`](super::ReplOutput): renders the session to stdout with ANSI color.
//!
//! Color is inline escapes (no color crate) and is suppressed when `NO_COLOR` is set (per
//! <https://no-color.org>). Assistant prose is word-wrapped; tool calls and results are indented and
//! tagged; audit denials are called out in bold red.

use bee_core::AuditEvent;

use super::ReplOutput;
use crate::tools::ToolResult;

/// Width to word-wrap assistant prose to.
const WRAP_WIDTH: usize = 88;
/// Max lines of a single tool result to print before eliding the rest.
const MAX_RESULT_LINES: usize = 40;
/// Max characters of a tool call's argument JSON to show on the call line.
const MAX_ARG_CHARS: usize = 100;

/// Writes the REPL to stdout with ANSI color (unless `NO_COLOR` is set).
pub struct TerminalOutput {
    color: bool,
}

impl TerminalOutput {
    /// Build a terminal output. Honors `NO_COLOR` (any value ⇒ no escapes).
    pub fn new() -> Self {
        TerminalOutput { color: std::env::var_os("NO_COLOR").is_none() }
    }

    /// Wrap `text` in an ANSI SGR sequence, or return it unchanged when color is disabled.
    fn paint(&self, code: &str, text: &str) -> String {
        if self.color {
            format!("\x1b[{code}m{text}\x1b[0m")
        } else {
            text.to_string()
        }
    }
}

impl Default for TerminalOutput {
    fn default() -> Self {
        TerminalOutput::new()
    }
}

/// Word-wrap `text` to `width` columns, preserving existing line breaks. Words longer than `width`
/// are emitted on their own (over-long) line rather than split.
fn wrap(text: &str, width: usize) -> String {
    let mut out = String::new();
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let mut col = 0usize;
        for word in line.split_whitespace() {
            let wlen = word.chars().count();
            if col == 0 {
                out.push_str(word);
                col = wlen;
            } else if col + 1 + wlen <= width {
                out.push(' ');
                out.push_str(word);
                col += 1 + wlen;
            } else {
                out.push('\n');
                out.push_str(word);
                col = wlen;
            }
        }
    }
    out
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

impl ReplOutput for TerminalOutput {
    fn assistant_text(&self, text: &str) {
        println!("\n{}", wrap(text.trim_end(), WRAP_WIDTH));
    }

    fn tool_call(&self, name: &str, arguments: &serde_json::Value) {
        let args = clip(&arguments.to_string(), MAX_ARG_CHARS);
        let line = format!("  ▸ {name} {args}");
        println!("{}", self.paint("2", &line)); // dim
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
            println!("{}", self.paint(code, &prefixed));
        }
        if lines.len() > shown {
            let more = lines.len() - shown;
            println!("{}", self.paint("2", &format!("    … {more} more line(s)")));
        }

        // Kernel denials stand out in bold red regardless of the result glyph above.
        for e in audit {
            if e.decision == "denied" {
                let line = format!("  ⚠ DENIED {} {}", e.op, e.target);
                println!("{}", self.paint("1;31", &line)); // bold red
            }
        }
    }

    fn error(&self, msg: &str) {
        eprintln!("{}", self.paint("31", &format!("error: {msg}"))); // red
    }

    fn info(&self, msg: &str) {
        println!("{}", self.paint("33", msg)); // yellow
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_breaks_long_lines() {
        let text = "one two three four five six seven eight nine ten";
        let wrapped = wrap(text, 12);
        assert!(wrapped.contains('\n'), "expected a wrap: {wrapped:?}");
        for line in wrapped.lines() {
            assert!(line.chars().count() <= 12 || !line.contains(' '), "line too long: {line:?}");
        }
    }

    #[test]
    fn wrap_preserves_existing_newlines() {
        let wrapped = wrap("alpha\nbeta", 80);
        assert_eq!(wrapped, "alpha\nbeta");
    }

    #[test]
    fn clip_collapses_and_truncates() {
        let clipped = clip("a  b\n c", 100);
        assert_eq!(clipped, "a b c");
        let long = clip(&"x".repeat(200), 10);
        assert!(long.ends_with('…'));
        assert_eq!(long.chars().count(), 11);
    }

    #[test]
    fn no_color_emits_no_escapes() {
        let plain = TerminalOutput { color: false };
        assert_eq!(plain.paint("31", "hi"), "hi");
        let colored = TerminalOutput { color: true };
        assert!(colored.paint("31", "hi").contains("\x1b["));
    }
}
