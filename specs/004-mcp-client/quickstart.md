# Quickstart & Validation Guide: MCP Client

**Feature**: `004-mcp-client` | **Spec**: [spec.md](./spec.md) · **Plan**: [plan.md](./plan.md)

A runnable guide proving the feature end-to-end. It maps each success criterion to a command and an
expected observable. It does **not** contain implementation code — see [data-model.md](./data-model.md)
and the contracts for that.

---

## Prerequisites

- Rust stable 1.85 toolchain (workspace default).
- The `mcp` Cargo feature (this feature; default-off): `--features mcp`.
- For **stdio** validation: an MCP server binary on `PATH`. The examples use the reference filesystem
  server (`npx -y @modelcontextprotocol/server-filesystem`), so Node/npx must be installed — OR
  substitute any stdio MCP server and adjust `command`/`args`.
- For **enforced** stdio validation (SC-020 kernel denial): a BPF-LSM + cgroup-v2 kernel and
  `--features enforce,mcp`. Without `enforce`, the server runs as a hardened host process (no scope,
  no kernel denial) — good enough for wiring/mapping/truncation checks, not for the denial assertion.
- For **remote** validation (Slice 2): reachable Streamable-HTTP MCP endpoints, and the token env var
  named by `token_env` exported (e.g. `export GITHUB_MCP_TOKEN=…`).

Example scenarios: [examples/stdio-filesystem.toml](./examples/stdio-filesystem.toml),
[examples/remote-github.toml](./examples/remote-github.toml), [examples/mixed.toml](./examples/mixed.toml).
Each references a sibling `*.policy.toml` (bee policy, 001 format — deny `/secrets`, allow `/workspace`).

---

## Gate 0 — no-MCP regression (SC-027)

**Prove nothing changed for existing scenarios.**

```bash
cargo test -p bee-harness                 # default build, no mcp feature
cargo test -p bee-harness --features mcp  # mcp compiled in, but scenarios have no [mcp]
```

**Expect**: identical pass set. A scenario with no `[mcp]` table connects no servers, registers no
proxies, and produces a byte-identical transcript shape. Building **without** `mcp` but running a
scenario whose `[mcp].enabled = true` ⇒ a clear startup error ("built without --features mcp"),
never a silent run (R11, Principle I).

---

## Gate 1 — stdio server sandboxed; denial in audit (US8 / SC-020, SC-023)

**The core security assertion.** Uses `MockModel` so the tool calls are deterministic.

```bash
# On a BPF-LSM VM:
cargo test -p bee-harness --features enforce,mcp --test mcp_stdio
```

The `mcp_stdio` test drives a `MockModel` that calls `mcp__filesystem__read_file` with
`/secrets/key.pem` (denied by the paired policy), then reports and stops.

**Expect**:
1. The filesystem MCP server was spawned via `Sandbox::tool_command` and **joined the scope cgroup**
   (verified directly by the R3 spike assertion; see [contracts/sandbox-transport.md](./contracts/sandbox-transport.md) §3.2).
2. The transcript's `RecordedCall` for that call has `result.is_error == true` and its `audit` vec
   contains a `file_open` event with `decision == "denied"` for `/secrets/key.pem` (SC-023).
3. A permissive read of `/workspace/...` succeeds with `is_error == false` and **no** denied audit
   events (US8 AS-2).

---

## Gate 2 — tool filtering hides denied tools (US8 AS-3 / FR-039)

The `filesystem` server config sets `denied_tools = ["write_file"]`.

**Expect**: `registry.schemas()` (what the model sees, and what `/tools` prints) contains
`mcp__filesystem__read_file` and `mcp__filesystem__list_directory` but **not**
`mcp__filesystem__write_file`. Assert in `mcp_stdio.rs`.

---

## Gate 3 — truncation & timeout (SC-024, SC-025)

**Expect**:
- A tool result larger than `DEFAULT_OUTPUT_CAP` (100 KB) comes back with `truncated == true`,
  `original_len == Some(n)`, and the `[truncated: … bytes, showing …]` marker (SC-024).
- A tool call exceeding `tool_timeout_secs` returns `ToolResult::error` mentioning "timed out"; the
  episode continues (SC-025). (Test with a deliberately slow mock MCP server or a short
  `tool_timeout_secs`.)

---

## Gate 4 — crash handling (US8 AS-4 / FR-042)

Kill the stdio server child mid-episode (the test harness sends SIGKILL to the child pid).

**Expect**: the in-flight call records an error result; the server's status becomes `Disconnected`;
a subsequent `mcp__filesystem__*` call returns `ToolResult::error("MCP server 'filesystem'
disconnected")`; built-in tools and other servers keep working; the episode completes (not aborted).

---

## Gate 5 — remote domain gating (US9 / SC-021, SC-022)

```bash
cargo test -p bee-harness --features mcp --test mcp_domain_gating
```

Configure two remote servers: one at a host in `allowed_domains`, one not. This gate needs **no live
network** for the refusal case — the refusal happens before any socket write.

**Expect** (and the normative matrix in [contracts/mcp-policy.md](./contracts/mcp-policy.md)):
1. Allowed host ⇒ connects; its tools appear as `mcp__{server}__*` (SC-022). *(Live endpoint needed
   only for the positive connect; use a local stub server or mark `#[ignore]` for CI.)*
2. Non-allowed host ⇒ **connection refused**, transcript records "connection refused by MCP policy",
   **no MCP protocol messages exchanged** (SC-021). Assert the client never opened a transport.
3. `denied_domains = ["admin.company.com"]` + `allowed_domains = ["*.company.com"]` ⇒
   `admin.company.com` refused (denied wins over wildcard).
4. `allow_dynamic_connect = false` ⇒ a request to connect to an undeclared server is refused.

Property test (`proptest`): `DomainPattern::matches` — wildcard never matches the bare apex, never
matches a suffix-substring (`evilcompany.com` vs `*.company.com`), always matches deeper subdomains.

---

## Gate 6 — credentials never leak (SC-028)

**Expect**:
- The value of `token_env` (e.g. `GITHUB_MCP_TOKEN`) appears **nowhere** in the transcript JSON or log
  output. Grep the produced transcript for the token value ⇒ zero hits.
- A **stdio** server child's environment contains no provider key vars and no configured `token_env`
  names with real values (assert via a stdio echo-env test server or by inspecting the built
  `Command`'s env before spawn).

---

## Gate 7 — REPL surfaces (US10)

```bash
cargo run -p bee-harness --features mcp --bin bee-repl -- \
    --tools bash,read_file --mcp-config specs/004-mcp-client/examples/stdio-filesystem.toml
```

**Expect**:
- `/tools` lists built-ins **and** `mcp__filesystem__*` (the proxies share the one registry, so this
  is free).
- `/mcp` prints each configured server: name, transport, connection status, and the domain allowlist.

---

## Success-criteria coverage map

| SC | Gate | SC | Gate |
|----|------|----|------|
| SC-020 | Gate 1 | SC-025 | Gate 3 |
| SC-021 | Gate 5.2/5.3 | SC-026 | `cargo tree -p bee-core` shows no `rmcp` (CI check) |
| SC-022 | Gate 5.1 | SC-027 | Gate 0 |
| SC-023 | Gate 1 | SC-028 | Gate 6 |
| SC-024 | Gate 3 | | |

`cargo tree --no-default-features -p bee-core | grep rmcp` (SC-026) MUST return nothing — add it as a
CI assertion alongside the tests.
