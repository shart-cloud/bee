# bee-harness

Drives an LLM agent through a multi-turn tool loop whose tools execute **inside a bee scope**,
producing an auditable episode transcript. Feature `002-llm-harness`, slice 1 (US1).

The multi-turn loop, tool execution, retry/backoff, output truncation, and per-tool-call audit
correlation all live here — not in [Rig](https://rig.rs/), which is confined behind the `Model`
provider seam. A kernel denial a tool hits shows up in the transcript as an `EACCES` result **and**
a `file_open` denial from the audit ring: the harness is orchestration; the LSM is the boundary
(Constitution III).

## Build modes

| Mode | Tools run as | Used for |
|------|--------------|----------|
| **default** | hardened, credential-stripped **host** processes (no kernel scope) | offline `MockModel` tests, live-provider smoke |
| **`--features enforce`** | children joined to a real bee scope cgroup; kernel denials drained into the transcript | the VM enforcement case |

The `enforce` feature is **opt-in** so the default workspace build stays free of the eBPF toolchain
(Constitution V / SC-005).

## Layout

- `provider` — the `Model` seam + value types; `MockModel` (offline) and `RigModel` (Anthropic +
  configurable OpenAI-compatible).
- `tools` — `bash` / `read_file` / `write_file` / `list_directory`, each a sandboxed child.
- `sandbox` — the `Host` / `Enforced` seam that lets one loop serve both build modes.
- `episode` — the bee-owned loop (`run_loop`) and `run_episode` (builds the scope, materializes the
  workdir, tears down).
- `transcript` — `EpisodeTranscript` + truncation.
- `scenario` / `config` — declarative TOML inputs (Constitution IV).

The commands that drive all of this — `bee run`, `bee repl`, `bee metrics` — live in the `bee-cli`
application package. This crate ships no binaries of its own (ADR-0002).

## Quickstart

```bash
# Offline, deterministic (host — no kernel, no network)
cargo test -p bee-harness

# Live provider smoke (host — network, no enforcement; skipped when absent)
cargo test -p bee-harness --test live_smoke -- --nocapture

# One episode via the CLI (mock provider — no key needed). `--host` is required on a build
# without enforcement: bee refuses to run unenforced unless you say so.
cargo run -p bee-cli -- run --host \
  --scenario specs/002-llm-harness/examples/read-denied-ssh.toml \
  --provider specs/002-llm-harness/examples/mock-read-denied.toml
```

Enforcement assertions run on the BPF-LSM VM (`test/vm/README.md`) as the `episode-file-deny` /
`episode-allow` cases. See `specs/002-llm-harness/quickstart.md` for the full validation path.

## Credentials (FR-018)

Provider keys are named by env var (`api_key_env`), never embedded in config or the transcript.
Every tool child is spawned with the provider key vars stripped from its environment, and the
harness marks itself non-dumpable (`prctl(PR_SET_DUMPABLE, 0)`) so a same-uid child cannot read the
key from `/proc/<harness-pid>/environ`.
