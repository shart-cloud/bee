# Phase 1 Data Model: bee LLM Agent Harness (Slice 1 — US1)

All types live in `bee-harness` unless noted. `serde` derive on everything that lands in the
transcript. Types marked *(US3/US4)* are reserved shape only — not built this slice.

## Provider seam

### `Model` (trait)
The provider abstraction (research H3). One method; both `RigModel` and `MockModel` implement it.

| Item | Type | Notes |
|------|------|-------|
| `id()` | `&str` | Stable id, `"<provider>/<model>"`, e.g. `anthropic/claude-opus-4-8`. Goes in the transcript. |
| `complete(convo, tools)` | `async fn(&Conversation, &[ToolSchema]) -> Result<Turn, ModelError>` | One model turn. |

### `Conversation`
Ordered message log the loop maintains and re-sends each turn.

| Field | Type | Notes |
|-------|------|-------|
| `system` | `String` | Scenario system prompt (Rig `preamble`). |
| `messages` | `Vec<Message>` | User/assistant/tool-result turns in order. |

### `Message`
| Variant | Fields | Notes |
|---------|--------|-------|
| `User` | `text: String` | Initial task and any plain user text. |
| `Assistant` | `text: Option<String>, tool_calls: Vec<ToolCall>` | A model turn as recorded. |
| `ToolResult` | `call_id: String, name: String, content: String, is_error: bool` | Result fed back to the model. |

### `Turn`
Return of `Model::complete` — a single model response.

| Field | Type | Notes |
|-------|------|-------|
| `text` | `Option<String>` | Assistant prose, if any. |
| `tool_calls` | `Vec<ToolCall>` | Empty ⇒ candidate `no_tool_calls`. |
| `stop` | `StopReason` | `EndTurn \| ToolUse \| MaxTokens \| Other(String)`. |
| `usage` | `Option<Usage>` | `input_tokens`, `output_tokens` when the provider reports them. |

### `ToolCall`
| Field | Type | Notes |
|-------|------|-------|
| `id` | `String` | Provider-assigned call id (correlates the result). |
| `name` | `String` | Tool name; must match a registered `Tool`. |
| `arguments` | `serde_json::Value` | Raw args; validated per-tool (may be malformed → FR-016). |

### `ModelError`
`Transient { status: u16, retry_after: Option<Duration> }` (429/5xx → backoff, FR-017) · `Auth` ·
`Request(String)` (malformed request / 4xx) · `Decode(String)` (unparsable response).

## Tools

### `Tool` (trait)
| Item | Type | Notes |
|------|------|-------|
| `name()` | `&'static str` | e.g. `"bash"`. |
| `schema()` | `ToolSchema` | `{ name, description, parameters: serde_json::Value }` (JSON Schema → Rig `ToolDefinition`). |
| `call(args, scope)` | `async fn(serde_json::Value, &Scope) -> ToolResult` | Executes **inside the scope**; never panics on bad args (returns an error `ToolResult`). |

Slice-1 tools: `bash` (`{ command: string }`), `read_file` (`{ path: string }`), `write_file`
(`{ path: string, content: string }`), `list_directory` (`{ path: string }`). *(US3)* `submit_flag`,
`give_up`.

### `ToolResult`
| Field | Type | Notes |
|-------|------|-------|
| `content` | `String` | stdout (+ formatted stderr/exit for `bash`), or the file/dir payload. |
| `exit_code` | `Option<i32>` | For process tools. |
| `is_error` | `bool` | True for a kernel denial (`EACCES`), non-zero exit, or a malformed-arg error. |
| `truncated` | `bool` | True if `content` was capped (FR-015). |
| `original_len` | `Option<usize>` | Pre-truncation byte length when `truncated`. |

## Scenario & config

### `Scenario` (TOML, FR-011)
| Field | Type | Notes |
|-------|------|-------|
| `id` | `String` | Scenario identifier. |
| `policy_path` | `PathBuf` | bee security policy (feature 001 TOML) for the scope. |
| `workdir` | `WorkdirSetup` | Files/dirs to create before the run; optional planted flag *(US3)*. |
| `system_prompt` | `String` | Agent framing. |
| `task` | `String` | First user message. |
| `turn_limit` | `u32` | Max tool-call rounds (FR-007). |
| `timeout` | `Duration` (secs in TOML) | Wall-clock cap (FR-007). |
| `tools` | `Vec<String>` | Enabled tool names (default: `[bash, read_file, write_file, list_directory]`). |
| `mode` | `ScoringMode` | `Standard` \| `Ctf` *(US3)*. |

### `ProviderConfig` (TOML)
| Field | Type | Notes |
|-------|------|-------|
| `provider` | `ProviderType` | `Anthropic` \| `OpenAiCompat`. |
| `base_url` | `Option<String>` | Required for `OpenAiCompat` (Ollama/OpenRouter/vLLM/OpenAI). Ignored for Anthropic. |
| `model` | `String` | Passed verbatim to `completion_model()` (H8 — current IDs). |
| `api_key_env` | `String` | **Name** of the env var holding the key (never the key itself). |
| `max_tokens` | `Option<u32>` | Provider max output tokens. |
| `temperature` | `Option<f32>` | Optional. |

**Credential rule (FR-018)**: the key is read from `api_key_env` at startup into the harness only;
`api_key_env` (and the known provider key vars) are stripped from every tool child's environment, and
the harness is marked non-dumpable (research H7).

## Episode

### `Episode` (lifecycle, in-memory)
State machine:

```text
Setup ──ok──▶ Running ──(done | turn_limit | give_up)──▶ Scoring ──▶ Done
  │                │
  │ scope/init err │ timeout / api_error(after retries)
  ▼                ▼
infra_error     terminate children + teardown ──▶ Done(status)
```

| Field | Type | Notes |
|-------|------|-------|
| `scenario` | `Scenario` | — |
| `model` | `Box<dyn Model>` | The provider under test. |
| `scope` | `Option<Scope>` | bee scope (None until Setup succeeds). |
| `reader` | `AuditReader` | Sync drain per tool call (H5). |
| `deadline` | `Instant` | `start + scenario.timeout`. |

### `EpisodeTranscript` (serde → JSON, FR-009)
| Field | Type | Notes |
|-------|------|-------|
| `scenario_id` | `String` | — |
| `model_id` | `String` | `Model::id()`. |
| `status` | `EpisodeStatus` | see below. |
| `turns` | `Vec<TranscriptTurn>` | Per-turn: assistant text, tool calls, results, and the **audit events correlated to each call**. |
| `audit_trail` | `Vec<AuditEvent>` | Full episode audit (bee-core `AuditEvent`), also inlined per turn. |
| `timing` | `Timing` | `started_at`, `ended_at`, per-turn durations, total. |
| `usage` | `Option<Usage>` | Summed token usage when reported. |
| `score` | `Option<ScoreReport>` | *(US3)* None in slice 1. |

### `EpisodeStatus` (enum)
`Completed` · `Timeout` · `NoToolCalls` · `ApiError { detail }` · `InfraError { detail }` ·
*(US3)* `Captured { value }` · `NotCaptured { techniques }`.

### `ScoreReport` *(US3 — reserved)*
`flag_captured: bool`, `coverage: f32`, `novelty: f32`, `parsimony: f32`. Computed post-episode from
the audit trail; not built this slice.

## Reused / referenced (feature 001)
- `bee_userspace::Engine`, `Scope`, `AuditReader` — scope lifecycle + sync audit drain.
- `bee_core::AuditEvent` — the serde audit record embedded in the transcript.
- `bee_core::Policy` / `PolicySet` — compiled from `scenario.policy_path`.
