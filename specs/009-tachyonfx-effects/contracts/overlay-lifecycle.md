# Contract: full-screen overlay lifecycle

## Layout

```text
┌──────────────────────────────────────────┐
│  header                                  │  ← never covered
├──────────────────────────────────────────┤
│                                          │
│         OVERLAY (agent widget)           │  ← covers the chat area only
│                                          │
│  Esc to dismiss · auto-dismiss in 24s    │  ← hint, bottom row of the overlay
├──────────────────────────────────────────┤
│  > input line                            │  ← never covered (FR-014)
└──────────────────────────────────────────┘
```

The input line stays live throughout. The operator can type, edit, and submit while an overlay is
showing; submitting does **not** dismiss it (US3 §6).

## States

| State | Render loop (FR-002) | Entered by | Left by |
|---|---|---|---|
| *(absent)* | idle, event-driven | — | `render_fullscreen` at level `takeover` |
| `Entering` | 60fps | overlay created | entrance effect completes |
| `Showing` | **1Hz** | entrance completes | Esc / TTL / assistant reply / replacement |
| `Dismissing` | 60fps | any dismiss trigger | fade-out completes → absent |

`Showing` is the reason FR-002 has three states rather than two: a faded-in overlay has no active
effects, yet the countdown needs a redraw per second. Holding 60fps for a 120s overlay would cost
~7200 redraws to animate 120 integers.

With animations disabled, `Entering` and `Dismissing` are zero-duration — the overlay appears and
vanishes on the next redraw — but `Showing` still ticks at 1Hz, because the countdown is information,
not motion (FR-015).

## TTL resolution

```text
effective_ttl = min(script_requested_ttl_ms ?? config_default, config_max)
```

| Input | Config `takeover_ttl_secs` | Effective |
|---|---|---|
| `render_fullscreen(w)` | 30 | 30s |
| `render_fullscreen_ttl(w, 10_000)` | 30 | 10s — shorter requests honored |
| `render_fullscreen_ttl(w, 90_000)` | 30 | 30s — clamped (FR-021) |
| `render_fullscreen_ttl(w, 0)` | any | **script error** — ttl must be > 0 |
| config `takeover_ttl_secs = 600` | — | **startup error** — max is 120 |

Resolved once at construction. A config reload mid-session cannot extend a live overlay.

## Dismiss triggers

All four produce the same 200ms fade-out and the same restored chat with its scroll position
preserved (FR-018).

| Trigger | Requirement | Notes |
|---|---|---|
| `Esc` | FR-016 | from **either** focus |
| TTL expiry | FR-017 | |
| assistant reply | FR-019 | the model's next turn dismisses its own overlay |
| replacement | FR-020 | cross-fade; the new overlay's `Entering` begins as the old one's fade-out ends |

### `Esc` precedence

Insertion point: `handle_key` (`bee-harness/src/tui/app.rs:203`), **after** the `help_open` swallow
(205-210), **before** the `KeyModifiers::CONTROL` block (213).

```text
help_open?        → Esc closes help, swallow            (existing, unchanged)
overlay active?   → Esc dismisses overlay, return       ← NEW
Ctrl-c/z/d        → quit / suspend / EOF                (existing, unchanged)
Tab               → cycle focus                         (existing, unchanged)
focus dispatch    → Input: Esc clears input             (existing, unchanged)
                    Chat:  Esc hides panel column       (existing, unchanged)
```

Consequences:

- Pressing `Esc` with an overlay up and text in the input **dismisses the overlay and leaves the text
  intact**. A second `Esc` then clears the input as usual (US3 §2).
- A help overlay opened on top of a takeover closes first — help is the more modal surface.
- `q` is **not** a dismiss key. It keeps meaning quit in chat focus (`app.rs:269`), so no key's
  destructiveness depends on whether an overlay happens to be showing.

## Dismiss hint

- Rendered in the overlay's **bottom row**, always, in the same position (FR-015).
- Text: ``Esc`` to dismiss · auto-dismiss in {N}s
- `{N}` = `ceil(remaining.as_secs_f64())`, updated by the 1Hz tick.
- Uses the theme's `dim` treatment; under `NO_COLOR` it renders unstyled but still present.
- If the overlay's area is too short to spare a row, the hint still wins — it is the operator's
  documented escape hatch and must never be the thing that gets clipped.

## Edge cases

| Situation | Behavior |
|---|---|
| dismissed mid-`Entering` | entrance cancelled; fade-out runs from the current buffer state |
| terminal resize while active | effects cancelled, overlay re-rendered at the new size (008's resize strategy) |
| overlay area resolves to 0×0 | effect is a no-op; no crash, no work |
| rapid dismiss → new overlay | fade-out completes before the new `Entering` starts; no overlap |
| `render_fullscreen` below level `takeover` | never reaches this lifecycle — downgraded by the gate first |

## Test obligations

| Assertion | Requirement |
|---|---|
| hint present in the overlay's last row | FR-015 |
| countdown decrements once per second | FR-015 |
| `Esc` from input focus dismisses **and** preserves typed text | FR-016, US3 §2 |
| `Esc` with no overlay still clears input / hides panels | regression on 008 |
| dismissal ≤ 1 frame from keypress | SC-006 |
| overlay absent from buffer after `ttl + 0.2s` | SC-007 |
| chat scroll position unchanged across the cycle | FR-018 |
| second `render_fullscreen` replaces rather than stacks | FR-020 |
| ~N redraws (not 60N) for an N-second `Showing` | SC-003 |
