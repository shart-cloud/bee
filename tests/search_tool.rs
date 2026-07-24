//! End-to-end for the `search` tool's payload: the `bee search-worker` subcommand the tool execs
//! through the sandbox. Runs the real built binary (`CARGO_BIN_EXE_bee`) over a temp tree so the
//! ripgrep-library search, argv contract, glob filter, and case handling are all exercised as the
//! tool actually invokes them. (The tool's own `current_exe()` resolves to the test harness under
//! `cargo test`, so the Tool wrapper is covered by `search::worker_argv` unit tests instead.)

use std::process::Command;

fn bee() -> Command {
    Command::new(env!("CARGO_BIN_EXE_bee"))
}

/// A temp tree: a matching Rust file, a matching text file, and a non-matching one. `tag` keeps each
/// test's tree distinct so the default parallel test runner cannot have one test's `remove_dir_all`
/// race another's search.
fn fixture(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("bee-search-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    std::fs::write(dir.join("a.rs"), "fn needle() {}\nlet other = 1;\n").unwrap();
    std::fs::write(dir.join("sub/b.txt"), "a needle in text\nplain line\n").unwrap();
    std::fs::write(dir.join("c.rs"), "nothing to see here\n").unwrap();
    dir
}

fn run(args: &[&str]) -> (String, bool) {
    let out = bee().args(args).output().expect("run bee search-worker");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        out.status.success(),
    )
}

#[test]
fn matches_across_files_as_path_line_text() {
    let dir = fixture("matches");
    let (out, ok) = run(&[
        "search-worker",
        "--path",
        dir.to_str().unwrap(),
        "--",
        "needle",
    ]);
    assert!(ok, "worker should exit 0");
    assert!(out.contains("a.rs:1:fn needle() {}"), "rust match: {out}");
    assert!(
        out.contains("b.txt:1:a needle in text"),
        "text match: {out}"
    );
    assert!(!out.contains("c.rs"), "non-matching file present: {out}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn glob_restricts_the_file_set() {
    let dir = fixture("glob");
    let (out, ok) = run(&[
        "search-worker",
        "--path",
        dir.to_str().unwrap(),
        "--glob",
        "**/*.rs",
        "--",
        "needle",
    ]);
    assert!(ok);
    assert!(out.contains("a.rs"), "rust file should match: {out}");
    assert!(
        !out.contains("b.txt"),
        "glob should exclude the .txt: {out}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ignore_case_is_opt_in() {
    let dir = fixture("case");
    // Default smart-case: an uppercase pattern does NOT match the lowercase text.
    let (sensitive, _) = run(&[
        "search-worker",
        "--path",
        dir.to_str().unwrap(),
        "--",
        "NEEDLE",
    ]);
    assert!(
        !sensitive.contains("needle"),
        "smart-case leaked: {sensitive}"
    );
    // With --ignore-case it does.
    let (insensitive, ok) = run(&[
        "search-worker",
        "--path",
        dir.to_str().unwrap(),
        "--ignore-case",
        "--",
        "NEEDLE",
    ]);
    assert!(ok);
    assert!(
        insensitive.contains("a.rs"),
        "ignore-case missed it: {insensitive}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn search_is_a_default_tool() {
    // The model gets it without opting in — it belongs to the default set.
    assert!(bee::tools::DEFAULT_TOOLS.contains(&"search"));
    assert!(bee::tools::is_known_tool("search"));
    let reg = bee::tools::registry_for(&["search".to_string()], None);
    assert!(reg.contains("search"));
}
