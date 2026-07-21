# Phase 1 Data Model: Dynamic Capability Grants

Entities and their relationships. Types are illustrative (Rust-shaped) — the authoritative signatures
live in `contracts/`. All new types are `bee-harness`-side except where noted `bee-userspace`.

## ActivePolicy (bee-harness, runtime state)

The loop's mutable current policy, the source of truth for the next reload's compiled plan.

| Field | Type | Notes |
|---|---|---|
| `base` | `bee_core::Policy` | Immutable episode floor (from `scenario.policy_path`). |
| `ceiling` | `bee_core::Policy` | Immutable bound (from `ceiling_policy_path`, else `= base`). |
| `active` | `bee_core::Policy` | Current enforced policy; invariant `base ⊆ active ⊆ ceiling`. |
| `leases` | `Vec<GrantLease>` | Live grants layered onto `base` to produce `active`. |

**Invariant (property-tested, Constitution II)**: after any sequence of apply/expire/release,
`active == base ∪ (union of live leases' deltas)` and `ceiling.derive(active).is_ok()`. Reconstructing
`active` from `base + live leases` MUST equal the enforced policy — no orphaned rules.

**State transitions**:
- `apply(delta) → active'` — widen; requires `ceiling.derive(active ∪ delta).is_ok()` first.
- `expire(turn) → active'` — drop leases whose `expires_at_turn ≤ turn`; narrow.
- `release(id) → active'` — drop the named lease; narrow.
Each transition recompiles `active` and yields an `EnforcementPlan` for `Sandbox::reload`.

## GrantLease (bee-harness)

One approved grant, tracked so it can be narrowed.

| Field | Type | Notes |
|---|---|---|
| `id` | `GrantId` (stable string) | Target of `Flow::Deescalate` / `release_capability`. |
| `origin` | `GrantOrigin` | `ReactiveDenial{op,target}` \| `SkillRequires{skill}` \| `Explicit`. |
| `delta` | `GrantDelta` | The capability change (see below). |
| `ttl` | `Ttl` | `Turns(u32)` (primary) \| `Duration(std::time::Duration)` (interactive only). |
| `granted_at_turn` | `u32` | For turn-count expiry (`expires_at_turn = granted_at_turn + n`). |

Provenance feeds the audit trail (Constitution IV): every lease records who asked, for what, and until
when.

## GrantDelta (bee-harness)

The proposed capability change, in bee-core policy vocabulary — the same shape 006-skills'
`SkillRequires` parses into, now also produced by reactive denials.

| Field | Type | Notes |
|---|---|---|
| `tools` | `Vec<String>` | Harness tools to register (Layer 1; unaffected by reload). |
| `filesystem` | `BTreeMap<String, Access>` | Path→access to fold into the policy (Layer 2). |
| `exec` | `Vec<String>` | Exec-allow additions (Layer 2). |
| `net` | `Vec<String>` | `host:port` egress additions (Layer 2). |

A `ReactiveDenial` builds a minimal delta: for a `file_open` denial on `target`, `filesystem =
{ target → Read|Write }` inferred from the denied mode; for a `socket_connect` denial, a `net` entry;
for `bprm_check_security`, an `exec` entry.

## StepEvent / StepEventKind (bee-harness)

The lifecycle event a hook observes. Borrows loop state; never owns it.

| Variant | Emitted | Payload |
|---|---|---|
| `BeforeModelCall` | before each `complete` | `&Conversation`, `&[ToolSchema]` |
| `BeforeToolCall` | before `registry.execute` | `&ToolCall` |
| `AfterToolResult` | after audit drain | `&ToolCall`, `&ToolResult`, `&[AuditEvent]` |
| `KernelDenial` | when drained audit has `decision=="denied"` | `op`, `target`, `&ToolCall` |
| `TurnStart` | top of each turn | `turn_index` |

`StepEventKind` is the field-less discriminant used by `LoopHook::observes` to skip unwanted events.

## Flow (bee-harness)

A hook's returned action. First non-`Continue` in the stack wins for that event.

| Variant | Effect |
|---|---|
| `Continue` | Observe only; try the next hook. |
| `Deny(String)` | Skip the tool call; return the reason to the model (Rig `skip`). |
| `Escalate(GrantDelta)` | Resolve → reload → auto-retry the triggering call once. |
| `Deescalate(GrantId)` | Drop the named lease; narrowing reload. |
| `Terminate(String)` | End the episode with a reason (Rig `terminate`). |

## LoopHook (bee-harness, trait)

`#[async_trait] trait LoopHook { async fn on_event(&self, ev: &StepEvent) -> Flow; fn observes(kind)
-> bool { true } }`. Registered as `Vec<Box<dyn LoopHook>>` on `LoopOptions`/`ReplConfig`. The existing
`refresh_tools` closure is re-expressed as a `LoopHook` observing `TurnStart`.

Built-in hooks shipped with the feature:
- `SkillEscalationHook` — on `BeforeToolCall` for the `skill` tool, if the target skill's `requires`
  exceeds `active`, returns `Escalate(delta)` (US2, proactive).
- `DenialEscalationHook` — on `KernelDenial`, returns `Escalate(delta)` scoped to the denied resource
  (US1, reactive). Configurable: off by default in batch unless a ceiling is set.

## ConsentSink / Decision (bee-harness) — async

`#[async_trait] trait ConsentSink { async fn confirm(&self, req: &GrantRequest) -> Decision }`.
`Decision = Granted | Denied`. Non-interactive impls resolve immediately: `DenyAll → Denied`,
`AllowWithinCeiling → Granted`. Interactive/out-of-band impls may poll internally; the loop bounds the
await with a timeout that maps elapse → `Denied` (R6). This is the async evolution of 006-skills'
synchronous `ConsentSink`.

## Scope net-key tracking (bee-userspace) — extends existing `Scope`

| Field (new) | Type | Notes |
|---|---|---|
| `net_keys` | `Vec<NetKey>` | The `NET_ALLOW` keys installed for this scope, so a narrowing reload can `remove` dropped ones (R2). Populated at `create_scope`, updated at `reload_scope`. |

`Scope { path, cgroup_id }` gains `net_keys`. `SCOPES`/`FS_DENY`/`EXEC_ALLOW` need no tracking (single
value per `cgroup_id`, overwrite covers widen+narrow).

## Relationships

```
Scenario ──(policy_path)──▶ ActivePolicy.base
        └─(ceiling_policy_path)─▶ ActivePolicy.ceiling
ActivePolicy.leases ──(GrantLease.delta: GrantDelta)──▶ recompiled EnforcementPlan
LoopHook.on_event(StepEvent) ──▶ Flow::Escalate(GrantDelta) ──▶ ConsentSink.confirm
        ──▶ ActivePolicy.apply ──▶ Sandbox.reload(plan) ──▶ Engine.reload_scope(cgroup_id, plan)
                                                          └─▶ Scope.net_keys updated
```
