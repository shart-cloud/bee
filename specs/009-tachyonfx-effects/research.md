# Phase 0 Research: Terminal Effects & Agent Visual Permissions

**Feature**: `009-tachyonfx-effects` | **Date**: 2026-07-22 | **Spec**: [spec.md](./spec.md)

All findings below are **empirical** — verified against the actual crate sources in
`~/.cargo/registry` and against real `cargo check` / `cargo test` runs on this working tree, not
inferred from documentation or memory. The working tree was restored to its committed state after
the probes.

---

## R1: Does the ratatui 0.29 → 0.30 bump actually break bee? — **NO**

**Decision**: Phase 0 is a **two-line manifest change**, not a migration. No source file changes.

**Evidence** — the bump was applied to `bee-harness/Cargo.toml` and compiled:

| Probe | Result |
|-------|--------|
| `cargo check -p bee-harness --features tui` | **0 errors, 0 warnings** |
| `cargo check -p bee-harness` (headless, no tui) | **clean** |
| `cargo test -p bee-harness --features tui` | **28 suites, 291 passed, 0 failed** |

This includes 008-grid-tui's full surface: `tests/grid.rs` (10), `tests/panel_replay.rs` (5),
`tests/render_sandbox.rs` (15), `tests/render_ansi.rs` (2), and the 202 lib unit tests covering
`tui/app.rs`, `tui/view.rs`, `tui/term.rs`, and `tui/panels.rs`.

**Rationale**: bee's ratatui usage sits entirely inside the API surface that 0.30 preserved —
`Buffer`, `Cell`, `Rect`, `Layout`, `Style`, the widget traits, `Terminal`, `TestBackend`, and
`CrosstermBackend`. The 0.30 facade split re-exports `ratatui-core` / `ratatui-widgets` /
`ratatui-crossterm` under the original paths, so `use ratatui::…` continues to resolve.

**Impact on the spec**: the spec's Dependencies section and Assumptions both describe Phase 0 as a
mechanical import migration "touching every file 008-grid-tui added under `bee-harness/src/tui/`".
That is now known to be false and should be corrected — Phase 0 touches `Cargo.toml` and `Cargo.lock`
only. The Phase 0 **gate** (008 suite green, zero behavior change) is unchanged and is already met.

**Alternatives considered**: a separate prerequisite PR (rejected during `/speckit-clarify` in favor
of Phase 0 in-feature); pinning to `ratatui-core` directly alongside ratatui 0.29 (impossible — two
incompatible `Buffer` types would coexist).

---

## R2: The `crossterm_0_28` feature does not do what the spec assumes — **correct the plan**

**Decision**: Move bee's own `crossterm` from 0.28 to 0.29 and do NOT enable
`ratatui/crossterm_0_28`. This supersedes the `/speckit-clarify` answer ("crossterm stays at 0.28 via
the `crossterm_0_28` feature"), which was given on the incorrect premise that the feature would hold
the version. Confirmed by the operator — see [Decision D1](#decision-d1-crossterm-version--resolved-config-c).

**Evidence** — `ratatui-crossterm-0.1.2/src/lib.rs:86-98`:

```rust
cfg_if::cfg_if! {
    // Re-export the selected Crossterm crate making sure to choose the latest version.
    if #[cfg(feature = "crossterm_0_29")] {
        pub use crossterm_0_29 as crossterm;
    } else if #[cfg(feature = "crossterm_0_28")] {
        pub use crossterm_0_28 as crossterm;
    } else { compile_error!(…) }
}
```

`crossterm_0_29` **wins** whenever both features are on. And `ratatui-crossterm`'s own
`default = ["crossterm_0_29", "underline-color"]`, while `ratatui 0.30.2` declares
`[dependencies.ratatui-crossterm] optional = true` **without** `default-features = false`. Cargo
feature unification means bee cannot turn that default off from downstream.

Therefore, with ratatui 0.30 + the crossterm backend, **crossterm 0.29 is unavoidable**. Enabling
`ratatui/crossterm_0_28` adds a *second* crossterm build without changing which one ratatui's
`CrosstermBackend` binds to. It is pure cost.

**Measured configurations**:

| Config | `bee-harness` crossterm | `ratatui/crossterm_0_28` | crossterm versions in lock | check | tests |
|--------|------------------------|--------------------------|----------------------------|-------|-------|
| A (as spec'd) | 0.28 | enabled | **0.28.1 + 0.29.0** | clean | 291/291 |
| B (drop the feature) | 0.28 | off | **0.28.1 + 0.29.0** | clean | not run |
| **C (recommended)** | **0.29** | off | **0.29.0 only** | clean | **291/291** |

Config C is the only one with a single crossterm. It compiles clean and passes the entire suite —
including 008's terminal-restore matrix and `RestoreGuard` — with **no source changes**.

**Why the duplicate matters beyond build size**: under A/B, bee's `RestoreGuard`, `enable_raw_mode`,
`EventStream`, and `KeyEvent` come from crossterm **0.28**, while ratatui's backend writes terminal
sequences via crossterm **0.29**. Two independent crossterm instances drive the same tty. That
compiles because bee never passes a crossterm type across the ratatui boundary — `Tui = Terminal<
CrosstermBackend<Stdout>>` uses `std::io::Stdout` (`tui/term.rs:17`) — but it is precisely the class
of split-brain terminal-state hazard that 008's restore matrix exists to catch.

### Decision D1: crossterm version — **RESOLVED, Config C**

**Resolved 2026-07-22**: Phase 0 moves `bee-harness`'s `crossterm` dependency from `0.28` to `0.29`
and does **not** enable `ratatui/crossterm_0_28`. This supersedes the `/speckit-clarify` answer
("crossterm stays at 0.28"), which was given on the incorrect premise that the `crossterm_0_28`
feature would hold the version.

Rationale: one crossterm driving the terminal is a correctness property, not a convenience. Config C
is the only configuration that achieves it, and it does so with **zero source changes** — measured at
291/291 tests passing, including 008's terminal-restore matrix and `RestoreGuard`.

`rustyline` stays pinned at 17, untouched and verified green under this configuration.

Phase 0's manifest change is therefore exactly:

```diff
- ratatui = { version = "0.29", default-features = false }
+ ratatui = { version = "0.30", default-features = false }

- crossterm = { version = "0.28", features = ["event-stream"], optional = true }
+ crossterm = { version = "0.29", features = ["event-stream"], optional = true }
```

with the `tui` feature list unchanged (`"ratatui/crossterm"`, no `crossterm_0_28`).

---

## R3: Do the spec's 12 curated effects map to real tachyonfx constructors? — **10 yes, 2 need composition**

**Decision**: `EffectSpec` keeps all 12 agent-facing variants. Ten resolve to a direct `fx::*` call;
`Pulse` and `Glow` are **composed** in `tui/effects.rs` from primitives. The Rhai surface is
unaffected — composition is a resolver-side detail.

**Evidence** — `tachyonfx-0.25.1/src/fx/mod.rs`:

| `EffectSpec` | tachyonfx resolution | Notes |
|---|---|---|
| `FadeIn { ms }` | `fx::fade_from(fg, bg, timer)` | fg/bg from the theme's `info` role (FR-003) |
| `FadeOut { ms }` | `fx::fade_to(fg, bg, timer)` | |
| `DissolveIn { ms }` | `fx::coalesce(timer)` | **naming inverts** — bee's "dissolve *in*" is tachyonfx `coalesce` |
| `DissolveOut { ms }` | `fx::dissolve(timer)` | |
| `SlideIn { dir, ms }` | `fx::slide_in(Motion, gradient_len, randomness, color, timer)` | 5 args, not 2 |
| `SlideOut { dir, ms }` | `fx::slide_out(…)` | same shape |
| `SweepIn { dir, ms }` | `fx::sweep_in(Motion, gradient_len, randomness, faded_color, timer)` | |
| `SweepOut { dir, ms }` | `fx::sweep_out(…)` | |
| `EvolveIn { ms }` | `fx::evolve_into(EvolveSymbolSet::BlocksHorizontal, timer)` | symbol set is exactly `▏▎▍▌▋▊▉█`, as US4 describes |
| `EvolveOut { ms }` | `fx::evolve_from(EvolveSymbolSet::BlocksHorizontal, timer)` | |
| `Pulse { color, ms }` | **composed**: `fx::sequence(&[fx::fade_to_fg(c, t/2), fx::fade_from_fg(c, t/2)])` | no `fx::pulse` exists |
| `Glow { ms }` | **composed**: `fx::ping_pong(fx::lighten(Some(a), Some(a), t/2))` | no `fx::glow` exists |

**Consequences for the design**:

1. The Rhai `slide_in(dir, ms)` / `sweep_in(dir, ms)` signatures in the spec expose 2 of the 5
   underlying parameters. `gradient_length` and `randomness` are **not** exposed to the agent; the
   resolver supplies house values. This is deliberate curation and matches FR-022's "curated set".
2. Direction strings map to `tachyonfx::Motion`: `"left"` → `LeftToRight`, `"right"` →
   `RightToLeft`, `"top"` → `UpToDown`, `"bottom"` → `DownToUp`. Unknown → `LeftToRight`
   (spec's `"left"` default).
3. `Pulse`/`Glow` being composites means their `ms` is split across sub-effects; the total still
   honors FR-023's 100–2000ms clamp.

**Alternatives considered**: dropping `Pulse`/`Glow` from the curated set (rejected — `pulse` is the
one effect US5 and US4 both call for, on status changes); exposing tachyonfx's full builder surface
to Rhai (rejected — violates the curation intent and widens the sandbox contract).

---

## R4: `EffectManager` gives FR-005 and FR-002 directly

**Decision**: Use `tachyonfx::EffectManager<String>` keyed by panel name. No custom effect registry.

**Evidence** — `tachyonfx-0.25.1/src/effect_manager.rs`:

| Need | API |
|---|---|
| FR-005 cancel-and-restart per panel | `unique(key, fx)` — "when a new unique effect is created with a key that matches an existing effect, the existing effect will be marked as complete on the next processing cycle" |
| FR-002 state 1 detection | `is_running() -> bool` (the spec called this `has_active()`) |
| FR-001 post-render application | `process_effects(duration: Duration, buf: &mut Buffer, area: Rect)` |
| Adding without a key | `add_effect(effect)` — used for chrome (FR-026) |

The key type is generic (`K: Clone + Ord + ThreadSafetyMarker`), so `String` panel names work
directly, and the overlay can use a reserved key.

**Determinism**: tachyonfx has **no `rand` dependency**. It ships `SimpleRng` (`src/simple_rng.rs`)
with a fixed default seed (`0x12345678`), and `motion.rs:81` seeds from the area's dimensions.
Character-scatter effects are therefore reproducible for a given area and timeline, which is what
makes the SC-001/SC-002/SC-008 buffer snapshots viable as `insta` tests.

---

## R5: Where the motion switch is applied

**Decision**: Resolve `EffectSpec → Option<Effect>` in `tui/effects.rs`, returning `None` when
animations are disabled. `None` means "apply content immediately, register nothing".

**Rationale**: FR-006b requires *every* effect to become an instant no-op — agent, automatic, overlay,
and chrome. A single resolver chokepoint guarantees that without dusting `if animations_enabled`
through each call site. It also makes FR-006c trivially true: if nothing is ever registered,
`is_running()` is always false, so the render loop can never enter the 60fps state.

`NO_COLOR` filtering (FR-006) happens at the same chokepoint but is per-variant rather than global:
color variants return `None`, text variants return `Some`. bee already has the predicate —
`viz::palette::is_color_enabled()` (`bee-harness/src/viz/palette.rs:28`).

**Alternatives considered**: filtering inside `EffectManager` via `CellFilter` (rejected — a filtered
effect still runs and still holds the loop at 60fps, violating FR-006c); a `no-op` tachyonfx effect
(rejected — same problem).

---

## R6: `Esc` dismiss placement

**Decision**: Insert the overlay-dismiss check in `handle_key` (`bee-harness/src/tui/app.rs:203`)
between the `help_open` swallow (line 205-210) and the `KeyModifiers::CONTROL` reserved-key block
(line 213).

**Evidence** — existing `Esc` bindings, all of which must keep working when no overlay is up:

| Location | Binding |
|---|---|
| `app.rs:206` | `help_open` → `?`/`Esc` closes help, swallows everything |
| `app.rs:260` | `Focus::Input` → `Esc` clears the input buffer |
| `app.rs:277` | `Focus::Chat` + `panels_visible` → `Esc` hides the panel column |

Placing dismiss above the focus dispatch satisfies FR-016 ("from either focus") while leaving all
three intact for the next keypress. Placing it *below* `help_open` means a help overlay opened on top
of a takeover still closes first — correct, since help is the more modal surface.

`q` is untouched (`app.rs:269`, `Focus::Chat` → quit), per the clarified decision.

---

## R7: Overlay → Panel downgrade target (spec's flagged open question)

**Decision**: Downgrade to a **reserved panel id `"takeover"`**, upserted through the normal
`PanelRegistry` path.

**Rationale**: `render_fullscreen` carries no name, so the gate must synthesize one. A fixed reserved
id gives FR-020's "at most one, replace don't stack" semantics for free — a second downgraded
takeover upserts the same panel. A generated id would accumulate panels and hit 008's "panel column
is full" error (`render_api.rs:155`) after a few turns.

**Validation interaction**: the existing id validator (`render_api.rs:182-189`) requires 1–32 chars of
`[a-z0-9_-]`. `"takeover"` satisfies it, so no validator change is needed — but the id must be
**reserved from agent use**: a script calling `render_to("takeover", w)` directly would collide. The
gate rejects agent use of the reserved id with a script error.

**Alternatives considered**: a `__`-prefixed magic id (rejected — requires relaxing the validator,
which was already declined for the API question in `/speckit-clarify`); generated ids (rejected —
unbounded panel growth); refusing the downgrade and erroring (rejected — FR-009 mandates silent
downgrade with a note, not failure).

---

## R8: The 1Hz countdown state

**Decision**: Compute a per-iteration timeout in the `tokio::select!` loop
(`bee-harness/src/tui/mod.rs`), not a persistent interval timer.

```text
timeout = if effects.is_running()      { Some(16ms) }
          else if overlay.is_some()    { Some(1s)   }
          else                         { None       }   // pure event-driven
```

**Rationale**: 008's loop is already a `select!` over `EventStream`, the `SessionEvent` channel, and a
tick. Making the tick arm's duration a function of state is a smaller change than introducing a
second timer, and it makes FR-002's three states literally one expression — easy to assert against in
the SC-003 redraw-counter test.

A recomputed-deadline variant (sleep to the next whole-second boundary) was considered and produces
identical wakeup counts; the interval form was chosen for symmetry with the existing tick arm.

---

## Summary of spec corrections this research forces

| Spec location | Says | Should say |
|---|---|---|
| Dependencies, "Scope" para | Phase 0 touches every `tui/` file | Phase 0 is a manifest-only change; 291 tests already verified green |
| Dependencies, crossterm | `crossterm_0_28` keeps crossterm at 0.28 | The feature is a no-op; crossterm 0.29 is unavoidable — see D1 |
| Project Structure note | "Every file under `tui/` is touched by Phase 0's import migration" | Delete — no import migration exists |
| Assumptions, ratatui bullet | "import paths move" | Import paths do **not** move for bee's usage |
| FR-022 / Rendering API | `pulse`, `glow` are effects | They are composites; note it in `contracts/rhai-effect-api.md` |

These are applied to `spec.md` as part of this planning pass.
