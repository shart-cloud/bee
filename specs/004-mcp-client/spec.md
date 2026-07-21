# Feature Specification: MCP Client Integration with Sandboxed Transport

**Feature Branch**: `004-mcp-client`

**Created**: 2026-07-20

**Status**: Draft

**Depends on**: `002-llm-harness` (the tool trait, registry, `Sandbox`, `ReplOutput`, the REPL exchange loop, and the `Model` provider seam)

**Input**: "Add MCP client support to the harness via the official Rust SDK (`rmcp`). MCP servers must not be an exfiltration path — if a domain isn't allowed, the connection is refused. Stdio MCP servers run inside the sandbox. Remote MCP servers are domain-gated. All MCP tool calls appear in the audit trail."

---

## Overview

MCP (Model Context Protocol) gives an LLM agent access to external tools, resources, and prompts
hosted by MCP servers. The agent discovers available tools via `list_tools`, calls them via
`call_tool`, and receives structured results — exactly the same flow as the harness's built-in tools
(`bash`, `read_file`, etc.), but the tool implementations live in external processes or remote
services.

This is a security problem. bee's enforcement model is that the kernel (eBPF-LSM) is the
authoritative boundary: a tool child runs inside a cgroup scope, and the kernel denies operations the
policy forbids. But an MCP server running *outside* the sandbox has no such constraint. If the agent
can talk to an unrestricted MCP server, it can read files, execute commands, exfiltrate data, and
make network connections through the MCP channel — all invisible to the kernel enforcement layer.

This feature adds MCP client support to the harness with a transport-level sandboxing model that
prevents MCP from becoming an exfiltration path:

1. **Stdio MCP servers are sandboxed children.** A stdio-transport MCP server is spawned as a child
   process that joins the episode's bee scope cgroup — exactly like a `bash` tool child. The kernel
   enforces the policy on the MCP server's I/O. If the server tries to read a denied path, the
   kernel denies it. This is Constitution III: the kernel is the boundary, not user-space filtering.

2. **Remote MCP servers are domain-gated.** An HTTP/SSE/Streamable-HTTP MCP server runs on a remote
   host. The harness cannot sandbox it with a cgroup. Instead, the harness enforces a
   **transport-level allowlist**: the connection is refused unless the server's domain is explicitly
   listed in the episode's MCP policy. This is a network-level gate, not a semantic one — the
   harness does not inspect what the tool does, it controls *whether the connection happens at all*.

3. **All MCP tool calls are audited.** Every MCP tool call, result, and any kernel audit events it
   triggers (for stdio servers) are recorded in the episode transcript alongside the built-in tool
   calls. The agent loop treats MCP tools identically to built-in tools — same turn structure, same
   truncation, same timeout.

The MCP client is built on the official Rust SDK (`rmcp` crate).

---

## Threat Model

### What MCP must not enable

| Threat | Without MCP | With unsandboxed MCP | This design |
|--------|-------------|---------------------|-------------|
| Read a policy-denied file | Kernel denies (`EACCES`) | MCP file-server reads it outside the scope | Stdio: kernel denies (server is in scope). Remote: connection refused unless domain allowed. |
| Exfiltrate data to the internet | eBPF network policy denies | MCP server relays data via its own network | Stdio: same network policy applies (server is in scope). Remote: only allowed domains reachable. |
| Execute arbitrary commands | Kernel + hardening apply | MCP shell-server runs commands unrestricted | Stdio: same hardening applies (server is sandboxed child). Remote: connection refused unless domain allowed. |
| Read the harness's API key | Non-dumpable + env-stripped | MCP server inherits the harness env | Stdio: env-stripped like any tool child (FR-018). Remote: no inheritance (separate process). |
| Discover the host's network topology | eBPF network policy | MCP server scans from outside the scope | Stdio: same network policy. Remote: domain allowlist limits reachability. |

### The fundamental rule

**Stdio MCP servers are tool children.** They get the same sandboxing as `bash`: cgroup join,
credential strip, hardening, audit drain. The kernel is authoritative.

**Remote MCP servers are untrusted external services.** The harness cannot sandbox them. It can only
control whether the agent is allowed to talk to them. The domain allowlist is a coarse gate — not a
substitute for the kernel boundary, but a necessary network-level control that prevents the agent
from connecting to arbitrary endpoints.

### What this design does NOT attempt

- **Semantic filtering of MCP tool calls.** The harness does not inspect the arguments or results of
  MCP tool calls to decide whether they violate the bee policy. For stdio servers, the kernel handles
  that. For remote servers, the domain gate is the control — once the connection is allowed, the
  server is trusted to the extent that its tools are available. This is consistent with Constitution
  III: the harness is orchestration, not an enforcement point.

- **Sandboxing remote MCP server processes.** The harness has no control over the remote server's
  execution environment. The domain allowlist is the only lever.

- **OAuth / credential management for remote MCP servers.** The initial implementation supports
  Bearer tokens via configuration. Full OAuth 2.0 flows are a follow-on feature.

---

## MCP Policy

MCP server access is configured per-scenario (or per-REPL session) via a new `[mcp]` section in the
scenario TOML and/or a standalone MCP config file.

### Scenario-level MCP configuration

```toml
# In a scenario TOML or a standalone mcp.toml

[mcp]
# Master switch: if false (or absent), no MCP servers are available.
enabled = true

# Domain allowlist for remote (HTTP/SSE) MCP servers.
# Only servers whose URL hostname matches an entry here can be connected.
# Supports exact match and wildcard subdomains (*.example.com).
# An empty list means no remote servers are allowed.
allowed_domains = [
    "mcp.internal.dev",
    "*.company.com",
]

# Domains that are always denied, even if they match an allowed pattern.
# Useful for excluding specific subdomains from a wildcard allow.
denied_domains = [
    "admin.company.com",
]

# Maximum number of MCP servers the agent can connect to simultaneously.
max_servers = 5

# Per-MCP-tool-call timeout (seconds). Same semantics as the built-in tool timeout.
tool_timeout_secs = 30

# Whether the agent is allowed to dynamically discover and connect to MCP servers
# not listed in [mcp.servers]. If false, only pre-configured servers are available.
# Default: false (fail-closed).
allow_dynamic_connect = false

[[mcp.servers]]
name = "filesystem"
transport = "stdio"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "/workspace"]
# Stdio servers: env vars to set in the server's environment.
# These are ADDED to the stripped environment (the server inherits no API keys).
env = { WORKSPACE = "/workspace" }

[[mcp.servers]]
name = "github"
transport = "sse"
url = "https://mcp.internal.dev/github"
# Remote servers: bearer token for authentication.
# Like api_key_env, this is the NAME of an env var, not the token itself.
token_env = "GITHUB_MCP_TOKEN"
# Optional: restrict which tools from this server the agent can call.
# If absent, all tools are available. If present, only listed tools are exposed.
allowed_tools = ["search_repositories", "get_file_contents"]
# Optional: tools the agent cannot call, even if the server advertises them.
denied_tools = ["create_repository", "delete_repository"]

[[mcp.servers]]
name = "database"
transport = "streamable_http"
url = "https://db.company.com/mcp"
token_env = "DB_MCP_TOKEN"
```

### Policy resolution

1. `mcp.enabled` must be `true`. Otherwise, no MCP servers are available.
2. For stdio servers: the server process is spawned as a sandboxed child (same as `bash`). No domain
   check — it's local.
3. For remote servers: the URL's hostname is checked against `allowed_domains` and `denied_domains`.
   Denied domains take precedence. If the hostname matches neither list, the connection is refused
   (deny-by-default, Constitution I).
4. `allow_dynamic_connect` controls whether the agent can request connections to servers not listed
   in `[mcp.servers]`. Default `false`. When `true`, domain checks still apply.
5. Per-server `allowed_tools` / `denied_tools` filter the tool list after `list_tools`. The agent
   never sees denied tools in its schema. This is a convenience filter, not a security boundary —
   the real security is the transport-level gate and the cgroup sandbox.

---

## Architecture

### Where MCP sits in the existing stack

```text
                    ┌──────────────────────────────────────────────────┐
                    │  Model (Claude / GPT / Mock)                    │
                    │  ToolCall { name: "mcp__fs__read_file", ... }   │
                    └───────────────────┬──────────────────────────────┘
                                        │
                    ┌───────────────────▼──────────────────────────────┐
                    │  ToolRegistry                                    │
                    │  ├─ bash, read_file, ...   → Sandbox (built-in) │
                    │  ├─ render                 → Rhai (in-process)  │
                    │  └─ mcp__fs__read_file     → McpBridge (⬡)     │
                    │    mcp__gh__search_repos   → McpBridge          │
                    └───────────────────┬──────────────────────────────┘
                                        │
              ┌─────────────────────────▼──────────────────────────────┐
              │ McpBridge                                              │
              │                                                        │
              │  ┌─────────────────────────────────────────────────┐   │
              │  │ McpRouter                                       │   │
              │  │  server "fs"  → StdioTransport → sandboxed child│   │
              │  │  server "gh"  → SseTransport   → remote (gated) │   │
              │  └─────────────────────────────────────────────────┘   │
              │                                                        │
              │  On tool call:                                         │
              │   1. Route by prefix (mcp__{server}__{tool})           │
              │   2. Forward to rmcp client.call_tool(tool, args)      │
              │   3. Collect result + audit events (stdio only)        │
              │   4. Return ToolResult to the agent loop               │
              └────────────────────────────────────────────────────────┘
```

### Stdio MCP servers: sandboxed children

```text
  ┌─────────────────────────────────────────────────────────────┐
  │ Harness process                                             │
  │                                                             │
  │  rmcp Client ◄──── stdio ────► MCP Server process           │
  │                                │                            │
  │                                │ joined to bee scope cgroup │
  │                                │ env-stripped (FR-018)      │
  │                                │ hardened (PR_SET_DUMPABLE) │
  │                                │ kernel-enforced I/O        │
  │                                │                            │
  │                                ▼                            │
  │                    ┌─────────────────────┐                  │
  │                    │ eBPF-LSM            │                  │
  │                    │ file_open: DENY /x  │                  │
  │                    │ connect:   DENY *   │                  │
  │                    └─────────────────────┘                  │
  └─────────────────────────────────────────────────────────────┘
```

The MCP server process is spawned using `Sandbox::tool_command` — the same path as `bash` and the
file tools. It joins the scope's cgroup, its environment is stripped of API keys, and the kernel
policy applies to all its I/O. If the MCP server tries to read a denied file, the kernel returns
`EACCES`, the server returns an error to the rmcp client, and the harness records the denial in the
audit trail.

The rmcp `TokioChildProcess` transport needs to be adapted to use `Sandbox::tool_command` instead of
a bare `tokio::process::Command`. This is the key integration point.

### Remote MCP servers: domain-gated

```text
  ┌─────────────────────────────────────────────────────────────┐
  │ Harness process                                             │
  │                                                             │
  │  McpPolicy                                                  │
  │   allowed: [mcp.internal.dev, *.company.com]                │
  │   denied:  [admin.company.com]                              │
  │                                                             │
  │  On connect(url):                                           │
  │   1. Parse hostname from URL                                │
  │   2. Check denied_domains → REFUSE if match                 │
  │   3. Check allowed_domains → REFUSE if no match             │
  │   4. Only then: open HTTP/SSE/StreamableHTTP transport      │
  │                                                             │
  │  rmcp Client ◄──── HTTPS ────► Remote MCP Server            │
  │                                (not sandboxed)              │
  └─────────────────────────────────────────────────────────────┘
```

The domain check happens at transport construction time, before any MCP protocol messages are
exchanged. A refused domain produces a clear error in the transcript ("MCP server at
admin.company.com: connection refused by MCP policy — domain not in allowlist"). The harness does not
attempt the connection.

---

## Tool Naming & Routing

MCP tools are exposed to the model with a namespaced name: `mcp__{server}__{tool}`. For example, a
tool `read_file` on an MCP server named `fs` becomes `mcp__fs__read_file`. This avoids collisions
with built-in tools and with tools from other MCP servers.

The `McpBridge` tool implementation routes calls by parsing the prefix:

```text
mcp__fs__read_file → server "fs", tool "read_file"
mcp__gh__search_repos → server "gh", tool "search_repos"
```

The model sees these namespaced tools in its tool schema alongside the built-in tools. The schema
(name, description, parameters JSON Schema) is fetched from each MCP server via `list_tools` at
connection time and cached.

### Schema refresh

When an MCP server sends a `tools/list_changed` notification, the harness re-fetches the tool list
and updates the schemas for the next model turn. The agent does not need to be told — it sees the
updated schema on its next call.

---

## User Scenarios & Testing

### User Story 8 — Stdio MCP Server in Sandbox (Priority: P2)

An operator configures a scenario with a stdio MCP server (e.g., the filesystem server). The harness
spawns the server process inside the bee scope (same cgroup, env-stripped, hardened), connects to it
via stdio, fetches its tools, and exposes them to the agent as `mcp__{name}__{tool}`. When the agent
calls an MCP tool, the call is forwarded to the server, the result is returned to the agent, and any
kernel audit events produced by the server process are drained and correlated in the transcript.

**Independent Test**: Run a `MockModel` episode where the model calls `mcp__fs__read_file` with a
path the bee policy denies. Verify that (a) the MCP server returns an error (kernel `EACCES`), (b)
the audit trail contains the `file_open` denial for the path, and (c) the `ToolResult` fed back to
the model contains the error.

**Acceptance Scenarios**:

1. **Given** a stdio MCP server `filesystem` and a policy denying `/secrets/`, **When** the agent
   calls `mcp__filesystem__read_file` with path `/secrets/key.pem`, **Then** the MCP server receives
   `EACCES` from the kernel, returns an error, and the transcript records the MCP tool call, the
   error result, and the `file_open` denial audit event.
2. **Given** a stdio MCP server and a permissive policy, **When** the agent calls a tool that reads
   an allowed file, **Then** the tool succeeds, the file content is returned, and no denied audit
   events are recorded.
3. **Given** a stdio MCP server configured with `denied_tools = ["write_file"]`, **When** the model's
   tool schema is built, **Then** `mcp__filesystem__write_file` does not appear in the schema list.
4. **Given** a stdio MCP server, **When** the server process crashes mid-episode, **Then** the
   harness records an error result for the in-flight tool call, marks the server as disconnected, and
   continues the episode (other tools remain available).

---

### User Story 9 — Remote MCP Server with Domain Gating (Priority: P3)

An operator configures a scenario with a remote MCP server (HTTP/SSE transport) at a URL whose
domain is in the allowlist. The harness connects, fetches tools, and exposes them to the agent. A
second scenario configures a server at a domain NOT in the allowlist — the connection is refused
before any MCP messages are exchanged.

**Independent Test**: Configure two MCP server entries: one at `mcp.allowed.dev` (in the allowlist)
and one at `mcp.evil.dev` (not in the allowlist). Start the episode. Verify the first connects
successfully and the second is refused with a clear error in the transcript.

**Acceptance Scenarios**:

1. **Given** a remote MCP server at `https://mcp.internal.dev/tools` with `mcp.internal.dev` in
   `allowed_domains`, **When** the episode starts, **Then** the harness connects, fetches tools, and
   the agent sees `mcp__tools__*` in its schema.
2. **Given** a remote MCP server at `https://mcp.evil.dev/tools` with `mcp.evil.dev` NOT in
   `allowed_domains`, **When** the episode starts, **Then** the connection is refused, the transcript
   records "connection refused by MCP policy", and the episode continues without that server's tools.
3. **Given** `denied_domains = ["admin.company.com"]` and `allowed_domains = ["*.company.com"]`,
   **When** a server at `admin.company.com` is configured, **Then** the connection is refused
   (denied takes precedence over wildcard allow).
4. **Given** `allow_dynamic_connect = false`, **When** the agent somehow requests a connection to a
   server not in `[mcp.servers]`, **Then** the request is refused.

---

### User Story 10 — MCP in the REPL (Priority: P3)

In a REPL session, the operator can configure MCP servers via the REPL config or a config file. MCP
tools appear alongside built-in tools. The `/tools` meta-command lists both built-in and MCP tools
with their server of origin. A `/mcp` meta-command shows connected servers, their status, and the
domain policy.

**Acceptance Scenarios**:

1. **Given** a REPL session with an MCP server configured, **When** the user types `/tools`, **Then**
   built-in tools and MCP tools are listed, with MCP tools showing their server name prefix.
2. **Given** a REPL session, **When** the user types `/mcp`, **Then** the output shows each
   configured server, its transport type, connection status, and the domain allowlist.

---

### Edge Cases

- **MCP server advertises a tool with the same name as a built-in**: The namespacing (`mcp__server__tool`)
  prevents collisions. The built-in tool is always preferred if called by its bare name.
- **MCP server's `list_tools` returns an empty list**: The server is connected but contributes no
  tools. This is not an error — the server may only provide resources or prompts.
- **MCP server is slow to start**: Stdio servers have a configurable startup timeout (default 10s).
  If `initialize` doesn't complete in time, the server is marked failed and its tools are unavailable.
- **MCP server sends a `tools/list_changed` notification mid-turn**: The schema update is deferred
  to the next model call. The in-flight turn uses the schema it started with.
- **Remote MCP server's TLS certificate is invalid**: The connection is refused. The harness does not
  disable certificate validation. Operators can configure a CA bundle if needed.
- **Agent tries to call an MCP tool on a disconnected server**: Returns `ToolResult::error` with
  "MCP server 'name' is not connected". The agent can retry or use other tools.
- **Credential handling for remote servers**: `token_env` names an env var (like `api_key_env` for
  providers). The token is read at startup and passed in the HTTP `Authorization` header. It is never
  logged, never in the transcript, and never in the tool child's environment.

---

## Requirements

### Functional Requirements

- **FR-033**: The harness MUST support connecting to MCP servers via the `rmcp` crate (the official
  Rust SDK for the Model Context Protocol), using stdio, SSE, and Streamable HTTP transports.
- **FR-034**: Stdio MCP server processes MUST be spawned as sandboxed children using
  `Sandbox::tool_command`, joining the episode's bee scope cgroup with the same credential stripping
  and hardening as built-in tool children (FR-018). The kernel policy MUST apply to the MCP server's
  I/O.
- **FR-035**: Remote MCP server connections MUST be gated by a domain allowlist configured per
  scenario. A connection to a domain not in `allowed_domains` or present in `denied_domains` MUST be
  refused before any MCP protocol messages are exchanged. Denied domains take precedence over allowed
  wildcard patterns. An empty allowlist means no remote servers are allowed (deny-by-default).
- **FR-036**: MCP tools MUST be exposed to the model with namespaced names (`mcp__{server}__{tool}`)
  to prevent collisions with built-in tools and with tools from other servers.
- **FR-037**: All MCP tool calls MUST appear in the episode transcript with the same structure as
  built-in tool calls: the `ToolCall` (name, arguments), the `ToolResult` (content, is_error), and
  for stdio servers, the correlated kernel audit events.
- **FR-038**: MCP tool results MUST be subject to the same output truncation (FR-015) and timeout
  (FR-007) as built-in tool results.
- **FR-039**: Per-server tool filtering (`allowed_tools`, `denied_tools`) MUST be applied after
  `list_tools` and before the schema is advertised to the model. Denied tools MUST NOT appear in the
  model's tool schema.
- **FR-040**: The `allow_dynamic_connect` flag MUST default to `false`. When `false`, the harness
  MUST refuse any attempt to connect to an MCP server not listed in the scenario's `[mcp.servers]`.
- **FR-041**: MCP server credentials (`token_env`) MUST follow the same credential handling as
  provider API keys: the env var NAME is in config, the value is read at startup, never logged, never
  in the transcript, and stripped from stdio server children's environments.
- **FR-042**: When a stdio MCP server process exits unexpectedly, the harness MUST record an error
  result for any in-flight tool call, mark the server as disconnected, and continue the episode. The
  agent MUST receive a clear error message ("MCP server 'name' disconnected") if it attempts further
  calls to that server.
- **FR-043**: The harness MUST support the MCP `tools/list_changed` notification: when a connected
  server sends it, the harness re-fetches the tool list and updates the schemas for the next model
  turn.

### Non-Functional Requirements

- **NFR-004**: The `rmcp` dependency MUST NOT propagate to `bee-core` or `bee-common`. It is confined
  to `bee-harness`.
- **NFR-005**: Connecting to a stdio MCP server (spawn + initialize) MUST complete within 10 seconds
  (configurable). Connecting to a remote MCP server (TCP + TLS + initialize) MUST complete within 15
  seconds (configurable).
- **NFR-006**: MCP support MUST be opt-in: scenarios without an `[mcp]` section behave identically
  to the current harness. No MCP-related code runs unless MCP is configured.

---

## Key Entities

### `McpPolicy` (from scenario TOML)

```text
McpPolicy {
    enabled: bool,
    allowed_domains: Vec<DomainPattern>,    // "example.com" or "*.example.com"
    denied_domains: Vec<DomainPattern>,
    max_servers: u32,
    tool_timeout_secs: u64,
    allow_dynamic_connect: bool,
}
```

`DomainPattern` supports exact match and `*.suffix` wildcard. Resolution order: denied first, then
allowed, then deny-by-default.

### `McpServerConfig` (from scenario TOML `[[mcp.servers]]`)

```text
McpServerConfig {
    name: String,                           // used in tool name prefix
    transport: McpTransport,                // Stdio | Sse | StreamableHttp
    // Stdio:
    command: Option<String>,
    args: Vec<String>,
    env: BTreeMap<String, String>,
    // Remote:
    url: Option<String>,
    token_env: Option<String>,
    // Filtering:
    allowed_tools: Option<Vec<String>>,
    denied_tools: Option<Vec<String>>,
}
```

### `McpTransport` (enum)

```text
Stdio       — child process, stdin/stdout, sandboxed
Sse         — HTTP Server-Sent Events, remote, domain-gated
StreamableHttp — HTTP Streamable, remote, domain-gated
```

### `McpBridge` (runtime, in-memory)

The component that manages connected MCP servers and routes tool calls. Constructed at episode/REPL
setup, torn down at the end.

```text
McpBridge {
    policy: McpPolicy,
    servers: BTreeMap<String, ConnectedServer>,  // name → client
}

ConnectedServer {
    config: McpServerConfig,
    client: rmcp::Client,                        // the rmcp client handle
    tools: Vec<ToolSchema>,                      // cached from list_tools
    status: ServerStatus,                        // Connected | Disconnected | Failed
}
```

### `McpToolProxy` (implements `Tool` trait)

A `Tool` impl that routes calls through the `McpBridge`. One is registered per MCP tool, with the
namespaced name. The `call` method forwards to the rmcp client and returns a `ToolResult`.

---

## Integration with the Tool Registry

MCP tools are registered in the `ToolRegistry` alongside built-in tools. The registry doesn't know
or care that they're MCP-backed — they implement the same `Tool` trait.

```text
registry_for(scenario) → ToolRegistry {
    bash            → Bash (built-in)
    read_file       → ReadFile (built-in)
    write_file      → WriteFile (built-in)
    list_directory  → ListDirectory (built-in)
    render          → RenderTool (Rhai, 003)
    mcp__fs__read_file    → McpToolProxy { server: "fs", tool: "read_file" }
    mcp__fs__write_file   → McpToolProxy { server: "fs", tool: "write_file" }
    mcp__gh__search_repos → McpToolProxy { server: "gh", tool: "search_repos" }
}
```

The agent loop (`run_exchange`, `run_loop`) is unchanged. It calls `registry.execute(tc, sandbox)`.
The `McpToolProxy::call` ignores the `sandbox` parameter (stdio MCP servers have their own sandbox
via the spawned process; remote servers are domain-gated at connection time, not per-call).

For stdio MCP servers, audit events are drained from the scope's audit ring after each tool call,
exactly as with built-in tools. The MCP server process runs in the same cgroup, so its denials
appear in the same audit stream.

---

## Sandbox Integration for Stdio Servers

The rmcp SDK's `TokioChildProcess` transport constructs a `tokio::process::Command` internally. For
bee, we need the child to join the bee scope. Two approaches:

### Option A: Custom transport wrapper (preferred)

Wrap `TokioChildProcess` with a bee-aware transport that uses `Sandbox::tool_command` to build the
`std::process::Command`, then converts it to a `tokio::process::Command`:

```rust
pub struct SandboxedChildTransport {
    inner: tokio::process::Child,
    // stdin/stdout handles extracted for the rmcp transport
}

impl SandboxedChildTransport {
    pub fn spawn(
        sandbox: &Sandbox,
        config: &McpServerConfig,
    ) -> Result<Self, SpawnError> {
        let mut std_cmd = sandbox.tool_command(
            &config.command,
            &config.args,
        )?;
        // Set any env vars from config.env
        for (k, v) in &config.env {
            std_cmd.env(k, v);
        }
        let mut cmd = tokio::process::Command::from(std_cmd);
        cmd.stdin(Stdio::piped())
           .stdout(Stdio::piped())
           .stderr(Stdio::null())
           .kill_on_drop(true);
        let child = cmd.spawn()?;
        // Extract stdin/stdout for the rmcp transport layer
        Ok(SandboxedChildTransport { inner: child })
    }
}
```

This transport is then passed to `rmcp::ServiceExt::serve()` to initialize the MCP client.

### Option B: Pre-spawn hook

Use rmcp's `ConfigureCommandExt` to inject the cgroup join closure before spawn. This depends on
rmcp exposing enough of the command configuration, which may not be sufficient for bee's
`pre_exec`-based cgroup join.

Option A is preferred because it keeps the sandbox logic in bee's code, not in rmcp callbacks.

---

## Success Criteria

- **SC-020**: A stdio MCP server spawned via `Sandbox::tool_command` joins the episode's bee scope
  cgroup. A tool call that triggers a policy-denied file read produces a `file_open` denial in the
  audit trail, the MCP server returns an error, and the `ToolResult` fed to the model is
  `is_error: true`.
- **SC-021**: A remote MCP server at a domain NOT in `allowed_domains` is refused at connection time.
  The transcript records the refusal. No MCP protocol messages are exchanged.
- **SC-022**: A remote MCP server at a domain in `allowed_domains` connects successfully. Its tools
  appear in the agent's schema with the `mcp__{server}__` prefix.
- **SC-023**: MCP tool calls appear in the episode transcript with the same structure as built-in
  tool calls (`RecordedCall` with `ToolCall`, `ToolResult`, and audit events).
- **SC-024**: MCP tool results exceeding the output cap (FR-015) are truncated with `truncated:
  true` and `original_len` set.
- **SC-025**: A stdio MCP server's tool call that exceeds `tool_timeout_secs` is killed and returns
  `ToolResult::error("tool timed out")`.
- **SC-026**: `rmcp` does not appear in `bee-core`'s or `bee-common`'s dependency tree.
- **SC-027**: An episode/REPL session with no `[mcp]` configuration behaves identically to the
  current harness — no MCP code runs, no new dependencies are exercised.
- **SC-028**: MCP server credentials (`token_env` values) do not appear in the episode transcript,
  in log output, or in stdio server children's environments.

---

## Dependencies

### New crate dependency (bee-harness only)

```toml
[dependencies]
rmcp = { version = "1", default-features = false, features = ["client", "transport-child-process", "transport-sse-client"] }
```

Feature flags:
- `client` — the MCP client implementation.
- `transport-child-process` — stdio transport (adapted for bee's sandbox).
- `transport-sse-client` — SSE and Streamable HTTP transports for remote servers.

`rmcp` depends on `tokio`, `serde`, `serde_json`, `reqwest` (for HTTP transports) — all of which
`bee-harness` already depends on or is compatible with.

---

## Project Structure

### New files

```text
bee-harness/src/
├── mcp.rs                   # McpBridge, McpPolicy, domain matching, connection lifecycle
├── mcp/
│   ├── policy.rs            # McpPolicy parsing, domain pattern matching (exact + wildcard)
│   ├── bridge.rs            # McpBridge: manages connected servers, routes tool calls
│   ├── transport.rs         # SandboxedChildTransport: bee-aware stdio transport for rmcp
│   ├── proxy.rs             # McpToolProxy: Tool trait impl that forwards to rmcp client
│   └── config.rs            # McpServerConfig, McpTransport enum, TOML parsing
```

### Modified files

```text
bee-harness/Cargo.toml       # add rmcp dependency
bee-harness/src/tools.rs     # is_known_tool extended for mcp__ prefix; registry_for gains MCP path
bee-harness/src/scenario.rs  # Scenario gains optional McpPolicy + Vec<McpServerConfig>
bee-harness/src/config.rs    # ProviderConfig gains mcp section
bee-harness/src/episode.rs   # Episode setup connects MCP servers; teardown disconnects
bee-harness/src/repl.rs      # REPL config gains MCP; /mcp meta-command; /tools shows MCP tools
bee-harness/src/repl/terminal.rs  # /mcp command rendering
```

### Spec files

```text
specs/004-mcp-client/
├── spec.md                  # this file
├── contracts/
│   ├── mcp-policy.md        # domain allowlist resolution, transport rules
│   ├── mcp-tool-naming.md   # namespacing convention (mcp__{server}__{tool})
│   └── sandbox-transport.md # how stdio MCP servers are sandboxed
└── examples/
    ├── stdio-filesystem.toml    # scenario with a sandboxed filesystem MCP server
    ├── remote-github.toml       # scenario with a domain-gated remote MCP server
    └── mixed.toml               # scenario with both stdio and remote servers
```

---

## Constitution Check

| Principle | Assessment |
|-----------|------------|
| **I. Deny-by-Default & Fail-Closed** | PASS. Remote MCP servers are refused unless their domain is explicitly allowed. `allow_dynamic_connect` defaults to `false`. An empty allowlist means no remote access. Stdio servers are sandboxed. |
| **II. Capability Attenuation** | PASS (deferred). A future subagent that spawns its own MCP client would inherit the parent scope's policy via `derive_scope`, attenuating the MCP policy. Not built this slice, but the design does not preclude it. |
| **III. Kernel Enforcement Is Authoritative** | PASS. For stdio MCP servers, the kernel is the boundary — the server runs in the scope, and the harness reports what the kernel decided. For remote servers, the harness acknowledges it cannot sandbox them and applies a transport-level gate instead, which is explicitly documented as a network control, not a semantic enforcement point. |
| **IV. Policy-as-Data** | PASS. MCP configuration is declarative TOML (`[mcp]`, `[[mcp.servers]]`). Domain allowlists are data, not code. |
| **V. Library-First, Runtime-Free Core** | PASS. `rmcp` is confined to `bee-harness`. `bee-core` and `bee-common` gain no new dependencies. |
| **Security/Platform** | PASS. Credential handling follows FR-018 (env var names in config, values read at startup, stripped from children). Domain gating prevents uncontrolled network access. Audit trail captures MCP tool calls. |

**Result: no violations.**

---

## Assumptions

- The `rmcp` crate (v1.x) exposes enough of the transport layer to allow a custom stdio transport
  that uses `Sandbox::tool_command` instead of a bare `Command`. If rmcp's `TokioChildProcess` is
  sealed, a custom transport implementation using rmcp's `IntoTransport` trait is the fallback.
- MCP servers using the stdio transport are single-process (no child spawning). If a server spawns
  its own children, those children inherit the cgroup and are subject to the same policy — which is
  the correct behavior.
- The `reqwest` HTTP client used by rmcp for SSE/StreamableHTTP transports supports TLS certificate
  validation by default. The harness does not weaken this.
- Remote MCP servers are trusted *within their domain*. Once the domain gate allows a connection, the
  harness does not inspect or filter individual tool calls beyond the `allowed_tools`/`denied_tools`
  convenience filter. This is a deliberate design choice: the harness is not a WAF.
- The rmcp client supports reconnection for transient failures (network blips). The harness does not
  add its own reconnection logic on top.
- OAuth 2.0 authentication for remote MCP servers is a follow-on feature. The initial implementation
  supports Bearer tokens via `token_env` only.
