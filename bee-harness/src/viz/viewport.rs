//! The live render viewport (008-grid-tui): how much drawable room the active front-end actually has.
//!
//! The `render` tool runs in-process with no idea what the screen looks like, so a script could
//! happily commit a 10-column grid into a 48-column panel and produce something unreadable — or, on a
//! terminal below the hard floor, produce nothing visible at all. The model got a cheerful "Rendered
//! …" either way and had no signal to adapt.
//!
//! The front-end publishes its real regions here on startup and on every resize; the drawing API
//! checks a widget against them at commit time and **fails the script** with an actionable message
//! (see `render_api`). Mutable process state (unlike the immutable [`super::theme`] `OnceLock`)
//! because the terminal resizes underneath a running session.
//!
//! Headless callers (episodes, batch runs, tests) never publish, so the default is
//! [`Viewport::unconstrained`] and **no fit check ever fires** off an interactive front-end.

use std::sync::RwLock;

/// The drawable regions the active front-end offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Viewport {
    /// The whole terminal.
    pub cols: u16,
    pub rows: u16,
    /// Columns available to an **inline** render — the chat pane in full-screen, the terminal width
    /// in the inline REPL.
    pub inline_cols: u16,
    /// Interior width/height of a **panel**, already net of its border. `0` when no panel column
    /// exists (inline REPL, or a layout too narrow for one).
    pub panel_cols: u16,
    pub panel_rows: u16,
    /// How many *additional* panels the column can seat before it overflows.
    pub panel_slots_free: u16,
    /// Ids of the panels currently live — re-rendering one of these needs no free slot.
    pub live_panels: Vec<String>,
    /// True for the full-screen TUI (a real panel column exists); false for the inline REPL, where
    /// panel renders fall back into the chat flow and are never space-constrained.
    pub full_screen: bool,
    /// False when nothing is known about the surface (headless). Fit checks are skipped entirely.
    pub constrained: bool,
}

impl Viewport {
    /// The default: nothing known about the surface, so every render is allowed.
    pub const fn unconstrained() -> Self {
        Viewport {
            cols: 0,
            rows: 0,
            inline_cols: 0,
            panel_cols: 0,
            panel_rows: 0,
            panel_slots_free: 0,
            live_panels: Vec::new(),
            full_screen: false,
            constrained: false,
        }
    }

    /// True when `id` already has a panel (so an update doesn't need a free slot).
    pub fn has_panel(&self, id: &str) -> bool {
        self.live_panels.iter().any(|p| p == id)
    }
}

impl Default for Viewport {
    fn default() -> Self {
        Viewport::unconstrained()
    }
}

static VIEWPORT: RwLock<Viewport> = RwLock::new(Viewport::unconstrained());

/// Publish the current drawable regions. Called by the front-end at startup and on every resize.
pub fn set(v: Viewport) {
    if let Ok(mut g) = VIEWPORT.write() {
        *g = v;
    }
}

/// The current viewport (a clone — callers hold it across validation without keeping the lock).
pub fn get() -> Viewport {
    VIEWPORT
        .read()
        .map(|g| g.clone())
        .unwrap_or_else(|_| Viewport::unconstrained())
}

/// Reset to unconstrained — used by tests so one test's publish can't leak into another.
pub fn reset() {
    set(Viewport::unconstrained());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_unconstrained_so_headless_runs_never_fail_a_render() {
        let v = Viewport::unconstrained();
        assert!(!v.constrained);
        assert!(v.live_panels.is_empty());
    }

    #[test]
    fn has_panel_tracks_live_ids() {
        let mut v = Viewport::unconstrained();
        v.live_panels = vec!["metrics".into()];
        assert!(v.has_panel("metrics"));
        assert!(!v.has_panel("other"));
    }
}
