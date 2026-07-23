//! The colors the scenes draw in.
//!
//! These are bee's `catppuccin-mocha` built-in theme (`bee-harness/src/viz/themes.rs`), transcribed
//! as literals. Transcribed rather than imported because this crate deliberately does not depend on
//! `bee-harness` — that crate's tree is tokio, rig-core, aya, rustyline and rhai, none of which
//! compile to wasm32.
//!
//! Mocha rather than bee's default `honeycomb`: honeycomb is basic-ANSI (`\e[33m` and friends), and a
//! browser has no terminal palette to resolve those against, so ratzilla substitutes its own table.
//! Truecolor roles mean the pixel in the baseline is the color this file names.

use ratatui::style::Color;

/// `success` — a passing episode.
pub const SUCCESS: Color = Color::Rgb(0xa6, 0xe3, 0xa1);
/// `error` — a failing one.
pub const ERROR: Color = Color::Rgb(0xf3, 0x8b, 0xa8);
/// `info`. Also the color every slide/sweep/fade in bee starts from or lands on: `tui::effects::build`
/// resolves its `bg` argument as `role_color(Role::Info)`.
pub const INFO: Color = Color::Rgb(0xf9, 0xe2, 0xaf);
/// `dim` — secondary text.
pub const DIM: Color = Color::Rgb(0x6c, 0x70, 0x86);
/// `accent` — the color a tool-result pulse flashes.
pub const ACCENT: Color = Color::Rgb(0x89, 0xb4, 0xfa);
/// `text` — ordinary foreground.
pub const TEXT: Color = Color::Rgb(0xcd, 0xd6, 0xf4);

/// Mocha `base`: the page background, restated here so widgets that paint their own background land
/// on exactly the backdrop rather than a near-miss.
pub const BASE: Color = Color::Rgb(0x1e, 0x1e, 0x2e);
/// Mocha `surface0` — panel fill, one step up from the backdrop.
pub const SURFACE0: Color = Color::Rgb(0x31, 0x32, 0x44);
