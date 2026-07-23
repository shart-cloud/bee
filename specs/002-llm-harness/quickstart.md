# Quickstart: bee LLM Agent Harness (Slice 1 — US1)

> **Command names changed.** The consolidation (ADR-0002) replaced `bee-episode`, `bee-repl`, and
> `bee-metrics` with subcommands of the single `bee` executable: `bee run`, `bee repl`, and
> `bee metrics`. The raw process runner moved from `bee run` to `bee exec`, and a session with no
> policy now needs an explicit `--host`. The commands below are recorded as this feature shipped
> them; translate accordingly.

Validates the first vertical slice: an LLM agent runs a multi-turn tool loop **inside a bee scope**,
tool denials come from the kernel, and the episode transcript records the calls + audit trail. Details
live in [data-model.md](./data-model.md) and [contracts/](./contracts/).

## Prerequisites
- Feature 001 building: `cargo build -p bee-cli --features enforce --release` works; the BPF-LSM VM is
  reachable (see `test/vm/README.md`). Enforcement assertions run there, not on a non-BPF-LSM host.
- New crate `bee-harness` added to the workspace `members`.
- For the offline path: nothing else (uses `MockModel`).
- For the live smoke path: a local **Ollama** (`ollama serve`, a small coder model pulled) for the
  openai-compat provider; optionally `ANTHROPIC_API_KEY` exported for the Anthropic path.

## 1. Offline, deterministic (host — no kernel, no network)
Proves the loop, tools, transcript, truncation, and malformed-arg handling with `MockModel`.

```bash
cargo test -p bee-harness           # loop + tools + transcript + FR-016/FR-015 + no_tool_calls
```
Expect: green. Includes a test asserting a spawned tool child does **not** see the provider key env var
(FR-018 env-strip) using a fake key var.

## 2. Live provider smoke (host — network, no enforcement)
Proves `RigModel` really talks to both provider shapes. No sandbox in this step (a permissive/no
policy), so it works off the VM.

```bash
# openai-compat via local Ollama (no paid key)
bee-episode --scenario specs/002-llm-harness/examples/hello.toml \
            --provider specs/002-llm-harness/examples/ollama.toml

# Anthropic (needs ANTHROPIC_API_KEY; uses a current model id)
bee-episode --scenario specs/002-llm-harness/examples/hello.toml \
            --provider specs/002-llm-harness/examples/anthropic.toml
```
Expect: a JSON transcript with `status: "completed"`, `model_id` set, and ≥1 tool call executed.

## 3. Enforcement assertion (VM — the US1 independent test)
The load-bearing check: a scripted (`MockModel`) episode whose `bash`/`read_file` call reads a
policy-denied path returns a kernel `EACCES`, and the denial is in the transcript.

```bash
# runs inside test/vm/remote-matrix.sh as the `episode-file-deny` case
BEE_SKIP_BUILD=1 bash test/vm/matrix.sh    # after building the enforce binary + harness
```
Expect (`RESULT|episode-file-deny|PASS|...`):
- the tool result for the denied read has `is_error: true` and `EACCES` in its content,
- the transcript `audit_trail` contains a `file_open` **denied** event for the path,
- the episode ends `status: "completed"` (the agent ran; the *operation* was denied) — matching
  **US1 Acceptance Scenario 1**.

## 4. Allowed-op scenario (VM)
A permissive policy + a prompt to create a file in the working dir → the write succeeds, **no** denied
audit events, transcript records the successful call (**US1 Acceptance Scenario 2**).

## Success criteria mapped
| Check | Criterion |
|-------|-----------|
| Step 1 green | FR-001/003/004/015/016; SC-007 (`no_tool_calls`); FR-018 env-strip |
| Step 2 transcript | FR-002 (both providers), FR-009 (transcript shape) |
| Step 3 PASS | US1 AS-1; FR-008 (audit correlation); Constitution III (kernel is the boundary) |
| Step 4 PASS | US1 AS-2 |
| 10-turn transcript < 1 MB | SC-006 |
| 10-turn episode < 2 min | SC-001 |

## Out of this slice (later validation)
Multi-provider batch diffing (US2), CTF flag capture + scoring (US3), and 4 concurrent episodes with
the async audit stream (US4) — the async `bee-userspace` layer is not built here (default build
unchanged, SC-005).
