//! `bee-harness` — drives an LLM agent through a multi-turn tool loop whose tools execute inside a
//! bee scope, producing an auditable [`transcript::EpisodeTranscript`].
//!
//! ## Build modes
//!
//! * **default** — tools run as hardened, credential-stripped **host** processes with no kernel
//!   scope (`Sandbox::Host`). Enough for the offline [`provider::MockModel`] tests and the live
//!   provider smoke; keeps the workspace default build free of the bpf toolchain (Constitution V,
//!   SC-005).
//! * **`--features enforce`** — additionally builds `bee-userspace`'s eBPF-LSM path so each tool
//!   call runs inside a real bee scope and kernel denials are drained into the transcript
//!   (`Sandbox::Enforced`). This is the mode the VM enforcement case exercises.
//!
//! The multi-turn loop, tool execution, retry/backoff, truncation, and audit correlation all live
//! here — not in Rig, which is confined behind the [`provider::Model`] seam (research H2/H3).

#![warn(rust_2018_idioms)]

pub mod batch;
pub mod config;
#[cfg(feature = "concurrent")]
pub mod concurrent;
pub mod episode;
pub mod provider;
pub mod repl;
pub mod sandbox;
pub mod scenario;
pub mod tools;
pub mod transcript;

pub use batch::{run_batch, BatchConfig, BatchError, BatchResult};
#[cfg(feature = "concurrent")]
pub use concurrent::run_concurrent;
pub use config::{ProviderConfig, ProviderType};
pub use episode::{run_episode, run_loop, LoopOptions, ProgressSink};
pub use provider::{
    model_from_config, Conversation, Message, Model, ModelError, ToolCall, ToolSchema, Turn,
};
pub use repl::{run_repl, ReplConfig, ReplOutput, ReplSession};

/// Mark the current process **non-dumpable** (FR-018, research H7): `/proc/<pid>/environ`, `mem`,
/// and `maps` become root-only, so a **same-uid** sandboxed tool child cannot read the still-in-
/// memory provider key from the harness. Call once at startup. Env-stripping (the primary defense)
/// happens per-child in [`sandbox`]; this closes the `/proc/self` read path.
pub fn set_non_dumpable() -> std::io::Result<()> {
    // SAFETY: `prctl(PR_SET_DUMPABLE, 0)` is a simple, side-effect-free process attribute set.
    let rc = unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0) };
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}
pub use sandbox::Sandbox;
pub use scenario::Scenario;
pub use tools::{Tool, ToolRegistry, ToolResult};
pub use transcript::{
    enforcement_trace, EnforcementEntry, EpisodeStatus, EpisodeTranscript, ScoreReport,
};
