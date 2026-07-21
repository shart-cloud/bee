//! `McpBridge` — owns the connected MCP servers for one episode/REPL session and registers their
//! tools into the shared [`ToolRegistry`] (004-mcp-client, T011).
//!
//! The bridge holds each server's `rmcp` [`RunningService`] (which owns the stdio child + background
//! task); dropping the bridge kills the children (kill-on-drop). `McpToolProxy`s in the registry
//! hold only a cheap [`Peer`] clone, so a late call after teardown returns a disconnected error
//! (FR-042). The per-transport `connect` bodies land in US8 (stdio, T018) and US9 (remote, T026).

use std::collections::BTreeMap;
use std::time::Duration;

use rmcp::service::{Peer, RoleClient, RunningService};
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::ServiceExt;

use crate::provider::ToolSchema;
use crate::sandbox::Sandbox;
use crate::tools::ToolRegistry;

use super::config::{McpServerConfig, McpTransport};
use super::policy::McpPolicy;
use super::proxy::{tool_schema, McpToolProxy};
use super::transport::spawn_stdio;

/// Stdio spawn + `initialize` handshake budget (NFR-005).
const STDIO_STARTUP_TIMEOUT: Duration = Duration::from_secs(10);
/// Remote TCP + TLS + `initialize` handshake budget (NFR-005).
const REMOTE_STARTUP_TIMEOUT: Duration = Duration::from_secs(15);

/// Lifecycle state of a configured MCP server.
#[derive(Debug, Clone)]
pub enum ServerStatus {
    /// Connected and its (filtered) tools registered.
    Connected,
    /// Was connected; the child crashed or the peer died (FR-042).
    Disconnected,
    /// Never became usable (spawn/initialize failed or timed out); carries the reason.
    Failed(String),
}

impl ServerStatus {
    pub fn label(&self) -> String {
        match self {
            ServerStatus::Connected => "connected".into(),
            ServerStatus::Disconnected => "disconnected".into(),
            ServerStatus::Failed(why) => format!("failed: {why}"),
        }
    }
}

/// The live `rmcp` handle for a connected server. Held for its RAII kill-on-drop; the `peer` is
/// cloned into each tool proxy.
struct ServerHandle {
    /// Owns the child + background task. Held, not read — dropping it tears the connection down.
    #[allow(dead_code)]
    service: RunningService<RoleClient, ()>,
    peer: Peer<RoleClient>,
}

/// One configured server and whatever we learned when connecting.
pub struct ConnectedServer {
    config: McpServerConfig,
    /// The surviving tools after `allowed_tools`/`denied_tools` filtering: `(bare_name, schema)`
    /// where `schema.name` is already the namespaced `mcp__{server}__{tool}`.
    tools: Vec<(String, ToolSchema)>,
    status: ServerStatus,
    handle: Option<ServerHandle>,
}

impl ConnectedServer {
    pub fn config(&self) -> &McpServerConfig {
        &self.config
    }
    pub fn status(&self) -> &ServerStatus {
        &self.status
    }
    pub fn tool_count(&self) -> usize {
        self.tools.len()
    }
}

/// Manages the connected MCP servers for an episode/REPL session.
pub struct McpBridge {
    policy: McpPolicy,
    servers: BTreeMap<String, ConnectedServer>,
}

impl McpBridge {
    /// A disabled bridge (no servers). Used when `[mcp]` is absent or `enabled = false`.
    pub fn disabled() -> Self {
        McpBridge { policy: McpPolicy::default(), servers: BTreeMap::new() }
    }

    /// Connect the configured servers under `policy`, spawning stdio children in `sandbox`.
    ///
    /// Never fails the episode: a server that cannot connect is recorded as [`ServerStatus::Failed`]
    /// and simply contributes no tools (Constitution I, fail-closed capability absence). The
    /// per-transport connection bodies are filled in US8 (stdio) and US9 (remote); today every
    /// declared server is recorded as `Failed("transport not yet implemented")`.
    pub async fn connect(policy: McpPolicy, sandbox: &Sandbox) -> Self {
        let mut servers = BTreeMap::new();
        if policy.enabled {
            let max = policy.max_servers as usize;
            for cfg in policy.servers.iter().take(max) {
                let server = match cfg.transport {
                    McpTransport::Stdio => connect_stdio(cfg, sandbox).await,
                    McpTransport::Sse | McpTransport::StreamableHttp => {
                        connect_remote(cfg, &policy).await
                    }
                };
                servers.insert(cfg.name.clone(), server);
            }
        }
        McpBridge { policy, servers }
    }

    /// Register one [`McpToolProxy`] per surviving tool of each connected server into `registry`
    /// (FR-036/FR-039). Filtering already happened at connect time (`ConnectedServer.tools`).
    pub fn register_into(&self, registry: &mut ToolRegistry) {
        let timeout = self.policy.tool_timeout();
        for (name, srv) in &self.servers {
            if !matches!(srv.status, ServerStatus::Connected) {
                continue;
            }
            let Some(handle) = &srv.handle else { continue };
            for (bare, schema) in &srv.tools {
                let proxy = McpToolProxy::new(
                    name,
                    bare.clone(),
                    schema.clone(),
                    handle.peer.clone(),
                    timeout,
                );
                registry.insert(Box::new(proxy));
            }
        }
    }

    /// The connected servers, for the `/mcp` REPL command and status reporting (US10).
    pub fn servers(&self) -> impl Iterator<Item = (&str, &ConnectedServer)> {
        self.servers.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// The policy this bridge was built from (exposes the domain allowlist for `/mcp`).
    pub fn policy(&self) -> &McpPolicy {
        &self.policy
    }

    /// Tear down: drop every server handle, killing stdio children. Consumes the bridge.
    pub fn teardown(self) {
        // Dropping `self.servers` drops each `RunningService`, whose kill-on-drop reaps the child.
    }
}

fn failed_server(cfg: &McpServerConfig, why: String) -> ConnectedServer {
    ConnectedServer { config: cfg.clone(), tools: Vec::new(), status: ServerStatus::Failed(why), handle: None }
}

/// After the `initialize` handshake, fetch + filter the server's tools and build the connected
/// record. Shared by the stdio and remote paths.
async fn finalize_connection(
    cfg: &McpServerConfig,
    running: RunningService<RoleClient, ()>,
) -> ConnectedServer {
    let tools = match running.list_all_tools().await {
        Ok(t) => t,
        Err(e) => return failed_server(cfg, format!("list_tools failed: {e}")),
    };
    // Apply allowed_tools/denied_tools (FR-039) and namespace the surviving schemas (FR-036).
    let filtered: Vec<(String, ToolSchema)> = tools
        .iter()
        .filter(|t| cfg.tool_allowed(&t.name))
        .map(|t| (t.name.to_string(), tool_schema(&cfg.name, t)))
        .collect();
    let peer = running.peer().clone();
    ConnectedServer {
        config: cfg.clone(),
        tools: filtered,
        status: ServerStatus::Connected,
        handle: Some(ServerHandle { service: running, peer }),
    }
}

/// Connect one stdio MCP server: spawn it sandboxed, run the `initialize` handshake under the
/// startup budget, then finalize. Never panics — any failure yields a [`ServerStatus::Failed`]
/// server with no tools (fail-closed, Constitution I).
async fn connect_stdio(cfg: &McpServerConfig, sandbox: &Sandbox) -> ConnectedServer {
    let transport = match spawn_stdio(sandbox, cfg) {
        Ok(t) => t,
        Err(e) => return failed_server(cfg, e),
    };
    // `()` is the no-op client handler; `on_tool_list_changed` support lands in T031.
    let running = match tokio::time::timeout(STDIO_STARTUP_TIMEOUT, ().serve(transport)).await {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => return failed_server(cfg, format!("initialize failed: {e}")),
        Err(_) => {
            return failed_server(cfg, format!("initialize timed out (>{}s)", STDIO_STARTUP_TIMEOUT.as_secs()))
        }
    };
    finalize_connection(cfg, running).await
}

/// Connect one remote MCP server over Streamable HTTP. The domain gate is checked **first** — a
/// refused domain yields a `Failed` server with **no transport opened** (SC-021). The `token_env`
/// value (if any) becomes the `Authorization: Bearer` header (FR-041); it is never logged.
async fn connect_remote(cfg: &McpServerConfig, policy: &McpPolicy) -> ConnectedServer {
    let url = match cfg.url.as_deref().map(str::trim).filter(|u| !u.is_empty()) {
        Some(u) => u,
        None => return failed_server(cfg, "remote transport requires `url`".into()),
    };

    // Deny-by-default domain gate, BEFORE any socket is opened (Constitution I, SC-021).
    if let Err(reason) = policy.allow_url(url) {
        return failed_server(cfg, reason);
    }

    // `transport = "sse"` is a deprecated alias for the Streamable-HTTP client (research R2); both
    // land here. (No logger to warn through; the deprecation is documented in the config contract.)
    let mut config = StreamableHttpClientTransportConfig::with_uri(url.to_string());
    if let Some(name) = &cfg.token_env {
        // rmcp applies this via `bearer_auth`, i.e. it prepends "Bearer " — pass the raw token.
        if let Ok(token) = std::env::var(name) {
            if !token.is_empty() {
                config = config.auth_header(token);
            }
        }
    }

    let transport = StreamableHttpClientTransport::from_config(config);
    let running = match tokio::time::timeout(REMOTE_STARTUP_TIMEOUT, ().serve(transport)).await {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => return failed_server(cfg, format!("initialize failed: {e}")),
        Err(_) => {
            return failed_server(cfg, format!("initialize timed out (>{}s)", REMOTE_STARTUP_TIMEOUT.as_secs()))
        }
    };
    finalize_connection(cfg, running).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_bridge_registers_nothing() {
        let bridge = McpBridge::disabled();
        let mut registry = ToolRegistry::new();
        bridge.register_into(&mut registry);
        assert!(registry.schemas().is_empty());
    }

    #[tokio::test]
    async fn connect_disabled_policy_is_empty() {
        let bridge = McpBridge::connect(McpPolicy::default(), &Sandbox::host(vec![])).await;
        assert_eq!(bridge.servers().count(), 0);
    }
}
