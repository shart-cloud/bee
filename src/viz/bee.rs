//! The built-in bee mascot (003-visual-render, Slice 2, FR-032): a 16×16 sprite and a 3-frame
//! wing-flap animation, built once via [`std::sync::OnceLock`] from a bitmap + palette (deterministic;
//! no clock/random). Available to the REPL startup banner (on by default; `--no-bee` / `BEE_MASCOT=0`) and to
//! the Rhai API as `bee_sprite()` / `bee_animation()`.

use std::sync::OnceLock;

use crate::render_spec::{AnimationSpec, SpriteSpec};

/// Map a bitmap character to an RGB pixel (`None` = transparent). The honeycomb bee palette.
fn palette(c: char) -> Option<(u8, u8, u8)> {
    match c {
        'K' => Some((0x1A, 0x1A, 0x1A)), // outline
        'Y' => Some((0xE5, 0xA1, 0x00)), // honey body
        'W' => Some((0xFF, 0xFF, 0xFF)), // wings
        'B' => Some((0x4A, 0x90, 0xD9)), // wing tint
        'R' => Some((0xD7, 0x5F, 0x5F)), // sting
        _ => None,                       // '.' and anything else → transparent
    }
}

/// Build a 16×16 sprite from 16 bitmap rows.
fn build(rows: &[&str; 16]) -> SpriteSpec {
    let mut pixels = Vec::with_capacity(16 * 16);
    for row in rows {
        let mut chars: Vec<char> = row.chars().collect();
        chars.resize(16, '.');
        for &c in chars.iter().take(16) {
            pixels.push(palette(c));
        }
    }
    SpriteSpec {
        width: 16,
        height: 16,
        pixels,
    }
}

// A side-view flying bee (003-visual-render, redesign), facing right and tilted up as if in flight.
// Left→right the anatomy reads: a red stinger, a yellow abdomen banded with vertical black stripes,
// a thorax the wings attach above, and a black head with a white eye and an antenna. Three little
// legs dangle below. The stripes + antenna + wings-over-the-thorax + stinger are what make it read as
// a bee rather than a blob. Everything except the wings (rows 3–6, columns 4–10) is byte-identical
// across the three frames — only the wings flap up → mid → down.

const FRAME_UP: [&str; 16] = [
    "................",
    ".............KK.", // antenna tip
    "............K...", // antenna
    "......WWWW..K...", // wings gathered high
    ".....WWWWW..KK..", // head crown
    "......WWWWWKKKK.",
    ".......WWW.KWWK.", // head + white eye
    "..KKKKKKKKKKKKKK", // body top outline
    ".KRYYKKYYKKYKWKK", // stinger · abdomen stripes · thorax · head
    "KRYYYKKYYKKYKKKK",
    ".KRYYKKYYKKYKKK.",
    "..KKKKKKKKKKKK..", // body bottom outline
    "...K..K..K......", // legs
    "................",
    "................",
    "................",
];

const FRAME_MID: [&str; 16] = [
    "................",
    ".............KK.",
    "............K...",
    ".......WWW..K...", // wings level
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

const FRAME_DOWN: [&str; 16] = [
    "................",
    ".............KK.",
    "............K...",
    "........WW..K...", // wings spread low
    "......WWWW..KK..",
    "....WWWWWWWKKKK.",
    "....WWWWWWWKWWK.",
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

/// The 16×16 bee sprite — the level-wing resting pose.
pub fn sprite() -> SpriteSpec {
    static BEE: OnceLock<SpriteSpec> = OnceLock::new();
    BEE.get_or_init(|| build(&FRAME_MID)).clone()
}

/// The 3-frame wing-flap animation (150 ms/frame, bounce).
pub fn animation() -> AnimationSpec {
    static ANIM: OnceLock<AnimationSpec> = OnceLock::new();
    ANIM.get_or_init(|| AnimationSpec {
        frames: vec![build(&FRAME_UP), build(&FRAME_MID), build(&FRAME_DOWN)],
        interval_ms: 150,
        bounce: true,
        cycles: 1,
    })
    .clone()
}
