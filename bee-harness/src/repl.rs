//! Interactive REPL (US5): a human chats with a sandboxed coding agent in real time.
//!
//! Where [`crate::episode`] drives a *scripted* task to completion and stops, the REPL replaces that
//! outer driver with an interactive loop — the user types a message, the agent responds (making any
//! number of tool calls along the way), everything is shown, and control returns to the user for the
//! next message. The conversation persists for the whole session.
//!
//! The *inner* agent-turn loop is structurally similar to the episode loop but has different
//! semantics — text-only means "back to the user", not "episode ends"; there is no wall-clock
//! deadline; the conversation outlives a single exchange — so it is its own code here. It reuses the
//! same low-level building blocks: [`Model::complete`], [`ToolRegistry::execute`],
//! [`Sandbox::drain_audit`]. `episode.rs` is deliberately left untouched.

use std::time::{Duration, Instant};

use bee_core::AuditEvent;
use time::OffsetDateTime;

use crate::provider::{Conversation, Message, Model, ModelError, ToolSchema, Turn, Usage};
use crate::sandbox::Sandbox;
use crate::tools::{ToolRegistry, ToolResult};
use crate::transcript::{EpisodeStatus, EpisodeTranscript, RecordedCall, Timing, TranscriptTurn};

pub mod terminal;

pub use terminal::TerminalOutput;

/// Configuration for an interactive REPL session.
pub struct ReplConfig {
    /// System prompt for the session.
    pub system_prompt: String,
    /// Max model calls per user message before forcing a pause (safety cap).
    /// Prevents a runaway agent from making 200 API calls off one message.
    pub agent_turn_budget: u32,
    /// Per-tool-call timeout (seconds). Individual tool calls that hang are killed.
    pub tool_timeout_secs: u64,
    /// Max retries on transient provider errors (429/5xx).
    pub max_retries: u32,
}

impl Default for ReplConfig {
    fn default() -> Self {
        ReplConfig {
            system_prompt: "You are a coding agent operating in a sandbox. You have access to \
                            bash, read_file, write_file, and list_directory tools. Use them to \
                            help the user with their tasks."
                .to_string(),
            agent_turn_budget: 25,
            tool_timeout_secs: 30,
            max_retries: 3,
        }
    }
}

/// How the REPL renders to the user. The default impl ([`TerminalOutput`]) writes to stdout with
/// ANSI color; a `Vec<String>` collector makes the agent-turn loop testable without a terminal.
pub trait ReplOutput: Send + Sync {
    /// Assistant prose.
    fn assistant_text(&self, text: &str);
    /// A tool call the agent requested (before it runs).
    fn tool_call(&self, name: &str, arguments: &serde_json::Value);
    /// A tool call's result, plus any kernel audit events it produced.
    fn tool_result(&self, result: &ToolResult, audit: &[AuditEvent]);
    /// An error the user should see (provider failure, bad meta-command).
    fn error(&self, msg: &str);
    /// An informational line (banners, warnings, meta-command output).
    fn info(&self, msg: &str);
}

/// Why one user→agent exchange ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExchangeOutcome {
    /// The agent produced a text-only turn and yielded control back to the user.
    Responded,
    /// The agent hit `agent_turn_budget` model calls without stopping; control was returned anyway.
    BudgetExhausted,
    /// A provider error (after retries) ended the exchange.
    ApiError(String),
}

/// The result of running one user→agent exchange (used by [`run_repl`] and by tests).
pub struct ExchangeResult {
    /// Why the exchange ended.
    pub outcome: ExchangeOutcome,
    /// Model round-trips made during this exchange.
    pub model_calls: u32,
    /// Tool calls executed during this exchange.
    pub tool_calls: u32,
    /// Denials observed during this exchange.
    pub denials: u32,
    /// The turns recorded, for optional transcript building. Indices are exchange-local (0-based).
    pub turns: Vec<TranscriptTurn>,
    /// Audit events observed during this exchange, in order.
    pub audit: Vec<AuditEvent>,
    /// Token usage summed across this exchange's model calls, when the provider reports it.
    pub usage: Option<Usage>,
}

/// The session record produced when the REPL exits.
pub struct ReplSession {
    /// The full conversation (system prompt + every user/assistant/tool message).
    pub conversation: Conversation,
    /// Number of user messages sent (meta-commands excluded).
    pub exchanges: u32,
    pub total_tool_calls: u32,
    pub total_denials: u32,
    /// The session as an [`EpisodeTranscript`] (always `Some` once the session ends).
    pub transcript: Option<EpisodeTranscript>,
}

/// A parsed meta-command (a line beginning with `/`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetaCommand {
    /// `/quit` or `/exit` — end the session.
    Quit,
    /// `/save [path]` — write the session transcript to a JSON file.
    Save(Option<String>),
    /// `/audit` — show a summary of all audit events so far.
    Audit,
    /// `/clear` — clear the conversation history (keep the system prompt).
    Clear,
    /// `/help` — list available commands.
    Help,
    /// An unrecognized `/command`.
    Unknown(String),
}

/// Parse a line as a meta-command. Returns `None` for a line that is not a meta-command (i.e. a
/// normal user message to send to the model). Only lines beginning with `/` are meta-commands.
pub fn parse_meta_command(line: &str) -> Option<MetaCommand> {
    let line = line.trim();
    if !line.starts_with('/') {
        return None;
    }
    let mut parts = line.splitn(2, char::is_whitespace);
    let cmd = parts.next().unwrap_or("");
    let arg = parts.next().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    Some(match cmd {
        "/quit" | "/exit" => MetaCommand::Quit,
        "/save" => MetaCommand::Save(arg),
        "/audit" => MetaCommand::Audit,
        "/clear" => MetaCommand::Clear,
        "/help" => MetaCommand::Help,
        other => MetaCommand::Unknown(other.to_string()),
    })
}

const HELP_TEXT: &str = "commands:\n  \
    /help          show this help\n  \
    /save [path]   write the session transcript to a JSON file (default: bee-repl-session.json)\n  \
    /audit         summarize the audit events seen so far\n  \
    /clear         clear the conversation history (keeps the system prompt)\n  \
    /quit, /exit   end the session\n\
    anything else is sent to the agent as a message.";

/// One provider round-trip with bounded exponential backoff on transient (429/5xx) errors. Unlike
/// the episode's retry helper there is no wall-clock deadline — the user is waiting and can always
/// re-send — so this simply caps the number of attempts.
async fn complete_with_retry(
    model: &dyn Model,
    convo: &Conversation,
    schemas: &[ToolSchema],
    max_retries: u32,
) -> Result<Turn, String> {
    let mut attempt: u32 = 0;
    loop {
        match model.complete(convo, schemas).await {
            Ok(turn) => return Ok(turn),
            Err(ModelError::Transient { status, retry_after }) => {
                if attempt >= max_retries {
                    return Err(format!(
                        "transient provider error (status {status}) after {attempt} retries"
                    ));
                }
                let backoff = retry_after
                    .unwrap_or_else(|| Duration::from_millis(200 * 2u64.pow(attempt.min(6))));
                tokio::time::sleep(backoff).await;
                attempt += 1;
            }
            Err(ModelError::Auth) => return Err("provider authentication failed".to_string()),
            Err(ModelError::Request(m)) => return Err(format!("provider rejected request: {m}")),
            Err(ModelError::Decode(m)) => return Err(format!("could not decode response: {m}")),
        }
    }
}

/// Run one user→agent exchange: inject `user_message`, run the agent turn loop until the agent
/// responds with text only (control back to the user), the model-call budget is exhausted, or the
/// provider errors. The conversation is mutated in place and persists across exchanges.
///
/// This is the testable core of the REPL — [`run_repl`] wraps it with readline and meta-commands.
#[allow(clippy::too_many_arguments)]
pub async fn run_exchange(
    user_message: &str,
    model: &dyn Model,
    conversation: &mut Conversation,
    registry: &ToolRegistry,
    sandbox: &mut Sandbox,
    config: &ReplConfig,
    output: &dyn ReplOutput,
) -> ExchangeResult {
    conversation.push(Message::User { text: user_message.to_string() });

    let schemas = registry.schemas();
    let mut turns: Vec<TranscriptTurn> = Vec::new();
    let mut audit_all: Vec<AuditEvent> = Vec::new();
    let mut model_calls = 0u32;
    let mut tool_calls = 0u32;
    let mut denials = 0u32;
    let mut usage_total: Option<Usage> = None;

    for _step in 0..config.agent_turn_budget {
        let turn_start = Instant::now();
        let turn = match complete_with_retry(model, conversation, &schemas, config.max_retries).await
        {
            Ok(t) => t,
            Err(detail) => {
                output.error(&detail);
                return ExchangeResult {
                    outcome: ExchangeOutcome::ApiError(detail),
                    model_calls,
                    tool_calls,
                    denials,
                    turns,
                    audit: audit_all,
                    usage: usage_total,
                };
            }
        };
        model_calls += 1;
        if let Some(u) = turn.usage {
            usage_total = Some(usage_total.map_or(u, |acc| acc + u));
        }

        let assistant_text = turn.text.clone();
        if let Some(text) = &assistant_text {
            if !text.trim().is_empty() {
                output.assistant_text(text);
            }
        }

        // A turn with no tool calls means the agent is done responding — record it and hand control
        // back to the user (this is the REPL's "back to prompt", not the episode's "end of episode").
        if turn.tool_calls.is_empty() {
            conversation.push(Message::Assistant { text: turn.text, tool_calls: Vec::new() });
            turns.push(TranscriptTurn {
                index: turns.len() as u32,
                assistant_text,
                calls: Vec::new(),
                duration_ms: turn_start.elapsed().as_millis() as u64,
            });
            return ExchangeResult {
                outcome: ExchangeOutcome::Responded,
                model_calls,
                tool_calls,
                denials,
                turns,
                audit: audit_all,
                usage: usage_total,
            };
        }

        conversation.push(Message::Assistant {
            text: turn.text.clone(),
            tool_calls: turn.tool_calls.clone(),
        });

        let mut recorded: Vec<RecordedCall> = Vec::new();
        for tc in &turn.tool_calls {
            tool_calls += 1;
            output.tool_call(&tc.name, &tc.arguments);

            let result = match tokio::time::timeout(
                Duration::from_secs(config.tool_timeout_secs),
                registry.execute(tc, sandbox),
            )
            .await
            {
                Ok(r) => r,
                Err(_) => ToolResult::error(format!(
                    "tool timed out after {}s",
                    config.tool_timeout_secs
                )),
            };

            // Correlate the kernel audit events this call produced (FR-008). `settle` lets the async
            // demux path deliver this call's events; it is a no-op for the sync sandboxes.
            sandbox.settle().await;
            let audit = sandbox.drain_audit();
            denials += audit.iter().filter(|e| e.decision == "denied").count() as u32;
            output.tool_result(&result, &audit);

            conversation.push(Message::ToolResult {
                call_id: tc.id.clone(),
                name: tc.name.clone(),
                content: result.content.clone(),
                is_error: result.is_error,
            });
            audit_all.extend(audit.iter().cloned());
            recorded.push(RecordedCall { call: tc.clone(), result, audit });
        }

        turns.push(TranscriptTurn {
            index: turns.len() as u32,
            assistant_text,
            calls: recorded,
            duration_ms: turn_start.elapsed().as_millis() as u64,
        });
        // Loop back — feed the tool results to the model for another turn.
    }

    // Budget exhausted: the agent never produced a text-only turn. The conversation is intact; the
    // user can send another message and the agent can continue.
    output.info("[agent reached tool-call limit for this exchange — returning control to you]");
    ExchangeResult {
        outcome: ExchangeOutcome::BudgetExhausted,
        model_calls,
        tool_calls,
        denials,
        turns,
        audit: audit_all,
        usage: usage_total,
    }
}

/// A count formatted with its noun, pluralized: `1 exchange`, `3 exchanges`.
fn plural(n: u32, noun: &str) -> String {
    if n == 1 {
        format!("{n} {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

fn rfc3339(t: OffsetDateTime) -> String {
    t.format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

/// Build a session transcript from the accumulated turns. A REPL session always ends `Completed`
/// (there is no turn limit or deadline that could time it out).
fn build_transcript(
    model_id: &str,
    started_at: OffsetDateTime,
    start: Instant,
    turns: &[TranscriptTurn],
    audit_trail: &[AuditEvent],
    usage: Option<Usage>,
) -> EpisodeTranscript {
    EpisodeTranscript {
        scenario_id: "repl".to_string(),
        model_id: model_id.to_string(),
        status: EpisodeStatus::Completed,
        turns: turns.to_vec(),
        audit_trail: audit_trail.to_vec(),
        timing: Timing {
            started_at: rfc3339(started_at),
            ended_at: rfc3339(OffsetDateTime::now_utc()),
            total_ms: start.elapsed().as_millis() as u64,
        },
        usage,
        score: None,
    }
}

/// Summarize the audit events seen so far: total denials and a per-op breakdown.
fn audit_summary(audit: &[AuditEvent]) -> String {
    if audit.is_empty() {
        return "no audit events recorded yet.".to_string();
    }
    let denials = audit.iter().filter(|e| e.decision == "denied").count();
    let mut by_op: std::collections::BTreeMap<(&str, &str), u32> = Default::default();
    for e in audit {
        *by_op.entry((e.op.as_str(), e.decision.as_str())).or_default() += 1;
    }
    let mut out = format!(
        "{} total, {}:",
        plural(audit.len() as u32, "audit event"),
        plural(denials as u32, "denial")
    );
    for ((op, decision), n) in by_op {
        out.push_str(&format!("\n  {op} {decision}: {n}"));
    }
    out
}

/// Read one line of input on a blocking thread so the tokio runtime is never blocked by readline.
/// The editor is moved in and returned so its history persists across iterations.
async fn read_line(
    mut editor: rustyline::DefaultEditor,
) -> (rustyline::DefaultEditor, Result<String, rustyline::error::ReadlineError>) {
    tokio::task::spawn_blocking(move || {
        let line = editor.readline("bee> ");
        (editor, line)
    })
    .await
    .expect("readline task panicked")
}

/// Drive an interactive REPL session over a ready sandbox: read a user message, run the agent turn
/// loop (showing every assistant message, tool call, tool result, and audit denial), then return to
/// the prompt. Meta-commands (`/quit`, `/save`, `/audit`, `/clear`, `/help`) are handled inline.
/// Returns the [`ReplSession`] record when the user quits or EOF is reached.
pub async fn run_repl(
    model: &dyn Model,
    registry: &ToolRegistry,
    sandbox: &mut Sandbox,
    config: &ReplConfig,
    output: &dyn ReplOutput,
) -> ReplSession {
    let started_at = OffsetDateTime::now_utc();
    let start = Instant::now();

    let mut conversation =
        Conversation { system: config.system_prompt.clone(), messages: Vec::new() };
    let mut turns: Vec<TranscriptTurn> = Vec::new();
    let mut audit_trail: Vec<AuditEvent> = Vec::new();
    let mut usage_total: Option<Usage> = None;
    let mut exchanges = 0u32;
    let mut total_tool_calls = 0u32;
    let mut total_denials = 0u32;

    let mut editor = match rustyline::DefaultEditor::new() {
        Ok(e) => e,
        Err(e) => {
            output.error(&format!("could not initialize readline: {e}"));
            let transcript = build_transcript(
                model.id(),
                started_at,
                start,
                &turns,
                &audit_trail,
                usage_total,
            );
            return ReplSession {
                conversation,
                exchanges,
                total_tool_calls,
                total_denials,
                transcript: Some(transcript),
            };
        }
    };

    output.info(&format!("interactive session — {} — type /help for commands", model.id()));

    loop {
        let (ed, readline) = read_line(editor).await;
        editor = ed;
        let line = match readline {
            Ok(l) => l,
            // Ctrl-D (EOF) ends the session; Ctrl-C cancels the current line and re-prompts.
            Err(rustyline::error::ReadlineError::Eof) => break,
            Err(rustyline::error::ReadlineError::Interrupted) => continue,
            Err(e) => {
                output.error(&format!("input error: {e}"));
                break;
            }
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let _ = editor.add_history_entry(line);

        if let Some(cmd) = parse_meta_command(line) {
            match cmd {
                MetaCommand::Quit => break,
                MetaCommand::Help => output.info(HELP_TEXT),
                MetaCommand::Audit => output.info(&audit_summary(&audit_trail)),
                MetaCommand::Clear => {
                    conversation.messages.clear();
                    output.info("conversation history cleared (system prompt kept).");
                }
                MetaCommand::Save(path) => {
                    let path = path.unwrap_or_else(|| "bee-repl-session.json".to_string());
                    let transcript = build_transcript(
                        model.id(),
                        started_at,
                        start,
                        &turns,
                        &audit_trail,
                        usage_total,
                    );
                    match std::fs::write(&path, transcript.to_json()) {
                        Ok(()) => output.info(&format!("saved transcript to {path}")),
                        Err(e) => output.error(&format!("could not write {path}: {e}")),
                    }
                }
                MetaCommand::Unknown(c) => {
                    output.error(&format!("unknown command: {c} (try /help)"));
                }
            }
            continue;
        }

        let result =
            run_exchange(line, model, &mut conversation, registry, sandbox, config, output).await;
        exchanges += 1;
        total_tool_calls += result.tool_calls;
        total_denials += result.denials;
        if let Some(u) = result.usage {
            usage_total = Some(usage_total.map_or(u, |acc| acc + u));
        }
        // Re-index the exchange's turns into the session-wide sequence before accumulating.
        for mut t in result.turns {
            t.index = turns.len() as u32;
            turns.push(t);
        }
        audit_trail.extend(result.audit);
    }

    output.info(&format!(
        "session ended ({}, {}, {})",
        plural(exchanges, "exchange"),
        plural(total_tool_calls, "tool call"),
        plural(total_denials, "denial")
    ));

    let transcript =
        build_transcript(model.id(), started_at, start, &turns, &audit_trail, usage_total);
    ReplSession {
        conversation,
        exchanges,
        total_tool_calls,
        total_denials,
        transcript: Some(transcript),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::mock_model::MockModel;
    use crate::provider::ToolCall;
    use crate::sandbox;
    use crate::tools::registry_for;
    use std::sync::Mutex;

    /// A [`ReplOutput`] that captures every rendered line into a shared buffer.
    #[derive(Default)]
    struct Collector {
        lines: Mutex<Vec<String>>,
    }

    impl Collector {
        fn contains(&self, needle: &str) -> bool {
            self.lines.lock().unwrap().iter().any(|l| l.contains(needle))
        }
        fn dump(&self) -> String {
            self.lines.lock().unwrap().join("\n")
        }
    }

    impl ReplOutput for Collector {
        fn assistant_text(&self, text: &str) {
            self.lines.lock().unwrap().push(format!("TEXT {text}"));
        }
        fn tool_call(&self, name: &str, arguments: &serde_json::Value) {
            self.lines.lock().unwrap().push(format!("CALL {name} {arguments}"));
        }
        fn tool_result(&self, result: &ToolResult, audit: &[AuditEvent]) {
            let mut buf = self.lines.lock().unwrap();
            buf.push(format!("RESULT err={} {}", result.is_error, result.content));
            for e in audit {
                buf.push(format!("AUDIT {} {} {}", e.decision, e.op, e.target));
            }
        }
        fn error(&self, msg: &str) {
            self.lines.lock().unwrap().push(format!("ERROR {msg}"));
        }
        fn info(&self, msg: &str) {
            self.lines.lock().unwrap().push(format!("INFO {msg}"));
        }
    }

    fn host_sandbox() -> Sandbox {
        Sandbox::host(sandbox::key_vars(None))
    }

    fn bash_call(id: &str, command: &str) -> ToolCall {
        ToolCall {
            id: id.into(),
            name: "bash".into(),
            arguments: serde_json::json!({ "command": command }),
        }
    }

    async fn exchange(model: &dyn Model, config: &ReplConfig) -> (ExchangeResult, Collector) {
        let registry = registry_for(&["bash".to_string(), "read_file".to_string()], None);
        let mut sb = host_sandbox();
        let mut convo = Conversation { system: config.system_prompt.clone(), messages: Vec::new() };
        let out = Collector::default();
        let res =
            run_exchange("hello", model, &mut convo, &registry, &mut sb, config, &out).await;
        (res, out)
    }

    #[tokio::test]
    async fn agent_responds_with_text() {
        let model = MockModel::scripted(vec![Turn::text("Hi there, how can I help?")]);
        let (res, out) = exchange(&model, &ReplConfig::default()).await;
        assert_eq!(res.outcome, ExchangeOutcome::Responded);
        assert_eq!(res.tool_calls, 0);
        assert!(out.contains("Hi there"), "output: {}", out.dump());
    }

    #[tokio::test]
    async fn agent_calls_tool_then_responds() {
        let model = MockModel::scripted(vec![
            Turn::calls(vec![bash_call("1", "echo hello-from-tool")]),
            Turn::text("Done — the tool printed a greeting."),
        ]);
        let (res, out) = exchange(&model, &ReplConfig::default()).await;
        assert_eq!(res.outcome, ExchangeOutcome::Responded);
        assert_eq!(res.tool_calls, 1);
        assert!(out.contains("CALL bash"), "output: {}", out.dump());
        assert!(out.contains("hello-from-tool"), "output: {}", out.dump());
        assert!(out.contains("Done —"), "output: {}", out.dump());
    }

    #[tokio::test]
    async fn agent_budget_exhausted() {
        // The model always wants another tool call, never text-only.
        let mut script = Vec::new();
        for i in 0..10 {
            script.push(Turn::calls(vec![bash_call(&i.to_string(), "echo loop")]));
        }
        let model = MockModel::scripted(script);
        let config = ReplConfig { agent_turn_budget: 3, ..ReplConfig::default() };
        let (res, out) = exchange(&model, &config).await;
        assert_eq!(res.outcome, ExchangeOutcome::BudgetExhausted);
        assert_eq!(res.model_calls, 3);
        assert_eq!(res.tool_calls, 3);
        assert!(out.contains("tool-call limit"), "output: {}", out.dump());
    }

    #[tokio::test]
    async fn denial_surfaces_in_output() {
        // In host mode this produces no real kernel denials, but it exercises the drain path: the
        // tool runs, audit is drained (empty on host), and the result is rendered. The enforcement
        // assertion is the VM's job.
        let model = MockModel::scripted(vec![
            Turn::calls(vec![ToolCall {
                id: "1".into(),
                name: "read_file".into(),
                arguments: serde_json::json!({ "path": "/etc/shadow" }),
            }]),
            Turn::text("I attempted to read the file."),
        ]);
        let (res, out) = exchange(&model, &ReplConfig::default()).await;
        assert_eq!(res.tool_calls, 1);
        // The drain path ran and a result was rendered (denial count is 0 on the host).
        assert_eq!(res.denials, 0);
        assert!(out.contains("RESULT"), "output: {}", out.dump());
    }

    #[test]
    fn meta_command_quit() {
        assert_eq!(parse_meta_command("/quit"), Some(MetaCommand::Quit));
        assert_eq!(parse_meta_command("/exit"), Some(MetaCommand::Quit));
        assert_eq!(parse_meta_command("  /quit  "), Some(MetaCommand::Quit));
    }

    #[test]
    fn meta_command_parsing() {
        assert_eq!(parse_meta_command("hello world"), None);
        assert_eq!(parse_meta_command("/help"), Some(MetaCommand::Help));
        assert_eq!(parse_meta_command("/audit"), Some(MetaCommand::Audit));
        assert_eq!(parse_meta_command("/clear"), Some(MetaCommand::Clear));
        assert_eq!(parse_meta_command("/save"), Some(MetaCommand::Save(None)));
        assert_eq!(
            parse_meta_command("/save out.json"),
            Some(MetaCommand::Save(Some("out.json".to_string())))
        );
        assert_eq!(
            parse_meta_command("/bogus"),
            Some(MetaCommand::Unknown("/bogus".to_string()))
        );
    }

    #[test]
    fn audit_summary_is_readable() {
        assert!(audit_summary(&[]).contains("no audit events"));
    }

    #[test]
    fn plural_agrees() {
        assert_eq!(plural(1, "denial"), "1 denial");
        assert_eq!(plural(0, "denial"), "0 denials");
        assert_eq!(plural(2, "exchange"), "2 exchanges");
    }
}
