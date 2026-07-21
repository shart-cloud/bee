//! The honeycomb palette (003-visual-render, FR-026): six semantic colors as basic-ANSI SGR codes,
//! plus the `paint`/`bold`/`is_color_enabled` helpers. Suppressed when `NO_COLOR` is set (any value,
//! per <https://no-color.org>). This is the single source of truth the REPL chrome (`terminal.rs`)
//! and the `render_to_ansi` pipeline both draw from; the `paint` output is byte-for-byte identical
//! to the pre-migration `terminal.rs::paint` (SC-012).

/// Yellow — info, banners, the bee identity.
pub const HONEY: &str = "33";
/// Green — success / pass / allowed.
pub const POLLEN: &str = "32";
/// Red — error / fail / denied.
pub const STING: &str = "31";
/// Dim — footers, metadata, tool args.
pub const SMOKE: &str = "2";
/// Cyan — steering, highlights, links.
pub const ROYAL: &str = "36";

/// Whether ANSI color should be emitted. `false` when `NO_COLOR` is set to any value.
pub fn is_color_enabled() -> bool {
    std::env::var_os("NO_COLOR").is_none()
}

/// Wrap `text` in an SGR sequence for `code`, or return it unchanged when color is disabled.
/// Byte-identical to the legacy `terminal.rs::paint` (SC-012).
pub fn paint(code: &str, text: &str) -> String {
    if is_color_enabled() {
        format!("\x1b[{code}m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

/// The bold variant: `1;{code}` (e.g. bold red for `⚠ DENIED`).
pub fn bold(code: &str, text: &str) -> String {
    if is_color_enabled() {
        format!("\x1b[1;{code}m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

/// Like [`paint`] but honoring an explicit `color` flag rather than the process-wide `NO_COLOR`
/// check — used by `TerminalOutput`, which decides color once at construction.
pub fn paint_if(color: bool, code: &str, text: &str) -> String {
    if color {
        format!("\x1b[{code}m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

/// Like [`bold`] but honoring an explicit `color` flag (see [`paint_if`]).
pub fn bold_if(color: bool, code: &str, text: &str) -> String {
    if color {
        format!("\x1b[1;{code}m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paint_if_wraps_and_bypasses() {
        assert_eq!(paint_if(true, POLLEN, "ok"), "\x1b[32mok\x1b[0m");
        assert_eq!(paint_if(false, POLLEN, "ok"), "ok");
        assert_eq!(bold_if(true, STING, "no"), "\x1b[1;31mno\x1b[0m");
    }
}
