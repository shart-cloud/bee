# Implementation Plan: MCP Client Integration with Sandboxed Transport

**Branch**: `004-mcp-client` | **Date**: 2026-07-20 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/004-mcp-client/spec.md`

## Summary

Add an MCP (Model Context Protocol) **client** to `bee-harness` so an agent can call tools hosted by
external MCP servers, without MCP becoming an exfiltration path. Two enforcement models, mirroring the
spec's threat model:

- **Stdio servers are sandboxed children** — spawned via `Sandbox::tool_command`, so they join the
  episode's scope cgroup and the kernel (eBPF-LSM) enforces the policy on their I/O. Their denials
  flow through the existing audit drain (Constitution III).
- **Remote servers are domain-gated** — a Streamable-HTTP connection is refused at construction time
  unless the host matches the scenario's allowlist (deny-by-default, Constitution I).

Built on the official `rmcp` crate (**v2.2**, corrected from the spec's stale `"1"`), confined to
`bee-harness` behind a default-off `mcp` Cargo feature. MCP tools are registered into the existing
`ToolRegistry` as `Box<dyn Tool>` proxies named `mcp__{server}__{tool}`, so the agent loop, schema
advertisement, truncation, timeout, and transcript recording all work **unchanged**. Full technical
grounding in [research.md](./research.md).

## Technical Context

**Language/Version**: Rust, stable 1.85 (workspace `rust-version`), edition 2021. User-space crates
build on stable (Principle V); no nightly for this feature.

**Primary Dependencies**: `rmcp = { version = "2.2", default-features = false, features = ["client",
"transport-child-process"(, "transport-streamable-http-client-reqwest")] }` (new, `bee-harness`
only, behind `mcp` feature). Existing: `tokio`, `serde`/`serde_json`, `toml`, `rig-core` (provider
seam), `async-trait`.

**Storage**: N/A — MCP state is in-memory (`McpBridge`) for the episode/REPL lifetime. Config is TOML.

**Testing**: `cargo test` (unit + integration in `bee-harness/tests`), `MockModel` for deterministic
episodes, `proptest` for domain-pattern matching (Principle: security behavior is property-tested).
The cgroup-join spike (R3) requires a BPF-LSM VM under `--features enforce`.

**Target Platform**: Linux + eBPF-LSM + cgroup v2 (for enforced stdio); the default (non-enforce)
build runs MCP servers as hardened, credential-stripped host processes (no scope) for offline tests.

**Project Type**: Single Rust workspace; this feature is one crate (`bee-harness`) + a new `mcp`
module.

**Performance Goals**: Stdio connect (spawn+initialize) < **10 s**; remote connect (TCP+TLS+init) <
**15 s** (NFR-005, configurable). Per-tool-call timeout = `tool_timeout_secs` (default 30 s).

**Constraints**: `rmcp` MUST NOT propagate to `bee-core`/`bee-common` (NFR-004/SC-026). No-MCP
scenarios MUST behave identically to today (NFR-006/SC-027). Credentials never logged/transcribed
(FR-041/SC-028). TLS validation never weakened.

**Scale/Scope**: `max_servers` (default 5) simultaneous servers; a handful of tools each. Tool-call
volume is the agent's turn budget — small.

## Constitution Check

*GATE: evaluated pre-Phase-0 and re-checked post-Phase-1 design. Principles I–V + Security/Platform.*

| Principle | Verdict | Basis |
|-----------|---------|-------|
| **I. Deny-by-Default & Fail-Closed** | **PASS** | Remote refused unless host in allowlist; empty allowlist ⇒ no remote; `allow_dynamic_connect` default `false`; a stdio server that fails to spawn/initialize in time is `Failed` (its tools absent — capability denied, not silently granted); MCP-configured-but-built-without-`mcp`-feature ⇒ **startup error**, never silent ignore (R11). |
| **II. Capability Attenuation** | **PASS (deferred)** | No subagent spawns its own MCP client this feature. Design doesn't preclude a future derived scope inheriting/attenuating `McpPolicy` via `derive_scope`. No new attenuation surface introduced. |
| **III. Kernel Enforcement Is Authoritative** | **PASS — conditional on R3 spike** | Stdio server runs in the scope cgroup; kernel decides; harness only reports (audit drain). **Hard gate**: the `pre_exec` cgroup-join must survive the rmcp/`process-wrap` spawn chain. If the spike can't prove the child joined the scope, ship the raw-pipe fallback (R4); if *neither* can prove it, **do not expose stdio MCP under enforce** (fail closed). Remote servers are explicitly documented as un-sandboxable — the domain gate is a network control, not a semantic enforcement point. |
| **IV. Policy-as-Data** | **PASS** | `[mcp]` / `[[mcp.servers]]` is declarative TOML; allow/deny domain lists are data; no imperative policy. |
| **V. Library-First, Runtime-Free Core** | **PASS** | `rmcp` confined to `bee-harness` behind `mcp` feature; `bee-core`/`bee-common` gain nothing (SC-026). The async requirement lives in the harness (already async via `tokio`/`rig-core`), not the sync core. |
| **Security/Platform** | **PASS** | `token_env` follows the FR-018 env-name convention and is stripped from stdio children (SC-028); TLS validation unweakened (rustls default); every MCP call + denial recorded in the transcript (FR-037). |

**Result: no unjustified violations.** One conditional gate (III, R3) is discharged by a Phase-0
spike that selects the transport path. Spec deviations (rmcp v2 / SSE removal / `is_known_tool`) are
non-violations, tracked in Complexity Tracking below.

## Project Structure

### Documentation (this feature)

```text
specs/004-mcp-client/
├── spec.md              # Feature spec (already committed)
├── plan.md              # This file
├── research.md          # Phase 0 — decisions R1–R12
├── data-model.md        # Phase 1 — entities, states, validation
├── quickstart.md        # Phase 1 — runnable validation guide
├── contracts/           # Phase 1 — refined (mcp-policy, mcp-tool-naming, sandbox-transport)
│   ├── mcp-policy.md
│   ├── mcp-tool-naming.md
│   └── sandbox-transport.md
├── examples/            # stdio-filesystem / remote-github / mixed scenario TOMLs
└── tasks.md             # Phase 2 — created by /speckit-tasks (NOT this command)
```

### Source Code (repository root)

```text
bee-harness/
├── Cargo.toml                 # + `mcp` feature; + rmcp dep (feature-gated)
└── src/
    ├── mcp.rs                 # module root: re-exports; McpBridge::connect entry; feature-gate glue
    ├── mcp/
    │   ├── config.rs          # McpServerConfig, McpTransport, TOML shapes
    │   ├── policy.rs          # McpPolicy, DomainPattern (exact + wildcard), resolution (proptest)
    │   ├── bridge.rs          # McpBridge: connect/register_into/teardown; ConnectedServer; status
    │   ├── transport.rs       # stdio spawn via Sandbox::tool_command (+ raw-pipe fallback, R4)
    │   └── proxy.rs           # McpToolProxy: impl Tool; rmcp CallToolResult → ToolResult
    ├── tools.rs               # (unchanged public API) registry_for stays sync; see note
    ├── scenario.rs            # ScenarioFile wrapper gains `mcp`; Scenario gains `mcp`; validate()
    ├── config.rs              # (REPL) MCP config surface; token_env read pattern
    ├── episode.rs             # run_episode: connect bridge post-registry; own for lifetime; teardown
    ├── sandbox.rs             # key_vars extended to strip configured token_env names
    ├── repl.rs                # /mcp meta-command; /tools already lists registry (proxies included)
    └── repl/terminal.rs       # /mcp rendering via output.info (plain multi-line string)

bee-harness/tests/
├── mcp_stdio.rs               # US8: MockModel + sandboxed filesystem server; denial in audit
├── mcp_domain_gating.rs       # US9: allow/deny/wildcard matrix; refusal recorded (no MCP messages)
└── mcp_repl.rs                # US10: /tools shows mcp__*, /mcp shows servers+policy
```

**Structure Decision**: Single-crate feature. A new `bee-harness/src/mcp/` module holds all MCP code
behind the `mcp` feature; the touch-points in existing files are minimal and additive
(`registry_for` stays byte-compatible for no-MCP scenarios). This matches the 003 precedent
(`RenderTool` as an in-process `Box<dyn Tool>` that ignores the sandbox) and keeps the agent loop and
transcript schema unchanged.

## Implementation Slices (delivery order)

Priority-ordered per the spec's user stories; each slice is independently testable.

**Slice 1 — Stdio MCP servers in the sandbox (P2, US8).** The security-critical core.
Deliverables: `mcp` Cargo feature + rmcp dep; `config.rs`/`policy.rs` (stdio fields + `enabled` +
tool filtering + `max_servers`/`tool_timeout_secs`); `McpBridge::connect` (stdio only);
`transport.rs` stdio spawn; **R3 spike first**; `McpToolProxy` + rmcp→`ToolResult` mapping (incl.
truncation); `run_episode` wiring + teardown; `token_env`/strip plumbing scaffolding.
Verifies: SC-020, SC-023, SC-024, SC-025, SC-028(stdio), FR-034/036/037/038/039/042.

**Slice 2 — Remote servers with domain gating (P3, US9).** Adds
`transport-streamable-http-client-reqwest`; `DomainPattern` matching (proptest) + `McpPolicy`
resolution (denied→allowed→default); remote connect + `token_env` Bearer auth + refusal recording;
`sse`→streamable alias warning; `allow_dynamic_connect` enforcement.
Verifies: SC-021, SC-022, SC-028(remote), FR-035/040/041, mcp-policy.md test matrix.

**Slice 3 — MCP in the REPL (P3, US10).** REPL MCP config surface; `/mcp` meta-command; confirm
`/tools` lists `mcp__*` (free, since proxies are in the shared registry); help text.
Verifies: US10 AS-1/AS-2.

`tools/list_changed` (FR-043) is a small add-on to Slice 1/2 via an `rmcp::ClientHandler` with
`on_tool_list_changed` refreshing the cached schemas for the next turn.

## Complexity Tracking

> Deviations from the spec. None are Constitution violations; each is a correction or simplification
> with rationale. No entry here justifies added complexity — all three *reduce* it.

| Spec item | Change | Why |
|-----------|--------|-----|
| `rmcp = "1"`, features incl. `transport-sse-client` | Use `rmcp = "2.2"`; drop SSE-client feature; remote = Streamable-HTTP only; `transport = "sse"` = warned alias | rmcp 1.x is a stale major with a different API; 2.x **removed** the SSE client. Verified against crates.io/docs.rs (R1, R2). |
| Spec Option A `SandboxedChildTransport` custom struct | No custom transport type: build the `Command` via `tool_command`, hand to `TokioChildProcess::new` (raw-pipe fallback only if the R3 spike fails) | rmcp already accepts a pre-built `tokio::process::Command`; a wrapper struct is unnecessary indirection (R3/R4). |
| `is_known_tool` "extended for mcp__ prefix"; `registry_for` "gains MCP path" | Neither changes: `registry_for` stays sync/unchanged; `is_known_tool` unchanged; MCP proxies are inserted post-registry by `McpBridge::register_into` | `is_known_tool` validates only declared built-ins; MCP names never pass through it, and dispatch is a registry `BTreeMap` lookup. Adding cases = dead code (R5/R12). |
| (new) opt-in only via absent `[mcp]` | Add a default-off `mcp` **Cargo** feature; fail-closed if configured but built without it | Strengthens NFR-006/SC-027 to compile-time; matches `enforce`/`concurrent` discipline; keeps `rmcp` out of default builds (R11). |

## Phase 0 — Outline & Research

**Status: COMPLETE.** See [research.md](./research.md). All NEEDS CLARIFICATION resolved (R1–R12).
The one item carried into implementation is the **R3 cgroup-join spike**, sequenced first in Slice 1,
with the R4 raw-pipe fallback pre-designed so its outcome cannot block the slice.

## Phase 1 — Design & Contracts

**Status: COMPLETE.**
- Entities, states, and validation rules → [data-model.md](./data-model.md).
- Interface contracts (refined this phase) → [contracts/mcp-policy.md](./contracts/mcp-policy.md),
  [contracts/mcp-tool-naming.md](./contracts/mcp-tool-naming.md),
  [contracts/sandbox-transport.md](./contracts/sandbox-transport.md).
- Runnable validation guide → [quickstart.md](./quickstart.md).

**Post-design Constitution re-check: PASS** (table above reflects the post-design state; the only
open gate is the R3 spike, which is a test, not a design flaw).

## Next Command

`/speckit-tasks` — generate the dependency-ordered `tasks.md` from these artifacts (Phase 2). Begin
Slice 1 with the R3 spike (T-spike-cgroup) before any transport code lands.
