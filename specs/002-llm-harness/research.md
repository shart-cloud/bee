# Phase 0 Research: bee LLM Agent Harness (Slice 1 — US1)

Each entry records the **Decision**, **Rationale**, and **Alternatives considered**. Findings target
the first vertical slice (US1); later-slice notes are marked. Rig facts are grounded in `rig-core`
0.40.0 rustdoc (docs.rs) and the crate's documented usage.

## H1. Rig integration surface — single-turn completion only

**Decision**: Depend on `rig-core` 0.40 and use exactly one entry point: the low-level
`CompletionModel::completion(CompletionRequest) -> Result<CompletionResponse, _>`. Build the request
from `CompletionRequest { preamble: Option<String> (system prompt), chat_history: Vec<Message>,
tools: Vec<ToolDefinition>, .. }`. A tool schema is `ToolDefinition { name: String, description:
String, parameters: serde_json::Value (JSON Schema) }`. A tool call arrives as
`AssistantContent::ToolCall(ToolCall { id, name, arguments: serde_json::Value })`; a normal reply is
`AssistantContent::Text`. Tool results are appended to `chat_history` as `Message::User(..)` carrying
a tool-result content variant, and prior model turns as `Message::Assistant(..)`.

**Rationale**: This is the smallest, most stable slice of Rig's API and is provider-agnostic — every
Rig provider client yields a `CompletionModel`. Confining usage to one method bounds our exposure to
Rig's fast release cadence (0.36→0.40 in weeks; the spec flags breaking-change risk).

**Alternatives**: Rig's high-level `Agent` + `.multi_turn(N)` (rejected in H2); hand-rolling HTTP to
each provider (rejected — reimplements Rig's provider matrix and message/tool normalization).

## H2. bee-harness owns the multi-turn loop

**Decision**: The multi-turn loop lives in `bee-harness::episode` (not Rig). Pseudocode:

```text
setup scope (Engine::create_scope from the scenario policy); take AuditReader
convo = [system(scenario.system_prompt), user(scenario.task)]
for turn in 0..scenario.turn_limit (under a wall-clock deadline):
    resp = model.complete(&convo, &tool_schemas).await         # Rig, one turn
    record assistant turn (text + tool_calls) in transcript
    if resp.tool_calls.is_empty(): status = no_tool_calls; break
    for tc in resp.tool_calls:
        result = tools.execute(tc, &scope).await                # sandboxed child
        audit  = reader.drain().filter(cgroup == scope.cgroup)  # correlate (H5)
        record (tc, result, audit) in transcript
        convo.push(assistant tool_call); convo.push(user tool_result)
teardown scope; finalize transcript + status
```

**Rationale**: FR-007 (turn/time limits), FR-008 (per-tool-call audit correlation), FR-015
(truncation), FR-016 (malformed-arg handling), and the `no_tool_calls`/`api_error`/`timeout` statuses
are all loop-control concerns. Owning the loop makes them explicit and makes US4's audit interleaving
tractable. Rig's internal `multi_turn` would hide these seams.

**Alternatives**: Delegate to `Agent::multi_turn` with bee tools as Rig `Tool`s (rejected — least
code, but per-turn transcript/audit hooks and limits are constrained by Rig's loop internals).

## H3. Provider seam — a thin `Model` trait with two impls

**Decision**: Define a minimal in-crate trait rather than exposing Rig types across the harness:

```rust
#[async_trait] pub trait Model: Send + Sync {
    fn id(&self) -> &str;                          // "anthropic/claude-opus-4-8"
    async fn complete(&self, convo: &Conversation, tools: &[ToolSchema])
        -> Result<Turn, ModelError>;
}
```

`Turn { text: Option<String>, tool_calls: Vec<ToolCall>, stop: StopReason }`. Two impls ship in slice
1: `RigModel` (wraps a Rig `CompletionModel`; translates `Conversation`↔Rig `Message`/`ToolDefinition`
and `CompletionResponse`→`Turn`) and `MockModel` (returns scripted `Turn`s from a fixture).

**Rationale**: (a) Testability — `MockModel` lets every loop/tool/transcript test run without network
or API keys, which is essential given SC-007 (text-only model) and the deterministic US1 assertion.
(b) Insulation — Rig types stay behind the seam, so a Rig breaking change touches only `rig_model.rs`.
(c) The seam is where a future non-Rig or local-inference backend would plug in.

**Alternatives**: Use Rig's `CompletionModel` as the harness's own abstraction (rejected — couples all
harness code + tests to Rig; a Rig mock is heavier than a scripted `Turn`).

## H4. Two providers from the start — Anthropic + one configurable OpenAI-compatible client

**Decision**: `RigModel` supports two provider *types* selected by config: `anthropic`
(`rig::providers::anthropic::Client`) and `openai-compat` (`rig::providers::openai::Client` with a
**configurable `base_url`**). One `openai-compat` type covers real OpenAI, OpenRouter, Ollama, vLLM,
and Groq by pointing `base_url` at the right endpoint. Model is an **arbitrary string** passed to
`client.completion_model(<model>)` — we do NOT rely on Rig's model-name constants.

**Rationale**: FR-002. A single `base_url`-parameterized client maximizes reach for one impl and lets
CI/smoke tests target a local Ollama (`http://localhost:11434/v1`) with no paid key. Rig's constants
(e.g. `CLAUDE_SONNET_4_6`) lag real model availability; passing strings lets config name current
models.

**NEEDS-CONFIRM (at impl, from Rig source, non-blocking)**: the exact `openai::Client` builder call
that sets `base_url` — the `ClientBuilder` exists and custom-endpoint use is a documented Rig pattern;
confirm whether it is `Client::from_url(key, url)` or `Client::builder(key).base_url(url).build()`.

**Alternatives**: A distinct client type per vendor (rejected — N impls for one wire protocol); pin to
`api.openai.com` only (rejected — loses Ollama/local smoke-test path and endpoint reach).

## H5. Audit correlation — drain the sync reader between turns (no async layer yet)

**Decision**: Keep the existing synchronous `AuditReader` (feature 001). After each tool call, call
`reader.drain()` and filter events by the episode scope's `cgroup_id`, attributing them to that call.
Run the (brief, non-blocking) drain directly inside the tokio task; no `spawn_blocking` needed because
`drain()` is a non-blocking read of already-buffered ring records (the blocking `poll()` is not used
in this path). The feature-gated `tokio` `AsyncFd` audit **stream** (FR-012/013) is **deferred to US4**.

**Rationale**: Slice 1 is a single sequential episode = one scope = one reader; tool calls do not
overlap, so drain-and-filter-by-cgroup cleanly attributes denials to the call that caused them (US1
independent test). Avoiding the async layer keeps `bee-userspace`'s default build untouched
(FR-014/SC-005) and removes a whole subsystem from the first slice's risk.

**US4 note**: Concurrent episodes share one global `AUDIT_RB`; the async layer must demux a single
event stream to per-`cgroup_id` subscribers. That demux — not raw async I/O — is the real US4 work.

**Alternatives**: Build the async `AsyncFd` stream now (rejected — unneeded for a sequential episode
and would modify `bee-userspace` before it's required, risking the MVP regression gate).

## H6. Executing a tool inside the scope — one sandboxed child per call

**Decision**: Each tool call spawns one short-lived process confined to the episode's scope, reusing
001's join+harden `pre_exec` path. Build a `std::process::Command` (`sh -c <cmd>` for `bash`; a small
helper binary or direct syscalls for file tools), set `pre_exec(Scope::join_closure())` (joins the
scope cgroup + applies hardening), strip credential env vars (H7), then run it via
`tokio::process::Command::from(std_cmd).output().await` to capture stdout/stderr/exit asynchronously.
A per-call `tokio::time::timeout` bounds a hung tool; the episode deadline bounds the whole loop
(FR-007) and kills outstanding children + tears down the scope on expiry.

**Rationale**: Reuses the exact enforced spawn path already validated on the VM (Constitution III —
the kernel scope is the boundary), and `tokio::process::Command::from(std)` is the supported way to
keep `pre_exec` while getting async capture. One process per call gives clean audit correlation (H5)
and natural resource teardown.

**Alternatives**: A single long-lived in-scope shell serving all calls (rejected for slice 1 — muddies
per-call audit attribution and complicates timeout/kill semantics; reconsider for latency later).
`read_file`/`write_file` implemented as `bash` (`cat`/`printf`) rejected in favor of explicit tools
with structured args (FR-004) — but they still execute as sandboxed children so enforcement applies.

## H7. FR-018 — provider credentials unreadable from any scope

**Decision**: Two mechanisms, both fail-closed:
1. **Env strip**: every sandboxed tool child is spawned with provider key vars removed
   (`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `OPENROUTER_API_KEY`, and any configured key var). Extend
   the existing `LD_*` `env_remove` on the spawn `Command` to also clear these — the child never
   inherits them.
2. **Non-dumpable harness**: the harness process sets `prctl(PR_SET_DUMPABLE, 0)` at startup, so
   `/proc/<harness-pid>/environ` (and mem/maps) becomes root-only and a **same-uid** sandboxed child
   cannot read the harness's still-in-memory key. (bee already marks the *sandboxed* child
   non-dumpable; here it is the *harness* that must be non-dumpable.)

Optionally register the on-disk key file (if a provider config references one) via bee-core
`sensitive_paths`/protected defaults so any scope denies it.

**Rationale**: US3 makes the harness an explicit exfiltration target. Env is the obvious leak (children
inherit it) and `/proc/self`-style reads are the second; closing both is necessary and sufficient for
same-uid confinement. Running the harness as a *different* uid than sandboxed children would also close
#2 but is an operational burden — non-dumpable achieves it within one uid.

**Alternatives**: Rely only on the security policy denying `/proc` (rejected — brittle; `/proc/<pid>`
paths are dynamic and reads of one's own-uid environ don't route through `file_open` path rules
reliably). Keep keys only in a broker subprocess (rejected as over-engineered for slice 1; revisit if
a stronger isolation bar is set).

## H8. Anthropic model identifiers

**Decision**: Configuration names models as **current strings** — Opus 4.8 `claude-opus-4-8`, Sonnet 5
`claude-sonnet-5`, Haiku 4.5 `claude-haiku-4-5-20251001` — passed straight to
`completion_model(<str>)`. Do not use Rig's built-in constants. When implementing `rig_model.rs`, load
the `claude-api` skill to confirm current IDs, the Messages API `max_tokens` requirement, and
tool-use request/response shape.

**Rationale**: Rig's constants lag real availability; a stale `claude-3-5-*` default would silently run
the wrong model. Model choice is config data (Constitution IV).

**Alternatives**: Rig constants (rejected — lag); hardcode one model (rejected — config data).

## H9. Transcript + truncation

**Decision**: `EpisodeTranscript` is a `serde` struct (JSON out) capturing provider/model id, scenario
id, status, full conversation (messages + tool calls + results), the correlated audit trail, and
timing (FR-009). Tool results are truncated to a configurable byte cap (default 100 KB, FR-015) with an
explicit `truncated: true` marker and the original length, both in the model-visible result and the
transcript, keeping a 10-turn transcript < 1 MB (SC-006).

**Rationale**: The transcript is the product of an episode (eval/research artifact); truncation keeps
both the model context and the on-disk record bounded.

## H10. Loop edge cases

**Decision**: `no_tool_calls` when a turn yields only text and the agent isn't "done" (SC-007, no
hang); malformed tool args (bad JSON / missing fields) → a structured error returned to the model as
the tool result, loop continues (FR-016); LLM transient errors (429/5xx) → bounded exponential backoff
retry, then `api_error` (FR-017); scope/engine setup failure → `infra_error`, loop never starts.
Statuses: `completed | timeout | no_tool_calls | api_error | infra_error` (+ `captured | not_captured`
reserved for US3).

## H11. Testing strategy

**Decision**: (a) Host `cargo test` with `MockModel` scripting deterministic tool-call sequences —
covers the loop, tools' argument handling, truncation, malformed args, `no_tool_calls`, transcript
shape, and the FR-018 env-strip (assert a child cannot see the key var). (b) A VM harness case
(`episode-file-deny` in `remote-matrix.sh`): a `MockModel` scenario whose scripted `bash` call reads a
policy-denied path — assert the tool result carries `EACCES`, the transcript's audit trail has the
`file_open` deny, and the episode ends `completed`. (c) Opt-in live smoke: Ollama for the
openai-compat path (no key), Anthropic when `ANTHROPIC_API_KEY` is set. The default host build stays
runtime-free and requires no kernel (MockModel + no scope) except the VM case.

**Rationale**: Constitution's "a denial no test exercises is not enforced" — the enforcement assertion
runs on a real kernel; MockModel makes everything else deterministic and offline. Mirrors 001's
split (host unit tests + VM enforcement harness).
