# Contract: visual permission tiers and downgrade rules

## Tiers

| Ord | TOML value | Panels | Panel width cap | Overlay |
|-----|-----------|--------|-----------------|---------|
| 0 | `none` | ✗ | — | ✗ |
| 1 | `panels` *(default)* | ✓ | ⌊w/3⌋ | ✗ |
| 2 | `panels-wide` | ✓ | ⌊w/2⌋ | ✗ |
| 3 | `takeover` | ✓ | ⌊w/2⌋ | ✓ |

Levels are a **ceiling**, not a mode: level 3 permits everything levels 1–2 permit.

The `VisualLevel` enum derives `Ord` with variants in this order, so every gate check is a
comparison. Adding a tier later means inserting a variant, not editing a match.

## Configuration

| Source | Form | Precedence |
|---|---|---|
| CLI | `--visual-level panels` | 1 (highest) |
| Env | `BEE_VISUAL_LEVEL=panels` | 2 |
| Scenario | `[harness] visual_level = "panels"` | 3 |
| Default | `panels` | 4 |

An unrecognized value at any source is a **hard startup error**. It does not fall back to the
default, and it does not fall back to the next source. Constitution I: a policy that cannot be
compiled is refused, never silently weakened.

## Downgrade matrix

Applied in the session event handler, before any panel or overlay state mutates.

| Level | Requested target | Result | Note appended to `ToolResult.content` |
|---|---|---|---|
| `none` | `Inline` | `Inline` | — |
| `none` | `Panel{id}` | `Inline` | `downgraded to inline: visual level is none` |
| `none` | `Overlay` | `Inline` | `downgraded to inline: visual level is none` |
| `panels` | `Inline` | `Inline` | — |
| `panels` | `Panel{id}` | `Panel{id}`, width ≤ ⌊w/3⌋ | — |
| `panels` | `Overlay` | `Panel{"takeover"}` | `downgraded to panel: visual level does not allow takeover` |
| `panels-wide` | `Panel{id}` | `Panel{id}`, width ≤ ⌊w/2⌋ | — |
| `panels-wide` | `Overlay` | `Panel{"takeover"}` | `downgraded to panel: visual level does not allow takeover` |
| `takeover` | `Overlay` | `Overlay`, ttl = min(requested, config) | — |
| any | `Inline` | `Inline` | always passes through |

### Why downgrade instead of error

FR-009. The model gets a *working* visual plus a note telling it what happened, so it can adapt on
the next turn. An error would make a level-1 session fail every time a model reached for takeover,
which trains the model to stop rendering at all.

### Width cap interaction with 008

The cap is `min(level_cap, existing_008_panel_w_cap)` (FR-011). 008's own constraint —
"this widget needs at least N columns" (`render_api.rs:144`) — still applies *after* the cap and
still produces its existing script error. The visual level narrows the budget; it does not suppress
008's minimum-width failure.

## The reserved `takeover` panel id

An `Overlay` downgraded to a `Panel` targets the reserved id `"takeover"` (research R7).

| Property | Behavior |
|---|---|
| Validator | satisfies the existing 1–32 char `[a-z0-9_-]` rule unchanged (`render_api.rs:182`) |
| Repeat downgrades | upsert the same panel — replace, never stack (matches FR-020) |
| Direct agent use | `render_to("takeover", w)` is a **script error**: the id is reserved |
| Column-full path | one reserved slot, so downgrades cannot exhaust the column (`render_api.rs:155`) |

## Effects and levels

- At level `none`, effects are stripped entirely (FR-024) — the content renders immediately.
- At levels 1–3, agent effects are permitted. The level does not gate *which* effects.
- **Harness chrome effects are outside this matrix.** `visual_level` governs the agent, never the
  harness's own UI (FR-006d). Silencing chrome is the motion switch's job — see
  [motion-control.md](./motion-control.md).

## Test obligations

| Assertion | Requirement |
|---|---|
| default is `panels` when no key present | FR-007, US2 §5 |
| unknown level errors at startup | Constitution I |
| CLI beats env beats scenario | FR-008 |
| each downgrade row produces its exact note text | FR-009, SC-004 |
| panel `Rect` width ≤ ⌊w/3⌋ at `panels` | SC-005 |
| `render_to("takeover", …)` is a script error | R7 |
