//! The pure animation seam (003-visual-render, Slice 2, FR-030; SC-016 ordering + cursor-up). No
//! tokio, no clock — the frame order and redraw bytes are tested as data (research D14). The stateful
//! reclaim / spinner-preemption assertions live as unit tests in `repl::terminal` (they need private
//! field access).

use bee_harness::render_spec::{AnimationSpec, SpriteSpec};
use bee_harness::viz::animator::{playback, redraw_block};

fn anim(frames: usize, bounce: bool, cycles: u32) -> AnimationSpec {
    let f = SpriteSpec {
        width: 2,
        height: 2,
        pixels: vec![None; 4],
    };
    AnimationSpec {
        frames: vec![f; frames],
        interval_ms: 150,
        bounce,
        cycles,
    }
}

#[test]
fn playback_applies_bounce_and_cycles() {
    // SC-016 ordering.
    assert_eq!(playback(&anim(3, true, 1)), vec![0, 1, 2, 1]); // the spec's 1-2-3-2, 0-indexed
    assert_eq!(playback(&anim(3, false, 2)), vec![0, 1, 2, 0, 1, 2]);
    assert_eq!(playback(&anim(3, false, 0)), vec![0, 1, 2]); // cycles=0 → one period
}

#[test]
fn redraw_block_starts_with_cursor_up() {
    // SC-016 cursor-up: an N-row redraw begins by moving the cursor up N rows.
    let rows = vec!["xx".to_string(), "yy".to_string(), "zz".to_string()];
    let block = redraw_block(&rows);
    assert!(
        block.starts_with("\x1b[3A"),
        "3-row block should move up 3: {block:?}"
    );
    assert!(
        block.contains("\r\x1b[2Kyy\n"),
        "each row is cleared + rewritten: {block:?}"
    );
}
