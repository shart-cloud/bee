//! `bee metrics` — report on the LLM usage, cost, and latency log the harness records
//! (consolidation issue 06).
//!
//! Reads the append-only JSONL log, optionally filters it, and prints a summary: total spend,
//! tokens, cost and tokens by model / project / day, and latency and time-to-first-token
//! percentiles. Records carry no prompt content — only counts, model, and timing.
//!
//! The default path stays `$XDG_STATE_HOME/bee/metrics/events.jsonl`. That is *state*, not
//! configuration, and issue 02 deliberately left it where it is.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Args;

use bee_harness::metrics::{self, report};

use crate::session::{EX_IOERR, EX_SOFTWARE};

const CMD: &str = "bee metrics";

#[derive(Args, Debug)]
pub struct MetricsArgs {
    /// Override the metrics log path (default: $XDG_STATE_HOME/bee/metrics/events.jsonl).
    #[arg(long)]
    pub path: Option<PathBuf>,
    /// Only include calls whose project path contains this substring.
    #[arg(long)]
    pub project: Option<String>,
    /// Only include calls whose model id contains this substring.
    #[arg(long)]
    pub model: Option<String>,
    /// Only include calls on or after this date (YYYY-MM-DD).
    #[arg(long)]
    pub since: Option<String>,
    /// Print the (filtered) raw records as JSON instead of the summary.
    #[arg(long)]
    pub json: bool,
}

pub fn main(args: MetricsArgs) -> ExitCode {
    let path = match args.path.or_else(metrics::metrics_path) {
        Some(p) => p,
        None => {
            eprintln!("{CMD}: no metrics path (set XDG_STATE_HOME or HOME, or pass --path)");
            return ExitCode::from(EX_IOERR);
        }
    };

    let mut records = metrics::read_log(&path);
    records.retain(|r| {
        args.project.as_ref().is_none_or(|p| r.project.contains(p))
            && args.model.as_ref().is_none_or(|m| r.model_id.contains(m))
            && args
                .since
                .as_ref()
                .is_none_or(|d| r.ts.as_str() >= d.as_str())
    });

    if args.json {
        match serde_json::to_string_pretty(&records) {
            Ok(s) => println!("{s}"),
            Err(e) => {
                eprintln!("{CMD}: serialize error: {e}");
                return ExitCode::from(EX_SOFTWARE);
            }
        }
        return ExitCode::SUCCESS;
    }

    println!("metrics: {} ({} records)", path.display(), records.len());
    print!("{}", report::render(&report::aggregate(&records)));
    ExitCode::SUCCESS
}
