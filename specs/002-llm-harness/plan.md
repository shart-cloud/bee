# Implementation Plan: bee LLM Agent Harness (Slice 1 — US1 vertical)

**Branch**: `002-llm-harness` | **Date**: 2026-07-19 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/002-llm-harness/spec.md`

## Summary

Add a new `bee-harness` crate that runs an LLM coding agent through a multi-turn tool-calling loop
**inside a bee-enforced scope**, producing an auditable episode transcript. This plan scopes the
**first vertical slice = User Story 1** (single-model episode) with the provider abstraction proven
across **two vendors from the start** (Anthropic + a single OpenAI-compatible client whose `base_url`
is configurable — OpenAI/OpenRouter/Ollama/vLLM/Groq).

Technical approach: **bee-harness owns the agent loop**; the Rig framework (`rig-core` 0.40) is used
only for the *single-turn* completion call (`CompletionModel::completion`), not its `Agent::multi_turn`.
That keeps turn/time limits, per-tool-call audit correlation, output truncation, and malformed-arg
handling under bee's explicit control and confines Rig's API surface to one method. tokio lives only
in `bee-harness`; `bee-core`/`bee-common` stay sync and runtime-free (Constitution V). The first slice
drains the kernel audit ring **between turns** using the existing synchronous `AuditReader`, so it
needs **no** async layer in `bee-userspace` — the feature-gated `tokio` audit stream (FR-012/013)
defers to US4 (concurrent episodes).

## Technical Context

**Language/Version**: Rust 2021, stable (rust-version 1.85). `bee-harness` is async (tokio); the
enforcement crates it depends on remain sync.

**Primary Dependencies**: `rig-core` 0.40 (provider abstraction, single-turn completion); `tokio` 1.x
(`rt-multi-thread`, `macros`, `process`, `time`); `async-trait`; `serde`/`serde_json`; `toml`;
in-repo `bee-userspace` (with `enforce`), `bee-core`, `bee-common`.

**Storage**: Episode transcripts serialized to JSON files; scenarios authored as TOML; the bee
security policy is the existing TOML (feature 001). No database.

**Testing**: `cargo test` on the host with a deterministic `MockModel` (no network/keys) for the loop,
tools, transcript, truncation, and malformed-arg paths; the live enforcement assertion (US1 independent
test: denied read → `EACCES` → denial in transcript) runs on the BPF-LSM VM harness (extends
`test/vm/`). Optional live-provider smoke test via a local **Ollama** endpoint (openai-compat path, no
paid key) and, when `ANTHROPIC_API_KEY` is present, an opt-in Anthropic smoke test.

**Target Platform**: Linux with eBPF LSM + cgroup v2 (same substrate as the MVP — the harness calls
`Engine::init`/`create_scope`).

**Project Type**: Rust library crate (`bee-harness`) with a thin CLI surface (a `bee episode` /
`bee-harness` binary) — the library holds all logic (Constitution V).

**Performance Goals**: SC-001 — a 10-turn single-agent episode completes < 2 min wall-clock (dominated
by LLM latency + process spawn, not bee overhead). SC-004 (4 concurrent episodes < 2× single) is a US4
goal, out of this slice.

**Constraints**: `bee-core`/`bee-common` MUST NOT gain an async runtime (Constitution V). The
`bee-userspace` `tokio` async layer is additive and deferred; the default no-feature build MUST stay
byte-for-byte behaviorally identical (FR-014 / SC-005). Provider credentials MUST be unreadable from
within any scope (FR-018). Tool output truncated to a configurable limit (default 100 KB, FR-015).

**Scale/Scope**: This slice = one model, one scenario, one scope, one transcript, with two provider
backends selectable by config. Batch (US2/FR-010), CTF scoring (US3), and concurrency (US4) are later
slices sharing this loop.

## Constitution Check

*GATE: evaluated against Principles I–V and the Security/Platform + Workflow constraints.*

| Principle | Assessment |
|-----------|------------|
| **I. Deny-by-Default & Fail-Closed** | PASS. Tools execute inside a bee scope, so their access is kernel-denied by default; the harness surfaces enforced errors (`EACCES`) to the model as tool results rather than swallowing them. Scope-creation failure ⇒ `infra_error`, loop never starts. **FR-018 credential isolation is fail-closed** (strip creds from tool-child env + non-dumpable harness). |
| **II. Capability Attenuation** | PASS (not exercised in this slice). The loop uses `create_scope` for one flat scope. Subagent tools (a tool that spawns a child agent) would use `derive_scope` and inherit 001's attenuation guarantees; deferred, and noted so no design here precludes it. |
| **III. Kernel Enforcement Is Authoritative** | PASS. The harness is **orchestration, not an enforcement point**: it does not filter or pre-authorize tool calls in user space as a substitute for the LSM. A tool's confinement is entirely the kernel scope it runs in; the harness only *reports* what the kernel decided (via the audit trail). |
| **IV. Policy-as-Data** | PASS. Scenarios and provider configs are declarative TOML; the security policy is the existing 001 TOML. No enforcement logic is authored as code in scenarios. |
| **V. Library-First, Runtime-Free Core** | PASS — **the load-bearing gate**. tokio/rig are confined to `bee-harness`. `bee-core` and `bee-common` stay sync with no async dep. `bee-userspace` gains its async path **only** behind a future `tokio` feature (US4), additive over the sync `AuditReader`; this slice adds none. The `bee-harness` CLI is a thin wrapper over the library. |
| **Security/Platform** | PASS. Linux BPF-LSM only (inherited). Privileged-target refusal + process hardening inherited from 001; FR-018 adds `PR_SET_DUMPABLE=0` on the harness. Every episode's denials are in the transcript's audit trail (Auditability). |
| **Workflow / Test-first** | PASS. The enforcement assertion (denied read → transcript) is written as a VM harness case; `MockModel` makes the loop deterministically testable on the host. No new attenuation logic ⇒ no new property tests required this slice. |

**Result: no violations.** Complexity Tracking below is empty.

## Project Structure

### Documentation (this feature)

```text
specs/002-llm-harness/
├── plan.md              # This file
├── research.md          # Phase 0 — Rig 0.40 integration, loop, credential isolation, decisions
├── data-model.md        # Phase 1 — Scenario, Episode, EpisodeTranscript, Turn, Model seam
├── quickstart.md        # Phase 1 — run the US1 episode (mock + Ollama smoke + VM enforcement case)
├── contracts/
│   ├── model-trait.md       # the bee-harness `Model` provider seam (over Rig)
│   ├── tool-contracts.md    # bash / read_file / write_file / list_directory tool I/O schemas
│   └── scenario-schema.md   # scenario TOML schema
└── tasks.md             # Phase 2 (/speckit-tasks — NOT created here)
```

### Source Code (repository root)

```text
bee-harness/                 # NEW workspace member (async; carries tokio + rig-core)
├── Cargo.toml               # deps: rig-core 0.40, tokio, async-trait, serde, toml, bee-userspace(enforce), bee-core
└── src/
    ├── lib.rs               # public API: run_episode(), types re-exports
    ├── provider.rs          # `Model` trait (async complete()), `Conversation`, `Turn`, `ToolCall`
    ├── provider/
    │   ├── rig_model.rs      # RigModel: Anthropic + openai-compat (base_url) over rig CompletionModel
    │   └── mock_model.rs     # MockModel: scripted turns for deterministic tests
    ├── tools.rs             # `Tool` trait + registry + JSON-schema definitions
    ├── tools/
    │   ├── bash.rs           # spawn `sh -c <cmd>` INTO the bee scope; capture stdout/stderr/exit
    │   └── files.rs          # read_file / write_file / list_directory (in-scope)
    ├── episode.rs           # bee-owned loop: setup scope → turns → per-call audit drain → teardown
    ├── transcript.rs        # EpisodeTranscript (serde) + truncation
    ├── scenario.rs          # Scenario TOML load + working-dir/flag setup
    ├── config.rs            # ProviderConfig (type, base_url, model, key ref) + credential handling
    └── bin/
        └── bee-episode.rs    # thin CLI: `bee-episode --scenario s.toml --provider p.toml`

bee-userspace/src/
    └── spawn.rs             # EXTEND: strip provider-key env vars from tool children (FR-018);
                             #         (async AuditReader lands here later, behind `tokio` feature — US4)

test/vm/
    └── remote-matrix.sh     # EXTEND: add an `episode-*` case (MockModel-driven denied read → transcript)
```

**Structure Decision**: One new library crate, `bee-harness`, added to the workspace `members`. It is
the only crate that depends on tokio/rig; the enforcement crates it consumes stay sync. The sole edit
outside the new crate this slice is `bee-userspace/src/spawn.rs` (env-strip for FR-018) — the async
`bee-userspace` layer is a **separate later change** so this slice cannot regress the default build.

## Complexity Tracking

> No Constitution violations — no entries.
