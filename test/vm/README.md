# bee live enforcement harness (KubeVirt VM)

eBPF LSM hooks only enforce on a **real kernel** with `bpf` in the active LSM list — a privileged
container shares the node kernel and the hooks self-skip, turning deny-cases into false greens. So
enforcement is validated against a KubeVirt VM (Ubuntu 24.04, kernel 6.8, `lsm=…,bpf`), mirroring the
pattern in `agentcontainers/test/vm/`.

## Layout

| File | Role |
|------|------|
| `matrix.sh` | **Host-side orchestrator**: builds `bee --features enforce`, ships it + the runner into the VM over the `virtctl` tunnel, runs the matrix as root, and gates on the results. |
| `remote-matrix.sh` | **VM-side runner**: exercises the enforcement matrix against the real kernel and emits `RESULT\|<case>\|<PASS\|FAIL>\|<note>` lines. Exits non-zero on any failure. |

The build happens on the host because host and guest are both Ubuntu noble (glibc 2.39) and the host
has the nightly bpf toolchain (nightly + `rust-src` + `bpf-linker`) the VM lacks.

## Prerequisites

- `kubectl` (context `admin@talos-cluster`) and `virtctl` on `PATH`.
- The VM `ac-matrix-vm` in namespace `ac-matrix` running, and the ssh key at `~/.ssh/ac-matrix-vm`
  (provisioned by `agentcontainers/test/vm/up.sh`).
- Host bpf toolchain: `rustup toolchain install nightly --component rust-src` + `cargo install bpf-linker`.

## Run

```bash
cd test/vm
./matrix.sh                 # build + ship + run the full matrix
BEE_SKIP_BUILD=1 ./matrix.sh   # reuse target/release/bee
```

Override target via env: `NS`, `VM`, `KEY`.

## Cases

| Case | Asserts |
|------|---------|
| `kernel-bpf-lsm` / `kernel-btf` / `bee-check` | substrate: `bpf` in LSM list, BTF present, all `bee check` gates pass |
| `net-allow` / `net-deny` | address allowlist: allowed dest connects, un-listed dest blocked (`EPERM`) |
| `file-deny` / `file-allow` | path policy: denied subtree blocked (`EACCES`), other paths open |
| `file-glob-deny` / `file-glob-allow` | postfix glob (`*.secret`) deny enforced; non-matching files still open |
| `file-segment-refused` | fail-closed: an unenforceable `**/name` rule makes bee refuse rather than under-enforce |
| `rw-read-source` / `rw-write-source-denied` | read/write modes: a `read`-marked source tree is readable but write-opens are blocked (`EACCES`) — US2 AS-2 |
| `rw-write-scratch` / `rw-write-default-deny` | a `write`-granted scratch dir is writable; an unlisted path is denied by default (scope declares a writable surface) |
| `exec-allow` / `exec-deny` | exec allowlist: allowlisted binary runs, un-listed `execve` denied |
| `exec-unresolvable-denied` / `file-unresolvable-denied` | fail-closed on what a hook cannot evaluate: a directory chain past `bpf_d_path`'s 4KB buffer makes the resolved path unrenderable, and the exec/open is refused rather than allowed (research R16) |
| `net-unix-denied` | a network-enforced scope refuses an AF_UNIX connect — the `host:port` language cannot name one, and no rule matching means deny |
| `observe-mode` | dry-run: operation allowed but emits a `decision:"observed"` audit event |
| `atten-reject` | subagent attenuation: an over-broad child (`--parent`) is refused before running |
| `atten-subset-allow` / `atten-subset-deny` | a valid subset child runs and enforces its *narrower* policy (a dest the parent allows but the child dropped is blocked) |
| `scope-isolation` | a process outside any bee scope is unaffected |
| `episode-file-deny` / `episode-allow` | LLM agent harness (002): a scripted episode's tool call that reads a policy-denied path returns kernel `EACCES` in the transcript with a `file_open` denial and status `completed` (US1 AS-1); a permissive in-scope write succeeds with no denials (US1 AS-2) |

All 35 pass on the reference VM. See the repo root `README.md` for the enforcement design and the
`bpf_d_path` / offset caveats.
