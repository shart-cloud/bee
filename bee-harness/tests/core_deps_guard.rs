//! T037 — the `bee-core` dependency guard (Constitution V, FR-021; SC-005).
//!
//! The whole TUI (ratatui, crossterm, color-eyre, textwrap) and the async runtime live in
//! `bee-harness` **only**. `bee-core` is the policy/enforcement core: it must stay free of terminal
//! and runtime dependencies so it can be audited, embedded, and built without a UI toolchain.
//!
//! This is easy to break by accident — one `use` in the wrong crate and Cargo quietly pulls a
//! terminal backend into the core. So we assert it structurally, against the manifests themselves,
//! rather than trusting review.

use std::path::{Path, PathBuf};

/// Crates that must never appear in `bee-core`'s (or `bee-common`'s) dependency tables.
const FORBIDDEN: &[&str] = &[
    // Terminal / UI
    "ratatui",
    "crossterm",
    "termion",
    "color-eyre",
    "textwrap",
    "rustyline",
    "indicatif",
    // Async runtimes
    "tokio",
    "async-std",
    "smol",
    "futures-util",
    // Scripting / rendering brought in by the harness
    "rhai",
    // MCP client
    "rmcp",
];

fn workspace_root() -> PathBuf {
    // CARGO_MANIFEST_DIR is bee-harness/; the workspace is its parent.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf()
}

/// Every dependency name declared in `manifest`, across normal/dev/build and target-specific tables.
/// Deliberately a **textual** scan of dependency table keys: it needs no toml crate in dev-deps and
/// catches a forbidden crate however it is declared (plain, table, renamed via `package = `).
fn declared_deps(manifest: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut in_deps = false;
    for raw in manifest.lines() {
        let line = raw.trim();
        if line.starts_with('[') {
            // e.g. [dependencies], [dev-dependencies], [target.'cfg(unix)'.dependencies]
            in_deps = line.ends_with("dependencies]");
            continue;
        }
        if !in_deps || line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, rest)) = line.split_once('=') {
            let key = key.trim().trim_matches('"');
            if !key.is_empty() {
                names.push(key.to_string());
            }
            // A renamed dependency hides the real crate behind `package = "..."`.
            if let Some(idx) = rest.find("package") {
                let tail = &rest[idx..];
                if let Some(start) = tail.find('"') {
                    if let Some(end) = tail[start + 1..].find('"') {
                        names.push(tail[start + 1..start + 1 + end].to_string());
                    }
                }
            }
        }
    }
    names
}

fn assert_clean(crate_name: &str) {
    let path = workspace_root().join(crate_name).join("Cargo.toml");
    let manifest = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    let deps = declared_deps(&manifest);

    let leaked: Vec<&String> = deps
        .iter()
        .filter(|d| FORBIDDEN.contains(&d.as_str()))
        .collect();

    assert!(
        leaked.is_empty(),
        "{crate_name} must stay free of terminal/async-runtime dependencies (Constitution V, \
         FR-021), but its manifest declares: {leaked:?}.\n\
         The TUI and runtime belong in bee-harness — move the code rather than the dependency."
    );
}

#[test]
fn bee_core_has_no_terminal_or_runtime_dependency() {
    assert_clean("bee-core");
}

#[test]
fn bee_common_has_no_terminal_or_runtime_dependency() {
    assert_clean("bee-common");
}

#[test]
fn the_guard_actually_detects_a_leak() {
    // A guard that cannot fail is worthless — prove the parser sees a forbidden dep in every shape
    // it could be declared in.
    let manifest = r#"
[package]
name = "fake"

[dependencies]
serde = "1"
ratatui = { version = "0.29", default-features = false }

[dev-dependencies]
tokio = { version = "1", features = ["macros"] }

[target.'cfg(unix)'.dependencies]
term = { package = "crossterm", version = "0.28" }
"#;
    let deps = declared_deps(manifest);
    for expected in ["ratatui", "tokio", "crossterm"] {
        assert!(
            deps.contains(&expected.to_string()),
            "parser missed {expected} in {deps:?}"
        );
    }
    // And a plain manifest is clean.
    assert!(!declared_deps("[dependencies]\nserde = \"1\"\n")
        .iter()
        .any(|d| FORBIDDEN.contains(&d.as_str())));
}
