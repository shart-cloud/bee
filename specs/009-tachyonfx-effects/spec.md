# Feature Specification: Terminal Effects & Agent Visual Permissions

**Feature Branch**: `009-tachyonfx-effects`

**Created**: 2026-07-22

**Status**: Draft

**Depends on**: `003-visual-render` (Rhai rendering API, `RenderSpec`, `viz` module, honeycomb design
language), `008-grid-tui` (full-screen TUI, panel registry, `PanelOp`, layout modes), `005-themes`
(semantic roles, `ThemeColor`, active theme)

**Input**: User description: "Use the tachyonfx effects/animation library for ratatui to let
agent-defined and -rendered TUIs have animation and look much better. Configuration should set exactly
what the agent is allowed to do, from full visual control to only allowed to show panels on the side.
If the agent takes over the whole screen there needs to be a TTL or way for the HITL to dismiss and
get back to the chat."

---

## Overview

The bee TUI today is static. Panels appear. Charts appear. Nothing moves. There are no transitions,
no fades, no visual feedback when data changes. The only animation is the flipbook sprite system
(003-visual-render, FR-029/FR-030): swap one picture for another, 50–1000ms per frame. That is a
slideshow, not polish.

This feature adds two things:

1. **tachyonfx integration** — a post-processing effects pipeline that makes the TUI look alive.
   Panels fade in. Updated data sweeps across. Status changes pulse. The bee mascot dissolves in from
   noise. Effects are applied *after* widgets are rendered to the ratatui `Buffer` and *before* the
   buffer goes to the screen — they change colors, characters, and visibility of already-drawn cells.
   This is how tachyonfx works: render your content first, then transform it.

2. **Agent visual permissions** — a configuration layer that controls how much of the screen the agent
   is allowed to use. At one end: the agent can only show small panels beside the chat. At the other:
   the agent can take over the entire screen with a full-bleed visualization. The operator (the
   human in the loop) always has a way to dismiss agent visuals and get back to the conversation. A
   screen takeover always has a time limit.

The permissions are not about safety in the security sense — the agent's Rhai scripts are already
sandboxed (003-visual-render, FR-020/FR-021). They are about **attention**. An agent that covers the
chat with a spinning globe is annoying. An agent that quietly updates a panel with useful metrics is
helpful. The operator controls which end of that spectrum is allowed.

---

## Clarifications

### Session 2026-07-22

- Q: How should the agent request a full-screen takeover, given the Rendering API defines no
  constructor that produces an `Overlay` target? → A: New Rhai functions `render_fullscreen(widget)`
  and `render_fullscreen_ttl(widget, ttl_ms)`, mirroring the existing `render_to` / `render_to_ttl`
  pair. The target is explicit at the call site, so the visual gate can downgrade before any panel
  or overlay state mutates.
- Q: What is the `Esc` / `q` precedence while an overlay is active, given `Esc` is already bound
  three ways in 008 and `q` already quits in chat focus? → A: Overlay dismiss sits directly below
  the help overlay and above focus dispatch in `handle_key`, so `Esc` dismisses from any focus; a
  second `Esc` then performs its normal per-focus job. `q` is NOT a dismiss key — it keeps meaning
  quit in every context.
- Q: How should the mandatory ratatui 0.29 → 0.30 bump be scoped relative to this feature? → A: As
  Phase 0 inside this feature — the first task group in `tasks.md`, on the same branch and in the
  same PR as the effects work. Phase 0 lands the bump with `crossterm_0_28` (crossterm stays at
  0.28) and must leave the entire 008-grid-tui suite green with zero behavior change before any
  tachyonfx task starts.
  - **Amended 2026-07-22 (Phase 0 research)**: the crossterm half of this answer was based on a
    wrong premise — `ratatui/crossterm_0_28` cannot hold crossterm at 0.28, because
    `ratatui-crossterm`'s `cfg_if` always prefers 0.29 and ratatui enables its defaults. Phase 0
    moves bee's own crossterm to **0.29** instead, yielding a single crossterm.
    [research.md R2/D1](./research.md). The rest of the answer stands, and Phase 0 turned out to be
    manifest-only rather than an import migration.
- Q: Should motion have its own kill switch separate from `visual_level`, and does
  `visual_level = "none"` silence the harness's own chrome effects? → A: A separate, orthogonal
  switch (`BEE_NO_ANIMATION` / `--no-animation` / `[harness] animations = false`) kills all motion —
  agent effects and harness chrome alike. `visual_level` governs only how much screen the agent may
  claim, so `visual_level = "none"` does NOT silence chrome effects. The three axes (`NO_COLOR`,
  animations, `visual_level`) are independent and compose.
- Q: FR-015's per-second countdown contradicts SC-003's zero-periodic-redraws-when-idle guarantee —
  how is the render loop scheduled while an overlay is up but no effects are running? → A: Three loop
  states. 60fps (16ms) while effects are running; 1Hz while an overlay is active with no effects, to
  drive the countdown; fully idle and event-driven otherwise. SC-003's guarantee is rescoped to "no
  active effects AND no active overlay".

---

## Motivation

### Why tachyonfx

tachyonfx is the official effects library in the ratatui ecosystem. It is maintained under the
`ratatui/` GitHub organization and is purpose-built for exactly this job: shader-like post-processing
of terminal buffers. It provides:

- **Color effects**: fade between colors, shift hue/saturation/lightness, paint, saturate, darken,
  lighten — all with configurable duration and easing.
- **Text effects**: dissolve (text breaks apart into random characters), coalesce (random characters
  reform into text), evolve (characters morph through symbol progressions like `▏▎▍▌▋▊▉█`),
  slide in/out, sweep in/out, explode.
- **Geometry effects**: translate (move a region), expand/stretch from nothing using block characters.
- **Timing combinators**: run effects in parallel or sequence, loop them, ping-pong them, delay them,
  freeze them at a point, repeat them N times or forever.
- **32 easing curves**: linear, sine, cubic, elastic, bounce, exponential, etc.
- **Spatial patterns**: control how an effect spreads across a region — checkerboard, diagonal sweep,
  radial (from center out), wave interference.
- **Cell filtering**: apply effects only to text cells, only to a certain color, only inside a margin.
- **An `EffectManager`** that owns active effects, advances them each frame, and cleans them up when
  they finish.

All of this works by modifying ratatui `Buffer` cells after widgets render. It never touches I/O,
files, or the network. It is pure computation on a grid of characters and colors.

The alternative — building a custom animation system — is not worth it. tachyonfx exists, is mature
(v0.25, ~1,300 GitHub stars, actively maintained), has no unsafe code, and does exactly what is needed.

### Why visual permissions

An LLM agent with unrestricted screen access is a UX problem, not a security problem. The agent is
already sandboxed: the Rhai engine has no I/O, the kernel sandbox confines tool children, and the
harness owns the terminal. But an agent that fills the screen with a chart the operator did not ask
for steals attention. An agent that shows three overlapping dashboards is noise. The operator should
control:

- Whether the agent can show panels at all.
- Whether the agent can show panels that are bigger than the side column.
- Whether the agent can take over the full screen.
- How long a full-screen takeover lasts before it auto-dismisses.

These are configuration knobs, not permission prompts. The operator sets them once (in the scenario
TOML or as CLI flags), and the harness enforces them for the session.

---

## User Scenarios & Testing

### User Story 1 — Panels fade in and data updates sweep across (Priority: P1)

During a full-screen session, the model renders a chart to a named panel. Instead of the chart
popping into existence, it fades in over 300ms. On a later turn the model updates the same panel with
new data. The old content dissolves (characters scatter) and the new content coalesces (characters
reform) over 400ms. The operator sees a smooth transition, not a jump-cut. These transitions happen
automatically — the model does not need to request them.

**Why this priority**: This is the single biggest visual improvement for the least effort. It hooks
into the existing panel upsert path, needs no Rhai API changes, and makes the TUI feel alive.

**Independent Test**: In a `tui` feature-gated test, render a `RenderSpec::BarChart` to a named panel,
advance the `EffectManager` by 300ms of simulated ticks, and verify that the buffer cells have
intermediate color values (not the final colors and not blank). Then upsert new content to the same
panel, advance 400ms, and verify the dissolve/coalesce sequence produces the expected character
mutations.

**Acceptance Scenarios**:

1. **Given** a full-screen session with no panels, **When** the model renders a widget to a new panel
   name, **Then** the panel region fades from the background color to the widget's content over
   200–400ms (configurable), using the theme's `info` role as the fade-from color.
2. **Given** an existing panel showing a bar chart, **When** the model upserts new content to the
   same panel, **Then** the old content dissolves (characters replaced by random glyphs progressing to
   blank) and the new content coalesces (random glyphs progressing to the correct characters), in a
   single parallel transition lasting 300–500ms.
3. **Given** an active transition on panel "metrics", **When** a second update arrives at panel
   "metrics" before the first transition finishes, **Then** the first transition is cancelled and a
   new transition starts from the current buffer state to the new content (no visual stacking).
4. **Given** `NO_COLOR=1`, **When** a panel transition plays, **Then** the character-based effects
   (dissolve, coalesce, evolve) still play but no color fades occur — effects degrade to text-only.

---

### User Story 2 — The operator configures how much screen the agent can use (Priority: P2)

A scenario author (or the operator via CLI) sets a visual permission level in the scenario TOML or
with a flag. The levels are:

| Level | Name | What the agent can do |
|-------|------|-----------------------|
| 0 | `none` | No panels, no visuals. `render` tool output goes inline in chat only. |
| 1 | `panels` | Side panels only (the 008-grid-tui column). No wider than 1/3 of the screen. |
| 2 | `panels-wide` | Panels can grow up to 1/2 of the screen. The agent can request more width. |
| 3 | `takeover` | The agent can request a full-screen overlay (`render_fullscreen`) for a single visualization. The overlay has a TTL and a dismiss key. Panels are still available. |

`visual_level` governs screen real estate for the agent only. It is independent of `NO_COLOR` (color)
and of the animations switch (motion) — see FR-006d. In particular, level `none` still lets the
harness animate its own chrome.

The default is `panels` (level 1). The levels are a ceiling — a level-3 session still allows panels;
it just also allows takeover. If the agent tries something its level does not permit (e.g., requests a
full-screen overlay at level 1), the visual is silently downgraded to the highest allowed form (e.g.,
a panel), and the tool result notes the downgrade so the model can adjust.

**Why this priority**: Without this, either the agent can do everything (risky for UX) or nothing
(throws away the feature). The permission system makes the feature safe to ship because the operator
stays in control.

**Independent Test**: Set `visual_level = "panels"` in a scenario, have the model attempt a
full-screen render, and verify the result is rendered as a panel with a downgrade note in the tool
summary.

**Acceptance Scenarios**:

1. **Given** `visual_level = "none"`, **When** the model calls `render` with a panel target, **Then**
   the widget is rendered inline in chat (not as a panel) and the tool summary says "downgraded to
   inline: visual level is none."
2. **Given** `visual_level = "panels"`, **When** the model calls `render` with a panel target,
   **Then** the panel appears in the side column, sized to at most 1/3 of the terminal width.
3. **Given** `visual_level = "panels"`, **When** the model's script calls `render_fullscreen(w)`,
   **Then** the widget is rendered as a panel instead and the tool summary says "downgraded to panel:
   visual level does not allow takeover."
4. **Given** `visual_level = "takeover"`, **When** the model's script calls `render_fullscreen(w)`,
   **Then** the overlay appears, covering the chat, with a visible dismiss hint and a TTL countdown.
5. **Given** a scenario with no `visual_level` key, **When** the session starts, **Then** the
   default is `panels` (level 1).
6. **Given** `visual_level = "takeover"` and `takeover_ttl_secs = 20`, **When** the script calls
   `render_fullscreen_ttl(w, 90000)` (90s), **Then** the overlay's TTL is 20s; **and when** it calls
   `render_fullscreen_ttl(w, 5000)` (5s), **Then** the TTL is 5s — the config is a cap, not an
   override.
7. **Given** `visual_level = "none"` and animations enabled, **When** the model calls
   `render_fullscreen(w)`, **Then** the widget renders inline in chat with a downgrade note, and the
   harness's own chrome effects continue to play normally.

---

### User Story 3 — Full-screen takeover with TTL and dismiss (Priority: P2)

At visual level `takeover`, the model can request a full-screen overlay: a single widget (chart,
grid, dashboard layout) that covers the chat area entirely. The overlay:

- **Shows a dismiss hint**: a subtle line at the bottom — "`Esc` to dismiss · auto-dismiss in 30s" —
  always visible, always in the same place.
- **Has a TTL**: the overlay disappears on its own after the configured duration (default 30 seconds,
  configurable per-scenario). When the timer runs out, the overlay fades out and the chat reappears.
- **Is dismissible by the operator**: pressing `Esc` — from either focus, including mid-keystroke in
  the input line — immediately dismisses the overlay with a short fade-out. `Esc` outranks its
  existing bindings (clear input, hide panel column) while an overlay is up. `q` is not a dismiss
  key; it keeps meaning quit.
- **Does not block input**: while the overlay is showing, the operator can still type into the input
  line (it stays visible below the overlay) and send messages. The model's next reply dismisses the
  overlay automatically.
- **Stacks at most one**: if the model requests a second overlay while one is showing, the first is
  replaced (not stacked). No overlay pile-up.

The TTL is a hard cap, not a request. Even if the model asks for a longer duration, the configured
maximum wins.

**Why this priority**: P2, same as the permission system, because takeover without safety rails is the
one thing that could make the feature annoying enough to disable entirely.

**Independent Test**: In a `tui` test, trigger a full-screen overlay, verify the dismiss hint is
rendered in the last row of the overlay, advance time past the TTL, and verify the overlay is gone and
the chat area is restored.

**Acceptance Scenarios**:

1. **Given** `visual_level = "takeover"` and `takeover_ttl_secs = 15`, **When** the model renders a
   full-screen widget, **Then** the overlay covers the chat area, the input line remains visible, a
   dismiss hint shows "Esc to dismiss · auto-dismiss in 15s", and the countdown updates each second.
2. **Given** an active overlay and focus on the input line with text typed, **When** the operator
   presses `Esc`, **Then** the overlay fades out over 200ms, the chat area reappears with its scroll
   position preserved, and the typed input is left **untouched** — the dismiss consumes the keypress
   before `Esc`'s clear-input binding sees it. A second `Esc` then clears the input as usual.
3. **Given** an active overlay, **When** the TTL expires, **Then** the overlay fades out and the chat
   area reappears — identical behavior to a manual dismiss.
4. **Given** an active overlay, **When** the model sends a new chat message (its next assistant turn),
   **Then** the overlay is dismissed (with fade-out) and the reply appears in chat.
5. **Given** an active overlay, **When** the model requests a second overlay, **Then** the first
   overlay is replaced in-place (cross-fade transition, no stacking).
6. **Given** an active overlay with 10s remaining, **When** the operator types a message and presses
   Enter, **Then** the message is sent and the overlay remains until dismissed, the TTL expires, or
   the model replies.

---

### User Story 4 — The agent requests specific effects via Rhai (Priority: P3)

At visual levels `panels-wide` and `takeover`, the model's Rhai script can request a specific
transition effect instead of accepting the default. The Rhai API exposes a curated set of effect
constructors — not the full tachyonfx surface, but a safe, useful subset — that the model attaches
to a widget before calling `render()`.

The curated effects:

| Rhai function | What it does |
|---------------|-------------|
| `fade_in(ms)` | Fade from background to content. |
| `fade_out(ms)` | Fade from content to background. |
| `dissolve_in(ms)` | Characters coalesce from random noise. |
| `dissolve_out(ms)` | Characters dissolve to random noise. |
| `slide_in(direction, ms)` | Content slides in from "left", "right", "top", or "bottom". |
| `slide_out(direction, ms)` | Content slides out. |
| `sweep_in(direction, ms)` | A colored wave reveals content. |
| `sweep_out(direction, ms)` | A colored wave hides content. |
| `pulse(color, ms)` | A single flash of color that fades back — for status changes. |
| `glow(ms)` | Lighten toward white and back — a gentle breathing effect. |
| `evolve_in(ms)` | Characters morph through block chars to real content. |
| `evolve_out(ms)` | Content devolves into blocks, then disappears. |

Effects are attached to widgets:

```rhai
let chart = bar_chart("CPU Usage");
chart.bar("core-0", 72);
chart.bar("core-1", 55);
chart.effect(slide_in("left", 400));
render_to("metrics", chart);
```

Durations are clamped to 100–2000ms. Unknown directions default to "left". If the visual level does
not permit the operation (e.g., an effect on a panel at level `none`), the effect is silently dropped
and the content renders without animation.

**Why this priority**: P3 because the auto-transitions from US1 handle the 80% case. Explicit effects
are a refinement — they let a skilled model author add directional intent ("this data came from the
left") but the TUI is already attractive without them.

**Independent Test**: In a `tui` test, run a Rhai script that builds a bar chart with
`chart.effect(slide_in("left", 400))`, render it to a panel, advance 200ms, and verify the buffer
shows partially-slid content (some cells visible, some blank/transitioning).

**Acceptance Scenarios**:

1. **Given** a Rhai script with `chart.effect(slide_in("left", 400))`, **When** rendered to a panel,
   **Then** the content slides in from the left over 400ms instead of using the default fade.
2. **Given** a Rhai script with `chart.effect(pulse("sting", 200))`, **When** rendered as a panel
   update, **Then** the panel flashes the theme's error color and returns to normal over 200ms.
3. **Given** a Rhai script with `chart.effect(slide_in("left", 5000))` (over the max), **When**
   processed, **Then** the duration is clamped to 2000ms and the effect plays at that duration.
4. **Given** `visual_level = "none"`, **When** a Rhai script attaches an effect, **Then** the effect
   is silently dropped and the content renders immediately without animation.

---

### User Story 5 — Honeycomb effect presets for bee's own chrome (Priority: P3)

The harness itself (not the agent) uses effects for its own UI transitions:

- **Session start**: the header fades in, the footer slides up from the bottom.
- **Tool call → result**: the tool-call line pulses the `accent` role briefly when the result arrives.
- **Episode pass/fail**: the final status line pulses `success` (green) or `error` (red) for 500ms.
- **Panel column appears**: when the first panel is created, the panel column expands from zero width
  using the `stretch` effect.
- **Bee mascot** (opt-in): the bee sprite dissolves in from noise on session start, using
  `evolve_in(600)` instead of the current pop-in.

These are hardcoded in the harness chrome, not agent-controlled. They use the same `EffectManager`
and the same tachyonfx pipeline. The agent cannot override them.

**Why this priority**: P3 because they are polish, not functionality. The TUI works fine without them.
They can be done one at a time, independently, after the core pipeline (US1) is in place.

**Independent Test**: Start a session, verify the header fade-in plays (buffer cells at t=0 are
background-colored, at t=300ms are the final header colors). Each preset is independently testable as
a buffer snapshot at known time offsets.

**Acceptance Scenarios**:

1. **Given** a session start, **When** the first frame draws, **Then** the header text fades from the
   background color to the `info` role over 300ms.
2. **Given** the bee mascot is enabled, **When** the session starts, **Then** the sprite evolves from
   block characters to the bee over 600ms.
3. **Given** `NO_COLOR=1`, **When** chrome effects play, **Then** text-based effects (evolve, dissolve)
   still play; color-based effects (fade, pulse, glow) are skipped.
4. **Given** `visual_level = "none"` and animations enabled, **When** the session starts, **Then**
   chrome effects still play in full — the level restricts the agent, not the harness's own UI.
5. **Given** `BEE_NO_ANIMATION=1`, **When** the session starts, **Then** no chrome effect plays: the
   header, footer, and mascot appear in their final state on the first frame, and the render loop
   never enters the 60fps state.

---

### Edge Cases

- **No `tui` feature**: when built without the `tui` feature, none of this code compiles. The inline
  REPL (rustyline) is unchanged. Effects are a TUI-only capability.
- **Effects on a headless buffer (inline REPL)**: if the `render` tool produces an `EffectSpec` but
  the session is running in inline mode (not full-screen), the effect is stripped and the content
  renders as static ANSI lines through `ExternalPrinter` — identical to today.
- **tachyonfx and `NO_COLOR`**: tachyonfx's color effects (fade, hsl_shift, paint) produce ANSI color
  escapes. When `NO_COLOR` is set, the `EffectManager` filters to text-only effects (dissolve,
  coalesce, evolve, slide, translate). Color effects are replaced with an instant no-op.
- **Multiple panels transitioning at once**: each panel has its own effect slot. Two panels can
  transition in parallel — the `EffectManager` applies each to its panel's buffer region. No conflict.
- **Very fast updates during a transition**: if the model updates a panel 10 times in 100ms, each
  update cancels the running transition and starts a new one from the current buffer state. The visual
  result is a fast series of short transitions — not ideal, but not broken. The coalescing in
  `PanelRegistry` (FR-011 from 008) reduces this: between redraws, only the last update survives.
- **Effect on an empty/zero-size region**: if the panel's allocated area is 0×0 (possible in a very
  narrow terminal), the effect is a no-op. No crash, no work.
- **Overlay dismissed during an effect**: if the operator dismisses a full-screen overlay while its
  entrance effect is still playing, the effect is cancelled, the overlay fade-out plays from whatever
  state the buffer was in, and the chat reappears.
- **Rapid overlay toggle**: pressing Esc to dismiss and then the model immediately requesting a new
  overlay: the dismiss fade-out completes before the new overlay's entrance effect begins. No overlap.
- **Terminal resize during an effect**: the `EffectManager`'s areas are recomputed on resize. Active
  effects are cancelled (their target region changed) and the content renders immediately at the new
  size. This is the same "cancel and re-render" strategy 008-grid-tui uses for resize.
- **Frame rate**: when effects are active, the TUI render loop ticks at ~60fps (16ms), driven by
  `EffectManager::is_running()`. When effects finish but an overlay is still showing, it drops to 1Hz
  to advance the countdown. When neither holds, it returns to the event-driven "only redraw when
  something changes" idle mode (SC-003 from 008).
- **Animations disabled**: with `BEE_NO_ANIMATION` (or `animations = false`), every effect resolves to
  an instant no-op and the 60fps tick mode is never entered. This is the mode CI and batch runs use,
  and the accommodation for operators who need a still screen or are on a high-latency link. It is
  independent of `NO_COLOR` and of `visual_level` — a session may have full color, full takeover
  rights, and zero motion.

---

## Requirements

### Functional Requirements

**Effects pipeline (P1)**

- **FR-001**: The TUI render loop MUST integrate a tachyonfx `EffectManager` that processes active
  effects after widgets are rendered to the `Buffer` and before the buffer is flushed to the screen.
- **FR-002**: The render loop MUST have exactly three scheduling states, checked in this order:
  1. **Motion** — `EffectManager::is_running()` is true: tick at approximately 60fps (16ms interval).
  2. **Countdown** — no effects are running but an overlay is active: tick at 1Hz, enough to advance
     the dismiss hint's countdown (FR-015) and to fire TTL expiry (FR-018).
  3. **Idle** — no effects and no overlay: fully event-driven, no periodic redraws (SC-003 from
     008-grid-tui).

  State 2 exists solely because a faded-in overlay has no active effects yet still needs a redraw per
  second. Scheduling MUST NOT hold state 1 for an overlay's lifetime — a 120s overlay must cost ~120
  wakeups, not ~7200.
- **FR-003**: When a panel is created (first upsert to a new name), the system MUST apply a default
  entrance effect (fade-in, 200–400ms configurable). The effect uses the theme's `info` role as the
  fade-from color.
- **FR-004**: When a panel's content is replaced (subsequent upsert to the same name), the system MUST
  apply a default update transition (parallel dissolve-out of old content + coalesce-in of new
  content, 300–500ms configurable).
- **FR-005**: When a second update arrives at a panel while a transition is still playing, the system
  MUST cancel the running transition and start a new transition from the current buffer state. The
  panel name is the `EffectManager::unique` key, which provides this cancellation directly.
- **FR-006**: Under `NO_COLOR`, color-based effects (fade, hsl_shift, paint, saturate, lighten,
  darken, glow, pulse) MUST be replaced with instant no-ops. Text-based effects (dissolve, coalesce,
  evolve, slide, sweep, translate) MUST still play.

**Motion control (P1)**

- **FR-006a**: The system MUST support an animation kill switch, settable as
  `[harness] animations = false` in scenario TOML, `--no-animation` on the CLI, and
  `BEE_NO_ANIMATION` in the environment (any value, matching the `NO_COLOR` convention in
  `viz/palette.rs`). Precedence: CLI > env > scenario > default. The default is animations enabled.
- **FR-006b**: When animations are disabled, EVERY effect — agent-requested (FR-022), automatic panel
  transitions (FR-003/FR-004), overlay entrance and fade-out (FR-018), and harness chrome (FR-026) —
  MUST resolve to an instant no-op. Content MUST appear in its final state on the next redraw.
- **FR-006c**: When animations are disabled, the render loop MUST NOT enter the 60fps tick mode of
  FR-002 under any circumstance. An animation-disabled session performs zero periodic redraws, exactly
  as an idle session does.
- **FR-006d**: The three presentation axes — `NO_COLOR` (color), animations (motion), and
  `visual_level` (screen real estate) — MUST be independent and compose freely. In particular,
  `visual_level = "none"` MUST NOT silence harness chrome effects: `visual_level` governs what the
  agent may claim, not how the harness draws its own UI. Disabling motion is the only way to silence
  chrome.

**Visual permissions (P2)**

- **FR-007**: The system MUST support a `visual_level` configuration key with four values: `none`,
  `panels`, `panels-wide`, `takeover`. The default MUST be `panels`.
- **FR-008**: `visual_level` MUST be settable in scenario TOML (`[harness] visual_level = "panels"`),
  as a CLI flag (`--visual-level panels`), and as an environment variable (`BEE_VISUAL_LEVEL=panels`).
  Precedence: CLI > env > scenario > default.
- **FR-009**: When the agent requests a visual operation above its permitted level, the system MUST
  silently downgrade the visual to the highest allowed form and include a short downgrade note in the
  `ToolResult.content` summary (so the model can adjust its behavior).
- **FR-010**: At level `none`, the `render` tool MUST still work: it produces `RenderSpec` values and
  returns text summaries. But all output goes inline in chat; no panels, no effects.
- **FR-011**: At level `panels`, panel width MUST NOT exceed 1/3 of the terminal width (or the
  existing 008-grid-tui `panel_w` cap, whichever is smaller).
- **FR-012**: At level `panels-wide`, panel width MUST be allowed up to 1/2 of the terminal width.
- **FR-013**: At level `takeover`, the system MUST allow full-screen overlays in addition to panels.

**Full-screen overlay (P2)**

- **FR-013a**: The Rhai API MUST expose `render_fullscreen(widget)` and
  `render_fullscreen_ttl(widget, ttl_ms)` as the only way for the agent to request an overlay. They
  are commit verbs in the same family as `render` / `render_to` / `render_to_ttl`, and they carry the
  `Overlay` target explicitly so the visual gate can act before any state mutates. `ttl_ms` MUST be
  `> 0`; non-positive values are a script error, matching `render_to_ttl`.
- **FR-014**: A full-screen overlay MUST cover the chat area but MUST NOT cover the input line. The
  operator can always type and send messages while an overlay is showing.
- **FR-015**: Every overlay MUST display a dismiss hint in its bottom row:
  "`Esc` to dismiss · auto-dismiss in {N}s". The countdown MUST update each second, driven by the 1Hz
  countdown state of FR-002. When animations are disabled (FR-006b) the countdown still updates — it
  is information, not motion.
- **FR-016**: The operator MUST be able to dismiss the overlay instantly by pressing `Esc`, from
  either focus (input line or chat). The dismiss check MUST sit directly below the help-overlay
  swallow and above the focus dispatch in `handle_key`, so overlay dismiss outranks the existing
  `Esc` bindings (clear input, hide panel column). Those bindings are unchanged and take effect on
  the next `Esc` once no overlay is active. `q` MUST NOT dismiss the overlay — it retains its
  existing meaning (quit) in every context, so the operator never has a key whose destructiveness
  depends on whether an overlay happens to be showing.
- **FR-017**: Every overlay MUST have a TTL. The default is 30 seconds. The scenario TOML key
  `takeover_ttl_secs` overrides it. The maximum configurable TTL is 120 seconds.
- **FR-018**: When the TTL expires, the overlay MUST fade out over 200ms and the chat area MUST
  reappear with its scroll position preserved.
- **FR-019**: When the model sends a new chat message (its next assistant turn), any active overlay
  MUST be automatically dismissed (with fade-out).
- **FR-020**: At most one overlay MAY be active at a time. A second overlay request MUST replace the
  first (cross-fade transition, no stacking).
- **FR-021**: The TTL is a hard maximum. Even if the model's Rhai script requests a longer duration
  via `render_fullscreen_ttl(w, ttl_ms)`, the configured `takeover_ttl_secs` wins. A shorter request
  is honored as given.

**Agent-requested effects via Rhai (P3)**

- **FR-022**: The Rhai rendering API MUST expose a curated set of effect constructors (listed in the
  Rendering API section) that return an `Effect` type the model attaches to a widget via
  `widget.effect(e)`.
- **FR-023**: Effect durations MUST be clamped to 100–2000ms. Durations outside this range are clamped
  silently.
- **FR-024**: If the visual level does not permit the operation (e.g., any effect at level `none`),
  the effect MUST be silently dropped and the content MUST render immediately without animation.
- **FR-025**: The effect constructors MUST be registered on the same Rhai `Engine` as the existing
  rendering API (003-visual-render). No separate engine.

**Harness chrome effects (P3)**

- **FR-026**: The harness MUST apply its own entrance/exit effects to UI chrome (header, footer, panel
  column appear/disappear) independently of the agent's effects.
- **FR-027**: Chrome effects MUST use the same `EffectManager` and tachyonfx pipeline as agent effects.
- **FR-028**: Chrome effects MUST NOT be overridable by the agent.

### Key Entities

- **`EffectSpec`** (new, serde): A declarative description of an effect, produced by Rhai and stored
  in the `RenderSpec` (as an optional field on each variant). Pure data — no tachyonfx types. The TUI
  layer resolves it to a live tachyonfx effect at render time. Variants: `FadeIn { ms }`,
  `FadeOut { ms }`, `DissolveIn { ms }`, `DissolveOut { ms }`, `SlideIn { direction, ms }`,
  `SlideOut { direction, ms }`, `SweepIn { direction, ms }`, `SweepOut { direction, ms }`,
  `Pulse { color, ms }`, `Glow { ms }`, `EvolveIn { ms }`, `EvolveOut { ms }`.

- **`VisualLevel`** (new, serde): The four permission tiers: `None`, `Panels`, `PanelsWide`,
  `Takeover`. Parsed from TOML/CLI/env. Defaults to `Panels`. Governs screen real estate for the
  agent only — orthogonal to color and to motion.

- **`MotionSetting`** (new, serde): The animation kill switch — enabled (default) or disabled.
  Sourced from `[harness] animations`, `--no-animation`, and `BEE_NO_ANIMATION` with CLI > env >
  scenario > default precedence. When disabled, every `EffectSpec` resolves to an instant no-op and
  the render loop's motion state (FR-002 state 1) is unreachable. Applies to harness chrome as well
  as agent effects.

- **`Overlay`** (new): The full-screen takeover state in `App`: the `RenderSpec` being shown, the
  instant it was created, the resolved TTL (requested value clamped by `takeover_ttl_secs`), and the
  active entrance/exit effect handle. At most one exists. Its presence is what puts the render loop
  into the 1Hz countdown state.

- **`EffectSlot`** (new): Per-panel metadata tracking the active tachyonfx effect handle (if any) and
  the "previous buffer" snapshot (for dissolve transitions that need to know what the old content
  looked like). Lives inside `Panel` in the `PanelRegistry`. The panel's name doubles as the
  `EffectManager::unique` key, which is what makes FR-005's cancel-and-restart automatic.

---

## Rendering API (Rhai-registered effect functions)

These are registered on the existing Rhai `Engine` alongside the 003-visual-render drawing API. They
produce `EffectSpec` values that get attached to widgets, not live tachyonfx objects (the Rhai sandbox
never sees tachyonfx types).

| Function | Rhai signature | Description |
|----------|---------------|-------------|
| `fade_in(ms)` | `fn(i64) -> Effect` | Fade from background to content. |
| `fade_out(ms)` | `fn(i64) -> Effect` | Fade from content to background. |
| `dissolve_in(ms)` | `fn(i64) -> Effect` | Characters coalesce from random noise. |
| `dissolve_out(ms)` | `fn(i64) -> Effect` | Characters dissolve into random noise. |
| `slide_in(dir, ms)` | `fn(String, i64) -> Effect` | Content slides in from a direction. |
| `slide_out(dir, ms)` | `fn(String, i64) -> Effect` | Content slides out to a direction. |
| `sweep_in(dir, ms)` | `fn(String, i64) -> Effect` | A colored wave reveals content. |
| `sweep_out(dir, ms)` | `fn(String, i64) -> Effect` | A colored wave hides content. |
| `pulse(color, ms)` | `fn(String, i64) -> Effect` | Flash a color, then fade back. |
| `glow(ms)` | `fn(i64) -> Effect` | Lighten toward white and back. |
| `evolve_in(ms)` | `fn(i64) -> Effect` | Characters morph through block chars to real content. |
| `evolve_out(ms)` | `fn(i64) -> Effect` | Content devolves into blocks, then disappears. |
| `widget.effect(e)` | `fn(&mut Widget, Effect)` | Attach an effect to any widget type. |

### Takeover commit functions

The existing commit verbs are `render(widget)` (inline into chat) and `render_to(id, widget)` /
`render_to_ttl(id, widget, ttl_ms)` (named side panel). Full-screen takeover adds a third pair, in
the same shape:

| Function | Rhai signature | Description |
|----------|---------------|-------------|
| `render_fullscreen(w)` | `fn(RenderSpec)` | Commit a widget as a full-screen overlay, using the configured default TTL. |
| `render_fullscreen_ttl(w, ttl_ms)` | `fn(RenderSpec, i64)` | Same, but request a specific TTL. The configured `takeover_ttl_secs` is a hard cap (FR-021); a longer request is clamped down, a shorter one is honored. |

The target is explicit at the call site rather than encoded in a widget property or a magic panel
id, so the visual gate (FR-009) can downgrade the commit before any panel or overlay state mutates.
`render_fullscreen*` at a level below `takeover` downgrades to a panel; at level `none` it
downgrades to inline. `ttl_ms` must be `> 0` (same validation as `render_to_ttl`).

Direction strings: `"left"`, `"right"`, `"top"`, `"bottom"`. Unknown values default to `"left"`.

Color strings for `pulse`: any theme role name (`"success"`, `"error"`, `"info"`, `"accent"`) or a hex
color (`"#FF0000"`). Unknown values default to the `info` role.

---

## Architecture

### Where the effects pipeline sits in the render loop

```text
  ┌──────────────────────────────────────────────────────────┐
  │  App state (model)                                        │
  │  ├─ chat: Vec<ChatMessage>                                │
  │  ├─ panels: PanelRegistry (each Panel has an EffectSlot)  │
  │  ├─ overlay: Option<Overlay>                              │
  │  └─ visual_level: VisualLevel                             │
  └──────────────────┬───────────────────────────────────────┘
                     │
  ┌──────────────────▼───────────────────────────────────────┐
  │  view(&App, &mut Frame)                                   │
  │                                                           │
  │  1. Layout: header · chat · [overlay | panels] · input    │
  │  2. Render widgets to Buffer (ratatui)     ← existing     │
  │  3. Apply effects to Buffer (tachyonfx)    ← NEW          │
  │  4. Render dismiss hint on overlay         ← NEW          │
  │  5. Buffer → screen (crossterm flush)      ← existing     │
  └──────────────────────────────────────────────────────────┘
                     │
  ┌──────────────────▼───────────────────────────────────────┐
  │  Event loop — three scheduling states (FR-002)             │
  │                                                           │
  │  if effect_manager.is_running():                          │
  │      tick every 16ms → advance effects        (motion)    │
  │  elif overlay.is_some():                                  │
  │      tick every 1s → advance countdown, TTL   (countdown) │
  │  else:                                                    │
  │      wait for input/session event (no timer)  ← existing  │
  │                                                           │
  │  if animations disabled → motion state is unreachable      │
  │                                                           │
  │  on Esc (any focus, above focus dispatch) → dismiss        │
  │  on TTL expiry → dismiss overlay                          │
  │  on assistant reply → dismiss overlay                     │
  └──────────────────────────────────────────────────────────┘
```

### Visual permission enforcement

```text
  ┌─────────────────────────────────────────────────────────────────┐
  │  Rhai commit verb → target                                       │
  │    render(w)                     → Inline                        │
  │    render_to(id, w)              → Panel{id}                     │
  │    render_to_ttl(id, w, ms)      → Panel{id, ttl}                │
  │    render_fullscreen(w)          → Overlay{default ttl}   ← NEW  │
  │    render_fullscreen_ttl(w, ms)  → Overlay{ttl}           ← NEW  │
  └───────────────────────────┬─────────────────────────────────────┘
                              │
  ┌───────────────────────────▼─────────────────────────────────────┐
  │  render tool → ToolResult { render_spec, target, effect_spec }  │
  └───────────────────────────┬─────────────────────────────────────┘
                              │
  ┌───────────────────────────▼─────────────────────────────────────┐
  │  Visual gate (in the session event handler, before panel apply)  │
  │                                                                  │
  │  match (visual_level, target) {                                  │
  │    (None,   Panel{..})     → downgrade to Inline, note in result │
  │    (None,   Overlay)       → downgrade to Inline, note           │
  │    (Panels, Overlay)       → downgrade to Panel,  note           │
  │    (Panels, Panel{wide})   → cap width to 1/3                   │
  │    (PanelsWide, Overlay)   → downgrade to Panel,  note           │
  │    (PanelsWide, Panel{..}) → cap width to 1/2                   │
  │    (Takeover, Overlay)     → allow, enforce TTL cap              │
  │    (_, Inline)             → always pass through                 │
  │  }                                                               │
  │                                                                  │
  │  Effects are stripped entirely at level None.                     │
  │  The motion switch is applied later, in tui/effects.rs — it is    │
  │  not a permission and does not produce a downgrade note.          │
  └──────────────────────────────────────────────────────────────────┘
```

**Resolved in [research.md R7](./research.md)**: an `Overlay` downgraded to a `Panel` upserts the
reserved panel id `"takeover"`. A fixed id gives FR-020's replace-don't-stack behavior for free and
avoids unbounded panel growth against 008's "panel column is full" path (`render_api.rs:155`). The id
satisfies the existing `[a-z0-9_-]` validator unchanged, but is reserved from agent use — a script
calling `render_to("takeover", w)` directly is a script error.

### Security boundaries (unchanged)

The effects pipeline runs inside the harness process, after the Rhai sandbox and after the
`RenderSpec` has been produced. tachyonfx operates on a ratatui `Buffer` — an in-memory grid of
characters. It has no I/O, no FFI, no network access. It does not interact with the kernel sandbox.
The Rhai engine never sees tachyonfx types — it produces `EffectSpec` values (pure serde data), and
the TUI layer resolves them to live effects.

No existing security boundary is changed. No existing sandbox is weakened. This is the same
architecture as the existing sprite animation (003-visual-render, FR-030): the Rhai script declares
intent, the harness executes it.

---

## Success Criteria

### Measurable Outcomes

- **SC-001**: A panel upsert (first creation) produces a visible fade-in transition: at t=0 the panel
  region matches the background; at t=150ms the cells have intermediate colors; at t=300ms the cells
  match the widget's final colors. Verified by buffer snapshot comparison in an `insta` test.
- **SC-002**: A panel update (second upsert to the same name) produces a dissolve/coalesce transition:
  at t=0 the old content is visible; at t=200ms some characters are random glyphs; at t=400ms the
  new content is visible. Verified by buffer snapshot.
- **SC-003**: An idle session — no active effects **and** no active overlay — performs zero periodic
  redraws (verified by a redraw counter in the test harness — the same counter as SC-003 from
  008-grid-tui). Additionally, a session showing a fully faded-in overlay for N seconds performs
  approximately N redraws, not 60N: the counter after a 10s overlay with no effects reads ≤ 15.
- **SC-004**: At `visual_level = "none"`, the `render` tool produces a `ToolResult` whose `content`
  includes "downgraded to inline" when the model requests a panel, and no panel appears.
- **SC-005**: At `visual_level = "panels"`, the maximum panel width is ≤ 1/3 of the terminal width
  (verified by measuring the panel `Rect` in a test at a known terminal size).
- **SC-006**: A full-screen overlay at `visual_level = "takeover"` is dismissed by `Esc` within one
  frame (≤ 16ms from keypress to redraw without overlay). The chat area's scroll position is
  unchanged.
- **SC-007**: An overlay whose TTL expires is automatically dismissed — the overlay is absent from the
  buffer after `takeover_ttl_secs + 0.2s` (TTL plus fade-out).
- **SC-008**: A Rhai script that attaches `slide_in("left", 400)` to a bar chart produces an
  `EffectSpec::SlideIn` in the `RenderSpec`, and the TUI resolves it to a tachyonfx `slide_in` effect
  that is active for 400ms. Verified by buffer snapshots at t=0, t=200ms, t=400ms.
- **SC-009**: `tachyonfx` does not appear in `bee-core`'s or `bee-common`'s dependency tree
  (`cargo tree -p bee-core` / `cargo tree -p bee-common` shows neither).
- **SC-010**: Under `NO_COLOR=1`, no ANSI color escape sequences appear in any effect output. Text
  effects (dissolve, evolve) still produce character mutations (verified by checking that buffer
  cell characters change over the effect duration even though `.fg` and `.bg` do not).
- **SC-011**: Under `BEE_NO_ANIMATION=1`, a session that creates panels, replaces panel content,
  raises an overlay, and starts up (chrome effects) performs **zero** periodic redraws — the same
  redraw counter used by SC-003 reads identically to an idle session. Every buffer snapshot taken at
  t=0 already shows final content, with no intermediate colors and no substituted glyphs.
- **SC-012**: The three axes compose: a session with `NO_COLOR=1`, animations enabled, and
  `visual_level = "none"` still plays harness chrome text effects (character mutations observable in
  the buffer) while producing no panels and no color escapes. Verified by one test asserting all
  three properties at once.

---

## Dependencies

### New crate dependencies (bee-harness only, behind `tui` feature gate)

```toml
[dependencies]
tachyonfx = { version = "0.25", default-features = false, optional = true }
```

tachyonfx 0.25.1 depends on `ratatui-core 0.1.2` — the exact version ratatui 0.30.2 pulls in, so the
two unify on one `Buffer`/`Cell` type. **This means the ratatui dependency must be bumped from 0.29
to 0.30.** The bump is a prerequisite, not an option: tachyonfx cannot see ratatui 0.29's `Buffer`.

ratatui 0.30 is a facade split — the crate re-exports `ratatui-core 0.1.2`, `ratatui-widgets 0.3.2`,
and `ratatui-crossterm 0.1.2`. **For bee's usage the split is invisible**: every `use ratatui::…`
path still resolves, and the bump requires no source changes at all. This was verified empirically
(see [research.md R1](./research.md)) — with only the manifest edited,
`cargo check -p bee-harness --features tui` reports zero errors and zero warnings, the headless build
is clean, and `cargo test -p bee-harness --features tui` passes **291 tests across 28 suites with
zero failures**, including all of 008-grid-tui's coverage.

Two dependency facts:

- **crossterm moves 0.28 → 0.29.** `ratatui-crossterm 0.1.2` defaults to `crossterm_0_29`, ratatui
  declares it without `default-features = false`, and its `cfg_if` picks 0.29 whenever both version
  features are enabled. Enabling `ratatui/crossterm_0_28` therefore does **not** hold crossterm at
  0.28 — it only adds a second crossterm build, leaving two crossterm instances driving the same tty.
  bee moves its own dependency to 0.29 instead, giving a single crossterm. Verified: no source
  changes, 291/291 tests pass including 008's terminal-restore matrix and `RestoreGuard`. See
  [research.md R2/D1](./research.md).
- `ratatui-core 0.1.2` declares `unicode-width = ">=0.2.0"`, replacing ratatui 0.29's hard `=0.2.0`
  pin. The `rustyline = "17"` pin exists solely because of that conflict, so it MAY be lifted after
  the bump. Lifting it is explicitly **out of scope** for this feature — the pin stays at 17, and was
  confirmed still passing under 0.30.

**Scope**: the bump is Phase 0 of this feature — the first task group in `tasks.md`, on the same
branch and in the same PR as the effects work. Phase 0 is a **manifest-only change**. Its gate is the
full 008-grid-tui test suite green with zero behavior change, which the research probe has already
demonstrated. No tachyonfx code is written until Phase 0's gate passes in CI.

The `tui` feature gate in `Cargo.toml` gains the new dep:

```toml
ratatui = { version = "0.30", default-features = false }
crossterm = { version = "0.29", features = ["event-stream"], optional = true }
tachyonfx = { version = "0.25", default-features = false, optional = true }

tui = ["dep:crossterm", "dep:color-eyre", "dep:textwrap", "dep:tachyonfx",
       "ratatui/crossterm", "tokio/sync"]
```

### What is NOT added

- No new dependencies outside `bee-harness`.
- No changes to `bee-core`, `bee-common`, `bee-ebpf`, or `bee-cli`.
- `tachyonfx` does not propagate outside `bee-harness` (enforced by `optional = true` + feature gate).

---

## Project Structure

### New files

```text
bee-harness/src/
├── tui/
│   ├── effects.rs           # EffectManager integration: owns tachyonfx state, drives the
│   │                        # 60fps tick, resolves EffectSpec → live tachyonfx effects,
│   │                        # handles NO_COLOR filtering and the animations kill switch,
│   │                        # manages per-panel EffectSlots (panel name = unique key)
│   ├── overlay.rs           # Full-screen overlay: state, TTL countdown, dismiss logic,
│   │                        # entrance/exit effects, the dismiss-hint widget
│   └── visual_gate.rs       # Visual permission enforcement: reads VisualLevel, downgrades
│                            # targets that exceed the level, produces downgrade notes
├── render_spec/
│   └── effect_spec.rs       # EffectSpec enum (serde): the declarative effect descriptions
│                            # produced by Rhai, stored in RenderSpec, resolved by tui/effects
└── render_api/
    └── effect_api.rs        # Rhai function registrations for effect constructors
```

### Modified files

```text
bee-harness/Cargo.toml           # Phase 0: ratatui 0.29 → 0.30 + crossterm 0.28 → 0.29
                                 # (manifest-only, no source changes; rustyline stays 17);
                                 # then add tachyonfx (optional, tui-gated)
bee-harness/src/render_spec.rs   # RenderSpec variants gain optional effect_spec: Option<EffectSpec>
bee-harness/src/render_api.rs    # register effect constructors + render_fullscreen/_ttl commit verbs
bee-harness/src/tui/app.rs       # App gains overlay/visual_level/motion; Esc dismiss above focus
                                 # dispatch in handle_key (below the help-overlay swallow)
bee-harness/src/tui/view.rs      # view() calls EffectManager::process_effects after widget render
bee-harness/src/tui/panels.rs    # Panel gains EffectSlot; PanelRegistry.upsert triggers effects
bee-harness/src/tui/mod.rs       # event loop: three scheduling states (60fps / 1Hz / idle)
bee-harness/src/config.rs        # parse visual_level + animations from TOML/CLI/env
bee-harness/src/tools/render.rs  # pass EffectSpec and Overlay target through to ToolResult
```

Phase 0 adds nothing to this list: the ratatui bump is manifest-only
([research.md R1](./research.md)).

### Spec files

```text
specs/009-tachyonfx-effects/
├── spec.md                      # this file
├── contracts/
│   ├── effect-spec.md           # EffectSpec serde contract
│   ├── visual-levels.md         # permission tiers and downgrade rules
│   ├── overlay-lifecycle.md     # overlay creation, TTL, dismiss, fade-out, Esc precedence
│   ├── rhai-effect-api.md       # the agent-facing effect constructor + render_fullscreen API
│   └── motion-control.md        # the animations kill switch and how the three presentation
│                                # axes (NO_COLOR / animations / visual_level) compose
└── examples/
    ├── panel-fade-in.rhai       # agent script: render a chart with a fade-in
    ├── slide-dashboard.rhai     # agent script: a grid layout with directional slides
    ├── status-pulse.rhai        # agent script: a dot grid that pulses on update
    └── fullscreen-chart.rhai    # agent script: a full-screen chart (takeover)
```

---

## Constitution Check

| Principle | Assessment |
|-----------|------------|
| **I. Deny-by-Default & Fail-Closed** | PASS. Visual level defaults to `panels` (restricted). Takeover is opt-in. Unknown levels are rejected. Effects above the permitted level are silently downgraded, never silently escalated. |
| **II. Capability Attenuation** | N/A. Visual permissions are not in the enforcement/scope path. They do not interact with cgroup scopes or policy attenuation. |
| **III. Kernel Enforcement Is Authoritative** | PASS. This feature does not touch the kernel sandbox. It is a presentation layer. |
| **IV. Policy-as-Data** | PASS. `visual_level` and `animations` are declared in TOML, the same policy-as-data format as everything else. `EffectSpec` is serde data recorded in the transcript. |
| **V. Library-First, Runtime-Free Core** | PASS. `tachyonfx` is confined to `bee-harness` behind the `tui` feature gate. `bee-core` and `bee-common` gain no new dependencies. `EffectSpec` is a pure serde type with no tachyonfx imports — it lives in `render_spec.rs` alongside the existing types, not in the tui module. |
| **Security/Platform** | PASS. tachyonfx operates on an in-memory `Buffer`. No I/O, no FFI, no network, and no `rand` — it carries its own deterministic `SimpleRng`. The Rhai sandbox is unchanged: it produces `EffectSpec` data and target-carrying commit verbs, never live tachyonfx objects. The visual gate prevents the agent from exceeding the operator's configured permission. The TTL prevents indefinite screen takeover, and `Esc` outranks every competing binding so the operator's escape hatch cannot be shadowed. |

**Result: no violations.**

---

## Assumptions

- ~~**ratatui 0.29 → 0.30 is a mechanical migration**~~ — **superseded by measurement.** The bump
  requires no source changes whatsoever: 291 tests across 28 suites pass with only `Cargo.toml`
  edited ([research.md R1](./research.md)). Import paths do not move for bee's usage. The one real
  dependency consequence is crossterm, not ratatui — see [research.md R2](./research.md). The
  `rustyline = "17"` pin stays, verified still green.
- **tachyonfx's `EffectManager` is the right abstraction**: it owns effects, advances them per-tick
  (`process_effects(duration, buf, area)`), and reports activity via `is_running()` for switching back
  to idle mode. Its `unique(key, fx)` API cancels any running effect sharing a key, which is exactly
  the per-panel cancel-and-restart semantics FR-005 requires — panel name is the key. tachyonfx also
  carries its own deterministic `SimpleRng` (no `rand` dependency, fixed default seed), so the
  character-scatter effects are reproducible and the SC-001/SC-002 buffer snapshots are stable.
- **The 60fps tick is acceptable**: when effects are active, the event loop polls at ~16ms. This is
  standard for terminal animations (tachyonfx's own examples use this rate). When effects finish, the
  loop returns to idle. A session that never triggers an effect never ticks.
- **The curated Rhai effect set is sufficient**: the 12 exposed effects cover the useful 90% of
  tachyonfx's surface. Exposing the full tachyonfx API (spatial patterns, cell filters, custom shader
  functions) is a future extension, not scoped here. The Rhai API is additive — new effects can be
  registered later without breaking existing scripts.
- **Visual level "none" is viable for testing and CI**: level "none" disables all visual output beyond
  inline chat, so batch/CI runs that do not care about visual polish can suppress effects entirely.
  This is the same strategy as the existing `--no-color` / `NO_COLOR` handling.
- **Overlay TTL of 30 seconds is reasonable**: long enough for the operator to read a visualization,
  short enough to not be annoying. The operator can always dismiss earlier. The maximum of 120 seconds
  prevents pathological configs.
