//! US10: the REPL surfaces MCP servers and the domain policy (004-mcp-client, T027).
//!
//! `/mcp` prints `McpBridge::summary()` verbatim (the dispatch is a one-liner: `output.info(&summary)`)
//! and `/tools` lists the MCP proxies because they share the one `ToolRegistry`. This test exercises
//! the summary content with no network — a refused remote server still renders name/transport/status
//! and the allow/deny lists (US10 AS-2).

#![cfg(feature = "mcp")]

use bee_harness::mcp::config::{McpServerConfig, McpTransport};
use bee_harness::mcp::policy::DomainPattern;
use bee_harness::mcp::McpBridge;
use bee_harness::sandbox::Sandbox;
use bee_harness::McpPolicy;

#[tokio::test]
async fn mcp_summary_reports_servers_and_domain_policy() {
    let server = McpServerConfig {
        name: "gh".into(),
        transport: McpTransport::StreamableHttp,
        command: None,
        args: vec![],
        env: Default::default(),
        url: Some("https://mcp.evil.dev/tools".into()), // not in the allowlist → refused, no network
        token_env: None,
        allowed_tools: None,
        denied_tools: None,
    };
    let policy = McpPolicy {
        enabled: true,
        allowed_domains: vec![DomainPattern::try_from("mcp.internal.dev".to_string()).unwrap()],
        denied_domains: vec![],
        servers: vec![server],
        ..Default::default()
    };

    let bridge = McpBridge::connect(policy, &Sandbox::host(vec![])).await;
    let summary = bridge.summary();

    // What `/mcp` shows the operator (US10 AS-2).
    assert!(summary.contains("gh"), "server name missing: {summary}");
    assert!(
        summary.contains("[streamable_http]"),
        "transport missing: {summary}"
    );
    assert!(summary.contains("failed:"), "status missing: {summary}");
    assert!(
        summary.contains("allowed_domains: [mcp.internal.dev]"),
        "allowlist missing: {summary}"
    );
    assert!(
        summary.contains("denied_domains: []"),
        "denylist missing: {summary}"
    );
}

#[tokio::test]
async fn disabled_mcp_summary_is_concise() {
    let bridge = McpBridge::connect(McpPolicy::default(), &Sandbox::host(vec![])).await;
    assert_eq!(bridge.summary(), "MCP: disabled");
}
