# Implementation Plan: bee — eBPF-Enforced Sandbox Harness for Coding Agents

**Branch**: `001-ebpf-agent-sandbox` | **Date**: 2026-07-19 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `specs/001-ebpf-agent-sandbox/spec.md`

## Summary

bee is a Rust workspace providing kernel-enforced, capability-scoped sandboxing for AI coding agent
tool calls. The MVP delivers deny-by-default file/exec/network enforcement (US1) and provable
per-subagent policy attenuation (US2) using eBPF **LSM** programs (Aya, `BPF_LSM_MAC`) attached to
`file_open`, `bprm_check_security`, and `socket_connect`, scoped per **cgroup v2** group via
`bpf_get_current_cgroup_id()`. Policies are declarative TOML compiled into BPF maps; paths are resolved
in-kernel with `bpf_d_path()`; globs are lowered in user space to verifier-safe primitives (prefix /
postfix / segment / bounded-`*`), with irreducible patterns rejected fail-closed. The MVP is
eBPF-LSM-only and refuses to start on unsupported kernels. Exfiltration detection (US3) and the
Landlock+seccomp degraded fallback are deferred. See [research.md](./research.md) for the grounded
technical decisions.

## Technical Context

**Language/Version**: Rust — **stable** for all user-space crates; **nightly** only for `bee-ebpf`
(BPF target `bpfel-unknown-none`, `-Z build-std=core`, pinned via `rust-toolchain.toml`).

**Primary Dependencies**: `aya` ≈0.14 (loader/maps), `aya-ebpf` ≈0.2.1 (probes), `aya-build` ≈0.1.2
(build script), `bpf-linker` (LLVM-21-matched); `serde`/`serde_json`, `toml`; `nix`/`libc` (cgroup,
prctl, name_to_handle_at); `clap` (CLI); `ctor` (`bee-hardening`); `proptest`, `criterion` (tests/benches).

**Storage**: Policy = TOML files (source of truth). Runtime state = BPF maps (LPM trie, hash, ring
buffer). No database.

**Testing**: `cargo test` unit tests (stable, no kernel); `proptest` for attenuation; **privileged
integration tests on a real BPF_LSM kernel** in QEMU/`vmtest`; `criterion` + `bpf_ktime_get_ns` benches.

**Target Platform**: Linux ≥ 5.7 with `CONFIG_BPF_LSM=y`, `bpf` in the active LSM list, BTF, cgroup v2.

**Project Type**: Rust workspace — multi-crate library + thin CLI.

**Performance Goals**: ≤ 5µs per hooked syscall (NFR-001/SC-003); `Policy::compile` < 50ms for ≤100
path + ≤50 net rules (NFR-002/SC-006).

**Constraints**: No async runtime in the core policy engine (NFR-005); user-space crates on stable Rust
(NFR-003); fail-closed everywhere (constitution Principle I).

**Scale/Scope**: MVP = User Stories 1 & 2. Exfiltration (US3) and degraded mode deferred post-MVP.

## Constitution Check

*GATE: evaluated before Phase 0 and re-checked after Phase 1 design.*

| # | Principle | Design compliance | Status |
|---|-----------|-------------------|--------|
| I | Deny-by-Default & Fail-Closed | Empty policy denies all; unsupported kernel ⇒ `Engine::init` errors (R3); unprovable glob subset ⇒ reject (R6); unresolvable exec/host ⇒ compile error | ✅ PASS |
| II | Capability Attenuation | `derive()` proves child ⊆ parent; conservative fail-closed for globs; property tests (SC-002) | ✅ PASS |
| III | Kernel Enforcement Authoritative | All allow/deny decisions in LSM programs; user space only compiles/loads/observes; capabilities not kernel-enforceable are not presented as enforced (exfil deferred, not faked) | ✅ PASS |
| IV | Policy-as-Data | TOML is the source of truth; kernel maps are derived only, never authored; `bee validate` reviews policy without running it | ✅ PASS |
| V | Library-First, Runtime-Free Core | `bee-core` has no async runtime (NFR-005); CLI wraps library entry points and adds no enforcement; each crate single-purpose | ✅ PASS |

**Security & Platform constraints**: eBPF-LSM-only MVP, bee-owned cgroups, refuse privileged targets
(FR-015), pre-exec hardening (FR-011), protected default paths (FR-008), audit every decision — all
reflected in the design. **Quality gates**: test-first for the security boundary, property tests for
attenuation, real-kernel integration tests, perf budgets as benches — all in the plan (R12).

**Result**: PASS, no violations. Complexity Tracking is empty. One item is *tracked, not a violation*:
in-kernel glob support is deliberately reduced to lowered primitives (R6) rather than a general engine —
this *reduces* complexity and upholds Principle I, so it needs no justification entry.

## Project Structure

### Documentation (this feature)

```text
specs/001-ebpf-agent-sandbox/
├── plan.md              # This file
├── spec.md              # Feature spec (clarified)
├── research.md          # Phase 0 — technical decisions (R1–R12)
├── data-model.md        # Phase 1 — authoring + runtime entities
├── quickstart.md        # Phase 1 — validation scenarios
├── contracts/           # Phase 1 — API/CLI/policy/audit contracts
│   ├── library-api.md
│   ├── cli.md
│   ├── policy.schema.md
│   └── audit-event.schema.json
└── tasks.md             # Phase 2 — created by /speckit-tasks (NOT here)
```

### Source Code (repository root)

```text
Cargo.toml                   # workspace root
rust-toolchain.toml          # pins stable + nightly + components
bee-core/                    # policy types, TOML parse, compiler/lowering, attenuation, audit types
├── src/
│   ├── lib.rs
│   ├── policy.rs            # Policy, FsRule, PathPattern, Exec/Net/Exfil, TOML deser
│   ├── compiler.rs          # Policy -> PolicySet (glob lowering, exec name resolution, DNS)
│   ├── attenuation.rs       # derive() subset validation (fail-closed on globs)
│   ├── scope.rs             # Scope/ScopeId/ScopeMode types (cgroup-agnostic core)
│   └── audit.rs             # AuditEvent (serde), record <-> event mapping
bee-ebpf/                    # eBPF LSM programs (nightly, aya-ebpf)
├── src/
│   ├── main.rs              # program entry points, shared maps
│   ├── file.rs              # file_open hook: bpf_d_path + match (exact/prefix/pattern/inode)
│   ├── exec.rs              # bprm_check_security hook
│   ├── net.rs               # socket_connect hook
│   ├── matcher.rs           # bounded in-kernel primitives (prefix/postfix/segment/bounded-*)
│   └── maps.rs              # #[repr(C)] key/value structs shared with userspace
bee-common/                  # #[repr(C)] map layouts shared by ebpf + userspace (Pod types)
bee-userspace/               # Aya loader, scope lifecycle, audit consumer
├── build.rs                 # aya-build compiles bee-ebpf
├── src/
│   ├── lib.rs               # Engine, EngineConfig, Support
│   ├── detect.rs            # R3 fail-closed support detection
│   ├── loader.rs            # load + attach LSM programs
│   ├── cgroup.rs            # create/teardown cgroup, name_to_handle_at -> cgroup_id
│   ├── maps.rs              # populate/update maps from PolicySet
│   ├── spawn.rs             # fork + hardening + move-into-cgroup + exec
│   └── events.rs            # ringbuf -> AuditReader (sync iterator)
bee-hardening/               # optional #[ctor] pre-main hardening for embedded-in-agent use
└── src/lib.rs
bee-cli/                     # thin CLI wrapper (clap)
└── src/main.rs              # run / check / validate

tests/                       # workspace integration tests (privileged, VM-gated)
├── integration/             # US1/US2 scenarios (quickstart)
└── property/                # attenuation property tests
```

**Structure Decision**: Rust Cargo **workspace**. Crate split follows the constitution's library-first
principle and the crate boundaries in the goals doc, with one addition — **`bee-common`** holds the
`#[repr(C)]` map layouts shared between `bee-ebpf` (nightly) and `bee-userspace` (stable), so the shared
POD structs live in a stable crate both sides depend on rather than being duplicated. `bee-core` stays
free of any eBPF/async dependency (Principle V). The CLI (`bee-cli`) depends only on the public library
API.

## Complexity Tracking

> No constitution violations. Table intentionally empty.

| Violation | Why Needed | Simpler Alternative Rejected Because |
|-----------|------------|-------------------------------------|
| — | — | — |
