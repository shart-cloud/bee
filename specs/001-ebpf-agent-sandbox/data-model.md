# Phase 1 Data Model: bee

**Feature**: 001-ebpf-agent-sandbox · **Date**: 2026-07-19

Three representations exist and must stay consistent:
- **Authoring model** — the declarative `Policy` (TOML source of truth, `bee-core`).
- **Compiled model** — backend-neutral, host-resolved `CompiledPolicy` intent (`bee-core`).
- **Runtime model** — an eBPF-specific `EnforcementPlan` encoded for installed maps (`bee-userspace`).

Compilation never claims backend support. Enforcement planning either produces a complete runnable
plan or fails before kernel state is touched (constitution Principles I and IV).

---

## 1. Authoring entities (`bee-core`)

### Policy
The top-level declarative unit. Fields:
- `name: String`, `description: Option<String>`
- `filesystem: Vec<FsRule>`
- `exec: ExecPolicy`
- `network: NetPolicy`
- `exfiltration: ExfilPolicy` *(parsed in MVP; enforcement deferred to P3)*
- `mode: ScopeMode` — `Enforce` (default) | `Observe` (audit-only / dry-run) — FR-016

Validation: `name` non-empty; rules parse into the primitives below; deny-by-default holds (no rule ⇒
no access). Well-known protected paths (VCS metadata, bee config, `~/.ssh`, `~/.aws`) are injected as
`Deny`/read-only defaults unless explicitly overridden (FR-008).

### FsRule
- `pattern: PathPattern` — see below
- `mode: AccessMode` — `Read` | `Write` | `Deny`
- **Specificity**: the most specific matching rule wins; `Deny` at equal specificity beats grant.

### PathPattern (the glob surface, R6)
A parsed path spec that the compiler lowers to kernel primitives. Variants:
- `Exact(path)`
- `Subtree(dir)` — directory + everything beneath
- `Postfix(suffix)` — from `*.ext`
- `Segment(name)` — from `**/name`
- `BoundedStar(pattern)` — single-segment `*`, no `/`
- Patterns that cannot be lowered to the above (irreducible mid-path `**`) are a **compile error**
  (fail-closed).

Path tokens `:project_root`, `~`, and env-style expansions are resolved to absolute paths at compile time.

### AccessMode
`Read` | `Write` (implies read) | `Deny`. Represented in the kernel as a bitflag `{READ, WRITE}` plus a
deny sentinel.

### ExecPolicy
- `allow: Vec<ExecRule>` where `ExecRule = { pattern: PathPattern, pin_inode: bool }`
- Names resolve to absolute paths via `PATH` at compile time (R7). `pin_inode` preserves author intent;
  the current eBPF planner rejects it until inode enforcement exists.

### NetPolicy
- `allow: Vec<NetRule>` where `NetRule = { host: HostSpec, port: u16 }`
- `HostSpec = Domain(String) | Ip(IpAddr) | Cidr(IpNet)`
- Domains resolve to IP sets in user space at load time; `deny-all` implicit when `allow` non-empty (R8).

### ExfilPolicy *(deferred)*
- `enabled: bool`, `sensitive_paths: Vec<PathPattern>`. Disabled metadata compiles; enabled detection is
  rejected by the current eBPF planner until P3.

### DerivedPolicy / Attenuation (FR-005, SC-002)
Not a distinct struct but a **relation**: `derive(parent: &Policy, request: Policy) -> Result<Policy,
AttenuationError>`. A request is valid iff every capability it grants is provably ⊆ the parent's grants:
- FS: each derived `FsRule`'s match-set ⊆ some parent grant of ≥ access, with no parent `Deny` overlap.
  Decidable for `Exact`/`Subtree`; **conservative fail-closed** for glob-derived patterns when
  containment is not provable.
- Exec/Net: derived allow-set ⊆ parent allow-set.
- `mode` may only narrow (parent `Enforce` → child `Enforce`; a parent may not be forced looser).
Property-based tests assert: `∀ parent, child: derive(parent, child).is_ok() ⇒ child ⊆ parent`.

---

## 2. Scope (`bee-userspace`)

Represents one confinement boundary = one bee-managed cgroup v2 group (FR-004).
- `scope_id: String` — identifier unique within the process
- `cgroup_path: PathBuf` — `<parent_cgroup>/bee/<scope_id>`
- `cgroup_id: u64` — the kernel cgroup inode id (via `name_to_handle_at`; R4). **Key** for all runtime maps.
- `plan: EnforcementPlan` — completed entries installed for this scope
- `mode: ScopeMode`
- Lifecycle: `create → populate maps → move target pid in → run → teardown (remove maps + rmdir cgroup)`.
  bee owns creation and reclamation. Refuses to attach to a privileged target (FR-015).

---

## 3. Runtime model — EnforcementPlan (BPF maps, shared `#[repr(C)]` structs)

All keys embed `cgroup_id` so a single global program set enforces every scope (R4/R6).

| Map | Type | Key | Value | Purpose |
|-----|------|-----|-------|---------|
| `SCOPES` | HASH | `cgroup_id: u64` | `ScopeMeta { mode: u8, flags: u8 }` | is-this-cgroup-managed + enforce/observe |
| `FS_DENY` | HASH | `cgroup_id: u64` | bounded `DenyList` | subtree/postfix access rules |
| `EXEC_ALLOW` | HASH | `cgroup_id: u64` | bounded `DenyList` | executable path allowlist |
| `NET_ALLOW` | HASH | `{cgroup_id, family, addr:[16], port}` | allow flag | egress connect allowlist |
| `AUDIT_RB` | RINGBUF | — | `AuditRecord` | kernel → user-space events |

Constraints: every installed key/value struct is `#[repr(C)]` + `aya::Pod`, identical on both sides.
The current verifier-safe list supports at most eight effective filesystem rules and eight executable
rules per scope; the planner enforces those limits before installation.

---

## 4. AuditEvent (FR-007) — kernel `AuditRecord` → user-space `AuditEvent`

Kernel `AuditRecord` (`#[repr(C)]`, fixed size, emitted to `AUDIT_RB`):
- `ts_ns: u64` (`bpf_ktime_get_ns`), `cgroup_id: u64`, `pid: u32`, `tgid: u32`
- `op: u8` — `FileOpen | Exec | Connect | Exfil`
- `decision: u8` — `Denied | Observed` (observed = would-deny, dry-run allowed)
- `errno: i32`, `target: [u8; 256]` (path or `addr:port`, truncated)

User-space `AuditEvent` (serde-serializable to JSON; `bee-core::audit`):
```
{ "ts": "<ISO-8601>", "scope_id": "...", "cgroup_id": 0, "pid": 0,
  "op": "file_open|exec|connect|exfil", "decision": "denied|observed",
  "errno": -13, "target": "/home/jg/.ssh/id_rsa" }
```
Exposed via a synchronous, blocking iterator over `AUDIT_RB` (R9). `ts_ns` (monotonic) is mapped to
wall-clock ISO-8601 at the user-space boundary.

---

## 5. Entity relationships

```
Policy ──derive/attenuate──▶ DerivedPolicy (⊆ parent)
  │ compile
  ▼
CompiledPolicy ──eBPF planning──▶ EnforcementPlan ──loaded into──▶ BPF maps
  ▲                              │ consulted by
  │ owns                         ▼
Scope (1 cgroup) ──emits──▶ AuditEvent stream
```

State transitions (Scope): `New → Created(cgroup) → Loaded(maps) → Active(pid attached) → Torn-down`.
Any failure before `Active` tears down and returns an error (fail-closed).
