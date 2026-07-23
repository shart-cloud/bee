//! The full-screen takeover overlay (009-tachyonfx-effects, US3).
//!
//! An overlay is the most intrusive thing the agent can do to the screen, so every property here is
//! about making it safe to enable: it covers the chat area and **never the input line** (FR-014), it
//! always carries a visible way out (FR-015), it always expires on its own (FR-017), and there is
//! never more than one (FR-020).
//!
//! ## Lifecycle
//!
//! `Entering` → `Showing` → `Dismissing` → gone. The three phases exist because they cost different
//! amounts: the two animated ones run at 60fps, while `Showing` needs only 1Hz — enough to tick the
//! countdown and fire the TTL. Holding 60fps for a two-minute overlay would spend ~7200 redraws
//! animating 120 integers.
//!
//! The data-model sketched this as a `dismissing` flag. It is an enum instead, because the scheduler
//! has to tell `Entering` from `Showing` — with a flag those two states are indistinguishable, and
//! the render loop would have to guess which one it is in.
//!
//! ## Why the TTL is resolved once
//!
//! At construction, never re-read. A config reload mid-session cannot extend an overlay that is
//! already on screen — the lifetime the operator agreed to when it appeared is the one it gets.

use std::time::{Duration, Instant};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use super::effects::{self, Effects, ResolveCtx, OVERLAY_FADE_MS};
use crate::config::VisualConfig;
use crate::render_spec::{EffectSpec, RenderSpec};
use crate::visual_gate;

/// The effect key every overlay transition is registered under. A reserved key, so an agent panel
/// named anything at all cannot cancel or replace an overlay's animation.
const OVERLAY_KEY: &str = "\u{0}overlay";

/// Where an overlay is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Fading in. 60fps.
    Entering,
    /// On screen, counting down. **1Hz** — the state that justifies FR-002 having three.
    Showing,
    /// Fading out, for any of the four dismiss triggers. 60fps.
    Dismissing,
}

/// One full-screen takeover.
pub struct Overlay {
    /// What the agent asked to show.
    pub spec: RenderSpec,
    /// When the overlay appeared. The TTL counts from here, through every phase.
    pub created: Instant,
    /// The lifetime actually granted: `min(requested, configured)`, resolved once (FR-021).
    pub ttl: Duration,
    phase: Phase,
    /// When the current phase began.
    phase_since: Instant,
    /// Entrance / exit durations. Both zero when animations are off, so the overlay appears and
    /// vanishes on the next redraw — but `Showing` still ticks, because the countdown is
    /// information, not motion (FR-015).
    fade: Duration,
    /// The overlay that replaces this one once its fade-out finishes (FR-020). Attached to the
    /// overlay being replaced rather than kept beside it, so "at most one on screen" stays a
    /// property of `App`'s single `Option` rather than a rule to remember.
    next: Option<Box<Overlay>>,
    /// The last frame this overlay drew, kept so the fade-out can erode *it* while the restored
    /// chat renders underneath.
    snapshot: Option<Buffer>,
    /// Set when the operator submits a message while this overlay is up. The model's reply then
    /// dismisses it (FR-019) — submitting alone does not (US3 §6).
    superseded: bool,
}

impl std::fmt::Debug for Overlay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Overlay")
            .field("phase", &self.phase)
            .field("ttl", &self.ttl)
            .field("replaced_by", &self.next.is_some())
            .finish()
    }
}

impl Overlay {
    /// Create an overlay for `spec`, resolving its lifetime against the operator's configuration.
    ///
    /// `requested_ms` is what the script asked for; the granted lifetime is
    /// `min(requested, configured)` and is fixed from here on (FR-021,
    /// contracts/overlay-lifecycle.md).
    pub fn new(
        spec: RenderSpec,
        requested_ms: Option<u32>,
        visual: &VisualConfig,
        now: Instant,
    ) -> Self {
        let ttl = Duration::from_millis(u64::from(visual_gate::resolve_ttl(requested_ms, visual)));
        let fade = if visual.animations {
            Duration::from_millis(u64::from(OVERLAY_FADE_MS))
        } else {
            Duration::ZERO
        };
        Overlay {
            spec,
            created: now,
            ttl,
            phase: Phase::Entering,
            phase_since: now,
            fade,
            next: None,
            snapshot: None,
            superseded: false,
        }
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// How long until the TTL fires. Counts from creation, so a slow entrance does not buy the
    /// overlay extra time on screen.
    pub fn remaining(&self, now: Instant) -> Duration {
        self.ttl
            .saturating_sub(now.saturating_duration_since(self.created))
    }

    /// The operator's way out, rendered in the overlay's bottom row (FR-015).
    ///
    /// The countdown rounds **up**, so it reads "1s" for the whole final second rather than
    /// flashing "0s" at something that is still on screen.
    pub fn hint(&self, now: Instant) -> String {
        let secs = self.remaining(now).as_secs_f64().ceil() as u64;
        format!("Esc to dismiss · auto-dismiss in {secs}s")
    }

    /// Begin the fade-out, from whatever is currently on screen.
    ///
    /// Dismissing mid-`Entering` is not a special case: the exit effect erodes the overlay's last
    /// drawn frame, which is the partially-faded one, so the entrance is simply abandoned where it
    /// stood (spec Edge Cases). Dismissing an already-dismissing overlay is a no-op — the operator
    /// mashing Esc must not restart the fade.
    pub fn dismiss(&mut self, now: Instant) {
        if self.phase != Phase::Dismissing {
            self.phase = Phase::Dismissing;
            self.phase_since = now;
        }
    }

    /// Queue `next` to take over once this overlay's fade-out finishes (FR-020).
    ///
    /// The cross-fade is sequential by construction: the replacement's `Entering` cannot start
    /// before this one's `Dismissing` ends, so two overlays never draw in the same frame. A second
    /// replacement arriving during the same fade simply supersedes the first — the queue is one
    /// deep, not a backlog.
    pub fn replace_with(&mut self, next: Overlay, now: Instant) {
        self.next = Some(Box::new(next));
        self.dismiss(now);
    }

    /// Note that the conversation has moved on: the operator submitted a message while this overlay
    /// was up. The model's reply will dismiss it (FR-019).
    pub fn mark_superseded(&mut self) {
        self.superseded = true;
    }

    /// Whether the model's next reply should dismiss this overlay.
    ///
    /// Only after the operator has spoken. An overlay is usually created *by* a tool call inside a
    /// turn whose prose has not been written yet; dismissing on that prose would make every
    /// takeover vanish the instant the model finished explaining it.
    pub fn superseded(&self) -> bool {
        self.superseded
    }

    /// Advance the state machine. Returns the overlay that should be on screen afterward: `self`,
    /// its queued replacement, or `None` when the fade-out has finished and nothing follows.
    /// Each transition is stamped with the instant it *actually* happened, not with `now`, so the
    /// machine converges to whatever `now` implies rather than advancing one phase per call. A loop
    /// descheduled for a second must not leave an overlay that should already be gone on screen.
    pub fn advance(mut self, now: Instant) -> Option<Overlay> {
        loop {
            let done_at = match self.phase {
                Phase::Entering => self.phase_since + self.fade,
                // The TTL counts from creation, so a slow entrance buys no extra time on screen.
                Phase::Showing => self.created + self.ttl,
                Phase::Dismissing => self.phase_since + self.fade,
            };
            if now < done_at {
                return Some(self);
            }
            match self.phase {
                Phase::Entering => {
                    self.phase = Phase::Showing;
                    self.phase_since = done_at;
                }
                Phase::Showing => {
                    self.phase = Phase::Dismissing;
                    self.phase_since = done_at;
                }
                Phase::Dismissing => {
                    return match self.next.take() {
                        // The replacement's clock starts when it reaches the screen, not when it was
                        // requested — its TTL is time on screen, and it has had none yet.
                        Some(next) => {
                            let mut next = *next;
                            next.created = done_at;
                            next.phase_since = done_at;
                            self = next;
                            // Keep looping: with animations off the newcomer's entrance is
                            // zero-length, so it must reach `Showing` in this same call.
                            continue;
                        }
                        None => None,
                    };
                }
            }
        }
    }

    /// Register this overlay's transition for the phase it is in, if it owes one.
    ///
    /// Chrome origin, not agent: the overlay's own fade is bee's UI drawing the operator's attention
    /// to a change of surface, and `visual_level` has already had its say — an overlay only exists
    /// at all because the level admitted it (FR-006d).
    pub fn register(&mut self, effects: &mut Effects, area: Rect, visual: &VisualConfig) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let ctx = ResolveCtx::chrome(*visual, area);
        match self.phase {
            Phase::Entering => {
                if let Some(fx) = effects::resolve(
                    &EffectSpec::FadeIn {
                        ms: OVERLAY_FADE_MS,
                    },
                    &ctx,
                ) {
                    effects.add_keyed(OVERLAY_KEY, fx);
                }
            }
            // The exit erodes the overlay's own last frame while the restored chat draws
            // underneath, so the chat comes back through the gaps rather than appearing all at once
            // when the fade ends.
            Phase::Dismissing => {
                if let Some(prev) = self.snapshot.clone() {
                    if let Some(fx) = effects::erode(prev, &ctx) {
                        effects.add_keyed(OVERLAY_KEY, fx);
                    }
                }
            }
            Phase::Showing => {}
        }
    }

    /// Drop everything tied to the old geometry after a resize (spec Edge Cases).
    ///
    /// A running transition is pinned to the `Rect` it was registered with and the snapshot was
    /// captured at the old size, so both are meaningless now. Cancelling rather than rescaling means
    /// the next frame simply draws final content at the new geometry.
    pub fn invalidate(&mut self, effects: &mut Effects) {
        effects.cancel(OVERLAY_KEY);
        self.snapshot = None;
    }

    /// Keep this frame's rendering as the source for a later fade-out.
    pub fn snapshot(&mut self, area: Rect, buf: &Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        self.snapshot = Some(effects::capture(area, buf));
    }

    /// Whether the overlay still draws its content this frame. A dismissing overlay does not — the
    /// chat renders instead, and the fade-out effect paints the departing overlay over it.
    pub fn draws_content(&self) -> bool {
        self.phase != Phase::Dismissing
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::VisualConfig;

    fn spec() -> RenderSpec {
        RenderSpec::Text {
            content: "chart".into(),
            style: None,
            bold: false,
            dim: false,
        }
    }

    fn cfg(ttl_secs: u32) -> VisualConfig {
        VisualConfig {
            takeover_ttl_secs: ttl_secs,
            ..VisualConfig::default()
        }
    }

    fn motionless(ttl_secs: u32) -> VisualConfig {
        VisualConfig {
            animations: false,
            ..cfg(ttl_secs)
        }
    }

    /// Drive the state machine to `now`, returning whatever should be on screen.
    fn at(o: Overlay, now: Instant) -> Option<Overlay> {
        o.advance(now)
    }

    #[test]
    fn a_longer_request_than_the_config_allows_is_clamped_silently() {
        // FR-021 / US2 §6: 90s asked against a 20s ceiling yields 20s, with no error.
        let t0 = Instant::now();
        let o = Overlay::new(spec(), Some(90_000), &cfg(20), t0);
        assert_eq!(o.ttl, Duration::from_secs(20));
    }

    #[test]
    fn a_shorter_request_is_honored_and_no_request_takes_the_configured_lifetime() {
        let t0 = Instant::now();
        assert_eq!(
            Overlay::new(spec(), Some(5_000), &cfg(30), t0).ttl,
            Duration::from_secs(5)
        );
        assert_eq!(
            Overlay::new(spec(), None, &cfg(30), t0).ttl,
            Duration::from_secs(30)
        );
    }

    #[test]
    fn the_ttl_is_fixed_at_construction_so_a_later_config_change_cannot_extend_it() {
        let t0 = Instant::now();
        let o = Overlay::new(spec(), None, &cfg(20), t0);
        // Whatever the operator does to their config now, this overlay's lifetime is settled.
        assert_eq!(o.ttl, Duration::from_secs(20));
        assert_eq!(
            o.remaining(t0 + Duration::from_secs(5)),
            Duration::from_secs(15)
        );
        assert!(o.remaining(t0 + Duration::from_secs(60)).is_zero());
    }

    #[test]
    fn the_phases_run_in_order_and_the_overlay_expires_on_its_own() {
        let t0 = Instant::now();
        let mut o = Overlay::new(spec(), Some(2_000), &cfg(30), t0);
        assert_eq!(o.phase(), Phase::Entering);

        o = at(o, t0 + Duration::from_millis(50)).expect("still entering");
        assert_eq!(o.phase(), Phase::Entering, "the entrance takes 200ms");

        o = at(o, t0 + Duration::from_millis(250)).expect("now showing");
        assert_eq!(o.phase(), Phase::Showing);

        o = at(o, t0 + Duration::from_millis(1_900)).expect("still showing");
        assert_eq!(o.phase(), Phase::Showing);

        // SC-007: the TTL fires, then the fade-out runs, then it is gone.
        o = at(o, t0 + Duration::from_millis(2_000)).expect("dismissing");
        assert_eq!(o.phase(), Phase::Dismissing);
        assert!(
            at(o, t0 + Duration::from_millis(2_200)).is_none(),
            "gone by ttl + 200ms"
        );
    }

    #[test]
    fn dismissing_mid_entrance_abandons_it_rather_than_waiting_it_out() {
        // Spec edge case: the fade-out runs from the current buffer state, so an operator who hits
        // Esc during the entrance does not have to watch the entrance finish first.
        let t0 = Instant::now();
        let mut o = Overlay::new(spec(), None, &cfg(30), t0);
        o.dismiss(t0 + Duration::from_millis(80));
        assert_eq!(o.phase(), Phase::Dismissing);
        assert!(at(o, t0 + Duration::from_millis(280)).is_none());
    }

    #[test]
    fn mashing_escape_does_not_restart_the_fade_out() {
        let t0 = Instant::now();
        let mut o = Overlay::new(spec(), None, &cfg(30), t0);
        o.dismiss(t0);
        o.dismiss(t0 + Duration::from_millis(150));
        assert!(
            at(o, t0 + Duration::from_millis(200)).is_none(),
            "the second Esc must not buy another 200ms"
        );
    }

    #[test]
    fn a_replacement_starts_only_once_the_first_has_finished_fading_out() {
        // FR-020: cross-fade, never stacked — and never two overlays in one frame.
        let t0 = Instant::now();
        let mut o = Overlay::new(spec(), None, &cfg(30), t0);
        o = at(o, t0 + Duration::from_millis(250)).expect("showing");

        let second = Overlay::new(spec(), Some(9_000), &cfg(30), t0 + Duration::from_secs(1));
        o.replace_with(second, t0 + Duration::from_secs(1));
        assert_eq!(o.phase(), Phase::Dismissing, "the first one leaves first");

        o = at(o, t0 + Duration::from_millis(1_100)).expect("still fading out");
        assert_eq!(o.phase(), Phase::Dismissing);

        let o = at(o, t0 + Duration::from_millis(1_200)).expect("the replacement took over");
        assert_eq!(o.phase(), Phase::Entering, "the new one starts fresh");
        assert_eq!(o.ttl, Duration::from_secs(9));
    }

    #[test]
    fn a_replacement_arriving_during_a_fade_supersedes_the_one_already_queued() {
        // The queue is one deep. A model spamming takeovers must not build a backlog the operator
        // then has to sit through.
        let t0 = Instant::now();
        let mut o = Overlay::new(spec(), None, &cfg(30), t0);
        o.replace_with(Overlay::new(spec(), Some(1_000), &cfg(30), t0), t0);
        o.replace_with(Overlay::new(spec(), Some(7_000), &cfg(30), t0), t0);
        let o = at(o, t0 + Duration::from_millis(200)).expect("a replacement took over");
        assert_eq!(o.ttl, Duration::from_secs(7), "the last request wins");
    }

    #[test]
    fn the_replacements_clock_starts_when_it_reaches_the_screen() {
        // Its TTL is time *on screen*; the fade-out it waited through is not its own lifetime.
        let t0 = Instant::now();
        let mut o = Overlay::new(spec(), None, &cfg(30), t0);
        o.replace_with(Overlay::new(spec(), Some(5_000), &cfg(30), t0), t0);
        let o = at(o, t0 + Duration::from_millis(200)).expect("replacement");
        assert_eq!(
            o.remaining(t0 + Duration::from_millis(200)),
            Duration::from_secs(5)
        );
    }

    #[test]
    fn the_hint_counts_down_once_per_second_and_rounds_up() {
        // FR-015. Rounding up means the final second reads "1s" rather than flashing "0s" at
        // something still on screen.
        let t0 = Instant::now();
        let o = Overlay::new(spec(), Some(10_000), &cfg(30), t0);
        assert!(o.hint(t0).contains("Esc to dismiss"));
        assert!(o.hint(t0).contains("in 10s"), "{}", o.hint(t0));
        assert!(
            o.hint(t0 + Duration::from_millis(500)).contains("in 10s"),
            "half a second in still rounds up to 10"
        );
        assert!(o.hint(t0 + Duration::from_secs(1)).contains("in 9s"));
        assert!(o.hint(t0 + Duration::from_secs(9)).contains("in 1s"));
        assert!(o.hint(t0 + Duration::from_secs(10)).contains("in 0s"));
    }

    #[test]
    fn the_countdown_still_advances_with_animations_disabled() {
        // FR-015: the countdown is information, not motion. Only the fades go away.
        let t0 = Instant::now();
        let mut o = Overlay::new(spec(), Some(10_000), &motionless(30), t0);
        assert!(o.hint(t0 + Duration::from_secs(3)).contains("in 7s"));

        // And with no fades, the phases are instant rather than skipped.
        o = at(o, t0).expect("showing immediately");
        assert_eq!(o.phase(), Phase::Showing, "no entrance to sit through");
        o.dismiss(t0 + Duration::from_secs(1));
        assert!(
            at(o, t0 + Duration::from_secs(1)).is_none(),
            "and no fade-out either"
        );
    }

    #[test]
    fn the_model_reply_dismisses_only_after_the_operator_has_spoken() {
        // FR-019 vs US3 §6: submitting keeps the overlay; the model's *reply* to that submission is
        // what ends it. Without this an overlay would vanish the moment the model finished the
        // sentence that introduced it.
        let t0 = Instant::now();
        let mut o = Overlay::new(spec(), None, &cfg(30), t0);
        assert!(!o.superseded(), "same-turn prose must not dismiss it");
        o.mark_superseded();
        assert!(o.superseded());
    }

    #[test]
    fn a_zero_area_overlay_registers_nothing_and_does_not_panic() {
        // Spec edge case: a terminal too small to give the overlay any room is a no-op, not a crash.
        let t0 = Instant::now();
        let mut o = Overlay::new(spec(), None, &cfg(30), t0);
        let mut fx = Effects::new();
        o.register(&mut fx, Rect::new(0, 0, 0, 0), &VisualConfig::default());
        o.snapshot(
            Rect::new(0, 0, 10, 0),
            &Buffer::empty(Rect::new(0, 0, 10, 1)),
        );
        assert!(!fx.is_running());
    }

    #[test]
    fn the_overlay_effect_key_is_one_no_panel_id_can_spell() {
        // FR-028's shape: panel ids are `[a-z0-9_-]`, so a NUL-prefixed key is unreachable from the
        // agent — it cannot cancel or hijack an overlay transition by naming a panel.
        assert!(OVERLAY_KEY.starts_with('\u{0}'));
    }
}
