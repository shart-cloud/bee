//! The effects pipeline (009-tachyonfx-effects): resolve [`EffectSpec`] data into live tachyonfx
//! effects, own them, and advance them each frame.
//!
//! **This module is the single chokepoint** every effect passes through (research R5). That matters
//! for FR-006c: when animations are disabled, [`resolve`] returns `None` for everything, nothing is
//! ever registered, [`Effects::is_running`] is permanently false, and the render loop therefore
//! *cannot* enter its 60fps state. Suppression is structural rather than a rule each of a dozen call
//! sites has to remember.
//!
//! Effects are applied to the ratatui [`Buffer`] **after** widgets render and **before** the buffer
//! is flushed (FR-001) — they transform already-drawn cells, they do not draw.
//!
//! Determinism: tachyonfx carries its own seeded `SimpleRng` and has no `rand` dependency, so the
//! character-scatter effects reproduce for a given area and timeline. That is what makes the
//! snapshot assertions below meaningful rather than flaky.

use std::time::Duration;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use tachyonfx::{fx, Effect, EffectManager, Motion};

use crate::config::{VisualConfig, VisualLevel};
use crate::render_spec::{EffectDirection, EffectSpec};
use crate::viz::buffer_render::theme_to_ratatui_color;
use crate::viz::palette;
use crate::viz::theme::{self, Role};

/// How wide the leading gradient is on slide/sweep effects, in cells. Not agent-tunable: the Rhai
/// surface exposes direction and duration only, and this is a house value chosen so the wave reads
/// as a wave at typical panel widths (contracts/rhai-effect-api.md).
const GRADIENT_LEN: u16 = 8;
/// Per-cell timing jitter on slide/sweep. A little randomness stops the edge looking like a ruler.
const RANDOMNESS: u16 = 15;
/// How far [`EffectSpec::Glow`] lightens before returning.
const GLOW_AMOUNT: f32 = 0.4;

/// Default entrance transition for a newly created panel (FR-003).
pub const PANEL_ENTER_MS: u32 = 300;
/// Default transition when a panel's content is replaced (FR-004).
pub const PANEL_UPDATE_MS: u32 = 400;
/// Overlay entrance / exit fade (FR-018).
pub const OVERLAY_FADE_MS: u32 = 200;

/// Where an effect came from. Only agent-originated effects are gated by [`VisualLevel`]: harness
/// chrome is bee's own UI, and `visual_level` governs the agent (FR-006d).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// Requested by a model script, or applied automatically to a model-owned panel.
    Agent,
    /// bee's own UI transitions (FR-026). Not overridable by the agent, and not restricted by level.
    Chrome,
}

/// Everything [`resolve`] needs to decide whether — and how — an effect should exist.
#[derive(Debug, Clone, Copy)]
pub struct ResolveCtx {
    /// The three presentation axes.
    pub visual: VisualConfig,
    /// The region the effect will play in. A zero-area region is a no-op rather than a crash.
    pub area: Rect,
    pub origin: Origin,
    /// Whether ANSI color may be emitted. Threaded rather than read from the environment so tests
    /// can exercise both states without mutating process-global state.
    pub color: bool,
}

impl ResolveCtx {
    /// A context for an agent-originated effect under the current process color setting.
    pub fn agent(visual: VisualConfig, area: Rect) -> Self {
        ResolveCtx {
            visual,
            area,
            origin: Origin::Agent,
            color: palette::is_color_enabled(),
        }
    }

    /// A context for a harness-chrome effect (FR-026).
    pub fn chrome(visual: VisualConfig, area: Rect) -> Self {
        ResolveCtx {
            origin: Origin::Chrome,
            ..Self::agent(visual, area)
        }
    }
}

/// Resolve a declarative [`EffectSpec`] into a live tachyonfx effect.
///
/// `None` means **"render the final content now, register nothing"** — never "fail". An effect is
/// decoration; a tool call must not break because the operator turned animations off. The four
/// suppression conditions (contracts/motion-control.md):
///
/// | Condition | Suppresses |
/// |---|---|
/// | animations disabled | every variant, every origin (FR-006b) |
/// | `NO_COLOR` | the four color variants only (FR-006) |
/// | zero-area region | every variant (spec Edge Cases) |
/// | `visual_level = none` | every **agent** variant (FR-024) — chrome still plays |
pub fn resolve(spec: &EffectSpec, ctx: &ResolveCtx) -> Option<Effect> {
    if !ctx.visual.animations {
        return None;
    }
    if ctx.area.width == 0 || ctx.area.height == 0 {
        return None;
    }
    if ctx.origin == Origin::Agent && ctx.visual.level == VisualLevel::None {
        return None;
    }
    // Color effects have nothing to say on a monochrome terminal, but text effects keep their
    // motion — a NO_COLOR user has not asked for a still screen (FR-006).
    if spec.is_color() && !ctx.color {
        return None;
    }
    Some(build(spec, ctx))
}

/// Map a resolved spec to its tachyonfx construction (research R3).
///
/// Ten of the twelve are direct calls. Two are composites, because tachyonfx has no such primitive:
/// [`EffectSpec::Pulse`] is a there-and-back fade pair, and [`EffectSpec::Glow`] is a ping-ponged
/// lighten. One inverts naming: bee's `dissolve_in` is tachyonfx's `coalesce` (characters
/// *reforming*), because bee names all twelve by their `_in`/`_out` sense.
fn build(spec: &EffectSpec, ctx: &ResolveCtx) -> Effect {
    // `EffectTimer: From<u32>` reads the value as milliseconds, which is exactly what an
    // `EffectSpec` carries — no Duration round-trip needed.
    let ms = |m: u32| m;
    let bg = role_color(Role::Info);
    match spec {
        EffectSpec::FadeIn { ms: m } => fx::fade_from(bg, bg, ms(*m)),
        EffectSpec::FadeOut { ms: m } => fx::fade_to(bg, bg, ms(*m)),
        // Naming inverts against tachyonfx: "in" means the content arrives.
        EffectSpec::DissolveIn { ms: m } => fx::coalesce(ms(*m)),
        EffectSpec::DissolveOut { ms: m } => fx::dissolve(ms(*m)),
        EffectSpec::SlideIn { direction, ms: m } => {
            fx::slide_in(motion(*direction), GRADIENT_LEN, RANDOMNESS, bg, ms(*m))
        }
        EffectSpec::SlideOut { direction, ms: m } => {
            fx::slide_out(motion(*direction), GRADIENT_LEN, RANDOMNESS, bg, ms(*m))
        }
        EffectSpec::SweepIn { direction, ms: m } => {
            fx::sweep_in(motion(*direction), GRADIENT_LEN, RANDOMNESS, bg, ms(*m))
        }
        EffectSpec::SweepOut { direction, ms: m } => {
            fx::sweep_out(motion(*direction), GRADIENT_LEN, RANDOMNESS, bg, ms(*m))
        }
        // Composite: flash toward the color, then back. Half the budget each way.
        EffectSpec::Pulse { color, ms: m } => {
            let c = named_color(color);
            let half = ms((*m).max(2) / 2);
            fx::sequence(&[fx::fade_to_fg(c, half), fx::fade_from_fg(c, half)])
        }
        // Composite: lighten and return, which is what "breathing" means here.
        EffectSpec::Glow { ms: m } => fx::ping_pong(fx::lighten(
            Some(GLOW_AMOUNT),
            Some(GLOW_AMOUNT),
            ms((*m).max(2) / 2),
        )),
        EffectSpec::EvolveIn { ms: m } => {
            fx::evolve_into(fx::EvolveSymbolSet::BlocksHorizontal, ms(*m))
        }
        EffectSpec::EvolveOut { ms: m } => {
            fx::evolve_from(fx::EvolveSymbolSet::BlocksHorizontal, ms(*m))
        }
    }
    .with_area(ctx.area)
}

/// The default entrance for a newly created panel (FR-003): a fade from the `info` role.
pub fn panel_enter_spec() -> EffectSpec {
    EffectSpec::FadeIn { ms: PANEL_ENTER_MS }
}

/// The default transition when a panel's content is replaced (FR-004): the outgoing content
/// dissolves while the incoming content coalesces, in parallel.
///
/// Returned as a pair rather than one spec because the two halves target different buffers — the
/// snapshot of the old content and the freshly rendered new content.
pub fn panel_update_specs() -> (EffectSpec, EffectSpec) {
    (
        EffectSpec::DissolveOut {
            ms: PANEL_UPDATE_MS,
        },
        EffectSpec::DissolveIn {
            ms: PANEL_UPDATE_MS,
        },
    )
}

fn motion(d: EffectDirection) -> Motion {
    match d {
        EffectDirection::Left => Motion::LeftToRight,
        EffectDirection::Right => Motion::RightToLeft,
        EffectDirection::Top => Motion::UpToDown,
        EffectDirection::Bottom => Motion::DownToUp,
    }
}

fn role_color(role: Role) -> Color {
    theme_to_ratatui_color(theme::active_theme().get(role))
}

/// Resolve a `pulse` color string against the active theme. Unknown names fall back to the `info`
/// role — resolved *here* rather than at construction, because which palette entries exist depends
/// on the theme active at render time (005-themes).
fn named_color(name: &str) -> Color {
    let t = theme::active_theme();
    theme_to_ratatui_color(&theme::resolve_color_name(t, name))
}

/// Owns the live effects and advances them (FR-001, research R4).
///
/// Keyed by `String`, which is the panel name for panel transitions and a reserved key for the
/// overlay. tachyonfx's `unique(key, fx)` cancels any running effect sharing a key, which *is*
/// FR-005 — a second update to a panel mid-transition replaces the first rather than stacking on it.
/// Chrome effects are added unkeyed, so no agent panel name can cancel or replace them (FR-028).
#[derive(Default)]
pub struct Effects {
    manager: EffectManager<String>,
}

impl std::fmt::Debug for Effects {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Effects")
            .field("running", &self.is_running())
            .finish()
    }
}

impl Effects {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a keyed effect, cancelling any running effect with the same key (FR-005).
    pub fn add_keyed(&mut self, key: impl Into<String>, effect: Effect) {
        let fx = self.manager.unique(key.into(), effect);
        self.manager.add_effect(fx);
    }

    /// Register an unkeyed effect — used for harness chrome, which the agent cannot address and
    /// therefore cannot cancel (FR-028).
    pub fn add(&mut self, effect: Effect) {
        self.manager.add_effect(effect);
    }

    /// Cancel a keyed effect if one is running.
    pub fn cancel(&mut self, key: impl Into<String>) {
        self.manager.cancel_unique_effect(key.into());
    }

    /// Whether any effect is still active. Drives the render loop's motion state (FR-002 state 1);
    /// when animations are disabled this is always false, because nothing is ever registered.
    pub fn is_running(&self) -> bool {
        self.manager.is_running()
    }

    /// Advance every active effect by `dt` and apply it to `buf` (FR-001). Called after widgets
    /// render, before the buffer is flushed.
    pub fn process(&mut self, dt: Duration, buf: &mut Buffer, area: Rect) {
        // tachyonfx carries its own compact `Duration`; convert at the boundary so callers
        // keep speaking std.
        self.manager.process_effects(dt.into(), buf, area);
    }
}

/// Convenience: resolve and register in one step, honoring every suppression rule. Returns whether
/// an effect was actually registered — `false` means the caller should show final content
/// immediately, which is the correct behavior in every suppressed case.
pub fn apply(
    effects: &mut Effects,
    key: Option<&str>,
    spec: &EffectSpec,
    ctx: &ResolveCtx,
) -> bool {
    match resolve(spec, ctx) {
        Some(fx) => {
            match key {
                Some(k) => effects.add_keyed(k, fx),
                None => effects.add(fx),
            }
            true
        }
        None => false,
    }
}

/// A style helper for effects that need one (`stretch`, `expand`).
pub fn effect_style() -> Style {
    Style::default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::VisualConfig;

    fn area() -> Rect {
        Rect::new(0, 0, 20, 5)
    }

    fn ctx(visual: VisualConfig, color: bool, origin: Origin) -> ResolveCtx {
        ResolveCtx {
            visual,
            area: area(),
            origin,
            color,
        }
    }

    fn on() -> VisualConfig {
        VisualConfig::default()
    }

    fn motionless() -> VisualConfig {
        VisualConfig {
            animations: false,
            ..VisualConfig::default()
        }
    }

    fn all_specs() -> Vec<EffectSpec> {
        vec![
            EffectSpec::FadeIn { ms: 300 },
            EffectSpec::FadeOut { ms: 300 },
            EffectSpec::DissolveIn { ms: 300 },
            EffectSpec::DissolveOut { ms: 300 },
            EffectSpec::SlideIn {
                direction: EffectDirection::Left,
                ms: 400,
            },
            EffectSpec::SlideOut {
                direction: EffectDirection::Right,
                ms: 400,
            },
            EffectSpec::SweepIn {
                direction: EffectDirection::Top,
                ms: 400,
            },
            EffectSpec::SweepOut {
                direction: EffectDirection::Bottom,
                ms: 400,
            },
            EffectSpec::Pulse {
                color: "error".into(),
                ms: 200,
            },
            EffectSpec::Glow { ms: 500 },
            EffectSpec::EvolveIn { ms: 600 },
            EffectSpec::EvolveOut { ms: 600 },
        ]
    }

    #[test]
    fn every_variant_resolves_when_nothing_suppresses_it() {
        let c = ctx(on(), true, Origin::Agent);
        for spec in all_specs() {
            assert!(resolve(&spec, &c).is_some(), "{spec:?} failed to resolve");
        }
    }

    #[test]
    fn disabling_animations_suppresses_everything_including_chrome() {
        // FR-006b: the kill switch reaches every origin. This is what makes FR-006c structural —
        // nothing registered means `is_running()` is permanently false.
        for origin in [Origin::Agent, Origin::Chrome] {
            let c = ctx(motionless(), true, origin);
            for spec in all_specs() {
                assert!(
                    resolve(&spec, &c).is_none(),
                    "{spec:?} survived the kill switch at {origin:?}"
                );
            }
        }
    }

    #[test]
    fn no_color_suppresses_color_effects_but_keeps_text_motion() {
        // FR-006: a monochrome terminal is not a request for a still screen.
        let c = ctx(on(), false, Origin::Agent);
        for spec in all_specs() {
            let resolved = resolve(&spec, &c).is_some();
            assert_eq!(
                resolved,
                !spec.is_color(),
                "{spec:?} misbehaved under NO_COLOR"
            );
        }
    }

    #[test]
    fn a_zero_area_region_is_a_no_op_not_a_crash() {
        for area in [
            Rect::new(0, 0, 0, 5),
            Rect::new(0, 0, 20, 0),
            Rect::new(0, 0, 0, 0),
        ] {
            let c = ResolveCtx {
                area,
                ..ctx(on(), true, Origin::Agent)
            };
            for spec in all_specs() {
                assert!(resolve(&spec, &c).is_none(), "{spec:?} at {area:?}");
            }
        }
    }

    #[test]
    fn visual_level_none_strips_agent_effects_but_not_chrome() {
        // FR-006d / FR-024: the level governs the agent; chrome is bee's own UI.
        let level_none = VisualConfig {
            level: VisualLevel::None,
            ..VisualConfig::default()
        };
        let agent = ctx(level_none, true, Origin::Agent);
        let chrome = ctx(level_none, true, Origin::Chrome);
        for spec in all_specs() {
            assert!(resolve(&spec, &agent).is_none(), "agent {spec:?}");
            assert!(resolve(&spec, &chrome).is_some(), "chrome {spec:?}");
        }
    }

    #[test]
    fn a_keyed_effect_cancels_the_previous_one_with_the_same_key() {
        // FR-005: the panel name is the key, so a second update mid-transition replaces the first
        // rather than stacking a second animation on top of it.
        let mut e = Effects::new();
        let c = ctx(on(), true, Origin::Agent);
        let mut buf = Buffer::empty(area());

        assert!(apply(&mut e, Some("metrics"), &all_specs()[0], &c));
        e.process(Duration::from_millis(16), &mut buf, area());
        assert!(e.is_running());

        assert!(apply(&mut e, Some("metrics"), &all_specs()[2], &c));
        // Drain: with the first effect cancelled, only the second's budget remains. If both were
        // live this would still be running well past a single effect's duration.
        for _ in 0..80 {
            e.process(Duration::from_millis(16), &mut buf, area());
        }
        assert!(!e.is_running(), "cancelled effect kept the loop alive");
    }

    #[test]
    fn different_keys_do_not_interfere() {
        let mut e = Effects::new();
        let c = ctx(on(), true, Origin::Agent);
        let mut buf = Buffer::empty(area());
        apply(&mut e, Some("a"), &all_specs()[0], &c);
        apply(&mut e, Some("b"), &all_specs()[1], &c);
        e.process(Duration::from_millis(16), &mut buf, area());
        assert!(e.is_running());
        e.cancel("a");
        e.process(Duration::from_millis(16), &mut buf, area());
        assert!(e.is_running(), "cancelling 'a' must not cancel 'b'");
    }

    #[test]
    fn nothing_registers_when_motion_is_off_so_the_loop_never_ticks() {
        // FR-006c, the structural guarantee: an animation-disabled session cannot enter the 60fps
        // state, because there is never anything to advance.
        let mut e = Effects::new();
        let c = ctx(motionless(), true, Origin::Agent);
        for spec in all_specs() {
            assert!(!apply(&mut e, Some("panel"), &spec, &c));
        }
        assert!(!e.is_running());
    }

    #[test]
    fn apply_reports_false_when_suppressed_so_callers_show_final_content() {
        let mut e = Effects::new();
        let suppressed = ctx(motionless(), true, Origin::Agent);
        assert!(!apply(&mut e, Some("p"), &all_specs()[0], &suppressed));
        let live = ctx(on(), true, Origin::Agent);
        assert!(apply(&mut e, Some("p"), &all_specs()[0], &live));
    }

    #[test]
    fn the_panel_defaults_are_a_fade_in_and_a_dissolve_pair() {
        assert!(matches!(panel_enter_spec(), EffectSpec::FadeIn { .. }));
        let (out, in_) = panel_update_specs();
        assert!(matches!(out, EffectSpec::DissolveOut { .. }));
        assert!(matches!(in_, EffectSpec::DissolveIn { .. }));
    }

    #[test]
    fn directions_map_onto_tachyonfx_motions() {
        assert_eq!(motion(EffectDirection::Left), Motion::LeftToRight);
        assert_eq!(motion(EffectDirection::Right), Motion::RightToLeft);
        assert_eq!(motion(EffectDirection::Top), Motion::UpToDown);
        assert_eq!(motion(EffectDirection::Bottom), Motion::DownToUp);
    }
}
