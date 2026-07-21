//! [`RigModel`] — the real provider backend, wrapping a `rig-core` completion model (research H1/H4).
//!
//! Two provider *shapes* are selected by config: Anthropic (`rig_core::providers::anthropic`) and a
//! configurable OpenAI-compatible client (`rig_core::providers::openai` with a custom `base_url`, which
//! covers Ollama / OpenRouter / vLLM / real OpenAI). The concrete Rig model type is erased behind a
//! boxed async closure so both providers flow through one `complete` path and Rig types never leak
//! past this file (research H3).
//!
//! Model ids are passed **verbatim** to `completion_model(<str>)` — current ids come from config
//! (H8): `claude-opus-4-8`, `claude-sonnet-5`, `claude-haiku-4-5`, or any OpenAI-compatible model
//! string. Anthropic requires `max_tokens`, so we default it to 4096 when the config omits it.

use std::future::Future;
use std::pin::Pin;

use futures_util::StreamExt;
use rig_core::client::CompletionClient;
use rig_core::completion::{
    AssistantContent, CompletionError, CompletionModel, CompletionRequest, GetTokenUsage, Message,
    ToolDefinition,
};
use rig_core::streaming::{StreamedAssistantContent, StreamingCompletionResponse};
use rig_core::OneOrMany;

use crate::config::{ProviderConfig, ProviderType};
use crate::provider::{
    Conversation, EventStream, Message as HMessage, Model, ModelError, StopReason, StreamEvent,
    ToolCall, ToolSchema, Turn, Usage,
};

/// Default output token cap for providers (Anthropic *requires* `max_tokens`).
const DEFAULT_MAX_TOKENS: u64 = 4096;

/// What the erased completer returns: the assistant content choice + token usage.
struct RigTurn {
    choice: OneOrMany<AssistantContent>,
    usage: rig_core::completion::Usage,
}

type CompleterFut = Pin<Box<dyn Future<Output = Result<RigTurn, CompletionError>> + Send>>;
type Completer = Box<dyn Fn(CompletionRequest) -> CompleterFut + Send + Sync>;

type StreamerFut = Pin<Box<dyn Future<Output = Result<EventStream, ModelError>> + Send>>;
type Streamer = Box<dyn Fn(CompletionRequest) -> StreamerFut + Send + Sync>;

/// A [`Model`] backed by a Rig completion model.
pub struct RigModel {
    id: String,
    max_tokens: Option<u64>,
    temperature: Option<f64>,
    complete: Completer,
    stream: Streamer,
}

/// Erase a concrete Rig `CompletionModel` into a boxed async closure that yields a [`RigTurn`].
fn make_completer<M>(model: M) -> Completer
where
    M: CompletionModel + Clone + Send + Sync + 'static,
{
    Box::new(move |req: CompletionRequest| {
        let model = model.clone();
        Box::pin(async move {
            let resp = model.completion(req).await?;
            Ok(RigTurn {
                choice: resp.choice,
                usage: resp.usage,
            })
        })
    })
}

/// Erase a concrete Rig `CompletionModel` into a boxed async closure that opens a provider stream
/// and maps it into an [`EventStream`]. R (the provider's streaming-response type) is erased here,
/// where it is statically known, so no Rig type escapes this file (research H3).
fn make_streamer<M>(model: M) -> Streamer
where
    M: CompletionModel + Clone + Send + Sync + 'static,
    M::StreamingResponse: Send + 'static,
{
    Box::new(move |req: CompletionRequest| {
        let model = model.clone();
        Box::pin(async move {
            let resp = model.stream(req).await.map_err(to_model_error)?;
            Ok(map_rig_stream(resp))
        })
    })
}

/// Map a Rig streaming response into an [`EventStream`]: text deltas are forwarded live, each
/// complete tool call is forwarded as it lands, and a single [`StreamEvent::Done`] carrying the
/// assembled [`Turn`] is emitted when the provider stream ends. Partial tool-call deltas and
/// reasoning are not surfaced (matching the non-streaming [`to_turn`]).
fn map_rig_stream<R>(mut resp: StreamingCompletionResponse<R>) -> EventStream
where
    R: Clone + Unpin + GetTokenUsage + Send + 'static,
{
    Box::pin(async_stream::stream! {
        let mut text = String::new();
        let mut tool_calls: Vec<ToolCall> = Vec::new();
        let mut usage = Usage::default();

        while let Some(item) = resp.next().await {
            match item {
                Ok(StreamedAssistantContent::Text(t)) => {
                    text.push_str(&t.text);
                    yield Ok(StreamEvent::TextDelta(t.text));
                }
                Ok(StreamedAssistantContent::ToolCall { tool_call, .. }) => {
                    let tc = ToolCall {
                        id: tool_call.id,
                        name: tool_call.function.name,
                        arguments: tool_call.function.arguments,
                    };
                    tool_calls.push(tc.clone());
                    yield Ok(StreamEvent::ToolCall(tc));
                }
                Ok(StreamedAssistantContent::Final(r)) => {
                    usage = usage_from_rig(&r.token_usage());
                }
                // Partial tool-call deltas, reasoning, and provider-native items are not surfaced.
                Ok(_) => {}
                Err(e) => {
                    yield Err(to_model_error(e));
                    return;
                }
            }
        }

        // Some providers report usage only via the aggregated response after the stream drains,
        // not as a `Final` event; fall back to it when no `Final` carried usage.
        if usage == Usage::default() {
            if let Some(r) = &resp.response {
                usage = usage_from_rig(&r.token_usage());
            }
        }

        let stop = if tool_calls.is_empty() { StopReason::EndTurn } else { StopReason::ToolUse };
        let turn = Turn {
            text: (!text.is_empty()).then_some(text),
            tool_calls,
            stop,
            usage: Some(usage),
        };
        yield Ok(StreamEvent::Done(turn));
    })
}

impl RigModel {
    /// Build a `RigModel` from a provider config and a resolved API key. `api_key` may be empty for
    /// local OpenAI-compatible endpoints (e.g. Ollama) that don't authenticate.
    pub fn from_config(cfg: &ProviderConfig, api_key: &str) -> Result<Self, ModelError> {
        let id = cfg.model_id();
        // Build the completer and streamer from one model instance (both back-ends' models are
        // `Clone`), so a single provider selection serves both the buffered and streaming paths.
        let (complete, stream) = match cfg.provider {
            ProviderType::Anthropic => {
                let client = rig_core::providers::anthropic::Client::new(api_key)
                    .map_err(|e| ModelError::Request(format!("anthropic client: {e}")))?;
                let model = client.completion_model(&cfg.model);
                (make_completer(model.clone()), make_streamer(model))
            }
            ProviderType::OpenAiCompat => {
                let base_url = cfg
                    .base_url
                    .as_deref()
                    .ok_or_else(|| ModelError::Request("openai-compat requires base_url".into()))?;
                let client = rig_core::providers::openai::CompletionsClient::builder()
                    .base_url(base_url)
                    .api_key(api_key.to_string())
                    .build()
                    .map_err(|e| ModelError::Request(format!("openai client: {e}")))?;
                let model = client.completion_model(&cfg.model);
                (make_completer(model.clone()), make_streamer(model))
            }
            ProviderType::Mock => {
                return Err(ModelError::Request(
                    "RigModel does not serve the `mock` provider (use model_from_config)".into(),
                ))
            }
        };
        Ok(RigModel {
            id,
            max_tokens: Some(cfg.max_tokens.map(u64::from).unwrap_or(DEFAULT_MAX_TOKENS)),
            temperature: cfg.temperature.map(f64::from),
            complete,
            stream,
        })
    }

    /// Translate the harness conversation + tool schemas into a Rig [`CompletionRequest`].
    fn build_request(
        &self,
        convo: &Conversation,
        tools: &[ToolSchema],
    ) -> Result<CompletionRequest, ModelError> {
        to_completion_request(convo, tools, self.max_tokens, self.temperature)
    }
}

/// The pure `Conversation` + tool-schema → Rig [`CompletionRequest`] mapping, factored out of
/// [`RigModel`] so it is unit-testable without constructing a provider client (no network). This is
/// the seam most exposed to Rig's release cadence, so it has its own tests below.
fn to_completion_request(
    convo: &Conversation,
    tools: &[ToolSchema],
    max_tokens: Option<u64>,
    temperature: Option<f64>,
) -> Result<CompletionRequest, ModelError> {
    let mut history: Vec<Message> = Vec::new();
    for m in &convo.messages {
        match m {
            HMessage::User { text } => history.push(Message::user(text)),
            HMessage::Assistant { text, tool_calls } => {
                let mut content: Vec<AssistantContent> = Vec::new();
                if let Some(t) = text {
                    if !t.is_empty() {
                        content.push(AssistantContent::text(t));
                    }
                }
                for tc in tool_calls {
                    content.push(AssistantContent::tool_call(
                        tc.id.clone(),
                        tc.name.clone(),
                        tc.arguments.clone(),
                    ));
                }
                // An assistant turn always has text or tool calls; guard anyway.
                if let Ok(oom) = OneOrMany::many(content) {
                    history.push(Message::Assistant {
                        id: None,
                        content: oom,
                    });
                }
            }
            HMessage::ToolResult {
                call_id, content, ..
            } => {
                history.push(Message::tool_result(call_id, content.clone()));
            }
        }
    }

    let chat_history =
        OneOrMany::many(history).map_err(|_| ModelError::Request("empty conversation".into()))?;

    let tool_defs: Vec<ToolDefinition> = tools
        .iter()
        .map(|t| ToolDefinition {
            name: t.name.clone(),
            description: t.description.clone(),
            parameters: t.parameters.clone(),
        })
        .collect();

    Ok(CompletionRequest {
        model: None,
        preamble: Some(convo.system.clone()),
        chat_history,
        documents: Vec::new(),
        tools: tool_defs,
        temperature,
        max_tokens,
        tool_choice: None,
        additional_params: None,
        output_schema: None,
    })
}

/// Map a Rig completion response's assistant content into a harness [`Turn`].
fn to_turn(rt: RigTurn) -> Turn {
    let mut text: Option<String> = None;
    let mut tool_calls: Vec<ToolCall> = Vec::new();
    for item in rt.choice.into_iter() {
        match item {
            AssistantContent::Text(t) => {
                let s = text.get_or_insert_with(String::new);
                s.push_str(&t.text);
            }
            AssistantContent::ToolCall(tc) => tool_calls.push(ToolCall {
                id: tc.id,
                name: tc.function.name,
                arguments: tc.function.arguments,
            }),
            // Reasoning / image content is not surfaced to the loop.
            _ => {}
        }
    }
    let stop = if tool_calls.is_empty() {
        StopReason::EndTurn
    } else {
        StopReason::ToolUse
    };
    let usage = Some(usage_from_rig(&rt.usage));
    Turn {
        text,
        tool_calls,
        stop,
        usage,
    }
}

/// Map Rig's usage into the harness [`Usage`], carrying the cache and reasoning counts (dropped by
/// the older mapping) so cost can be computed accurately downstream.
fn usage_from_rig(u: &rig_core::completion::Usage) -> Usage {
    Usage {
        input_tokens: u.input_tokens as u32,
        output_tokens: u.output_tokens as u32,
        cache_read_tokens: u.cached_input_tokens as u32,
        cache_write_tokens: u.cache_creation_input_tokens as u32,
        reasoning_tokens: u.reasoning_tokens as u32,
    }
}

/// Map a Rig `CompletionError` into the harness [`ModelError`] (429/5xx → transient; the loop owns
/// backoff, FR-017).
fn to_model_error(e: CompletionError) -> ModelError {
    if let CompletionError::HttpError(_) = &e {
        // Preserve a captured HTTP status when there is one; otherwise treat a bare transport
        // failure (timeout / connection reset) as transient.
        if let Some(status) = e.provider_response_status() {
            return classify_status(status.as_u16(), &e);
        }
        return ModelError::Transient {
            status: 0,
            retry_after: None,
        };
    }
    if let Some(status) = e.provider_response_status() {
        return classify_status(status.as_u16(), &e);
    }
    match e {
        CompletionError::ResponseError(m) => ModelError::Decode(m),
        other => ModelError::Request(other.to_string()),
    }
}

fn classify_status(status: u16, e: &CompletionError) -> ModelError {
    match status {
        429 | 500..=599 => ModelError::Transient {
            status,
            retry_after: None,
        },
        401 | 403 => ModelError::Auth,
        _ => ModelError::Request(format!("status {status}: {e}")),
    }
}

#[async_trait::async_trait]
impl Model for RigModel {
    fn id(&self) -> &str {
        &self.id
    }

    async fn complete(
        &self,
        convo: &Conversation,
        tools: &[ToolSchema],
    ) -> Result<Turn, ModelError> {
        let req = self.build_request(convo, tools)?;
        let rt = (self.complete)(req).await.map_err(to_model_error)?;
        Ok(to_turn(rt))
    }

    async fn stream(
        &self,
        convo: &Conversation,
        tools: &[ToolSchema],
    ) -> Result<EventStream, ModelError> {
        let req = self.build_request(convo, tools)?;
        (self.stream)(req).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{Conversation, Message as HMessage, ToolCall, ToolSchema};

    fn sample_convo() -> Conversation {
        Conversation {
            system: "SYSTEM-PROMPT".to_string(),
            messages: vec![
                HMessage::User {
                    text: "read the file".to_string(),
                },
                HMessage::Assistant {
                    text: Some("I'll read it.".to_string()),
                    tool_calls: vec![ToolCall {
                        id: "call-1".to_string(),
                        name: "read_file".to_string(),
                        arguments: serde_json::json!({ "path": "/etc/hostname" }),
                    }],
                },
                HMessage::ToolResult {
                    call_id: "call-1".to_string(),
                    name: "read_file".to_string(),
                    content: "myhost\n".to_string(),
                    is_error: false,
                },
            ],
        }
    }

    fn sample_tools() -> Vec<ToolSchema> {
        vec![ToolSchema {
            name: "read_file".to_string(),
            description: "Read a file".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "required": ["path"]
            }),
        }]
    }

    #[test]
    fn maps_system_tools_and_limits() {
        let req = to_completion_request(&sample_convo(), &sample_tools(), Some(4096), Some(0.0))
            .expect("build request");
        assert_eq!(req.preamble.as_deref(), Some("SYSTEM-PROMPT"));
        assert_eq!(req.max_tokens, Some(4096));
        assert_eq!(req.temperature, Some(0.0));
        assert_eq!(req.tools.len(), 1);
        assert_eq!(req.tools[0].name, "read_file");
        // system prompt is a preamble, not a chat_history entry.
        assert_eq!(req.chat_history.iter().count(), 3);
    }

    #[test]
    fn empty_conversation_is_rejected() {
        let convo = Conversation {
            system: "s".into(),
            messages: vec![],
        };
        assert!(matches!(
            to_completion_request(&convo, &[], None, None),
            Err(ModelError::Request(_))
        ));
    }

    // The load-bearing guard: assert the assistant tool-call and the tool-result survive Rig's OWN
    // serialization of `Message`/`AssistantContent`. If a Rig update changes how `tool_call`/
    // `tool_result` are constructed or serialized, these fields move/vanish and this test fails —
    // catching the drift before it silently breaks a live provider round-trip.
    #[test]
    fn tool_call_and_result_survive_rig_serialization() {
        let req = to_completion_request(&sample_convo(), &sample_tools(), Some(64), None)
            .expect("build request");
        let wire = serde_json::to_string(&req.chat_history).expect("serialize chat_history");

        // assistant text + tool call
        assert!(
            wire.contains("I'll read it."),
            "assistant text missing: {wire}"
        );
        assert!(wire.contains("read_file"), "tool name missing: {wire}");
        assert!(wire.contains("call-1"), "tool call id missing: {wire}");
        assert!(
            wire.contains("/etc/hostname"),
            "tool call arguments missing: {wire}"
        );
        // tool result fed back as user content, keyed by the same call id
        assert!(
            wire.contains("myhost"),
            "tool result content missing: {wire}"
        );
    }
}
