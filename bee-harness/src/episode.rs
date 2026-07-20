//! The bee-owned multi-turn loop (research H2) and the `run_episode` entry point.
//!
//! The loop — not Rig — owns turn/time limits (FR-007), per-tool-call audit correlation (FR-008),
//! truncation (FR-015), malformed-arg handling (FR-016), retry/backoff (FR-017), and the terminal
//! status. [`run_loop`] is the pure loop over a ready [`Sandbox`] (host-testable with `MockModel`);
//! [`run_episode`] builds the sandbox (a real bee scope under `--features enforce`), materializes
//! the workdir, runs the loop, and tears down.

use std::time::{Duration, Instant};

use time::OffsetDateTime;

use crate::provider::{Conversation, Message, Model, ModelError, Turn, Usage};
use crate::sandbox::{self, Sandbox};
use crate::scenario::{Scenario, ScoringMode, WorkdirSetup};
use crate::tools::{self, ToolRegistry};
use crate::transcript::{EpisodeStatus, EpisodeTranscript, RecordedCall, Timing, TranscriptTurn};

/// A sink for human-readable live progress lines (research-agnostic; the CLI wires it to stderr).
/// The loop owns the formatting; the sink just renders. `None` ⇒ silent (the default).
pub type ProgressSink = Box<dyn Fn(&str) + Send + Sync>;

/// Loop tuning knobs.
pub struct LoopOptions {
    /// Max retries for transient (429/5xx) provider errors before `api_error` (FR-017).
    pub max_retries: u32,
    /// Optional live-progress sink (per-turn / per-tool-call / per-denial lines).
    pub progress: Option<ProgressSink>,
}

impl Default for LoopOptions {
    fn default() -> Self {
        LoopOptions {
            max_retries: 5,
            progress: None,
        }
    }
}

fn emit(opts: &LoopOptions, msg: impl AsRef<str>) {
    if let Some(sink) = &opts.progress {
        sink(msg.as_ref());
    }
}

/// First meaningful line of `s`, clipped to `max` chars — for compact progress lines. Skips blank
/// lines and the `[stderr]` section marker `run_child` inserts, so a denial shows its actual message.
fn preview(s: &str, max: usize) -> String {
    let first = s
        .lines()
        .map(str::trim_end)
        .find(|l| !l.is_empty() && *l != "[stderr]")
        .unwrap_or("");
    if first.chars().count() > max {
        let clipped: String = first.chars().take(max).collect();
        format!("{clipped}…")
    } else {
        first.to_string()
    }
}

fn status_label(s: &EpisodeStatus) -> String {
    match s {
        EpisodeStatus::Completed => "completed".into(),
        EpisodeStatus::Timeout => "timeout".into(),
        EpisodeStatus::NoToolCalls => "no_tool_calls".into(),
        EpisodeStatus::ApiError { detail } => format!("api_error ({detail})"),
        EpisodeStatus::InfraError { detail } => format!("infra_error ({detail})"),
        EpisodeStatus::Captured { turn } => format!("captured (turn {turn})"),
        EpisodeStatus::NotCaptured => "not_captured".into(),
    }
}

fn ms_since(t: Instant) -> u64 {
    t.elapsed().as_millis() as u64
}

fn rfc3339(t: OffsetDateTime) -> String {
    t.format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

/// One provider round-trip with bounded exponential backoff on transient errors (FR-017). Never
/// sleeps past `deadline`. Returns a human-readable detail string on terminal failure.
async fn complete_with_retry(
    model: &dyn Model,
    convo: &Conversation,
    schemas: &[crate::provider::ToolSchema],
    max_retries: u32,
    deadline: Instant,
) -> Result<Turn, String> {
    let mut attempt: u32 = 0;
    loop {
        match model.complete(convo, schemas).await {
            Ok(turn) => return Ok(turn),
            Err(ModelError::Transient {
                status,
                retry_after,
            }) => {
                if attempt >= max_retries {
                    return Err(format!(
                        "transient provider error (status {status}) after {attempt} retries"
                    ));
                }
                let backoff = retry_after
                    .unwrap_or_else(|| Duration::from_millis(200 * 2u64.pow(attempt.min(6))));
                if Instant::now() + backoff >= deadline {
                    return Err(format!(
                        "transient provider error (status {status}); deadline reached before retry"
                    ));
                }
                tokio::time::sleep(backoff).await;
                attempt += 1;
            }
            Err(ModelError::Auth) => return Err("provider authentication failed".to_string()),
            Err(ModelError::Request(m)) => return Err(format!("provider rejected request: {m}")),
            Err(ModelError::Decode(m)) => return Err(format!("could not decode response: {m}")),
        }
    }
}

/// Run the loop over a ready sandbox. The sandbox is `&mut` so audit can be drained between calls.
pub async fn run_loop(
    model: &dyn Model,
    scenario: &Scenario,
    registry: &ToolRegistry,
    sandbox: &mut Sandbox,
    opts: &LoopOptions,
) -> EpisodeTranscript {
    let started_at = OffsetDateTime::now_utc();
    let start = Instant::now();
    let deadline = start + scenario.timeout();

    emit(
        opts,
        format!(
            "episode '{}' · {} · tools: {}",
            scenario.id,
            model.id(),
            scenario.tools.join(",")
        ),
    );

    let mut convo = Conversation::new(&scenario.system_prompt, &scenario.task);
    let schemas = registry.schemas();

    let mut turns: Vec<TranscriptTurn> = Vec::new();
    let mut audit_trail = Vec::new();
    let mut usage_total: Option<Usage> = None;
    let mut any_tool_called = false;

    // If the loop runs out of turns without the agent stopping, it hit the turn limit (FR-007). In
    // CTF mode (US3) a turn-limit-expired episode means the agent *failed* to capture the flag, so
    // it is `NotCaptured` rather than `Timeout` (which we reserve for the wall-clock deadline, set
    // explicitly at the deadline checks below — see US3 slice decision note).
    let mut status = match scenario.mode {
        ScoringMode::Ctf => EpisodeStatus::NotCaptured,
        ScoringMode::Standard => EpisodeStatus::Timeout,
    };

    'outer: for index in 0..scenario.turn_limit {
        let turn_start = Instant::now();
        if Instant::now() >= deadline {
            status = EpisodeStatus::Timeout;
            break;
        }

        let turn =
            match complete_with_retry(model, &convo, &schemas, opts.max_retries, deadline).await {
                Ok(t) => t,
                Err(detail) => {
                    status = EpisodeStatus::ApiError { detail };
                    break;
                }
            };

        if let Some(u) = turn.usage {
            usage_total = Some(usage_total.map_or(u, |acc| acc + u));
        }
        let assistant_text = turn.text.clone();

        emit(opts, format!("turn {index}"));
        if let Some(t) = &assistant_text {
            if !t.trim().is_empty() {
                emit(opts, format!("  text: {}", preview(t, 120)));
            }
        }

        // A turn with no tool calls terminates the episode: `completed` if the agent had already
        // used a tool (natural wrap-up, incl. US1 AS-1 where the op was denied), else `no_tool_calls`
        // (a model that never engages — SC-007, no hang).
        if turn.tool_calls.is_empty() {
            turns.push(TranscriptTurn {
                index,
                assistant_text: assistant_text.clone(),
                calls: Vec::new(),
                duration_ms: ms_since(turn_start),
            });
            convo.push(Message::Assistant {
                text: assistant_text,
                tool_calls: Vec::new(),
            });
            status = if any_tool_called {
                EpisodeStatus::Completed
            } else {
                EpisodeStatus::NoToolCalls
            };
            break;
        }

        convo.push(Message::Assistant {
            text: assistant_text.clone(),
            tool_calls: turn.tool_calls.clone(),
        });

        let mut recorded: Vec<RecordedCall> = Vec::new();
        for tc in &turn.tool_calls {
            any_tool_called = true;

            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                status = EpisodeStatus::Timeout;
                turns.push(TranscriptTurn {
                    index,
                    assistant_text,
                    calls: recorded,
                    duration_ms: ms_since(turn_start),
                });
                break 'outer;
            }

            emit(
                opts,
                format!(
                    "  call {} {}",
                    tc.name,
                    preview(&tc.arguments.to_string(), 100)
                ),
            );

            let (result, timed_out) =
                match tokio::time::timeout(remaining, registry.execute(tc, sandbox)).await {
                    Ok(r) => (r, false),
                    Err(_) => (
                        crate::tools::ToolResult::error(
                            "tool aborted: episode wall-clock deadline reached",
                        ),
                        true,
                    ),
                };

            // Correlate the kernel audit events this call produced (FR-008). `settle` lets the async
            // demux path (US4) deliver this call's events; it is a no-op for the sync paths.
            sandbox.settle().await;
            let audit = sandbox.drain_audit();
            audit_trail.extend(audit.iter().cloned());

            let denied = audit.iter().any(|e| e.decision == "denied");
            let tag = if denied {
                "DENIED"
            } else if result.is_error {
                "ERR"
            } else {
                "ok"
            };
            let exit = result
                .exit_code
                .map(|c| format!(" (exit {c})"))
                .unwrap_or_default();
            emit(
                opts,
                format!("  result {tag}{exit}: {}", preview(&result.content, 120)),
            );
            for e in &audit {
                if e.decision == "denied" {
                    emit(opts, format!("  audit: {} denied {}", e.op, e.target));
                }
            }

            convo.push(Message::ToolResult {
                call_id: tc.id.clone(),
                name: tc.name.clone(),
                content: result.content.clone(),
                is_error: result.is_error,
            });

            // A terminal tool (`submit_flag`/`give_up`, US3) ends the episode immediately. The loop
            // stays tool-name-agnostic except for the one distinction that decides the status:
            // `submit_flag` with a non-error (correct) result ⇒ Captured, anything else ⇒ NotCaptured.
            if result.terminal {
                status = if tc.name == "submit_flag" && !result.is_error {
                    EpisodeStatus::Captured { turn: index }
                } else {
                    EpisodeStatus::NotCaptured
                };
                recorded.push(RecordedCall {
                    call: tc.clone(),
                    result,
                    audit,
                });
                turns.push(TranscriptTurn {
                    index,
                    assistant_text,
                    calls: recorded,
                    duration_ms: ms_since(turn_start),
                });
                break 'outer;
            }

            recorded.push(RecordedCall {
                call: tc.clone(),
                result,
                audit,
            });

            if timed_out {
                status = EpisodeStatus::Timeout;
                turns.push(TranscriptTurn {
                    index,
                    assistant_text,
                    calls: recorded,
                    duration_ms: ms_since(turn_start),
                });
                break 'outer;
            }
        }

        turns.push(TranscriptTurn {
            index,
            assistant_text,
            calls: recorded,
            duration_ms: ms_since(turn_start),
        });
    }

    emit(
        opts,
        format!(
            "done: {} · {} turns · {}ms",
            status_label(&status),
            turns.len(),
            ms_since(start)
        ),
    );

    let ended_at = OffsetDateTime::now_utc();
    EpisodeTranscript {
        scenario_id: scenario.id.clone(),
        model_id: model.id().to_string(),
        status,
        turns,
        audit_trail,
        timing: Timing {
            started_at: rfc3339(started_at),
            ended_at: rfc3339(ended_at),
            total_ms: ms_since(start),
        },
        usage: usage_total,
        score: None,
    }
}

/// Build an `infra_error` transcript (setup failed; the loop never ran).
fn infra_error(scenario: &Scenario, model: &dyn Model, detail: String) -> EpisodeTranscript {
    let now = rfc3339(OffsetDateTime::now_utc());
    EpisodeTranscript {
        scenario_id: scenario.id.clone(),
        model_id: model.id().to_string(),
        status: EpisodeStatus::InfraError { detail },
        turns: Vec::new(),
        audit_trail: Vec::new(),
        timing: Timing {
            started_at: now.clone(),
            ended_at: now,
            total_ms: 0,
        },
        usage: None,
        score: None,
    }
}

/// Create the scenario's declared dirs/files (relative to the current working directory), plus the
/// planted CTF flag (US3) when one is declared.
pub(crate) fn materialize_workdir(w: &WorkdirSetup) -> std::io::Result<()> {
    for d in &w.create_dirs {
        std::fs::create_dir_all(d)?;
    }
    for f in &w.create_files {
        write_with_parents(&f.path, f.content.as_bytes())?;
    }
    if let Some(flag) = &w.flag {
        write_with_parents(&flag.path, flag.value.as_bytes())?;
    }
    Ok(())
}

/// Write `bytes` to `path`, creating any missing parent directories first.
fn write_with_parents(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    std::fs::write(path, bytes)
}

/// Run a full episode: build the sandbox (a real bee scope under `--features enforce`), materialize
/// the workdir, run the loop, tear down. `provider_key_env` names the provider key var so it (plus
/// the default key vars) is stripped from every tool child (FR-018).
pub async fn run_episode(
    model: &dyn Model,
    scenario: &Scenario,
    provider_key_env: Option<&str>,
    progress: Option<ProgressSink>,
) -> EpisodeTranscript {
    let flag = scenario.workdir.flag.as_ref().map(|f| f.value.as_str());
    let registry = tools::registry_for(&scenario.tools, flag);
    let strip_env = sandbox::key_vars(provider_key_env);
    let opts = LoopOptions {
        progress,
        ..LoopOptions::default()
    };

    if let Err(e) = materialize_workdir(&scenario.workdir) {
        return infra_error(scenario, model, format!("workdir setup failed: {e}"));
    }

    #[cfg(feature = "enforce")]
    let mut sandbox = match build_enforced_sandbox(scenario, strip_env) {
        Ok(s) => s,
        Err(detail) => return infra_error(scenario, model, detail),
    };
    #[cfg(not(feature = "enforce"))]
    let mut sandbox = Sandbox::host(strip_env);

    let mut transcript = run_loop(model, scenario, &registry, &mut sandbox, &opts).await;
    sandbox.teardown();

    // CTF episodes carry a score derived from the audit trail (US3).
    if scenario.mode == crate::scenario::ScoringMode::Ctf {
        transcript.score = Some(crate::transcript::ScoreReport::from_transcript(&transcript));
    }
    transcript
}

/// Bring up a real bee scope from the scenario policy and wrap it as an [`Sandbox::Enforced`].
#[cfg(feature = "enforce")]
fn build_enforced_sandbox(scenario: &Scenario, strip_env: Vec<String>) -> Result<Sandbox, String> {
    use bee_core::Policy;
    use bee_userspace::{EnforcementPlan, Engine, ScopeMode, SystemResolver};

    let policy = Policy::from_path(&scenario.policy_path)
        .map_err(|e| format!("policy {}: {e}", scenario.policy_path.display()))?;
    let resolver = SystemResolver::current();
    let compiled = policy
        .compile(&resolver)
        .map_err(|e| format!("policy compile: {e}"))?;
    let plan = EnforcementPlan::prepare(&compiled, ScopeMode::Enforce)
        .map_err(|e| format!("enforcement plan: {e}"))?;
    let mut engine = Engine::init().map_err(|e| format!("engine init: {e}"))?;

    let scope_id = format!(
        "bee-episode-{}-{}",
        sanitize(&scenario.id),
        std::process::id()
    );
    let scope = engine
        .create_scope(&scope_id, bee_userspace::cgroup::DEFAULT_PARENT, &plan)
        .map_err(|e| format!("create scope: {e}"))?;
    let reader = engine
        .take_audit_reader(&scope_id)
        .map_err(|e| format!("audit reader: {e}"))?;
    // `engine` moves into the sandbox so its eBPF programs stay attached for the whole episode.
    Ok(Sandbox::enforced(engine, scope, reader, strip_env))
}

#[cfg(feature = "enforce")]
fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}
