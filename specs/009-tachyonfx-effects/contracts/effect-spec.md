# Contract: `EffectSpec` serde format

**Stability**: recorded in episode transcripts, so this format is versioned data, not an internal
detail. Changes require the same care as `RenderSpec` (003-visual-render).

## Wire format

Internally tagged on `kind`, snake_case:

```json
{ "kind": "fade_in",     "ms": 300 }
{ "kind": "slide_in",    "direction": "left", "ms": 400 }
{ "kind": "pulse",       "color": "sting",    "ms": 200 }
{ "kind": "evolve_in",   "ms": 600 }
```

Attached to a widget through the `Renderable` wrapper:

```json
{
  "spec":   { "type": "bar_chart", "title": "CPU Usage", "bars": [ … ] },
  "effect": { "kind": "slide_in", "direction": "left", "ms": 400 }
}
```

`"effect": null` (or absent) means "use the default transition for this target" — **not** "no
animation". Suppressing animation is the motion switch's job, never the absence of an `EffectSpec`.

## Variants

| `kind` | Fields | Category |
|---|---|---|
| `fade_in` | `ms` | color |
| `fade_out` | `ms` | color |
| `dissolve_in` | `ms` | text |
| `dissolve_out` | `ms` | text |
| `slide_in` | `direction`, `ms` | text |
| `slide_out` | `direction`, `ms` | text |
| `sweep_in` | `direction`, `ms` | text |
| `sweep_out` | `direction`, `ms` | text |
| `pulse` | `color`, `ms` | color |
| `glow` | `ms` | color |
| `evolve_in` | `ms` | text |
| `evolve_out` | `ms` | text |

**Category** drives `NO_COLOR` behavior (FR-006): color variants resolve to `None` under `NO_COLOR`;
text variants still play.

## Field rules

| Field | Type | Constraint |
|---|---|---|
| `ms` | `u32` | **Already clamped** to `100..=2000` when serialized (FR-023) |
| `direction` | string enum | `"left"` \| `"right"` \| `"top"` \| `"bottom"` |
| `color` | string | theme role name or `#RRGGBB` |

### Clamping is a construction-time property

`ms` is clamped in the Rhai constructor, before the value is ever stored. A transcript therefore
records the *effective* duration, and replaying a transcript reproduces the original animation
exactly. A deserializer encountering an out-of-range `ms` (hand-edited transcript, older producer)
MUST clamp on read as well — defense in depth, not a validation error.

### Unknown values

| Case | Behavior |
|---|---|
| unknown `direction` | resolve to `"left"` at *construction*, so the recorded value is canonical |
| unknown `color` role | resolve to the `info` role at *resolve* time — theme-dependent, so it cannot be canonicalized at construction |
| unknown `kind` | **deserialization error** — an unrecognized effect must not silently become a no-op |

The asymmetry is deliberate: `direction` and `kind` are closed sets known to the producer;
`color` depends on which theme is active at render time (005-themes), which the producer cannot know.

## Boundary guarantees

- `EffectSpec` lives in `bee-harness/src/render_spec/effect_spec.rs` — **not** under `tui/`, and
  **not** behind the `tui` feature gate. It compiles in the headless build.
- It imports no tachyonfx types. Verified by the existing `tests/core_deps_guard.rs` pattern extended
  with an assertion that `tachyonfx` is absent from `bee-core` and `bee-common` trees (SC-009).
- Resolution to a live `tachyonfx::Effect` happens only in `tui/effects.rs`.
