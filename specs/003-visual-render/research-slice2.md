# Research: bee Visual Rendering — Slice 2 (sprites, animation, the bee)

Phase 0 decisions for the sprite + animation subsystem (FR-028…FR-032, SC-015…SC-018). Builds on
Slice 1's seams: `RenderSpec` is `#[non_exhaustive]` (variants added additively), the `render` tool's
`Arc`-free owned `Engine` (additive `register_fn`), and the spinner's `spinning`/`reclaim`/`spin_task`
machinery (generalized to N rows). No new crate dependencies — sprite rendering is hand-rolled ANSI;
the animator reuses tokio (already in `bee-harness`).

Decision IDs continue from Slice 1 (which ended at D11).

---

## D12 — Half-block pixel packing

**Decision**: Render a `W×H`-pixel sprite as `W` columns × `⌈H/2⌉` terminal rows by packing two
vertical pixels into one cell, choosing the block glyph by which halves are opaque:

| Top pixel (row 2r) | Bottom pixel (row 2r+1) | Glyph | Colors emitted |
|--------------------|-------------------------|-------|----------------|
| opaque | opaque | `▄` U+2584 | `bg` = top, `fg` = bottom |
| opaque | transparent | `▀` U+2580 | `fg` = top, no bg |
| transparent | opaque | `▄` U+2584 | `fg` = bottom, no bg |
| transparent | transparent | space | none |

**Rationale**: the lower-half block `▄` paints its **foreground** in the lower half and lets the cell
**background** show in the upper half — so one cell renders both pixels with one glyph. Switching to
the upper-half block `▀` for the "only-top-opaque" case means a transparent half **never emits a
background escape** (SC-015) — the terminal's own background shows through. Odd `H`: the final row has
only a top pixel → treated as "top opaque, bottom transparent" (`▀`). A fully-transparent sprite is
`⌈H/2⌉` rows of spaces; the tool summary notes "sprite is fully transparent" (Edge Case).

**Alternatives**: one full block `█` per pixel (2 cells tall per pixel row) — rejected: doubles the
footprint, and the spec fixes the half-block technique. Sixel/kitty graphics protocols — rejected:
terminal-specific, not universally supported, and outside the "ANSI through `ExternalPrinter`" model.

---

## D13 — Truecolor escapes + graceful color degradation

**Decision**: Emit 24-bit SGR (`\x1b[38;2;R;G;Bm` fg, `\x1b[48;2;R;G;Bm` bg) when the terminal
supports it; otherwise quantize. `viz::sprite::detect_color_mode() -> ColorMode`:

- `TrueColor` — `COLORTERM` is `truecolor` or `24bit`.
- `Ansi256` — else if `TERM` contains `256color`.
- `Ansi16` — otherwise.
- **Non-tty / `NO_COLOR`** — monochrome: opaque → `█`/`▄` with no color, transparent → space (Edge
  Case "Sprites degrade to a monochrome block-character rendering").

`quantize_256(r,g,b) -> u8`: nearest of the xterm-256 palette — the 6×6×6 color cube
(`16 + 36·q(r) + 6·q(g) + q(b)`, `q` mapping 0–255→0–5 at the cube's step points) **and** the 24-step
grayscale ramp (232–255), whichever is closer by squared RGB distance. Emitted as `\x1b[38;5;{n}m` /
`\x1b[48;5;{n}m`. `quantize_16(r,g,b) -> u8`: nearest of the 16 basic ANSI colors by RGB distance,
emitted as `\x1b[{30+n}m` / `\x1b[{40+n}m` (bright via `9x`/`10x`). "Recognizable but not pretty"
(spec) is the accepted quality bar.

**Rationale**: truecolor is common in modern terminals; the two fallbacks keep sprites legible
everywhere without breaking. All three are pure string generation — no crate needed.

---

## D14 — The animator: generalize the spinner's single-row reclaim to N rows

**Decision**: Replace `TerminalOutput`'s `reclaim: AtomicBool` with `reclaim_rows: AtomicUsize`
(0 = none, N = the next real output reclaims N rows in place). `emit()` reads it: when `N > 0`, prepend
"cursor up N rows + clear those N lines" before printing so the drawn block is overwritten with no
residue. The spinner sets `reclaim_rows = 1` (its current behavior, byte-for-byte — SC-012 for the
spinner tests still holds); a finished sprite animation sets `reclaim_rows = ⌈H/2⌉`.

The animation task mirrors the spinner exactly:
1. Emit frame 0's `⌈H/2⌉` rows synchronously (instant first paint).
2. Spawn a tokio task that every `interval_ms` prints `\x1b[{N}A` (up N), redraws N lines of the next
   frame, re-checking the shared active-flag under the printer lock (same as the spinner's re-check).
3. Advance frames per `bounce` (ping-pong sequence `0,1,…,k,…,1`) and stop after `cycles` full cycles
   (or run until interrupted when `cycles == 0`).
4. On completion/interrupt, set `reclaim_rows = N` so subsequent output overwrites the sprite.

**Testability seam (mirrors the spinner's `spinner_frame` split)**: factor the *what/when* apart so the
spec-visible behavior is tested as pure data, never through real time:

```rust
// pure, no tokio, no clock:
pub fn playback(spec: &AnimationSpec) -> Vec<usize>;   // ordered frame indices (applies bounce + cycles)
pub fn redraw_block(rows: &[String]) -> String;        // one in-place redraw: "\x1b[{N}A" + N lines
```

- `playback` encodes the exact order — e.g. 3 frames, `bounce=true`, `cycles=1` → `[0,1,2,1]` (the
  "1-2-3-2" the spec shows, 0-indexed); `bounce=false`, `cycles=2` → `[0,1,2,0,1,2]`; `cycles=0` →
  one period (the task then repeats it until interrupted).
- The tokio task is trivial glue over these — the exact shape of the spinner loop:
  `emit frame0; for idx in playback(spec)[1..] { sleep(interval).await; if !active { break } print(redraw_block(render_frame(&frames[idx]))) } reclaim_rows = N`.

SC-016 is then covered deterministically: (a) ordering → assert `playback(spec)` equals the expected
index vector; (b) cursor-up → assert `redraw_block(..)` starts with `\x1b[{N}A`; (c) reclaim → start,
stop, assert `reclaim_rows == N` (same shape as the existing spinner reclaim test). Only the
`sleep`-between-frames glue relies on real tokio, and it is identical to the already-trusted spinner —
no timing-sensitive assertion needed. This mirrors Slice 1's pure `spinner_frame(tick, start, color)`
tested apart from its spawned loop.

**Rationale**: the spinner already solved in-place redraw + reclaim for one row; sprites are the same
problem at N rows. Generalizing the counter (rather than adding a parallel mechanism) keeps one
code path and preserves the spinner's exact semantics at `N = 1`. The pure `playback`/`redraw_block`
split makes the animation deterministically testable without a mock clock.

**Alternatives**: a separate sprite-animation subsystem independent of the spinner — rejected: it
would duplicate the reclaim/redraw logic and make FR-031 (mutual exclusion) harder.

---

## D15 — One animated element at a time (FR-031)

**Decision**: Unify the spinner and sprite-animation under a single "active animation" slot: the
existing `spinning: Arc<AtomicBool>` becomes the shared active-flag and `spin_task` the shared task
handle. `busy_start()` (spinner) and `start_animation()` (sprite) each first stop whatever is running
(swap the flag false, abort the task), then claim the slot. Starting a sprite animation while the
spinner runs clears `spinning` (SC-017); starting the spinner while an animation runs stops the
animation. A tool call mid-animation stops it and reclaims its rows before the tool result prints
(Edge Case), exactly as `busy_stop` already does for the spinner.

**Rationale**: a single slot makes mutual exclusion structural rather than coordinated — there is only
one `AtomicBool` and one `JoinHandle`, so two animated elements cannot coexist by construction.

---

## D16 — `SpriteSpec` / `AnimationSpec` + additive `RenderSpec` variants

**Decision**: Add `RenderSpec::Sprite { spec: SpriteSpec }` and `RenderSpec::Animation { spec:
AnimationSpec }` to the `#[non_exhaustive]` enum. `SpriteSpec { width: u16, height: u16, pixels:
Vec<Option<(u8,u8,u8)>> }` (row-major, `None` = transparent). `AnimationSpec { frames: Vec<SpriteSpec>,
interval_ms: u64, bounce: bool, cycles: u32 }`. Both derive serde + `PartialEq` (transcript-recordable
like every other `RenderSpec`, so a future viewer can re-render sprites too).

The Slice-1 helper methods (`element_count`, `layout_depth`, `ascii_fallback`, summary) already have
wildcard `_` arms, so the additions compile without touching existing arms — but Slice 2 replaces the
wildcards with explicit `Sprite`/`Animation` arms giving meaningful fallbacks (e.g. `ascii_fallback`
for a sprite = `[sprite 16×16]`, for an animation = `[animation: 3 frames]`) so the test `Collector`
and non-terminal sinks degrade gracefully.

**Rationale**: additive, serde-stable, and keeps the data model ratatui/rhai-free (NFR-002) — sprites
render through hand-rolled ANSI, never ratatui.

---

## D17 — `viz::bee`: the built-in mascot

**Decision**: `viz::bee::sprite() -> SpriteSpec` (the 16×16 bee) and `viz::bee::animation() ->
AnimationSpec` (3-frame wing-flap, 150 ms, `bounce = true`) are built once from a bitmap-string +
palette via `std::sync::OnceLock` (no `Date`/random; deterministic). Consumers:

- **REPL startup** — opt-in via a `--bee` flag or `BEE_MASCOT=1`: play the animation once beside the
  session line, then reclaim (FR-032). Absent in batch mode.
- **Episode completion** — a static bee beside the final status line for `Completed`/`Captured`.
- **Rhai** — `bee_sprite()` / `bee_animation()` return the constants (SC-018).

**Rationale**: one source of truth for the mascot, reused by chrome and the agent API; `OnceLock` fits
the "built once, not per call" posture without the forbidden `Date::now`/`Math::random`.

---

## D18 — Rhai sprite/animation/palette API (additive registration)

**Decision**: Additional `register_fn`/`register_type_with_name` calls on the **same** Slice-1 engine:
`palette()` → `PaletteBuilder` (`HashMap<char, Option<(u8,u8,u8)>>`, max 32 entries); `pal.set(char,
color)` parses `"#RRGGBB"` or `"transparent"`; `sprite(w,h,pal)` → `SpriteBuilder` (validates ≤ 32×32);
`sprite.paint(rows)` maps each character via the palette (unknown → transparent); `sprite.set(x,y,
color)`, `sprite.fill(color)`; `animation(ms)` clamps 50–1000; `anim.add(sprite)` (≤ 16 frames, all
same dims), `anim.bounce(b)`, `anim.cycles(n)`; `bee_sprite()`/`bee_animation()`. `render(widget)` and
`layout.add(widget)` gain `try_cast` arms for the sprite/animation builders. Cap violations return Rhai
errors → `ToolResult::error` (same pattern as Slice 1's structural caps).

**Constraints** (API-enforced): sprite ≤ 32×32, ≤ 16 frames, ≤ 32 palette entries, interval clamped
50–1000 ms. A sprite/animation inside a layout still renders (the layout allots it `⌈H/2⌉` rows);
animations inside a layout render only their **first frame** (a layout is a static composition — noted
in the summary), since the animator drives a single top-level element.

**Rationale**: reuses Slice 1's builder/`RenderContext` pattern; the engine is still built once per
session, so this is pure additive registration.

---

## Summary of decisions

| # | Decision | Key rationale |
|---|----------|---------------|
| D12 | Half-block packing, glyph chosen by opaque halves | one cell = 2 pixels; transparency emits no bg escape |
| D13 | Truecolor + `quantize_256`/`quantize_16`; mono off-tty | legible everywhere; pure ANSI, no crate |
| D14 | Generalize `reclaim` to `reclaim_rows: AtomicUsize`; animator mirrors spinner | one redraw/reclaim path; N=1 preserves spinner |
| D15 | Single "active animation" slot (shared flag + task) | FR-031 mutual exclusion is structural |
| D16 | Additive `Sprite`/`Animation` variants; explicit helper arms | serde-stable, ratatui/rhai-free data model |
| D17 | `viz::bee` via `OnceLock`; opt-in REPL, episode, Rhai | one mascot source; no `Date`/random |
| D18 | Additive Rhai palette/sprite/animation registration + caps | same engine/builder pattern; caps as script errors |
