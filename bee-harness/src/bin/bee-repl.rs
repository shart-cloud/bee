//! `bee-repl` — an interactive chat session with a sandboxed coding agent (US5). The user types
//! messages; the agent responds, executing its tool calls inside a bee scope; audit denials surface
//! live. Under `--features enforce` with `--policy`, tools run in a real kernel-enforced scope;
//! otherwise they run as hardened, credential-stripped host processes.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;

use bee_harness::repl::{run_repl, ReplConfig};
use bee_harness::sandbox::{self, Sandbox};
use bee_harness::tools::registry_for;
use bee_harness::{model_from_config, set_non_dumpable, ProviderConfig};

#[derive(Parser)]
#[command(
    name = "bee-repl",
    about = "Interactive chat with a sandboxed coding agent"
)]
struct Args {
    /// Provider TOML (which model / endpoint, or a `mock` script). Same format as bee-episode.
    #[arg(long)]
    provider: PathBuf,
    /// Bee policy for the scope. Omit to run tools as hardened host processes with no kernel scope.
    #[arg(long)]
    policy: Option<PathBuf>,
    /// Override the session system prompt.
    #[arg(long)]
    system: Option<String>,
    /// Comma-separated tools the agent gets.
    #[arg(long, default_value = "bash,read_file,write_file,list_directory")]
    tools: String,
    /// Max model calls per user message (safety cap against a runaway agent).
    #[arg(long, default_value_t = 25)]
    budget: u32,
    /// Write the session transcript to this path on exit.
    #[arg(long)]
    save: Option<PathBuf>,
    /// Play the bee mascot animation at startup (also enabled by `BEE_MASCOT=1`).
    #[arg(long)]
    bee: bool,
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();

    // FR-018: make the harness non-dumpable so a same-uid tool child can't read the key from
    // /proc/<pid>/environ. Non-fatal on failure — env-stripping remains the primary defense.
    if let Err(e) = set_non_dumpable() {
        eprintln!("bee-repl: warning: could not set non-dumpable: {e}");
    }

    let provider = match ProviderConfig::from_path(&args.provider) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("bee-repl: provider error: {e}");
            return ExitCode::from(64); // EX_USAGE
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
            eprintln!("bee-repl: model error: {e}");
            return ExitCode::from(70); // EX_SOFTWARE
        }
    };

    let tools: Vec<String> = args
        .tools
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let registry = registry_for(&tools, None);

    let key_env = (!provider.api_key_env.is_empty()).then_some(provider.api_key_env.as_str());
    let strip_env = sandbox::key_vars(key_env);

    let (mut sbox, policy_label) = match build_sandbox(args.policy.as_ref(), strip_env) {
        Ok(pair) => pair,
        Err(detail) => {
            eprintln!("bee-repl: {detail}");
            return ExitCode::from(70);
        }
    };

    let config = ReplConfig {
        system_prompt: args
            .system
            .clone()
            .unwrap_or_else(|| ReplConfig::default().system_prompt),
        agent_turn_budget: args.budget.max(1),
        policy_label: policy_label.clone(),
        mascot: args.bee || std::env::var_os("BEE_MASCOT").is_some_and(|v| v == "1"),
        ..ReplConfig::default()
    };

    // Startup banner.
    println!("bee-repl");
    println!("  model:  {}", model.id());
    println!("  policy: {policy_label}");
    println!("  tools:  {}", tools.join(", "));
    println!();

    let session = run_repl(model.as_ref(), &registry, &mut sbox, &config).await;
    sbox.teardown();

    // Persist the transcript to --save on exit, if requested.
    if let Some(path) = &args.save {
        if let Some(transcript) = &session.transcript {
            match std::fs::write(path, transcript.to_json()) {
                Ok(()) => eprintln!("bee-repl: saved transcript to {}", path.display()),
                Err(e) => {
                    eprintln!("bee-repl: cannot write {}: {e}", path.display());
                    return ExitCode::from(74); // EX_IOERR
                }
            }
        }
    }

    ExitCode::SUCCESS
}

/// Build the sandbox for the session. With `--features enforce` and a policy, this is a real
/// kernel-enforced bee scope; otherwise (no policy, or the host build) it is a hardened host
/// sandbox with no scope. Returns the sandbox plus a human-readable label for the banner.
#[cfg(feature = "enforce")]
fn build_sandbox(
    policy: Option<&PathBuf>,
    strip_env: Vec<String>,
) -> Result<(Sandbox, String), String> {
    use bee_core::Policy;
    use bee_userspace::{EnforcementPlan, Engine, ScopeMode, SystemResolver};

    let Some(policy_path) = policy else {
        return Ok((
            Sandbox::host(strip_env),
            "none — host mode (no kernel scope)".to_string(),
        ));
    };

    let compiled = Policy::from_path(policy_path)
        .map_err(|e| format!("policy {}: {e}", policy_path.display()))?;
    let resolver = SystemResolver::current();
    let compiled = compiled
        .compile(&resolver)
        .map_err(|e| format!("policy compile: {e}"))?;
    let plan = EnforcementPlan::prepare(&compiled, ScopeMode::Enforce)
        .map_err(|e| format!("enforcement plan: {e}"))?;
    let mut engine = Engine::init().map_err(|e| format!("engine init: {e}"))?;

    let scope_id = format!("bee-repl-{}", std::process::id());
    let scope = engine
        .create_scope(&scope_id, bee_userspace::cgroup::DEFAULT_PARENT, &plan)
        .map_err(|e| format!("create scope: {e}"))?;
    let reader = engine
        .take_audit_reader(&scope_id)
        .map_err(|e| format!("audit reader: {e}"))?;
    let label = format!("{} (enforced)", policy_path.display());
    Ok((Sandbox::enforced(engine, scope, reader, strip_env), label))
}

/// Host build: there is no kernel enforcement available, so `--policy` (if given) is noted but not
/// applied. The tools still run hardened + credential-stripped.
#[cfg(not(feature = "enforce"))]
fn build_sandbox(
    policy: Option<&PathBuf>,
    strip_env: Vec<String>,
) -> Result<(Sandbox, String), String> {
    let label = match policy {
        Some(p) => format!(
            "{} — IGNORED (host build; rebuild with --features enforce to enforce it)",
            p.display()
        ),
        None => "none — host mode (no kernel scope)".to_string(),
    };
    Ok((Sandbox::host(strip_env), label))
}
