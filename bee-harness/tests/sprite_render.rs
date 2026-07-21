//! Half-block sprite rendering (003-visual-render, Slice 2, FR-030; SC-015). Pure functions — no
//! terminal, no timing.

use bee_harness::render_spec::SpriteSpec;
use bee_harness::viz::sprite_render::{quantize_16, quantize_256, render_frame_with, ColorMode};

/// Build a `w×h` sprite from an explicit pixel list.
fn sprite(w: u16, h: u16, pixels: Vec<Option<(u8, u8, u8)>>) -> SpriteSpec {
    assert_eq!(pixels.len(), (w * h) as usize);
    SpriteSpec {
        width: w,
        height: h,
        pixels,
    }
}

#[test]
fn sixteen_by_sixteen_is_eight_truecolor_rows() {
    // SC-015: a 16×16, 4-color sprite renders to 8 rows, each carrying truecolor escapes.
    let colors = [
        Some((0xE5, 0xA1, 0x00)),
        Some((0xFF, 0xFF, 0xFF)),
        None,
        Some((0x1A, 0x1A, 0x1A)),
    ];
    let pixels: Vec<_> = (0..256).map(|i| colors[i % 4]).collect();
    let rows = render_frame_with(&sprite(16, 16, pixels), ColorMode::TrueColor, true);
    assert_eq!(rows.len(), 8, "16 px tall → 8 half-block rows");
    for r in &rows {
        assert!(
            r.contains("\x1b[38;2;") || r.contains("\x1b[48;2;"),
            "row lacks truecolor: {r:?}"
        );
    }
}

#[test]
fn transparent_half_emits_no_background_escape() {
    // SC-015: a transparent half emits NO bg escape for its half of the cell.
    let red = Some((255, 0, 0));
    // top transparent, bottom opaque → lower half block, fg only, no bg.
    let bottom_only = render_frame_with(&sprite(1, 2, vec![None, red]), ColorMode::TrueColor, true);
    assert!(
        bottom_only[0].contains("38;2;255;0;0"),
        "bottom fg missing: {bottom_only:?}"
    );
    assert!(
        !bottom_only[0].contains("48;2;"),
        "should have no bg escape: {bottom_only:?}"
    );
    // top opaque, bottom transparent → upper half block, fg only, no bg.
    let top_only = render_frame_with(&sprite(1, 2, vec![red, None]), ColorMode::TrueColor, true);
    assert!(
        top_only[0].contains("▀"),
        "expected upper half block: {top_only:?}"
    );
    assert!(
        !top_only[0].contains("48;2;"),
        "should have no bg escape: {top_only:?}"
    );
}

#[test]
fn fully_transparent_sprite_is_blank_rows() {
    // Edge case (M1): all-transparent → ⌈h/2⌉ blank rows, no escapes.
    let rows = render_frame_with(&sprite(4, 4, vec![None; 16]), ColorMode::TrueColor, true);
    assert_eq!(rows.len(), 2);
    for r in &rows {
        assert!(
            !r.contains('\u{1b}'),
            "blank row must have no escapes: {r:?}"
        );
        assert!(
            r.chars().all(|c| c == ' '),
            "blank row must be spaces: {r:?}"
        );
    }
}

#[test]
fn no_color_degrades_to_mono_blocks() {
    // Off-tty / NO_COLOR (L3): monochrome block chars, no SGR.
    let px = vec![Some((255, 0, 0)), None, None, Some((0, 255, 0))];
    let rows = render_frame_with(&sprite(2, 2, px), ColorMode::TrueColor, false);
    assert_eq!(rows.len(), 1);
    assert!(
        !rows[0].contains('\u{1b}'),
        "mono must have no escapes: {rows:?}"
    );
    assert!(rows[0].contains('▀') || rows[0].contains('▄') || rows[0].contains('█'));
}

#[test]
fn ansi256_and_ansi16_use_indexed_escapes() {
    let red = Some((255, 0, 0));
    let r256 = render_frame_with(&sprite(1, 2, vec![red, red]), ColorMode::Ansi256, true);
    assert!(
        r256[0].contains("\x1b[38;5;") && r256[0].contains("\x1b[48;5;"),
        "256: {r256:?}"
    );
    let r16 = render_frame_with(&sprite(1, 2, vec![red, red]), ColorMode::Ansi16, true);
    // bright red fg = 91, bg = 101.
    assert!(
        r16[0].contains("\x1b[91m") || r16[0].contains("\x1b[31m"),
        "16 fg: {r16:?}"
    );
}

#[test]
fn quantization_maps_known_colors() {
    assert_eq!(quantize_256(0, 0, 0), 16, "black → cube 16");
    assert_eq!(quantize_256(255, 255, 255), 231, "white → cube 231");
    assert_eq!(quantize_16(255, 0, 0), 9, "pure red → bright red (9)");
    assert_eq!(quantize_16(0, 0, 0), 0, "black → 0");
}
