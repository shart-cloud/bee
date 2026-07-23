use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn policy(name: &str, body: &str) -> PathBuf {
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("bee-validate-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{name}.toml"));
    std::fs::write(&path, format!("[policy]\nname = \"{name}\"\n{body}")).unwrap();
    path
}

fn validate(path: &PathBuf) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_bee"))
        .args(["validate", "--policy"])
        .arg(path)
        .output()
        .unwrap()
}

#[test]
fn supported_policy_is_runnable() {
    let path = policy("supported", "[policy.filesystem]\n\"/tmp\" = \"write\"\n");
    let output = validate(&path);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("valid and runnable"));
}

#[test]
fn unsupported_segment_is_rejected_before_kernel_probe() {
    let path = policy("segment", "[policy.filesystem]\n\"**/target\" = \"deny\"\n");
    let output = validate(&path);
    assert_eq!(output.status.code(), Some(64));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("enforcement plan error"), "{stderr}");
    assert!(stderr.contains("segment"), "{stderr}");
    assert!(!stderr.contains("kernel"), "{stderr}");
}

#[test]
fn inode_pin_and_enabled_exfiltration_are_rejected() {
    let pinned = policy("pinned", "[policy.exec]\nallow = [\"!sh\"]\n");
    let output = validate(&pinned);
    assert_eq!(output.status.code(), Some(64));
    assert!(String::from_utf8_lossy(&output.stderr).contains("inode-pinned"));

    let exfil = policy("exfil", "[policy.exfiltration]\nenabled = true\n");
    let output = validate(&exfil);
    assert_eq!(output.status.code(), Some(64));
    assert!(String::from_utf8_lossy(&output.stderr).contains("exfiltration"));
}

#[test]
fn attenuated_child_must_also_be_runnable() {
    let parent = policy("parent", "[policy.filesystem]\n\"**/target\" = \"deny\"\n");
    let child = policy("child", "[policy.filesystem]\n\"**/target\" = \"deny\"\n");
    let output = Command::new(env!("CARGO_BIN_EXE_bee"))
        .args(["validate", "--policy"])
        .arg(&child)
        .arg("--parent")
        .arg(&parent)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(64));
    assert!(String::from_utf8_lossy(&output.stderr).contains("segment"));
}
