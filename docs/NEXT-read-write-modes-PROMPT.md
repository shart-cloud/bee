# Task: enforce read/write/deny modes in bee's `file_open` hook

You are continuing work on **bee**, a Rust + eBPF-LSM sandbox for AI coding agents, at
`/home/jg/git/bee`. Read this whole prompt before touching code. The core is done and tested; your job
is one focused increment on the kernel filesystem hook.

## What bee already does (don't rebuild it)

- 6-crate Cargo workspace. `bee-core` (policy/compiler/attenuation, 47 host tests), `bee-common`
  (`#[repr(C)]` map layouts + matcher primitives, `no_std`), `bee-ebpf` (LSM programs, **excluded from
  the workspace**), `bee-userspace` (aya loader/maps/spawn), `bee-cli`, `bee-hardening`.
- Three LSM hooks enforce on a real kernel today: `file_open` (path deny — subtree + postfix `*.ext`),
  `socket_connect` (address allowlist), `bprm_check_security` (exec allowlist). Per-cgroup scoping,
  observe/dry-run mode, JSON audit over a ring buffer, subagent attenuation (`bee run --parent`).
- Full design + status: `specs/001-ebpf-agent-sandbox/` (spec.md, plan.md, research.md, tasks.md —
  45/53 done). Read `plan.md` and `research.md` (esp. R5/R6) first.

## The gap you're closing

Filesystem enforcement is currently **deny-list only**. A policy rule's *mode* (`read`/`write`/`deny`)
is not fully honored: `deny` blocks, but a `read`-marked path does **not block writes** to it. So
US2's AS-2 ("subagent gets read-only project source; writes are denied") is not yet enforced in-kernel.
Your task: make the `file_open` hook enforce read-only — **block write-opens of paths marked `read`**,
while allowing reads. This completes FR-001 and US2 AS-2.

### The mechanism
- `file_open`'s hook argument is `struct file *`. The requested access is in `file->f_mode`
  (`FMODE_WRITE` bit = `0x2`). You need `offsetof(struct file, f_mode)` — get it from the target
  kernel's BTF: `pahole -C file | grep f_mode` (run it in the test VM, see below). Add it as a
  compile-time `const` next to `FILE_F_PATH_OFF = 152` in `bee-ebpf/src/main.rs`, and read it with a
  **direct load** (`*((file as usize + off) as *const <int type>)`) — NOT `bpf_probe_read` (that
  yields an untyped scalar; see gotchas). Mask/So bound any derived index.
- Extend the per-scope rule model so the kernel knows, for each path rule, its **mode**. Today
  `FS_DENY` (`DenyList` of `DenyRule{kind,len,bytes}` in `bee-common/src/layout.rs`) carries deny
  prefixes only. Add a `mode` byte to `DenyRule` (or a parallel "read-only rules" list), populated by
  `Engine::create_scope` in `bee-userspace/src/lib.rs` from the compiled `PolicySet.fs`
  (`FsPrimitive::Prefix { mode, .. }` — use `bee_common::AccessMode::is_deny()` / check for read-only).
- In `file_open`: after resolving the path (via `bpf_d_path`), find the **most-specific** matching
  rule and apply: `deny` → block; `read` → block iff the open requests write; `write` → allow.

### The design decision you MUST make explicitly (and document)
What is the default when **no rule matches**? The constitution says deny-by-default, but pure
deny-by-default on reads breaks every toolchain (can't open libc, etc.). The pragmatic, defensible
model (used by real agent sandboxes) is:
- **Writes**: deny-by-default — allowed only where a `write` rule grants it.
- **Reads**: allow-by-default — allowed unless a `deny` rule matches.

Decide this deliberately, implement it, and write it down in `research.md` (a new decision entry) and
the policy schema contract. If you diverge from the above, justify it against the constitution
(`.specify/memory/constitution.md`, Principle I). Do not silently pick a default.

## Critical gotchas (learned the hard way — do not rediscover)

1. **Verifier instruction budget (1M).** Do NOT scan the 4 KB path buffer for a NUL to get its length
   — it explodes the budget. `bpf_d_path` **returns** the length (bytes incl. NUL); use that
   (`resolved_len()` already exists).
2. **Unbounded memory access.** Any buffer index derived from a runtime value must be masked
   `& (PATH_MAX - 1)` (PATH_MAX = 4096, a power of two) or the verifier rejects it.
3. **Struct field reads need constant offsets + direct loads.** The verifier requires *constant*
   offsets into a trusted BTF pointer; `bpf_probe_read` returns an untyped scalar that `bpf_d_path`
   won't accept. Offsets are hardcoded per-kernel — **CO-RE auto-relocation is not achievable** with
   aya-ebpf 0.2.1 (bindgen emits no `preserve_access_index`); see the `bee-ebpf` module docs.
4. **Fail closed, always.** If a rule kind/mode can't be enforced in-kernel, make `create_scope`
   **refuse** (return `ScopeError`) rather than silently under-enforce — matches the existing
   segment/bounded-star handling.
5. `pre_exec` (post-fork) code must be async-signal-safe: raw syscalls only, no allocation/locks.

## Build + test loop (host builds, VM runs)

The eBPF only enforces on a real BPF-LSM kernel. There's a KubeVirt VM for this — see the memory file
`~/.claude/projects/-home-jg-git-bee/memory/bee-bpf-lsm-test-vm.md` for full access details
(`virtctl ssh ubuntu@vmi/ac-matrix-vm/ac-matrix -i ~/.ssh/ac-matrix-vm`). The host has the nightly bpf
toolchain and the same glibc as the VM, so:

```bash
# iterate the eBPF quickly (catches verifier-facing compile errors, not verifier itself):
cd bee-ebpf && RUSTFLAGS="-C link-arg=--btf" cargo +nightly build \
  --target bpfel-unknown-none -Z build-std=core --release

cargo build -p bee-cli --features enforce --release   # builds+embeds the eBPF
cargo test --workspace --tests                        # default host tests must stay green
cargo clippy --workspace                              # and --features enforce; keep both clean

bash test/vm/matrix.sh          # host: build → ship → run the 17-case matrix, gated exit
BEE_SKIP_BUILD=1 bash test/vm/matrix.sh   # reuse target/release/bee
```

Run the harness in the background (it makes many VM round-trips) and wait for it — don't foreground it.
If the eBPF fails to load, capture the verifier log: `sudo bee run --policy <p> -- true 2>&1 | tail -20`.

## Definition of done

- [ ] `file_open` enforces read-only: writing to a `read`-marked subtree is denied (`EACCES`), reading
      it succeeds, and the default-access decision is implemented as chosen.
- [ ] `create_scope` fails closed on any file mode/kind it can't enforce.
- [ ] New harness cases in `test/vm/remote-matrix.sh` prove it: e.g. a policy granting `write` to a
      scratch dir and `read` to a source dir — writing the source file is blocked, reading it works,
      writing scratch works. Update the case count in `test/vm/README.md`.
- [ ] Default `cargo test`/clippy green in both build modes; harness all-green on the VM.
- [ ] The default-access decision recorded in `research.md` + policy schema contract.
- [ ] Mark the relevant task(s) `[X]` in `specs/001-ebpf-agent-sandbox/tasks.md`; update the memory
      file with any new verifier lessons.

## Other open items (not this task, for context)
Segment (`**/name`) / bounded-star globs via `bpf_loop`; the boot-time **offset fail-closed guard**
(read running kernel BTF, refuse if compiled offsets don't match — closes the CO-RE fail-open);
5µs perf benchmark (SC-003); GitHub CI wiring the harness (T006).
