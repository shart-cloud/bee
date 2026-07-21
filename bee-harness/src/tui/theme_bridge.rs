//! Theme → ratatui `Style` bridge (008-grid-tui, T007).
//!
//! Maps the harness's semantic [`Role`]s through the active theme to a ratatui [`Style`], reusing the
//! [`theme_to_ratatui_color`] mapping the headless renderer already uses. **Honors `NO_COLOR`**: when
//! color is disabled it returns an unstyled `Style`, so the full-screen path stays legible in
//! monochrome exactly like the inline REPL (FR-014 / SC-005; resolves analysis G2).

use ratatui::style::{Modifier, Style};

use crate::viz::buffer_render::theme_to_ratatui_color;
use crate::viz::palette;
use crate::viz::theme::{self, Role};

/// The ratatui foreground [`Style`] for a semantic role under the active theme. Unstyled under
/// `NO_COLOR`.
pub fn role_style(role: Role) -> Style {
    if !palette::is_color_enabled() {
        return Style::default();
    }
    Style::default().fg(theme_to_ratatui_color(theme::active_theme().get(role)))
}

/// [`role_style`] with the bold modifier (titles, the focused message role). Bold survives `NO_COLOR`
/// — weight is not color — so a monochrome terminal still carries the emphasis.
pub fn role_style_bold(role: Role) -> Style {
    role_style(role).add_modifier(Modifier::BOLD)
}

/// The dim style for metadata / footers (`Role::Dim`), unstyled under `NO_COLOR`.
pub fn dim_style() -> Style {
    role_style(Role::Dim)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_color_yields_unstyled() {
        // SC-005: with NO_COLOR set, the bridge carries no foreground color (monochrome floor).
        std::env::set_var("NO_COLOR", "1");
        assert_eq!(role_style(Role::Info), Style::default());
        assert_eq!(role_style(Role::Error).fg, None);
        // Bold still applies — weight is not color.
        assert!(role_style_bold(Role::Info)
            .add_modifier
            .contains(Modifier::BOLD));
        std::env::remove_var("NO_COLOR");
    }

    #[test]
    fn color_on_sets_a_foreground() {
        std::env::remove_var("NO_COLOR");
        assert!(role_style(Role::Info).fg.is_some());
    }
}
