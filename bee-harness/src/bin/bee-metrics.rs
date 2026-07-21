//! `bee-metrics` — report on the LLM usage/cost/performance log the harness records.
//!
//! Reads the append-only JSONL log (default `$XDG_STATE_HOME/bee/metrics/events.jsonl`), optionally
//! filters it, and prints a summary: total spend, tokens, cost/tokens by model / project / day, and
//! latency + time-to-first-token percentiles. Records carry no prompt content — only counts, model,
//! and timing.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;

use bee_harness::metrics::{self, report};

#[derive(Parser)]
#[command(name = "bee-metrics", about = "Report on recorded LLM usage, cost, and latency")]
struct Args {
    /// Override the metrics log path (default: $XDG_STATE_HOME/bee/metrics/events.jsonl).
    #[arg(long)]
    path: Option<PathBuf>,
    /// Only include calls whose project path contains this substring.
    #[arg(long)]
    project: Option<String>,
    /// Only include calls whose model id contains this substring.
    #[arg(long)]
    model: Option<String>,
    /// Only include calls on or after this date (YYYY-MM-DD).
    #[arg(long)]
    since: Option<String>,
    /// Print the (filtered) raw records as JSON instead of the summary.
    #[arg(long)]
    json: bool,
}

fn main() -> ExitCode {
    let args = Args::parse();

    let path = match args.path.or_else(metrics::metrics_path) {
        Some(p) => p,
        None => {
            eprintln!("bee-metrics: no metrics path (set XDG_STATE_HOME or HOME, or pass --path)");
            return ExitCode::from(74); // EX_IOERR
        }
    };

    let mut records = metrics::read_log(&path);
    records.retain(|r| {
        args.project.as_ref().is_none_or(|p| r.project.contains(p))
            && args.model.as_ref().is_none_or(|m| r.model_id.contains(m))
            && args.since.as_ref().is_none_or(|d| r.ts.as_str() >= d.as_str())
    });

    if args.json {
        match serde_json::to_string_pretty(&records) {
            Ok(s) => println!("{s}"),
            Err(e) => {
                eprintln!("bee-metrics: serialize error: {e}");
                return ExitCode::from(70);
            }
        }
        return ExitCode::SUCCESS;
    }

    println!("metrics: {} ({} records)", path.display(), records.len());
    print!("{}", report::render(&report::aggregate(&records)));
    ExitCode::SUCCESS
}
