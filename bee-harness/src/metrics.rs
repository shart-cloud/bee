//! Usage/cost/performance metrics for LLM calls (in-harness; not a kernel concern).
//!
//! Every model call the harness makes can be recorded as one [`CallRecord`] appended to a global,
//! append-only JSONL log at `$XDG_STATE_HOME/bee/metrics/events.jsonl`, tagged with the project it
//! ran in. Records carry token counts, latency, time-to-first-token (streaming only), the derived
//! USD cost, and outcome — **never prompt content**, so the log is safe to keep. The [`report`]
//! module aggregates the log; [`pricing`] turns raw token counts into cost.
//!
//! Recording is opt-in per call site via a [`Recorder`]: the REPL and episode entry points build one
//! and pass it down, while the host tests that drive the loop directly pass `None`, so the suite
//! never writes to the real store.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::provider::{StopReason, Usage};

pub mod pricing;
pub mod report;

/// One recorded model call. Serialized one-per-line to the metrics log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallRecord {
    /// When the call completed (RFC 3339).
    pub ts: String,
    /// The session/episode this call belongs to (groups calls from one run).
    pub session_id: String,
    /// Which entry point produced it: `repl`, `episode`, or `batch`.
    pub kind: String,
    /// The working directory the run happened in (for per-project attribution).
    pub project: String,
    /// Provider shape, parsed from the model id prefix (`anthropic`, `openai-compat`, `mock`, …).
    pub provider: String,
    /// The full model id as the provider reports it (e.g. `anthropic/claude-opus-4-8`).
    pub model_id: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    #[serde(default)]
    pub cache_read_tokens: u32,
    #[serde(default)]
    pub cache_write_tokens: u32,
    #[serde(default)]
    pub reasoning_tokens: u32,
    /// Time to first streamed event, in ms — `None` for non-streaming (episode) calls.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttft_ms: Option<u64>,
    /// Total wall-clock for the call (dispatch → final token), in ms.
    pub latency_ms: u64,
    /// Output tokens per second over the call, when derivable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub throughput_tok_s: Option<f64>,
    /// Why the model stopped (`end_turn`, `tool_use`, …), or the error kind on a failed call.
    pub stop_reason: String,
    /// `ok` or `error:<kind>`.
    pub outcome: String,
    /// Derived USD cost, or `None` when the model is unpriced (local endpoints / mock).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    /// The pricing table version the cost was computed against.
    pub pricing_version: String,
}

/// Split a model id into `(provider, model)` on the first `/`. `anthropic/claude-opus-4-8` →
/// `("anthropic", "claude-opus-4-8")`; an id with no slash is all model, provider `"unknown"`.
fn split_provider(model_id: &str) -> (&str, &str) {
    match model_id.split_once('/') {
        Some((p, _)) => (p, model_id),
        None => ("unknown", model_id),
    }
}

fn rfc3339(t: OffsetDateTime) -> String {
    t.format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

impl CallRecord {
    /// Assemble a record, deriving cost (via [`pricing`]) and throughput from the raw inputs.
    #[allow(clippy::too_many_arguments)]
    fn build(
        session_id: &str,
        kind: &str,
        project: &str,
        model_id: &str,
        usage: Usage,
        latency_ms: u64,
        ttft_ms: Option<u64>,
        stop_reason: &str,
        outcome: &str,
    ) -> Self {
        let (provider, _) = split_provider(model_id);
        // Throughput over the generation window: output tokens ÷ (latency after first token). Fall
        // back to whole-call latency when there is no TTFT (non-streaming).
        let gen_ms = match ttft_ms {
            Some(t) => latency_ms.saturating_sub(t),
            None => latency_ms,
        };
        let throughput_tok_s = (gen_ms > 0 && usage.output_tokens > 0)
            .then(|| usage.output_tokens as f64 / (gen_ms as f64 / 1000.0));
        CallRecord {
            ts: rfc3339(OffsetDateTime::now_utc()),
            session_id: session_id.to_string(),
            kind: kind.to_string(),
            project: project.to_string(),
            provider: provider.to_string(),
            model_id: model_id.to_string(),
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            cache_read_tokens: usage.cache_read_tokens,
            cache_write_tokens: usage.cache_write_tokens,
            reasoning_tokens: usage.reasoning_tokens,
            ttft_ms,
            latency_ms,
            throughput_tok_s,
            stop_reason: stop_reason.to_string(),
            outcome: outcome.to_string(),
            cost_usd: pricing::cost(model_id, &usage),
            pricing_version: pricing::PRICING_VERSION.to_string(),
        }
    }
}

/// The stop reason as the string stored in a record.
pub fn stop_label(stop: &StopReason) -> &'static str {
    match stop {
        StopReason::EndTurn => "end_turn",
        StopReason::ToolUse => "tool_use",
        StopReason::MaxTokens => "max_tokens",
        StopReason::Other(_) => "other",
    }
}

/// The metrics log path: `$XDG_STATE_HOME/bee/metrics/events.jsonl`, falling back to
/// `~/.local/state/bee/metrics/events.jsonl`. Creates the parent directory. `None` if neither
/// `XDG_STATE_HOME` nor `HOME` is set.
pub fn metrics_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
    let dir = base.join("bee").join("metrics");
    let _ = std::fs::create_dir_all(&dir);
    Some(dir.join("events.jsonl"))
}

/// The current working directory as a string, for per-project attribution. `"?"` if unavailable.
fn current_project() -> String {
    std::env::current_dir()
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "?".to_string())
}

/// Records one run's model calls to the metrics log. Construct once per run and pass it to the loop;
/// each `record` call appends a single line. Errors are swallowed — metrics are best-effort and must
/// never take down a session.
pub struct Recorder {
    path: PathBuf,
    session_id: String,
    kind: String,
    project: String,
}

impl Recorder {
    /// Build a recorder for a run of `kind` (`repl`/`episode`/`batch`) identified by `session_id`.
    /// Returns `None` when no metrics path is resolvable (recording then simply doesn't happen).
    pub fn new(kind: &str, session_id: impl Into<String>) -> Option<Self> {
        Some(Recorder {
            path: metrics_path()?,
            session_id: session_id.into(),
            kind: kind.to_string(),
            project: current_project(),
        })
    }

    /// Append one call to the log.
    #[allow(clippy::too_many_arguments)]
    pub fn record(
        &self,
        model_id: &str,
        usage: Usage,
        latency_ms: u64,
        ttft_ms: Option<u64>,
        stop_reason: &str,
        outcome: &str,
    ) {
        let rec = CallRecord::build(
            &self.session_id,
            &self.kind,
            &self.project,
            model_id,
            usage,
            latency_ms,
            ttft_ms,
            stop_reason,
            outcome,
        );
        append(&self.path, &rec);
    }
}

/// Append one record to `path` as a single JSON line (atomic for our line sizes on POSIX).
fn append(path: &PathBuf, rec: &CallRecord) {
    let Ok(mut line) = serde_json::to_string(rec) else {
        return;
    };
    line.push('\n');
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = f.write_all(line.as_bytes());
    }
}

/// Read every record from the log at `path`, skipping unparseable lines (forward-compatible with
/// records written by a newer schema).
pub fn read_log(path: &PathBuf) -> Vec<CallRecord> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<CallRecord>(l).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_usage() -> Usage {
        Usage {
            input_tokens: 1000,
            output_tokens: 500,
            cache_read_tokens: 4000,
            cache_write_tokens: 200,
            reasoning_tokens: 100,
        }
    }

    #[test]
    fn provider_split() {
        assert_eq!(split_provider("anthropic/claude-opus-4-8").0, "anthropic");
        assert_eq!(split_provider("mock/scripted").0, "mock");
        assert_eq!(split_provider("bare-id").0, "unknown");
    }

    #[test]
    fn build_derives_cost_and_throughput() {
        let rec = CallRecord::build(
            "s1",
            "repl",
            "/proj",
            "anthropic/claude-opus-4-8",
            sample_usage(),
            2000,
            Some(400),
            "end_turn",
            "ok",
        );
        assert_eq!(rec.provider, "anthropic");
        assert!(rec.cost_usd.unwrap() > 0.0);
        // throughput uses the post-first-token window: 500 tok / 1.6s = 312.5 tok/s
        assert!((rec.throughput_tok_s.unwrap() - 312.5).abs() < 0.01, "{:?}", rec.throughput_tok_s);
    }

    #[test]
    fn unpriced_model_has_no_cost() {
        let rec = CallRecord::build(
            "s1", "repl", "/p", "mock/scripted", sample_usage(), 100, None, "end_turn", "ok",
        );
        assert_eq!(rec.cost_usd, None);
    }

    #[test]
    fn append_then_read_roundtrips() {
        let dir = std::env::temp_dir().join(format!("bee-metrics-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("events.jsonl");
        let _ = std::fs::remove_file(&path);
        for i in 0..3 {
            let rec = CallRecord::build(
                "s", "episode", "/p", "anthropic/claude-haiku-4-5", sample_usage(), 100 + i, None,
                "tool_use", "ok",
            );
            append(&path, &rec);
        }
        let back = read_log(&path);
        assert_eq!(back.len(), 3);
        assert_eq!(back[0].model_id, "anthropic/claude-haiku-4-5");
        let _ = std::fs::remove_file(&path);
    }
}
