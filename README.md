# bee — eBPF-Enforced Sandbox Harness for Coding Agents

bee is a Rust library and CLI that provides kernel-enforced, capability-scoped sandboxing for AI
coding agents. It uses eBPF **LSM** hooks (via [Aya](https://aya-rs.dev)) to enforce per-tool-call and
per-subagent policy — file access control, process-exec allowlisting, and network egress filtering —
without containers, VMs, or namespace overhead.

See [`specs/001-ebpf-agent-sandbox/`](specs/001-ebpf-agent-sandbox/) for the full spec, plan, research,
data model, and contracts.

## Design principles (constitution)

1. **Deny-by-default & fail-closed** — zero capabilities to start; refuse rather than degrade silently.
2. **Capability attenuation** — a subagent policy is *provably* a subset of its parent.
3. **Kernel enforcement is authoritative** — user space compiles/loads/observes; the kernel decides.
4. **Policy-as-data** — declarative, diffable, reviewable TOML is the source of truth.
5. **Library-first, runtime-free core** — the CLI is a thin wrapper; the core needs no async runtime.

## Workspace

| Crate | Role | Status |
|-------|------|--------|
| `bee-common` | `#[repr(C)]` map layouts + verifier-safe matcher primitives (`no_std`) | ✅ implemented + tested |
| `bee-core` | Policy types, TOML parsing, compiler/glob-lowering, attenuation validator, audit types | ✅ implemented + tested |
| `bee-hardening` | Pre-exec / pre-main process hardening (FR-011) | ✅ implemented + tested |
| `bee-userspace` | Support detection, cgroup lifecycle, hardened launcher, engine gating | ✅ host logic tested; ⏳ eBPF attach behind `--features enforce` |
| `bee-cli` | The application: `bee check` / `validate` / `exec` | ✅ `check` + `validate` work; `exec` fail-closed here |
| `bee-harness` | Agent episodes, batch/concurrent runs, CTF scoring, and REPL | ✅ host-tested; enforcement behind features |
| `bee-ebpf` | LSM programs (`file_open`, `bprm_check_security`, `socket_connect`) | ⏳ requires nightly bpf toolchain + BPF-LSM kernel |

## Build & test

User-space crates build on **stable** Rust:

```bash
cargo build
cargo test          # ~47 unit/integration/property tests (no kernel needed)
cargo clippy --workspace --all-targets
```

Try the CLI (works without a special kernel):

```bash
./target/debug/bee                                                    # help — bare `bee` runs nothing
./target/debug/bee check                                              # kernel support gates
./target/debug/bee validate --policy policies/cargo-test.toml         # prove policy is runnable
./target/debug/bee validate --policy policies/subagent.toml \
                            --parent policies/parent.toml             # attenuation check
```

`bee exec --policy P -- COMMAND` runs a single host command inside a scope. It is a **diagnostic** —
a way to prove a policy enforces with no agent in the picture — and needs a BPF-LSM kernel, so it
fails closed on an ordinary host. The primary journeys are `bee run` (headless) and `bee repl`
(interactive); see [ADR-0002](docs/adr/0002-present-bee-as-one-user-facing-application.md).

### Full-screen TUI (optional)

`bee-repl` runs the classic inline REPL by default. Build with the `tui` feature and pass `--tui` for
a full-screen chat surface with model-owned live panels (008-grid-tui):

```bash
cargo build --release -p bee-harness --features tui --bin bee-repl
./target/release/bee-repl --provider <provider.toml> --tui        # --no-tui forces inline
```

The agent can address a rendered widget to a named side panel with `render_to("metrics", widget)`
(and `render_to_ttl` / `remove_panel` / `clear_panels` to manage them); an untargeted `render(widget)`
still flows inline. `--tui` degrades honestly — piped output, `TERM=dumb`, or a terminal below 40×10
fall back to the inline REPL with a one-line note. The feature is **off by default**, so the standard
build pulls in no terminal backend.

Assistant replies render as **markdown** once the message finishes streaming — headings, emphasis,
lists, links and code, styled through the active theme's semantic roles rather than a second palette.
A skill's instructions render the same way when you `/skill <name>`, and the agent can emit a block
itself with `markdown(source)` in a render script.

Keys: `↑`/`↓` and the mouse wheel scroll the conversation (they move the cursor first when the input
holds a `Shift+Enter` newline), `PgUp`/`PgDn` page it, `gg`/`G` jump to the ends, `Ctrl-P`/`Ctrl-N`
walk input history, `Tab` focus, `p` panel overlay on narrow terminals, `y` yank, `?` help, `q` quit.
Mouse reporting is on so the wheel works, which means drag-to-select is the terminal's `Shift`-click
gesture there.

## Kernel requirements (for enforcement)

The `enforce` feature and the `bee-ebpf` crate require:

* Linux **≥ 5.7** with `CONFIG_BPF_LSM=y` and `CONFIG_DEBUG_INFO_BTF=y`,
* **`bpf` in the active LSM list** — check `cat /sys/kernel/security/lsm` for a `bpf` token; if absent,
  add `lsm=...,bpf` to the kernel cmdline and reboot (`CONFIG_BPF_LSM=y` alone is *not* sufficient),
* **cgroup v2** unified hierarchy, and
* the nightly bpf toolchain: `rustup toolchain install nightly --component rust-src` and
  `cargo install bpf-linker` (LLVM version matched to the pinned nightly — the top build-break risk).

`bee check` verifies these gates and **fails closed** (exit 65) when enforcement is unavailable, so bee
never runs with silently-absent enforcement.

## Caveats worth knowing

* **Policy preparation has two stages.** `bee-core` compiles author intent into a backend-neutral
  `CompiledPolicy`; `bee-userspace` must then produce an eBPF `EnforcementPlan` before any kernel state
  is touched. `bee validate` runs both stages, so success means the policy is runnable by this backend.
* **Current eBPF glob support** is exact/subtree plus postfix (`*.ext`). Segment (`**/name`) and
  bounded-star patterns are normalized by the core but rejected during enforcement planning, as are
  inode-pinned executables and enabled exfiltration detection. Nothing is silently weakened.
* **Path matching sees resolved paths.** LSM hooks fire after path resolution, so enforcement is
  TOCTOU-resistant, but a rule written against a *symlink's* name won't match its target, and name-based
  rules don't catch hard links / alternate bind mounts to the same inode. Inode-pinned rules are not
  yet implemented and are rejected during enforcement planning.
* **Capabilities** to load LSM programs: `CAP_BPF + CAP_PERFMON` (possibly `+ CAP_MAC_ADMIN`) or
  `CAP_SYS_ADMIN`. The exact minimal set is kernel-version-dependent — verify empirically per target.

## Not in the MVP

Exfiltration detection (US3), the Landlock+seccomp degraded fallback, macOS/Windows, and behavior when
bee runs inside an existing container are out of scope for the MVP (see the spec's assumptions).

## License

MIT OR Apache-2.0
