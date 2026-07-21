# Feature Specification: Dynamic Capability Grants via a Loop Hooks Lifecycle

**Feature Branch**: `007-dynamic-grants`

**Created**: 2026-07-21

**Status**: Draft

**Input**: Extend 006-skills' preload-only capability grants to work *mid-episode*. Introduce an
events/hooks lifecycle in bee's own agent loop (Rig's `AgentHook`/`StepEvent`/`Flow` as the
blueprint — Rig's loop is unused, our `run_loop` owns the turns) so a scope's compiled eBPF policy
can be **reloaded live** — widening on escalation and narrowing on expiry/release — always bounded by
the attenuation ceiling and the kernel-authoritative invariant.

## Context & Motivation

006-skills resolves a skill's `requires` block **once, before the scope compiles** (preload). A
skill loaded mid-episode cannot gain capabilities, and a tool that hits an unforeseen kernel denial
cannot recover — the scope is fixed for the episode's life. This feature makes the active policy a
lease-managed range `[base, ceiling]` that moves at runtime:

- The `Engine` gains a `reload_scope` operation. Because every rule map is keyed by `cgroup_id` and
  the LSM programs attach globally, a reload is a re-population of that cgroup's map entries — no
  re-attach, no new cgroup. It is safe at the **turn boundary**, where no sandboxed child is running.
- `run_loop` (episode) and `run_exchange` (REPL) gain a shared **hooks lifecycle** — a generalization
  of today's per-turn `refresh_tools` closure — where hooks observe loop events and return a `Flow`
  action, including `Escalate`.
- The attenuation ceiling remains the unpromptable bound: the active policy never leaves
  `[base, ceiling]`, and every reload is validated `⊆ ceiling` before it touches the kernel.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Reactive escalation on a kernel denial, then auto-retry (Priority: P1)

An agent runs a tool that the current scope denies (e.g. `bash` reading a path outside the grant).
The kernel returns `EACCES`; instead of only surfacing the denial, the loop emits a `KernelDenial`
event carrying the exact `op` + `target`. A registered escalation hook proposes a grant for *that
path*; if approved (within the ceiling), the scope is reloaded and the **same tool call is retried
once** under the widened scope, now succeeding. If refused, the denial is fed back to the model
unchanged.

**Why this priority**: This is the MVP and the highest-value behavior — the agent discovers what it
needs by hitting a wall, and the grant is precise (scoped to the denied path). Delivers value even
without proactive or narrowing support.

**Independent Test**: A `MockModel` episode whose tool call targets a path the base policy omits but
the ceiling permits. With an approving consent sink, the transcript shows the initial denial, a grant
+ reload, and a successful retry of the same call. With a denying sink, it shows only the denial.

**Acceptance Scenarios**:

1. **Given** a base scope that denies `/data` and a ceiling that permits it, **When** the agent reads
   `/data/x` and the escalation hook is approved, **Then** the scope is reloaded, the read is retried
   once and succeeds, and the transcript records the grant + reload as auditable events.
2. **Given** the same setup but the request exceeds the ceiling, **When** the hook fires, **Then** the
   grant is refused *without* consulting the consent sink, and the denial is returned to the model.
3. **Given** an approved grant, **When** the retry runs, **Then** the model observes a single
   successful result (not a denial followed by a success) for that call.

---

### User Story 2 - Proactive escalation when a loaded skill needs capabilities (Priority: P1)

Mid-episode, the model loads a skill (via the `skill` tool) whose `requires` block asks for
capabilities beyond the active scope. Before the skill's instructions take effect, the loop escalates
the declared delta through the same cycle: attenuation check → consent → reload → continue. This
completes 006-skills' deferred v2 (dynamic grants) using the same machinery as US1.

**Why this priority**: Turns 006-skills' `requires` block into a runtime capability, not just a
preload one — the originating goal of this feature. Shares the escalation cycle with US1.

**Independent Test**: An episode with `skills` roots and a ceiling, where the model calls `skill` for
a capability-requesting skill after the scope is already up. The transcript shows the grant + reload
happening at load time, and a subsequent tool call using the new capability succeeds.

**Acceptance Scenarios**:

1. **Given** a skill whose `requires` sits within the ceiling, **When** the model loads it mid-episode
   and consent is granted, **Then** the scope is reloaded and the skill's capabilities are live for
   subsequent tool calls.
2. **Given** a skill whose `requires` exceeds the ceiling, **When** the model loads it, **Then** the
   grant is refused, the skill's body still loads (instructions-only degrade), and the model is told
   the capability was not granted.

---

### User Story 3 - Async / out-of-band approval with polling (Priority: P2)

Approval need not be an inline prompt. A `ConsentSink` may resolve asynchronously — an interactive
REPL prompt, or an out-of-band approver (a queue / webhook / human on another channel) the sink polls
until it returns a decision or the loop's timeout elapses. A timeout resolves as **deny**
(deny-by-default). This is what makes escalation usable in headless and collaborative runs.

**Why this priority**: The trigger and reload machinery (US1/US2) work with a synchronous sink; async
approval widens applicability but is separable.

**Independent Test**: A consent sink that returns `Granted` only after N polls (simulated delay). The
episode blocks on escalation until the sink resolves, then proceeds; a sink that never resolves hits
the timeout and the grant is denied.

**Acceptance Scenarios**:

1. **Given** an approver that resolves after a delay, **When** an escalation fires, **Then** the loop
   awaits the decision (up to the timeout) and proceeds on `Granted`.
2. **Given** an approver that never responds, **When** the escalation timeout elapses, **Then** the
   grant is denied and the episode continues fail-closed.

---

### User Story 4 - Narrowing: TTL leases auto-revoke, and explicit de-escalation (Priority: P2)

A grant is a **lease**, not a permanent widening. Each approved grant carries a TTL (a turn count or
duration); when it expires, the loop narrows the scope back — a reload that *removes* the grant's
rules. Separately, a `Deescalate` flow action (from a hook, or a model-facing `release_capability`
tool) drops a grant before its TTL. Narrowing keeps least-privilege: capabilities are raised for the
operation that needs them and dropped afterward. (Skill-scoped auto-revoke is explicitly out of
scope — a loaded skill has no defined end-of-life.)

**Why this priority**: Strengthens least-privilege but is not required for escalation to deliver
value; builds on the same `reload_scope` primitive (now exercising removal).

**Independent Test**: A grant with a 1-turn TTL: the capability works on the granting turn, and a
tool call using it on a later turn is denied again (the lease expired → scope narrowed). Separately,
a `Deescalate` action drops a live grant and a subsequent use is denied.

**Acceptance Scenarios**:

1. **Given** a grant leased for N turns, **When** turn N+1 begins, **Then** the scope is narrowed
   (grant rules removed) and a tool call relying on it is denied by the kernel.
2. **Given** a live grant, **When** a `Deescalate` action fires for it, **Then** the scope is reloaded
   without that grant's rules and subsequent use is denied.
3. **Given** a narrowing reload, **When** it is validated, **Then** the resulting active policy is
   still `⊆ ceiling` (narrowing is trivially within the ceiling) and is recorded as an audit event.

### Edge Cases

- **Turn-boundary safety**: a reload MUST occur only when no sandboxed child is executing in the scope
  (between tool calls). The loop already serializes tool execution, so escalation resolves before the
  child spawns and narrowing resolves after it exits.
- **Rule-count ceiling**: a widening that would exceed `DENY_MAX_RULES` (8 fs/exec rules) MUST be
  refused with a clear reason, not silently truncated — even if within the policy ceiling.
- **Retry loops**: an approved-then-still-denied retry (e.g. a path the kernel matcher can't express)
  MUST NOT re-escalate indefinitely — a call is retried at most once per escalation.
- **Concurrent episodes (US4 concurrency model)**: each scope reloads independently by its own
  `cgroup_id`; a reload in one scope MUST NOT affect another's maps.
- **Batch / CTF reproducibility**: with no interactive approver, the async sink resolves via the
  ceiling-as-pre-authorization model (grant within ceiling) or deny — never an interactive hang.
- **Host build (no enforce)**: reload is a no-op on the kernel (there is no scope); Layer-1 tool
  registration and the hooks lifecycle still function so the loop behaves identically in tests.

## Requirements *(mandatory)*

### Functional Requirements

**Hooks lifecycle**

- **FR-001**: The agent loop MUST expose a hooks lifecycle emitting, at minimum, `TurnStart`,
  `BeforeModelCall`, `BeforeToolCall`, `AfterToolResult`, and a bee-specific `KernelDenial` event
  (`TurnStart` is the point at which lease expiry is evaluated, US4); hooks return a `Flow` action and
  MAY be async.
- **FR-002**: `Flow` MUST include at least `Continue`, `Deny(reason)`, `Escalate(delta)`,
  `Deescalate(grant)`, and `Terminate(reason)`. The first hook returning a non-`Continue` action wins
  for that event (Rig's composition rule).
- **FR-003**: The existing per-turn `refresh_tools` behavior (004-mcp-client, FR-043) MUST be
  expressible as a registered hook, and `run_loop` (episode) and `run_exchange` (REPL) MUST share the
  same lifecycle rather than duplicating tool-loop logic.

**Escalation cycle**

- **FR-004**: On `Escalate(delta)` the loop MUST resolve the grant with the same attenuation + consent
  discipline as 006-skills: validate `active ∪ delta ⊆ ceiling` *before* consulting consent; a
  beyond-ceiling request is refused without prompting.
- **FR-005**: A `KernelDenial` event MUST carry the denied `op` and `target` so a reactive hook can
  propose a grant scoped to exactly that resource.
- **FR-006**: On an approved escalation the loop MUST reload the scope and then **auto-retry the
  triggering tool call exactly once** under the widened scope; a refused escalation MUST feed the
  original denial/result back to the model.
- **FR-007**: Consent MUST be resolvable asynchronously; the loop MUST bound the wait with a timeout,
  and a timeout MUST resolve as **deny** (deny-by-default, Constitution I).

**Live reload (kernel)**

- **FR-008**: `bee-userspace` MUST provide a scope-reload operation that re-populates a live scope's
  rule maps (`SCOPES`, `FS_DENY`, `EXEC_ALLOW`, `NET_ALLOW`) for a given `cgroup_id` from a newly
  compiled plan, without detaching programs or recreating the cgroup.
- **FR-009**: Reload MUST support both **widening** (add rules) and **narrowing** (remove rules);
  single-value-per-cgroup maps overwrite, and `NET_ALLOW` MUST diff and remove dropped keys.
- **FR-010**: Every reload — widen or narrow — MUST be preceded by an attenuation validation that the
  resulting active policy is `⊆ ceiling`; the kernel MUST never enforce a policy the ceiling does not
  permit (Constitution II/III).
- **FR-011**: A reload MUST occur only at a turn boundary with no sandboxed child executing in the
  scope; the loop MUST NOT reload while a tool child is live.

**Narrowing**

- **FR-012**: An approved grant MUST be recordable as a **lease** with a TTL (turn count or duration);
  the loop MUST narrow the scope automatically when a lease expires.
- **FR-013**: A `Deescalate` action (hook-driven, or via an optional model-facing
  `release_capability` tool) MUST drop a named live grant before its TTL, triggering a narrowing
  reload.
- **FR-014**: Skill-scoped auto-revocation is explicitly NOT provided; narrowing is driven only by TTL
  expiry and explicit de-escalation.

**Auditing & limits**

- **FR-015**: Grant, reload (widen/narrow), lease-expiry, and de-escalation events MUST be recorded in
  the transcript/audit trail with enough provenance (which skill/hook/turn, what delta) to review the
  capability history of an episode (Constitution IV).
- **FR-016**: A widening that would exceed `DENY_MAX_RULES` MUST be refused with a clear reason rather
  than truncated.

### Key Entities

- **ActivePolicy**: the loop's mutable current policy, initialized to `base`, always kept within
  `[base, ceiling]`. Source of truth for the next reload's plan.
- **GrantLease**: an approved grant with provenance (origin: reactive-denial | skill `requires` |
  explicit), the capability delta, a TTL (turns or duration), and a stable id for de-escalation.
- **StepEvent / Flow**: the lifecycle event a hook observes and the action it returns.
- **ConsentSink / Decision**: the async approval seam; `Decision` is `Granted | Denied`; a timeout
  maps to `Denied`.
- **GrantDelta**: the capability change proposed by an escalation (tools + filesystem/exec/net rules),
  expressed in the bee-core policy vocabulary.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: On the BPF-LSM VM, a widening reload makes a previously-denied operation succeed on
  auto-retry, and a narrowing reload makes a previously-granted operation fail again — both proven in
  the enforcement matrix (`test/vm/remote-matrix.sh`).
- **SC-002**: No reload ever produces an active policy outside `[base, ceiling]`; a beyond-ceiling
  escalation is refused without consulting the consent sink (host-testable, plus a VM negative case).
- **SC-003**: An escalation timeout with an unresponsive approver denies the grant and the episode
  continues fail-closed (no hang), verified headless.
- **SC-004**: A TTL-leased grant is enforced on the granting turn and denied on the turn after
  expiry, with the narrowing reload visible in the audit trail.
- **SC-005**: A triggering tool call is retried at most once per escalation; an approved-but-still-
  denied call does not re-escalate.
- **SC-006**: `run_loop` and `run_exchange` share one hooks lifecycle — the tool-execution/audit-drain
  path is not duplicated — and the existing 006-skills preload path and all prior matrix cases still
  pass unchanged.
- **SC-007**: Concurrent episodes each reload independently; a widen/narrow in one scope leaves other
  scopes' enforced rules unchanged (extends the US4 isolation case).

## Assumptions

- The attenuation ceiling is fixed at episode/session start (compiled as the bound); only the *active*
  policy moves at runtime. Changing the ceiling mid-episode is out of scope.
- The eBPF program set and map layout are those of 001-ebpf-agent-sandbox; reload reuses the existing
  `EnforcementPlan` compile path, so the `DENY_MAX_RULES=8` and matcher-expressiveness limits carry
  over unchanged.
- Reloads are infrequent (per escalation / lease boundary), so re-running the plan compile per reload
  is acceptable; no incremental-diff compiler is required.
- The loop already serializes tool execution (one child at a time per scope), providing the
  turn-boundary safe point for reloads without new locking.

## Out of Scope

- Changing the ceiling at runtime.
- Skill-scoped auto-revocation (only TTL + explicit de-escalation narrow).
- An incremental map-diff compiler (full re-compile per reload is acceptable).
- Adopting Rig's `AgentRunner`/`AgentHook` machinery (Rig stays behind the `Model` seam; the lifecycle
  is bee's own, Rig-shaped).
- Cross-scope or cross-episode capability sharing.

## Constitution Alignment

- **I — Deny-by-default & fail-closed**: escalation starts from `base`; timeout/no-approver → deny;
  narrowing returns to less capability.
- **II — Capability attenuation**: every reload validated `⊆ ceiling`; consent cannot exceed it.
- **III — Kernel enforcement authoritative**: user space recompiles and reloads maps; the kernel
  decides. Reload never bypasses the LSM.
- **IV — Policy-as-data**: the ceiling file is the reviewable pre-authorization; grants/reloads are
  recorded as diffable audit events.
- **V — Library-first, runtime-free core**: the reload primitive lives in `bee-userspace` (enforce);
  the hooks lifecycle in `bee-harness`; bee-core/bee-common stay untouched.
