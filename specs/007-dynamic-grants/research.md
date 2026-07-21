# Phase 0 Research: Dynamic Capability Grants

All unknowns from the plan's Technical Context are resolved below. No open `NEEDS CLARIFICATION`.

## R1 — Is a live map reload atomic enough that no child sees a torn ruleset?

**Decision**: Reload by re-populating per-`cgroup_id` map entries via `aya::maps::HashMap::insert`
(overwrite) and `::remove`, performed **only at the turn boundary** while no sandboxed child is
executing in the scope.

**Rationale**: Two layers of safety. (1) The whole FS ruleset for a scope is a *single* `DenyList`
value at key `cgroup_id` (see `create_scope`, `FS_DENY`/`EXEC_ALLOW`), so a rewrite is one
`bpf_map_update_elem` — kernel-atomic per element; there is no partial-ruleset window. `SCOPES` meta
is likewise a single value. (2) bee's loop serializes tool execution — the escalation hook resolves
*before* the tool child is spawned, and lease-expiry/de-escalation narrowing resolves *after* the
child exits — so even the multi-key `NET_ALLOW` case never overlaps a running child. No new locking is
required; the turn structure is the critical section.

**Alternatives considered**: Map-in-map (swap a whole inner map atomically) — rejected as
over-engineering given single-value FS/exec maps already swap atomically and no child runs during a
reload. Double-buffered scopes (spin up a new cgroup, migrate) — rejected: re-attach cost, cgroup
churn, and audit-reader re-plumbing for no safety gain.

## R2 — Narrowing `NET_ALLOW`: how to remove the right keys without map iteration?

**Decision**: The `Scope` tracks the set of `NetKey`s it has installed. Reload diffs the new rule set
against the tracked set: `insert` additions, `remove` deletions, update the tracked set.

**Rationale**: `NET_ALLOW` is keyed `NetKey{cgroup_id, …}` (one entry per allowed dest), so narrowing
must delete dropped keys. aya 0.14 exposes `HashMap::remove(&mut self, key)` (confirmed in the
registry source). Enumerating a shared map's keys for one cgroup is racy and awkward; tracking the
installed keys in the `Scope` (userspace) is deterministic and cheap (low single-digit net rules).
`SCOPES`/`FS_DENY`/`EXEC_ALLOW` need no tracking — a single overwrite covers widen and narrow.

**Alternatives considered**: `HashMap::keys()` iteration + filter by `cgroup_id` — rejected as racy
against concurrent scopes and unnecessary given userspace already knows what it inserted.

## R3 — The `Engine` is moved into `EnforcedSandbox` and "held, never read". How does reload reach it?

**Decision**: Add `Engine::reload_scope(&mut self, cgroup_id, &EnforcementPlan)`. `EnforcedSandbox`
(and `ConcurrentSandbox`) expose `fn reload(&mut self, plan) -> Result<(), _>` that calls it with the
scope's `cgroup_id` and updates the tracked net-keys. `Sandbox::reload` dispatches; `Sandbox::Host`
returns `Ok(())` (no-op — no kernel scope). The loop holds `&mut Sandbox` already, so no ownership
change is needed beyond making the currently-private `engine` field reachable through the new method.

**Rationale**: The Engine owns `ebpf` (the maps) and must stay alive for the episode (dropping it
detaches the LSM programs — see the field comment and [[bee-bpf-lsm-test-vm]]). It is already `&mut`
at the point reloads happen (`run_loop` takes `sandbox: &mut Sandbox`). Exposing a `reload` method
keeps the Engine encapsulated and the map access inside `bee-userspace`.

**Alternatives considered**: Making `engine` `pub` — rejected (leaks aya types across the harness
boundary, violates encapsulation). A free function taking `&mut Engine` — rejected (Engine's `ebpf`
is private; the method belongs on `Engine`).

## R4 — Async hook + consent traits: object-safety with `dyn`?

**Decision**: Use `#[async_trait]` (already a workspace dep) for both `LoopHook` and the async
`ConsentSink`; store hooks as `Vec<Box<dyn LoopHook>>`. Dispatch awaits each hook in registration
order; the first non-`Continue` `Flow` wins (Rig's composition rule). Provide `LoopHook::observes`
so high-frequency events can be filtered without allocation.

**Rationale**: `async_trait` yields object-safe async trait methods, which the existing MCP/tool code
already relies on (`#[async_trait] impl Tool`). The hook stack is tiny; boxing cost is irrelevant next
to a model round-trip.

**Alternatives considered**: Native `async fn` in trait (Rust 2024) — object-unsafe for `dyn` today
without extra machinery; `async_trait` is the established pattern in this crate. Enum of hook kinds
instead of a trait — rejected: closes the set and blocks user-supplied hooks.

## R5 — Lease clock: turn count vs wall-clock duration?

**Decision**: `GrantLease::ttl` is primarily a **turn count** (`expires_after_turns`), evaluated
against the loop's turn index. A wall-clock `Duration` variant is *optional* and only consulted in
interactive/REPL contexts; the deterministic episode/batch path uses turns.

**Rationale**: Turn counts are deterministic and reproducible (batch/CTF), need no clock, and map
cleanly onto the loop's existing turn index. Wall-clock leases are useful for a human-paced REPL but
introduce nondeterminism, so they are opt-in and never the default in scored runs.

**Alternatives considered**: Wall-clock only — rejected (nondeterministic tests, reproducibility loss).
Model-token budget as the clock — rejected (indirect, hard to reason about for least-privilege).

## R6 — Async consent timeout semantics.

**Decision**: The loop wraps `ConsentSink::confirm(..).await` in `tokio::time::timeout(escalation_
timeout, ..)`. Elapse → `Decision::Denied`. The timeout is a `LoopOptions`/`ReplConfig` knob with a
conservative default; `DenyAll` returns `Denied` immediately, `AllowWithinCeiling` returns `Granted`
immediately (both used non-interactively).

**Rationale**: Constitution I (fail-closed) mandates deny on the absence of an approval. Wrapping the
await is the minimal, standard mechanism and keeps the sink implementation free to poll internally
(interactive prompt, queue, webhook).

**Alternatives considered**: No timeout (rely on the episode wall-clock deadline) — rejected: an
escalation should fail fast and deny, not stall the whole episode budget.

## R7 — Turn-boundary safety: is tool execution already serialized?

**Decision**: Yes — `run_loop` executes tool calls sequentially within a turn (`for tc in
&turn.tool_calls { … registry.execute(tc, sandbox).await … }`), one child at a time per scope.
Reloads are scheduled at these boundaries: escalation before `execute`, narrowing (expiry check) at
the top of each turn / after `execute`. No concurrency within a scope, so no lock is added.

**Rationale**: Confirmed by reading `episode.rs::run_loop`. The concurrent runner (US4) parallelizes
*across* scopes (distinct `cgroup_id`s), each with its own `Sandbox`, so per-scope serialization
holds there too.

## R8 — How is the auto-retry recorded in the transcript?

**Decision**: On an approved escalation the loop re-invokes `registry.execute` for the same
`ToolCall` once and records the **final** (post-reload) `RecordedCall`, annotated with an escalation
audit event (grant delta + reload) so the history shows "denied → granted → retried". The model's
conversation sees a single tool result (the successful retry), per SC-005/US1-AS-3. The initial denial
is preserved in the audit trail, not fed to the model as a separate tool result.

**Rationale**: US1 requires the model observe one successful result, while Constitution IV requires the
capability history be auditable. Recording the retry as the tool result plus escalation audit events
satisfies both. Retry is capped at once per escalation (SC-005) to prevent loops.

**Alternatives considered**: Feed both denial and success to the model — rejected (the user chose
auto-retry / single-success in the spec decisions). Retry N times — rejected (loop risk; an
approved-but-still-denied call must not re-escalate).

## Summary of decisions

| # | Decision |
|---|----------|
| R1 | Reload = per-cgroup map overwrite/remove at the turn boundary; single-value FS/exec maps are atomic; no child runs during reload. |
| R2 | `Scope` tracks installed `NetKey`s; reload diffs to add/remove; other maps overwrite. |
| R3 | `Engine::reload_scope` + `Sandbox::reload`; Host is a no-op; Engine stays encapsulated. |
| R4 | `#[async_trait]` `LoopHook`/`ConsentSink`; `Box<dyn>` stack; first non-`Continue` wins; `observes` filter. |
| R5 | Turn-count leases primary; optional wall-clock for interactive only. |
| R6 | `tokio::time::timeout` around async consent; elapse → deny (fail-closed). |
| R7 | Tool execution already serialized per scope; turn boundary is the reload critical section. |
| R8 | Record the post-reload retry as the tool result + escalation audit events; one retry per escalation. |
