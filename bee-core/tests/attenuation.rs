//! Attenuation validator unit tests (T040) — targeted soundness/usability cases.

use bee_core::Policy;

fn p(toml: &str) -> Policy {
    Policy::from_toml(toml).expect("valid policy")
}

#[test]
fn subset_child_is_accepted() {
    let parent = p(r#"
[policy]
name = "parent"
[policy.filesystem]
"/proj" = "write"
[policy.exec]
allow = ["cargo", "rustc"]
[policy.network]
allow = ["crates.io:443"]
"#);
    let child = p(r#"
[policy]
name = "child"
[policy.filesystem]
"/proj/src" = "read"
[policy.exec]
allow = ["cargo"]
"#);
    assert!(parent.derive(child).is_ok());
}

#[test]
fn over_grant_filesystem_rejected() {
    // AS-1: child adds write to a path the parent cannot write.
    let parent = p("[policy]\nname=\"p\"\n[policy.filesystem]\n\"/proj\" = \"write\"\n");
    let child = p("[policy]\nname=\"c\"\n[policy.filesystem]\n\"/etc/passwd\" = \"write\"\n");
    let err = parent.derive(child).unwrap_err();
    assert!(err.capability.contains("/etc/passwd"));
}

#[test]
fn read_only_child_cannot_upgrade_to_write() {
    let parent = p("[policy]\nname=\"p\"\n[policy.filesystem]\n\"/proj\" = \"read\"\n");
    let child = p("[policy]\nname=\"c\"\n[policy.filesystem]\n\"/proj/src\" = \"write\"\n");
    assert!(parent.derive(child).is_err());
}

#[test]
fn child_may_narrow_access() {
    let parent = p("[policy]\nname=\"p\"\n[policy.filesystem]\n\"/proj\" = \"write\"\n");
    let child = p("[policy]\nname=\"c\"\n[policy.filesystem]\n\"/proj/src\" = \"read\"\n");
    assert!(parent.derive(child).is_ok());
}

#[test]
fn parent_deny_inside_grant_must_be_replicated() {
    // Parent grants /proj=write but denies /proj/.git. A child re-granting /proj write without
    // replicating the deny would over-grant (could write .git) -> rejected.
    let parent = p(r#"
[policy]
name = "p"
[policy.filesystem]
"/proj" = "write"
"/proj/.git" = "deny"
"#);
    let child_bad = p("[policy]\nname=\"c\"\n[policy.filesystem]\n\"/proj\" = \"write\"\n");
    assert!(parent.derive(child_bad).is_err());

    let child_ok = p(r#"
[policy]
name = "c"
[policy.filesystem]
"/proj" = "write"
"/proj/.git" = "deny"
"#);
    assert!(parent.derive(child_ok).is_ok());
}

#[test]
fn exec_not_in_parent_rejected() {
    let parent = p("[policy]\nname=\"p\"\n[policy.exec]\nallow=[\"cargo\"]\n");
    let child = p("[policy]\nname=\"c\"\n[policy.exec]\nallow=[\"curl\"]\n");
    let err = parent.derive(child).unwrap_err();
    assert!(err.capability.contains("curl"));
}

#[test]
fn network_not_in_parent_rejected() {
    let parent = p("[policy]\nname=\"p\"\n[policy.network]\nallow=[\"crates.io:443\"]\n");
    let child = p("[policy]\nname=\"c\"\n[policy.network]\nallow=[\"evil.com:443\"]\n");
    assert!(parent.derive(child).is_err());
}

#[test]
fn no_network_child_is_subset() {
    // AS-3: a child with no network is trivially within a parent that has network.
    let parent = p("[policy]\nname=\"p\"\n[policy.network]\nallow=[\"crates.io:443\"]\n");
    let child = p("[policy]\nname=\"c\"\n");
    assert!(parent.derive(child).is_ok());
}

#[test]
fn child_may_not_be_looser_mode() {
    let parent = p("[policy]\nname=\"p\"\nmode=\"enforce\"\n");
    let child = p("[policy]\nname=\"c\"\nmode=\"observe\"\n");
    assert!(parent.derive(child).is_err(), "observe child under enforce parent is looser");

    let parent2 = p("[policy]\nname=\"p\"\nmode=\"observe\"\n");
    let child2 = p("[policy]\nname=\"c\"\nmode=\"enforce\"\n");
    assert!(parent2.derive(child2).is_ok(), "enforce child under observe parent is stricter");
}

#[test]
fn identical_glob_grant_is_allowed() {
    let parent = p("[policy]\nname=\"p\"\n[policy.filesystem]\n\"*.log\" = \"write\"\n");
    let child = p("[policy]\nname=\"c\"\n[policy.filesystem]\n\"*.log\" = \"read\"\n");
    assert!(parent.derive(child).is_ok());
}

#[test]
fn novel_glob_grant_rejected() {
    let parent = p("[policy]\nname=\"p\"\n[policy.filesystem]\n\"/proj\" = \"write\"\n");
    let child = p("[policy]\nname=\"c\"\n[policy.filesystem]\n\"*.log\" = \"write\"\n");
    assert!(parent.derive(child).is_err(), "glob not provably contained in a prefix grant");
}
