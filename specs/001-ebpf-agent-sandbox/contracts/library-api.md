# Contract: `bee-core` / `bee-userspace` Public Library API

**Feature**: 001-ebpf-agent-sandbox

This is the embeddable surface an agent framework depends on (constitution Principle V: library-first,
CLI is a thin wrapper over these same entry points). Signatures are illustrative Rust; exact names are
fixed here so `tasks.md` and tests can target them. The **core** (`bee-core`) requires no async runtime.

## `bee-core` — policy, attenuation, audit types (no eBPF, stable Rust)

```rust
// Parse & validate a policy from TOML (source of truth).
pub fn Policy::from_toml(s: &str) -> Result<Policy, PolicyError>;
pub fn Policy::from_path(p: &Path) -> Result<Policy, PolicyError>;

// Normalize and resolve author intent without choosing an enforcement backend.
// MUST complete < 50ms for ≤100 path + ≤50 net rules (SC-006).
pub fn Policy::compile(&self, resolver: &dyn Resolver) -> Result<CompiledPolicy, CompileError>;
// CompileContext carries: project_root, home dir, PATH (for exec name resolution),
// DNS resolver handle (for domain→IP), and clock — all injected (no ambient I/O in core logic).

// Attenuation: derive a subset policy. Rejects any over-grant (FR-005, SC-002).
// Returns the *effective* child policy, not the request: silence inherits rather than resets, so a
// dimension the request omits is filled from the parent and parent `deny` rules / inode pins the
// request dropped are re-added (research R15).
pub fn Policy::derive(&self, request: Policy) -> Result<Policy, AttenuationError>;

// Audit event (serde Serialize/Deserialize → JSON, FR-007).
pub struct AuditEvent { /* see contracts/audit-event.schema.json */ }
```

Errors: `PolicyError` (parse/validation), `CompileError` (incl. `UnsupportedGlob` for irreducible
patterns — fail-closed), `AttenuationError { capability, reason }`.

## `bee-userspace` — loader, scope lifecycle, audit stream (eBPF, stable Rust)

```rust
// One-time process init: detect support (R3) and load+attach LSM programs.
// FAILS CLOSED: Err on kernels lacking BPF-LSM / cgroup v2 (FR-009, SC-007).
pub fn Engine::init(cfg: EngineConfig) -> Result<Engine, EngineError>;
pub fn Engine::supported() -> Result<Support, EngineError>;   // diagnostics without attaching

// Purely prove that compiled intent is structurally runnable by the current eBPF backend.
// Unsupported capabilities fail here, before Engine initialization or cgroup creation.
pub fn EnforcementPlan::prepare(policy: &CompiledPolicy, mode: ScopeMode)
        -> Result<EnforcementPlan, PlanError>;

// Create a scope from a completed Enforcement Plan (creates cgroup, populates maps).
pub fn Engine::create_scope(&mut self, scope_id: &str, parent_cgroup: &str,
        plan: &EnforcementPlan) -> Result<Scope, ScopeError>;

// Produce the fork-safe cgroup join closure used by the hardened command launcher.
pub fn Scope::join_closure(&self) -> Result<impl Fn() -> io::Result<()>, ScopeError>;

// Synchronous, blocking audit stream (Q3, R9). No async runtime required.
pub fn Engine::take_audit_reader(&mut self, scope_id: &str) -> Result<AuditReader, ScopeError>;

// Teardown: removes maps + cgroup. Also runs on Drop.
pub fn Scope::teardown(&self) -> io::Result<()>;
```

### Contract guarantees (tested)
- **Truthful planning**: every successful plan contains only capabilities the current backend can
  install; unsupported intent is never weakened or deferred to Scope creation.
- **Attenuation**: `derive_scope` never yields a scope broader than its parent (property test, SC-002).
- **Fail-closed init**: on an unsupported kernel, `Engine::init` returns `Err`, never a no-op engine.
- **No ambient async**: `bee-core` compiles and links with no async runtime dependency (NFR-005).
- **Audit completeness**: every denied (enforce) or would-deny (observe) op yields exactly one event.
