//! `bee repl` must be indistinguishable from `bee-repl` (consolidation issue 04).
//!
//! An interactive session is harder to pin than a headless one: it reads stdin, and its output is
//! meant for a person. What these assert is the part a script or a person can check without a
//! terminal — the startup banner's content, the refusals, and that closed stdin ends the session
//! cleanly rather than hanging or panicking. The `mock` provider keeps every case offline.
//!
//! Feeding stdin and reading the banner is exactly how the front-end fallback is meant to behave:
//! a piped session is not a terminal, so it runs inline. That makes it testable at all.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// The `bee-repl` binary. `None` once issue 06 removes it.
fn repl_bin() -> Option<PathBuf> {
    let candidate = Path::new(env!("CARGO_BIN_EXE_bee"))
        .parent()?
        .join("bee-repl");
    candidate.exists().then_some(candidate)
}

fn write(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, body).unwrap();
    path
}

fn mock_provider(dir: &Path, name: &str, reply: &str) -> PathBuf {
    write(
        dir,
        name,
        &format!("[provider]\nprovider = \"mock\"\n\n[[provider.script]]\ntext = \"{reply}\"\n"),
    )
}

/// Run a command with `input` on stdin, closing it so the session ends.
fn run_with_stdin(bin: &Path, args: &[&str], input: &str) -> Output {
    let mut child = Command::new(bin)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    drop(child.stdin.take());
    child.wait_with_output().unwrap()
}

fn bee_repl(args: &[&str], input: &str) -> Output {
    let mut a = vec!["repl"];
    a.extend_from_slice(args);
    run_with_stdin(Path::new(env!("CARGO_BIN_EXE_bee")), &a, input)
}

/// `bee-repl` has no `--host`: it silently ran unenforced, which is what issue 05 stopped. Strip
/// the flag so the comparison is about behaviour under the same intent.
fn without_host<'a>(args: &[&'a str]) -> Vec<&'a str> {
    args.iter().copied().filter(|a| *a != "--host").collect()
}

fn old_bee_repl(args: &[&str], input: &str) -> Option<Output> {
    let Some(bin) = repl_bin() else {
        eprintln!(
            "[skip] parity comparison: no `bee-repl` beside the test binary. \
             Run `cargo build --workspace` first to compare against it."
        );
        return None;
    };
    Some(run_with_stdin(&bin, &without_host(args), input))
}

/// The banner lines both commands print, minus the first line (which names the command and so
/// legitimately differs) and minus the theme line when no theme is configured.
fn banner_fields(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .filter(|l| l.starts_with("  "))
        .map(|l| l.trim().to_string())
        .collect()
}

#[test]
fn the_banner_reports_the_same_session() {
    let dir = tempfile::tempdir().unwrap();
    let provider = mock_provider(dir.path(), "p.toml", "hi");
    let args = [
        "--provider",
        provider.to_str().unwrap(),
        "--no-bee",
        "--host",
        "--tools",
        "bash,read_file",
    ];

    let mine = bee_repl(&args, "");
    assert!(
        mine.status.success(),
        "{}",
        String::from_utf8_lossy(&mine.stderr)
    );
    let mine_out = String::from_utf8_lossy(&mine.stdout);
    assert!(mine_out.starts_with("bee repl"), "{mine_out}");
    assert!(mine_out.contains("model:  mock/scripted"), "{mine_out}");
    assert!(mine_out.contains("tools:  bash, read_file"), "{mine_out}");
    // Host mode is announced on stderr now, not buried in the banner (issue 05).
    assert!(
        String::from_utf8_lossy(&mine.stderr).contains("HOST MODE"),
        "{}",
        String::from_utf8_lossy(&mine.stderr)
    );

    if let Some(theirs) = old_bee_repl(&args, "") {
        assert_eq!(mine.status.code(), theirs.status.code());
        assert_eq!(
            banner_fields(&mine_out),
            banner_fields(&String::from_utf8_lossy(&theirs.stdout)),
            "banner fields differ"
        );
    }
}

#[test]
fn closed_stdin_ends_the_session_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    let provider = mock_provider(dir.path(), "p.toml", "hi");
    let out = bee_repl(
        &[
            "--provider",
            provider.to_str().unwrap(),
            "--no-bee",
            "--host",
        ],
        "",
    );
    assert!(out.status.success());
}

#[test]
fn a_missing_provider_is_a_usage_error() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("nope.toml");
    let out = bee_repl(&["--provider", missing.to_str().unwrap(), "--no-bee"], "");
    assert_eq!(out.status.code(), Some(64));
    assert!(String::from_utf8_lossy(&out.stderr).contains("nope.toml"));

    if let Some(theirs) = old_bee_repl(&["--provider", missing.to_str().unwrap()], "") {
        assert_eq!(out.status.code(), theirs.status.code());
    }
}

#[test]
fn no_provider_at_all_reports_what_is_missing() {
    // The resolver's contribution: `bee-repl` required `--provider` as a clap argument and said so
    // in usage. `bee repl` can take it from configuration, so its absence is a *completeness*
    // failure that names the places it could come from.
    let out = bee_repl(&["--no-bee"], "");
    assert_eq!(out.status.code(), Some(64));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("incomplete configuration"), "{stderr}");
    assert!(stderr.contains("--provider"), "{stderr}");
}

#[test]
fn a_transcript_is_saved_on_exit() {
    let dir = tempfile::tempdir().unwrap();
    let provider = mock_provider(dir.path(), "p.toml", "Hello there.");
    let save = dir.path().join("session.json");

    let out = bee_repl(
        &[
            "--provider",
            provider.to_str().unwrap(),
            "--no-bee",
            "--host",
            "--save",
            save.to_str().unwrap(),
        ],
        "hello\n",
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let written = std::fs::read_to_string(&save)
        .unwrap_or_else(|e| panic!("no transcript at {}: {e}", save.display()));
    serde_json::from_str::<serde_json::Value>(&written)
        .unwrap_or_else(|e| panic!("transcript is not JSON: {e}\n{written}"));
}

#[test]
fn a_full_screen_request_falls_back_when_piped() {
    // Piped output is not a terminal, so `--tui` degrades to inline with a note rather than
    // silently doing nothing — or worse, driving escape sequences into a pipe.
    let dir = tempfile::tempdir().unwrap();
    let provider = mock_provider(dir.path(), "p.toml", "hi");
    let out = bee_repl(
        &[
            "--provider",
            provider.to_str().unwrap(),
            "--no-bee",
            "--host",
            "--tui",
        ],
        "",
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.is_empty(),
        "a --tui request that cannot be honoured must say so"
    );
}

#[test]
fn mcp_configuration_without_the_feature_is_refused() {
    if cfg!(feature = "mcp") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let provider = mock_provider(dir.path(), "p.toml", "hi");
    let mcp = write(dir.path(), "mcp.toml", "[mcp]\nservers = []\n");

    let out = bee_repl(
        &[
            "--provider",
            provider.to_str().unwrap(),
            "--no-bee",
            "--host",
            "--mcp-config",
            mcp.to_str().unwrap(),
        ],
        "",
    );
    assert_eq!(out.status.code(), Some(64));
    assert!(String::from_utf8_lossy(&out.stderr).contains("--features mcp"));
}
