//! US8 integration tests for the stdio MCP path (004-mcp-client, T013–T015, T020).
//!
//! These drive a **real** stdio MCP server (`@modelcontextprotocol/server-filesystem`) so they
//! require Node + that package on PATH; they are `#[ignore]` by default and run explicitly:
//!   MCP_FS_BIN=$(which mcp-server-filesystem) \
//!     cargo test -p bee-harness --features mcp --test mcp_stdio -- --ignored --nocapture
//!
//! The kernel-denial assertion (SC-020) additionally needs `--features enforce,mcp` on a BPF-LSM VM.

#![cfg(feature = "mcp")]

use bee_harness::mcp::config::{McpServerConfig, McpTransport};
use bee_harness::mcp::McpBridge;
use bee_harness::sandbox::Sandbox;
use bee_harness::tools::ToolRegistry;
use bee_harness::McpPolicy;

fn fs_bin() -> String {
    std::env::var("MCP_FS_BIN").unwrap_or_else(|_| "mcp-server-filesystem".into())
}

fn fs_server(root: &str, denied: Option<Vec<String>>) -> McpServerConfig {
    McpServerConfig {
        name: "fs".into(),
        transport: McpTransport::Stdio,
        command: Some(fs_bin()),
        args: vec![root.to_string()],
        env: Default::default(),
        url: None,
        token_env: None,
        allowed_tools: None,
        denied_tools: denied,
    }
}

fn policy(server: McpServerConfig) -> McpPolicy {
    McpPolicy {
        enabled: true,
        servers: vec![server],
        ..Default::default()
    }
}

fn workspace(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("bee-mcp-smoke-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[tokio::test]
#[ignore = "needs Node + @modelcontextprotocol/server-filesystem on PATH (MCP_FS_BIN)"]
async fn connect_lists_and_calls() {
    let dir = workspace("list");
    std::fs::write(dir.join("hello.txt"), "hi from mcp").unwrap();

    let sandbox = Sandbox::host(vec![]);
    let bridge = McpBridge::connect(
        policy(fs_server(&dir.display().to_string(), None)),
        &sandbox,
    )
    .await;

    // Surface the server's status/tools so a connection failure is legible.
    for (name, srv) in bridge.servers() {
        eprintln!(
            "server {name}: {} ({} tools)",
            srv.status().label(),
            srv.tool_count()
        );
    }

    let mut reg = ToolRegistry::new();
    bridge.register_into(&mut reg);
    let names: Vec<String> = reg.schemas().into_iter().map(|s| s.name).collect();
    eprintln!("REGISTERED MCP TOOLS: {names:?}");

    assert!(
        names.iter().any(|n| n.starts_with("mcp__fs__")),
        "expected mcp__fs__* tools; got {names:?}"
    );
    assert!(names.iter().any(|n| n == "mcp__fs__read_text_file"));
    bridge.teardown();
}

/// T015: a real MCP tool call round-trips through the proxy and returns the file content.
#[tokio::test]
#[ignore = "needs Node + @modelcontextprotocol/server-filesystem on PATH (MCP_FS_BIN)"]
async fn call_read_returns_content() {
    let dir = workspace("read");
    let file = dir.join("hello.txt");
    std::fs::write(&file, "hi from mcp").unwrap();

    let sandbox = Sandbox::host(vec![]);
    let bridge = McpBridge::connect(
        policy(fs_server(&dir.display().to_string(), None)),
        &sandbox,
    )
    .await;
    let mut reg = ToolRegistry::new();
    bridge.register_into(&mut reg);

    let call = bee_harness::ToolCall {
        id: "c1".into(),
        name: "mcp__fs__read_text_file".into(),
        arguments: serde_json::json!({ "path": file.display().to_string() }),
    };
    let result = reg.execute(&call, &sandbox).await;
    eprintln!(
        "read result: is_error={} content={:?}",
        result.is_error, result.content
    );
    assert!(!result.is_error, "read failed: {}", result.content);
    assert!(
        result.content.contains("hi from mcp"),
        "unexpected content: {}",
        result.content
    );
    bridge.teardown();
}

/// T031 / FR-043: a `tools/list_changed` notification re-fetches the tool list and the loop's
/// refresh hook surfaces the new tool. Uses the notifying fixture server (grows `alpha` → `alpha`+
/// `beta`), driving the exact refresh the agent loop runs each turn.
#[tokio::test]
#[ignore = "needs Node (tests/fixtures/notify_server.mjs)"]
async fn tools_list_changed_refreshes_registry() {
    use std::time::Duration;
    let script = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/notify_server.mjs"
    );
    let cfg = McpServerConfig {
        name: "nf".into(),
        transport: McpTransport::Stdio,
        command: Some("node".into()),
        args: vec![script.into()],
        env: Default::default(),
        url: None,
        token_env: None,
        allowed_tools: None,
        denied_tools: None,
    };
    let sandbox = Sandbox::host(vec![]);
    let bridge = McpBridge::connect(policy(cfg), &sandbox).await;

    let mut reg = ToolRegistry::new();
    bridge.register_into(&mut reg);
    let names0: Vec<String> = reg.schemas().into_iter().map(|s| s.name).collect();
    assert!(
        names0.iter().any(|n| n == "mcp__nf__alpha"),
        "initial set: {names0:?}"
    );
    assert!(
        !names0.iter().any(|n| n == "mcp__nf__beta"),
        "beta should not be present yet"
    );

    // The loop's per-turn refresh: poll for the notification, then re-register (as run_loop does).
    let mut refreshed = false;
    for _ in 0..100 {
        tokio::time::sleep(Duration::from_millis(20)).await;
        if bridge.take_dirty() {
            reg.remove_mcp_tools();
            bridge.register_into(&mut reg);
            refreshed = true;
            break;
        }
    }
    assert!(refreshed, "no tools/list_changed observed within budget");

    let names1: Vec<String> = reg.schemas().into_iter().map(|s| s.name).collect();
    assert!(
        names1.iter().any(|n| n == "mcp__nf__beta"),
        "beta should appear after refresh: {names1:?}"
    );
    bridge.teardown();
}

/// T014: `denied_tools` removes a tool from the model-facing schema (FR-039).
#[tokio::test]
#[ignore = "needs Node + @modelcontextprotocol/server-filesystem on PATH (MCP_FS_BIN)"]
async fn denied_tools_are_hidden() {
    let dir = workspace("deny");
    let sandbox = Sandbox::host(vec![]);
    let server = fs_server(&dir.display().to_string(), Some(vec!["write_file".into()]));
    let bridge = McpBridge::connect(policy(server), &sandbox).await;
    let mut reg = ToolRegistry::new();
    bridge.register_into(&mut reg);
    let names: Vec<String> = reg.schemas().into_iter().map(|s| s.name).collect();

    assert!(
        names.iter().any(|n| n == "mcp__fs__read_text_file"),
        "read tool should remain"
    );
    assert!(
        !names.iter().any(|n| n == "mcp__fs__write_file"),
        "denied write_file must not appear: {names:?}"
    );
    bridge.teardown();
}
