# Tasks: MCP Client Integration with Sandboxed Transport

**Input**: Design documents from `/specs/004-mcp-client/`

**Prerequisites**: [plan.md](./plan.md), [spec.md](./spec.md), [research.md](./research.md),
[data-model.md](./data-model.md), [contracts/](./contracts/), [quickstart.md](./quickstart.md)

**Tests**: INCLUDED. The bee constitution (Development Workflow & Quality Gates) mandates test-first
for enforcement behavior and **property-based** tests for security-relevant matching. Domain-pattern
matching, stdio kernel-denial correlation, and credential non-leakage therefore have failing tests
written before implementation.

**Organization**: By user story. US8 (stdio, P2) is the MVP; US9 (remote gating, P3) and US10 (REPL,
P3) build on the shared foundation.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies on incomplete tasks)
- **[Story]**: US8 / US9 / US10 (maps to spec user stories); Setup/Foundational/Polish carry no label
- All MCP code is in `bee-harness/` behind the default-off `mcp` Cargo feature (research R11)

## Path Conventions

Single Rust workspace. New code: `bee-harness/src/mcp/` + `bee-harness/src/mcp.rs`. Integration tests:
`bee-harness/tests/`. Touch-points: `bee-harness/src/{tools,scenario,config,episode,sandbox,repl}.rs`,
`bee-harness/src/repl/terminal.rs`, `bee-harness/Cargo.toml`.

---

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Add the feature-gated dependency and module skeleton so nothing compiles into default builds.

- [x] T001 Add a default-off `mcp` feature and the feature-gated `rmcp = { version = "2.2", default-features = false, features = ["client", "transport-child-process"], optional = true }` dependency to `bee-harness/Cargo.toml` (research R1); gate `mcp` on `dep:rmcp`. **Done**: verified default build stays rmcp-free, `cargo tree -p bee-core -i rmcp` finds nothing (SC-026), `--features mcp` compiles against rmcp 2.2.0.
- [x] T002 [P] Create the module skeleton with `#[cfg(feature = "mcp")]` glue: `bee-harness/src/mcp.rs` (re-exports + `McpBridge::connect` entry) and empty `bee-harness/src/mcp/{config,policy,bridge,transport,proxy}.rs`; register `pub mod mcp;` in `bee-harness/src/lib.rs` behind the feature.
- [x] T003 [P] Add the fail-closed guard: when a scenario has `[mcp].enabled = true` but the binary was built without `--features mcp`, return a clear startup error ("MCP configured but bee-harness built without --features mcp") in `bee-harness/src/episode.rs` and the binaries (research R11, Constitution I).
- [x] T004 [P] Add the SC-026 dependency-boundary assertion — a CI/test check that `cargo tree -p bee-core --no-default-features | grep rmcp` returns nothing — in `bee-harness/tests/mcp_dep_boundary.rs` (or a CI script).

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Config parsing, the shared proxy/bridge scaffolding, and credential plumbing that every
user story depends on.

**⚠️ CRITICAL**: No user-story work begins until this phase is complete.

- [x] T005 [P] Implement `McpTransport` enum (`Stdio` | `StreamableHttp` | deprecated `Sse` alias, `#[serde(rename_all = "snake_case")]`) and `McpServerConfig` struct with serde defaults in `bee-harness/src/mcp/config.rs` (data-model.md §Config).
- [x] T006 [P] Implement `DomainPattern` **parsing only** (`TryFrom<String>`: `*.suffix` → `Wildcard`, bare `*`/`*foo` → error, else `Exact` lowercased/trailing-dot-stripped) in `bee-harness/src/mcp/policy.rs`, with unit tests for parse/reject. (Matching logic is deferred to US9/T026.)
- [x] T007 Implement `McpPolicy` struct (`enabled`, `allowed_domains`, `denied_domains`, `max_servers`=5, `tool_timeout_secs`=30, `allow_dynamic_connect`=false, `servers: Vec<McpServerConfig>`) with `#[serde(default)]` on every field in `bee-harness/src/mcp/policy.rs` (data-model.md §McpPolicy). Depends on T005, T006.
- [x] T008 Wire `[mcp]` into the scenario: add `#[serde(default)] mcp: Option<McpPolicy>` to the private `ScenarioFile` wrapper and a `mcp: McpPolicy` field to `Scenario`; in `Scenario::from_path` set `s.mcp = file.mcp.unwrap_or_default()`; add MCP semantic checks (unique server names, transport-appropriate fields, `max_servers>=1`, valid domains) to `Scenario::validate` in `bee-harness/src/scenario.rs` (research R10, data-model.md validation). Depends on T007.
- [x] T009 [P] Implement the rmcp→bee result mapping helper `call_tool_result_to_tool_result(CallToolResult, cap) -> ToolResult` in `bee-harness/src/mcp/proxy.rs`: flatten text `ContentBlock`s into `content`, set `is_error = res.is_error.unwrap_or(false)`, then apply `transcript::truncate` (research R6, SC-024). Unit-test with a synthetic `CallToolResult`.
- [x] T010 Implement `McpToolProxy` (`impl Tool` with `#[async_trait]`) in `bee-harness/src/mcp/proxy.rs`: `name()` returns a `Box::leak`ed `mcp__{server}__{tool}`; `schema()` returns the cached `ToolSchema`; `call()` validates args are a JSON object, wraps `peer.call_tool(CallToolRequestParams{..})` in `tokio::time::timeout(self.timeout)`, maps success via T009, maps elapse → `"timed out"`, maps dead-peer `ServiceError` → `"disconnected"` (research R6/R8, data-model.md §McpToolProxy). Depends on T009.
- [x] T011 Implement the `McpBridge` container in `bee-harness/src/mcp/bridge.rs`: `servers: BTreeMap<String, ConnectedServer>`, `ServerStatus` enum, `register_into(&self, &mut ToolRegistry)` (apply `allowed_tools`/`denied_tools`, insert one `McpToolProxy` per surviving tool), and `teardown(self)` (drop services → kill children). `connect()` is stubbed here and filled per-transport in US8/US9. Depends on T010.
- [x] T012 Implement `token_env` credential plumbing: read the named env var once at startup (`std::env::var(...).unwrap_or_default()`), and extend `sandbox::key_vars` (or the strip-list construction in `episode.rs`/`bin/bee-repl.rs`) to add every configured `token_env` name so it is stripped from stdio children in `bee-harness/src/sandbox.rs` (research R9, FR-041). Depends on T008.

**Checkpoint**: Config parses, proxies/bridge scaffold exists, credentials strip — user stories can begin.

---

## Phase 3: User Story 8 - Stdio MCP Server in Sandbox (Priority: P2) 🎯 MVP

**Goal**: Spawn a stdio MCP server as a sandboxed child (scope cgroup + env-strip + hardening),
expose its tools as `mcp__{name}__{tool}`, and correlate its kernel denials into the transcript.

**Independent Test**: A `MockModel` episode calls `mcp__filesystem__read_file` on a policy-denied path;
the server gets `EACCES`, the `ToolResult` is `is_error`, and the `file_open` denial appears in that
call's audit (spec US8 Independent Test / SC-020).

### Tests for User Story 8 (write first, ensure they FAIL) ⚠️

- [x] T013 [P] [US8] Failing integration test in `bee-harness/tests/mcp_stdio.rs` (`--features enforce,mcp`): MockModel calls `mcp__filesystem__read_file` with `/secrets/key.pem`; assert `result.is_error == true` and the `RecordedCall.audit` contains a `file_open`/`denied` event for that path (SC-020, SC-023, US8 AS-1).
- [x] T014 [P] [US8] Failing test in `bee-harness/tests/mcp_stdio.rs`: server configured with `denied_tools = ["write_file"]` ⇒ `registry.schemas()` excludes `mcp__filesystem__write_file` (US8 AS-3, FR-039).
- [x] T015 [P] [US8] Failing test in `bee-harness/tests/mcp_stdio.rs`: a permissive `/workspace` read succeeds (`is_error == false`) with no denied audit events (US8 AS-2); plus a truncation case (>100 KB ⇒ `truncated`, `original_len`) and a per-call timeout case returning `"timed out"` (SC-024, SC-025).

### Implementation for User Story 8

- [x] T016 [US8] **R3 cgroup-join spike** (Constitution III gate) in `bee-harness/tests/mcp_cgroup_spike.rs` (`--features enforce,mcp`): spawn via `TokioChildProcess::new(tokio_cmd_from(sandbox.tool_command(...)))` and assert the child's cgroup equals the scope cgroup (read `/proc/<pid>/cgroup` or assert a scope-filtered denial). Record the outcome; **it selects T017's path** (contracts/sandbox-transport.md §3.2). **PASSED on ac-matrix-vm**: rmcp TokioChildProcess child joins the scope cgroup (== control); process-wrap preserves pre_exec. Primary path confirmed; raw-pipe fallback unneeded.
- [x] T017 [US8] Implement the stdio transport in `bee-harness/src/mcp/transport.rs`: build `std::process::Command` via `Sandbox::tool_command`, apply `config.env`, convert to `tokio::process::Command`. **If T016 passed** use `TokioChildProcess::new(cmd)?` (§3.1); **else** the raw-pipe fallback (spawn with piped stdio + `kill_on_drop`, hand `(child.stdout, child.stdin)` to serve — §3.3). Wrap `initialize` in a 10 s `tokio::time::timeout` (NFR-005). Depends on T016.
- [x] T018 [US8] Implement `McpBridge::connect` stdio branch in `bee-harness/src/mcp/bridge.rs`: for each stdio server (respecting `max_servers`), spawn+`().serve()` via T017, `list_all_tools()`, cache filtered `ToolSchema`s, set `ConnectedServer.status`; a spawn/init failure ⇒ `Failed`, tools unregistered, non-fatal. Depends on T017, T011.
- [x] T019 [US8] Wire the bridge into `bee-harness/src/episode.rs` `run_episode`: after `registry_for(...)` and sandbox construction, `let bridge = McpBridge::connect(scenario.mcp.clone(), &sandbox).await;` then `bridge.register_into(&mut registry);`; hold `bridge` for the episode and `bridge.teardown()` at the end (research R5). Depends on T018.
- [x] T020 [US8] Implement crash handling in `bee-harness/src/mcp/{bridge,proxy}.rs`: a child exit / dead peer flips `ServerStatus` to `Disconnected`; in-flight call ⇒ error result; later calls ⇒ `ToolResult::error("MCP server '{name}' disconnected")`; episode continues (FR-042, US8 AS-4). Add a failing→passing test in `bee-harness/tests/mcp_stdio.rs` that SIGKILLs the child mid-episode. Depends on T019.

**Checkpoint**: MVP — a sandboxed stdio MCP server works end-to-end; T013–T015, T020 pass under `enforce`.

---

## Phase 4: User Story 9 - Remote MCP Server with Domain Gating (Priority: P3)

**Goal**: Connect to remote Streamable-HTTP MCP servers only when the host matches the allowlist;
refuse everything else before any protocol message.

**Independent Test**: Two remote servers configured — one allowed host, one not. The first connects
and exposes `mcp__*` tools; the second is refused with a recorded reason and no MCP messages (spec US9
Independent Test / SC-021, SC-022).

### Tests for User Story 9 (write first, ensure they FAIL) ⚠️

- [x] T021 [P] [US9] **Property test** for `DomainPattern::matches` in `bee-harness/src/mcp/policy.rs` (`#[cfg(test)] proptest`): wildcard never matches the bare apex, never matches a suffix-substring (`evilcompany.com` vs `*.company.com`), always matches deeper subdomains; case-insensitive (Constitution: security matching is property-tested).
- [x] T022 [P] [US9] Domain-resolution matrix test in `bee-harness/tests/mcp_domain_gating.rs` encoding contracts/mcp-policy.md §3's 7-row table (denied wins, empty allowlist denies, wildcard ≠ apex) (SC-021).
- [x] T023 [P] [US9] Refusal + dynamic-connect tests in `bee-harness/tests/mcp_domain_gating.rs`: a non-allowed host is refused, the transcript records "connection refused by MCP policy", and no transport is opened (assert no socket write); `allow_dynamic_connect = false` refuses an undeclared server (SC-021, FR-040); a `token_env` value never appears in the transcript/logs (SC-028 remote).

### Implementation for User Story 9

- [x] T024 [US9] Add the `transport-streamable-http-client-reqwest` rmcp feature to the `mcp` feature set in `bee-harness/Cargo.toml`; implement the `transport = "sse"` deprecated-alias warning routing to the Streamable-HTTP client (research R2).
- [x] T025 [US9] Implement `DomainPattern::matches` and `resolve(host, &McpPolicy) -> Decision` (denied → allowed → deny-by-default) in `bee-harness/src/mcp/policy.rs`; parse the URL host with port/scheme/path stripped (research R6, mcp-policy.md §3). Depends on T006.
- [x] T026 [US9] Implement `McpBridge::connect` remote branch in `bee-harness/src/mcp/bridge.rs`: resolve the domain **first** (refuse + record, no transport on deny), else open the Streamable-HTTP client with the `token_env` Bearer header under a 15 s timeout, `list_all_tools()`, cache filtered schemas; enforce `max_servers` and `allow_dynamic_connect` (FR-035, FR-040, FR-041, NFR-005). Depends on T025, T011, T024.

**Checkpoint**: US8 and US9 both work independently; domain matrix + refusal tests pass.

---

## Phase 5: User Story 10 - MCP in the REPL (Priority: P3)

**Goal**: Configure MCP in a REPL session; `/tools` lists MCP tools; `/mcp` shows servers + policy.

**Independent Test**: In a REPL with an MCP server configured, `/tools` lists `mcp__*` alongside
built-ins and `/mcp` shows each server's transport, status, and the domain allowlist (spec US10 AS-1/2).

### Tests for User Story 10 (write first, ensure they FAIL) ⚠️

- [ ] T027 [P] [US10] Failing REPL test in `bee-harness/tests/mcp_repl.rs` using the `Collector` output impl: `/tools` output includes `mcp__filesystem__read_file`; `/mcp` output includes the server name, transport, status, and allowlist (US10 AS-1, AS-2).

### Implementation for User Story 10

- [ ] T028 [US10] Add the REPL MCP config surface: an `--mcp-config <path>` arg (and/or reuse the scenario `[mcp]`) parsed in `bee-harness/src/bin/bee-repl.rs`, connecting the bridge after `registry_for(...)` and registering proxies into the shared registry (mirrors T019).
- [ ] T029 [US10] Add the `/mcp` meta-command in `bee-harness/src/repl.rs`: a `MetaCommand::Mcp` variant, a `"/mcp"` parser arm, a dispatch arm calling a new `mcp_summary(&McpBridge) -> String`, and a `/mcp` line in `HELP_TEXT`; render via `output.info(&str)` (plain multi-line, per contracts + research). Depends on T028.
- [ ] T030 [US10] Confirm/extend `/tools`: since MCP proxies live in the shared `ToolRegistry`, `tools_summary` already lists them — add a coverage assertion and, if desired, annotate MCP rows with their server prefix in `bee-harness/src/repl.rs`. Depends on T028.

**Checkpoint**: All three user stories independently functional.

---

## Phase 6: Polish & Cross-Cutting Concerns

- [ ] T031 [P] Implement `tools/list_changed` (FR-043): replace the `()` client handler with an `rmcp::ClientHandler` impl whose `on_tool_list_changed` refreshes the cached schemas for the next turn, in `bee-harness/src/mcp/bridge.rs`. Add a test with a mock server emitting the notification.
- [ ] T032 [P] Add the `*.policy.toml` companions for the example scenarios (`bee-harness` policy format, deny `/secrets`, allow `/workspace`) in `specs/004-mcp-client/examples/{stdio-filesystem,remote-github,mixed}.policy.toml`.
- [ ] T033 [P] Update `specs/004-mcp-client/spec.md` to record the FR-033 amendment (SSE removed in rmcp 2.x → Streamable-HTTP only; `sse` = deprecated alias), cross-referencing research.md R2.
- [ ] T034 SC-027 regression test in `bee-harness/tests/mcp_stdio.rs` (or a dedicated file): a scenario with no `[mcp]` section produces a transcript shape identical to the pre-feature path, and the default (no-`mcp`) build test suite is unchanged.
- [ ] T035 Run the [quickstart.md](./quickstart.md) validation gates 0–7; fix any gaps and record results.

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: no dependencies — start immediately.
- **Foundational (Phase 2)**: depends on Setup — **blocks all user stories**.
- **US8 (Phase 3)**: depends on Foundational. MVP. The R3 spike (T016) gates the transport code (T017).
- **US9 (Phase 4)**: depends on Foundational. Independent of US8 (does not need the spike or stdio code).
- **US10 (Phase 5)**: depends on Foundational + at least one connectable transport (US8 or US9) to show tools/servers.
- **Polish (Phase 6)**: depends on the user stories it touches.

### Critical path (MVP)

`T001 → T002 → (T005,T006) → T007 → T008 → (T009 → T010) → T011 → T013–T015 (failing) → T016 spike → T017 → T018 → T019 → T020` ⇒ US8 done.

### Within each user story

- Tests (T013–T015, T021–T023, T027) are written and MUST FAIL before their implementation tasks.
- Config/types before bridge; bridge before episode wiring; connect before crash-handling.

### Parallel Opportunities

- Setup: T002, T003, T004 in parallel after T001.
- Foundational: T005 ∥ T006 (different files); T009 is [P]. T007→T008 and T010→T011 are sequential chains.
- US8 tests T013 ∥ T014 ∥ T015 (same file, different fns — stage together, but they share `mcp_stdio.rs`, so treat as one edit unit if a single author).
- US9 tests T021 ∥ T022 ∥ T023 (T021 is in-crate proptest, T022/T023 in the tests file).
- US9 and US10 can proceed in parallel with US8 once Foundational is done (different files), except T028/T029/T030 want a live transport for their tests.
- Polish T031 ∥ T032 ∥ T033.

---

## Parallel Example: Foundational

```bash
# After T001 (Cargo feature) lands, run the independent scaffolding tasks together:
Task: "T002 Create mcp module skeleton in bee-harness/src/mcp{,.rs}"
Task: "T003 Fail-closed guard for [mcp] without --features mcp in episode.rs"
Task: "T004 SC-026 dep-boundary check in tests/mcp_dep_boundary.rs"

# Config types in parallel (different files):
Task: "T005 McpTransport + McpServerConfig in mcp/config.rs"
Task: "T006 DomainPattern parsing in mcp/policy.rs"
```

---

## Implementation Strategy

### MVP First (User Story 8 only)

1. Phase 1 Setup.
2. Phase 2 Foundational (blocks everything).
3. Phase 3 US8 — **run the R3 spike (T016) before writing transport code (T017)**; it decides the
   `TokioChildProcess` vs raw-pipe path (Constitution III gate).
4. STOP & VALIDATE: quickstart Gate 1 under `--features enforce,mcp` — the kernel denial must appear
   in the transcript.
5. Demo the sandboxed stdio MCP server.

### Incremental Delivery

1. Setup + Foundational → foundation ready.
2. US8 → validate → demo (MVP: sandboxed stdio MCP).
3. US9 → validate domain matrix → demo (remote gating).
4. US10 → validate REPL surfaces → demo.
5. Polish: `list_changed`, example policies, SSE amendment, SC-027 regression, full quickstart.

---

## Notes

- All MCP code sits behind `--features mcp`; the default build must stay `rmcp`-free (SC-026, SC-027).
- The R3 spike (T016) is the one true unknown — everything downstream of the transport assumes its
  outcome. Do it first in US8; do not let transport code land before it (Constitution III, fail-closed).
- `[P]` = different files, no incomplete dependencies. Tasks touching the same file (e.g. several in
  `mcp_stdio.rs` or `bridge.rs`) are sequential for a single author.
- Commit after each task or logical group; keep no-MCP scenarios byte-identical at every step.
