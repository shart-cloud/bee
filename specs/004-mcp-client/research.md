# Phase 0 Research: MCP Client Integration

**Feature**: `004-mcp-client` | **Date**: 2026-07-20 | **Spec**: [spec.md](./spec.md)

This document resolves the technical unknowns behind the plan. Each entry: **Decision**,
**Rationale**, **Alternatives considered**. Findings are grounded in the current `bee-harness`
source and in the `rmcp` crate as published on crates.io (verified v2.2.0, 2026-07-08).

---

## R1. rmcp crate version and Cargo features  — SPEC ASSUMPTIONS CORRECTED

**Decision**: Depend on **`rmcp = { version = "2.2", default-features = false, features = ["client",
"transport-child-process"] }`** for the stdio slice. Add
`"transport-streamable-http-client-reqwest"` only when the remote slice is built.

**Rationale**:
- The spec's `rmcp = "1"` is stale. The current major is **2.x** (latest 2.2.0). `^1` caps at 1.8.0
  and never resolves to 2.x — and 2.x renamed core types (see R6), so 1.x snippets won't compile.
- `default-features` includes `server`, **not** `client`. A client build MUST set
  `default-features = false` and add `client` explicitly, or the client API is absent.
- `transport-child-process` (stdio) is a correct feature name. The stdio path pulls **no reqwest and
  no TLS** — a pure stdio client has zero HTTP surface.

**Alternatives considered**: Pinning `rmcp = "1.8"` — rejected: an unmaintained older major with a
different API. Building our own MCP JSON-RPC client — rejected: FR-033 mandates the official SDK.

---

## R2. Remote transport: SSE is gone in rmcp 2.x  — SPEC DEVIATION

**Decision**: Remote MCP support is **Streamable HTTP only**
(`transport-streamable-http-client-reqwest`). The scenario config value `transport = "sse"` is
accepted as a **deprecated alias** that routes to the Streamable-HTTP client and logs a warning;
`transport = "streamable_http"` is the canonical value. FR-033's "SSE … transport" is amended
accordingly.

**Rationale**: rmcp 2.x **removed the SSE client transport** — there is no `transport-sse-client`
feature and no `Sse*` client type. The MCP spec itself deprecated HTTP+SSE in favor of Streamable
HTTP, and rmcp followed. Most endpoints described as "SSE" now serve Streamable HTTP, which uses
SSE-style streaming responses under the hood, so routing the alias there is the pragmatic behavior.

**Alternatives considered**: (a) Implement a bespoke SSE client to honor `transport = "sse"` literally
— rejected: reintroduces the maintenance burden FR-033 exists to avoid. (b) Hard-reject `sse` at
config parse — rejected as needlessly hostile to existing configs; a warned alias is friendlier and
still deterministic. This deviation is logged in plan.md Complexity Tracking.

---

## R3. Sandboxed stdio transport: the spawn seam  — CONSTITUTION III GATE

**Decision**: Build the child with **`Sandbox::tool_command(program, &args)`** (returns
`std::process::Command`), convert to tokio via `tokio::process::Command::from(std_cmd)`, apply
`config.env`, then hand it to **`TokioChildProcess::new(cmd)?`** and `().serve(child_process).await`.
A **Phase-0 spike (T-spike-cgroup)** MUST verify the `pre_exec` cgroup-join closure survives the
`TokioChildProcess` → `process-wrap::CommandWrap` → spawn chain under `--features enforce`. If it does
not, fall back to the **raw-pipe transport** (R4).

**Rationale**:
- Research confirmed `TokioChildProcess::new(command: impl Into<CommandWrap>)` is a **public,
  constructable** function that accepts a pre-built `tokio::process::Command`. So bee owns program,
  args, env, cwd, and — critically — the `pre_exec` closure that `tool_command` installs to join the
  scope cgroup and run `pre_exec_hardening()`.
- **The gate**: `TokioChildProcess` wraps the command in `process-wrap`'s `CommandWrap`, which may
  install its **own** `pre_exec` for process-group management. If that overrides or precedes bee's
  cgroup-join closure, the MCP server would run **outside** the scope — a silent Constitution III
  violation (the kernel would not be the boundary). This is not acceptable to assume; it must be
  proven by test. Fail-closed (Principle I): if the spike cannot prove the child is in the scope
  cgroup, bee MUST NOT expose that server.

**Alternatives considered**: `ConfigureCommandExt::configure(|c| …)` — ergonomic only; equivalent to
building the Command directly, and doesn't address the process-wrap `pre_exec` question. Option B from
the spec (rmcp pre-spawn hook) — rejected for the same reason: it keeps spawn control inside rmcp.

---

## R4. Fallback stdio transport: raw pipes (if R3 spike fails)

**Decision**: If the spike shows `process-wrap` clobbers bee's `pre_exec`, spawn the child ourselves
— `tokio::process::Command::from(sandbox.tool_command(...)?)` with `.stdin(piped).stdout(piped)
.stderr(null).kill_on_drop(true)`, `.spawn()?` — take `child.stdout` + `child.stdin`, and pass the
`(ChildStdout, ChildStdin)` pair to `().serve(transport)` via rmcp's `IntoTransport` for an
async-read/async-write pair. bee retains the `tokio::process::Child` for kill-on-drop.

**Rationale**: This bypasses `process-wrap` entirely, so bee's `pre_exec` is the only one installed —
the cgroup join is guaranteed. It also gives bee direct ownership of the child handle for the
crash-handling and kill-on-drop requirements (FR-042). Cost: bee owns a little more transport
plumbing and must confirm the rmcp feature that enables the byte-pair transport
(`transport-io` / async-rw). The spike settles which path ships; the plan carries both so a failed
spike does not block the slice.

**Alternatives considered**: Patching `process-wrap` — out of scope and fragile.

---

## R5. Where MCP connects (async) vs. the sync registry

**Decision**: Keep `registry_for(enabled, flag)` **unchanged** (sync). Add an **async**
`McpBridge::connect(policy, &sandbox) -> McpBridge` called in `run_episode`/the REPL binary *after*
the base registry is built and the sandbox exists, then `bridge.register_into(&mut registry)` inserts
one `McpToolProxy` (`Box<dyn Tool>`) per surviving MCP tool. `run_episode` owns the `McpBridge` for
the episode's lifetime and drops it at teardown (killing the children).

**Rationale**:
- `registry_for` is synchronous and called from both `run_episode` and `bee-repl`. MCP connection is
  async (`serve().await`) and needs the constructed `Sandbox` (for `tool_command`) — neither is
  available inside `registry_for`. Doing it after keeps `registry_for` backward-compatible (SC-027:
  no-MCP scenarios are byte-for-byte unchanged) and keeps the connection in an async context that
  already has the sandbox.
- Ownership: the `McpBridge` holds each `RunningService` (which owns the child + background task).
  The `McpToolProxy` objects in the registry hold only a cheap `Peer<RoleClient>` clone (a handle).
  When the bridge drops at teardown, the services drop, the children die, and any late proxy call
  returns a disconnected error (FR-042) — exactly the required lifecycle.

**Alternatives considered**: Threading `McpServerConfig`s into `registry_for` and making it async —
rejected: forces `async` onto 4 sync call sites and every test that builds a registry, for no gain.

---

## R6. Mapping rmcp types to bee's Tool/ToolResult

**Decision**: `McpToolProxy` implements bee's existing `Tool` trait. Mapping:

| bee | rmcp 2.x | Notes |
|-----|----------|-------|
| `Tool::name() -> &'static str` | tool `name: Cow<'static,str>` | **Intern**: `Box::leak(format!("mcp__{server}__{tool}").into_boxed_str())` — once per tool at connect. |
| `ToolSchema { name, description, parameters }` | `Tool { name, description: Option<Cow>, input_schema: Arc<JsonObject> }` | `parameters = Value::Object((*input_schema).clone())`; description default "". |
| `Tool::call(args: Value, _sandbox)` | `Peer::call_tool(CallToolRequestParams { name, arguments: Some(map), .. })` | args must be a JSON **object** → `Map`; non-object → `ToolResult::invalid_args`. |
| `ToolResult { content, is_error, truncated, original_len }` | `CallToolResult { content: Vec<ContentBlock>, is_error: Option<bool>, structured_content }` | Flatten text `ContentBlock`s into `content`; `is_error = res.is_error.unwrap_or(false)`; then `transcript::truncate(content, cap)`. |

**Rationale**: The `name()` `&'static str` return is the one sharp edge — MCP names are runtime
`String`s, and the `ToolRegistry` keys on `&'static str`. A bounded, once-per-session
`Box::leak` mirrors how `RenderTool` is constructed once and lives for the session; leaked bytes are
proportional to the (small, fixed) MCP tool count, not to call volume. Content truncation is **not**
automatic for in-process tools (only the process path caps output), so the proxy calls
`transcript::truncate` / `tool_result_from_output` itself to satisfy FR-038/SC-024.

**Alternatives considered**: Extending the `Tool` trait to return `String` names — rejected: ripples
through the registry `BTreeMap<&'static str, …>` and every built-in tool for one caller's benefit.

---

## R7. Audit correlation for stdio servers is automatic

**Decision**: No new audit plumbing. Because the stdio server child joins the scope cgroup, and
`run_loop` already runs `sandbox.settle().await; sandbox.drain_audit()` after **every**
`registry.execute`, an MCP tool call that provokes a kernel denial in the server child yields
`AuditEvent`s drained into that call's `RecordedCall.audit`.

**Rationale**: The existing correlation (episode.rs) filters drained events by `scope.cgroup_id`; the
server child shares that cgroup, so its `file_open`/`connect` denials land in the same ring/channel.
By the time `call_tool().await` returns, the server has already attempted (and been denied) the I/O,
so the event is present at drain time. This directly satisfies FR-037/SC-020/SC-023 with zero changes
to the loop.

**Alternatives considered**: A dedicated per-proxy drain inside `call` — rejected: duplicates the
loop's drain and risks double-counting events into both the call and the episode `audit_trail`.

---

## R8. Timeout and crash handling

**Decision**: Wrap `peer.call_tool(...)` in `tokio::time::timeout(policy.tool_timeout)` inside
`McpToolProxy::call`; on elapse return `ToolResult::error("MCP tool '…': timed out")`. On any
`ServiceError` indicating a dead peer, return `ToolResult::error("MCP server '{name}' disconnected")`
and mark the `ConnectedServer` status `Disconnected`. Connection/startup uses
`tokio::time::timeout(10s stdio / 15s remote)` around `serve()`; on elapse the server is `Failed`,
its tools are never registered, and the episode continues.

**Rationale**: The episode loop's outer `tokio::time::timeout` is a *wall-clock* budget that only
drops the future — it does not kill a child. The proxy therefore owns a per-call timeout
(`tool_timeout_secs`, FR-038/SC-025) and, via the bridge's `kill_on_drop`/`RunningService` drop,
kill-on-drop of the child. Startup timeouts satisfy NFR-005 and the "slow to start" edge case,
fail-closed (the capability is simply absent).

**Alternatives considered**: Relying solely on the episode wall-clock — rejected: it gives no
per-tool bound and never reclaims a wedged child mid-episode.

---

## R9. Credential handling for remote `token_env`

**Decision**: Mirror `api_key_env`. `token_env` names an env var; the value is read once at startup
via `std::env::var(&name).unwrap_or_default()`, passed as the `Authorization: Bearer` header to the
Streamable-HTTP client, never stored in config/transcript/logs. Every configured `token_env` name is
added to the sandbox strip-list (extend `sandbox::key_vars` or append at sandbox construction) so it
is stripped from **stdio** server children too (FR-041/SC-028).

**Rationale**: This is the established provider-key convention (`config.rs` `api_key_env`,
`DEFAULT_KEY_VARS`, `strip`). Reusing it verbatim means the same audited non-leakage guarantees apply
to MCP tokens with no new secret-handling surface.

**Alternatives considered**: OAuth 2.0 — explicitly deferred by the spec.

---

## R10. TOML shape: `[mcp]` is a top-level sibling of `[scenario]`

**Decision**: The scenario file's private `ScenarioFile` wrapper gains
`#[serde(default)] mcp: Option<McpPolicy>`; `Scenario::from_path` moves it onto the `Scenario`
(`s.mcp = file.mcp.unwrap_or_default()`). This keeps the example TOMLs' top-level `[mcp]` /
`[[mcp.servers]]` valid.

**Rationale**: `scenario.rs` deserializes the whole file through `struct ScenarioFile { scenario:
Scenario }`, so everything under `[scenario]` maps to `Scenario`. A top-level `[mcp]` table is a
**sibling** of `[scenario]` and must be a field of the wrapper, not of `Scenario`. Relative paths in
MCP config (if any command scripts) resolve against the scenario dir at the same site as
`policy_path` (scenario.rs:88-96). MCP semantic validation joins `Scenario::validate`.

**Alternatives considered**: Nesting under `[scenario.mcp]` — rejected: contradicts the spec's TOML
examples and the standalone-`mcp.toml` story.

---

## R11. Compile-time opt-in: an `mcp` Cargo feature

**Decision**: Gate the `rmcp` dependency and the entire `mcp` module behind a new
**default-off `mcp` Cargo feature** in `bee-harness`. Built **without** `mcp`, a scenario whose
`[mcp].enabled = true` produces a **fail-closed startup error**
("MCP configured but bee-harness was built without --features mcp"). Built **with** `mcp` but no
`[mcp]` section: no servers connect, no proxies register — behavior identical to today.

**Rationale**: This strengthens NFR-006/SC-027 from a runtime property to a compile-time one, matches
bee's existing feature discipline (`enforce`, `concurrent`), and keeps `rmcp` out of the default
build. Silently ignoring an `[mcp]` config would be the deny-by-default-safe direction (fewer
capabilities), but an explicit error is the Principle-I-correct "fail closed with a clear diagnostic"
rather than a silent surprise. Confines `rmcp` to `bee-harness` (NFR-004/SC-026) at the strongest
granularity.

**Alternatives considered**: rmcp as an unconditional `bee-harness` dep — acceptable per NFR-004 but
adds compile cost to every build; the feature is nearly free to add given `enforce`/`concurrent`
precedent.

---

## R12. `is_known_tool` needs no MCP change

**Decision**: Leave `is_known_tool` and the `DEFAULT_TOOLS`/`CTF_TOOLS`/`RENDER_TOOLS` allowlists
unchanged. Do **not** add an `mcp__` predicate.

**Rationale**: `is_known_tool` validates only the scenario's declared `tools` list (built-ins the
operator lists). MCP tools are **not** listed there — they arrive from `[[mcp.servers]]` and are
registered into the `ToolRegistry` directly. At execute time, dispatch is a `BTreeMap` lookup on the
registered (leaked) name; a miss returns `ToolResult::error("unknown tool")`. Nothing on the
execute path consults `is_known_tool`. Adding an `mcp__` case would be dead code. (The spec's
"is_known_tool extended for mcp__ prefix" is therefore dropped — recorded in plan.md deviations.)

**Alternatives considered**: Adding the predicate defensively — rejected as unused; verified no
execute-path guard references it (T-verify in Phase 1).

---

## Resolved unknowns summary

| # | Unknown | Resolution |
|---|---------|-----------|
| R1 | rmcp version/features | `rmcp = "2.2"`, `default-features=false`, `client` + `transport-child-process` |
| R2 | SSE transport | Removed in 2.x → Streamable-HTTP only; `sse` = warned alias |
| R3 | Sandbox spawn seam | `tool_command` → tokio → `TokioChildProcess::new`; **spike the cgroup-join** |
| R4 | Spike-fail fallback | Raw `(ChildStdout, ChildStdin)` pair transport, bee owns the child |
| R5 | Async connect vs sync registry | `McpBridge::connect` post-registry; `register_into` inserts proxies |
| R6 | rmcp↔bee type mapping | Table in R6; `Box::leak` names; self-truncate |
| R7 | Audit correlation | Automatic via existing `drain_audit` (shared cgroup) |
| R8 | Timeout/crash | Per-call `timeout`; startup `timeout`; kill-on-drop; disconnect errors |
| R9 | `token_env` | Mirror `api_key_env`; strip from children |
| R10 | TOML shape | `[mcp]` on `ScenarioFile` wrapper, sibling of `[scenario]` |
| R11 | Opt-in | Default-off `mcp` Cargo feature; fail-closed if configured-but-absent |
| R12 | `is_known_tool` | No change needed |

**All NEEDS CLARIFICATION resolved.** One item carries into implementation as a **gated spike**
(R3): the plan ships both the primary and fallback transport so the spike outcome selects the path
without blocking the slice.
