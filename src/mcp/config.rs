//! MCP server configuration (004-mcp-client, `[[mcp.servers]]`). Pure declarative data
//! (Constitution IV) — **no `rmcp` dependency**, so it compiles in every build and lets the
//! fail-closed guard detect an `[mcp]` section even when the crate was built without `--features
//! mcp` (research R11). The runtime that consumes it (`bridge`, `transport`, `proxy`) is gated.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// How the harness talks to an MCP server.
///
/// `Sse` is a **deprecated alias** kept for config compatibility: rmcp 2.x removed the SSE client,
/// so it routes to the Streamable-HTTP client with a warning (research R2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpTransport {
    /// Child process over stdin/stdout — spawned as a sandboxed bee child (kernel-enforced).
    Stdio,
    /// Deprecated alias for `StreamableHttp` (rmcp 2.x has no SSE client).
    Sse,
    /// Remote HTTP, domain-gated at connection time.
    StreamableHttp,
}

impl McpTransport {
    /// True for transports that reach a remote host (and are therefore domain-gated).
    pub fn is_remote(self) -> bool {
        matches!(self, McpTransport::Sse | McpTransport::StreamableHttp)
    }
}

/// One `[[mcp.servers]]` entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    /// Routing key; becomes the `mcp__{name}__…` tool prefix. `^[a-z0-9_-]+$`, no `__`.
    pub name: String,
    pub transport: McpTransport,

    // --- stdio ---
    /// Program to spawn (required for stdio).
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    /// Env vars added atop the stripped environment (never silently re-adds a secret).
    #[serde(default)]
    pub env: BTreeMap<String, String>,

    // --- remote ---
    /// Endpoint URL (required for remote); its host is checked against the domain allowlist.
    #[serde(default)]
    pub url: Option<String>,
    /// **Name** of the env var holding the Bearer token (never the token itself; FR-041).
    #[serde(default)]
    pub token_env: Option<String>,

    // --- tool filtering (convenience, not a security boundary) ---
    /// If set, only these bare tool names are exposed.
    #[serde(default)]
    pub allowed_tools: Option<Vec<String>>,
    /// Bare tool names never exposed; overrides `allowed_tools` on conflict.
    #[serde(default)]
    pub denied_tools: Option<Vec<String>>,
}

impl McpServerConfig {
    /// Whether a bare tool name survives this server's `allowed_tools`/`denied_tools` filter (FR-039).
    /// `denied_tools` wins over `allowed_tools`.
    pub fn tool_allowed(&self, tool: &str) -> bool {
        if let Some(allow) = &self.allowed_tools {
            if !allow.iter().any(|t| t == tool) {
                return false;
            }
        }
        if let Some(deny) = &self.denied_tools {
            if deny.iter().any(|t| t == tool) {
                return false;
            }
        }
        true
    }

    /// Semantic validation for one server entry. Returns a human-readable error string on failure
    /// (the caller wraps it in `ConfigError::Invalid`).
    pub fn validate(&self) -> Result<(), String> {
        // Name: non-empty, [a-z0-9_-], and no `__` so `mcp__{name}__{tool}` splits unambiguously.
        if self.name.is_empty() {
            return Err("mcp server name is required".into());
        }
        if self.name.contains("__") {
            return Err(format!(
                "mcp server name '{}' must not contain '__'",
                self.name
            ));
        }
        if !self
            .name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
        {
            return Err(format!(
                "mcp server name '{}' must match [a-z0-9_-]",
                self.name
            ));
        }
        match self.transport {
            McpTransport::Stdio => {
                if self.command.as_deref().unwrap_or("").trim().is_empty() {
                    return Err(format!(
                        "mcp server '{}': stdio transport requires `command`",
                        self.name
                    ));
                }
            }
            McpTransport::Sse | McpTransport::StreamableHttp => {
                if self.url.as_deref().unwrap_or("").trim().is_empty() {
                    return Err(format!(
                        "mcp server '{}': remote transport requires `url`",
                        self.name
                    ));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_filter_denied_overrides_allowed() {
        let mut c = McpServerConfig {
            name: "fs".into(),
            transport: McpTransport::Stdio,
            command: Some("srv".into()),
            args: vec![],
            env: BTreeMap::new(),
            url: None,
            token_env: None,
            allowed_tools: Some(vec!["read".into(), "write".into()]),
            denied_tools: Some(vec!["write".into()]),
        };
        assert!(c.tool_allowed("read"));
        assert!(!c.tool_allowed("write")); // denied wins
        assert!(!c.tool_allowed("exec")); // not in allowlist
        c.allowed_tools = None;
        assert!(c.tool_allowed("exec")); // no allowlist → allow all but denied
        assert!(!c.tool_allowed("write"));
    }

    #[test]
    fn stdio_requires_command_remote_requires_url() {
        let stdio_bad = McpServerConfig {
            name: "fs".into(),
            transport: McpTransport::Stdio,
            command: None,
            args: vec![],
            env: BTreeMap::new(),
            url: None,
            token_env: None,
            allowed_tools: None,
            denied_tools: None,
        };
        assert!(stdio_bad.validate().is_err());

        let remote_bad = McpServerConfig {
            transport: McpTransport::StreamableHttp,
            url: None,
            ..stdio_bad.clone()
        };
        assert!(remote_bad.validate().is_err());
    }

    #[test]
    fn rejects_name_with_double_underscore() {
        let c = McpServerConfig {
            name: "a__b".into(),
            transport: McpTransport::Stdio,
            command: Some("srv".into()),
            args: vec![],
            env: BTreeMap::new(),
            url: None,
            token_env: None,
            allowed_tools: None,
            denied_tools: None,
        };
        assert!(c.validate().is_err());
    }
}
