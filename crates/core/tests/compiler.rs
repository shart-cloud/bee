//! Compiler integration tests (T021, T022) + FR-008 default-protection coverage (analyze finding G1).

use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;

use bee_common::AccessMode;
use bee_core::{CompileError, FsPrimitive, Policy, Resolver};

struct Mock;

impl Resolver for Mock {
    fn project_root(&self) -> &str {
        "/proj"
    }
    fn home(&self) -> &str {
        "/home/u"
    }
    fn resolve_exec(&self, name: &str) -> Result<PathBuf, CompileError> {
        if name == "missing" {
            return Err(CompileError::UnresolvableExec(
                name.into(),
                "not found on PATH".into(),
            ));
        }
        if name.starts_with('/') {
            Ok(PathBuf::from(name))
        } else {
            Ok(PathBuf::from(format!("/usr/bin/{name}")))
        }
    }
    fn resolve_host(&self, host: &str) -> Result<Vec<IpAddr>, CompileError> {
        if host == "bad" {
            return Err(CompileError::UnresolvableHost(
                host.into(),
                "nxdomain".into(),
            ));
        }
        Ok(vec![IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34))])
    }
}

fn has_prefix(set: &[FsPrimitive], path: &[u8], mode: AccessMode) -> bool {
    set.iter().any(
        |p| matches!(p, FsPrimitive::Prefix { path: pp, mode: m, .. } if pp == path && *m == mode),
    )
}

#[test]
fn lowers_globs_and_paths() {
    let toml = r#"
[policy]
name = "t"
[policy.filesystem]
":project_root" = "write"
"*.log" = "write"
"**/target" = "write"
"~/.cargo/registry" = "read"
"#;
    let set = Policy::from_toml(toml).unwrap().compile(&Mock).unwrap();

    assert!(has_prefix(
        &set.fs,
        b"/proj",
        AccessMode::WRITE.union(AccessMode::READ)
    ));
    assert!(has_prefix(
        &set.fs,
        b"/home/u/.cargo/registry",
        AccessMode::READ
    ));
    assert!(set
        .fs
        .iter()
        .any(|p| matches!(p, FsPrimitive::Postfix { suffix, .. } if suffix == b".log")));
    assert!(set
        .fs
        .iter()
        .any(|p| matches!(p, FsPrimitive::Segment { name, .. } if name == b"target")));
}

#[test]
fn injects_protected_defaults_when_not_overridden() {
    // FR-008 / G1: a policy that grants a writable root but never mentions ~/.ssh still denies it.
    let toml = r#"
[policy]
name = "t"
[policy.filesystem]
":project_root" = "write"
"#;
    let set = Policy::from_toml(toml).unwrap().compile(&Mock).unwrap();
    assert!(
        has_prefix(&set.fs, b"/home/u/.ssh", AccessMode::DENY),
        "~/.ssh must be denied by default"
    );
    assert!(
        has_prefix(&set.fs, b"/home/u/.aws", AccessMode::DENY),
        "~/.aws must be denied by default"
    );
    assert!(
        has_prefix(&set.fs, b"/proj/.bee", AccessMode::DENY),
        ".bee must be denied by default"
    );
    assert!(
        has_prefix(&set.fs, b"/proj/.git", AccessMode::READ),
        ".git must be read-only by default"
    );
}

#[test]
fn unsupported_glob_fails_closed() {
    let toml = r#"
[policy]
name = "t"
[policy.filesystem]
"/a/*/b" = "read"
"#;
    let err = Policy::from_toml(toml).unwrap().compile(&Mock).unwrap_err();
    assert!(matches!(err, CompileError::UnsupportedGlob(..)));
}

#[test]
fn resolves_exec_names_and_pins() {
    let toml = r#"
[policy]
name = "t"
[policy.exec]
allow = ["cargo", "!rustc", "/opt/tool"]
"#;
    let set = Policy::from_toml(toml).unwrap().compile(&Mock).unwrap();
    let cargo = set
        .exec
        .iter()
        .find(|e| e.path == b"/usr/bin/cargo")
        .unwrap();
    assert!(!cargo.pin_inode);
    let rustc = set
        .exec
        .iter()
        .find(|e| e.path == b"/usr/bin/rustc")
        .unwrap();
    assert!(rustc.pin_inode, "leading ! pins the inode");
    assert!(set.exec.iter().any(|e| e.path == b"/opt/tool"));
}

#[test]
fn unresolvable_exec_fails() {
    let toml = "[policy]\nname=\"t\"\n[policy.exec]\nallow=[\"missing\"]\n";
    let err = Policy::from_toml(toml).unwrap().compile(&Mock).unwrap_err();
    assert!(matches!(err, CompileError::UnresolvableExec(..)));
}

#[test]
fn resolves_network_and_rejects_bad() {
    let toml = "[policy]\nname=\"t\"\n[policy.network]\nallow=[\"crates.io:443\"]\n";
    let set = Policy::from_toml(toml).unwrap().compile(&Mock).unwrap();
    assert_eq!(set.net.len(), 1);
    assert_eq!(set.net[0].port, 443);

    let bad_host = "[policy]\nname=\"t\"\n[policy.network]\nallow=[\"bad:443\"]\n";
    assert!(matches!(
        Policy::from_toml(bad_host).unwrap().compile(&Mock),
        Err(CompileError::UnresolvableHost(..))
    ));

    // Malformed rule is caught at parse-time validation (needs a colon).
    let bad_rule = "[policy]\nname=\"t\"\n[policy.network]\nallow=[\"noport\"]\n";
    assert!(Policy::from_toml(bad_rule).is_err());
}

#[test]
fn preserves_exfiltration_intent_for_backend_planning() {
    let enabled = r#"
[policy]
name = "t"
[policy.exfiltration]
enabled = true
sensitive_paths = ["~/.ssh"]
"#;
    let compiled = Policy::from_toml(enabled).unwrap().compile(&Mock).unwrap();
    assert!(compiled.exfiltration_enabled);
    assert!(has_prefix(
        &compiled.sensitive,
        b"/home/u/.ssh",
        AccessMode::READ
    ));

    let disabled = r#"
[policy]
name = "t"
[policy.exfiltration]
enabled = false
sensitive_paths = ["~/.ssh"]
"#;
    let compiled = Policy::from_toml(disabled).unwrap().compile(&Mock).unwrap();
    assert!(!compiled.exfiltration_enabled);
    assert!(has_prefix(
        &compiled.sensitive,
        b"/home/u/.ssh",
        AccessMode::READ
    ));
}

#[test]
fn compilation_does_not_apply_backend_encoded_length_limits() {
    let path = format!("/{}", "x".repeat(300));
    let toml = format!("[policy]\nname = \"t\"\n[policy.filesystem]\n\"{path}\" = \"read\"\n");
    let compiled = Policy::from_toml(&toml).unwrap().compile(&Mock).unwrap();
    assert!(compiled
        .fs
        .iter()
        .any(|rule| matches!(rule, FsPrimitive::Prefix { path: p, .. } if p.len() == path.len())));
}
