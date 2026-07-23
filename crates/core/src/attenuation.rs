//! Attenuation: prove a derived policy is a subset of its parent (FR-005, SC-002).
//!
//! The validator is **sound and conservative** (constitution Principle II): when subset containment
//! cannot be proven it rejects (fail-closed), even at the cost of rejecting some children that would
//! in fact be safe. Reasoning is at the authoring level over raw pattern strings — parent and child
//! share the same token vocabulary (`:project_root`, `~`), so string-level subtree reasoning is
//! valid without resolving to the host environment.

use std::collections::BTreeMap;

use crate::compiler::{lower_pattern, FsPrimitive};
use crate::error::AttenuationError;
use crate::policy::{Access, Policy};
use bee_common::AccessMode;

/// A pattern's region for containment reasoning.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Region {
    /// A concrete path (exact/subtree).
    Prefix(Vec<u8>),
    /// A glob, identified by a canonical (kind, bytes) key. Two globs are "the same region" iff equal.
    Glob(u8, Vec<u8>),
}

fn region_of(raw: &str) -> Option<Region> {
    match lower_pattern(raw, AccessMode::READ).ok()? {
        FsPrimitive::Prefix { path, .. } => Some(Region::Prefix(path)),
        FsPrimitive::Postfix { suffix, .. } => Some(Region::Glob(0, suffix)),
        FsPrimitive::Segment { name, .. } => Some(Region::Glob(1, name)),
        FsPrimitive::BoundedStar { pattern, .. } => Some(Region::Glob(2, pattern)),
    }
}

/// True if prefix `outer` contains `inner` (inner == outer, or inner lies beneath outer).
fn prefix_contains(outer: &[u8], inner: &[u8]) -> bool {
    if inner == outer {
        return true;
    }
    inner.len() > outer.len() && inner.starts_with(outer) && inner[outer.len()] == b'/'
}

impl Policy {
    /// Derive a validated child policy that is provably ⊆ `self`. Returns the request on success,
    /// or the first [`AttenuationError`] found.
    pub fn derive(&self, request: Policy) -> Result<Policy, AttenuationError> {
        // Mode may only stay the same or become stricter (parent Enforce ⇒ child Enforce).
        if request.mode.strictness() < self.mode.strictness() {
            return Err(AttenuationError::new(
                "mode",
                "child mode is looser than parent (parent enforces; child must enforce)",
            ));
        }

        self.check_filesystem(&request.filesystem)?;
        self.check_exec(&request)?;
        self.check_network(&request)?;
        // Exfiltration: a child may only *add* sensitive paths / enable detection (narrowing) — no check.

        Ok(request)
    }

    fn check_filesystem(&self, child: &BTreeMap<String, Access>) -> Result<(), AttenuationError> {
        for (raw, &access) in child {
            if access == Access::Deny {
                continue; // adding a denial always narrows.
            }
            let cap = format!("filesystem:{raw}");
            let region = region_of(raw)
                .ok_or_else(|| AttenuationError::new(&cap, "child pattern is not a valid rule"))?;

            match region {
                Region::Prefix(pc) => self.check_prefix_grant(&cap, &pc, access, child)?,
                Region::Glob(kind, bytes) => {
                    // Conservative: a glob grant is permitted only if the parent has an identical
                    // glob rule granting at least this access.
                    let ok = self.filesystem.iter().any(|(pr, &pa)| {
                        pa != Access::Deny
                            && access.is_subset_of(pa)
                            && region_of(pr) == Some(Region::Glob(kind, bytes.clone()))
                    });
                    if !ok {
                        return Err(AttenuationError::new(
                            &cap,
                            "glob grant not provably contained in parent (needs an identical parent glob rule)",
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    /// Sound check for a concrete-path child grant.
    fn check_prefix_grant(
        &self,
        cap: &str,
        pc: &[u8],
        access: Access,
        child: &BTreeMap<String, Access>,
    ) -> Result<(), AttenuationError> {
        // 1. Parent's effective access AT pc = the most specific parent rule whose region ⊇ pc.
        let mut eff: Option<(Access, usize)> = None;
        for (pr, &pa) in &self.filesystem {
            if let Some(Region::Prefix(pp)) = region_of(pr) {
                if prefix_contains(&pp, pc) && pp.len() >= eff.map_or(0, |(_, l)| l) {
                    eff = Some((pa, pp.len()));
                }
            }
        }
        let parent_access = match eff {
            None => {
                return Err(AttenuationError::new(
                    cap,
                    "no parent rule covers this path",
                ));
            }
            Some((Access::Deny, _)) => {
                return Err(AttenuationError::new(cap, "parent denies this path"));
            }
            Some((a, _)) => a,
        };
        if !access.is_subset_of(parent_access) {
            return Err(AttenuationError::new(
                cap,
                "requested access exceeds the parent's access for this path",
            ));
        }

        // 2. Any parent rule that REDUCES access *inside* pc must be replicated by the child.
        for (pr, &pa) in &self.filesystem {
            let reduces = !access.is_subset_of(pa); // deny, or a lesser grant
            if !reduces {
                continue;
            }
            match region_of(pr) {
                Some(Region::Prefix(pd)) if prefix_contains(pc, &pd) && pd.as_slice() != pc => {
                    if !child_replicates_prefix(child, &pd, pa) {
                        return Err(AttenuationError::new(
                            cap,
                            "parent restricts a path inside this grant; child must replicate that restriction",
                        ));
                    }
                }
                // A parent glob restriction could intersect pc; require the child replicates it.
                Some(Region::Glob(kind, bytes))
                    if !child_replicates_glob(child, kind, &bytes, pa) =>
                {
                    return Err(AttenuationError::new(
                        cap,
                        "parent has a glob restriction that may intersect this grant; child must replicate it",
                    ));
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn check_exec(&self, request: &Policy) -> Result<(), AttenuationError> {
        for entry in &request.exec.allow {
            let name = entry.strip_prefix('!').unwrap_or(entry);
            let ok = self
                .exec
                .allow
                .iter()
                .any(|p| p.strip_prefix('!').unwrap_or(p) == name);
            if !ok {
                return Err(AttenuationError::new(
                    format!("exec:{name}"),
                    "executable not in parent's allowlist",
                ));
            }
        }
        Ok(())
    }

    fn check_network(&self, request: &Policy) -> Result<(), AttenuationError> {
        for entry in &request.network.allow {
            if !self.network.allow.iter().any(|p| p == entry) {
                return Err(AttenuationError::new(
                    format!("network:{entry}"),
                    "destination not in parent's allowlist",
                ));
            }
        }
        Ok(())
    }
}

fn child_replicates_prefix(
    child: &BTreeMap<String, Access>,
    pd: &[u8],
    parent_access: Access,
) -> bool {
    child.iter().any(|(cr, &ca)| {
        ca.is_subset_of(parent_access)
            && matches!(region_of(cr), Some(Region::Prefix(cp)) if prefix_contains(&cp, pd))
    })
}

fn child_replicates_glob(
    child: &BTreeMap<String, Access>,
    kind: u8,
    bytes: &[u8],
    parent_access: Access,
) -> bool {
    child.iter().any(|(cr, &ca)| {
        ca.is_subset_of(parent_access) && region_of(cr) == Some(Region::Glob(kind, bytes.to_vec()))
    })
}
