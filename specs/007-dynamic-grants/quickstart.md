# Quickstart: Validating Dynamic Capability Grants

How to prove the feature works, host-side first (no kernel), then on the BPF-LSM VM (kernel truth).
See `contracts/` for signatures and `data-model.md` for entities; this is a run/validation guide, not
implementation.

## Prerequisites

- Host build for the lifecycle + lease logic: `cargo test -p bee-harness` (stable, no kernel).
- Kernel truth: the `ac-matrix-vm` BPF-LSM VM (see memory `bee-bpf-lsm-test-vm`), reached via
  `test/vm/matrix.sh` (build on host → `virtctl scp` → run as root).

## Host validation (`bee-harness/tests/dynamic_grants.rs`, `MockModel`)

No kernel needed — reload is a no-op under `Sandbox::Host`, but Layer-1 tool grants, the hook
dispatch, the escalation/consent cycle, lease TTL/expiry, de-escalation, and the attenuation bound are
all exercised.

1. **Reactive escalate → auto-retry** (US1): a scenario whose base tools omit `bash`; a
   `DenialEscalationHook` + `AllowWithinCeiling`. The `MockModel` calls a tool that would be denied;
   assert the transcript shows the escalation audit events and the retried call's success, and the
   model saw one result (SC-005).
2. **Beyond-ceiling refusal without prompting** (US1-AS-2, SC-002): a spy `ConsentSink` that records
   whether it was consulted; a delta exceeding the ceiling. Assert refused and the sink was **never
   called**.
3. **Async timeout → deny** (US3, SC-003): a `ConsentSink` that never resolves; a short
   `escalation_timeout`. Assert the grant is denied and the episode continues (no hang).
4. **TTL expiry narrows** (US4, SC-004): a grant leased `Turns(1)`; assert `ActivePolicy.active`
   includes the delta on the granting turn and excludes it after `TurnStart` of the next turn, with a
   "narrowed" audit event.
5. **Explicit de-escalation** (US4): a hook returns `Flow::Deescalate(id)`; assert the lease is dropped
   and the tool it granted is de-registered.
6. **Attenuation property test** (Constitution II gate): generate random sequences of
   escalate/deescalate/expire over a `[base, ceiling]` pair; assert the invariant
   `base ⊆ active ⊆ ceiling` holds after every step and `active == base ∪ live-lease deltas`.
7. **Loop unification regression** (SC-006): the existing `episode_loop.rs`, `ctf.rs`,
   `cred_isolation.rs`, and 006-skills `skills_episode.rs` tests pass unchanged on the shared driver;
   `refresh_tools` still refreshes MCP tools (as a migrated hook).

Run: `cargo test -p bee-harness --lib --test dynamic_grants --test skills_episode --test episode_loop`.

## VM validation (kernel truth) — new `test/vm/remote-matrix.sh` cases

The load-bearing proof that reload changes what the kernel enforces. A/B on the same operation, only
the reload differs — mirroring how 006-skills' Layer 2 was proven.

1. **`reload-widen-allow`** (SC-001, US1): base denies `/home/ubuntu/dyn`; ceiling permits it; the
   episode escalates (reactive) on the first denied write, reloads, and the auto-retry **succeeds**.
   Assert the transcript shows initial `file_open denied` then a successful write, and the data landed.
2. **`reload-narrow-deny`** (SC-001/SC-004, US4): start with `/home/ubuntu/dyn` granted via a
   `Turns(1)` lease; a first-turn write succeeds; after lease expiry a second-turn write to the same
   path is **kernel-denied** (`EACCES`). Assert the narrowing reload removed the rule at the kernel.
   (Recall the `file_open`-hook nuance: a denied write leaves a **0-byte** file — assert the *data*
   did not land, never file absence. See memory `bee-skills-feature`.)
3. **`reload-beyond-ceiling`** (SC-002): escalation requests a path outside the ceiling; assert
   refused (no reload), the write stays denied, and the consent sink was not consulted.
4. **`reload-scope-isolation`** (SC-007): two concurrent scopes; widen one and assert the other's
   enforced rules are unchanged (extends `concurrent-audit-isolation`).

Run: `bash test/vm/matrix.sh` (or `BEE_SKIP_BUILD=1 …` to reuse binaries). Expect all prior cases plus
these to pass. Confirm a real eBPF load first with `sudo bee run --policy p -- echo OK` (a case can
false-PASS if the program fails to verify and the child never runs — see memory `bee-bpf-lsm-test-vm`).

## Definition of done

- Host suite green including the attenuation property test.
- VM matrix green including the four new reload cases (widen-allow, narrow-deny, beyond-ceiling,
  scope-isolation).
- 006-skills preload path and all prior matrix cases pass unchanged (no regression from loop
  unification).
