//! A tool child that backgrounds a redirected descendant must not leave it running (f022).
//!
//! The direct child is a shell that exits immediately; the descendant it daemonizes is what the
//! launcher never had a handle on. Under `enforce` such a survivor keeps the scope cgroup populated
//! and outlives the LSM programs, which is a sandbox escape rather than a leak — the host-mode case
//! tested here exercises the same cleanup path (the tool child's process group).

use bee::sandbox::Sandbox;
use bee::tools::exec::run_child;

/// A marker file the descendant creates only *after* it would have outlived the tool call.
#[tokio::test]
async fn backgrounded_descendant_does_not_survive_the_tool_call() {
    let dir = std::env::temp_dir().join(format!("bee-descendant-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let marker = dir.join("survived");

    let sandbox = Sandbox::host(Vec::new());
    // Background a sleeper that writes the marker when it wakes, with stdio redirected so the
    // shell's own pipes close and `wait_with_output` returns straight away.
    let script = format!(
        "(sleep 2; touch {}) >/dev/null 2>&1 & echo started",
        marker.display()
    );
    let result = run_child(&sandbox, "sh", &["-c".to_string(), script], None, 4096).await;
    assert!(
        result.content.contains("started"),
        "the shell should have run: {}",
        result.content
    );

    // Well past the descendant's sleep: if the process group was reaped, the marker never appears.
    tokio::time::sleep(std::time::Duration::from_secs(4)).await;
    assert!(
        !marker.exists(),
        "a backgrounded descendant outlived the tool call"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
