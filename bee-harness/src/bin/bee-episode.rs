//! `bee-episode` — run one LLM agent episode through a scenario, inside a bee scope, and emit the
//! transcript JSON (T019). Under `--features enforce` the tools execute in a real kernel-enforced
//! scope; otherwise they run as hardened, credential-stripped host processes (offline / live smoke).

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;

use bee_harness::transcript::EpisodeStatus;
use bee_harness::{
    model_from_config, run_batch, run_episode, set_non_dumpable, BatchConfig, ProviderConfig,
    Scenario,
};

#[derive(Parser)]
#[command(
    name = "bee-episode",
    about = "Run an LLM agent episode inside a bee scope"
)]
struct Args {
    /// Scenario TOML (policy + task + limits). Omit and use --task for an ad-hoc run.
    #[arg(long, conflicts_with = "task")]
    scenario: Option<PathBuf>,
    /// Provider TOML (which model / endpoint, or a `mock` script). Required unless --batch.
    #[arg(long)]
    provider: Option<PathBuf>,

    // --- batch mode (US2): cross a set of scenarios with a set of providers ---
    /// Run every (scenario, provider) pair and emit one transcript each.
    #[arg(long)]
    batch: bool,
    /// Batch scenarios: a directory of `*.toml`, or a comma-separated list of paths.
    #[arg(long, requires = "batch")]
    scenarios: Option<String>,
    /// Batch providers: a directory of `*.toml`, or a comma-separated list of paths.
    #[arg(long, requires = "batch")]
    providers: Option<String>,
    /// Run the batch episodes concurrently (US4). Requires building with `--features concurrent`
    /// (async audit demux + shared engine); an error otherwise.
    #[arg(long, requires = "batch")]
    concurrent: bool,

    // --- ad-hoc scenario (alternative to --scenario) ---
    /// The task/prompt to give the agent — builds a scenario on the fly (no TOML needed).
    #[arg(long)]
    task: Option<String>,
    /// Policy for the scope when using --task (default: none / unenforced on the host).
    #[arg(long, requires = "task")]
    policy: Option<PathBuf>,
    /// System prompt for --task runs.
    #[arg(
        long,
        requires = "task",
        default_value = "You are a coding agent operating in a sandbox. Use the available tools to \
                         accomplish the task, then briefly report what you did and stop."
    )]
    system: String,
    /// Comma-separated tools for --task runs.
    #[arg(
        long,
        requires = "task",
        default_value = "bash,read_file,write_file,list_directory"
    )]
    tools: String,
    /// Max tool-call rounds for --task runs.
    #[arg(long, requires = "task", default_value_t = 8)]
    turn_limit: u32,
    /// Wall-clock cap (seconds) for --task runs.
    #[arg(long, requires = "task", default_value_t = 120)]
    timeout_secs: u64,

    /// Write the transcript JSON here instead of stdout.
    #[arg(long)]
    out: Option<PathBuf>,
    /// Suppress the live per-turn progress lines on stderr.
    #[arg(long)]
    quiet: bool,
    /// Color theme for rendered output (built-in name or a custom one from config). Overrides
    /// `BEE_THEME` and the config file (005-themes).
    #[arg(long)]
    theme: Option<String>,
}

/// Expand a `--scenarios` / `--providers` argument into concrete TOML paths. A value containing a
/// comma is a list; otherwise a directory is expanded to its sorted `*.toml` entries and anything
/// else is a single path.
fn expand_paths(arg: &str) -> std::io::Result<Vec<PathBuf>> {
    if arg.contains(',') {
        return Ok(arg
            .split(',')
            .map(|s| PathBuf::from(s.trim()))
            .filter(|p| !p.as_os_str().is_empty())
            .collect());
    }
    let p = Path::new(arg);
    if p.is_dir() {
        let mut out: Vec<PathBuf> = std::fs::read_dir(p)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().map(|x| x == "toml").unwrap_or(false))
            .collect();
        out.sort();
        Ok(out)
    } else {
        Ok(vec![PathBuf::from(arg)])
    }
}

/// Slug a model id for a filename: `anthropic/claude-opus-4-8` → `anthropic_claude-opus-4-8`.
fn slug(model_id: &str) -> String {
    model_id.replace('/', "_")
}

impl Args {
    /// Build the scenario either from a TOML file (`--scenario`) or from the ad-hoc flags (`--task`).
    fn resolve_scenario(&self) -> Result<Scenario, String> {
        if let Some(path) = &self.scenario {
            return Scenario::from_path(path).map_err(|e| e.to_string());
        }
        let task = self
            .task
            .as_ref()
            .ok_or("either --scenario or --task is required")?;
        // An enforced scope must have a real policy; the `/dev/null` default below is only valid on
        // the host (no-enforce) build, where the policy is never compiled. Fail clean rather than
        // let `Policy::from_path("/dev/null")` produce a confusing TOML parse error at run time.
        #[cfg(feature = "enforce")]
        if self.policy.is_none() {
            return Err(
                "under --features enforce, --task requires --policy <bee policy TOML> \
                 (an enforced scope must have a policy)"
                    .to_string(),
            );
        }
        Ok(Scenario {
            id: "adhoc".to_string(),
            policy_path: self
                .policy
                .clone()
                .unwrap_or_else(|| PathBuf::from("/dev/null")),
            system_prompt: self.system.clone(),
            task: task.clone(),
            turn_limit: self.turn_limit.max(1),
            timeout_secs: self.timeout_secs.max(1),
            tools: self
                .tools
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
            mode: Default::default(),
            workdir: Default::default(),
            mcp: Default::default(),
        })
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();

    // FR-018: make the harness non-dumpable so a same-uid tool child can't read the key from
    // /proc/<pid>/environ. Non-fatal on failure — env-stripping remains the primary defense.
    if let Err(e) = set_non_dumpable() {
        eprintln!("bee-episode: warning: could not set non-dumpable: {e}");
    }

    // Resolve + install the color theme (005-themes): --theme > BEE_THEME > config > honeycomb.
    let (theme, theme_warning) = bee_harness::viz::theme::load(args.theme.as_deref());
    if let Some(w) = theme_warning {
        eprintln!("bee-episode: {w}");
    }
    bee_harness::viz::theme::init_theme(theme);

    if args.batch {
        return run_batch_mode(&args).await;
    }

    let scenario = match args.resolve_scenario() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("bee-episode: scenario error: {e}");
            return ExitCode::from(64); // EX_USAGE
        }
    };
    let provider_path = match &args.provider {
        Some(p) => p,
        None => {
            eprintln!("bee-episode: --provider is required (or use --batch)");
            return ExitCode::from(64);
        }
    };
    let provider = match ProviderConfig::from_path(provider_path) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("bee-episode: provider error: {e}");
            return ExitCode::from(64);
        }
    };

    // Resolve the key from its env var (never from the config file); empty is fine for local
    // endpoints and for `mock`.
    let api_key = if provider.api_key_env.is_empty() {
        String::new()
    } else {
        std::env::var(&provider.api_key_env).unwrap_or_default()
    };

    let model = match model_from_config(&provider, &api_key) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("bee-episode: model error: {e}");
            return ExitCode::from(70); // EX_SOFTWARE
        }
    };

    let key_env = (!provider.api_key_env.is_empty()).then_some(provider.api_key_env.as_str());
    // Live progress → stderr (JSON transcript stays on stdout, so piping is unaffected).
    let progress: Option<bee_harness::ProgressSink> = if args.quiet {
        None
    } else {
        Some(Box::new(|line: &str| eprintln!("bee-episode: {line}")))
    };
    let transcript = run_episode(model.as_ref(), &scenario, key_env, progress).await;

    let json = transcript.to_json();
    match &args.out {
        Some(path) => {
            if let Err(e) = std::fs::write(path, &json) {
                eprintln!("bee-episode: cannot write {}: {e}", path.display());
                return ExitCode::from(74); // EX_IOERR
            }
        }
        None => println!("{json}"),
    }

    match transcript.status {
        EpisodeStatus::InfraError { .. } => ExitCode::from(70),
        EpisodeStatus::ApiError { .. } => ExitCode::from(69), // EX_UNAVAILABLE
        _ => ExitCode::SUCCESS,
    }
}

/// Batch mode (US2): cross `--scenarios` with `--providers`, writing one transcript per pair to
/// `--out` (a directory) or a JSON array to stdout. Exits nonzero iff a setup error prevented a
/// pair from running at all (a TOML that could not be loaded).
async fn run_batch_mode(args: &Args) -> ExitCode {
    let (Some(scn_arg), Some(prov_arg)) = (&args.scenarios, &args.providers) else {
        eprintln!("bee-episode: --batch requires --scenarios and --providers");
        return ExitCode::from(64);
    };
    let scenarios = match expand_paths(scn_arg) {
        Ok(v) if !v.is_empty() => v,
        Ok(_) => {
            eprintln!("bee-episode: --scenarios matched no TOML files");
            return ExitCode::from(64);
        }
        Err(e) => {
            eprintln!("bee-episode: --scenarios: {e}");
            return ExitCode::from(64);
        }
    };
    let providers = match expand_paths(prov_arg) {
        Ok(v) if !v.is_empty() => v,
        Ok(_) => {
            eprintln!("bee-episode: --providers matched no TOML files");
            return ExitCode::from(64);
        }
        Err(e) => {
            eprintln!("bee-episode: --providers: {e}");
            return ExitCode::from(64);
        }
    };

    let progress: Option<bee_harness::ProgressSink> = if args.quiet {
        None
    } else {
        Some(Box::new(|line: &str| eprintln!("bee-episode: {line}")))
    };

    if args.concurrent {
        return run_concurrent_mode(scenarios, providers, &args.out, progress).await;
    }

    let config = BatchConfig {
        scenarios,
        providers,
    };
    let result = run_batch(&config, progress).await;

    for e in &result.errors {
        eprintln!(
            "bee-episode: setup error [{} × {}]: {}",
            e.scenario_path.display(),
            e.provider_path.display(),
            e.detail
        );
    }

    // Human-facing status grid to stderr (stdout carries the JSON artifact) — 003-visual-render US7.
    if !result.transcripts.is_empty() {
        eprint!(
            "{}",
            bee_harness::batch::status_grid_summary(&result.transcripts)
        );
    }

    if let Err(code) = emit_transcripts(&result.transcripts, &args.out) {
        return code;
    }

    if result.errors.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(65) // EX_DATAERR — a pair could not be set up
    }
}

/// Write transcripts to `--out` (one JSON file per transcript) or a JSON array to stdout. Returns
/// `Err(code)` on an I/O failure.
fn emit_transcripts(
    transcripts: &[bee_harness::EpisodeTranscript],
    out: &Option<PathBuf>,
) -> Result<(), ExitCode> {
    match out {
        Some(dir) => {
            if let Err(e) = std::fs::create_dir_all(dir) {
                eprintln!("bee-episode: cannot create {}: {e}", dir.display());
                return Err(ExitCode::from(74));
            }
            for t in transcripts {
                let name = format!("{}_{}.json", t.scenario_id, slug(&t.model_id));
                let path = dir.join(&name);
                if let Err(e) = std::fs::write(&path, t.to_json()) {
                    eprintln!("bee-episode: cannot write {}: {e}", path.display());
                    return Err(ExitCode::from(74));
                }
            }
        }
        None => {
            let json = serde_json::to_string_pretty(transcripts)
                .unwrap_or_else(|e| format!("[/* serialize error: {e} */]"));
            println!("{json}");
        }
    }
    Ok(())
}

/// Concurrent batch (US4): cross the scenarios with the providers and run every pair concurrently
/// against one shared engine. Only available when built with `--features concurrent`.
#[cfg(feature = "concurrent")]
async fn run_concurrent_mode(
    scenario_paths: Vec<PathBuf>,
    provider_paths: Vec<PathBuf>,
    out: &Option<PathBuf>,
    progress: Option<bee_harness::ProgressSink>,
) -> ExitCode {
    // Load + validate every scenario/provider; a load failure aborts (unlike sequential batch, the
    // shared-engine setup wants a clean input set).
    let mut scenarios = Vec::new();
    for p in &scenario_paths {
        match Scenario::from_path(p) {
            Ok(s) => scenarios.push(s),
            Err(e) => {
                eprintln!("bee-episode: scenario {}: {e}", p.display());
                return ExitCode::from(64);
            }
        }
    }
    let mut providers = Vec::new();
    for p in &provider_paths {
        match ProviderConfig::from_path(p) {
            Ok(c) => providers.push(c),
            Err(e) => {
                eprintln!("bee-episode: provider {}: {e}", p.display());
                return ExitCode::from(64);
            }
        }
    }

    let episodes: Vec<(Scenario, ProviderConfig)> = scenarios
        .iter()
        .flat_map(|s| providers.iter().map(move |p| (s.clone(), p.clone())))
        .collect();

    let transcripts = bee_harness::run_concurrent(episodes, progress).await;
    match emit_transcripts(&transcripts, out) {
        Ok(()) => ExitCode::SUCCESS,
        Err(code) => code,
    }
}

/// Without the `concurrent` feature, `--concurrent` is unavailable.
#[cfg(not(feature = "concurrent"))]
async fn run_concurrent_mode(
    _scenario_paths: Vec<PathBuf>,
    _provider_paths: Vec<PathBuf>,
    _out: &Option<PathBuf>,
    _progress: Option<bee_harness::ProgressSink>,
) -> ExitCode {
    eprintln!(
        "bee-episode: --concurrent requires building with --features concurrent \
         (async audit demux + shared engine)"
    );
    ExitCode::from(64)
}
