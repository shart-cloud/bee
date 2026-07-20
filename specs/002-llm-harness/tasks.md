---
description: "Task list for bee LLM Agent Harness — Slice 1 (US1 vertical)"
---

# Tasks: bee LLM Agent Harness (Slice 1 — US1)

**Input**: Design documents in `/specs/002-llm-harness/` (plan.md, research.md, data-model.md, contracts/, quickstart.md)

**Scope**: This list delivers **User Story 1** end-to-end — one model, one scenario, tools that
execute inside a bee scope, an auditable transcript — with the provider seam proven across **two
vendors** (Anthropic + a configurable OpenAI-compatible client). US2/US3/US4 are captured in the
**Deferred** section, not built here.

**Tests**: The constitution mandates **test-first for enforcement behavior**. Enforcement/credential
tests are written to **fail first** (marked ⛔FAIL-FIRST) before their implementation tasks.

## Format: `[ID] [P?] [Story] Description with file path`
- **[P]**: parallelizable (different files, no incomplete dependency)
- **[US1]**: belongs to User Story 1 (setup/foundational/polish carry no story label)

---

## Phase 1: Setup (Shared Infrastructure)

- [x] T001 Create the `bee-harness/` crate skeleton (`bee-harness/Cargo.toml` + `bee-harness/src/lib.rs`) and add `"bee-harness"` to `members` in `/home/jg/git/bee/Cargo.toml`
- [x] T002 Add dependencies to `bee-harness/Cargo.toml`: `rig-core = "0.40"`, `tokio` (features `rt-multi-thread`, `macros`, `process`, `time`), `async-trait`, `serde`/`serde_json`, `toml`, `thiserror`; path deps `bee-userspace = { path = "../bee-userspace", features = ["enforce"] }`, `bee-core`, `bee-common`. Promote shared versions to `[workspace.dependencies]` where sensible
  - **Implemented differently (same outcome):** `enforce` is an **opt-in feature** on `bee-harness` (`[features] enforce = ["bee-userspace/enforce"]`), **not** an unconditional `features=["enforce"]` on the path dep. An unconditional pull would make Cargo feature-unification build the eBPF object for the *entire* workspace, breaking the default `cargo build`/`cargo test --workspace` (SC-005). Default build ⇒ `Sandbox::Host` (no scope); `--features enforce` ⇒ `Sandbox::Enforced`. `clap` was also added later for the CLI (T019).
- [x] T003 [P] Configure crate lints to match the repo (deny warnings in user-space crates) in `bee-harness/src/lib.rs` / `Cargo.toml`; confirm `cargo clippy -p bee-harness` is wired

**Checkpoint**: `cargo build -p bee-harness` compiles an empty lib; workspace still builds.

---

## Phase 2: Foundational (Blocking Prerequisites — shared types)

**Purpose**: The seam types and offline test double every US1 task builds on. No story label.

- [x] T004 [P] Define provider-seam value types in `bee-harness/src/provider.rs`: `Conversation`, `Message` (`User`/`Assistant`/`ToolResult`), `Turn`, `ToolCall`, `StopReason`, `Usage`, `ModelError`, `ToolSchema` (per data-model.md)
- [x] T005 [P] Define the `Model` trait (`async fn complete(&Conversation, &[ToolSchema]) -> Result<Turn, ModelError>`, `fn id`) in `bee-harness/src/provider.rs` (per contracts/model-trait.md)
- [x] T006 [P] Define the `Tool` trait + `ToolResult` + tool registry (name→tool, schema collection) in `bee-harness/src/tools.rs` (per contracts/tool-contracts.md)
- [x] T007 [P] Define `EpisodeTranscript`, `TranscriptTurn`, `EpisodeStatus`, `Timing` (serde) + the output-truncation helper (default 100 KB, sets `truncated`/`original_len`, FR-015) in `bee-harness/src/transcript.rs`
- [x] T008 [P] Define `Scenario`/`WorkdirSetup`/`ScoringMode` (`bee-harness/src/scenario.rs`) and `ProviderConfig`/`ProviderType` (`bee-harness/src/config.rs`) with TOML load + validation (per contracts/scenario-schema.md); reject `mode = "ctf"` (US3) and unknown tool names
- [x] T009 Implement `MockModel` (scripted `Vec<Turn>` / closure) in `bee-harness/src/provider/mock_model.rs` — the offline, deterministic backend all host tests use

**Checkpoint**: seam + MockModel compile; the loop can be built against `Model` without any real provider.

---

## Phase 3: User Story 1 — Single-Model Agent Episode (P1)

**Goal**: An operator runs one LLM agent through a scenario; tools execute inside a bee scope; kernel
denials reach the model and the transcript; an `EpisodeTranscript` is produced.

**Independent test**: A restrictive policy + a prompt to read a protected path → the tool returns
`EACCES`, the transcript's audit trail has the `file_open` deny, status `completed` (US1 AS-1); a
permissive policy + a write in the workdir → success, no denials (US1 AS-2).

### Tests (write first)

- [x] T010 [P] [US1] Host loop tests with `MockModel` in `bee-harness/tests/episode_loop.rs`: turn-limit → `timeout`/limit status, text-only turn → `no_tool_calls` (SC-007), malformed tool args → error `ToolResult` + loop continues (FR-016), oversized output → truncated + marked (FR-015), transcript shape (FR-009). ⛔FAIL-FIRST
- [x] T011 [P] [US1] Host credential-isolation test in `bee-harness/tests/cred_isolation.rs`: plant a fake key env var, run a `bash` tool that echoes it, assert the child's env does **not** contain it (FR-018 env-strip). ⛔FAIL-FIRST
- [x] T012 [US1] VM enforcement cases in `test/vm/remote-matrix.sh`: `episode-file-deny` (MockModel scenario whose scripted read hits a policy-denied path → tool result `is_error` + `EACCES`, transcript audit has `file_open` deny, status `completed`) and `episode-allow` (permissive write succeeds, no denials). ⛔FAIL-FIRST (real-kernel enforcement — constitution test-first)

### Implementation

- [x] T013 [US1] Implement `RigModel` in `bee-harness/src/provider/rig_model.rs`: build Anthropic and `openai-compat` (custom `base_url`) Rig clients; map `Conversation`→`CompletionRequest{preamble,chat_history,tools}` and `CompletionResponse`(`AssistantContent::{Text,ToolCall}`)→`Turn`; map 429/5xx→`ModelError::Transient`. **Load the `claude-api` skill**; pass current model ids as strings (H8); confirm the exact `openai::Client` base_url builder from rig source (research H4 NEEDS-CONFIRM)
- [x] T014 [P] [US1] Implement the `bash` tool in `bee-harness/src/tools/bash.rs`: `std::process::Command` `sh -c <cmd>` with `pre_exec(Scope::join_closure())` + hardening, credential env stripped, run via `tokio::process::Command::from(std).output().await` under a per-call `tokio::time::timeout`; map exit/`EACCES` → `ToolResult` (per contracts/tool-contracts.md)
  - **Minor method note:** spawn/harden goes through the shared `tools/exec.rs::run_child` (used by all tools). Output is captured via `wait_with_output()` with `kill_on_drop(true)` and a concurrently-spawned stdin writer (avoids a pipe deadlock). The timeout is enforced by the **loop's wall-clock deadline** wrapping each tool call (`episode.rs`) rather than a per-call `tokio::time::timeout` — same guarantee, and `kill_on_drop` reaps the child if the deadline fires.
- [x] T015 [P] [US1] Implement `read_file`/`write_file`/`list_directory` in `bee-harness/src/tools/files.rs` as in-scope sandboxed children with structured args; malformed args → error `ToolResult` (FR-016)
- [x] T016 [US1] FR-018 credential isolation: extend `bee-userspace/src/spawn.rs` to `env_remove` the configured provider key vars (alongside the existing `LD_*`) for tool children, and set `prctl(PR_SET_DUMPABLE, 0)` on the harness process at startup in `bee-harness/src/lib.rs`. The `spawn.rs` change MUST keep the default (no-enforce) build behavior identical (SC-005)
  - **Implemented differently (same outcome):** `spawn.rs` was **left untouched**. The provider-key env-strip is done **harness-side** — `Sandbox::tool_command` calls `cmd.env_remove(k)` for each key var (`sandbox.rs`, `key_vars()`) — because stripping provider keys is harness policy, not something every `bee run` child should inherit. `prctl(PR_SET_DUMPABLE, 0)` is `bee_harness::set_non_dumpable()`, called at CLI startup. This is a *stronger* SC-005 guarantee than the task specified (zero `bee-userspace` changes). Verified by `tests/cred_isolation.rs`.
- [x] T017 [US1] Implement the bee-owned episode loop in `bee-harness/src/episode.rs`: `Engine::init` + `create_scope` from `scenario.policy_path`, `take_audit_reader`; multi-turn loop calling `Model::complete`; execute each `ToolCall` in-scope; after each call `reader.drain(..)` filtered by `scope.cgroup_id` and correlate to the call (FR-008, research H5); enforce `turn_limit` + wall-clock `deadline` (kill children + teardown on expiry, FR-007); statuses `completed`/`timeout`/`no_tool_calls`/`api_error` (backoff retry FR-017)/`infra_error`
- [x] T018 [US1] Assemble `run_episode()` public API + `EpisodeTranscript` (per-turn calls + correlated audit + timing + usage) in `bee-harness/src/lib.rs`
- [x] T019 [US1] Implement the `bee-episode` CLI in `bee-harness/src/bin/bee-episode.rs` (`--scenario`, `--provider`; resolve key from `api_key_env`; write the transcript JSON to stdout/`--out`)
- [x] T020 [P] [US1] Add example configs under `specs/002-llm-harness/examples/`: `hello.toml`, `ollama.toml`, `anthropic.toml`, `read-denied-ssh.toml` (referenced by quickstart.md)

### Make tests pass

- [x] T021 [US1] Make T010/T011 green: `cargo test -p bee-harness` + `cargo clippy -p bee-harness` clean
- [x] T022 [US1] Build `bee-cli --features enforce` + the harness, ship to the VM, make T012 green (`episode-file-deny` + `episode-allow`); bump the case count in `test/vm/README.md`

**Checkpoint (MVP)**: US1 independent test passes on the VM; an LLM tool call hitting the kernel
boundary shows up as a denial in the transcript. This is the shippable slice. **Status: green — 23/23
on the VM** (incl. `episode-file-deny`/`episode-allow`); Anthropic proven under live enforcement, Ollama
proven for the openai-compat tool-calling path.

### Built beyond the slice-1 task text (in tree)
- **`mock` provider type** (`config.rs`: `ProviderType::Mock` + `provider.script`; factory in `provider.rs::model_from_config`) — a scripted, no-key/no-network backend. Makes the VM enforcement case self-contained and gives an offline CLI demo path.
- **Live progress → stderr** (`episode.rs`: `ProgressSink` on `LoopOptions`; `--quiet` on the CLI) and the **`--task` ad-hoc flag** (`bin/bee-episode.rs`: build a scenario from flags, no TOML) — from the UX pass.
- Two VM-only bugs found + fixed: the `Engine` (owning `aya::Ebpf`) must be moved into `Sandbox::Enforced` and kept alive for the episode (else the LSM programs detach mid-run); and `create_scope` must be passed `cgroup::DEFAULT_PARENT`, not `""`.

---

## Phase 4: Polish & Cross-Cutting

- [x] T023 [P] Doc comments across `bee-harness` + a crate README; add a harness overview to the repo root `README.md`
- [x] T024 [P] Wire the opt-in live smoke tests from quickstart.md (Ollama for openai-compat; Anthropic gated on `ANTHROPIC_API_KEY`), skipped cleanly when the endpoint/key is absent
- [x] T025 Regression gate (SC-005): `cargo test --workspace` + both clippy modes stay green; confirm the `spawn.rs` change leaves the default `bee-userspace` build/tests unchanged and the 001 VM matrix still passes

---

## Deferred (OUT OF SLICE 1 — do not implement here)

Captured so the seam stays compatible; each is its own future slice building on this loop.

### User Story 2 — Multi-Provider Comparison (P2)
- Batch runner over `scenarios × providers` (FR-010); per-provider `api_error` isolation (US2 AS-2); transcript diff / identical-enforcement assertion (SC-002). Reuses `Model` + `run_episode` unchanged.

### User Story 3 — Adversarial CTF Episode (P3)
- `submit_flag` + `give_up` tools; flag planting in `WorkdirSetup`; `EpisodeStatus::{Captured,NotCaptured}`; `ScoreReport` (coverage/novelty/parsimony) computed from the audit trail (FR-005/006, SC-003).

### User Story 4 — Async Audit Stream & Concurrent Episodes (P4)
- Feature-gated (`tokio`) `AsyncFd` audit reader in `bee-userspace` (FR-012/013) — **additive, default build unchanged** (FR-014/SC-005); per-`cgroup_id` demux of the single `AUDIT_RB` stream; concurrent episodes via `tokio::spawn` (SC-004). This is the first task to touch `bee-userspace`'s async path.

---

## Dependencies & Execution

**Phase order**: Setup (T001–T003) → Foundational (T004–T009) → US1 (T010–T022) → Polish (T023–T025).

**Within US1**:
- Tests T010/T011/T012 are written first (fail), then unblocked by implementation.
- T013 (RigModel) depends on T004/T005; T014/T015 (tools) depend on T006 and `Scope` (001); they are `[P]` with each other.
- T017 (loop) depends on T009 (MockModel), T006/T014/T015 (tools), T007 (transcript), T016 (cred isolation for spawned children).
- T018 depends on T017; T019 depends on T018.
- T021 unblocks after T014–T018; T022 after T016–T019 + T012.

**Parallel opportunities**:
- Foundational: T004, T005, T006, T007, T008 (distinct files) run in parallel; T009 after T004/T005.
- US1: T010 ∥ T011 (tests); T014 ∥ T015 ∥ T020 (distinct files) once the seam exists.

## Implementation Strategy

**MVP = Phase 1 + 2 + 3.** Ship when the US1 checkpoint (T022) is green on the VM: a real LLM (or
MockModel) drives tools inside a bee scope and the enforced denial appears in the transcript. Defer US2/
US3/US4 until then. Order the live-provider work so the **Ollama openai-compat path** is the CI smoke
(no paid key); the Anthropic path is proven by the opt-in smoke in T024.
