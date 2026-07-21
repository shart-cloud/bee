//! US9 integration tests: remote MCP servers are domain-gated (004-mcp-client, T022/T023).
//!
//! These exercise the **refusal** path, which is deny-by-default and needs no network — a refused
//! domain is rejected before any socket is opened (SC-021). The positive connect (SC-022) needs a
//! live remote endpoint and is out of scope here.

#![cfg(feature = "mcp")]

use bee_harness::mcp::config::{McpServerConfig, McpTransport};
use bee_harness::mcp::policy::DomainPattern;
use bee_harness::mcp::McpBridge;
use bee_harness::sandbox::Sandbox;
use bee_harness::McpPolicy;

fn patterns(items: &[&str]) -> Vec<DomainPattern> {
    items.iter().map(|s| DomainPattern::try_from(s.to_string()).unwrap()).collect()
}

fn remote_server(name: &str, url: &str, token_env: Option<&str>) -> McpServerConfig {
    McpServerConfig {
        name: name.into(),
        transport: McpTransport::StreamableHttp,
        command: None,
        args: vec![],
        env: Default::default(),
        url: Some(url.into()),
        token_env: token_env.map(str::to_string),
        allowed_tools: None,
        denied_tools: None,
    }
}

fn policy(allow: &[&str], deny: &[&str], servers: Vec<McpServerConfig>) -> McpPolicy {
    McpPolicy {
        enabled: true,
        allowed_domains: patterns(allow),
        denied_domains: patterns(deny),
        servers,
        ..Default::default()
    }
}

/// Assert the single server in the bridge is `Failed` and its reason contains `needle`.
async fn expect_refused(pol: McpPolicy, needle: &str) {
    let bridge = McpBridge::connect(pol, &Sandbox::host(vec![])).await;
    let (_, srv) = bridge.servers().next().expect("one server present");
    let label = srv.status().label();
    assert!(label.starts_with("failed:"), "expected failed, got: {label}");
    assert!(
        label.contains(needle),
        "reason {label:?} should mention {needle:?}"
    );
    assert_eq!(srv.tool_count(), 0, "a refused server contributes no tools");
}

#[tokio::test]
async fn not_in_allowlist_is_refused() {
    let pol = policy(
        &["mcp.internal.dev"],
        &[],
        vec![remote_server("evil", "https://mcp.evil.dev/tools", None)],
    );
    expect_refused(pol, "not in the allowlist").await;
}

#[tokio::test]
async fn denied_domain_wins_over_wildcard() {
    let pol = policy(
        &["*.company.com"],
        &["admin.company.com"],
        vec![remote_server("admin", "https://admin.company.com/mcp", None)],
    );
    expect_refused(pol, "denylist").await;
}

#[tokio::test]
async fn empty_allowlist_refuses_all_remote() {
    let pol = policy(&[], &[], vec![remote_server("any", "https://anything.example/mcp", None)]);
    expect_refused(pol, "not in the allowlist").await;
}

/// SC-028 (remote): the `token_env` value never appears in any status/error text. A refused domain
/// is rejected before the token is even read, so a leak here would be egregious — assert it directly.
#[tokio::test]
async fn token_value_never_leaks_in_status() {
    const SENTINEL: &str = "SENTINEL-TOKEN-do-not-leak-1234";
    // SAFETY: single-threaded test setup; the value is a non-secret sentinel.
    unsafe { std::env::set_var("BEE_TEST_MCP_TOKEN", SENTINEL) };

    let pol = policy(
        &["mcp.internal.dev"],
        &[],
        vec![remote_server("evil", "https://mcp.evil.dev/tools", Some("BEE_TEST_MCP_TOKEN"))],
    );
    let bridge = McpBridge::connect(pol, &Sandbox::host(vec![])).await;
    for (name, srv) in bridge.servers() {
        let label = srv.status().label();
        assert!(!label.contains(SENTINEL), "server {name} leaked the token in: {label}");
    }
    unsafe { std::env::remove_var("BEE_TEST_MCP_TOKEN") };
}
