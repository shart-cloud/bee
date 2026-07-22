# Implementation Plan: Terminal Effects & Agent Visual Permissions

**Branch**: `009-tachyonfx-effects` | **Date**: 2026-07-22 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/009-tachyonfx-effects/spec.md`

## Summary

Add a tachyonfx post-processing pipeline to the 008-grid-tui full-screen TUI so panels fade in,
updates dissolve and coalesce, and the harness animates its own chrome — plus a configuration layer
(`visual_level`) bounding how much screen the agent may claim, a full-screen takeover with a TTL and
an `Esc` escape hatch, and an orthogonal motion kill switch for accessibility and CI.

The technical approach is a **two-layer split along the existing Constitution V boundary**: Rhai
produces `EffectSpec` serde data (headless, no tachyonfx import, recorded in the transcript); a single
resolver in `tui/effects.rs` turns that data into live `tachyonfx::Effect` values, applying the motion
and `NO_COLOR` filters at one chokepoint. `EffectManager<String>` keyed by panel name supplies
cancel-and-restart for free.

Phase 0 research measured, rather than assumed, the ratatui 0.29 → 0.30 bump this feature requires:
it is a **manifest-only change** — 291 tests across 28 suites pass with zero source edits.

## Technical Context

**Language/Version**: Rust (stable; workspace toolchain). No nightly — this is user-space only.

**Primary Dependencies**: `tachyonfx 0.25.1` (new, `bee-harness` only, `optional`, `tui`-gated);
`ratatui 0.29 → 0.30.2` and `crossterm 0.28 → 0.29` (both Phase 0, manifest-only); existing `rhai 1`,
`tokio`. `rustyline` stays pinned at 17.

**Storage**: N/A. `EffectSpec` is recorded in the existing episode transcript; no new persistence.

**Testing**: `cargo test` with `insta` buffer snapshots against ratatui's `TestBackend`; the redraw
counter from 008-grid-tui for the idle/wakeup assertions. No VM, no kernel capability, no network —
unlike bee's enforcement work, this layer is fully host-testable.

**Target Platform**: Linux terminal (same as 008). Headless builds compile the serde layer only.

**Project Type**: Rust workspace; this feature touches one crate (`bee-harness`) plus its spec dir.

**Performance Goals**: 60fps (16ms) while effects run; **1Hz** while an overlay shows with no effects;
zero periodic redraws when idle. A 120s overlay costs ~120 redraws, not ~7200.

**Constraints**: `tachyonfx` MUST NOT appear in `bee-core` or `bee-common` (SC-009). Effect durations
clamped 100–2000ms. Takeover TTL default 30s, hard max 120s. No change to any enforcement path.

**Scale/Scope**: ~5 new modules in `bee-harness/src/`, 12 `EffectSpec` variants, 4 permission tiers,
2 new Rhai commit verbs, 12 new Rhai constructors. 5 user stories, P1→P3.

## Constitution Check

*GATE: evaluated before Phase 0 research and re-evaluated after Phase 1 design.*

| Principle | Pre-Phase-0 | Post-Phase-1 | Assessment |
|---|---|---|---|
| **I. Deny-by-Default & Fail-Closed** | PASS | **PASS** | `visual_level` defaults to `panels` (restricted), takeover is opt-in, and the `Ord`-derived tier order makes deny-by-default structural rather than a match arm to get right. Design added a fail-closed detail: an unrecognized `visual_level` or an out-of-range `takeover_ttl_secs` is a **hard startup error**, never a silent fall back to the default. Agent requests above the ceiling downgrade — never escalate. |
| **II. Capability Attenuation** | N/A | **N/A** | Visual permissions are not capabilities in the enforcement sense. They do not derive, attenuate, or interact with cgroup scopes or policy subsetting. No subagent derivation path is touched. |
| **III. Kernel Enforcement Is Authoritative** | PASS | **PASS** | Presentation layer only. No eBPF, no LSM hook, no cgroup, no policy compilation. `bee-ebpf` and `bee-userspace` are untouched. |
| **IV. Policy-as-Data** | PASS | **PASS** | `visual_level`, `animations`, and `takeover_ttl_secs` are declarative TOML, reviewable in a diff. `EffectSpec` is serde data recorded in the transcript. Design strengthened this: `ms` is clamped at *construction*, so the transcript records the effective value and a replay reproduces the original animation exactly. |
| **V. Library-First, Runtime-Free Core** | PASS | **PASS** | `tachyonfx` is `optional = true` behind `tui`, confined to `bee-harness`. `EffectSpec` lives in `render_spec/`, compiles headless, imports no tachyonfx type. Verified by `core_deps_guard` + `cargo tree` (SC-009). No async runtime added to any core path. |
| **Security & Platform** | PASS | **PASS** | tachyonfx operates on an in-memory `Buffer`: no I/O, no FFI, no network, and — measured, not assumed — **no `rand` dependency** (it ships a seeded `SimpleRng`). The Rhai sandbox is unchanged; it produces data, never live effect objects. The TTL bounds screen takeover and `Esc` outranks every competing binding, so the operator's escape hatch cannot be shadowed. |
| **Test-first for the security boundary** | PASS | **PASS** | No security boundary is modified, so the test-first mandate does not bind here. The feature nonetheless carries testable requirements for every downgrade row, every dismiss trigger, and every axis combination. |
| **Performance budgets are gates** | PASS | **PASS** | SC-003 is a hard gate with a counted assertion (~N redraws for an N-second overlay, not 60N), not an aspiration. |

**Result: no violations. Complexity Tracking is empty.**

One decision surfaced by Phase 0 research contradicted an earlier clarification and was escalated
rather than silently resolved: the `crossterm_0_28` feature cannot hold crossterm at 0.28. Resolved
in favor of moving bee to crossterm 0.29 for a single crossterm in the tree — see
[research.md D1](./research.md). Not a constitution matter, but a correctness one: two crossterm
instances driving the same tty is the hazard 008's terminal-restore matrix exists to catch.

## Project Structure

### Documentation (this feature)

```text
specs/009-tachyonfx-effects/
├── spec.md              # Feature spec (clarified 2026-07-22, 5 decisions recorded)
├── plan.md              # This file
├── research.md          # Phase 0 output — 8 findings, all measured
├── data-model.md        # Phase 1 output — serde layer / runtime layer split
├── quickstart.md        # Phase 1 output — phase-gated validation guide
├── contracts/           # Phase 1 output
│   ├── effect-spec.md         # EffectSpec serde format
│   ├── visual-levels.md       # permission tiers + downgrade matrix
│   ├── overlay-lifecycle.md   # states, TTL, dismiss, Esc precedence
│   ├── rhai-effect-api.md     # agent-facing constructors + takeover verbs
│   └── motion-control.md      # the kill switch and the three axes
├── examples/            # agent-authored Rhai samples (populated during implementation)
└── tasks.md             # Phase 2 output — NOT created by /speckit-plan
```

### Source Code (repository root)

```text
bee-harness/
├── Cargo.toml                      # Phase 0: ratatui 0.30 + crossterm 0.29 (manifest-only).
│                                   # Then: tachyonfx dep, optional + tui-gated.
└── src/
    ├── render_spec.rs              # MODIFIED — Renderable { spec, effect } wrapper
    ├── render_spec/
    │   └── effect_spec.rs          # NEW — EffectSpec, Direction (serde, headless, no tui gate)
    ├── render_api.rs               # MODIFIED — register constructors + render_fullscreen verbs
    ├── render_api/
    │   └── effect_api.rs           # NEW — Rhai bindings; clamping happens here
    ├── config.rs                   # MODIFIED — VisualLevel, MotionSetting, TakeoverConfig
    ├── tools/render.rs             # MODIFIED — carry EffectSpec + RenderTarget into ToolResult
    └── tui/                        # all tui-gated
        ├── mod.rs                  # MODIFIED — three-state scheduling in the select! loop
        ├── app.rs                  # MODIFIED — overlay/visual_level/motion; Esc above focus dispatch
        ├── view.rs                 # MODIFIED — process_effects after widget render
        ├── panels.rs               # MODIFIED — Panel gains EffectSlot
        ├── effects.rs              # NEW — the resolver chokepoint + EffectManager ownership
        ├── overlay.rs              # NEW — Overlay state machine, TTL, dismiss hint widget
        └── visual_gate.rs          # NEW — tier enforcement, downgrade notes
```

**Structure Decision**: single-crate change inside the existing workspace. The split that matters is
not directory layout but the **feature-gate boundary**: `render_spec/effect_spec.rs` and
`render_api/effect_api.rs` compile in the headless build and import no tachyonfx; everything under
`tui/` is gated. This mirrors how 003-visual-render already separates `RenderSpec` from
`viz::buffer_render`, so the boundary is enforced by the existing `core_deps_guard` test rather than
by convention.

`bee-core`, `bee-common`, `bee-ebpf`, `bee-userspace`, and `bee-cli` are untouched.

## Phase ordering

Sequenced so each phase is independently shippable and gated by the prior one.

| Phase | Story | Priority | Gate |
|---|---|---|---|
| **0** | ratatui 0.30 + crossterm 0.29 | — | 291 tests green, zero behavior change, single crossterm in `cargo tree -d` (already measured) |
| **1** | US1 effects pipeline | P1 | SC-001, SC-002, SC-003 |
| **2** | US2 visual permissions | P2 | SC-004, SC-005 |
| **3** | US3 overlay + TTL + dismiss | P2 | SC-006, SC-007 |
| **4** | US4 agent effects via Rhai | P3 | SC-008 |
| **5** | US5 harness chrome presets | P3 | US5 scenarios 1–5 |
| **X** | motion switch (FR-006a–d) | P1 | SC-011, SC-012 |

Phase X is authored **with Phase 1**, not after it: the resolver chokepoint it requires is the same
function Phase 1 introduces, and retrofitting a kill switch across established call sites is exactly
the failure mode FR-006c's structural guarantee exists to prevent.

## Complexity Tracking

No constitution violations. Table intentionally empty.
