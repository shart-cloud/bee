//! SC-026 / NFR-004: `rmcp` MUST NOT propagate into `bee-core` (or `bee-common`). The MCP client is
//! confined to the application package behind the `mcp` feature. This guards the dependency boundary from
//! regressions (e.g. someone moving an MCP type into the core crate).
//!
//! Implemented by asking `cargo tree` whether `rmcp` appears in `bee-core`'s dependency tree. It
//! runs without building anything (tree resolves from Cargo.lock). If `cargo` is unavailable in the
//! environment, the check is skipped rather than failing spuriously.

use std::process::Command;

fn cargo_tree_contains_rmcp(package: &str) -> Option<bool> {
    // `-i rmcp` inverts the tree onto rmcp; `-p <pkg>` scopes to that crate. With rmcp absent the
    // command exits non-zero ("did not match any packages") and prints nothing to stdout.
    let out = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .args(["tree", "-p", package, "-i", "rmcp"])
        .output()
        .ok()?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    // A real rmcp node prints a line beginning with the crate name.
    Some(stdout.lines().any(|l| l.trim_start().starts_with("rmcp")))
}

#[test]
fn rmcp_absent_from_bee_core() {
    match cargo_tree_contains_rmcp("bee-core") {
        Some(true) => panic!("SC-026 violation: `rmcp` is in bee-core's dependency tree"),
        Some(false) => {} // good — rmcp confined to the application package
        None => eprintln!("skipping rmcp dep-boundary check: `cargo tree` unavailable"),
    }
}
