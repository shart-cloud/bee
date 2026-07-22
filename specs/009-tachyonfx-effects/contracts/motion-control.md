# Contract: motion control and the three presentation axes

## The three axes are independent

| Axis | Governs | Sources | Default |
|---|---|---|---|
| **Color** | ANSI color output | `NO_COLOR` (any value) | color on |
| **Motion** | whether anything animates | `--no-animation`, `BEE_NO_ANIMATION`, `[harness] animations` | motion on |
| **Real estate** | how much screen the *agent* may claim | `--visual-level`, `BEE_VISUAL_LEVEL`, `[harness] visual_level` | `panels` |

They compose freely (FR-006d). Every combination is valid:

| `NO_COLOR` | animations | `visual_level` | Result |
|---|---|---|---|
| — | on | `takeover` | everything |
| set | on | `takeover` | monochrome, still animated — text effects only |
| — | **off** | `takeover` | full color, full takeover rights, **zero motion** |
| — | on | `none` | no agent panels, **harness chrome still animates** |
| set | off | `none` | the CI configuration |

The fourth row is the one the spec previously left undefined: **`visual_level = "none"` does not
silence harness chrome.** The level restricts the agent, not bee's own UI. Motion is the only switch
that reaches chrome.

## Configuration

| Source | Form | Precedence |
|---|---|---|
| CLI | `--no-animation` | 1 (highest) |
| Env | `BEE_NO_ANIMATION` (any value, per `NO_COLOR` convention) | 2 |
| Scenario | `[harness] animations = false` | 3 |
| Default | animations enabled | 4 |

`BEE_NO_ANIMATION` follows `viz/palette.rs:28`'s existing pattern — presence is truth, value is
irrelevant — so `BEE_NO_ANIMATION=0` still disables. That is surprising in isolation but consistent
with `NO_COLOR`, which bee already implements this way, and consistency beats novelty here.

## Enforcement point

A single chokepoint in `tui/effects.rs` (research R5):

```rust
fn resolve(spec: &EffectSpec, ctx: &ResolveCtx) -> Option<tachyonfx::Effect>
```

`None` means "render the final content now, register nothing with the `EffectManager`".

| Condition | Returns `None` for |
|---|---|
| animations disabled | **every** variant, every origin — agent, panel default, overlay, chrome |
| `NO_COLOR` | color variants only: `FadeIn`, `FadeOut`, `Pulse`, `Glow` |
| area is 0×0 | every variant |
| `visual_level == none` | every agent-originated variant |

Why one chokepoint rather than checks at each call site: FR-006c requires that a motion-disabled
session **never** enters the 60fps render state. If nothing is ever registered, `is_running()` is
permanently false and that guarantee is structural rather than a thing to remember at each of a dozen
call sites.

### Rejected alternative

Filtering inside the `EffectManager` with tachyonfx's `CellFilter` was considered and rejected: a
filtered effect still *runs*: it still advances, still reports active, and still holds the loop at
60fps. That satisfies the visual requirement while violating FR-006c.

## What motion-off does not change

| Still happens | Why |
|---|---|
| the overlay's 1Hz countdown | information, not motion (FR-015) |
| TTL expiry and dismissal | a safety property, not decoration |
| panel creation, replacement, removal | content, not animation |
| downgrade notes from the visual gate | orthogonal axis |
| color output | orthogonal axis |

Content transitions still *happen* — they just happen instantly. A panel update with animations off
shows the new content on the next redraw.

## Test obligations

| Assertion | Requirement |
|---|---|
| `BEE_NO_ANIMATION=1` → zero periodic redraws across panel create/replace/overlay/startup | SC-011 |
| every buffer snapshot at t=0 already shows final content | SC-011 |
| `NO_COLOR=1` + animations on + `visual_level=none` → chrome text effects still mutate cells, no color escapes, no panels | SC-012 |
| CLI beats env beats scenario | FR-006a |
| the countdown still advances with motion off | FR-015 |
