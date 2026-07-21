//! MCP (Model Context Protocol) client integration (004-mcp-client).
//!
//! ## Two enforcement models
//! * **Stdio servers are sandboxed children** — spawned via [`crate::sandbox::Sandbox::tool_command`]
//!   so they join the episode's scope cgroup and the kernel enforces the policy on their I/O
//!   (Constitution III). Their denials flow through the existing audit drain.
//! * **Remote servers are domain-gated** — a connection is refused at construction time unless the
//!   host matches the scenario's allowlist (deny-by-default, Constitution I).
//!
//! ## Compilation
//! The **config/policy data types** ([`config`], [`policy`]) are always compiled — pure `serde`, no
//! `rmcp` — so the fail-closed guard can detect an `[mcp]` section even in a build without
//! `--features mcp`. The **runtime** ([`bridge`], [`transport`], [`proxy`]) depends on `rmcp` and is
//! gated behind that feature (research R11, NFR-004/SC-026).

pub mod config;
pub mod policy;

pub use config::{McpServerConfig, McpTransport};
pub use policy::{DomainPattern, McpPolicy};

// The runtime (depends on `rmcp`) is gated behind the `mcp` feature (NFR-004/SC-026).
#[cfg(feature = "mcp")]
pub mod bridge;
#[cfg(feature = "mcp")]
pub mod proxy;

#[cfg(feature = "mcp")]
pub use bridge::{ConnectedServer, McpBridge, ServerStatus};
#[cfg(feature = "mcp")]
pub use proxy::McpToolProxy;
