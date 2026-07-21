# Contract: MCP Tool Naming & Routing

**Feature**: `004-mcp-client`
**Status**: Draft
**Related**: FR-036, FR-043, SC-022, SC-023

Defines the namespacing convention for MCP tools and how the `McpBridge` routes calls back to the
originating server.

---

## 1. Naming convention

An MCP tool `{tool}` served by an MCP server named `{server}` is advertised to the model as:

```
mcp__{server}__{tool}
```

- Separator is a **double underscore** (`__`), on both sides of `{server}`.
- `{server}` is the `name` field from the `[[mcp.servers]]` entry.
- `{tool}` is the bare tool name as returned by the server's `list_tools`.

Examples:

```
server "fs",  tool "read_file"      → mcp__fs__read_file
server "gh",  tool "search_repos"   → mcp__gh__search_repos
server "db",  tool "run_query"      → mcp__db__run_query
```

## 2. Name constraints

- `{server}` MUST match `^[a-z0-9_-]+$` and MUST NOT itself contain `__` (double underscore), so the
  split in §3 is unambiguous. Invalid server names are rejected at config-parse time.
- `{tool}` MAY contain single underscores. Reassembly (§3) treats **everything after the second `__`**
  as the tool name, so tool names containing `__` round-trip correctly.

## 3. Routing (parse on call)

Given an incoming `ToolCall` name:

1. If it does not start with `mcp__` → it is a **built-in** tool; route to the built-in registry.
2. Else strip the `mcp__` prefix, then split the remainder on the **first** `__`:
   - left part = `{server}`
   - right part = `{tool}` (may contain further `__`)
3. Look up `{server}` in the `McpBridge`. If absent/disconnected → `ToolResult::error`
   (`"MCP server '{server}' is not connected"`).
4. Forward `call_tool({tool}, arguments)` to that server's rmcp client.

## 4. Collision rules (edge cases)

- **MCP tool vs built-in with the same bare name**: no collision — MCP tools always carry the
  `mcp__{server}__` prefix. A bare name (e.g. `read_file`) always resolves to the built-in.
- **Two servers advertising the same bare tool**: no collision — the `{server}` segment differs
  (`mcp__fs__read_file` vs `mcp__other__read_file`).
- **Empty `list_tools`**: the server contributes zero tools. Not an error.

## 5. Schema lifecycle

- Tool schemas (name, description, JSON-Schema parameters) are fetched via `list_tools` at connection
  time and cached in `ConnectedServer.tools`.
- On a `tools/list_changed` notification (FR-043), the cached list is refreshed. The refresh is
  **deferred to the next model turn** — an in-flight turn uses the schema it started with.
- Filtered tools (`allowed_tools`/`denied_tools`, per `mcp-policy.md` §7) are removed **before** the
  schema is advertised. Denied tools never appear in the model-visible schema.

## 6. Transcript structure (FR-037, SC-023)

An MCP tool call records the same shape as a built-in call:

```
RecordedCall {
    tool_call:  ToolCall  { name: "mcp__fs__read_file", arguments: {...} },
    tool_result: ToolResult { content, is_error, truncated, original_len },
    audit_events: [ ... ],   // stdio servers only; empty for remote
}
```

Output truncation (FR-015) and timeout (FR-007 / `tool_timeout_secs`) apply identically to MCP
results.
