//! The animation playback seam (003-visual-render, Slice 2, FR-030; research D14). These two
//! functions are **pure** — no tokio, no clock — so the spec-visible behavior (frame ordering,
//! in-place redraw bytes) is tested as data (SC-016), exactly as Slice 1 tests the pure
//! `spinner_frame` apart from its spawned loop. `TerminalOutput::start_animation` is the thin tokio
//! glue that walks `playback` and prints `redraw_block` between `sleep`s.

use crate::render_spec::AnimationSpec;

/// One playback period's frame indices, applying `bounce`. Forward `0,1,…,k`; with bounce, then back
/// `k-1,…,1` (the `1-2-3-2` the spec shows, 0-indexed → `[0,1,2,1]` for 3 frames).
fn period(n: usize, bounce: bool) -> Vec<usize> {
    if n <= 1 {
        return vec![0];
    }
    let mut seq: Vec<usize> = (0..n).collect();
    if bounce {
        seq.extend((1..n - 1).rev());
    }
    seq
}

/// The ordered frame indices for a full playback (research D14): one [`period`] repeated `cycles`
/// times, or a single period when `cycles == 0` (the caller's task then repeats it until
/// interrupted). Examples: 3 frames `bounce=true cycles=1` → `[0,1,2,1]`; `bounce=false cycles=2` →
/// `[0,1,2,0,1,2]`.
pub fn playback(spec: &AnimationSpec) -> Vec<usize> {
    let n = spec.frames.len().max(1);
    let one = period(n, spec.bounce);
    if spec.cycles == 0 {
        return one;
    }
    let mut out = Vec::with_capacity(one.len() * spec.cycles as usize);
    for _ in 0..spec.cycles {
        out.extend_from_slice(&one);
    }
    out
}

/// One in-place redraw of an `N`-row frame: move the cursor up `N` rows, then clear+rewrite each row.
/// Always starts with `\x1b[{N}A` (SC-016 cursor-up). Paired with an initial plain emit of frame 0.
pub fn redraw_block(rows: &[String]) -> String {
    let n = rows.len();
    if n == 0 {
        return String::new();
    }
    let mut s = format!("\x1b[{n}A");
    for row in rows {
        s.push_str(&format!("\r\x1b[2K{row}\n"));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render_spec::SpriteSpec;

    fn anim(frames: usize, bounce: bool, cycles: u32) -> AnimationSpec {
        let f = SpriteSpec { width: 2, height: 2, pixels: vec![None; 4] };
        AnimationSpec { frames: vec![f; frames], interval_ms: 150, bounce, cycles }
    }

    #[test]
    fn playback_bounce_and_cycles() {
        assert_eq!(playback(&anim(3, true, 1)), vec![0, 1, 2, 1]);
        assert_eq!(playback(&anim(3, false, 2)), vec![0, 1, 2, 0, 1, 2]);
        assert_eq!(playback(&anim(3, false, 0)), vec![0, 1, 2]); // one period when infinite
        assert_eq!(playback(&anim(1, true, 3)), vec![0, 0, 0]);
    }

    #[test]
    fn redraw_block_starts_with_cursor_up() {
        let rows = vec!["aa".to_string(), "bb".to_string()];
        let block = redraw_block(&rows);
        assert!(block.starts_with("\x1b[2A"), "got: {block:?}");
        assert!(block.contains("\r\x1b[2Kaa\n"));
        assert!(block.contains("\r\x1b[2Kbb\n"));
        assert_eq!(redraw_block(&[]), "");
    }
}
