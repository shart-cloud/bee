---
description: "Task list for 007-dynamic-grants implementation"
---

# Tasks: Dynamic Capability Grants via a Loop Hooks Lifecycle

**Input**: Design documents from `/specs/007-dynamic-grants/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/{reload-api,hooks,consent}.md

**Tests**: REQUIRED — the constitution mandates test-first for the enforcement boundary and a
property-based test for attenuation. Test tasks precede their implementation within each phase.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: parallelizable (different files, no dependency on an incomplete task)
- **[Story]**: US1–US4 for user-story phases; Setup/Foundational/Polish carry no story label
- Paths are repo-relative.

---

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Scaffold the new modules and make 006-skills' grant resolver reusable at runtime.

- [X] T001 Create module scaffolding — `bee-harness/src/hooks.rs`, `bee-harness/src/grants/mod.rs`, `bee-harness/src/grants/escalate.rs` (stubs) and register them in `bee-harness/src/lib.rs`.
- [X] T002 [P] Promote 006-skills' `GrantRequest`/`resolve_grants`/sink types so `bee-harness/src/grants/` re-exports them for the runtime path (no behavior change) in `bee-harness/src/skills/grant.rs` + `bee-harness/src/grants/mod.rs`.

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: The reload primitive, hook types, async consent, state machine, and unified driver that
every user story builds on. **No user story can start until this phase completes.**

### Kernel reload primitive (bee-userspace, enforce)

- [X] T003 [P] Add `net_keys: Vec<NetKey>` to `Scope` and populate it in `create_scope` (track installed `NET_ALLOW` keys) in `bee-userspace/src/lib.rs`.
- [X] T004 Implement `Engine::reload_scope(&mut self, cgroup_id, &EnforcementPlan, prev_net_keys) -> Result<Vec<NetKey>, ScopeError>` — overwrite `SCOPES`/`FS_DENY`/`EXEC_ALLOW` (or remove when empty), diff-and-remove/insert `NET_ALLOW`, return the new key set (contract `reload-api.md`) in `bee-userspace/src/lib.rs`. *(depends: T003)*
- [X] T005 Add `Sandbox::reload(&mut self, &EnforcementPlan) -> Result<(), SpawnError>` — Host no-op; Enforced/Concurrent reach the held `Engine` via a method (no `pub` field) and update `Scope.net_keys` in `bee-harness/src/sandbox.rs`. *(depends: T004)*

### Hook lifecycle + async consent + state machine (bee-harness)

- [X] T006 [P] Define `StepEvent`, `StepEventKind`, `Flow`, and the `#[async_trait] LoopHook` trait with a hook-stack dispatcher (registration order, first non-`Continue` wins, `observes` filter) in `bee-harness/src/hooks.rs` (contract `hooks.md`).
- [X] T007 [P] Migrate `ConsentSink` to async (`async fn confirm -> Decision`) preserving 006 semantics for `DenyAll`/`AllowWithinCeiling`, and make REPL `PromptConsent` async, in `bee-harness/src/skills/grant.rs` and `bee-harness/src/bin/bee-repl.rs` (contract `consent.md`).
- [X] T008 [P] Define `GrantDelta`, `GrantLease`, `GrantOrigin`, `Ttl`, and `ActivePolicy` with `apply`/`expire`/`release` → recompiled `bee_core::Policy`, plus unit tests for each transition, in `bee-harness/src/grants/mod.rs` (data-model `ActivePolicy`/`GrantLease`).

### Unified driver + regression-preserving migration

- [ ] T009 Extract a shared loop driver (schema snapshot → hook dispatch → tool execute → audit drain → `KernelDenial` emission) and route both `run_loop` and `run_exchange` through it; add `hooks: Vec<Box<dyn LoopHook>>` to `LoopOptions` and `ReplConfig` in `bee-harness/src/episode.rs`, `bee-harness/src/repl.rs`, `bee-harness/src/hooks.rs`. *(depends: T006)*
- [ ] T010 Re-express the 004-mcp `refresh_tools` closure as a built-in `LoopHook` on `TurnStart` (keep `tools/list_changed` refresh working) in `bee-harness/src/hooks.rs`, `bee-harness/src/episode.rs`, `bee-harness/src/repl.rs`. *(depends: T009)*
- [ ] T011 Regression: update callers so `episode_loop.rs`, `ctf.rs`, `cred_isolation.rs`, `concurrent.rs`, and `skills_episode.rs` pass unchanged on the shared driver in `bee-harness/tests/`. *(depends: T009, T010)*

**Checkpoint**: Reload primitive, hooks, async consent, and `ActivePolicy` exist and all prior tests
pass on the unified driver — user stories can now proceed.

---

## Phase 3: User Story 1 — Reactive escalation on kernel denial + auto-retry (P1) 🎯 MVP

**Goal**: A denied tool op triggers a `KernelDenial` → escalation for that exact resource → reload →
the same call auto-retries once and succeeds; refusal feeds the denial back.

**Independent test**: `MockModel` op targeting a path in `(base, ceiling]`; approving sink → transcript
shows denial, grant+reload, successful retry (one model-visible result); beyond-ceiling → refused
without consulting consent.

- [ ] T012 [P] [US1] Host test: reactive escalate → reload → auto-retry succeeds and the model sees a single result; also assert the retry runs **at most once** — an approved-but-still-denied call is returned to the model, not re-escalated (SC-005), in `bee-harness/tests/dynamic_grants.rs`.
- [ ] T013 [P] [US1] Host test — fail-closed refusal paths: (a) beyond-ceiling escalation is refused WITHOUT calling the consent sink (spy sink asserts zero calls; SC-002); (b) a widening exceeding `DENY_MAX_RULES` is refused with a reason, not truncated (FR-016); (c) a reload whose recompile fails leaves `ActivePolicy` and the enforced maps unchanged (Constitution I, plan gate), in `bee-harness/tests/dynamic_grants.rs`.
- [X] T014 [US1] Implement `escalate()` (candidate = active ∪ delta → `ceiling.derive` → compile/`prepare` with rule-cap refusal → `timeout(consent)` → `Sandbox.reload` → `ActivePolicy.apply` + push `GrantLease` → register `delta.tools`) in `bee-harness/src/grants/escalate.rs` (contract `consent.md`). *(depends: T004, T007, T008)*
- [X] T015 [US1] Implement `DenialEscalationHook`: build a minimal `GrantDelta` from a `KernelDenial` (`file_open`→fs, `socket_connect`→net, `bprm_check_security`→exec) and return `Flow::Escalate`; gate on the `reactive_escalation` knob, in `bee-harness/src/grants/escalate.rs`.
- [X] T016 [US1] Wire escalation into the driver: on `Flow::Escalate` from `BeforeToolCall`/`KernelDenial`, run `escalate()` then re-run the triggering `ToolCall` exactly once; on refusal return the original denial to the model, in `bee-harness/src/episode.rs` + `bee-harness/src/hooks.rs`. *(depends: T014, T009)*
- [ ] T017 [US1] Add grant + reload transcript/audit event types with provenance (origin, delta, turn) and record them on the retried call (R8) in `bee-harness/src/transcript.rs`. *(depends: T016)*
- [X] T018 [US1] VM matrix cases `reload-widen-allow` (denied write → escalate → reload → retry writes data) and `reload-beyond-ceiling` (escalation refused, write stays denied, sink not consulted) in `test/vm/remote-matrix.sh`. *(depends: T004, T016)*

**Checkpoint**: US1 delivers standalone value — an agent recovers from a denial via an approved grant.

---

## Phase 4: User Story 2 — Proactive escalation when a loaded skill needs capabilities (P1)

**Goal**: Loading a skill whose `requires` exceeds the active scope escalates before its instructions
take effect; refusal degrades the skill to instructions-only.

**Independent test**: episode with skills roots + ceiling; model calls `skill` mid-run for a
capability-requesting skill; within-ceiling → capability live for later calls; beyond-ceiling → body
still loads, model told the capability was withheld.

- [ ] T019 [P] [US2] Host tests: within-ceiling mid-episode skill grant makes a later tool call succeed; beyond-ceiling skill loads instructions-only, in `bee-harness/tests/dynamic_grants.rs`.
- [X] T020 [US2] Implement `SkillEscalationHook`: on `BeforeToolCall` for the `skill` tool, if the target skill's `requires` exceeds `ActivePolicy.active`, return `Flow::Escalate(delta)`, in `bee-harness/src/grants/escalate.rs`. *(depends: T014)*
- [ ] T021 [US2] On refusal, the `skill` tool still returns the body (instructions-only) with a note that the capability was not granted, in `bee-harness/src/tools/skill.rs` + driver. *(depends: T020)*

**Checkpoint**: 006-skills' deferred v2 (dynamic skill grants) is closed, sharing US1's cycle.

---

## Phase 5: User Story 3 — Async / pollable approval with polling + timeout (P2)

**Goal**: Consent may resolve asynchronously (prompt/queue/webhook); a timeout denies (fail-closed).

**Independent test**: a sink that resolves after N polls → loop awaits then proceeds; a sink that never
resolves → timeout → deny, episode continues without hanging.

- [ ] T022 [P] [US3] Host tests: delayed-approver proceeds on grant; never-resolving approver hits the timeout and denies without hanging (SC-003), in `bee-harness/tests/dynamic_grants.rs`.
- [ ] T023 [US3] Wrap `consent.confirm` in `tokio::time::timeout(escalation_timeout, …)` mapping elapse → `Denied`; add `escalation_timeout`, `consent`, and `reactive_escalation` knobs to `LoopOptions`/`ReplConfig` in `bee-harness/src/grants/escalate.rs`, `bee-harness/src/episode.rs`, `bee-harness/src/repl.rs`. *(depends: T014)*

---

## Phase 6: User Story 4 — Narrowing: TTL leases + explicit de-escalation (P2)

**Goal**: Grants are leases; TTL expiry and `Flow::Deescalate` each trigger a narrowing reload that
removes the grant's rules at the kernel.

**Independent test**: a `Turns(1)` lease works on the granting turn and is denied the next turn; a
`Deescalate` drops a live grant and subsequent use is denied.

- [ ] T024 [P] [US4] Host tests: `Turns(1)` lease enforced on grant turn and narrowed on the next `TurnStart`; `Deescalate(id)` drops a live lease and de-registers its tool (SC-004), in `bee-harness/tests/dynamic_grants.rs`.
- [ ] T025 [US4] Implement lease TTL (`Turns`) + an expiry sweep at `TurnStart` that drops due leases, recomputes `active`, calls `Sandbox.reload`, de-registers tools no longer granted, and **emits a "narrowed" audit event** (dropped lease ids + `reason=expiry`, FR-015) in `bee-harness/src/grants/mod.rs` + `bee-harness/src/grants/escalate.rs`. *(depends: T008, T005)*
- [ ] T026 [US4] Implement `Flow::Deescalate(GrantId)` handling and an optional model-facing `release_capability` tool that raises it, **emitting a "deescalated" audit event** (lease id + `reason=deescalate`, FR-015) in `bee-harness/src/grants/escalate.rs` + `bee-harness/src/tools/`. *(depends: T025)*
- [ ] T027 [US4] VM matrix case `reload-narrow-deny` — granted write succeeds turn 1, then after lease expiry the same write is kernel-denied (assert the DATA did not land, not file absence — `file_open`-hook nuance) in `test/vm/remote-matrix.sh`. *(depends: T004, T025)*

---

## Phase 7: Polish & Cross-Cutting Concerns

- [X] T028 [P] Attenuation **property test** (proptest): random escalate/deescalate/expire sequences over a `[base, ceiling]` pair always keep `base ⊆ active ⊆ ceiling` and `active == base ∪ live-lease deltas` (Constitution II gate) in `bee-harness/tests/dynamic_grants.rs` or `bee-harness/src/grants/mod.rs`.
- [ ] T029 [P] VM matrix case `reload-scope-isolation` — widen one of two concurrent scopes; assert the other's enforced rules are unchanged (SC-007) in `test/vm/remote-matrix.sh`.
- [ ] T030 [P] Surface the capability history (grant/reload/expiry/deescalate events with provenance) in the transcript summary + `/audit` output in `bee-harness/src/transcript.rs`, `bee-harness/src/repl.rs`.
- [ ] T031 [P] Final gates: `cargo clippy --workspace --all-targets` clean, `cargo fmt`, doc comments; run full host suite + `bash test/vm/matrix.sh`; update memories `bee-skills-feature` and `bee-bpf-lsm-test-vm` (matrix count, dynamic-grants status).

---

## Dependencies & Execution Order

- **Setup (T001–T002)** → **Foundational (T003–T011)** blocks everything below.
  - Kernel chain: T003 → T004 → T005. Harness chain: T006, T007, T008 in parallel; then T009 → T010 → T011.
- **US1 (T012–T018)** depends on Foundational; is the MVP and unblocks US2–US4.
  - T012/T013 (tests) before T014; T014 → T015 → T016 → T017 → T018.
- **US2 (T019–T021)** depends on T014.
- **US3 (T022–T023)** depends on T014.
- **US4 (T024–T027)** depends on T005 + T008 (+ T014 for grant setup).
- **Polish (T028–T031)** after the stories it validates (T028 after T008/T025; T029 after T004; T030 after T017; T031 last).

US2, US3, and US4 are largely independent of each other once US1's `escalate()` exists — they can be
built in parallel by separate contributors.

## Parallel Execution Examples

- **Foundational fan-out**: T003, T006, T007, T008 run in parallel (distinct files/crates); T004/T005
  follow T003; T009 follows T006.
- **Per-story tests-first**: within each story the `[P]` test tasks (T012+T013, T019, T022, T024) are
  authored in parallel before their implementation tasks.
- **VM cases**: T018, T027, T029 touch the same `remote-matrix.sh` — author sequentially or in one
  editing pass, but they validate independent stories.

## Implementation Strategy

- **MVP = Phase 1 + Phase 2 + Phase 3 (US1)**: reactive escalation with auto-retry, proven on the VM
  (`reload-widen-allow`). This alone delivers dynamic grants end-to-end.
- **Increment 2 = US2**: closes 006-skills' deferred dynamic skill grants on the same cycle.
- **Increment 3 = US3 + US4**: async approval and least-privilege narrowing (the `reload-narrow-deny`
  VM case is the second load-bearing kernel proof).
- **Constitution gates run throughout**: test-first per security-boundary task, the T028 property test,
  and the VM matrix (T018/T027/T029) are non-negotiable before "done".

## Summary

- **Total**: 31 tasks — Setup 2, Foundational 9, US1 7, US2 3, US3 2, US4 4, Polish 4.
- **Independent test criteria**: each user-story phase states its own; every story is demonstrable
  alone once Foundational is done.
- **MVP**: User Story 1 (reactive escalate → reload → auto-retry), VM-proven.
