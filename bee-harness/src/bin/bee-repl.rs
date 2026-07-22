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
    /// Capability ceiling for skill grants (006-skills). A skill's `requires` block may widen
    /// `--policy` only up to this ceiling (attenuation-bounded); you approve each within-ceiling
    /// request at startup. Omit ⇒ `--policy` is the ceiling ⇒ skills grant nothing.
    #[arg(long)]
    ceiling_policy: Option<PathBuf>,
    /// Override the session system prompt.
    #[arg(long)]
    system: Option<String>,
    /// Comma-separated tools the agent gets. `render` is included by default so the agent can draw
    /// charts/tables/sprites in the terminal (drop it from the list to disable visuals).
    #[arg(
        long,
        default_value = "bash,read_file,write_file,list_directory,render"
    )]
    tools: String,
    /// Max model calls per user message (safety cap against a runaway agent).
    #[arg(long, default_value_t = 25)]
    budget: u32,
    /// Write the session transcript to this path on exit.
    #[arg(long)]
    save: Option<PathBuf>,
    /// Suppress the bee mascot animation at startup (on by default; also suppressible with
    /// `BEE_MASCOT=0`).
    #[arg(long)]
    no_bee: bool,
    /// Color theme (built-in name or a custom one from config). Overrides `BEE_THEME` and the config
    /// file. Built-ins: honeycomb (default), catppuccin-{mocha,latte,frappe,macchiato}, dracula, nord.
    #[arg(long)]
    theme: Option<String>,
    /// Standalone MCP config TOML (top-level `[mcp]` + `[[mcp.servers]]`). Requires building with
    /// `--features mcp` (004-mcp-client).
    #[arg(long)]
    mcp_config: Option<PathBuf>,
    /// Launch the full-screen TUI front-end (008-grid-tui). Requires building with `--features tui`;
    /// opt-in for now — capability-based auto-selection and fallback land in US3.
    #[arg(long, conflicts_with = "no_tui")]
    tui: bool,
    /// Force the inline REPL, overriding `--tui`.
    #[arg(long)]
    no_tui: bool,
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();

    // FR-018: make the harness non-dumpable so a same-uid tool child can't read the key from
    // /proc/<pid>/environ. Non-fatal on failure — env-stripping remains the primary defense.
    if let Err(e) = set_non_dumpable() {
        eprintln!("bee-repl: warning: could not set non-dumpable: {e}");
    }

    // Resolve + install the color theme (005-themes) before anything renders: --theme > BEE_THEME >
    // config file > honeycomb. An unknown name warns and falls back rather than failing (FR-052).
    let (theme, theme_warning) = bee_harness::viz::theme::load(args.theme.as_deref());
    if let Some(w) = theme_warning {
        eprintln!("bee-repl: {w}");
    }
    bee_harness::viz::theme::init_theme(theme);

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
    let mut registry = registry_for(&tools, None);

    let key_env = (!provider.api_key_env.is_empty()).then_some(provider.api_key_env.as_str());
    // Strip provider key vars plus any MCP token_env names from tool children (FR-018/FR-041).
    #[cfg(feature = "mcp")]
    let mcp_policy = match &args.mcp_config {
        Some(path) => match bee_harness::McpPolicy::from_path(path) {
            Ok(p) => Some(p),
            Err(e) => {
                eprintln!("bee-repl: mcp config error: {e}");
                return ExitCode::from(64);
            }
        },
        None => None,
    };
    #[cfg(not(feature = "mcp"))]
    if args.mcp_config.is_some() {
        eprintln!("bee-repl: --mcp-config requires building with --features mcp");
        return ExitCode::from(64);
    }

    #[cfg_attr(not(feature = "mcp"), allow(unused_mut))]
    let mut strip_env = sandbox::key_vars(key_env);
    #[cfg(feature = "mcp")]
    if let Some(p) = &mcp_policy {
        strip_env.extend(p.token_env_names());
    }

    // Discover skills (006-skills): project `.claude/skills` then user `~/.claude/skills`. Malformed
    // skills warn but never abort.
    let skills = {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let roots = bee_harness::SkillRegistry::default_roots(&cwd);
        std::sync::Arc::new(bee_harness::SkillRegistry::discover(&roots))
    };
    for w in skills.warnings() {
        eprintln!(
            "bee-repl: skill warning: {}: {}",
            w.path.display(),
            w.reason
        );
    }

    // Resolve skill capability grants BEFORE the scope compiles (006-skills, step 3). The base
    // policy is `--policy`, the ceiling `--ceiling-policy` (or base). Each within-ceiling request is
    // approved interactively; beyond-ceiling requests are refused. Granted tools are registered and
    // the widened policy flows into the scope. Instructions-only skills never prompt.
    let resolved_policy = resolve_repl_grants(&args, &skills, &mut registry).await;

    // Register the model-facing `skill` tool; `/skill` reads the same registry regardless.
    if skills.model_facing().next().is_some() {
        registry.insert(Box::new(bee_harness::tools::skill::SkillTool::new(
            skills.clone(),
        )));
    }

    let (mut sbox, policy_label) =
        match build_sandbox(resolved_policy, args.policy.as_deref(), strip_env) {
            Ok(pair) => pair,
            Err(detail) => {
                eprintln!("bee-repl: {detail}");
                return ExitCode::from(70);
            }
        };

    // Connect MCP servers (if configured), register their tools alongside the built-ins, and build
    // the per-turn refresh hook (FR-043). The bridge (Arc so the hook can hold a clone) owns the
    // stdio children for the session; it is dropped before the scope (US10).
    #[cfg(feature = "mcp")]
    let (mcp_bridge, mcp_summary, mcp_refresh) = match mcp_policy {
        Some(policy) => {
            let bridge =
                std::sync::Arc::new(bee_harness::mcp::McpBridge::connect(policy, &sbox).await);
            bridge.register_into(&mut registry);
            let summary = bridge.summary();
            let hook = bridge.clone();
            let refresh: Box<dyn Fn(&mut bee_harness::ToolRegistry) + Send + Sync> =
                Box::new(move |reg| {
                    if hook.take_dirty() {
                        reg.remove_mcp_tools();
                        hook.register_into(reg);
                    }
                });
            (Some(bridge), Some(summary), Some(refresh))
        }
        None => (None, None, None),
    };

    let config = ReplConfig {
        system_prompt: args
            .system
            .clone()
            .unwrap_or_else(|| ReplConfig::default().system_prompt),
        agent_turn_budget: args.budget.max(1),
        policy_label: policy_label.clone(),
        mascot: !args.no_bee && std::env::var_os("BEE_MASCOT").is_none_or(|v| v != "0"),
        #[cfg(feature = "mcp")]
        mcp_summary,
        #[cfg(feature = "mcp")]
        refresh_tools: mcp_refresh,
        skills: skills.clone(),
        ..ReplConfig::default()
    };

    // Startup banner.
    println!("bee-repl");
    println!("  model:  {}", model.id());
    println!("  policy: {policy_label}");
    println!("  tools:  {}", tools.join(", "));
    println!("  theme:  {}", bee_harness::viz::theme::active_theme().name);
    if !skills.is_empty() {
        println!(
            "  skills: {} ({} agent-loadable)",
            skills.len(),
            skills.model_facing().count()
        );
    }
    #[cfg(feature = "mcp")]
    if let Some(s) = &config.mcp_summary {
        println!("  {}", s.lines().next().unwrap_or("mcp: configured"));
    }
    println!();

    // --tui opts into the full-screen front-end; --no-tui forces the inline REPL (they conflict, so
    // at most one is set). The default is the inline REPL — auto-selecting the TUI on a capable
    // terminal is US3 (T033). The TUI front-end produces no transcript yet, so `session` is `None`
    // in that path and `--save` is a no-op there.
    let use_tui = args.tui && !args.no_tui;
    let session: Option<bee_harness::repl::ReplSession> = {
        #[cfg(feature = "tui")]
        {
            if use_tui {
                if let Err(e) =
                    bee_harness::tui::run(model.as_ref(), &mut registry, &mut sbox, &config).await
                {
                    eprintln!("bee-repl: tui error: {e}");
                }
                None
            } else {
                Some(run_repl(model.as_ref(), &mut registry, &mut sbox, &config).await)
            }
        }
        #[cfg(not(feature = "tui"))]
        {
            if use_tui {
                eprintln!("bee-repl: --tui requires building with --features tui; running inline");
            }
            Some(run_repl(model.as_ref(), &mut registry, &mut sbox, &config).await)
        }
    };

    // Drop the refresh hook (its bridge clone lives in `config`) then the bridge, killing the MCP
    // children (they live in the scope cgroup) before the scope itself is torn down.
    #[cfg(feature = "mcp")]
    {
        drop(config);
        drop(mcp_bridge);
    }
    sbox.teardown();

    // Persist the transcript to --save on exit, if requested. (The TUI path yields no transcript
    // yet, so `session` is `None` there and `--save` writes nothing.)
    if let Some(path) = &args.save {
        if let Some(transcript) = session.as_ref().and_then(|s| s.transcript.as_ref()) {
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
    policy: Option<bee_core::Policy>,
    label_hint: Option<&std::path::Path>,
    strip_env: Vec<String>,
) -> Result<(Sandbox, String), String> {
    use bee_userspace::{EnforcementPlan, Engine, ScopeMode, SystemResolver};

    let Some(policy) = policy else {
        return Ok((
            Sandbox::host(strip_env),
            "none — host mode (no kernel scope)".to_string(),
        ));
    };

    let resolver = SystemResolver::current();
    let compiled = policy
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
    let label = match label_hint {
        Some(p) => format!("{} (enforced)", p.display()),
        None => "policy (enforced)".to_string(),
    };
    Ok((Sandbox::enforced(engine, scope, reader, strip_env), label))
}

/// Host build: there is no kernel enforcement available, so the resolved policy (if any) is noted but
/// not applied. The tools still run hardened + credential-stripped.
#[cfg(not(feature = "enforce"))]
fn build_sandbox(
    _policy: Option<bee_core::Policy>,
    label_hint: Option<&std::path::Path>,
    strip_env: Vec<String>,
) -> Result<(Sandbox, String), String> {
    let label = match label_hint {
        Some(p) => format!(
            "{} — IGNORED (host build; rebuild with --features enforce to enforce it)",
            p.display()
        ),
        None => "none — host mode (no kernel scope)".to_string(),
    };
    Ok((Sandbox::host(strip_env), label))
}

/// Resolve skill capability grants for the REPL (006-skills, step 3), prompting interactively. The
/// base policy is `--policy`; the ceiling is `--ceiling-policy` (or base). Registers granted tools
/// into `registry` and returns the widened policy (base ∪ approved deltas ∪ readable skill dirs) to
/// compile, or `None` when there is no parseable base policy (host mode without `--policy`).
async fn resolve_repl_grants(
    args: &Args,
    skills: &bee_harness::SkillRegistry,
    registry: &mut bee_harness::ToolRegistry,
) -> Option<bee_core::Policy> {
    let base = match args.policy.as_ref() {
        Some(p) => match bee_core::Policy::from_path(p) {
            Ok(pol) => pol,
            Err(e) => {
                eprintln!("bee-repl: policy parse warning ({e}); skill grants disabled");
                return None;
            }
        },
        None => return None, // no scope policy ⇒ nothing to widen (host mode)
    };
    let ceiling = match args.ceiling_policy.as_ref() {
        Some(p) => bee_core::Policy::from_path(p).unwrap_or_else(|e| {
            eprintln!("bee-repl: ceiling parse warning ({e}); using base as ceiling");
            base.clone()
        }),
        None => base.clone(),
    };

    let skill_refs: Vec<&bee_harness::skills::Skill> = skills.iter().collect();
    let mut outcome =
        bee_harness::skills::resolve_grants(&skill_refs, &base, &ceiling, &PromptConsent).await;
    for name in &outcome.granted {
        println!("bee-repl: skill '{name}' capabilities granted");
    }
    for (name, reason) in &outcome.refused {
        eprintln!("bee-repl: skill '{name}' refused — {reason}");
    }
    for tool in &outcome.tools {
        bee_harness::tools::register_named(registry, tool, None);
    }
    // Readable-scope: each discovered skill dir becomes readable so bundled resources resolve.
    for s in &skill_refs {
        if let Some(dir) = s.dir.to_str() {
            outcome
                .policy
                .filesystem
                .entry(dir.to_string())
                .or_insert(bee_core::Access::Read);
        }
    }
    Some(outcome.policy)
}

/// Interactive skill-grant consent (006-skills): prompt on stderr, read a y/N line from stdin. Any
/// non-`y` answer (including EOF) refuses — deny-by-default. Runs once per capability-requesting
/// skill at startup, before the readline loop begins.
struct PromptConsent;

#[async_trait::async_trait]
impl bee_harness::skills::ConsentSink for PromptConsent {
    async fn confirm(
        &self,
        request: &bee_harness::skills::GrantRequest<'_>,
    ) -> bee_harness::skills::Decision {
        use bee_harness::skills::Decision;
        use std::io::Write;
        eprintln!("\nskill '{}' requests extra capabilities:", request.skill);
        if !request.tools.is_empty() {
            eprintln!("  tools: {}", request.tools.join(", "));
        }
        for (path, access) in request.filesystem {
            eprintln!("  filesystem: {path} = {access}");
        }
        eprint!("grant these (within the ceiling)? [y/N] ");
        let _ = std::io::stderr().flush();
        let mut line = String::new();
        let granted = std::io::stdin().read_line(&mut line).is_ok()
            && matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes");
        if granted {
            Decision::Granted
        } else {
            Decision::Denied
        }
    }
}
