//! Property-based attenuation tests (T039 / SC-002): a derived policy can never exceed its parent.
//!
//! We assert both directions:
//! * **Completeness** — a child built as a construction-guaranteed subset is accepted.
//! * **Soundness** — a child that adds any capability the parent lacks is rejected.

use std::collections::BTreeMap;

use bee_core::policy::{Access, ExecPolicy, ExfilPolicy, Mode, NetPolicy};
use bee_core::Policy;
use proptest::prelude::*;

fn mk(
    mode: Mode,
    fs: BTreeMap<String, Access>,
    exec: Vec<String>,
    net: Vec<String>,
) -> Policy {
    Policy {
        name: "p".into(),
        description: None,
        mode,
        filesystem: fs,
        exec: ExecPolicy { allow: exec },
        network: NetPolicy { allow: net },
        exfiltration: ExfilPolicy::default(),
    }
}

// (id, is_write, keep_in_child)
fn specs() -> impl Strategy<Value = Vec<(u32, bool, bool)>> {
    prop::collection::vec((0u32..20, any::<bool>(), any::<bool>()), 0..8)
}

proptest! {
    #[test]
    fn subset_by_construction_is_accepted(s in specs()) {
        let mut parent_fs = BTreeMap::new();
        let mut child_fs = BTreeMap::new();
        for (id, is_write, keep) in &s {
            let path = format!("/proj/d{id}");
            parent_fs.insert(path.clone(), if *is_write { Access::Write } else { Access::Read });
            if *keep {
                // Read is a subset of both Read and Write, so this is always within parent.
                child_fs.insert(path, Access::Read);
            }
        }
        let parent = mk(Mode::Enforce, parent_fs, vec!["cargo".into(), "rustc".into()], vec!["crates.io:443".into()]);
        let child = mk(Mode::Enforce, child_fs, vec!["cargo".into()], vec![]);
        prop_assert!(parent.derive(child).is_ok(), "constructed subset must be accepted");
    }

    #[test]
    fn adding_outside_filesystem_grant_is_rejected(s in specs(), outside in 0u32..50) {
        let mut parent_fs = BTreeMap::new();
        for (id, is_write, _) in &s {
            parent_fs.insert(format!("/proj/d{id}"), if *is_write { Access::Write } else { Access::Read });
        }
        // Child = an exact copy of the parent's grants (⊆) PLUS one grant outside any parent region.
        let mut child_fs = parent_fs.clone();
        child_fs.insert(format!("/outside/x{outside}"), Access::Write);
        let parent = mk(Mode::Enforce, parent_fs, vec![], vec![]);
        let child = mk(Mode::Enforce, child_fs, vec![], vec![]);
        prop_assert!(parent.derive(child).is_err(), "a grant outside every parent region must be rejected");
    }

    #[test]
    fn adding_exec_not_in_parent_is_rejected(s in specs()) {
        let mut parent_fs = BTreeMap::new();
        for (id, is_write, _) in &s {
            parent_fs.insert(format!("/proj/d{id}"), if *is_write { Access::Write } else { Access::Read });
        }
        let parent = mk(Mode::Enforce, parent_fs.clone(), vec!["cargo".into()], vec![]);
        // "curl" is never in the parent's allowlist.
        let child = mk(Mode::Enforce, parent_fs, vec!["cargo".into(), "curl".into()], vec![]);
        prop_assert!(parent.derive(child).is_err());
    }
}
