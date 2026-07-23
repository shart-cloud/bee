//! `bee-userspace` — kernel-facing loader, scope lifecycle, and process launcher.
//!
//! ## Build modes
//!
//! * **default (stable toolchain)** — host-verifiable logic only: support detection ([`detect`]),
//!   cgroup id/lifecycle ([`cgroup`]), the hardened launcher ([`spawn`]). [`Engine::init`] runs the
//!   fail-closed support gate but returns [`EngineError::EnforceFeatureDisabled`], since no BPF
//!   object is embedded.
//! * **`--features enforce` (nightly bpf toolchain + BPF-LSM kernel)** — additionally compiles and
//!   embeds the eBPF programs, attaches the LSM hooks ([`loader`]), populates per-scope maps, and
//!   streams audit events ([`events`]).

pub mod cgroup;
pub mod detect;
pub mod events;
// Pre-exec / pre-main process hardening (FR-011). Was its own package until consolidation issue
// 07: one source file, one consumer (`spawn`), and none of the runtime, kernel, `no_std`, or
// BPF-target constraints that earn the other crates their separation.
pub mod hardening;
pub mod kbtf;
pub mod loader;
pub mod plan;
pub mod resolve;
pub mod spawn;

#[cfg(feature = "tokio")]
pub mod async_events;
#[cfg(feature = "tokio")]
pub mod audit_demux;

pub use bee_common::ScopeMode;
pub use detect::{detect, Support};
pub use plan::{EnforcementPlan, PlanError};
pub use resolve::SystemResolver;
pub use spawn::{hardened_command, SpawnError};

#[cfg(feature = "tokio")]
pub use async_events::AsyncAuditStream;
#[cfg(feature = "tokio")]
pub use audit_demux::{AuditDemux, AuditSubscription};

use thiserror::Error;

/// Errors from bringing up the enforcement engine.
#[derive(Debug, Error)]
pub enum EngineError {
    /// The kernel cannot enforce (missing BPF LSM / cgroup v2). Fail-closed (FR-009 / SC-007).
    #[error("bee cannot enforce on this kernel:\n{0}")]
    Unsupported(String),
    /// Built without the `enforce` feature, so no BPF object is embedded to attach.
    #[error(
        "bee-userspace was built without the `enforce` feature; rebuild with `--features enforce` \
         on a BPF-LSM-capable kernel and the nightly bpf toolchain to attach enforcement"
    )]
    EnforceFeatureDisabled,
    /// Loading or attaching the eBPF object failed.
    #[error("eBPF load/attach failed: {0}")]
    Load(String),
}

/// Errors from creating or managing a scope.
#[derive(Debug, Error)]
pub enum ScopeError {
    #[error("cgroup error: {0}")]
    Cgroup(#[from] std::io::Error),
    #[error("map error: {0}")]
    Map(String),
}

/// The process-wide enforcement engine.
pub struct Engine {
    // Read via `support()` and by the enforce-path loader; unused in the default build.
    #[allow(dead_code)]
    support: Support,
    #[cfg(feature = "enforce")]
    ebpf: aya::Ebpf,
}

impl Engine {
    /// Probe kernel support without attaching anything (backs `bee check`).
    pub fn supported() -> Support {
        detect::detect()
    }

    /// Initialize the engine. **Fail-closed**: returns `Err(Unsupported)` on any kernel that cannot
    /// enforce, never a no-op engine (SC-007).
    pub fn init() -> Result<Engine, EngineError> {
        let support = Self::supported();
        if !support.is_supported() {
            return Err(EngineError::Unsupported(support.to_string()));
        }

        #[cfg(feature = "enforce")]
        {
            let ebpf = loader::load_and_attach()?;
            Ok(Engine { support, ebpf })
        }
        #[cfg(not(feature = "enforce"))]
        {
            Err(EngineError::EnforceFeatureDisabled)
        }
    }

    /// The support probe this engine was initialized against.
    pub fn support(&self) -> &Support {
        &self.support
    }

    /// Install a completed enforcement plan into a new cgroup and the eBPF maps.
    #[cfg(feature = "enforce")]
    pub fn create_scope(
        &mut self,
        scope_id: &str,
        parent_cgroup: &str,
        plan: &EnforcementPlan,
    ) -> Result<Scope, ScopeError> {
        let path = cgroup::create_scope_cgroup(parent_cgroup, scope_id)?;
        let cgroup_id = cgroup::cgroup_id(&path)?;

        // Planning has already validated all capabilities and encoded the mode/flags.
        {
            let map = self
                .ebpf
                .map_mut("SCOPES")
                .ok_or_else(|| ScopeError::Map("SCOPES map missing".into()))?;
            let mut scopes: aya::maps::HashMap<_, u64, bee_common::layout::ScopeMeta> =
                aya::maps::HashMap::try_from(map).map_err(|e| ScopeError::Map(e.to_string()))?;
            scopes
                .insert(cgroup_id, plan.meta, 0)
                .map_err(|e| ScopeError::Map(e.to_string()))?;
        }

        // Populate the network egress allowlist, tracking the installed keys on the Scope so a later
        // narrowing reload (007-dynamic-grants) can remove the ones that get dropped.
        let mut net_keys: Vec<bee_common::layout::NetKey> = Vec::new();
        if !plan.net_rules.is_empty() {
            use bee_common::layout::NetKey;
            let map = self
                .ebpf
                .map_mut("NET_ALLOW")
                .ok_or_else(|| ScopeError::Map("NET_ALLOW map missing".into()))?;
            let mut net: aya::maps::HashMap<_, NetKey, u8> =
                aya::maps::HashMap::try_from(map).map_err(|e| ScopeError::Map(e.to_string()))?;
            for template in &plan.net_rules {
                let mut key = *template;
                key.cgroup_id = cgroup_id;
                net.insert(key, 1u8, 0)
                    .map_err(|e| ScopeError::Map(e.to_string()))?;
                net_keys.push(key);
            }
        }

        if plan.has_fs_rules {
            let map = self
                .ebpf
                .map_mut("FS_DENY")
                .ok_or_else(|| ScopeError::Map("FS_DENY map missing".into()))?;
            let mut deny: aya::maps::HashMap<_, u64, bee_common::layout::DenyList> =
                aya::maps::HashMap::try_from(map).map_err(|e| ScopeError::Map(e.to_string()))?;
            deny.insert(cgroup_id, plan.fs_rules, 0)
                .map_err(|e| ScopeError::Map(e.to_string()))?;
        }

        if plan.has_exec_rules {
            let map = self
                .ebpf
                .map_mut("EXEC_ALLOW")
                .ok_or_else(|| ScopeError::Map("EXEC_ALLOW map missing".into()))?;
            let mut execs: aya::maps::HashMap<_, u64, bee_common::layout::DenyList> =
                aya::maps::HashMap::try_from(map).map_err(|e| ScopeError::Map(e.to_string()))?;
            execs
                .insert(cgroup_id, plan.exec_rules, 0)
                .map_err(|e| ScopeError::Map(e.to_string()))?;
        }

        Ok(Scope {
            path,
            cgroup_id,
            net_keys,
        })
    }

    /// Re-populate an already-created scope's rule maps from `plan` (007-dynamic-grants), supporting
    /// both widening (added rules) and narrowing (removed rules). The LSM programs stay attached;
    /// only map contents change. `prev_net_keys` is the scope's currently-installed `NET_ALLOW` key
    /// set; the method inserts additions and removes deletions and returns the new key set.
    ///
    /// Caller preconditions: no sandboxed child is executing in this scope (a turn boundary), and
    /// `plan` was compiled from a policy already validated `⊆ ceiling`. See
    /// `specs/007-dynamic-grants/contracts/reload-api.md`.
    #[cfg(feature = "enforce")]
    pub fn reload_scope(
        &mut self,
        cgroup_id: u64,
        plan: &EnforcementPlan,
        prev_net_keys: &[bee_common::layout::NetKey],
    ) -> Result<Vec<bee_common::layout::NetKey>, ScopeError> {
        use bee_common::layout::{DenyList, NetKey, ScopeMeta};

        // SCOPES meta — overwrite (mode/flags may change, e.g. the write-default-deny latch).
        {
            let map = self
                .ebpf
                .map_mut("SCOPES")
                .ok_or_else(|| ScopeError::Map("SCOPES map missing".into()))?;
            let mut scopes: aya::maps::HashMap<_, u64, ScopeMeta> =
                aya::maps::HashMap::try_from(map).map_err(|e| ScopeError::Map(e.to_string()))?;
            scopes
                .insert(cgroup_id, plan.meta, 0)
                .map_err(|e| ScopeError::Map(e.to_string()))?;
        }

        // FS_DENY — single value per cgroup: overwrite when present, remove when the policy has none.
        {
            let map = self
                .ebpf
                .map_mut("FS_DENY")
                .ok_or_else(|| ScopeError::Map("FS_DENY map missing".into()))?;
            let mut deny: aya::maps::HashMap<_, u64, DenyList> =
                aya::maps::HashMap::try_from(map).map_err(|e| ScopeError::Map(e.to_string()))?;
            if plan.has_fs_rules {
                deny.insert(cgroup_id, plan.fs_rules, 0)
                    .map_err(|e| ScopeError::Map(e.to_string()))?;
            } else {
                // Ignore a missing-key error — narrowing to no fs rules is idempotent.
                let _ = deny.remove(&cgroup_id);
            }
        }

        // EXEC_ALLOW — same single-value overwrite/remove.
        {
            let map = self
                .ebpf
                .map_mut("EXEC_ALLOW")
                .ok_or_else(|| ScopeError::Map("EXEC_ALLOW map missing".into()))?;
            let mut execs: aya::maps::HashMap<_, u64, DenyList> =
                aya::maps::HashMap::try_from(map).map_err(|e| ScopeError::Map(e.to_string()))?;
            if plan.has_exec_rules {
                execs
                    .insert(cgroup_id, plan.exec_rules, 0)
                    .map_err(|e| ScopeError::Map(e.to_string()))?;
            } else {
                let _ = execs.remove(&cgroup_id);
            }
        }

        // NET_ALLOW — per-rule keys: diff the new set against `prev_net_keys`, insert additions,
        // remove deletions, and return the resulting installed set.
        let new_keys: Vec<NetKey> = plan
            .net_rules
            .iter()
            .map(|t| {
                let mut k = *t;
                k.cgroup_id = cgroup_id;
                k
            })
            .collect();
        {
            let map = self
                .ebpf
                .map_mut("NET_ALLOW")
                .ok_or_else(|| ScopeError::Map("NET_ALLOW map missing".into()))?;
            let mut net: aya::maps::HashMap<_, NetKey, u8> =
                aya::maps::HashMap::try_from(map).map_err(|e| ScopeError::Map(e.to_string()))?;
            for key in &new_keys {
                if !prev_net_keys.contains(key) {
                    net.insert(*key, 1u8, 0)
                        .map_err(|e| ScopeError::Map(e.to_string()))?;
                }
            }
            for key in prev_net_keys {
                if !new_keys.contains(key) {
                    let _ = net.remove(key);
                }
            }
        }

        Ok(new_keys)
    }

    /// Take ownership of the audit ring buffer as a synchronous reader (call once). (enforce feature)
    #[cfg(feature = "enforce")]
    pub fn take_audit_reader(&mut self, scope_id: &str) -> Result<events::AuditReader, ScopeError> {
        let map = self
            .ebpf
            .take_map("AUDIT_RB")
            .ok_or_else(|| ScopeError::Map("AUDIT_RB map missing".into()))?;
        let rb = aya::maps::RingBuf::try_from(map).map_err(|e| ScopeError::Map(e.to_string()))?;
        Ok(events::AuditReader::new(rb, scope_id.to_string()))
    }

    /// Take ownership of the audit ring buffer as an **async** stream (US4, `tokio` feature). Like
    /// [`take_audit_reader`](Self::take_audit_reader), the underlying map is `take_map` — it can be
    /// taken **once**, so an engine serves either the sync reader OR the async stream for its
    /// lifetime, never both. Pick one when the engine is created.
    #[cfg(feature = "tokio")]
    pub fn take_async_audit_stream(
        &mut self,
        scope_id: &str,
    ) -> Result<async_events::AsyncAuditStream, ScopeError> {
        let map = self
            .ebpf
            .take_map("AUDIT_RB")
            .ok_or_else(|| ScopeError::Map("AUDIT_RB map missing (already taken?)".into()))?;
        let rb = aya::maps::RingBuf::try_from(map).map_err(|e| ScopeError::Map(e.to_string()))?;
        async_events::AsyncAuditStream::new(rb, scope_id.to_string())
            .map_err(|e| ScopeError::Map(format!("async audit stream: {e}")))
    }
}

/// A live scope = one bee-managed cgroup registered for enforcement. (enforce feature)
#[cfg(feature = "enforce")]
pub struct Scope {
    pub path: std::path::PathBuf,
    pub cgroup_id: u64,
    /// The `NET_ALLOW` keys currently installed for this scope, so a narrowing reload
    /// (007-dynamic-grants) can remove the ones that get dropped. Updated by `reload_scope`.
    pub net_keys: Vec<bee_common::layout::NetKey>,
}

#[cfg(feature = "enforce")]
impl Scope {
    /// A fork-safe `pre_exec` closure that joins the calling process to this scope's cgroup.
    pub fn join_closure(
        &self,
    ) -> Result<impl Fn() -> std::io::Result<()> + Send + Sync + 'static, ScopeError> {
        let procs = cgroup::procs_path_c(&self.path)?;
        Ok(move || cgroup::raw_join_self(&procs))
    }

    /// Remove the scope's cgroup (best-effort).
    pub fn teardown(&self) -> std::io::Result<()> {
        cgroup::teardown_scope_cgroup(&self.path)
    }
}

#[cfg(all(test, not(feature = "enforce")))]
mod tests {
    use super::*;

    #[test]
    fn init_is_fail_closed_on_unsupported_kernel() {
        match Engine::init() {
            Err(EngineError::Unsupported(_)) => {} // non-BPF-LSM kernel
            Err(EngineError::EnforceFeatureDisabled) => {} // supported, but no embedded object
            Err(EngineError::Load(_)) => {}
            Ok(_) => panic!("Engine::init must never succeed without the enforce feature"),
        }
    }
}
