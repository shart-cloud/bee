# Contract: Sandboxed Stdio Transport for MCP Servers

**Feature**: `004-mcp-client`
**Status**: Draft
**Related**: FR-034, FR-041, FR-042, SC-020, SC-025, SC-028

Defines how a stdio-transport MCP server is spawned as a bee-scoped sandboxed child, so the kernel
(eBPF-LSM) is the enforcement boundary for its I/O — identical treatment to a `bash` tool child.

---

## 1. Spawn path

A stdio MCP server process MUST be created via `Sandbox::tool_command(command, args)` — the **same**
constructor used for `bash`, `read_file`, etc. It MUST NOT be spawned with a bare
`std::process::Command` / `tokio::process::Command`.

Consequences inherited from `Sandbox::tool_command` (do not reimplement — reuse):
- **cgroup join**: the child joins the episode's bee scope cgroup (via the established `pre_exec`
  hook), so kernel policy applies to all its syscalls.
- **credential strip** (FR-018 / FR-041): the environment is stripped of API keys and provider
  secrets. The server inherits **no** harness credentials.
- **hardening**: `PR_SET_DUMPABLE = 0` and any other tool-child hardening already applied.

## 2. Environment composition

The child's environment is: **stripped base**  +  the server's `[[mcp.servers]].env` map.

- `env` entries are additive on top of the stripped environment.
- `env` MUST NOT be able to re-introduce a stripped secret name to a real value — if a config `env`
  key collides with a stripped credential var, the config value is used but this is logged as a
  warning (it is the operator's explicit choice, not inheritance).
- `token_env` is **not** used for stdio servers (it is a remote-transport auth mechanism). If present
  on a stdio server config, it is ignored with a warning.

## 3. Transport wiring (Option A — preferred)

```
SandboxedChildTransport::spawn(sandbox, config):
    std_cmd = sandbox.tool_command(config.command, config.args)   // cgroup + strip + harden
    for (k, v) in config.env: std_cmd.env(k, v)
    cmd = tokio::process::Command::from(std_cmd)
    cmd.stdin(piped).stdout(piped).stderr(null).kill_on_drop(true)
    child = cmd.spawn()
    // hand child.stdin / child.stdout to rmcp's transport layer
    rmcp::ServiceExt::serve(transport)   // performs MCP `initialize`
```

- `stderr` is `null` (or captured to the transcript diagnostics channel — never to stdout, which is
  the MCP framing channel).
- `kill_on_drop(true)` guarantees the child dies with the bridge.
- Fallback (Option B) — rmcp's `ConfigureCommandExt` pre-spawn hook — is only acceptable if it can
  install bee's `pre_exec` cgroup-join closure. If it cannot, Option A is mandatory.

## 4. Startup timeout (NFR-005)

- Spawn + `initialize` MUST complete within **10 s** (configurable). On timeout the server is marked
  `Failed`, the child is killed, its tools are not advertised, and the episode continues.

## 5. Audit correlation

- Because the child is in the scope cgroup, its kernel denials appear in the **same** audit ring as
  built-in tools' denials.
- After each MCP tool call, the bridge drains the scope audit ring and attaches the events to the
  `RecordedCall.audit_events` for that call (SC-020, SC-023).
- A policy-denied file read by the server ⇒ `file_open` denial in the audit trail **and**
  `ToolResult { is_error: true }` fed to the model.

## 6. Per-call timeout (SC-025)

- A tool call exceeding `mcp.tool_timeout_secs` is aborted; the result is
  `ToolResult::error("tool timed out")`. The server process is NOT necessarily killed (only the
  in-flight request is abandoned) unless the transport is unrecoverable.

## 7. Crash handling (FR-042)

If the child exits unexpectedly:
1. Any in-flight tool call returns `ToolResult::error` (server disconnected).
2. The server's `status` becomes `Disconnected`.
3. Subsequent calls to that server return `ToolResult::error("MCP server '{name}' disconnected")`.
4. The episode continues; other servers and built-in tools remain available.

## 8. Credential non-leakage (SC-028)

- `token_env` values (remote) and any stripped secret MUST NOT appear in: the transcript, log output,
  or a stdio child's environment.
- The stdio child's environment is asserted in tests to contain none of the harness's credential
  variable names with real values.
