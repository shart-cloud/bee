# Contract: Agent tools (slice 1)

Every tool executes as a **sandboxed child inside the episode's bee scope** (research H6) and returns a
`ToolResult`. A tool MUST NOT panic on bad arguments — it returns `is_error: true` with a message
(FR-016). Output is truncated to the configured cap (default 100 KB, FR-015) with `truncated`/
`original_len` set. A kernel denial (`EACCES`/`EPERM`) surfaces as `is_error: true` and the matching
audit event is correlated into the transcript (FR-008).

Each tool's `schema().parameters` is a JSON Schema advertised to the model as a Rig `ToolDefinition`.

## `bash`
- **parameters**: `{ "command": string }` (required).
- **behavior**: runs `sh -c <command>` in the scope; captures stdout, stderr, exit code.
- **result.content**: stdout, then `"\n[stderr]\n"+stderr` if non-empty; `exit_code` set; `is_error =
  exit_code != 0`.
- **denial**: if the command opens a policy-denied path, the child sees `EACCES` (e.g. `cat: …:
  Permission denied`); `is_error: true`; audit has the `file_open` deny.

## `read_file`
- **parameters**: `{ "path": string }` (required).
- **result.content**: file bytes as UTF-8 (lossy) up to the cap; `is_error: true` on open failure
  (denied / missing), with errno context.

## `write_file`
- **parameters**: `{ "path": string, "content": string }` (required).
- **behavior**: writes `content` to `path` (truncating) inside the scope.
- **result.content**: `"wrote N bytes to <path>"`; `is_error: true` if the write is denied (the
  read/write-mode enforcement from T033a) or fails.

## `list_directory`
- **parameters**: `{ "path": string }` (required).
- **result.content**: newline-separated entries (name + type); `is_error: true` on denied/missing dir.

## Malformed arguments (all tools, FR-016)
If `arguments` fails to deserialize into the tool's expected shape (missing field / wrong type /
invalid JSON), the tool returns `ToolResult { is_error: true, content: "<tool>: invalid arguments:
<detail>", .. }` — the loop feeds it back to the model and continues; it never crashes the episode.

## Enforcement note (Constitution III)
Tools do **not** pre-check paths against the policy in user space. They attempt the operation in the
scope and report whatever the kernel decided. The harness is orchestration; the LSM is the boundary.
