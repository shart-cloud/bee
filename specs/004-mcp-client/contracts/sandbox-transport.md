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

## 3. Transport wiring — rmcp 2.x (corrected per research R1–R4)

> The `rmcp` API and version were corrected during Phase 0. Use **`rmcp = "2.2"`**,
> `default-features = false`, features `["client", "transport-child-process"]`. The spec's
> `SandboxedChildTransport` custom struct is **not** needed — rmcp accepts a pre-built
> `tokio::process::Command`.

### 3.1 Primary path — `TokioChildProcess::new` with a sandbox-built command

```
connect_stdio(sandbox, config):
    std_cmd = sandbox.tool_command(config.command, &config.args)?   // cgroup-join pre_exec + strip + harden
    for (k, v) in &config.env: std_cmd.env(k, v)                    // additive; never re-adds a secret silently
    tok_cmd = tokio::process::Command::from(std_cmd)                // preserves pre_exec
    child_transport = TokioChildProcess::new(tok_cmd)?              // rmcp; pipes stdio, spawns
    running = ().serve(child_transport).await?                      // MCP `initialize`; RunningService<RoleClient, ()>
```

- `().serve(...)` uses the default `ClientHandler for ()`. When `tools/list_changed` (FR-043) is
  implemented, `()` is replaced by a struct impl'ing `rmcp::ClientHandler` whose
  `on_tool_list_changed` refreshes the cached schemas.
- `TokioChildProcess::new` returns `Result` (it spawns) — propagate with `?`.
- The `RunningService` owns the child + background task; dropping it kills the child (kill-on-drop),
  satisfying teardown and FR-042.

### 3.2 The cgroup-join gate (R3 spike — MUST run before this path ships)

`TokioChildProcess` wraps the command in `process-wrap`'s `CommandWrap`, which **may install its own
`pre_exec`** for process-group management. If that overrides or precedes bee's cgroup-join closure,
the server would run **outside** the scope — a silent Constitution III violation. Before shipping §3.1,
a spike MUST prove, under `--features enforce`, that the spawned child's cgroup **equals the scope
cgroup** — e.g. read `/proc/<child_pid>/cgroup`, or assert a deliberately-denied I/O by the child
produces a `file_open` denial filtered to `scope.cgroup_id`. Fail-closed: if it cannot be proven,
§3.1 MUST NOT be used under enforce — use §3.3.

### 3.3 Fallback path — raw pipes (if the §3.2 spike fails)

bee spawns the child itself and hands rmcp only the byte streams, so `process-wrap` never touches the
spawn and bee's `pre_exec` is the only one installed:

```
std_cmd = sandbox.tool_command(config.command, &config.args)?
tok_cmd = tokio::process::Command::from(std_cmd)
tok_cmd.stdin(piped).stdout(piped).stderr(null).kill_on_drop(true)
child = tok_cmd.spawn()?                                   // bee owns the Child (kill-on-drop)
transport = (child.stdout.take(), child.stdin.take())      // (AsyncRead, AsyncWrite) pair
running = ().serve(transport).await?
```

- `stderr` is `null` (never stdout — that is the JSON-RPC framing channel).
- bee retains the `tokio::process::Child` for explicit kill on teardown/crash.
- Requires the rmcp feature providing the async-read/write pair transport; confirm during the spike.

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
