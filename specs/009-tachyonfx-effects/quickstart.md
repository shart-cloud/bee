# Quickstart: validating 009-tachyonfx-effects

> **Command names changed.** The consolidation (ADR-0002) replaced `bee-episode`, `bee-repl`, and
> `bee-metrics` with subcommands of the single `bee` executable: `bee run`, `bee repl`, and
> `bee metrics`. The raw process runner moved from `bee run` to `bee exec`, and a session with no
> policy now needs an explicit `--host`. The commands below are recorded as this feature shipped
> them; translate accordingly.

Runnable checks that prove the feature works, ordered so each phase gates the next. Details live in
[data-model.md](./data-model.md) and [contracts/](./contracts/); this file is the run guide.

## Prerequisites

```bash
cd /home/jg/git/bee
rustc --version        # workspace toolchain
```

No VM, no kernel capabilities, no network. This feature is a presentation layer — unlike the eBPF
work, everything here runs on the host with plain `cargo test`.

---

## Phase 0 gate — the ratatui bump

Manifest-only. **Already verified during planning** ([research.md R1](./research.md)); this is the
reproduction.

```bash
cargo check -p bee-harness --features tui     # expect: 0 errors, 0 warnings
cargo check -p bee-harness                    # headless build still clean
cargo test  -p bee-harness --features tui     # expect: 291 passed, 0 failed (28 suites)
```

**Gate**: byte-identical test results to the pre-bump run. Any behavior change means the bump is not
the drop-in it measured as, and Phase 0 grows before any effects work starts.

Confirm the dependency graph collapsed to a single crossterm:

```bash
grep -A1 '^name = "crossterm"' Cargo.lock | grep -c version   # expect: 1
```

Check the lock, not `cargo tree -d` — crossterm appears inside other duplicated subtrees (`nix`
0.29/0.30), so grepping the tree gives false positives. More than one version here means two
crossterm instances are driving the same tty — the hazard
[research.md R2/D1](./research.md) documents, and the reason Phase 0 moves bee's own crossterm to
0.29 rather than enabling `ratatui/crossterm_0_28`.

---

## Phase 1 — effects pipeline (US1, P1)

```bash
cargo test -p bee-harness --features tui effects::
```

| Check | Asserts | Req |
|---|---|---|
| new panel fades in | cells at t=0 match background, t=150ms intermediate, t=300ms final | SC-001 |
| panel replace dissolves→coalesces | t=0 old content, t=200ms scattered glyphs, t=400ms new content | SC-002 |
| rapid re-update cancels | one transition active after two upserts 50ms apart | FR-005 |
| idle redraw count | zero periodic redraws with no effects and no overlay | SC-003 |

Snapshots are stable because tachyonfx carries a seeded `SimpleRng` and no `rand` dependency
([research.md R4](./research.md)) — the scatter pattern is reproducible for a given area.

Manual look:

```bash
cargo run -p bee-harness --features tui --bin bee-repl -- \
  --scenario .scratch/tui-demo.toml
```

---

## Phase 2 — visual permissions (US2, P2)

```bash
cargo test -p bee-harness --features tui visual_gate::
```

Walk the levels by hand — each should behave per
[contracts/visual-levels.md](./contracts/visual-levels.md):

```bash
BEE_VISUAL_LEVEL=none        cargo run … # panels become inline + downgrade note
BEE_VISUAL_LEVEL=panels      cargo run … # panel ≤ 1/3 width
BEE_VISUAL_LEVEL=panels-wide cargo run … # panel ≤ 1/2 width
BEE_VISUAL_LEVEL=takeover    cargo run … # render_fullscreen works
BEE_VISUAL_LEVEL=bogus       cargo run … # startup ERROR, not a fallback
```

The last one is the Constitution I check: an uncompilable policy is refused, never silently
downgraded to the default.

Precedence:

```bash
BEE_VISUAL_LEVEL=none cargo run … -- --visual-level takeover   # CLI wins
```

---

## Phase 3 — overlay lifecycle (US3, P2)

```bash
cargo test -p bee-harness --features tui overlay::
```

| Check | Asserts | Req |
|---|---|---|
| hint in bottom row | `Esc to dismiss · auto-dismiss in {N}s` present | FR-015 |
| countdown advances | N decrements once per second | FR-015 |
| Esc from input focus | overlay gone, **typed text preserved** | FR-016 |
| Esc with no overlay | still clears input / hides panels (008 regression) | — |
| TTL expiry | absent from buffer after `ttl + 0.2s` | SC-007 |
| replacement | second request replaces, does not stack | FR-020 |
| wakeup budget | ~N redraws for an N-second overlay, not 60N | SC-003 |

Interactive check, since keyboard precedence is the thing most likely to regress:

```bash
BEE_VISUAL_LEVEL=takeover cargo run -p bee-harness --features tui --bin bee-repl -- \
  --scenario specs/009-tachyonfx-effects/examples/fullscreen-chart.rhai
```

1. Type text into the input, do **not** submit.
2. Press `Esc` → overlay dismisses, **your text is still there**.
3. Press `Esc` again → input clears.
4. Raise another overlay, wait out the TTL → auto-dismiss, chat scroll unchanged.
5. Raise another, press `q` → bee **quits**. `q` is not a dismiss key, by design.

---

## Phase 4 — agent effects + chrome (US4/US5, P3)

```bash
cargo test -p bee-harness --features tui effect_api::
```

| Check | Asserts | Req |
|---|---|---|
| `slide_in("left", 400)` | produces `EffectSpec::SlideIn`, resolves, active 400ms | SC-008 |
| `slide_in("left", 5000)` | clamped to 2000ms | FR-023 |
| `slide_in("sideways", 400)` | direction falls back to `left` | contract |
| `pulse("sting", 200)` | composite resolves; panel flashes the error role | R3 |

---

## Cross-cutting: the three axes

```bash
BEE_NO_ANIMATION=1 cargo test -p bee-harness --features tui motion::
NO_COLOR=1         cargo test -p bee-harness --features tui motion::
```

| Check | Asserts | Req |
|---|---|---|
| motion off → zero periodic redraws | across create/replace/overlay/startup | SC-011 |
| motion off → t=0 shows final content | no intermediate colors, no substituted glyphs | SC-011 |
| `NO_COLOR` + motion on + level `none` | chrome text effects mutate cells, no color escapes, no panels | SC-012 |
| motion off, overlay up | countdown **still** advances | FR-015 |

---

## Dependency boundary (Constitution V)

```bash
cargo tree -p bee-core    | grep -c tachyonfx     # expect 0
cargo tree -p bee-common  | grep -c tachyonfx     # expect 0
cargo check -p bee-harness                        # headless: no tachyonfx compiled
cargo test  -p bee-harness --test core_deps_guard
```

SC-009. `tachyonfx` is `optional = true` behind the `tui` feature; `EffectSpec` lives in
`render_spec/` and compiles in the headless build with no tachyonfx import.

---

## Full sweep

```bash
cargo test --workspace
cargo test -p bee-harness --features tui
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
```
