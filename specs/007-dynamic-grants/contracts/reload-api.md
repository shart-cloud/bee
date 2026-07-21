# Contract: Live Scope Reload (bee-userspace, `enforce` feature)

The one kernel-facing addition. Re-populates a live scope's rule maps from a freshly compiled plan,
without detaching programs or recreating the cgroup.

## `Engine::reload_scope`

```rust
#[cfg(feature = "enforce")]
impl Engine {
    /// Re-populate the maps for an already-created scope (`cgroup_id`) from `plan`. Supports both
    /// widening (added rules) and narrowing (removed rules). The LSM programs stay attached; only
    /// map contents change. Returns the NetKeys now installed, so the caller can update the Scope.
    ///
    /// Preconditions (caller-enforced): no sandboxed child is executing in this scope (turn
    /// boundary), and `plan` was compiled from a policy already validated `⊆ ceiling`.
    pub fn reload_scope(
        &mut self,
        cgroup_id: u64,
        plan: &EnforcementPlan,
        prev_net_keys: &[NetKey],
    ) -> Result<Vec<NetKey>, ScopeError>;
}
```

**Semantics** (mirrors `create_scope`'s population, minus cgroup creation):
- `SCOPES[cgroup_id] = plan.meta` — overwrite (mode/flags may change, e.g. write-default-deny latch).
- `FS_DENY[cgroup_id] = plan.fs_rules` — overwrite the single `DenyList` (atomic; covers widen+narrow).
  If `!plan.has_fs_rules`, `remove(cgroup_id)`.
- `EXEC_ALLOW[cgroup_id] = plan.exec_rules` — overwrite, or `remove` if none.
- `NET_ALLOW` — diff: `insert` keys in `plan.net_rules` (stamped with `cgroup_id`) not in
  `prev_net_keys`; `remove` keys in `prev_net_keys` not in the new set. Return the new key set.
- On any map error, return `ScopeError` **before** partial application where possible; the caller
  keeps the prior `ActivePolicy` and enforced maps (fail-closed, Constitution I).

**Rule-count cap**: `EnforcementPlan::prepare` already rejects `> DENY_MAX_RULES` (8). A widening that
would exceed it fails at plan compile, surfaced as a refusal (spec FR-016) — reload is never called.

## `Sandbox::reload` (bee-harness)

```rust
impl Sandbox {
    /// Apply a recompiled plan to the live scope. Host sandbox is a no-op (no kernel scope).
    pub fn reload(&mut self, plan: &EnforcementPlan) -> Result<(), SpawnError>;
}
```

- `Sandbox::Enforced(e)` / `Sandbox::Concurrent(c)` → `engine.reload_scope(scope.cgroup_id, plan,
  &scope.net_keys)` then `scope.net_keys = returned`.
- `Sandbox::Host(_)` → `Ok(())`.

Makes `EnforcedSandbox`'s currently-private `engine` reachable via this method only (no `pub` field).

## Invariants

- **INV-R1**: After `reload_scope`, the kernel enforces exactly `plan` for `cgroup_id` and nothing
  from a prior plan persists (verified: a narrowed path denies again; a widened path allows).
- **INV-R2**: A reload never touches another `cgroup_id`'s entries (concurrent-scope isolation).
- **INV-R3**: `reload_scope` is only correct at a turn boundary; calling it with a live child in the
  scope is a caller error (the harness guarantees the boundary).
