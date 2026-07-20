//! Error types for policy parsing, compilation, and attenuation.

use thiserror::Error;

/// Errors from parsing/validating a [`crate::Policy`].
#[derive(Debug, Error, PartialEq, Eq)]
pub enum PolicyError {
    #[error("TOML parse error: {0}")]
    Toml(String),
    #[error("I/O error: {0}")]
    Io(String),
    #[error("invalid policy: {0}")]
    Invalid(String),
}

/// Errors from compiling a policy into a [`crate::CompiledPolicy`]. All are fail-closed.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum CompileError {
    /// A glob pattern cannot be lowered to a verifier-safe primitive (research R6).
    #[error("unsupported glob pattern '{0}': {1}")]
    UnsupportedGlob(String, String),
    #[error("cannot resolve executable '{0}': {1}")]
    UnresolvableExec(String, String),
    #[error("cannot resolve network host '{0}': {1}")]
    UnresolvableHost(String, String),
    #[error("malformed network rule '{0}': {1}")]
    BadNetRule(String, String),
}

/// A specific reason a derived policy exceeded its parent (FR-005 / SC-002).
#[derive(Debug, Error, PartialEq, Eq)]
#[error("attenuation violation on {capability}: {reason}")]
pub struct AttenuationError {
    /// Which capability over-reached, e.g. `filesystem:/etc/passwd`, `exec:curl`, `network:host:443`.
    pub capability: String,
    /// Human-readable explanation.
    pub reason: String,
}

impl AttenuationError {
    pub fn new(capability: impl Into<String>, reason: impl Into<String>) -> Self {
        AttenuationError {
            capability: capability.into(),
            reason: reason.into(),
        }
    }
}
