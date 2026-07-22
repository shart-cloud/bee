# Contract: agent-facing Rhai effect + takeover API

Registered on the **same** `Engine` as the 003-visual-render drawing API (FR-025). The sandbox never
sees a tachyonfx type — these constructors produce `EffectSpec` data.

## Effect constructors

| Rhai | Signature | `EffectSpec` produced |
|---|---|---|
| `fade_in(ms)` | `fn(i64) -> Effect` | `FadeIn { ms }` |
| `fade_out(ms)` | `fn(i64) -> Effect` | `FadeOut { ms }` |
| `dissolve_in(ms)` | `fn(i64) -> Effect` | `DissolveIn { ms }` |
| `dissolve_out(ms)` | `fn(i64) -> Effect` | `DissolveOut { ms }` |
| `slide_in(dir, ms)` | `fn(String, i64) -> Effect` | `SlideIn { direction, ms }` |
| `slide_out(dir, ms)` | `fn(String, i64) -> Effect` | `SlideOut { direction, ms }` |
| `sweep_in(dir, ms)` | `fn(String, i64) -> Effect` | `SweepIn { direction, ms }` |
| `sweep_out(dir, ms)` | `fn(String, i64) -> Effect` | `SweepOut { direction, ms }` |
| `pulse(color, ms)` | `fn(String, i64) -> Effect` | `Pulse { color, ms }` |
| `glow(ms)` | `fn(i64) -> Effect` | `Glow { ms }` |
| `evolve_in(ms)` | `fn(i64) -> Effect` | `EvolveIn { ms }` |
| `evolve_out(ms)` | `fn(i64) -> Effect` | `EvolveOut { ms }` |
| `widget.effect(e)` | `fn(&mut Widget, Effect)` | attaches to any widget type |

### Argument handling

| Input | Rule |
|---|---|
| `ms` outside 100–2000 | clamped **silently** at construction (FR-023) |
| `ms` non-positive | clamped to 100 — not an error |
| unknown `dir` | `"left"` |
| unknown `color` | the `info` role, resolved at render time against the active theme |

Clamping rather than erroring is deliberate: a model that guesses 5000ms should get a 2000ms
animation and keep working, not a failed tool call.

## Takeover commit verbs

Completing the existing commit family (`render`, `render_to`, `render_to_ttl`):

| Rhai | Signature | Target |
|---|---|---|
| `render_fullscreen(w)` | `fn(RenderSpec)` | `Overlay { ttl_ms: None }` |
| `render_fullscreen_ttl(w, ttl_ms)` | `fn(RenderSpec, i64)` | `Overlay { ttl_ms: Some(_) }` |

`ttl_ms <= 0` is a script error, matching `render_to_ttl` (`render_api.rs:912`).

The target is explicit at the call site — not a widget property, not a magic panel id — so the visual
gate can downgrade before any panel or overlay state mutates.

## Examples

```rhai
// Default entrance — the harness picks the transition.
let chart = bar_chart("CPU Usage");
chart.bar("core-0", 72);
chart.bar("core-1", 55);
render_to("metrics", chart);

// Directional intent: this data arrived from the left.
let chart = bar_chart("CPU Usage");
chart.bar("core-0", 72);
chart.effect(slide_in("left", 400));
render_to("metrics", chart);

// Status change: flash the error role.
let dots = dots("suite");
dots.fail("test_auth");
dots.effect(pulse("sting", 200));
render_to("results", dots);

// Full-screen, 10s instead of the configured default.
let chart = line_chart("Throughput");
chart.effect(fade_in(300));
render_fullscreen_ttl(chart, 10000);
```

## Degradation

An attached effect is **silently dropped** — content renders immediately, no error — when:

| Condition | Requirement |
|---|---|
| `visual_level == none` | FR-024 |
| animations disabled | FR-006b |
| `NO_COLOR` set, and the effect is a color variant | FR-006 |
| the target area is 0×0 | spec Edge Cases |
| the session is the inline REPL, not the full-screen TUI | spec Edge Cases |

Dropping is silent because an effect is decoration. A tool call must not fail because the operator
turned animations off.

## Implementation notes

Two of the twelve are **composites**, not direct tachyonfx calls (research R3) — invisible to the
agent, but relevant when reading `tui/effects.rs`:

| Spec | Resolution |
|---|---|
| `Pulse { color, ms }` | `fx::sequence(&[fade_to_fg(c, ms/2), fade_from_fg(c, ms/2)])` |
| `Glow { ms }` | `fx::ping_pong(fx::lighten(Some(a), Some(a), ms/2))` |

And one **inverts naming** against tachyonfx: bee's `dissolve_in` is tachyonfx's `coalesce`
(characters reforming), while bee's `dissolve_out` is tachyonfx's `dissolve`. bee's naming is
consistent with its own `_in`/`_out` convention across all twelve; the mapping lives in the resolver.

`slide_*` and `sweep_*` take five arguments in tachyonfx (`Motion`, `gradient_length`, `randomness`,
color, timer). The Rhai surface exposes two; `gradient_length` and `randomness` are house values set
by the resolver, deliberately not agent-tunable.
