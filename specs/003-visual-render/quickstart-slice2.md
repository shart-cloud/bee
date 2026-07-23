# Quickstart: bee Visual Rendering — Slice 2 (sprites & animation)

> **Command names changed.** The consolidation (ADR-0002) replaced `bee-episode`, `bee-repl`, and
> `bee-metrics` with subcommands of the single `bee` executable: `bee run`, `bee repl`, and
> `bee metrics`. The raw process runner moved from `bee run` to `bee exec`, and a session with no
> policy now needs an explicit `--host`. The commands below are recorded as this feature shipped
> them; translate accordingly.

Validation guide for Slice 2 — sprites, animation, and the bee mascot. All checks run on the host
with `cargo test` (no terminal, no timing-sensitive assertions). See `contracts/sprite-api.md` and
`data-model-slice2.md` for the details, and `research-slice2.md` (D12–D18) for the decisions.

## Prerequisites

- Slice 1 shipped (commit `b519f15`); no new dependency in `bee-harness/Cargo.toml`.

## 1. Half-block sprite rendering (SC-015)

```bash
cargo test -p bee-harness --test sprite_render
```

Covers: a 16×16, 4-color sprite → **8** terminal rows, each with truecolor escapes; a transparent
half emits **no** background escape for its half; a fully-transparent sprite → `⌈h/2⌉` blank rows
(M1); off-tty/`NO_COLOR` → monochrome block chars, no SGR (L3); `quantize_256`/`quantize_16` map
known RGBs to expected indices; the `Ansi256`/`Ansi16` paths emit indexed escapes.

## 2. The animation seam (SC-016)

```bash
cargo test -p bee-harness --test sprite_anim         # pure: playback ordering + redraw cursor-up
cargo test -p bee-harness --lib repl::terminal        # stateful: reclaim + spinner preemption
```

Covers: `playback(spec)` frame order — `[0,1,2,1]` (bounce) / `[0,1,2,0,1,2]` (2 cycles) / one period
(cycles=0); `redraw_block` starts with `\x1b[{N}A` (SC-016 cursor-up); starting an animation while the
spinner runs claims the slot (SC-017, `animation_preempts_spinner`); a stopped 2-row animation leaves
`reclaim_rows == 2` and the next output overwrites both rows (SC-016 reclaim,
`animation_reclaims_all_its_rows`). **Regression**: the Slice-1 spinner test passes unchanged.

## 3. The Rhai sprite/animation caps + the bee (SC-018)

```bash
cargo test -p bee-harness --test render_sandbox
```

Covers (⛔fail-closed): `sprite(40,40,..)` → error; palette > 32 → error; bad hex → error; 17th
`anim.add` → error; mismatched-dims frame → error; interval below 50 ms is **clamped** (not an error).
And SC-018: `bee_sprite()` → a 16×16 `Sprite`, `bee_animation()` → a 3-frame `Animation`. And M1: a
fully-transparent sprite's tool summary notes it.

## 4. Dependency confinement (SC-019, still holds)

```bash
cargo tree -p bee-core   | grep -E 'rhai|ratatui' || echo clean
cargo tree -p bee-common | grep -E 'rhai|ratatui' || echo clean
```

Slice 2 added **no** dependency — sprites are hand-rolled ANSI, the animator reuses tokio.

## 5. Try it live (optional)

```bash
BEE_MASCOT=1 cargo run -p bee-harness --bin bee-repl -- --provider <mock>.toml   # or --bee
```

The bee wing-flap plays once at startup, then its rows are reclaimed. A `render`-enabled scenario can
have the agent draw `specs/003-visual-render/examples/pixel-bee.rhai` or `status-sprite.rhai`.

---

## Success-criteria coverage (Slice 2)

| SC | Where validated |
|----|-----------------|
| SC-015 | §1 sprite_render (8 rows, truecolor, transparency) |
| SC-016 | §2 sprite_anim (playback/redraw) + terminal unit tests (reclaim) |
| SC-017 | §2 `animation_preempts_spinner` |
| SC-018 | §3 `bee_functions_are_callable` |
| SC-019 | §4 (no new dependency) |

**Deferred**: the episode-completion static bee (analyze M3) — surface undefined; shipped via `--bee`
startup + Rhai `bee_*` only.
