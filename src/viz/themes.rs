//! The built-in themes (005-themes, FR-045): `honeycomb` (the 003 default, basic ANSI), the four
//! Catppuccin flavors, `dracula`, and `nord`. Values are embedded as Rust constants — no external
//! file, no new crate dependency (NFR-008). Each flavor's extended palette gives charts and sprites
//! theme-coordinated accent colors.

use std::collections::BTreeMap;

use crate::viz::theme::{Theme, ThemeColor};

/// An `Ansi` theme color from a bare SGR code.
fn a(code: &str) -> ThemeColor {
    ThemeColor::Ansi(code.to_string())
}

/// An `Rgb` theme color from a `#RRGGBB` literal (const-known; panics only on a typo in this file).
fn h(hex: &str) -> ThemeColor {
    let (r, g, b) = crate::viz::theme::parse_hex(hex).expect("built-in theme hex is valid");
    ThemeColor::Rgb { r, g, b }
}

/// Build an extended palette from `(name, hex)` pairs.
fn ext(pairs: &[(&str, &str)]) -> BTreeMap<String, ThemeColor> {
    pairs.iter().map(|(k, v)| (k.to_string(), h(v))).collect()
}

/// Look up a built-in theme by name (case-insensitive; `_`/spaces normalized to `-`). `None` if the
/// name is not a built-in.
pub fn builtin(name: &str) -> Option<Theme> {
    let n = name.trim().to_ascii_lowercase().replace(['_', ' '], "-");
    Some(match n.as_str() {
        "honeycomb" | "default" => honeycomb(),
        "catppuccin-mocha" | "mocha" => catppuccin_mocha(),
        "catppuccin-latte" | "latte" => catppuccin_latte(),
        "catppuccin-frappe" | "frappe" => catppuccin_frappe(),
        "catppuccin-macchiato" | "macchiato" => catppuccin_macchiato(),
        "dracula" => dracula(),
        "nord" => nord(),
        _ => return None,
    })
}

/// The names of every built-in theme (for `--help` text / diagnostics).
pub const BUILTIN_NAMES: &[&str] = &[
    "honeycomb",
    "catppuccin-mocha",
    "catppuccin-latte",
    "catppuccin-frappe",
    "catppuccin-macchiato",
    "dracula",
    "nord",
];

/// `honeycomb` — the original 003 palette, basic-ANSI for maximum compatibility. No extended palette
/// (charts/sprites use basic color words).
pub fn honeycomb() -> Theme {
    Theme {
        name: "honeycomb".into(),
        success: a("32"), // green
        error: a("31"),   // red
        info: a("33"),    // yellow
        dim: a("2"),      // dim
        accent: a("36"),  // cyan
        text: a("0"),     // reset (terminal default fg)
        extended: BTreeMap::new(),
    }
}

pub fn catppuccin_mocha() -> Theme {
    Theme {
        name: "catppuccin-mocha".into(),
        success: h("#a6e3a1"),
        error: h("#f38ba8"),
        info: h("#f9e2af"),
        dim: h("#6c7086"),
        accent: h("#89b4fa"),
        text: h("#cdd6f4"),
        extended: ext(&[
            ("pink", "#f5c2e7"),
            ("mauve", "#cba6f7"),
            ("peach", "#fab387"),
            ("teal", "#94e2d5"),
            ("sky", "#89dceb"),
            ("sapphire", "#74c7ec"),
            ("lavender", "#b4befe"),
            ("flamingo", "#f2cdcd"),
            ("rosewater", "#f5e0dc"),
            ("maroon", "#eba0ac"),
            ("surface0", "#313244"),
            ("surface1", "#45475a"),
            ("surface2", "#585b70"),
        ]),
    }
}

pub fn catppuccin_latte() -> Theme {
    Theme {
        name: "catppuccin-latte".into(),
        success: h("#40a02b"),
        error: h("#d20f39"),
        info: h("#df8e1d"),
        dim: h("#9ca0b0"),
        accent: h("#1e66f5"),
        text: h("#4c4f69"),
        extended: ext(&[
            ("pink", "#ea76cb"),
            ("mauve", "#8839ef"),
            ("peach", "#fe640b"),
            ("teal", "#179299"),
            ("sky", "#04a5e5"),
            ("sapphire", "#209fb5"),
            ("lavender", "#7287fd"),
            ("flamingo", "#dd7878"),
            ("rosewater", "#dc8a78"),
            ("maroon", "#e64553"),
            ("surface0", "#ccd0da"),
            ("surface1", "#bcc0cc"),
            ("surface2", "#acb0be"),
        ]),
    }
}

pub fn catppuccin_frappe() -> Theme {
    Theme {
        name: "catppuccin-frappe".into(),
        success: h("#a6d189"),
        error: h("#e78284"),
        info: h("#e5c890"),
        dim: h("#737994"),
        accent: h("#8caaee"),
        text: h("#c6d0f5"),
        extended: ext(&[
            ("pink", "#f4b8e4"),
            ("mauve", "#ca9ee6"),
            ("peach", "#ef9f76"),
            ("teal", "#81c8be"),
            ("sky", "#99d1db"),
            ("sapphire", "#85c1dc"),
            ("lavender", "#babbf1"),
            ("flamingo", "#eebebe"),
            ("rosewater", "#f2d5cf"),
            ("maroon", "#ea999c"),
            ("surface0", "#414559"),
            ("surface1", "#51576d"),
            ("surface2", "#626880"),
        ]),
    }
}

pub fn catppuccin_macchiato() -> Theme {
    Theme {
        name: "catppuccin-macchiato".into(),
        success: h("#a6da95"),
        error: h("#ed8796"),
        info: h("#eed49f"),
        dim: h("#6e738d"),
        accent: h("#8aadf4"),
        text: h("#cad3f5"),
        extended: ext(&[
            ("pink", "#f5bde6"),
            ("mauve", "#c6a0f6"),
            ("peach", "#f5a97f"),
            ("teal", "#8bd5ca"),
            ("sky", "#91d7e3"),
            ("sapphire", "#7dc4e4"),
            ("lavender", "#b7bdf8"),
            ("flamingo", "#f0c6c6"),
            ("rosewater", "#f4dbd6"),
            ("maroon", "#ee99a0"),
            ("surface0", "#363a4f"),
            ("surface1", "#494d64"),
            ("surface2", "#5b6078"),
        ]),
    }
}

pub fn dracula() -> Theme {
    Theme {
        name: "dracula".into(),
        success: h("#50fa7b"),
        error: h("#ff5555"),
        info: h("#f1fa8c"),
        dim: h("#6272a4"),
        accent: h("#8be9fd"),
        text: h("#f8f8f2"),
        extended: ext(&[
            ("purple", "#bd93f9"),
            ("pink", "#ff79c6"),
            ("orange", "#ffb86c"),
        ]),
    }
}

pub fn nord() -> Theme {
    Theme {
        name: "nord".into(),
        success: h("#a3be8c"),
        error: h("#bf616a"),
        info: h("#ebcb8b"),
        dim: h("#4c566a"),
        accent: h("#88c0d0"),
        text: h("#d8dee9"),
        extended: ext(&[
            ("orange", "#d08770"),
            ("purple", "#b48ead"),
            ("frost", "#5e81ac"),
        ]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_builtins_resolve() {
        for name in BUILTIN_NAMES {
            assert!(builtin(name).is_some(), "missing built-in {name}");
        }
        // FR-045: the six required themes are present.
        for required in [
            "honeycomb",
            "catppuccin-mocha",
            "catppuccin-latte",
            "catppuccin-frappe",
            "catppuccin-macchiato",
            "dracula",
        ] {
            assert!(builtin(required).is_some(), "FR-045 requires {required}");
        }
    }

    #[test]
    fn honeycomb_is_basic_ansi() {
        let t = honeycomb();
        assert_eq!(t.info, ThemeColor::Ansi("33".into()));
        assert_eq!(t.success, ThemeColor::Ansi("32".into()));
        assert_eq!(t.error, ThemeColor::Ansi("31".into()));
        assert!(t.extended.is_empty());
    }

    #[test]
    fn aliases_and_normalization() {
        assert_eq!(builtin("MOCHA").unwrap().name, "catppuccin-mocha");
        assert_eq!(
            builtin("catppuccin_mocha").unwrap().name,
            "catppuccin-mocha"
        );
        assert!(builtin("bogus").is_none());
    }
}
