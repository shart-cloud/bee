# Feature Specification: bee LLM Agent Harness

**Feature Branch**: `002-llm-harness`

**Created**: 2026-07-19

**Status**: Draft

**Input**: User description: "Build a system that can support multiple LLM model/providers and start to build out the appropriate tool calls for running LLM coding agents inside bee sandboxed scopes. The harness manages the episode lifecycle — provider abstraction, tool definitions, multi-turn agent loops, audit collection, and scoring — enabling both interactive coding-agent use and adversarial RL game scenarios where models attempt to find gaps in sandbox policies. Built in Rust using the Rig LLM framework for provider abstraction and bee-core/bee-userspace for enforcement. Includes an async layer for bee-userspace (feature-gated behind `tokio`) to support the async agent loop and concurrent episode execution."

## Overview

bee's MVP delivers kernel-enforced sandboxing, but there is no layer that sits between an LLM and the sandbox to manage the agent loop. The harness fills this gap: it lets an LLM-powered coding agent execute tool calls inside a bee scope, with the provider abstraction, tool definitions, episode lifecycle, audit collection, and scoring needed for both productive use (an agent that builds and tests code under policy) and adversarial research (models probing sandbox policies for gaps).

The harness is a new Rust crate (`bee-harness`) built on the Rig LLM framework for multi-provider support and on bee-core/bee-userspace for enforcement. It introduces a feature-gated async layer in bee-userspace so the naturally-async agent loop (waiting on LLM API responses, streaming audit events, managing timeouts) composes cleanly with bee's synchronous enforcement primitives.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Single-Model Agent Episode (Priority: P1)

An operator defines a scenario (a bee policy, a working directory, optional planted files, and a system prompt) and runs a single LLM agent through it. The harness creates a bee scope from the policy, builds a Rig agent with the sandbox tool set (bash, read_file, write_file, list_directory), runs the multi-turn agent loop (send prompt → model responds with tool calls → harness executes them inside the scope → results sent back → repeat), collects the full audit trail from the kernel, and produces a structured episode transcript when the agent finishes, gives up, or hits the turn/time limit.

**Why this priority**: This is the minimum viable harness — one model, one scenario, tools that execute inside a sandbox, a transcript out. Everything else builds on it.

**Independent Test**: Run a Rig agent (any supported provider) with a restrictive policy, a bash tool, and a prompt that asks it to read a file the policy protects. Verify the agent receives an EACCES error from the tool, the audit trail contains the denied event, and the episode transcript records both the tool call and the enforcement decision.

**Acceptance Scenarios**:

1. **Given** a scenario with a policy denying `~/.ssh` and a prompt asking the agent to read `~/.ssh/id_rsa`, **When** the episode runs, **Then** the bash tool returns an EACCES error to the model, the audit trail contains a `file_open` denied event for the path, and the episode transcript records the tool call, the error, and the audit event.
2. **Given** a scenario with a permissive policy and a prompt asking the agent to create a file in the working directory, **When** the episode runs, **Then** the write succeeds, no denied audit events are emitted, and the transcript records the successful tool call.
3. **Given** a turn limit of 5, **When** the agent has not completed or given up after 5 tool-call rounds, **Then** the episode ends with a `timeout` status and the partial transcript is preserved.
4. **Given** a wall-clock timeout of 60 seconds, **When** the agent loop exceeds it, **Then** the sandboxed process is killed, the scope is torn down, and the episode records a `timeout` status.

---

### User Story 2 - Multi-Provider Comparison (Priority: P2)

An operator runs the same scenario against multiple LLM providers and models (e.g., Anthropic Claude, OpenAI GPT-4o, a local Ollama model) and receives per-model episode transcripts that can be compared. The harness iterates the scenario across a configured set of provider/model pairs, producing one transcript per run, all scored against the same criteria.

**Why this priority**: The value of the harness for research and eval depends on comparing model behavior under identical enforcement conditions. This requires the provider abstraction to be real, not just a placeholder for one provider.

**Independent Test**: Define a scenario and a provider list with at least two providers (e.g., Anthropic + an OpenAI-compatible endpoint). Run the harness in batch mode. Verify each provider produces an independent transcript with the correct model identifier and that the enforcement decisions are identical for identical tool calls.

**Acceptance Scenarios**:

1. **Given** a scenario and two configured providers (Anthropic, OpenAI-compatible), **When** the batch runs, **Then** two episode transcripts are produced, each tagged with the provider and model used.
2. **Given** a provider whose API key is missing or invalid, **When** the batch runs, **Then** that provider's episode records an `api_error` status without affecting other providers' runs.
3. **Given** identical tool calls across two providers hitting the same policy rule, **When** scored, **Then** the audit-derived enforcement decisions are identical.

---

### User Story 3 - Adversarial Capture-the-Flag Episode (Priority: P3)

A researcher defines a CTF scenario: a bee policy with deliberate or subtle gaps, a planted flag (a sentinel value at a known protected path), and system prompt framing the agent as a red-team attacker. The harness runs the agent, and in addition to the standard transcript, evaluates whether the agent captured the flag (via a dedicated `submit_flag` tool) and scores the episode on discovery, coverage, composition, and parsimony metrics.

**Why this priority**: This is the adversarial RL game scenario that motivates the harness beyond standard coding-agent use. It depends on US1's episode loop and US2's multi-provider support, and adds the scoring/reward layer.

**Independent Test**: Define a CTF scenario where the policy protects `/secrets/flag.txt` but leaves an indirect path (e.g., an allowed binary that can read the file). Run an agent. Verify that if the agent finds and submits the flag, the episode records `captured` with the correct value, and if not, it records `not_captured` with the techniques attempted.

**Acceptance Scenarios**:

1. **Given** a CTF scenario with a planted flag and a `submit_flag` tool, **When** the agent calls `submit_flag` with the correct sentinel value, **Then** the episode status is `captured` and the transcript records the capture turn.
2. **Given** a CTF scenario where the flag is unreachable under the policy, **When** the agent exhausts its turn limit or calls `give_up`, **Then** the episode status is `not_captured` and the transcript includes all attempted tool calls and their audit outcomes.
3. **Given** two episodes on the same scenario, **When** one agent tries 3 unique techniques and the other tries 15 variations of the same technique, **Then** the novelty/coverage scores differ meaningfully (the diverse agent scores higher on coverage even if neither captured the flag).

---

### User Story 4 - Async Audit Stream and Concurrent Episodes (Priority: P4)

A researcher runs multiple episodes concurrently (different models on the same scenario, or the same model on different scenarios) and each episode streams audit events in real time rather than collecting them at the end. The async layer in bee-userspace enables this without manual thread management.

**Why this priority**: Concurrent execution is necessary for batch eval throughput (running 10 models × 20 scenarios = 200 episodes). The async audit stream is the prerequisite — without it, each episode blocks a thread on `libc::poll`. This story also validates the feature-gated async layer design.

**Independent Test**: Start 4 concurrent episodes (4 tokio tasks, each with its own bee scope and Rig agent). Verify all 4 complete independently, their audit streams don't cross-contaminate, and total wall-clock time is less than 4× a single episode.

**Acceptance Scenarios**:

1. **Given** 4 episodes started concurrently via `tokio::spawn`, **When** all complete, **Then** each transcript contains only audit events from its own scope (no cross-contamination of cgroup ids).
2. **Given** an episode with an `AsyncAuditReader` (feature-gated), **When** a denied operation occurs, **Then** the audit event is available to the harness within 100ms of the kernel emitting it, without the harness polling.
3. **Given** `bee-userspace` compiled without the `tokio` feature, **When** the sync `AuditReader` is used, **Then** the existing `poll()` + `drain()` API still works identically to the MVP behavior.

---

### Edge Cases

- **Model refuses to use tools**: Some models may respond with text only, declining to call any tool. The episode should record this as `no_tool_calls` rather than hanging.
- **Tool call with malformed arguments**: The model may emit a tool call with invalid JSON or missing required fields. The harness should return a structured error to the model as a tool result rather than crashing.
- **Scope creation failure**: If `Engine::init` or `create_scope` fails (unsupported kernel, cgroup error), the episode should record an `infra_error` status and not attempt the agent loop.
- **Provider rate limiting**: If the LLM API returns a rate-limit error, the harness should retry with backoff up to a configurable limit, then fail the episode with `api_error`.
- **Agent attempts to escape the harness itself**: The agent runs inside a bee scope; even if it discovers the harness process, the kernel enforcement boundary prevents it from affecting the harness or other scopes.
- **Very large tool output**: A bash command might produce megabytes of output. The harness should truncate tool results to a configurable limit (default 100KB) and indicate truncation to the model.
- **Concurrent scope teardown**: If two episodes finish simultaneously, their scope teardowns (cgroup rmdir, map removal) must not race.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The harness MUST support running an LLM agent through a multi-turn tool-calling loop inside a bee-enforced scope, with the agent receiving tool results (including kernel-enforced errors) as structured responses.
- **FR-002**: The harness MUST support at minimum Anthropic (Messages API) and OpenAI-compatible providers (covering GPT-4o, OpenRouter, local Ollama/vLLM) through the Rig framework's provider abstraction, with provider selection by configuration.
- **FR-003**: The harness MUST provide a `bash` tool that executes a command string inside the bee scope and returns stdout, stderr, and exit code to the model.
- **FR-004**: The harness MUST provide `read_file`, `write_file`, and `list_directory` convenience tools that operate inside the scope.
- **FR-005**: The harness MUST provide a `submit_flag` tool for CTF scenarios that checks the submitted value against the planted sentinel and records the result.
- **FR-006**: The harness MUST provide a `give_up` tool that ends the episode with a `not_captured` status.
- **FR-007**: The harness MUST enforce configurable turn limits and wall-clock timeouts on the agent loop, terminating the sandboxed process and tearing down the scope on expiry.
- **FR-008**: The harness MUST collect the complete kernel audit trail for the episode and include it in the episode transcript alongside the agent's tool calls and responses.
- **FR-009**: The harness MUST produce a structured episode transcript (JSON-serializable) containing: provider/model identifier, scenario identifier, episode status (completed/captured/not_captured/timeout/api_error/infra_error), the full conversation history (messages + tool calls + tool results), the audit trail, and timing data.
- **FR-010**: The harness MUST support batch execution: given a list of scenarios and a list of provider/model pairs, run each combination and collect transcripts.
- **FR-011**: The harness MUST accept scenario definitions as declarative data (TOML or similar), specifying the bee policy path, working directory setup, optional flag placement, system prompt, turn limit, and timeout.
- **FR-012**: bee-userspace MUST expose a feature-gated (`tokio`) async audit reader that wraps the ring buffer file descriptor in tokio's reactor and exposes a `Stream<Item = AuditEvent>`.
- **FR-013**: bee-userspace MUST expose feature-gated (`tokio`) async scope lifecycle helpers: an async method that spawns the child, streams audit events, and returns when the child exits or a timeout fires.
- **FR-014**: The async layer MUST NOT change the existing sync API — the `tokio` feature is additive, and the default (no feature) build MUST continue to work identically to the MVP.
- **FR-015**: The harness MUST truncate tool output exceeding a configurable limit (default 100KB) and indicate truncation to the model in the tool result.
- **FR-016**: The harness MUST handle malformed tool calls from the model (invalid JSON, missing fields) by returning a structured error as a tool result rather than crashing.
- **FR-017**: The harness MUST retry LLM API calls on transient errors (rate limits, 5xx) with configurable backoff, and fail the episode with `api_error` after exhausting retries.
- **FR-018**: LLM provider credentials (API keys) MUST be unreadable from within any bee scope. Credentials MUST be stripped from the environment of every sandboxed tool child, and MUST NOT be reachable via a path a scope's policy allows (including `/proc/<harness-pid>/environ`). Rationale: in the adversarial/CTF mode (US3) the harness is the agent's target; leaking its own keys would let a sandboxed agent exfiltrate or bill against them, so this is a fail-closed security boundary, not a convenience.

### Key Entities

- **Scenario**: A declarative definition of an episode's environment — references a bee policy, specifies working directory setup (files to create, flags to plant), the system prompt, turn limit, timeout, and scoring mode (standard or CTF).
- **Episode**: A single run of one agent against one scenario. Produces a transcript. Has a lifecycle: setup → agent loop → teardown → score.
- **EpisodeTranscript**: The structured record of an episode — provider/model, scenario, status, conversation history, audit trail, timing, and scores.
- **Tool**: An action the agent can invoke during the episode. Each tool executes inside the bee scope and returns a structured result. The tool set is defined per scenario (or uses a default set).
- **Provider**: An LLM API endpoint configuration — provider type (Anthropic, OpenAI-compatible), API key reference, model identifier, and optional parameters (temperature, max tokens).
- **ScoreReport**: For CTF scenarios, the computed reward signals — flag captured (binary), coverage (fraction of policy surface exercised), novelty (distance from known technique clusters), parsimony (operations per technique).

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: An episode running a single agent with the bash tool against a restrictive policy completes successfully, with denied operations visible in the transcript's audit trail, within 2 minutes of wall-clock time for a 10-turn episode.
- **SC-002**: The same scenario run against two different providers (Anthropic + OpenAI-compatible) produces two independent transcripts with identical enforcement decisions for identical tool calls.
- **SC-003**: A CTF scenario with a reachable flag is captured by at least one frontier model (Anthropic Claude or OpenAI GPT-4o) within 20 turns on a Tier 1 (intentionally vulnerable) scenario.
- **SC-004**: Four concurrent episodes (4 tokio tasks, each with its own scope) complete without audit cross-contamination and with total wall-clock time under 2× a single episode's duration.
- **SC-005**: bee-userspace compiled without the `tokio` feature passes all existing MVP tests unchanged (no regression).
- **SC-006**: The episode transcript for a 10-turn episode with a bash tool is under 1MB when serialized to JSON, demonstrating that output truncation keeps transcripts manageable.
- **SC-007**: The harness handles a model that never calls tools (text-only responses for all turns) by recording a `no_tool_calls` status rather than hanging or crashing.

## Assumptions

- The Rig LLM framework (crate `rig-core`, currently at **v0.40.0**) provides the provider abstraction. The harness depends on Rig's single-turn `CompletionModel` completion API and provider clients (Anthropic, OpenAI-compatible, and 20+ others). **bee-harness owns the multi-turn agent loop itself** rather than delegating to Rig's `Agent::multi_turn`, so turn/time limits, per-tool-call audit correlation, output truncation, and malformed-arg handling stay under bee's explicit control. Breaking changes in Rig may require adaptation; the loop-ownership choice limits Rig's surface area to the completion call, reducing that exposure.
- The harness introduces a tokio dependency (via Rig and the async audit layer). This dependency is confined to `bee-harness` and the feature-gated `tokio` path in `bee-userspace`; `bee-core` remains sync and runtime-free per the constitution.
- The async audit reader uses `tokio::io::unix::AsyncFd` on the ring buffer's file descriptor. This is Linux-only, which is already the target platform.
- Scenarios for the adversarial/CTF mode are authored manually for the initial release. Procedural scenario generation (automated policy mutation, difficulty scaling) is a follow-on feature.
- Scoring beyond binary flag capture (coverage, novelty, parsimony) requires post-episode analysis of the audit trail. The initial release provides the audit data and a scoring API; sophisticated scoring models (technique embedding, novelty detection) are follow-on work.
- The harness runs on the same host as the bee enforcement engine (it calls `Engine::init` and `create_scope` directly). Remote/distributed episode execution is out of scope.
- API keys for LLM providers are supplied via environment variables or a configuration file, not managed by the harness.
