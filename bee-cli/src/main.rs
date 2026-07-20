//! `bee` CLI — a thin wrapper over `bee-core` + `bee-userspace` (constitution Principle V; the CLI
//! adds no enforcement logic of its own). See `contracts/cli.md` for the command surface + exit codes.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use bee_core::Policy;
#[cfg(feature = "enforce")]
use bee_userspace::spawn;
use bee_userspace::{EnforcementPlan, Engine, EngineError, SystemResolver};
use clap::{Parser, Subcommand};

// Exit codes (contracts/cli.md).
const EX_POLICY: u8 = 64;
const EX_UNSUPPORTED: u8 = 65;
#[cfg_attr(not(feature = "enforce"), allow(dead_code))]
const EX_PRIVILEGED: u8 = 66;
const EX_INTERNAL: u8 = 70;

#[derive(Parser)]
#[command(
    name = "bee",
    version,
    about = "eBPF-enforced sandbox harness for coding agents"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Print kernel support diagnostics and exit (never attaches).
    Check,
    /// Compile-check a policy, or check attenuation of a child against a parent.
    Validate {
        #[arg(long)]
        policy: PathBuf,
        /// If given, verify `--policy` is a subset of this parent policy.
        #[arg(long)]
        parent: Option<PathBuf>,
    },
    /// Run a command inside a sandbox scope.
    Run {
        #[arg(long)]
        policy: PathBuf,
        /// Parent policy to attenuate against (subagent mode): the effective policy is
        /// `parent.derive(policy)`. Refuses to run if `policy` exceeds `parent` (FR-005).
        #[arg(long)]
        parent: Option<PathBuf>,
        /// Override the policy's `mode` (enforce|observe). Defaults to the policy file's mode.
        #[arg(long)]
        mode: Option<String>,
        #[arg(long, default_value = bee_userspace::cgroup::DEFAULT_PARENT)]
        parent_cgroup: String,
        /// Command and args after `--`.
        #[arg(last = true, required = true)]
        command: Vec<String>,
    },
}

fn main() -> ExitCode {
    match Cli::parse().cmd {
        Cmd::Check => cmd_check(),
        Cmd::Validate { policy, parent } => cmd_validate(&policy, parent.as_deref()),
        Cmd::Run {
            policy,
            parent,
            mode,
            parent_cgroup,
            command,
        } => cmd_run(
            &policy,
            parent.as_deref(),
            mode.as_deref(),
            &parent_cgroup,
            &command,
        ),
    }
}

fn cmd_check() -> ExitCode {
    let support = Engine::supported();
    println!("{support}");
    if support.is_supported() {
        println!("=> enforcement available");
        ExitCode::SUCCESS
    } else {
        ExitCode::from(EX_UNSUPPORTED)
    }
}

fn cmd_validate(policy: &Path, parent: Option<&Path>) -> ExitCode {
    let child = match Policy::from_path(policy) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("policy error: {e}");
            return ExitCode::from(EX_POLICY);
        }
    };
    let (effective, attenuation) = match parent {
        None => (child, false),
        Some(parent_path) => match Policy::from_path(parent_path) {
            Ok(parent) => match parent.derive(child) {
                Ok(derived) => (derived, true),
                Err(e) => {
                    eprintln!("attenuation violation: {e}");
                    return ExitCode::from(EX_POLICY);
                }
            },
            Err(e) => {
                eprintln!("parent policy error: {e}");
                return ExitCode::from(EX_POLICY);
            }
        },
    };

    let compiled = match effective.compile(&SystemResolver::current()) {
        Ok(compiled) => compiled,
        Err(e) => {
            eprintln!("compile error: {e}");
            return ExitCode::from(EX_POLICY);
        }
    };
    if let Err(e) = EnforcementPlan::prepare(&compiled, effective.mode.into()) {
        eprintln!("enforcement plan error: {e}");
        return ExitCode::from(EX_POLICY);
    }

    if attenuation {
        println!("attenuation OK: child is a runnable subset of parent");
    } else {
        println!("policy '{}' is valid and runnable", effective.name);
    }
    ExitCode::SUCCESS
}

fn cmd_run(
    policy: &Path,
    parent: Option<&Path>,
    mode: Option<&str>,
    parent_cgroup: &str,
    command: &[String],
) -> ExitCode {
    if let Some(m) = mode {
        if m != "enforce" && m != "observe" {
            eprintln!("invalid --mode '{m}' (expected enforce|observe)");
            return ExitCode::from(EX_POLICY);
        }
    }
    let child = match Policy::from_path(policy) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("policy error: {e}");
            return ExitCode::from(EX_POLICY);
        }
    };
    // Subagent mode: attenuate the child against its parent, refusing any over-grant (FR-005).
    let policy = match parent {
        None => child,
        Some(parent_path) => {
            let parent = match Policy::from_path(parent_path) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("parent policy error: {e}");
                    return ExitCode::from(EX_POLICY);
                }
            };
            match parent.derive(child) {
                Ok(derived) => derived,
                Err(e) => {
                    eprintln!("refusing to run: attenuation violation: {e}");
                    return ExitCode::from(EX_POLICY);
                }
            }
        }
    };
    // The policy's `mode` field is the source of truth; `--mode` overrides it if given.
    let mode: &str = mode.unwrap_or(match policy.mode {
        bee_core::Mode::Enforce => "enforce",
        bee_core::Mode::Observe => "observe",
    });
    // Compile and plan before touching the kernel so every unsupported capability fails early.
    let compiled = match policy.compile(&SystemResolver::current()) {
        Ok(compiled) => compiled,
        Err(e) => {
            eprintln!("compile error: {e}");
            return ExitCode::from(EX_POLICY);
        }
    };
    let scope_mode = if mode == "observe" {
        bee_userspace::ScopeMode::Observe
    } else {
        bee_userspace::ScopeMode::Enforce
    };
    let plan = match EnforcementPlan::prepare(&compiled, scope_mode) {
        Ok(plan) => plan,
        Err(e) => {
            eprintln!("enforcement plan error: {e}");
            return ExitCode::from(EX_POLICY);
        }
    };
    #[cfg(not(feature = "enforce"))]
    let _ = &plan;

    // Fail-closed: refuse to run if the kernel cannot enforce (SC-007).
    match Engine::init() {
        Ok(engine) => {
            #[cfg(feature = "enforce")]
            {
                enforce_run(engine, &policy.name, mode, parent_cgroup, command, &plan)
            }
            #[cfg(not(feature = "enforce"))]
            {
                let _ = (engine, parent_cgroup, command, mode);
                unreachable!("init() cannot return Ok without the enforce feature")
            }
        }
        Err(EngineError::Unsupported(diag)) => {
            eprintln!("refusing to run: kernel cannot enforce (fail-closed)\n{diag}");
            ExitCode::from(EX_UNSUPPORTED)
        }
        Err(EngineError::EnforceFeatureDisabled) => {
            eprintln!(
                "kernel supports BPF-LSM but this `bee` was built without enforcement.\n\
                 Rebuild with `--features enforce` on the nightly bpf toolchain to attach policies."
            );
            ExitCode::from(EX_INTERNAL)
        }
        Err(EngineError::Load(d)) => {
            eprintln!("failed to load/attach enforcement: {d}");
            ExitCode::from(EX_INTERNAL)
        }
    }
}

/// Full enforce path: create a scope cgroup, register it for enforcement, spawn the hardened command
/// into it, and stream audit events until it exits.
#[cfg(feature = "enforce")]
fn enforce_run(
    mut engine: Engine,
    policy_name: &str,
    mode: &str,
    parent_cgroup: &str,
    command: &[String],
    plan: &EnforcementPlan,
) -> ExitCode {
    let scope_id = format!(
        "{}-{}",
        policy_name
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect::<String>(),
        std::process::id()
    );

    let scope = match engine.create_scope(&scope_id, parent_cgroup, plan) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("create scope failed: {e}");
            return ExitCode::from(EX_INTERNAL);
        }
    };
    let mut audit = match engine.take_audit_reader(&scope_id) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("audit reader failed: {e}");
            let _ = scope.teardown();
            return ExitCode::from(EX_INTERNAL);
        }
    };
    let join = match scope.join_closure() {
        Ok(j) => j,
        Err(e) => {
            eprintln!("cgroup join setup failed: {e}");
            let _ = scope.teardown();
            return ExitCode::from(EX_INTERNAL);
        }
    };

    let (program, args) = command.split_first().expect("clap requires >=1 arg");
    let mut cmd = match spawn::hardened_command(program, args, Some(join)) {
        Ok(c) => c,
        Err(spawn::SpawnError::PrivilegedTarget(t)) => {
            eprintln!("refusing to sandbox privileged target: {t}");
            let _ = scope.teardown();
            return ExitCode::from(EX_PRIVILEGED);
        }
        Err(e) => {
            eprintln!("{e}");
            let _ = scope.teardown();
            return ExitCode::from(EX_INTERNAL);
        }
    };

    eprintln!(
        "bee: scope '{scope_id}' (cgroup id {}) — {mode} mode",
        scope.cgroup_id
    );
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("spawn failed: {e}");
            let _ = scope.teardown();
            return ExitCode::from(EX_INTERNAL);
        }
    };

    let mut print = |ev: bee_core::AuditEvent| eprintln!("{}", ev.to_json());
    let code = loop {
        let _ = audit.poll(200);
        audit.drain(&mut print);
        match child.try_wait() {
            Ok(Some(status)) => {
                audit.drain(&mut print); // final flush
                break status.code().unwrap_or(1) as u8;
            }
            Ok(None) => continue,
            Err(e) => {
                eprintln!("wait failed: {e}");
                break 1;
            }
        }
    };
    let _ = scope.teardown();
    ExitCode::from(code)
}
