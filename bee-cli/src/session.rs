//! Session construction, shared by `bee run` and `bee repl` (consolidation issue 03).
//!
//! `bee-episode` and `bee-repl` each built a session from scratch: set the process non-dumpable,
//! resolve and install the theme, load the provider, read the key from its named variable, build
//! the model. Two copies, in two orders, with two sets of error strings — and when they drifted,
//! the enforcement behaviour of a headless run and an interactive run drifted with them.
//!
//! This is that sequence, once. It takes an [`EffectiveConfig`] and returns the pieces a session
//! runs on. What it deliberately does *not* do is decide anything: the presentation choices, the
//! consent prompting, and the sandbox lifetime belong to the command, because those are the parts
//! that genuinely differ between headless and interactive.

use std::process::ExitCode;

use bee_harness::{model_from_config, set_non_dumpable, Model};

use crate::config::EffectiveConfig;

/// Exit codes, from `sysexits.h` — the ones the harness binaries have always used. A CI harness
/// somewhere is reading these, so they are an interface rather than an implementation detail.
pub const EX_USAGE: u8 = 64;
pub const EX_DATAERR: u8 = 65;
pub const EX_UNAVAILABLE: u8 = 69;
pub const EX_SOFTWARE: u8 = 70;
pub const EX_IOERR: u8 = 74;

/// The constructed pieces of a session: a model to talk to and the name of the variable holding its
/// key, which tool children must not inherit.
pub struct Session {
    pub model: Box<dyn Model>,
    /// The provider's `api_key_env`, or `None` for a keyless provider (`mock`, a local endpoint).
    /// Carried separately because stripping it from tool children is a security property, not a
    /// detail of the model.
    pub key_env: Option<String>,
}

/// Why a session could not be built. Each is a refusal before the provider is contacted.
pub struct SessionError {
    pub detail: String,
    pub code: u8,
}

impl SessionError {
    fn new(detail: impl Into<String>, code: u8) -> Self {
        SessionError {
            detail: detail.into(),
            code,
        }
    }
}

/// Harden the process, install the presentation, and build the model.
///
/// The ordering matters and is the reason this is one function rather than a handful of helpers a
/// command could call in any sequence:
///
/// 1. Non-dumpable first, before anything can read the key out of `/proc/<pid>/environ` (FR-018).
/// 2. The visual gate before anything can render, because the render tool reads the level
///    process-globally and the gate must be in place before the first script evaluates.
/// 3. The theme before anything draws.
/// 4. The model last — it is the first thing that could touch the network.
pub fn build(cfg: &EffectiveConfig, command: &str) -> Result<Session, SessionError> {
    // Non-fatal: env-stripping remains the primary defence, and refusing to start because a
    // hardening nicety failed would trade a real session for a marginal gain.
    if let Err(e) = set_non_dumpable() {
        eprintln!("{command}: warning: could not set non-dumpable: {e}");
    }

    bee_harness::visual_gate::set(cfg.visual);

    if let Some(w) = &cfg.theme_warning {
        eprintln!("{command}: {w}");
    }
    bee_harness::viz::theme::init_theme(cfg.theme.clone());

    // The key comes from its named variable, never from a config file (FR-018). Empty is fine for
    // local endpoints and for `mock`.
    let key_env = (!cfg.provider.api_key_env.is_empty()).then(|| cfg.provider.api_key_env.clone());
    let api_key = match &key_env {
        Some(name) => std::env::var(name).unwrap_or_default(),
        None => String::new(),
    };

    let model = model_from_config(&cfg.provider, &api_key)
        .map_err(|e| SessionError::new(format!("model error: {e}"), EX_SOFTWARE))?;

    Ok(Session { model, key_env })
}

/// Print a session error the way every command should and turn it into an exit code.
pub fn fail(command: &str, e: &SessionError) -> ExitCode {
    eprintln!("{command}: {}", e.detail);
    ExitCode::from(e.code)
}
