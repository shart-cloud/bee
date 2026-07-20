# Contract: `Model` provider seam

The stable interface every LLM backend implements. `bee-harness` code and tests depend on this trait,
**not** on Rig types (research H3). Two impls ship in slice 1.

```rust
#[async_trait::async_trait]
pub trait Model: Send + Sync {
    /// Stable "<provider>/<model>" id recorded in the transcript, e.g. "anthropic/claude-opus-4-8".
    fn id(&self) -> &str;

    /// One model turn: send the conversation + available tool schemas, get back text and/or tool calls.
    /// MUST NOT retry internally on transient errors — the loop owns retry/backoff (FR-017).
    async fn complete(
        &self,
        convo: &Conversation,
        tools: &[ToolSchema],
    ) -> Result<Turn, ModelError>;
}
```

### Guarantees the loop relies on
- `complete` performs **exactly one** provider round-trip; it does not execute tools or loop.
- On a transient provider failure it returns `ModelError::Transient{..}` (never blocks/sleeps); the
  loop decides backoff and the `api_error` terminal status.
- `Turn.tool_calls` preserves provider order; each `ToolCall.id` is unique within the turn and is the
  key the loop uses to attach the `ToolResult`.
- Unparsable provider output → `ModelError::Decode`, not a panic.

### `RigModel` (slice 1)
- Wraps a Rig `CompletionModel` (Anthropic or `openai-compat`).
- Maps `Conversation` → Rig `CompletionRequest { preamble, chat_history, tools }` and
  `CompletionResponse` (`AssistantContent::{Text,ToolCall}`) → `Turn`.
- Model string passed verbatim to `client.completion_model(<model>)` (H8).

### `MockModel` (slice 1 — tests)
- Constructed from a script: `Vec<Turn>` (or a closure `Fn(&Conversation) -> Turn`).
- Returns successive scripted turns; ignores `tools` except to echo names when asserting schemas.
- Enables deterministic, offline tests of the loop, tools, truncation, malformed args, and
  `no_tool_calls`.

### Conformance tests (host, MockModel-independent)
1. A `RigModel` built for `openai-compat` with `base_url=http://localhost:11434/v1` returns a `Turn`
   for a trivial prompt (Ollama smoke, opt-in).
2. Given a scripted assistant turn containing one `ToolCall`, `complete` surfaces it with matching
   `id/name/arguments`.
3. A provider 429 maps to `ModelError::Transient` with `retry_after` when the header is present.
