//! `bee run` must be indistinguishable from `bee-episode` (consolidation issue 03).
//!
//! The consolidation moves a journey, not its behaviour. These drive identical inputs through both
//! and compare what a caller can actually observe: the transcript JSON on stdout, the exit code,
//! and that progress stayed on stderr where a pipe cannot see it. The `mock` provider makes every
//! case deterministic and offline.
//!
//! Timing fields differ between runs by construction, so they are stripped before comparison —
//! everything else must match exactly. When issue 06 deletes `bee-episode`, these become
//! single-command tests of the surviving behaviour.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// The `bee-episode` binary, built from the harness package. `None` once issue 06 removes it, at
/// which point these tests are retired rather than skipped.
fn episode_bin() -> Option<PathBuf> {
    let candidate = Path::new(env!("CARGO_BIN_EXE_bee"))
        .parent()?
        .join("bee-episode");
    candidate.exists().then_some(candidate)
}

fn write(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, body).unwrap();
    path
}

/// A scripted provider: one text turn, no network, no key.
fn mock_provider(dir: &Path, name: &str, reply: &str) -> PathBuf {
    write(
        dir,
        name,
        &format!("[provider]\nprovider = \"mock\"\n\n[[provider.script]]\ntext = \"{reply}\"\n"),
    )
}

fn scenario(dir: &Path, name: &str, id: &str, task: &str) -> PathBuf {
    write(
        dir,
        name,
        &format!(
            "[scenario]\n\
             id = \"{id}\"\n\
             policy_path = \"/dev/null\"\n\
             system_prompt = \"You are a test agent.\"\n\
             task = \"{task}\"\n\
             turn_limit = 2\n\
             timeout_secs = 10\n\
             tools = []\n"
        ),
    )
}

/// Drop the fields that cannot match across two runs: wall-clock stamps and durations.
fn normalize(json: &str) -> String {
    json.lines()
        .filter(|l| {
            let t = l.trim_start();
            !(t.starts_with("\"started_at\"")
                || t.starts_with("\"ended_at\"")
                || t.starts_with("\"total_ms\"")
                || t.starts_with("\"duration_ms\""))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn bee_run(args: &[&str]) -> Output {
    let mut a = vec!["run"];
    a.extend_from_slice(args);
    Command::new(env!("CARGO_BIN_EXE_bee"))
        .args(&a)
        .output()
        .unwrap()
}

fn bee_episode(args: &[&str]) -> Option<Output> {
    let Some(bin) = episode_bin() else {
        // Cargo builds only the binaries of the package under test, so `cargo test -p bee-cli`
        // alone leaves nothing to compare against. Say so loudly: a parity test that quietly
        // compares against nothing is worse than one that fails.
        eprintln!(
            "[skip] parity comparison: no `bee-episode` beside the test binary. \
             Run `cargo build --workspace` first to compare against it."
        );
        return None;
    };
    Some(Command::new(bin).args(args).output().unwrap())
}

/// Run the same arguments through both and assert they agree on everything observable.
fn assert_parity(args: &[&str]) -> Output {
    let mine = bee_run(args);
    let Some(theirs) = bee_episode(args) else {
        // `bee-episode` is gone (issue 06). The single-command assertions in each test still run.
        return mine;
    };
    assert_eq!(
        mine.status.code(),
        theirs.status.code(),
        "exit codes differ for {args:?}\nbee run stderr: {}\nbee-episode stderr: {}",
        String::from_utf8_lossy(&mine.stderr),
        String::from_utf8_lossy(&theirs.stderr)
    );
    assert_eq!(
        normalize(&String::from_utf8_lossy(&mine.stdout)),
        normalize(&String::from_utf8_lossy(&theirs.stdout)),
        "stdout differs for {args:?}"
    );
    mine
}

#[test]
fn a_single_episode_matches() {
    let dir = tempfile::tempdir().unwrap();
    let provider = mock_provider(dir.path(), "p.toml", "Done.");
    let scenario = scenario(dir.path(), "s.toml", "parity", "do the thing");

    let out = assert_parity(&[
        "--scenario",
        scenario.to_str().unwrap(),
        "--provider",
        provider.to_str().unwrap(),
        "--quiet",
    ]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("\"scenario_id\": \"parity\""), "{stdout}");
}

#[test]
fn an_ad_hoc_task_matches() {
    let dir = tempfile::tempdir().unwrap();
    let provider = mock_provider(dir.path(), "p.toml", "Nothing to do.");

    let out = assert_parity(&[
        "--task",
        "say hello",
        "--provider",
        provider.to_str().unwrap(),
        "--quiet",
    ]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("\"scenario_id\": \"adhoc\""));
}

#[test]
fn a_batch_matches() {
    let dir = tempfile::tempdir().unwrap();
    let s1 = scenario(dir.path(), "scenarios/a.toml", "a", "first");
    let s2 = scenario(dir.path(), "scenarios/b.toml", "b", "second");
    let p1 = mock_provider(dir.path(), "providers/one.toml", "One.");
    let p2 = mock_provider(dir.path(), "providers/two.toml", "Two.");

    let scenarios = format!("{},{}", s1.to_str().unwrap(), s2.to_str().unwrap());
    let providers = format!("{},{}", p1.to_str().unwrap(), p2.to_str().unwrap());

    let out = assert_parity(&[
        "--batch",
        "--scenarios",
        &scenarios,
        "--providers",
        &providers,
        "--quiet",
    ]);
    assert!(out.status.success());
    // Two scenarios × two providers.
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(stdout.matches("\"scenario_id\"").count(), 4, "{stdout}");
}

#[test]
fn a_missing_provider_file_fails_the_same_way() {
    let dir = tempfile::tempdir().unwrap();
    let scenario = scenario(dir.path(), "s.toml", "x", "task");
    let missing = dir.path().join("nope.toml");

    let out = bee_run(&[
        "--scenario",
        scenario.to_str().unwrap(),
        "--provider",
        missing.to_str().unwrap(),
        "--quiet",
    ]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("nope.toml"), "{stderr}");

    // `bee-episode` reports the same failure; the exit code is what scripts branch on.
    if let Some(theirs) = bee_episode(&[
        "--scenario",
        scenario.to_str().unwrap(),
        "--provider",
        missing.to_str().unwrap(),
        "--quiet",
    ]) {
        assert_eq!(out.status.code(), theirs.status.code());
    }
}

#[test]
fn an_unparseable_scenario_fails_the_same_way() {
    let dir = tempfile::tempdir().unwrap();
    let provider = mock_provider(dir.path(), "p.toml", "hi");
    let broken = write(
        dir.path(),
        "broken.toml",
        "id = \"x\"\nthis is not toml [[[\n",
    );

    let out = assert_parity(&[
        "--scenario",
        broken.to_str().unwrap(),
        "--provider",
        provider.to_str().unwrap(),
        "--quiet",
    ]);
    assert_eq!(out.status.code(), Some(64), "usage error");
    assert!(String::from_utf8_lossy(&out.stderr).contains("scenario"));
}

#[test]
fn the_transcript_stays_out_of_the_progress_stream() {
    // The artifact contract: stdout is parseable JSON even with progress on, because progress goes
    // to stderr. This is what makes `bee run ... > out.json` work.
    let dir = tempfile::tempdir().unwrap();
    let provider = mock_provider(dir.path(), "p.toml", "Done.");
    let scenario = scenario(dir.path(), "s.toml", "piped", "task");

    let out = bee_run(&[
        "--scenario",
        scenario.to_str().unwrap(),
        "--provider",
        provider.to_str().unwrap(),
    ]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    serde_json::from_str::<serde_json::Value>(&stdout)
        .unwrap_or_else(|e| panic!("stdout is not valid JSON: {e}\n{stdout}"));
}

#[test]
fn out_writes_the_transcript_to_a_file() {
    let dir = tempfile::tempdir().unwrap();
    let provider = mock_provider(dir.path(), "p.toml", "Done.");
    let scenario = scenario(dir.path(), "s.toml", "tofile", "task");
    let out_path = dir.path().join("transcript.json");

    let out = bee_run(&[
        "--scenario",
        scenario.to_str().unwrap(),
        "--provider",
        provider.to_str().unwrap(),
        "--out",
        out_path.to_str().unwrap(),
        "--quiet",
    ]);
    assert!(out.status.success());
    assert!(out.stdout.is_empty(), "nothing goes to stdout with --out");
    let written = std::fs::read_to_string(&out_path).unwrap();
    assert!(written.contains("\"scenario_id\": \"tofile\""), "{written}");
}

#[test]
fn concurrent_without_the_feature_is_an_explicit_refusal() {
    // Never a silent fall back to sequential: the operator asked for concurrency and would
    // otherwise believe they got it.
    if cfg!(feature = "concurrent") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let s = scenario(dir.path(), "s.toml", "x", "task");
    let p = mock_provider(dir.path(), "p.toml", "hi");

    let out = bee_run(&[
        "--batch",
        "--concurrent",
        "--scenarios",
        s.to_str().unwrap(),
        "--providers",
        p.to_str().unwrap(),
        "--quiet",
    ]);
    assert_eq!(out.status.code(), Some(64));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("--features concurrent"), "{stderr}");
}
