# Phase 1 Data Model: MCP Client

**Feature**: `004-mcp-client` | **Date**: 2026-07-20 | **Spec**: [spec.md](./spec.md) · **Research**:
[research.md](./research.md)

Entities are grouped by lifetime: **config** (parsed from TOML, `serde`), **runtime** (in-memory for
the episode/REPL), and **adapter** (the bee↔rmcp boundary). Types live in `bee-harness/src/mcp/`,
behind the `mcp` Cargo feature.

---

## Config entities (deserialized from the scenario `[mcp]` table)

### `McpPolicy`  — `mcp/policy.rs`

The `[mcp]` table. Attached to `Scenario` via the `ScenarioFile` wrapper (R10). `#[derive(Debug,
Clone, Serialize, Deserialize)]`, `#[serde(default)]` on every field so an absent/partial `[mcp]`
degrades safely.

| Field | Type | Default | Notes |
|-------|------|---------|-------|
| `enabled` | `bool` | `false` | Master switch. `false`/absent ⇒ no MCP (SC-027). |
| `allowed_domains` | `Vec<DomainPattern>` | `[]` | Remote allowlist. Empty ⇒ no remote (deny-by-default). |
| `denied_domains` | `Vec<DomainPattern>` | `[]` | Takes precedence over `allowed_domains`. |
| `max_servers` | `u32` | `5` | Cap on simultaneous connections. |
| `tool_timeout_secs` | `u64` | `30` | Per-MCP-tool-call timeout (FR-038). |
| `allow_dynamic_connect` | `bool` | `false` | Fail-closed default (FR-040). |
| `servers` | `Vec<McpServerConfig>` | `[]` | The `[[mcp.servers]]` array. |

> **Serde note**: `[[mcp.servers]]` nests under `[mcp]`, so `servers` is a field of `McpPolicy` (not a
> separate `Scenario` field as the spec's Key-Entities sketch implied). `DomainPattern` deserializes
> from a bare string via `#[serde(try_from = "String")]`.

**Validation** (in `Scenario::validate`, fail-closed):
- If `enabled` and any server has `transport ∈ {sse, streamable_http}` while both domain lists are
  empty ⇒ warn (every remote will be refused) — not a hard error (a stdio-only + remote-declared
  config may be intentional), but surfaced.
- `max_servers >= 1` when `enabled`.
- `tool_timeout_secs >= 1`.
- Duplicate `servers[].name` ⇒ **error** (names must be unique — they key routing).
- Every `DomainPattern` parses (see below); a bare-`*` or `*foo` pattern ⇒ **error**.

### `McpServerConfig`  — `mcp/config.rs`

One `[[mcp.servers]]` entry. `#[serde(deny_unknown_fields)]` is **not** set (forward-compat), but
transport-appropriateness is validated.

| Field | Type | Applies to | Notes |
|-------|------|-----------|-------|
| `name` | `String` | all | Routing key; `^[a-z0-9_-]+$`, no `__` (naming contract). |
| `transport` | `McpTransport` | all | `stdio` \| `sse` \| `streamable_http`. |
| `command` | `Option<String>` | stdio | Program; required for stdio. |
| `args` | `Vec<String>` | stdio | `#[serde(default)]`. |
| `env` | `BTreeMap<String,String>` | stdio | Added atop the stripped env (never re-adds a secret to a real value silently — logged if it collides). |
| `url` | `Option<String>` | remote | Required for remote; hostname is domain-gated. |
| `token_env` | `Option<String>` | remote | **Name** of the Bearer-token env var (FR-041). Ignored (warned) for stdio. |
| `allowed_tools` | `Option<Vec<String>>` | all | If `Some`, whitelist of bare tool names. |
| `denied_tools` | `Option<Vec<String>>` | all | Blacklist; overrides `allowed_tools` on conflict. |

**Validation** (fail-closed, in `Scenario::validate`):
- `transport = stdio` ⇒ `command` required, `url`/`token_env` must be absent (warn if present).
- `transport ∈ {sse, streamable_http}` ⇒ `url` required, `command`/`args`/`env` must be absent (warn).
- `name` matches the naming-contract regex.

### `McpTransport` (enum)  — `mcp/config.rs`

```text
Stdio           → sandboxed child (feature transport-child-process)
StreamableHttp  → remote, domain-gated (feature transport-streamable-http-client-reqwest)
Sse             → DEPRECATED alias: parses OK, warns, routes to the StreamableHttp client (R2)
```

`#[serde(rename_all = "snake_case")]` so TOML `"streamable_http"` maps to `StreamableHttp`.

### `DomainPattern` (enum)  — `mcp/policy.rs`

```text
Exact(String)       // "example.com"      — matches that host, case-insensitive
Wildcard(String)    // "*.example.com"    — matches one-or-more subdomain labels, NOT the apex
```

- Parse (`TryFrom<String>`): a leading `*.` ⇒ `Wildcard(suffix)`; a leading bare `*` or `*foo`
  (no dot) ⇒ **parse error**; otherwise `Exact(lowercased, trailing-dot-stripped)`.
- `matches(host: &str) -> bool`: lowercase + strip trailing dot on `host` first. `Exact` = equality;
  `Wildcard(suffix)` = `host` ends with `.suffix` **and** has at least one label before it (apex
  excluded). **Property-tested** (Principle: security behavior via proptest) — e.g. `*.company.com`
  never matches `evilcompany.com`, always matches `a.b.company.com`, never matches `company.com`.

Resolution order (a free fn `resolve(host, &policy) -> Decision`): **denied → allowed → deny**. See
[contracts/mcp-policy.md](./contracts/mcp-policy.md) §3 for the normative matrix.

---

## Runtime entities (in-memory, episode/REPL lifetime)

### `McpBridge`  — `mcp/bridge.rs`

Owns every connected server; constructed in `run_episode`/REPL after the base registry + sandbox
exist; dropped at teardown (killing stdio children).

```text
McpBridge {
    policy:  McpPolicy,
    servers: BTreeMap<String, ConnectedServer>,   // name → server
}
```

Methods:
- `async connect(policy: McpPolicy, sandbox: &Sandbox) -> McpBridge` — for each `servers[]` (up to
  `max_servers`): stdio ⇒ spawn-in-sandbox + `serve()` under a 10 s timeout; remote ⇒ resolve domain
  (refuse+record if denied), else Streamable-HTTP `serve()` under 15 s. Each result becomes a
  `ConnectedServer` (`Connected` or `Failed`). Refusals/failures are recorded, not fatal.
- `register_into(&self, registry: &mut ToolRegistry)` — for each `Connected` server, apply
  `allowed_tools`/`denied_tools`, then insert one `McpToolProxy` per surviving tool.
- `teardown(self)` — drops all `RunningService`s (kill-on-drop children); idempotent.

### `ConnectedServer`  — `mcp/bridge.rs`

```text
ConnectedServer {
    config:  McpServerConfig,
    service: RunningService<RoleClient, Handler>,  // owns child + background task (stdio); or HTTP client
    peer:    Peer<RoleClient>,                      // cheap clone handed to each proxy
    tools:   Vec<ToolSchema>,                       // cached from list_all_tools, post-filter
    status:  ServerStatus,
}
```

`Handler` is `()` for Slice 1, upgraded to an `rmcp::ClientHandler` impl when `tools/list_changed`
(FR-043) lands, so `on_tool_list_changed` can refresh `tools` for the next turn.

### `ServerStatus` (enum) — state machine

```text
        connect() ok            call/notify sees dead peer
   ─────────────────────►  Connected ──────────────────────►  Disconnected
        │                                                          
        │ spawn/init fails or times out (NFR-005)                  
        └────────────────────────────────────────────►  Failed    
```

- `Failed` — never became usable; tools never registered (fail-closed capability absence).
- `Disconnected` — was `Connected`, child crashed/peer died (FR-042). In-flight call ⇒ error result;
  further calls ⇒ `ToolResult::error("MCP server '{name}' disconnected")`. Episode continues.
- No transition back to `Connected` (no bespoke reconnection — the spec defers that to rmcp's own
  transient handling).

---

## Adapter entity (bee ↔ rmcp boundary)

### `McpToolProxy`  — `mcp/proxy.rs` — `impl Tool`

One per registered MCP tool. The `RenderTool` analog: implements bee's `Tool` trait, ignores the
`&Sandbox` param (the server, not this in-process proxy, is what the kernel sandboxes).

```text
McpToolProxy {
    name:      &'static str,      // leaked "mcp__{server}__{tool}" (R6 interning)
    schema:    ToolSchema,        // {name, description, parameters=input_schema}
    peer:      Peer<RoleClient>,  // handle into the owning ConnectedServer's service
    bare_tool: String,            // tool name as the server knows it
    timeout:   Duration,          // policy.tool_timeout_secs
}
```

`impl Tool` (with `#[async_trait::async_trait]`):
- `name(&self) -> &'static str` → the leaked name.
- `schema(&self) -> ToolSchema` → cached clone.
- `async call(&self, arguments: Value, _sandbox: &Sandbox) -> ToolResult`:
  1. `arguments` must be a JSON object → `serde_json::Map`; else `ToolResult::invalid_args`.
  2. `tokio::time::timeout(self.timeout, self.peer.call_tool(CallToolRequestParams { name:
     self.bare_tool.clone().into(), arguments: Some(map), ..Default::default() }))`.
  3. `Err(elapsed)` → `ToolResult::error("MCP tool '…': timed out")` (SC-025). `Ok(Err(ServiceError
     dead-peer))` → mark server `Disconnected`, `ToolResult::error("…disconnected")` (FR-042).
  4. `Ok(Ok(CallToolResult))` → flatten text `ContentBlock`s into `content`, `is_error =
     res.is_error.unwrap_or(false)`, then `transcript::truncate(content, DEFAULT_OUTPUT_CAP)` and
     build the `ToolResult` (SC-024).

The resulting `RecordedCall { call, result, audit }` is produced by the **unchanged** episode loop;
for stdio servers `audit` is populated automatically from the shared-cgroup drain (R7).

---

## Relationships

```text
Scenario ──has──▶ McpPolicy ──has many──▶ McpServerConfig
                     │                         │
                     │ connect(&sandbox)       │
                     ▼                         ▼
                  McpBridge ──has many──▶ ConnectedServer ──has one──▶ Peer<RoleClient>
                     │                         │                            ▲
                     │ register_into           │ list_all_tools             │ clone
                     ▼                         ▼                            │
              ToolRegistry ◀──inserts── McpToolProxy (impl Tool) ───────────┘
                     ▲
                     │ execute(ToolCall) [unchanged loop]
                  agent turn
```

## Field-level provenance (why each field exists)

| Field | Requirement |
|-------|-------------|
| `McpPolicy.allowed_domains`/`denied_domains` | FR-035, SC-021/022 |
| `McpPolicy.allow_dynamic_connect` (default false) | FR-040 |
| `McpPolicy.tool_timeout_secs` | FR-038, SC-025 |
| `McpServerConfig.transport = stdio` → sandboxed spawn | FR-034, SC-020 |
| `McpServerConfig.token_env` (name only, stripped) | FR-041, SC-028 |
| `McpServerConfig.allowed_tools`/`denied_tools` | FR-039 |
| `McpToolProxy.name = mcp__{server}__{tool}` | FR-036 |
| `McpToolProxy.call` truncation | FR-038, SC-024 |
| `ServerStatus::Disconnected` handling | FR-042 |
| `ConnectedServer` + `ClientHandler` refresh | FR-043 |
