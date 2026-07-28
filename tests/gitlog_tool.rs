//! 016-native-tools US5 — the `git_log` tool's payload: the `bee gitlog-worker` subcommand the tool
//! execs through the sandbox, covered the way `tests/astgrep_tool.rs` covers `astgrep-worker`.
//!
//! The fixture is a real repository built with the `git` CLI rather than a checked-in one. A `.git`
//! directory inside a git repository is not something this repo can carry, and a fixture built by
//! hand would be asserting against bee's idea of a repository instead of git's.
//!
//! The load-bearing assertion is [`a_path_outside_any_repository_says_so`]: a history that cannot be
//! read must never render as a history with nothing in it (FR-012, US5 scenario 3).

#![cfg(feature = "gitlog")]

use std::path::Path;
use std::process::Command;

fn bee() -> Command {
    Command::new(env!("CARGO_BIN_EXE_bee"))
}

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
    ok: bool,
}

fn run(args: &[&str]) -> Run {
    let out = bee().args(args).output().expect("run bee gitlog-worker");
    Run {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        code: out.status.code(),
        ok: out.status.success(),
    }
}

fn git(repo: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(repo)
        .env("GIT_AUTHOR_NAME", "Ada Lovelace")
        .env("GIT_AUTHOR_EMAIL", "ada@example.com")
        .env("GIT_COMMITTER_NAME", "Ada Lovelace")
        .env("GIT_COMMITTER_EMAIL", "ada@example.com")
        .output()
        .unwrap_or_else(|e| panic!("these tests need the `git` CLI to build their fixture: {e}"));
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A two-commit repository whose second commit changes exactly one line. That is what makes blame
/// falsifiable: line 1 belongs to the first commit and line 3 to the second, so a blame that simply
/// reported the tip would pass on one line and fail on the other.
struct Fixture {
    _dir: tempfile::TempDir,
    path: std::path::PathBuf,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_path_buf();
    git(&path, &["init", "--initial-branch=main", "."]);
    git(&path, &["config", "commit.gpgsign", "false"]);

    std::fs::write(path.join("app.py"), "one\ntwo\nthree\n").unwrap();
    git(&path, &["add", "app.py"]);
    git(&path, &["commit", "-m", "first: add the file"]);

    std::fs::write(path.join("app.py"), "one\ntwo\nCHANGED\n").unwrap();
    git(&path, &["add", "app.py"]);
    git(&path, &["commit", "-m", "second: change the last line"]);

    Fixture { _dir: dir, path }
}

fn short_sha(repo: &Path, rev: &str) -> String {
    let out = Command::new("git")
        .args(["rev-parse", "--short=8", rev])
        .current_dir(repo)
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// US5 scenario 1 — a log carries who, when, and what, for each commit.
#[test]
fn log_reports_author_date_and_summary() {
    let f = fixture();
    let r = run(&[
        "gitlog-worker",
        "--path",
        f.path.to_str().unwrap(),
        "--mode",
        "log",
        "--limit",
        "10",
    ]);
    assert!(r.ok, "worker should exit 0: {}", r.stderr);

    let lines: Vec<&str> = r.stdout.lines().filter(|l| !l.is_empty()).collect();
    assert_eq!(lines.len(), 2, "two commits, got:\n{}", r.stdout);

    // Newest first, the order every `git log` reader already expects.
    assert!(
        lines[0].contains("second: change the last line"),
        "{}",
        lines[0]
    );
    assert!(lines[1].contains("first: add the file"), "{}", lines[1]);
    for line in &lines {
        assert!(line.contains("Ada Lovelace"), "author missing: {line}");
        // An ISO-8601 date, not a raw epoch: the reader is a person or a model, not a clock.
        assert!(
            line.contains("T") && line.contains("Z"),
            "date missing: {line}"
        );
    }
    assert!(
        lines[0].starts_with(&short_sha(&f.path, "HEAD")),
        "expected the tip's sha first: {}",
        lines[0]
    );
}

/// US5 scenario 2 — blame names the commit that introduced a specific line.
#[test]
fn blame_identifies_the_commit_that_introduced_a_line() {
    let f = fixture();
    let head = short_sha(&f.path, "HEAD");
    let first = short_sha(&f.path, "HEAD~1");

    let line3 = run(&[
        "gitlog-worker",
        "--path",
        f.path.join("app.py").to_str().unwrap(),
        "--mode",
        "blame",
        "--line",
        "3",
    ]);
    assert!(line3.ok, "worker should exit 0: {}", line3.stderr);
    assert!(
        line3.stdout.contains(&head),
        "line 3 was changed by the second commit ({head}), got: {}",
        line3.stdout
    );

    // The other half of the assertion: an untouched line still belongs to the commit that wrote it.
    // Without this, a blame that always answered "HEAD" would pass.
    let line1 = run(&[
        "gitlog-worker",
        "--path",
        f.path.join("app.py").to_str().unwrap(),
        "--mode",
        "blame",
        "--line",
        "1",
    ]);
    assert!(line1.ok, "worker should exit 0: {}", line1.stderr);
    assert!(
        line1.stdout.contains(&first),
        "line 1 is still the first commit ({first}), got: {}",
        line1.stdout
    );
}

/// US5 scenario 3 / FR-012 — the refusal that matters. A directory with no repository above it has
/// no history to report, and reporting *nothing* would be indistinguishable from a repository whose
/// history is empty.
#[test]
fn a_path_outside_any_repository_says_so() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("loose.txt"), "not versioned\n").unwrap();

    let r = run(&[
        "gitlog-worker",
        "--path",
        dir.path().to_str().unwrap(),
        "--mode",
        "log",
    ]);
    assert!(!r.ok, "a non-repository must not exit 0");
    assert_eq!(
        r.code,
        Some(bee::gitlog::EXIT_NOT_A_REPOSITORY),
        "the tool wrapper maps this exit code to `Unavailable {{ NotARepository }}`; \
         stderr was: {}",
        r.stderr
    );
    assert!(
        r.stdout.trim().is_empty(),
        "a refusal must print no history: {}",
        r.stdout
    );
    assert!(
        r.stderr.contains("not a repository") || r.stderr.contains("not inside a repository"),
        "the diagnostic should name the problem: {}",
        r.stderr
    );
}

/// A repository with commits, asked about a file that has none, is a *clean* answer — the empty
/// case that the refusal above must stay distinguishable from.
#[test]
fn an_untracked_file_is_an_empty_history_not_a_refusal() {
    let f = fixture();
    std::fs::write(f.path.join("untracked.py"), "x = 1\n").unwrap();

    let r = run(&[
        "gitlog-worker",
        "--path",
        f.path.join("untracked.py").to_str().unwrap(),
        "--mode",
        "log",
    ]);
    assert!(
        r.ok,
        "a tracked-nothing file inside a repository is not a failure: {}",
        r.stderr
    );
    assert!(
        r.stdout.trim().is_empty(),
        "no commits touch it, so nothing to report: {}",
        r.stdout
    );
}

/// The limit is a bound on output, and it takes from the top (newest), not the bottom.
#[test]
fn the_limit_keeps_the_newest_commits() {
    let f = fixture();
    let r = run(&[
        "gitlog-worker",
        "--path",
        f.path.to_str().unwrap(),
        "--mode",
        "log",
        "--limit",
        "1",
    ]);
    assert!(r.ok, "{}", r.stderr);
    let lines: Vec<&str> = r.stdout.lines().filter(|l| !l.is_empty()).collect();
    assert_eq!(lines.len(), 1);
    assert!(
        lines[0].contains("second: change the last line"),
        "{}",
        lines[0]
    );
}

/// Blame needs a line number; asking for one past the end of the file is a diagnostic, not a guess.
#[test]
fn a_line_outside_the_file_is_refused_with_a_diagnostic() {
    let f = fixture();
    let r = run(&[
        "gitlog-worker",
        "--path",
        f.path.join("app.py").to_str().unwrap(),
        "--mode",
        "blame",
        "--line",
        "999",
    ]);
    assert!(!r.ok, "a line that does not exist cannot be blamed");
    assert!(
        r.stdout.trim().is_empty(),
        "no invented attribution: {}",
        r.stdout
    );
}
