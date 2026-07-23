//! The honeycomb palette (003-visual-render, FR-026): six semantic colors, plus the `paint`/`bold`/
//! `is_color_enabled` helpers. Suppressed when `NO_COLOR` is set (any value, per <https://no-color.org>).
//! This is the single source of truth the REPL chrome (`terminal.rs`) and the `render_to_ansi`
//! pipeline both draw from.
//!
//! **Themes (005-themes):** the colors are no longer hardcoded — the six [`Role`] functions
//! (`success`/`error`/`info`/`dim`/`accent`/`text`) resolve through the process-wide active theme
//! (`viz::theme`). The `honeycomb` default keeps the basic-ANSI codes below, so with it active the
//! output is byte-for-byte identical to the pre-theming behavior (SC-030). The legacy `HONEY`/`POLLEN`
//! constants and `paint`/`paint_if` remain for the status-grid/dot path.

use crate::viz::sprite_render::{detect_color_mode, ColorMode};
use crate::viz::theme::{self, Role, Theme};

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

// --- Theme-aware role painting (005-themes) --------------------------------------------------

/// Paint `text` with `role` from an explicit `theme`, color `mode`, and `color` flag. The pure core
/// every other role function delegates to (no globals ⇒ deterministic in tests). `bold` prepends the
/// `1;` intensity, exactly as the legacy `bold`/`bold_if` did. With `color` off, returns `text`
/// unchanged (no escapes — SC-031).
pub fn paint_with(
    theme: &Theme,
    role: Role,
    mode: ColorMode,
    color: bool,
    bold: bool,
    text: &str,
) -> String {
    if !color {
        return text.to_string();
    }
    let params = theme.get(role).sgr_params(mode);
    if bold {
        format!("\x1b[1;{params}m{text}\x1b[0m")
    } else {
        format!("\x1b[{params}m{text}\x1b[0m")
    }
}

/// Paint `text` with `role` honoring an explicit `color` flag (the active theme + ambient color mode).
/// Used by `TerminalOutput`, which decides color once at construction — the role analogue of
/// [`paint_if`].
pub fn role_if(color: bool, role: Role, text: &str) -> String {
    paint_with(
        theme::active_theme(),
        role,
        detect_color_mode(),
        color,
        false,
        text,
    )
}

/// Like [`role_if`] but bold (e.g. the `⚠ DENIED` line) — the role analogue of [`bold_if`].
pub fn bold_role_if(color: bool, role: Role, text: &str) -> String {
    paint_with(
        theme::active_theme(),
        role,
        detect_color_mode(),
        color,
        true,
        text,
    )
}

/// Paint `text` with `role` from the active theme, honoring the process-wide `NO_COLOR`.
fn paint_role(role: Role, text: &str) -> String {
    role_if(is_color_enabled(), role, text)
}

/// Success (green states). Theme role [`Role::Success`].
pub fn success(text: &str) -> String {
    paint_role(Role::Success, text)
}
/// Error / denial (red states). Theme role [`Role::Error`].
pub fn error(text: &str) -> String {
    paint_role(Role::Error, text)
}
/// Info / banners / identity. Theme role [`Role::Info`].
pub fn info(text: &str) -> String {
    paint_role(Role::Info, text)
}
/// Secondary / metadata. Theme role [`Role::Dim`].
pub fn dim(text: &str) -> String {
    paint_role(Role::Dim, text)
}
/// Highlights / steering. Theme role [`Role::Accent`].
pub fn accent(text: &str) -> String {
    paint_role(Role::Accent, text)
}
/// Default text. Theme role [`Role::Text`].
pub fn text(s: &str) -> String {
    paint_role(Role::Text, s)
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
    use crate::viz::themes;

    #[test]
    fn paint_if_wraps_and_bypasses() {
        assert_eq!(paint_if(true, POLLEN, "ok"), "\x1b[32mok\x1b[0m");
        assert_eq!(paint_if(false, POLLEN, "ok"), "ok");
        assert_eq!(bold_if(true, STING, "no"), "\x1b[1;31mno\x1b[0m");
    }

    #[test]
    fn honeycomb_info_is_basic_ansi_yellow() {
        // SC-030: with honeycomb, info() emits the legacy \x1b[33m — byte-identical to pre-theming.
        let out = paint_with(
            &themes::honeycomb(),
            Role::Info,
            ColorMode::TrueColor,
            true,
            false,
            "x",
        );
        assert_eq!(out, "\x1b[33mx\x1b[0m");
    }

    #[test]
    fn mocha_info_is_truecolor() {
        // SC-029: catppuccin-mocha info() (#f9e2af) on a truecolor terminal.
        let out = paint_with(
            &themes::catppuccin_mocha(),
            Role::Info,
            ColorMode::TrueColor,
            true,
            false,
            "x",
        );
        assert!(out.contains("\x1b[38;2;249;226;175m"), "got: {out:?}");
    }

    #[test]
    fn mocha_info_quantizes_without_truecolor() {
        // SC-034: no truecolor ⇒ a valid 256-color \x1b[38;5;Nm escape, no error.
        let out = paint_with(
            &themes::catppuccin_mocha(),
            Role::Info,
            ColorMode::Ansi256,
            true,
            false,
            "x",
        );
        assert!(out.contains("\x1b[38;5;"), "got: {out:?}");
    }

    #[test]
    fn color_off_emits_no_escapes() {
        // SC-031: any theme, color disabled ⇒ no \x1b[ anywhere.
        for role in [
            Role::Success,
            Role::Error,
            Role::Info,
            Role::Dim,
            Role::Accent,
            Role::Text,
        ] {
            let out = paint_with(
                &themes::catppuccin_mocha(),
                role,
                ColorMode::TrueColor,
                false,
                false,
                "x",
            );
            assert_eq!(out, "x");
            assert!(!out.contains("\x1b["));
        }
    }
}
