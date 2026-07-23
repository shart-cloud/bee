//! bee's mascot, drawn into a ratatui buffer as half-block glyphs.
//!
//! Transcribed from `bee-harness/src/viz/bee.rs` (the 16×16 bitmap and its palette) and
//! `bee-harness/src/viz/sprite_render.rs` (the glyph-selection rule), because this crate cannot
//! depend on `bee-harness`. The `evolve_in` / `evolve_out` scenes need a sprite specifically: the
//! `EvolveSymbolSet::BlocksHorizontal` effect substitutes block glyphs, and block glyphs against a
//! surface already made of block glyphs is the one visual case that catches an upstream change to
//! tachyonfx's substitution table.
//!
//! Every pixel pair becomes ONE cell, so the sprite occupies 16 columns × 8 rows.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};

/// The sprite's cell dimensions: 16 pixels wide, 16 pixels tall packed two-per-cell.
pub const COLS: u16 = 16;
pub const ROWS: u16 = 8;

/// Lower half block `▄` — its foreground fills the lower half, the cell background the upper half.
const LOWER: &str = "▄";
/// Upper half block `▀` — its foreground fills the upper half.
const UPPER: &str = "▀";

/// The wings-level frame of bee's three-frame flap. Byte-identical to `FRAME_MID` in
/// `bee-harness/src/viz/bee.rs`. Left→right the anatomy reads: stinger, banded abdomen, thorax with
/// the wings above it, then the head with its white eye and antenna.
const FRAME_MID: [&str; 16] = [
    "................",
    ".............KK.",
    "............K...",
    ".......WWW..K...",
    "......WWWWW.KK..",
    ".....WWWWWWKKKK.",
    "....WWWWWW.KWWK.",
    "..KKKKKKKKKKKKKK",
    ".KRYYKKYYKKYKWKK",
    "KRYYYKKYYKKYKKKK",
    ".KRYYKKYYKKYKKK.",
    "..KKKKKKKKKKKK..",
    "...K..K..K......",
    "................",
    "................",
    "................",
];

/// The honeycomb bee palette. `None` is transparent, which is what `'.'` and anything unrecognized
/// map to.
fn pixel(c: char) -> Option<Color> {
    match c {
        'K' => Some(Color::Rgb(0x1A, 0x1A, 0x1A)), // outline
        'Y' => Some(Color::Rgb(0xE5, 0xA1, 0x00)), // honey body
        'W' => Some(Color::Rgb(0xFF, 0xFF, 0xFF)), // wings
        'B' => Some(Color::Rgb(0x4A, 0x90, 0xD9)), // wing tint
        'R' => Some(Color::Rgb(0xD7, 0x5F, 0x5F)), // sting
        _ => None,
    }
}

fn at(x: u16, y: u16) -> Option<Color> {
    FRAME_MID
        .get(y as usize)
        .and_then(|row| row.chars().nth(x as usize))
        .and_then(pixel)
}

/// Draw the mascot into `buf`, top-left aligned inside `area`.
///
/// The glyph choice follows bee's rule exactly, and for the same reason: a transparent half must
/// emit no color of its own. Both halves opaque uses `▄` with the top pixel as the *background*;
/// a lone top half uses `▀`; a lone bottom half uses `▄`.
pub fn draw(buf: &mut Buffer, area: Rect) {
    for r in 0..ROWS {
        for x in 0..COLS {
            let (cx, cy) = (area.x + x, area.y + r);
            if !buf.area.contains((cx, cy).into()) {
                continue;
            }
            let top = at(x, 2 * r);
            let bottom = at(x, 2 * r + 1);
            let cell = &mut buf[(cx, cy)];
            match (top, bottom) {
                (None, None) => {
                    cell.set_symbol(" ");
                }
                (Some(t), Some(b)) => {
                    cell.set_symbol(LOWER).set_style(Style::default().fg(b).bg(t));
                }
                (Some(t), None) => {
                    cell.set_symbol(UPPER).set_style(Style::default().fg(t));
                }
                (None, Some(b)) => {
                    cell.set_symbol(LOWER).set_style(Style::default().fg(b));
                }
            }
        }
    }
}
