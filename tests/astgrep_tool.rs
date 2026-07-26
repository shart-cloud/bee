//! 016-native-tools US1 — the `ast_grep` tool's payload: the `bee astgrep-worker` subcommand the
//! tool execs through the sandbox. Runs the real built binary over the checked-in decoy fixture, the
//! same way `tests/search_tool.rs` covers `search-worker`.
//!
//! The load-bearing assertion is [`structural_search_ignores_comments_and_strings`]: it is the
//! difference between this tool and the regex `search` that already exists, and it is SC-007.
//!
//! Requires `--features astgrep-rust`; without it the whole file self-skips, because a build that
//! compiled no grammar has nothing to assert about grammars.

#![cfg(feature = "astgrep-rust")]

use std::process::Command;

fn bee() -> Command {
    Command::new(env!("CARGO_BIN_EXE_bee"))
}

fn fixture() -> String {
    concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/astgrep").to_string()
}

struct Run {
    stdout: String,
    stderr: String,
    ok: bool,
}

fn run(args: &[&str]) -> Run {
    let out = bee().args(args).output().expect("run bee astgrep-worker");
    Run {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        ok: out.status.success(),
    }
}

/// SC-007 / US1 scenario 1 — the whole reason this tool exists.
///
/// `tests/fixtures/astgrep/decoy.rs` contains the text `v.unwrap()` three times: as a real method
/// call (line 8), inside a comment (line 11), and inside a string literal (line 14). A regex returns
/// all three. Matching on the parse tree must return only the call.
#[test]
fn structural_search_ignores_comments_and_strings() {
    let r = run(&[
        "astgrep-worker",
        "--lang",
        "rust",
        "--path",
        &fixture(),
        "--",
        "$X.unwrap()",
    ]);
    assert!(r.ok, "worker should exit 0: {}", r.stderr);

    let lines: Vec<&str> = r.stdout.lines().filter(|l| !l.is_empty()).collect();
    assert_eq!(
        lines.len(),
        1,
        "expected exactly the live-code match, got:\n{}",
        r.stdout
    );
    assert!(
        lines[0].contains("decoy.rs:8:"),
        "match should be the call on line 8, got: {}",
        lines[0]
    );
    // The decoy lines must not appear at all — this is the assertion a regex implementation fails.
    for decoy in ["decoy.rs:11:", "decoy.rs:14:", "decoy.rs:1:"] {
        assert!(
            !r.stdout.contains(decoy),
            "matched a decoy ({decoy}) — this is text matching, not structural matching:\n{}",
            r.stdout
        );
    }
}

/// US1 scenario 4 / FR-012 — an unsupported language must name what *is* available, never return an
/// empty result that reads as "scanned, found nothing".
///
/// This also guards a real crash: `ast_grep_language::SupportLang` keeps every variant regardless of
/// which grammar features are compiled, and its parser lookup is `unimplemented!()` when the feature
/// is off. bee must therefore refuse from its own registry *before* constructing a `SupportLang` —
/// otherwise this call panics instead of refusing (research R12).
#[test]
fn an_uncompiled_language_refuses_and_names_what_is_available() {
    let r = run(&[
        "astgrep-worker",
        "--lang",
        "cobol",
        "--path",
        &fixture(),
        "--",
        "$X.unwrap()",
    ]);
    assert!(!r.ok, "an unsupported language must not exit 0");
    assert!(
        r.stdout.trim().is_empty(),
        "a refusal must emit no matches: {}",
        r.stdout
    );
    assert!(
        r.stderr.contains("cobol"),
        "the refusal should name the language asked for: {}",
        r.stderr
    );
    assert!(
        r.stderr.contains("rust"),
        "the refusal should name a language that IS compiled in: {}",
        r.stderr
    );
    // Never a panic — `Tool::call` must not panic on bad arguments (FR-016).
    assert!(
        !r.stderr.contains("panicked"),
        "refusal panicked instead of reporting: {}",
        r.stderr
    );
}

/// US1 scenario 3 — a malformed pattern is a diagnostic, not a crash and not an empty result.
#[test]
fn an_invalid_pattern_reports_a_diagnostic() {
    let r = run(&[
        "astgrep-worker",
        "--lang",
        "rust",
        "--path",
        &fixture(),
        "--",
        "fn $(((",
    ]);
    assert!(!r.ok, "an invalid pattern must not exit 0");
    assert!(
        !r.stderr.contains("panicked"),
        "invalid pattern panicked: {}",
        r.stderr
    );
    assert!(
        !r.stderr.trim().is_empty(),
        "an invalid pattern must say what was wrong"
    );
}

/// A pattern that parses but matches nothing is a *clean* result: exit 0, no matches. This is the
/// negative that must stay distinguishable from the refusals above.
#[test]
fn a_pattern_with_no_matches_is_a_clean_result() {
    let r = run(&[
        "astgrep-worker",
        "--lang",
        "rust",
        "--path",
        &fixture(),
        "--",
        "$X.expect($Y)",
    ]);
    assert!(r.ok, "no matches is success, not failure: {}", r.stderr);
    assert!(
        r.stdout.trim().is_empty(),
        "expected no matches, got:\n{}",
        r.stdout
    );
}

/// The result set is bounded, and the bound is announced rather than inferred (FR-013).
#[test]
fn the_match_set_is_bounded() {
    let r = run(&[
        "astgrep-worker",
        "--lang",
        "rust",
        "--path",
        &fixture(),
        "--limit",
        "1",
        "--",
        "$X.unwrap()",
    ]);
    assert!(r.ok, "{}", r.stderr);
    assert_eq!(r.stdout.lines().filter(|l| !l.is_empty()).count(), 1);
}

/// Only files the requested language claims are parsed — a `.txt` beside the fixture must not be fed
/// to the Rust grammar.
#[test]
fn non_matching_extensions_are_skipped() {
    let dir = std::env::temp_dir().join(format!("bee-astgrep-ext-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("note.txt"), "v.unwrap()\n").unwrap();
    std::fs::write(dir.join("real.rs"), "fn f(v: Option<u8>) { v.unwrap(); }\n").unwrap();

    let r = run(&[
        "astgrep-worker",
        "--lang",
        "rust",
        "--path",
        dir.to_str().unwrap(),
        "--",
        "$X.unwrap()",
    ]);
    assert!(r.ok, "{}", r.stderr);
    assert!(r.stdout.contains("real.rs"), "{}", r.stdout);
    assert!(
        !r.stdout.contains("note.txt"),
        "a .txt was parsed as Rust: {}",
        r.stdout
    );
    let _ = std::fs::remove_dir_all(&dir);
}
