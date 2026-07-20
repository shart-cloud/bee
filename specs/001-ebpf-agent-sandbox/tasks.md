---
description: "Task list for bee — eBPF-Enforced Sandbox Harness"
---

# Tasks: bee — eBPF-Enforced Sandbox Harness for Coding Agents

**Input**: Design documents from `/specs/001-ebpf-agent-sandbox/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/

**Tests**: INCLUDED. The constitution mandates test-first for the security boundary (allow/deny
outcomes, attenuation rejection, fail-closed paths) and **property-based** tests for the attenuation
validator, verified by integration tests on a real BPF_LSM kernel. Test tasks precede implementation
within each story.

**Organization**: Grouped by user story. MVP = User Story 1 (P1) + User Story 2 (P2). User Story 3
(exfiltration detection, P3) is **out of MVP scope** (deferred to a second release) — no phase here.

**Path conventions**: Rust Cargo workspace at repo root; crate layout per plan.md.

---

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Workspace, pinned toolchain, and the privileged test harness.

- [X] T001 Create Cargo workspace root `Cargo.toml` with members: `bee-common`, `bee-core`, `bee-ebpf`, `bee-userspace`, `bee-hardening`, `bee-cli`
- [X] T002 [P] Add `rust-toolchain.toml` pinning stable + nightly (with `rust-src`), and document the nightly ↔ `bpf-linker` ↔ LLVM-21 pin in `README.md` (top build-break risk per research R1)
- [X] T003 [P] Scaffold empty crates with `Cargo.toml` + `src/lib.rs`/`main.rs` for all six workspace members per plan.md structure
- [X] T004 [P] Configure `rustfmt.toml`, `clippy` lints (deny warnings in user-space crates), and `.gitignore`
- [X] T005 Create a privileged integration-test harness `tests/vm/` (QEMU/`vmtest` image with kernel ≥5.7, `CONFIG_BPF_LSM=y`, `lsm=...,bpf`, cgroup v2) and a `make test-integration` entrypoint that runs with `CAP_SYS_ADMIN`
- [ ] T006 [P] Add CI workflow that runs stable unit/property tests on the host and integration/bench tests inside the T005 VM

**Checkpoint**: `cargo build` succeeds; empty VM harness boots and reports kernel LSM support.

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Shared POD map layouts, policy types, fail-closed detection, and the load/attach/spawn/
audit plumbing that BOTH user stories build on.

**⚠️ CRITICAL**: No user story work begins until this phase is complete.

### Shared map layouts & core types

- [X] T007 [P] Define `#[repr(C)]` + `aya::Pod` map layout structs in `bee-common/src/lib.rs`: `ScopeMeta{mode,flags}`, `FsPrefixKey{prefixlen,cgroup_id,path[256]}`, `PatternRule{kind,bytes[128],mode}`, `InodeKey{cgroup_id,dev,ino}`, `NetKey{cgroup_id,family,addr[16],port}`, `AuditRecord{ts_ns,cgroup_id,pid,tgid,op,decision,errno,target[256]}`, plus `AccessMode` bitflags and `Op`/`Decision` enums (identical on both eBPF and user-space sides)
- [X] T008 [P] Implement authoring policy types in `bee-core/src/policy.rs`: `Policy`, `FsRule`, `PathPattern` (Exact/Subtree/Postfix/Segment/BoundedStar), `AccessMode`, `ExecPolicy`/`ExecRule`, `NetPolicy`/`NetRule`/`HostSpec`, `ExfilPolicy`, `ScopeMode`
- [X] T009 Implement TOML deserialization + structural validation in `bee-core/src/policy.rs` (`Policy::from_toml`, `Policy::from_path`), including protected-default injection for `.git`/`.bee`/`~/.ssh`/`~/.aws` (FR-008) and path-token (`:project_root`, `~`) placeholders
- [X] T010 [P] Implement `AuditEvent` (serde) and `AuditRecord`→`AuditEvent` mapping (monotonic ts → ISO-8601) in `bee-core/src/audit.rs`, matching `contracts/audit-event.schema.json`
- [X] T011 [P] Implement `ScopeId`, `ScopeMode`, and cgroup-agnostic `Scope` core types in `bee-core/src/scope.rs`

### Fail-closed detection & eBPF toolchain

- [X] T012 [P] Unit tests for support detection in `bee-userspace/tests/detect.rs`: kernel-version parse, `/sys/kernel/security/lsm` token check (present/absent), cgroup v2 presence, BTF availability (fixtures) — **must FAIL first**
- [X] T013 Implement fail-closed support detection in `bee-userspace/src/detect.rs` (`Engine::supported`, `Support`) per research R3: kernel ≥5.7 → `bpf` in active LSM list → cgroup v2 → BTF; return structured per-gate diagnostics
- [X] T013a **Boot-time offset fail-closed guard** (R3 gate 4, closes the CO-RE fail-open in R5): `bee-userspace/src/kbtf.rs` parses `/sys/kernel/btf/vmlinux` (dependency-free BTF reader; aya exposes no public member-offset API) to resolve `file.f_mode`/`file.f_path`/`linux_binprm.file` and refuses to run if any disagrees with `bee_common::offsets` (the shared compiled-offset source of truth). Wired as the `offsets_ok` gate in `Support`/`is_supported()` so both `bee check` and `bee run` fail closed. Host-tested with synthetic BTF; verified `PASS` on the 6.8 VM
- [X] T014 Set up `bee-ebpf` crate: `main.rs` entry module, `maps.rs` re-exporting `bee-common` layouts, and a minimal no-op LSM program that loads; wire `bee-userspace/build.rs` to compile it via `aya-build`
- [X] T015 Implement LSM program load + attach scaffold in `bee-userspace/src/loader.rs` (`Btf::from_sys_fs`, `Lsm::load(hook,&btf)`, `attach()`); returns links held for the engine lifetime. Return-value convention `0=allow / -errno=deny` documented; assert NOT using `BPF_LSM_CGROUP` semantics

### Scope lifecycle, spawn, audit transport

- [X] T016 [P] Implement cgroup v2 lifecycle in `bee-userspace/src/cgroup.rs`: create `<parent>/bee/<scope_id>`, obtain `cgroup_id` via `name_to_handle_at` (cross-check `statx` `st_ino`), teardown/rmdir (research R4)
- [X] T017 [P] Implement `bee-hardening/src/lib.rs`: reusable `apply_hardening()` (prctl `PR_SET_DUMPABLE=0`, `setrlimit(RLIMIT_CORE,0)`, strip `LD_*`) plus an optional `#[ctor]` entry for embedded-in-agent use (FR-011)
- [X] T018 Implement spawn-into-scope in `bee-userspace/src/spawn.rs`: fork, call `apply_hardening()` in child pre-exec, move child into the scope cgroup (`CLONE_INTO_CGROUP` or `cgroup.procs`), `execve`; **refuse privileged targets** (setuid/broad caps) → `PrivilegedTarget` (FR-015)
- [X] T019 [P] Implement the ring-buffer audit consumer in `bee-userspace/src/events.rs`: `AuditReader` as a synchronous, blocking `Iterator<Item = AuditEvent>` over `AUDIT_RB` (no async runtime — NFR-005, research R9)
- [X] T020 Assemble `Engine`, `EngineConfig`, `Engine::init` (fail-closed: errors on unsupported kernel), and `SCOPES` map population in `bee-userspace/src/lib.rs`; wire `bee-cli` `bee check` command (contracts/cli.md) to `Engine::supported`

**Checkpoint**: On a supported kernel `Engine::init` attaches the no-op program and `bee check` passes;
on an unsupported kernel both fail closed with a clear diagnostic (SC-007). Foundation ready.

---

## Phase 3: User Story 1 — Single-Agent Tool Call Sandbox (Priority: P1) 🎯 MVP

**Goal**: Enforce deny-by-default file/exec/network policy on a single sandboxed command; denied
operations return EACCES/EPERM and emit audit events; allowed operations succeed.

**Independent Test**: Run a command under `bee run --policy <restrictive>`; verify `~/.ssh/id_rsa` read,
un-allowlisted `execve`, and un-allowlisted `connect()` are denied with audit events, while permitted
writes/execs/connects succeed (quickstart Scenario 1; SC-001, SC-004).

### Tests for User Story 1 ⚠️ (write first, ensure they FAIL)

- [X] T021 [P] [US1] Unit tests for glob lowering in `bee-core/tests/compiler.rs`: `*.log`→Postfix, `**/target`→Segment, single-`*`→BoundedStar, exact/subtree; irreducible mid-path `**`→`CompileError::UnsupportedGlob` (fail-closed, research R6)
- [X] T022 [P] [US1] Unit tests for exec-name resolution and domain→IP compilation in `bee-core/tests/compiler.rs` (injected PATH + resolver; unresolvable name/host → compile error)
- [X] T023 [P] [US1] Unit tests for the in-kernel matcher primitives (prefix/postfix/segment/bounded-star, specificity/deny-tie) in `bee-ebpf/tests/matcher.rs` (host-compilable pure fns)
- [X] T024 [P] [US1] Integration test: denied secret read → EACCES + audit event, in `tests/integration/us1_file_deny.rs` (SC-001)
- [X] T025 [P] [US1] Integration test: denied network connect → EPERM, in `tests/integration/us1_net_deny.rs`
- [X] T026 [P] [US1] Integration test: denied `execve` of un-allowlisted binary → EACCES, in `tests/integration/us1_exec_deny.rs`
- [X] T027 [P] [US1] Integration test: allowed operations succeed with no sandbox-caused failures (write in project, exec `cargo`, connect allowed host), in `tests/integration/us1_allow.rs` (SC-004)
- [X] T028 [P] [US1] Integration test: audit-only (`--mode observe`) allows but emits `decision:"observed"` events, in `tests/integration/us1_observe.rs` (FR-016)
- [ ] T029 [P] [US1] Integration test: live policy update (DNS refresh) updates `NET_ALLOW` without detach/restart, in `tests/integration/us1_update.rs` (FR-006)

### Implementation for User Story 1

- [X] T030 [US1] Implement `PolicySet` and `Policy::compile` in `bee-core/src/compiler.rs`: glob lowering to primitives, exec-name→path resolution, domain→IP set, protected-path defaults; error on unsupported glob (depends on T008, T009)
- [X] T031 [P] [US1] Implement the bounded matcher primitives in `bee-ebpf/src/matcher.rs` (prefix/postfix/segment/bounded-star, fixed length caps, verifier-safe; no backtracking)
- [X] T032 [US1] Implement map population from `PolicySet` in `bee-userspace/src/maps.rs`: fill `FS_PREFIX` (LPM, key = `cgroup_id‖path`), `FS_PATTERN`, `FS_INODE`, `EXEC_ALLOW`, `NET_ALLOW`, and `SCOPES` mode flag; live `Scope::update` (depends on T030, T007)
- [X] T033 [US1] Implement the `file_open` LSM hook in `bee-ebpf/src/file.rs`: `bpf_d_path` into per-CPU buffer, filter by `bpf_get_current_cgroup_id`, LPM + pattern + inode lookup, most-specific/deny-tie decision, return `0`/`-EACCES`, emit `AuditRecord`; honor observe mode (research R5/R6/R9)
- [X] T033a [US1][US2] Enforce read/write/deny **modes** in `file_open` (FR-001, US2 AS-2): read `FMODE_WRITE` from `file->f_mode` (direct load, offset 20), carry each rule's `AccessMode` in `DenyRule.mode`, and apply the R13 precedence — most-specific match wins (deny breaks ties) via a **load-time sort** (`rule_sort_key`) + in-kernel first-match early-return (an in-kernel specificity rank overran the verifier budget), then deny-by-default writes latched by `FLAG_FS_WRITE_DEFAULT_DENY`. Reference decision `bee_common::matcher::fs_open_blocked` (host-tested) mirrored by `bee-ebpf::fs_should_block`; `create_scope` fails closed on segment/bounded-star rules of any mode. VM cases `rw-*` in `remote-matrix.sh`
- [X] T034 [P] [US1] Implement the `bprm_check_security` hook in `bee-ebpf/src/exec.rs`: `bpf_d_path` on the binary, allowlist match (path + optional inode), return `0`/`-EACCES`, emit audit (research R7)
- [X] T035 [P] [US1] Implement the `socket_connect` hook in `bee-ebpf/src/net.rs`: extract dest addr/port, `NET_ALLOW` lookup, return `0`/`-EPERM`, emit audit (research R8)
- [X] T036 [US1] Extend `loader.rs` to load+attach all three hooks (replace the T014 no-op) and hold their links
- [X] T037 [US1] Implement `Engine::create_scope` + `Scope::spawn` end-to-end wiring in `bee-userspace/src/lib.rs` (compile → cgroup → populate maps → spawn hardened child); `Scope::teardown`/`Drop`
- [X] T038 [US1] Implement `bee run` in `bee-cli/src/main.rs` per contracts/cli.md (flags, exit codes 0/64/65/66/70, audit JSON to sink)

**Checkpoint**: US1 fully functional and independently testable — the MVP core. T024–T029 pass.

---

## Phase 4: User Story 2 — Subagent Permission Scoping (Priority: P2)

**Goal**: Derive an attenuated child policy that is provably a subset of its parent; reject any
over-grant before enforcement; enforce the attenuated scope on a child process.

**Independent Test**: `bee validate --policy child --parent parent` rejects over-grants (exit 64);
a derived read-only/no-network child scope denies writes to source and all connects; property test
proves `derive(parent,child).is_ok() ⇒ child ⊆ parent` (quickstart Scenario 2; SC-002).

### Tests for User Story 2 ⚠️ (write first, ensure they FAIL)

- [X] T039 [P] [US2] **Property-based** tests (proptest) in `bee-core/tests/attenuation_prop.rs`: over generated parent/child policies, `derive(parent,child).is_ok() ⇒ child ⊆ parent` for FS/exec/net; glob-derived rules that can't be proven contained are rejected (fail-closed) (SC-002)
- [X] T040 [P] [US2] Unit tests for `derive()` in `bee-core/tests/attenuation.rs`: exact/subtree containment, deny-overlap rejection, mode-narrowing only, specific `AttenuationError` reasons
- [X] T041 [P] [US2] Integration test: over-grant child (adds write to `/etc/passwd`) rejected before enforcement, in `tests/integration/us2_reject.rs` (AS-1)
- [X] T042 [P] [US2] Integration test: derived read-only child denied write to project `src/`, in `tests/integration/us2_readonly.rs` (AS-2)
- [X] T043 [P] [US2] Integration test: derived no-network child denied all `connect()`, in `tests/integration/us2_nonet.rs` (AS-3)

### Implementation for User Story 2

- [X] T044 [US2] Implement the attenuation validator `Policy::derive` in `bee-core/src/attenuation.rs`: subset checks for FS (exact/subtree decidable; glob conservative fail-closed), exec, net, and mode; return `AttenuationError{capability,reason}` (depends on T008)
- [ ] T045 [US2] Implement `Scope::derive_scope` in `bee-userspace/src/lib.rs`: derive → compile → new child cgroup + maps, surfacing `AttenuationError` (depends on T044, T037)
- [X] T046 [US2] Implement `bee validate` in `bee-cli/src/main.rs` (`--policy` compile-check; `--policy/--parent` attenuation check), exit 0/64 with specific messages (contracts/cli.md)

**Checkpoint**: US1 and US2 both work independently; attenuation is provable (property tests green).

---

## Phase 5: Polish & Cross-Cutting Concerns

**Purpose**: Performance gates, docs, and end-to-end validation.

- [ ] T047 [P] Benchmark `bench_file_open` (criterion + `bpf_ktime_get_ns`): ≥10,000 `open()` under an enforcing policy, assert median hook overhead ≤ 5µs, in `bee-ebpf/benches/` + `tests/bench/` (NFR-001/SC-003)
- [X] T048 [P] Benchmark `Policy::compile` < 50ms for 100 path + 50 net rules, in `bee-core/benches/compile.rs` (NFR-002/SC-006)
- [X] T049 [P] Author example policies `policies/cargo-test.toml`, `parent.toml`, `subagent.toml`, `overbroad.toml` per contracts/policy.schema.md
- [ ] T050 Run full quickstart.md validation (Scenarios 1–4) on the VM harness and record results
- [ ] T051 [P] Document capability requirements (empirically confirm minimal set vs `CAP_SYS_ADMIN`, research R11) and the symlink/hard-link/bind-mount matching caveat (research R5) in `README.md`/`docs/`
- [ ] T052 [P] Document deferred scope (US3 exfiltration, Landlock+seccomp degraded mode, container coexistence OQ-005) and the supported glob surface in `docs/`
- [ ] T053 API docs (`cargo doc`) for the public `bee-core`/`bee-userspace` surface (contracts/library-api.md) and a crate-level usage example

---

## Dependencies & Execution Order

### Phase dependencies

- **Setup (Phase 1)**: no dependencies.
- **Foundational (Phase 2)**: depends on Setup — **BLOCKS US1 and US2**.
- **US1 (Phase 3)**: depends on Foundational. The MVP-critical story.
- **US2 (Phase 4)**: the attenuation validator (T039/T040/T044) depends only on `bee-core` (Foundational) and is independently testable without a kernel; US2 *enforcement* integration tests (T042/T043) reuse US1 hooks, so run them after Phase 3.
- **Polish (Phase 5)**: depends on US1 (+US2 for T050).

### Within each story

- Tests (T021–T029, T039–T043) are written and MUST FAIL before implementation.
- `bee-common` layouts (T007) before any map population/hook code.
- Compiler (T030) before map population (T032) before hooks (T033–T035).
- `derive()` (T044) before `derive_scope` (T045) before `bee validate` (T046).

### Parallel opportunities

- Setup: T002, T003, T004, T006 in parallel.
- Foundational: T007, T008, T010, T011 in parallel; T012 before T013; T016/T017/T019 in parallel.
- US1 tests T021–T029 all [P]. Hooks T034/T035 [P] with each other (different files) after T032.
- US2 tests T039–T043 all [P].
- Polish: T047, T048, T049, T051, T052 in parallel.

---

## Parallel Example: User Story 1 tests

```bash
# After Foundational, launch US1 tests together (all must FAIL first):
Task: "Unit tests for glob lowering in bee-core/tests/compiler.rs"                # T021
Task: "Matcher primitive tests in bee-ebpf/tests/matcher.rs"                      # T023
Task: "Integration: denied secret read → EACCES in tests/integration/us1_file_deny.rs"  # T024
Task: "Integration: denied connect → EPERM in tests/integration/us1_net_deny.rs"        # T025
Task: "Integration: denied execve in tests/integration/us1_exec_deny.rs"                # T026
```

---

## Implementation Strategy

### MVP First (User Story 1)

1. Phase 1 Setup → 2. Phase 2 Foundational (fail-closed init working end-to-end) →
3. Phase 3 US1 → **STOP and VALIDATE** on the VM harness (quickstart Scenario 1) → demo the single-call
   sandbox. This alone is the minimum viable, valuable product (SC-001, SC-004).

### Incremental Delivery

1. Setup + Foundational → foundation ready (fail-closed detection + attach + spawn + audit).
2. US1 → test independently → **MVP**.
3. US2 → attenuation validator (property-proven) + attenuated scopes → test independently.
4. Polish → perf gates (5µs / 50ms), docs, full quickstart.
5. (Post-MVP, not in this task list) US3 exfiltration detection; Landlock+seccomp degraded mode.

### Notes

- [P] = different files, no incomplete dependencies.
- Verify security-boundary tests FAIL before implementing (constitution: an untested denial is not
  enforced).
- Integration/bench tasks require the privileged VM harness (T005) and run with the capabilities in
  research R11.
- Commit after each task or logical group.
