# Implementation Plan: Dynamic Capability Grants via a Loop Hooks Lifecycle

**Branch**: `007-dynamic-grants` | **Date**: 2026-07-21 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/007-dynamic-grants/spec.md`

## Summary

Make an episode's enforced policy a lease-managed range `[base, ceiling]` that moves at runtime. Add
a `bee-userspace` `Engine::reload_scope` that re-populates a live scope's per-`cgroup_id` rule maps
from a freshly compiled plan (widen or narrow, no re-attach), and a `bee-harness` **loop hooks
lifecycle** — modeled on Rig's `AgentHook`/`StepEvent`/`Flow` but living in bee's own `run_loop`,
where the `Sandbox`/`Scope`/audit ring are reachable — through which escalation (reactive on kernel
denial, proactive on skill `requires`) triggers an attenuation-checked, consent-gated reload followed
by a single auto-retry, and narrowing (TTL-lease expiry or explicit de-escalation) triggers a
reducing reload. The existing per-turn `refresh_tools` closure becomes one hook; `run_loop` (episode)
and `run_exchange` (REPL) are unified onto the shared lifecycle.

## Technical Context

**Language/Version**: Rust — user-space crates on **stable**; `bee-ebpf` on pinned **nightly** (bpf
target). No change to the toolchain split (Constitution V, bee-enforce-build).

**Primary Dependencies**: `aya` 0.14 (map update/remove on live maps), `async-trait` (async hook +
consent traits, already a dep), `tokio` (timeout on async consent — already behind `enforce`/harness),
`bee-core` (policy compile + attenuation `derive`). No new crates anticipated.

**Storage**: N/A (in-memory loop state; eBPF maps are the kernel-side store, reused via the existing
`EnforcementPlan` → map path).

**Testing**: `cargo test` host-side (`MockModel`, no kernel) for the lifecycle, escalation cycle,
lease/TTL logic, attenuation bounds, and timeout→deny. Kernel truth on the `ac-matrix-vm` BPF-LSM VM
via `test/vm/remote-matrix.sh` (widen-reload, narrow-reload, beyond-ceiling refusal). Property tests
for the attenuation bound on every reload (Constitution II gate).

**Target Platform**: Linux ≥5.7 with `CONFIG_BPF_LSM` + `bpf` in the active LSM list; cgroup v2
(same as 001). Host build (no `enforce`) runs the lifecycle with reload as a no-op.

**Project Type**: Rust workspace (library-first) — `bee-userspace` (kernel-adjacent) + `bee-harness`
(agent loop). Single-project layout.

**Performance Goals**: A reload runs at a turn boundary (per escalation / lease boundary), not per
tool call — infrequent. Full plan re-compile per reload is acceptable (no incremental diff compiler).
Target: a reload adds negligible latency versus a model round-trip.

**Constraints**: Reload MUST be atomic-enough that no sandboxed child ever sees a torn ruleset (met by
reloading only at the turn boundary, plus per-key atomic map updates). `DENY_MAX_RULES = 8` fs/exec
cap carries over — a widening past it is refused, not truncated. bee-core/bee-common stay untouched
(Constitution V). No wall-clock dependence in the deterministic path: turn-count leases are primary.

**Scale/Scope**: One active policy per scope; leases number in the low single digits per episode;
reloads bounded by escalations + lease expiries. Concurrent episodes reload independently by
`cgroup_id`.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

- **I — Deny-by-Default & Fail-Closed** ✅ Active policy starts at `base`; escalation is opt-in; an
  async-consent timeout or unreachable approver resolves to **deny**; a reload/compile failure aborts
  the reload and keeps the prior (narrower-or-equal) enforced policy. Gate: a timeout→deny test and a
  compile-failure→keep-prior test.
- **II — Capability Attenuation** ✅ Every reload (widen *and* narrow) is validated `active' ⊆ ceiling`
  via `bee-core` `derive` before touching the kernel; beyond-ceiling escalation is refused without
  consulting consent. Gate: **property test** that no sequence of escalate/deescalate/expire reloads
  ever yields an active policy outside `[base, ceiling]` (Constitution requires property tests here).
- **III — Kernel Enforcement Is Authoritative** ✅ Reload recompiles the policy to an `EnforcementPlan`
  and re-populates kernel maps; the LSM programs remain the decision point. User space never enforces.
  Gate: the VM widen/narrow matrix cases (allow becomes deny and vice-versa at the kernel).
- **IV — Policy-as-Data** ✅ The ceiling file is the reviewable pre-authorization; grants, reloads,
  lease expiries, and de-escalations are emitted as structured audit events with provenance. The
  active policy at any point is expressible as declarative policy text (base + applied deltas).
- **V — Library-First, Runtime-Free Core** ✅ `reload_scope` lives in `bee-userspace` (enforce);
  the hooks lifecycle in `bee-harness`; both leave `bee-core`/`bee-common` synchronous and untouched.
  Async (hooks, consent, timeout) is an opt-in harness layer, not in the core.

**Result**: No violations. Complexity Tracking table left empty.

## Project Structure

### Documentation (this feature)

```text
specs/007-dynamic-grants/
├── plan.md              # This file
├── research.md          # Phase 0 output — resolved unknowns (reload atomicity, net-key removal,
│                        #   Engine ownership, async hook object-safety, lease clock, retry recording)
├── data-model.md        # Phase 1 output — ActivePolicy, GrantLease, StepEvent, Flow, ConsentSink
├── quickstart.md        # Phase 1 output — host + VM validation guide
├── contracts/           # Phase 1 output
│   ├── reload-api.md     #   Engine::reload_scope / Scope net-key tracking
│   ├── hooks.md          #   LoopHook trait, StepEvent, Flow
│   └── consent.md        #   async ConsentSink, Decision, timeout→deny
└── tasks.md             # /speckit-tasks output (NOT created here)
```

### Source Code (repository root)

```text
bee-userspace/src/
├── lib.rs               # Engine::reload_scope(cgroup_id, &plan); Scope gains tracked net-keys
├── plan.rs              # reuse EnforcementPlan::prepare for the widened/narrowed policy (no change
│                        #   to encoding; reload calls the same compile path)
└── cgroup.rs            # unchanged (cgroup already exists at reload time)

bee-harness/src/
├── hooks.rs             # NEW: LoopHook trait, StepEvent, StepEventKind, Flow, hook stack + dispatch
├── grants/              # promote 006-skills grant logic into a shared runtime module
│   ├── mod.rs           #   re-exports; the ActivePolicy + GrantLease state machine
│   └── escalate.rs      #   escalation cycle: resolve → reload → retry; narrow: expire/deescalate
├── skills/grant.rs      # ConsentSink becomes async (Decision); AllowWithinCeiling/DenyAll updated
├── episode.rs           # run_loop drives the shared lifecycle; owns ActivePolicy + reload
├── repl.rs              # run_exchange drives the same lifecycle (unify with run_loop)
├── sandbox.rs           # EnforcedSandbox exposes a reload path to its held Engine (mutable reach)
└── scenario.rs          # (already has ceiling_policy_path from 006; add lease defaults if needed)

bee-harness/tests/
├── dynamic_grants.rs    # NEW host tests: escalate→retry, timeout→deny, TTL expiry, deescalate,
│                        #   attenuation property test over reload sequences
test/vm/
└── remote-matrix.sh     # NEW cases: reload-widen-allow, reload-narrow-deny, reload-beyond-ceiling
```

**Structure Decision**: Single Rust workspace, extending two existing crates. New surface is
concentrated in `bee-harness` (the loop + hooks + lease state machine) with one focused
`bee-userspace` addition (`reload_scope`). 006-skills' `skills/grant.rs` resolver is reused and
promoted into a runtime-facing `grants` module so preload and dynamic paths share one code path.

## Complexity Tracking

> No Constitution violations — table intentionally empty.
