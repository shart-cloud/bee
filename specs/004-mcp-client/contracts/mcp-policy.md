# Contract: MCP Policy — Domain Allowlist Resolution & Transport Rules

**Feature**: `004-mcp-client`
**Status**: Draft
**Related**: FR-035, FR-040, SC-021, SC-022

This contract fixes the observable behavior of MCP policy resolution. Implementations MUST satisfy
every rule here; tests derive directly from it.

---

## 1. Master switch

- If `[mcp]` is absent, or `mcp.enabled` is `false`, the harness connects to **no** MCP servers and
  registers **no** MCP tools. Behavior is identical to a harness without MCP support (NFR-006).

## 2. Transport classification

| `transport` | Class | Gate |
|-------------|-------|------|
| `stdio` | Local | Sandboxed child (cgroup + env-strip + hardening). No domain check. |
| `sse` | Remote | Domain allowlist. |
| `streamable_http` | Remote | Domain allowlist. |

## 3. Domain resolution (remote transports only)

Given a server URL, extract the **hostname** (no port, no scheme, no path). Resolve in this order:

1. **Denied wins.** If the hostname matches any `denied_domains` pattern → **REFUSE**.
2. **Allowed required.** Else if the hostname matches any `allowed_domains` pattern → **ALLOW**.
3. **Deny by default.** Else → **REFUSE**.

An empty `allowed_domains` list ⇒ every remote server is refused.

### 3.1 Pattern matching (`DomainPattern`)

| Pattern | Matches | Does NOT match |
|---------|---------|----------------|
| `example.com` (exact) | `example.com` | `sub.example.com`, `notexample.com` |
| `*.example.com` (wildcard) | `a.example.com`, `a.b.example.com` | `example.com` (bare apex), `evilexample.com` |

Rules:
- Matching is **case-insensitive** (hostnames are lowercased before comparison).
- `*.` matches one or more subdomain labels, but NOT the bare apex. To allow both, list both
  `example.com` and `*.example.com`.
- A leading `*` without the dot (`*example.com`) is invalid config and MUST be rejected at parse time.
- Trailing dots in a hostname (`example.com.`) are normalized away before matching.

## 4. Refusal is observable and non-fatal

- A refused connection produces a transcript entry:
  `MCP server at {host}: connection refused by MCP policy — domain not in allowlist`
  (or `— domain in denylist` when rule 1 fired).
- **No MCP protocol messages are exchanged** with a refused server. The refusal happens at transport
  construction, before any socket write (SC-021).
- Refusing one server does NOT abort the episode. Other servers connect; the episode proceeds.

## 5. `allow_dynamic_connect` (FR-040)

- Default `false`.
- When `false`: any request to connect to a server not present in `[[mcp.servers]]` is refused,
  regardless of domain.
- When `true`: dynamic connections are permitted, but domain resolution (§3) still applies to remote
  transports, and `max_servers` still caps the total.

## 6. `max_servers`

- Caps simultaneously-connected servers. Attempts beyond the cap are refused with a clear transcript
  message. Pre-configured servers are connected in declaration order until the cap is reached.

## 7. Per-server tool filtering (FR-039)

Applied **after** `list_tools`, **before** advertising schemas to the model:

1. If `allowed_tools` is present: keep only tools whose bare name is in the list.
2. Then, if `denied_tools` is present: drop any tool whose bare name is in the list.
   (`denied_tools` overrides `allowed_tools` on conflict.)
3. Surviving tools are advertised as `mcp__{server}__{tool}`.

This is a **convenience filter, not a security boundary** — the transport gate (§3) and the cgroup
sandbox are the real controls.

---

## Test matrix (normative)

| # | allowed_domains | denied_domains | host | Expect |
|---|-----------------|----------------|------|--------|
| 1 | `[mcp.internal.dev]` | `[]` | `mcp.internal.dev` | ALLOW |
| 2 | `[mcp.internal.dev]` | `[]` | `mcp.evil.dev` | REFUSE (default) |
| 3 | `[*.company.com]` | `[admin.company.com]` | `admin.company.com` | REFUSE (denied wins) |
| 4 | `[*.company.com]` | `[admin.company.com]` | `app.company.com` | ALLOW |
| 5 | `[*.company.com]` | `[]` | `company.com` | REFUSE (wildcard ≠ apex) |
| 6 | `[]` | `[]` | anything remote | REFUSE (empty allowlist) |
| 7 | any | any | any `stdio` server | N/A — no domain check |
