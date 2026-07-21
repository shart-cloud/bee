//! Half-block sprite rendering (003-visual-render, Slice 2, FR-030; research D12/D13). A `W×H`-pixel
//! [`SpriteSpec`] renders to `⌈H/2⌉` terminal rows by packing two vertical pixels into one cell:
//! the block glyph is chosen by which halves are opaque so a transparent half emits **no** color
//! escape (SC-015). Colors are truecolor when the terminal supports it, else quantized to the
//! xterm-256 or 16-color palette; off a tty / under `NO_COLOR` the sprite degrades to monochrome
//! block characters. Pure ANSI — no ratatui, no new crate (NFR-002).

use crate::render_spec::SpriteSpec;

/// Lower half block `▄` (U+2584): its foreground fills the **lower** half, the cell background the
/// upper half.
const LOWER: &str = "▄";
/// Upper half block `▀` (U+2580): foreground fills the **upper** half.
const UPPER: &str = "▀";
/// Full block `█` (U+2588): monochrome "both halves opaque".
const FULL: &str = "█";

/// How much color fidelity the terminal supports (research D13).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorMode {
    /// 24-bit `38;2;R;G;B`.
    TrueColor,
    /// xterm-256 `38;5;N`.
    Ansi256,
    /// The 16 basic ANSI colors.
    Ansi16,
}

/// Detect the terminal's color capability from `COLORTERM` / `TERM` (research D13).
pub fn detect_color_mode() -> ColorMode {
    if let Ok(ct) = std::env::var("COLORTERM") {
        let c = ct.to_ascii_lowercase();
        if c.contains("truecolor") || c.contains("24bit") {
            return ColorMode::TrueColor;
        }
    }
    if std::env::var("TERM").map(|t| t.contains("256color")).unwrap_or(false) {
        return ColorMode::Ansi256;
    }
    ColorMode::Ansi16
}

/// Squared RGB distance.
fn dist(a: (u8, u8, u8), b: (u8, u8, u8)) -> u32 {
    let d = |x: u8, y: u8| {
        let v = x as i32 - y as i32;
        (v * v) as u32
    };
    d(a.0, b.0) + d(a.1, b.1) + d(a.2, b.2)
}

/// Nearest xterm-256 index for an RGB color: the closer of the 6×6×6 color cube (16–231) and the
/// 24-step grayscale ramp (232–255), by squared distance.
pub fn quantize_256(r: u8, g: u8, b: u8) -> u8 {
    const STEPS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    let comp = |v: u8| -> (usize, u8) {
        let mut best = 0usize;
        let mut bd = u32::MAX;
        for (i, &s) in STEPS.iter().enumerate() {
            let d = (s as i32 - v as i32).unsigned_abs();
            if d < bd {
                bd = d;
                best = i;
            }
        }
        (best, STEPS[best])
    };
    let (ri, rv) = comp(r);
    let (gi, gv) = comp(g);
    let (bi, bv) = comp(b);
    let cube_idx = 16 + 36 * ri + 6 * gi + bi;
    let cube_d = dist((r, g, b), (rv, gv, bv));

    let avg = ((r as u32 + g as u32 + b as u32) / 3) as i32;
    let gi2 = (((avg - 8).max(0) as u32) / 10).min(23);
    let gray_v = (8 + gi2 * 10) as u8;
    let gray_d = dist((r, g, b), (gray_v, gray_v, gray_v));

    if gray_d < cube_d {
        232 + gi2 as u8
    } else {
        cube_idx as u8
    }
}

/// The 16 basic ANSI colors as approximate RGB (0..7 normal, 8..15 bright).
const ANSI16: [(u8, u8, u8); 16] = [
    (0, 0, 0),
    (128, 0, 0),
    (0, 128, 0),
    (128, 128, 0),
    (0, 0, 128),
    (128, 0, 128),
    (0, 128, 128),
    (192, 192, 192),
    (128, 128, 128),
    (255, 0, 0),
    (0, 255, 0),
    (255, 255, 0),
    (0, 0, 255),
    (255, 0, 255),
    (0, 255, 255),
    (255, 255, 255),
];

/// Nearest of the 16 basic ANSI colors (index 0..=15) for an RGB color.
pub fn quantize_16(r: u8, g: u8, b: u8) -> u8 {
    let mut best = 0u8;
    let mut bd = u32::MAX;
    for (i, &c) in ANSI16.iter().enumerate() {
        let d = dist((r, g, b), c);
        if d < bd {
            bd = d;
            best = i as u8;
        }
    }
    best
}

/// The foreground SGR sequence for `(r,g,b)` in `mode`.
fn fg_seq(mode: ColorMode, (r, g, b): (u8, u8, u8)) -> String {
    match mode {
        ColorMode::TrueColor => format!("\x1b[38;2;{r};{g};{b}m"),
        ColorMode::Ansi256 => format!("\x1b[38;5;{}m", quantize_256(r, g, b)),
        ColorMode::Ansi16 => {
            let n = quantize_16(r, g, b);
            let code = if n < 8 { 30 + n } else { 90 + (n - 8) };
            format!("\x1b[{code}m")
        }
    }
}

/// The background SGR sequence for `(r,g,b)` in `mode`.
fn bg_seq(mode: ColorMode, (r, g, b): (u8, u8, u8)) -> String {
    match mode {
        ColorMode::TrueColor => format!("\x1b[48;2;{r};{g};{b}m"),
        ColorMode::Ansi256 => format!("\x1b[48;5;{}m", quantize_256(r, g, b)),
        ColorMode::Ansi16 => {
            let n = quantize_16(r, g, b);
            let code = if n < 8 { 40 + n } else { 100 + (n - 8) };
            format!("\x1b[{code}m")
        }
    }
}

/// Render one frame using the ambient terminal capabilities (`detect_color_mode` + `NO_COLOR`).
pub fn render_frame(spec: &SpriteSpec) -> Vec<String> {
    render_frame_with(spec, detect_color_mode(), crate::viz::palette::is_color_enabled())
}

/// Render one frame with an explicit color `mode` and `color` flag (exposed for deterministic tests).
/// When `color` is false the sprite degrades to monochrome block characters with no SGR (off-tty /
/// `NO_COLOR`, spec Edge Case). A fully-transparent sprite yields `⌈height/2⌉` blank rows.
pub fn render_frame_with(spec: &SpriteSpec, mode: ColorMode, color: bool) -> Vec<String> {
    let rows = spec.height.div_ceil(2);
    let mut out = Vec::with_capacity(rows as usize);
    for r in 0..rows {
        let mut line = String::new();
        for x in 0..spec.width {
            let top = spec.pixel(x, 2 * r);
            let bottom = spec.pixel(x, 2 * r + 1);
            match (top, bottom, color) {
                // --- monochrome (no color): block silhouette, no SGR ---
                (None, None, _) => line.push(' '),
                (Some(_), Some(_), false) => line.push_str(FULL),
                (Some(_), None, false) => line.push_str(UPPER),
                (None, Some(_), false) => line.push_str(LOWER),
                // --- color: glyph chosen so a transparent half emits no escape (SC-015) ---
                (Some(t), Some(b), true) => {
                    line.push_str(&bg_seq(mode, t));
                    line.push_str(&fg_seq(mode, b));
                    line.push_str(LOWER);
                    line.push_str("\x1b[0m");
                }
                (Some(t), None, true) => {
                    line.push_str(&fg_seq(mode, t));
                    line.push_str(UPPER);
                    line.push_str("\x1b[0m");
                }
                (None, Some(b), true) => {
                    line.push_str(&fg_seq(mode, b));
                    line.push_str(LOWER);
                    line.push_str("\x1b[0m");
                }
            }
        }
        out.push(line);
    }
    out
}
