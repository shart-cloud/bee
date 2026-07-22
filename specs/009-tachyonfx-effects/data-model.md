# Phase 1 Data Model: Terminal Effects & Agent Visual Permissions

**Feature**: `009-tachyonfx-effects` | **Spec**: [spec.md](./spec.md) | **Research**: [research.md](./research.md)

Two layers, deliberately separated by the Constitution V boundary:

- **Serde layer** (`bee-harness/src/render_spec.rs`, no `tui` gate, no tachyonfx import) — pure data
  the Rhai engine produces and the transcript records.
- **Runtime layer** (`bee-harness/src/tui/`, `tui`-gated) — live tachyonfx objects, resolved from the
  serde layer at render time.

The Rhai sandbox only ever touches the serde layer.

---

## Serde layer

### `EffectSpec`

Declarative effect description. Lives in `render_spec/effect_spec.rs`. No tachyonfx types.

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EffectSpec {
    FadeIn     { ms: u32 },
    FadeOut    { ms: u32 },
    DissolveIn { ms: u32 },
    DissolveOut{ ms: u32 },
    SlideIn    { direction: Direction, ms: u32 },
    SlideOut   { direction: Direction, ms: u32 },
    SweepIn    { direction: Direction, ms: u32 },
    SweepOut   { direction: Direction, ms: u32 },
    Pulse      { color: String, ms: u32 },
    Glow       { ms: u32 },
    EvolveIn   { ms: u32 },
    EvolveOut  { ms: u32 },
}
```

| Field | Rule | Source |
|---|---|---|
| `ms` | clamped to `100..=2000` at construction in the Rhai binding | FR-023 |
| `direction` | `Direction` enum; unknown strings resolve to `Left` before construction | Rendering API |
| `color` | theme role name or `#RRGGBB`; unknown resolves to the `info` role at *resolve* time, not construction | Rendering API |

Clamping happens in the Rhai constructor so the recorded transcript value is already the effective
one — a transcript replay cannot produce a different animation than the live run.

### `Direction`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction { Left, Right, Top, Bottom }
```

Maps to `tachyonfx::Motion` only in the runtime layer (R3): `Left → LeftToRight`,
`Right → RightToLeft`, `Top → UpToDown`, `Bottom → DownToUp`.

### `RenderTarget`

The commit verb's target, produced by the Rhai binding and consumed by the visual gate.

```rust
pub enum RenderTarget {
    Inline,
    Panel   { id: String, ttl_ms: Option<u32> },
    Overlay { ttl_ms: Option<u32> },          // NEW (FR-013a)
}
```

| Verb | Target |
|---|---|
| `render(w)` | `Inline` |
| `render_to(id, w)` | `Panel { id, ttl_ms: None }` |
| `render_to_ttl(id, w, ms)` | `Panel { id, ttl_ms: Some(ms) }` |
| `render_fullscreen(w)` | `Overlay { ttl_ms: None }` |
| `render_fullscreen_ttl(w, ms)` | `Overlay { ttl_ms: Some(ms) }` |

`ttl_ms` must be `> 0` for both TTL verbs — script error otherwise, matching the existing
`render_to_ttl` validation (`render_api.rs:912`).

### `VisualLevel`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum VisualLevel { None, #[default] Panels, PanelsWide, Takeover }
```

`Ord` is derived and the variant order is the permission order, so the gate's checks are comparisons
(`level >= VisualLevel::Takeover`) rather than match arms — deny-by-default falls out of the ordering.

Parsed from `[harness] visual_level`, `--visual-level`, `BEE_VISUAL_LEVEL`. Precedence CLI > env >
scenario > default (FR-008). An unrecognized value is a **hard error**, not a fallback to the default
(Constitution I: fail closed, never silently escalate or degrade).

### `MotionSetting`

```rust
pub struct MotionSetting { pub enabled: bool }   // default: true
```

Sourced from `[harness] animations` (bool), `--no-animation` (flag), `BEE_NO_ANIMATION` (any value,
matching `NO_COLOR` semantics per `viz/palette.rs:28`). Precedence CLI > env > scenario > default.

Orthogonal to `VisualLevel` and to `NO_COLOR` (FR-006d).

### `TakeoverConfig`

```rust
pub struct TakeoverConfig { pub ttl_secs: u32 }   // default 30, hard max 120
```

From `[harness] takeover_ttl_secs`. Values above 120 are a hard error, not a clamp — a config asking
for 600s is a mistake worth surfacing, unlike an agent request which is clamped silently (FR-021).

### `RenderSpec` change

Each variant gains an optional attached effect. The spec's wording ("as an optional field on each
variant") is implemented as a wrapper rather than 12 duplicated fields:

```rust
pub struct Renderable {
    pub spec: RenderSpec,
    pub effect: Option<EffectSpec>,
}
```

This keeps `RenderSpec`'s existing serde shape byte-compatible for the 003-visual-render transcript
format — old transcripts deserialize unchanged, and `widget.effect(e)` sets the wrapper field.

---

## Runtime layer (`tui`-gated)

### `EffectSlot`

Per-panel effect state, stored inside `Panel` in the `PanelRegistry`.

```rust
pub struct EffectSlot {
    pub prev: Option<Buffer>,   // snapshot of the outgoing content, for dissolve→coalesce (FR-004)
    pub started: Option<Instant>,
}
```

There is deliberately **no effect handle** here. `EffectManager::unique(panel_name, fx)` owns
cancellation (R4), so the slot holds only what tachyonfx cannot: the previous buffer.

**Lifecycle**:

| Transition | Effect applied | Key |
|---|---|---|
| new panel name upserted | `FadeIn` default (FR-003) | panel name |
| existing panel replaced | `parallel([dissolve(prev), coalesce(new)])` (FR-004) | panel name |
| replaced mid-transition | previous cancelled by the shared key (FR-005) | panel name |
| panel removed / TTL expired | `FadeOut`, then drop | panel name |

### `Overlay`

```rust
pub struct Overlay {
    pub spec: Renderable,
    pub created: Instant,
    pub ttl: Duration,        // already resolved: min(requested, config max)
    pub dismissing: bool,     // true once fade-out starts; blocks re-entry
}
```

**State machine**:

```text
        render_fullscreen (level == Takeover)
                │
                ▼
        ┌───────────────┐   second render_fullscreen
        │   Entering    │──────────────┐
        │  (fade-in fx) │              │  cross-fade, replace in place (FR-020)
        └───────┬───────┘              │
                │ effect completes     │
                ▼                      │
        ┌───────────────┐              │
        │    Showing    │◀─────────────┘
        │ (1Hz countdown│
        │  state, FR-002│
        │  state 2)     │
        └───────┬───────┘
                │ Esc (any focus)  ·  TTL expiry  ·  assistant reply
                ▼
        ┌───────────────┐
        │   Dismissing  │   fade-out, 200ms (FR-018)
        └───────┬───────┘
                │
                ▼
             (None)  — chat restored, scroll position preserved
```

Invariants:

- At most one `Overlay` exists (FR-020). `Option<Overlay>` in `App` enforces it structurally.
- `ttl` is resolved at construction, never re-read from config, so a mid-session config change cannot
  extend a live overlay.
- Dismissal during `Entering` cancels the entrance effect and fades out from the current buffer state
  (spec Edge Cases).
- With animations disabled, `Entering` and `Dismissing` are zero-duration: the overlay appears and
  disappears on the next redraw, but the countdown still runs (FR-015).

### `EffectResolver`

The single chokepoint (R5). Lives in `tui/effects.rs`.

```rust
fn resolve(spec: &EffectSpec, ctx: &ResolveCtx) -> Option<tachyonfx::Effect>
```

Returns `None` — meaning "render final content, register nothing" — when:

| Condition | Scope | Requirement |
|---|---|---|
| animations disabled | all variants | FR-006b |
| `NO_COLOR` set | `FadeIn/Out`, `Pulse`, `Glow` only | FR-006 |
| target area is 0×0 | all variants | spec Edge Cases |
| `visual_level == None` | all agent-originated variants | FR-024 |

`ResolveCtx` carries the active theme (for role→`Color`), the target `Rect`, and the three
presentation axes.

---

## Entity relationships

```text
  Rhai script
      │ produces
      ▼
  Renderable { spec: RenderSpec, effect: Option<EffectSpec> }  +  RenderTarget
      │                                                             │
      │                              ┌──────────────────────────────┘
      ▼                              ▼
  ┌─────────────────────────────────────────┐
  │  visual_gate (VisualLevel)              │   downgrade + note (FR-009)
  └────────────┬────────────────────────────┘
               │ Inline          │ Panel{id}         │ Overlay
               ▼                 ▼                   ▼
         chat transcript   PanelRegistry        App.overlay
                             └─ Panel            (Option<Overlay>)
                                 └─ EffectSlot
                                       │
                                       └──────────┬─────────────┐
                                                  ▼             ▼
                                        EffectResolver ──> EffectManager<String>
                                        (motion / NO_COLOR)     │ unique(key, fx)
                                                                ▼
                                                    process_effects(dt, buf, area)
```
