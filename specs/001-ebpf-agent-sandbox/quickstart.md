# Quickstart & Validation Guide: bee

**Feature**: 001-ebpf-agent-sandbox

Runnable scenarios that prove the MVP works end-to-end. These map to the spec's Independent Tests and
Success Criteria. Implementation details live in `tasks.md`; this is a run/validate guide.

## Prerequisites

- Linux kernel ≥ 5.7 with `CONFIG_BPF_LSM=y`, `CONFIG_DEBUG_INFO_BTF=y`, and **`bpf` in the active LSM
  list** — verify: `cat /sys/kernel/security/lsm` contains `bpf`. If not, add `lsm=...,bpf` to the kernel
  cmdline and reboot (integration CI uses a QEMU image already configured this way).
- cgroup v2 mounted at `/sys/fs/cgroup` (unified hierarchy).
- Rust stable + nightly (`rustup toolchain install nightly --component rust-src`), `bpf-linker`
  (LLVM-matched), pinned via `rust-toolchain.toml`.
- Run bee as root or with `CAP_BPF + CAP_PERFMON (+ CAP_MAC_ADMIN)` — see `research.md` R11.

## Build

```bash
cargo build --release          # aya-build compiles bee-ebpf via build.rs
sudo ./target/release/bee check   # prints support gates; exits non-zero if unsupported (fail-closed)
```

Expected `bee check`: all gates PASS (kernel, `bpf` LSM active, cgroup v2, BTF). On an unconfigured
kernel it prints which gate failed and exits 65 — **never** runs unprotected (SC-007).

---

## Scenario 1 — Single-agent tool call sandbox (US1 / SC-001)

`policies/cargo-test.toml` — see `contracts/policy.schema.md` for the full example (write to project,
exec `cargo`/`rustc`/`cc`, network to crates.io, deny `~/.ssh` + `~/.aws`).

```bash
sudo bee run --policy policies/cargo-test.toml -- cargo test
```

Validate (all four are automated integration tests):
1. **Denied secret read**: inside the sandbox, `cat ~/.ssh/id_rsa` fails with `EACCES`; an audit event
   `{op:file_open, decision:denied, target:.../.ssh/id_rsa}` appears on the audit stream. (SC-001)
2. **Denied network**: `curl https://93.184.216.34` (not in allowlist) fails with `EPERM`. (AS-2)
3. **Denied exec**: `curl`/`nc` (not allowlisted) fails to `execve` with `EACCES`. (AS-3)
4. **Allowed ops succeed**: `cargo test` on a medium project completes with **no sandbox-caused
   failures** — writes under project, execs `cargo`/`rustc`, reaches crates.io. (SC-004)

## Scenario 2 — Subagent attenuation (US2 / SC-002)

```bash
# Parent: broad project access. Child: read-only src + rw scratch, exec test runner only, no network.
sudo bee validate --policy policies/subagent.toml --parent policies/parent.toml   # exits 0 (⊆ parent)
sudo bee validate --policy policies/overbroad.toml --parent policies/parent.toml  # exits 64: attenuation violation
```

Validate:
1. **Over-grant rejected**: a child adding write to `/etc/passwd` (parent can't) is rejected *before*
   enforcement. (AS-1)
2. **Read-only enforced**: child writing to project `src/main.rs` is denied. (AS-2)
3. **No-network enforced**: child `connect()` denied regardless of destination. (AS-3)
4. **Property test**: `derive(parent, child).is_ok() ⇒ child ⊆ parent`, over generated policies. (SC-002)

## Scenario 3 — Audit-only / dry-run (FR-016)

```bash
sudo bee run --policy policies/cargo-test.toml --mode observe -- cargo test
```
Validate: operations that *would* be denied still succeed, but each emits an audit event with
`decision: "observed"`. Enables authoring a policy against a real workload before switching to `enforce`.

## Scenario 4 — Live policy update (FR-006)

Validate that refreshing a network domain's resolved IPs updates `NET_ALLOW` map entries **without**
detaching programs or restarting the sandboxed process (integration test drives `Scope::update`).

---

## Performance validation (gates)

- **SC-003 / NFR-001**: `bench_file_open` runs ≥ 10,000 `open()` calls under an enforcing policy with
  `bpf_ktime_get_ns` instrumentation; assert median hook overhead ≤ 5µs.
- **SC-006 / NFR-002**: criterion bench asserts `Policy::compile` < 50ms for 100 path + 50 net rules.

## Out of scope for this guide
Exfiltration detection (US3, deferred to P3); degraded Landlock+seccomp mode (deferred). Behavior when
bee runs inside an existing container (OQ-005) is documented but not enforced.
