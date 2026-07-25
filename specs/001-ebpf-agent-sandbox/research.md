# Phase 0 Research: bee — eBPF-Enforced Sandbox Harness

**Feature**: 001-ebpf-agent-sandbox · **Date**: 2026-07-19

This document resolves the technical unknowns in the plan's Technical Context. Each entry
records the Decision, Rationale, and Alternatives considered. Findings are grounded in current
(late 2025 / mid 2026) Aya and Linux kernel sources; key citations are listed at the end.

---

## R1. eBPF toolchain (Aya versions, build, nightly)

**Decision**: Rust workspace. User-space crates target **stable** Rust. The single eBPF probe crate
(`bee-ebpf`) targets **nightly** (`rust-src`, `-Z build-std=core`, target `bpfel-unknown-none`) and
is compiled via **`aya-build`** invoked from `bee-userspace`'s `build.rs`. Pin `aya` ≈ **0.14**,
`aya-ebpf` ≈ **0.2.1**, `aya-build` ≈ **0.1.2**. Require `bpf-linker` with an LLVM version matched to
the pinned nightly (currently LLVM 21).

**Rationale**: Satisfies constitution Principle V / NFR-003 (stable Rust for user space; nightly
confined to the eBPF crate). `aya-build` in a build script gives an ordinary `cargo build` workflow.

**Alternatives considered**: libbpf-rs / BCC (pulls in C toolchain + clang at runtime, heavier, not
pure-Rust); hand-invoking the bpf target without `aya-build` (works but more fragile CI). The top
build-break risk is **nightly ↔ bpf-linker ↔ LLVM version skew** — the three MUST be pinned together
in `rust-toolchain.toml` + CI.

---

## R2. LSM program type, hooks, attach, return semantics

**Decision**: Use Aya's `#[lsm(hook = "…")]` (program type `BPF_PROG_TYPE_LSM`, attach type
`BPF_LSM_MAC`). MVP hooks: **`file_open`**, **`bprm_check_security`**, **`socket_connect`**. Load with
`Btf::from_sys_fs()` (BTF mandatory), `program.load(hook, &btf)`, `program.attach()`. Return **`0` =
allow**, **negative errno = deny** (`-EACCES` for file/exec, `-EPERM` for network).

**Rationale**: These three hooks cover FR-001/002/003. All are valid attachable hooks exposed via
BTF on kernels ≥ 5.7. `inode_permission` (FR-001 secondary) and `socket_sendmsg` (exfil, P3) are
**deferred** — not attached in the MVP.

**Critical gotchas** (must be encoded in code + tests):
- **Do NOT confuse with `BPF_LSM_CGROUP`** (kernel ≥ 6.0), whose return convention is *inverted*
  (`1` = allow). Aya's `#[lsm]` macro is `BPF_LSM_MAC`; we filter by cgroup id in-program (R4) rather
  than using cgroup-attach. Returning `1` from a `BPF_LSM_MAC` program would be a subtle allow/deny bug.
- LSM decisions are a logical AND across the chain; a BPF program cannot turn a prior denial into an
  allow. The previous program's result arrives as the trailing `ret` arg.
- Newer kernels range-check LSM return values; return only `0` or a valid `-errno`.

**Alternatives**: `BPF_LSM_CGROUP` (kernel does per-cgroup scoping, no manual id filter) — rejected for
MVP: requires kernel ≥ 6.0, inverted semantics, and less-mature Aya support. Revisit post-MVP.

---

## R3. Kernel enablement + fail-closed startup detection (FR-009, SC-007)

**Decision**: At startup, gate enforcement behind this ordered check; **any failure ⇒ refuse to start**:
1. Kernel ≥ 5.7.
2. `/sys/kernel/security/lsm` exists and its comma-separated list contains an exact **`bpf`** token.
3. cgroup v2 is mounted (unified hierarchy at `/sys/fs/cgroup`).
4. **Compiled kernel struct offsets match the running kernel's BTF** (`kbtf::check_running`): the eBPF
   probe reads `file->f_mode`/`file->f_path`/`linux_binprm->file` at hardcoded offsets
   (`bee_common::offsets`), and a wrong offset reads the wrong field and fails **open**. bee parses
   `/sys/kernel/btf/vmlinux` and refuses if any offset disagrees — closing the CO-RE fail-open flagged
   in R5. Surfaced as the `kernel offsets [PASS/FAIL]` gate in `bee check`.
5. `Btf::from_sys_fs()` succeeds (BTF present).
6. Actual `load()` + `attach()` of the LSM programs succeeds.

**Rationale**: `CONFIG_BPF_LSM=y` alone is insufficient — `bpf` must be in the *active* LSM list, which
is fixed at boot via `CONFIG_LSM` / the `lsm=` cmdline. `/sys/kernel/security/lsm` is the authoritative
runtime signal. Steps 1–5 give clear diagnostics; step 6 is the ultimate proof. Step 4 exists because
aya-ebpf 0.2.1 emits no CO-RE field relocations (R5/R6), so offsets are compile-time constants that
must be validated at runtime, not trusted. This directly implements the constitution's fail-closed
principle and SC-007.

**Alternatives**: Trusting `CONFIG_BPF_LSM` from `/proc/config.gz` (present but not proof of active
list); attempting attach only (works but yields worse diagnostics). We do both: diagnose then prove.

---

## R4. Per-cgroup scoping (FR-004)

**Decision**: Attach LSM programs **globally**; scope in-program via `bpf_get_current_cgroup_id()`
(returns the cgroup v2 inode id, `u64`). bee creates one cgroup per scope under a configurable parent
(default `/sys/fs/cgroup/bee/<scope-id>`), obtains that cgroup's id in user space via
**`name_to_handle_at(2)`** on the cgroup directory (the `f_handle` bytes are the 64-bit id; `statx`
`st_ino` yields the same value on cgroupfs), and records it in an `allowed-scopes` map. The target
process is placed into the cgroup before `execve` (via `CLONE_INTO_CGROUP` where available, else
writing the pid to `cgroup.procs` pre-exec).

**Rationale**: `BPF_LSM_MAC` cannot be attached per-cgroup, so the id filter is the portable mechanism.
`name_to_handle_at` is the officially documented way to obtain the id that matches
`bpf_get_current_cgroup_id()`. bee owning the cgroup lifecycle satisfies the Q2 clarify decision.

**Alternatives**: `BPF_LSM_CGROUP` (see R2 — deferred). Reading `st_ino` only (works, but `f_handle` is
the blessed path; we use `name_to_handle_at` with `statx` as a cross-check).

---

## R5. In-kernel path retrieval (FR-001, FR-002)

**Decision**: In `file_open` and `bprm_check_security`, obtain the resolved path with
**`bpf_d_path(&file->f_path, buf, sz)`** into a **per-CPU array** buffer sized to `PATH_MAX` (4096).

**Rationale**: `bpf_d_path` is gated by a hard-coded BTF-ID allowlist, and both `security_file_open`
and `security_bprm_check` are **on that allowlist** — so this is the kernel-blessed way to get a path
in exactly our two hooks. It returns the fully mount-resolved canonical path, avoiding a hand-rolled
dentry walk. The 512-byte stack limit forces the buffer into a per-CPU map, not the stack.

**Symlink/TOCTOU** (spec edge case): LSM hooks fire **after** full path resolution, so the hook sees
the resolved target, not the symlink — inherently TOCTOU-resistant. Consequence to document: a policy
rule written against a *symlink's* name will not match (the hook sees the target); and name-based rules
do not catch hard links / alternate bind mounts to the same inode. This is a documented property, and
motivates the optional `(dev,ino)` fast path in R6 for rules that need identity-based guarantees.

**Alternatives**: manual dentry-chain walk (fiddly, builds the string backwards, needs `bpf_loop`);
`(dev,ino)`-only matching (robust but cannot express prefixes/globs — see R6).

---

## R6. Path matching strategy: exact / subtree / glob (FR-001, Q4 clarify)

**Decision**: **Compile the policy language's globs in user space into a small set of verifier-friendly
in-kernel primitives.** The kernel side does only O(1) lookups plus a couple of fixed-bound compare
loops per hook. Concretely, each filesystem rule lowers to one of:

| Primitive | Matches | Kernel structure |
|-----------|---------|------------------|
| `exact` | one full path | hash map of path bytes; optional `(dev,ino)` variant for TOCTOU-hard rules |
| `subtree` (prefix) | a directory and everything under it | **LPM trie** keyed on `cgroup_id ‖ path` (see below), ≤ 256 path bytes |
| `postfix` | a literal suffix, e.g. `.log` (compiled from `*.log`) | bounded suffix compare (≤ 128 bytes) |
| `segment` | a path component equals a literal, e.g. `target` (from `**/target`) | bounded segment scan |
| `bounded-star` | single-segment `*` (no `/`) via a backtrack-free two-pointer matcher, fixed caps | bounded matcher (≤ 128-byte pattern) |

**Irreducible patterns** (e.g. mid-path greedy `**` such as `/var/**/secret/*.key` that cannot be
lowered to the above) are **rejected at policy-compile time** with a clear error — fail-closed per
constitution Principle I. The policy compiler documents exactly which glob shapes are supported.

**cgroup-scoped keying without map-in-map**: the LPM trie key is `{prefixlen, [8-byte cgroup_id][path
bytes]}`. Because every rule for a scope carries the full 64-bit cgroup id as a fixed leading prefix,
longest-prefix-match naturally requires an exact cgroup-id match before any path bits are considered.
This scopes rules per cgroup in a single flat trie, avoiding `HASH_OF_MAPS` complexity. Exact/hash and
network maps use a compound key struct `{cgroup_id, …}`.

**Rationale**: Grounded directly in prior art — **no** production eBPF tool (Tetragon, KubeArmor,
bpfbox) does true in-kernel backtracking glob matching; general `**` provably exceeds the 1M-instruction
verifier limit. Tetragon uses prefix (≤256) / postfix (≤128) primitives; the inode-keyed tools cannot do
prefixes at all. Lowering globs to these primitives in user space keeps the promised glob *surface*
(satisfies Q4: `*.log`, `**/target` work) while staying verifier-safe.

**Attenuation impact** (spec edge case): subset checking is decidable for `exact`/`subtree`; for glob-
derived rules the validator is **conservative and fails closed** when containment cannot be proven
(SC-002). Property-based tests cover this.

**Alternatives**: full in-kernel glob engine (rejected — verifier instruction blowup, confirmed by
KubeArmor's abandoned attempt); user-space upcall per open for glob decisions (rejected for the hot path
— synchronous round-trip per syscall reintroduces a race window and violates NFR-001; retained only as a
possible slow-path escape hatch for rare irreducible patterns, out of MVP scope).

---

## R7. Exec allowlisting (FR-002)

**Decision**: In `bprm_check_security`, get the executable path via `bpf_d_path(&bprm->file->f_path, …)`
and match against a per-scope allowlist compiled from executable names/paths (exact + subtree via the
same primitives as R6). Provide an optional `(dev,ino)` exact fast path for TOCTOU-hard binary pinning.
Executable *names* in policy (e.g. `cargo`) are resolved to absolute paths via `PATH` at policy-load
time in user space.

**Rationale**: Matches Tetragon's `matchBinaries` approach; `security_bprm_check` is on the `bpf_d_path`
allowlist. Name→path resolution belongs in user space (policy-as-data, Principle IV).

**Alternatives**: inode-only allowlist (robust but cannot express "any binary under /opt/app/bin");
raw `execve` tracepoint (not TOCTOU-safe — rejected).

---

## R8. Network egress enforcement (FR-003)

**Decision**: In `socket_connect`, read the destination `sockaddr` (IPv4/IPv6 addr + port), look up a
per-scope hash map of allowed `{cgroup_id, addr, port}` tuples; deny (`-EPERM`) if absent. Domain names
in policy are resolved to IP sets **in user space at policy-load time** and refreshed on a configurable
interval by updating the map entries live (FR-006). `deny-all` is implicit when an allowlist is present.

**Rationale**: In-kernel DNS is impractical; user-space resolution + live map update is the standard
pattern and honors Principle IV. Aya supports runtime map updates while programs stay attached.

**Open item (OQ-005, deferred)**: interaction with host DNS and connections proxied through a local
resolver — documented in quickstart, not enforced in MVP.

**Alternatives**: `socket_sendmsg`/`sendto` filtering for connectionless flows (deferred with exfil P3);
cgroup/skb network programs (different attach model; the LSM `socket_connect` hook is sufficient for
egress connect-time control).

---

## R9. Audit event transport + audit-only mode (FR-007, FR-016, Q3/Q5 clarify)

**Decision**: Kernel programs emit fixed-size `#[repr(C)]` audit records to a **BPF ring buffer**
(`BPF_MAP_TYPE_RINGBUF`). User space exposes them through a **synchronous, blocking iterator** that
yields `AuditEvent`s (JSON-serializable via serde); no async runtime in the core. A per-scope
**mode flag** (`enforce` | `observe`) lives in the `allowed-scopes` map: in `observe` (audit-only /
dry-run) mode the hook emits the audit record but returns `0` (allow).

**Rationale**: Ring buffer is the modern, lower-overhead successor to perf buffers and preserves event
ordering. Synchronous iterator satisfies Q3 + NFR-005. The mode flag makes dry-run a one-branch change
in each hook (Q5), enabling policy development against real workloads (SC-004).

**Alternatives**: perf event array (older, per-CPU ordering quirks); async `Stream` / Unix-socket
exporter (deferred per Q3 — can wrap the sync iterator later without redesign).

---

## R10. Process hardening (FR-011)

**Decision**: Hardening is applied by the **launcher** in the forked child *between fork and exec*:
`prctl(PR_SET_DUMPABLE, 0)`, `setrlimit(RLIMIT_CORE, 0)`, and stripping `LD_*` environment variables,
then `execve`. A separate `bee-hardening` crate provides an optional `#[ctor]` entry point for the case
where an agent embeds bee **in its own process** and wants pre-`main` hardening there.

**Rationale**: The `#[ctor]` approach only runs if the sandboxed binary links bee — untrue for arbitrary
tools like `cargo`/`rustc`. The robust, general path is the launcher applying hardening in the child
pre-exec. The ctor crate covers the embedded-in-agent case (goals doc's original framing).

**Alternatives**: rely solely on `#[ctor]` (rejected — doesn't cover third-party tool binaries).

---

## R11. Required privileges (Assumptions)

**Decision**: The loader (bee itself) requires elevated privilege to load/attach LSM programs and manage
cgroups: target **`CAP_BPF + CAP_PERFMON` (+ `CAP_MAC_ADMIN`)**, or simply run as root / `CAP_SYS_ADMIN`.
The **sandboxed** process remains unprivileged and enforcement is transparent to it. bee MUST **refuse to
attach a policy to a privileged target** (setuid or holding broad capabilities) — FR-015.

**Rationale / uncertainty**: The exact minimal capability set for `BPF_LSM_MAC` attach is not fully
pinned down in primary kernel docs — `CAP_MAC_ADMIN` may or may not be strictly required depending on
kernel version. Treat the minimal set as **empirically verified per target kernel** in integration CI;
default runbook uses `CAP_SYS_ADMIN` for reliability and documents the granular set as best-effort.

---

## R12. Testing & CI strategy

**Decision**: Unit tests (stable, no kernel) for the policy parser, compiler/lowering, and attenuation
validator — the last with **property-based tests** (proptest). Enforcement behavior is validated by
**integration tests on a real BPF_LSM-capable kernel** in a VM (e.g. QEMU/`vmtest`), gated in CI, run
with the required privileges. Performance gates (NFR-001 ≤ 5µs/hook via `bpf_ktime_get_ns`
instrumentation; SC-006 compile < 50ms via criterion) run as benchmark tests.

**Rationale**: Constitution mandates test-first for the security boundary and real-kernel integration,
not mocks. The privileged VM runner is the only way to prove allow/deny outcomes.

**Alternatives**: mocking the LSM layer (rejected — proves nothing about actual enforcement).

---

## R13. Filesystem read/write modes + default-access decision (FR-001, US2 AS-2)

**Context**: `file_open` originally enforced a deny-list only; a rule's *mode* (`read`/`write`/`deny`)
was not honored, so a `read`-marked path did not block writes. Enforcing read-only requires reading
the requested access (`FMODE_WRITE` bit of `file->f_mode`, direct load at `offsetof(file,f_mode)=20`
on 6.8 x86_64) and, critically, deciding what happens when **no rule matches** an open.

**Decision** — within a scope that declares any filesystem policy, `file_open` applies **most-
specific-match-wins, deny-breaks-ties** (reference impl `bee_common::matcher::fs_open_blocked`,
mirrored verifier-side in `bee-ebpf::fs_should_block`, both host-unit-tested):
1. The **most-specific matching rule** governs (postfix/`*.ext` rules outrank subtree rules; longer
   patterns outrank shorter); a `deny` of equal-or-greater specificity beats a grant.
2. That rule decides: `deny` ⇒ block; a grant ⇒ block iff it does not grant the requested access
   (read for a read-open, write for a write-open). Grants always include read, so a **read** is only
   ever blocked by a `deny` (**allow-by-default reads**); a `read`-only rule nested in a writable
   subtree blocks writes to it (e.g. `.git` inside a writable project — FR-008).
3. **No matching rule** — reads allowed; **writes deny-by-default** *iff the scope declares ≥1
   `write` grant* (the `FLAG_FS_WRITE_DEFAULT_DENY` latch). A scope that declares no writable root
   does not have its write surface managed by bee, so such writes are allowed unless explicitly denied.

**Verifier technique (learned the hard way).** "Most-specific wins" is realized by **sorting the rule
list most-specific-first at load time** (`rule_sort_key`) so the kernel decides by the *first* matching
rule and scans with an **early return**. An in-kernel loop that instead tracks a running "best
specificity" across all rules (no early return) explodes verifier state and overran the 1M-instruction
budget even at 8 rules — the early-returning scan is the same shape as the exec allowlist match and
verifies with margin. This is why `DENY_MAX_RULES` stays at 8 and rules are pre-sorted, not ranked
in-kernel.

**Divergence from pure global write-deny-by-default (justified vs. Principle I).** The constitution's
deny-by-default ideal argues every unmatched write should be denied unconditionally. We arm write-
deny-by-default on the *presence of write intent* instead, because a truly global write-deny requires
every scope to enumerate device pseudo-files (`/dev/null`, `/dev/tty`, `/dev/pts/*`, `/proc/self/*`)
or break basic tooling — an allowlist so broad and easy to under-specify that it becomes fail-open by
omission (the opposite of the principle's intent). Arming on write intent preserves deny-by-default
for any scope that manages its writable roots (the case the principle targets), while a read-only or
purely-network scope is not silently forced into an unusable state. The security-relevant guarantee of
US2 AS-2 — an explicitly `read`-marked tree blocks writes — is enforced **unconditionally** (step 3),
independent of the latch. Reads are allow-by-default by the same operational necessity (a process
cannot even load libc under read-deny-by-default); denies remain blanket.

**Fail-closed**: segment (`**/name`) and bounded-star grants are as unenforceable in-kernel as their
deny counterparts, so `create_scope` now refuses them for **any** mode rather than silently dropping a
grant (a dropped grant would under-serve writes under the latch and confuse the author).

**Alternatives**: (a) pure global write-deny-by-default — rejected as unshippable without a fail-open-
prone device allowlist; (b) blanket-deny (any matching deny wins regardless of specificity) — rejected
as the more surprising model and, more importantly, it needs a second in-kernel pass that overran the
verifier budget; the load-time sort gives most-specific-wins in a single early-returning scan, and
because postfix (`*.ext`) rules sort *above* subtree rules a blanket `*.secret` deny still wins over a
broader writable subtree. (c) in-kernel specificity ranking — rejected: overruns the verifier budget.

---

## R14. Backend-neutral compilation + truthful eBPF planning

**Context**: The former compiled `PolicySet` was described as map-ready, but it could contain segment
or bounded-star rules that `create_scope` refused, inode-pinning intent that map loading ignored, and
enabled exfiltration metadata with no enforcing implementation. Consequently `bee validate` could
succeed for a policy that was not runnable by the current backend.

**Decision**: `bee-core` compilation produces a backend-neutral `CompiledPolicy`: resolved author
intent, not a claim of installability. `bee-userspace::EnforcementPlan::prepare` is a pure second stage
that owns the current eBPF backend's supported kinds, encoded lengths, fixed capacities, precedence,
flags, and map-ready entries. Scope installation accepts only a completed plan. All planning happens
before Engine initialization or cgroup creation.

The planner fails closed on segment/bounded-star filesystem rules, inode-pinned executables, enabled
exfiltration detection, overlong encoded rules, and capacity overflow. Disabled exfiltration metadata
may remain in the compiled Policy because it requests no behavior. `bee validate` runs attenuation
(when requested), compilation, and planning, so exit 0 now means runnable by the current eBPF backend.

**Rationale**: Backend-neutral compilation preserves the runtime-free core and leaves room for future
adapters with different capability sets. A separate deep planning module concentrates backend truth
and makes capability checks host-testable. Rejecting unsupported intent prevents silent weakening.

**Alternatives**: (a) make core compilation eBPF-specific — rejected because it couples the portable
domain module to one adapter; (b) keep rejecting inside Scope creation — rejected because validation
remains misleading and failures occur after kernel setup begins; (c) silently drop future capabilities
— rejected by fail-closed enforcement.

---

## R15. Attenuation: silence inherits, it does not reset (FR-005, SC-002)

**Context**: `derive` validated only the rules a child *stated*, and returned the request verbatim.
Every downstream layer, though, reads absence as "nothing to enforce": an empty `[policy.network]`
leaves `FLAG_NET_ENFORCED` clear (egress unrestricted), an empty `[policy.exec]` installs no
`EXEC_ALLOW` entries, a policy with no `write` grant never sets `FLAG_FS_WRITE_DEFAULT_DENY`, and a
dropped `!` inode pin degrades to path matching. Intentional for a root policy the operator authored;
fatal for a derived one, where a child could widen its authority purely by omission. Separately, the
FR-008 protected defaults (`.git`, `.bee`, `~/.ssh`, `~/.aws`) are injected during *compilation* —
after attenuation — and a more-specific rule out-ranks them at load, so a child under a broad parent
grant could name `~/.ssh` precisely and out-rank the protection.

**Decision**: A dimension the child does not mention is **inherited** from the parent, and a
restriction the parent placed inside a region the child re-grants is **re-added** if the child dropped
it. `derive` therefore returns the *effective* child policy, not the request:

- empty child `filesystem` / `exec.allow` / `network.allow` → the parent's map/list is copied in;
- a non-empty child filesystem map re-absorbs every parent `deny`;
- a child exec entry whose parent counterpart is inode-pinned (`!cargo`) is re-pinned.

Attenuation additionally refuses any child grant landing inside an FR-008 protected region unless the
parent named a region containing that path explicitly, so compile-time injection stays trustworthy for
derived policies.

**Rationale**: Inheritance is trivially ⊆ the parent — every inherited rule *is* a parent rule — so the
subset property is preserved by construction, and the fix lives entirely in the core validator where
the property tests already run. It also keeps a child usable: a subagent that only wants to narrow the
filesystem does not silently lose its toolchain.

**Alternatives**: (a) treat an empty child list as deny-all — rejected: the backend cannot express
"enforced but zero destinations" today (`plan.rs` derives the enforcement flags from rule presence), so
it would require a backend change to mean anything, and it makes exec unusable for a partial child;
(b) reject a child that omits a dimension the parent constrains — rejected as hostile to the common
case of narrowing one dimension; (c) fix it downstream in `plan.rs` — rejected because the subset
property belongs to attenuation, and every future backend would have to re-implement it.

---

## Resolved Technical Context values

| Field | Value |
|-------|-------|
| Language/Version | Rust — stable (user space), nightly (only `bee-ebpf`) |
| Primary Dependencies | aya ≈0.14, aya-ebpf ≈0.2.1, aya-build ≈0.1.2, bpf-linker (LLVM 21); serde/serde_json; toml; proptest, criterion; nix/libc; clap (CLI); ctor (`bee-hardening`) |
| Storage | Policy = TOML files (source of truth); runtime state = BPF maps (LPM trie, hash, ringbuf) |
| Testing | cargo test (unit), proptest (attenuation), privileged VM integration tests, criterion benches |
| Target Platform | Linux ≥ 5.7, `CONFIG_BPF_LSM=y` + `bpf` in active LSM list, cgroup v2 |
| Project Type | Rust workspace: multi-crate library + thin CLI |
| Performance Goals | ≤ 5µs / hooked syscall; policy compile < 50ms (≤100 path + 50 net rules) |
| Constraints | No async runtime in core; user space stable Rust; fail-closed everywhere |
| Scale/Scope | MVP: US1 + US2 (single-call + subagent attenuation); exfil (US3) deferred |

---

## Sources

Aya & kernel LSM: aya-rs.dev/book/programs/lsm, aya-rs.dev/book/start/development,
docs.kernel.org/bpf/prog_lsm.html, docs.rs/aya (0.14) maps, docs.ebpf.io BPF_PROG_TYPE_LSM,
docs.ebpf.io bpf_get_current_cgroup_id, torvalds/linux commit bf6fa2c (cgroup id via f_handle),
lwn.net/Articles/917290 (LSM return-value checking). ·
Path/glob prior art: docs.ebpf.io bpf_d_path + the bpf-next allowlist patch (security_file_open,
security_bprm_check), docs.kernel.org/bpf/map_lpm_trie.html, Cloudflare LPM-trie performance (2025),
KubeArmor Discussion #448 (glob impracticality + inode fallback), Tetragon selectors (prefix 256 /
postfix 128), Tetragon filename-access (hard-link/bind-mount caveat), bpfbox (CCSW 2020) &
bpfcontain-rs (inode+dev design), eBPF-LSM TOCTOU analyses.
