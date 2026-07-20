//! Fail-closed support detection (FR-009 / SC-007, research R3).
//!
//! Before attaching any program, bee MUST prove the kernel can enforce. This module reads the
//! authoritative runtime signals and reports a per-gate diagnostic. The parsing logic is pure and
//! unit-tested with fixtures; [`detect`] wires it to the real `/proc` and `/sys` files.

use std::fmt;

/// Minimum kernel for BPF LSM programs.
pub const MIN_KERNEL: (u32, u32) = (5, 7);

/// Result of the support probe. Enforcement is available only when every gate passes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Support {
    /// Kernel ≥ 5.7.
    pub kernel_ok: bool,
    /// `bpf` is in the active LSM list (`/sys/kernel/security/lsm`).
    pub bpf_lsm_active: bool,
    /// cgroup v2 unified hierarchy is mounted.
    pub cgroup_v2: bool,
    /// bee's compiled-in kernel struct offsets match the running kernel's BTF (`kbtf`). A mismatch
    /// means the eBPF probe would read the wrong fields and fail **open**, so this is a hard gate.
    pub offsets_ok: bool,
    /// Detected kernel version (major, minor), if parseable.
    pub kernel_version: Option<(u32, u32)>,
    /// First failing gate's human-readable reason, if any.
    pub reason: Option<String>,
}

impl Support {
    /// All gates pass → bee may attach and enforce.
    pub fn is_supported(&self) -> bool {
        self.kernel_ok && self.bpf_lsm_active && self.cgroup_v2 && self.offsets_ok
    }
}

impl fmt::Display for Support {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mark = |b: bool| if b { "PASS" } else { "FAIL" };
        writeln!(
            f,
            "kernel>=5.7      [{}] {}",
            mark(self.kernel_ok),
            self.kernel_version.map(|(a, b)| format!("{a}.{b}")).unwrap_or_else(|| "unknown".into())
        )?;
        writeln!(f, "bpf in LSM list  [{}]", mark(self.bpf_lsm_active))?;
        writeln!(f, "cgroup v2        [{}]", mark(self.cgroup_v2))?;
        write!(f, "kernel offsets   [{}]", mark(self.offsets_ok))?;
        if let Some(r) = &self.reason {
            write!(f, "\n=> {r}")?;
        }
        Ok(())
    }
}

/// Parse a kernel `uname -r` release string into `(major, minor)`.
pub fn parse_kernel_version(release: &str) -> Option<(u32, u32)> {
    let mut it = release.split(['.', '-', '_']);
    let major = it.next()?.parse().ok()?;
    let minor = it.next()?.parse().ok()?;
    Some((major, minor))
}

/// Is `(major, minor)` at least `MIN_KERNEL`?
pub fn kernel_meets_min(v: (u32, u32)) -> bool {
    v >= MIN_KERNEL
}

/// Does the `/sys/kernel/security/lsm` contents contain an exact `bpf` token?
/// `CONFIG_BPF_LSM=y` alone is insufficient — `bpf` must be in the *active* list (research R3).
pub fn lsm_list_has_bpf(contents: &str) -> bool {
    contents.trim().split(',').any(|tok| tok.trim() == "bpf")
}

/// Probe the running kernel. Any failure leaves `is_supported() == false` (fail-closed).
pub fn detect() -> Support {
    let kernel_version = read_kernel_release().and_then(|r| parse_kernel_version(&r));
    let kernel_ok = kernel_version.map(kernel_meets_min).unwrap_or(false);

    let bpf_lsm_active = std::fs::read_to_string("/sys/kernel/security/lsm")
        .map(|s| lsm_list_has_bpf(&s))
        .unwrap_or(false);

    // cgroup v2 unified hierarchy exposes cgroup.controllers at the mount root.
    let cgroup_v2 = std::path::Path::new("/sys/fs/cgroup/cgroup.controllers").exists();

    // Validate compiled kernel struct offsets against the running kernel's BTF (fail-closed: a
    // mismatch would make the eBPF probe read the wrong fields and fail open).
    let offset_check = crate::kbtf::check_running();
    let offsets_ok = offset_check.is_ok();
    let offset_reason = offset_check.err();

    let reason = if !kernel_ok {
        Some(format!(
            "kernel {} is below the required {}.{} for BPF LSM",
            kernel_version.map(|(a, b)| format!("{a}.{b}")).unwrap_or_else(|| "unknown".into()),
            MIN_KERNEL.0,
            MIN_KERNEL.1
        ))
    } else if !bpf_lsm_active {
        Some(
            "'bpf' is not in the active LSM list (/sys/kernel/security/lsm); add 'lsm=...,bpf' to \
             the kernel cmdline and reboot"
                .into(),
        )
    } else if !cgroup_v2 {
        Some("cgroup v2 unified hierarchy is not mounted at /sys/fs/cgroup".into())
    } else if !offsets_ok {
        offset_reason
    } else {
        None
    };

    Support { kernel_ok, bpf_lsm_active, cgroup_v2, offsets_ok, kernel_version, reason }
}

fn read_kernel_release() -> Option<String> {
    // /proc/sys/kernel/osrelease is the release string without a syscall.
    std::fs::read_to_string("/proc/sys/kernel/osrelease").ok().map(|s| s.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_parsing() {
        assert_eq!(parse_kernel_version("6.6.114.1-microsoft-standard-WSL2"), Some((6, 6)));
        assert_eq!(parse_kernel_version("5.7.0"), Some((5, 7)));
        assert_eq!(parse_kernel_version("5.15.0-generic"), Some((5, 15)));
        assert_eq!(parse_kernel_version("garbage"), None);
    }

    #[test]
    fn min_kernel_boundary() {
        assert!(!kernel_meets_min((5, 6)));
        assert!(kernel_meets_min((5, 7)));
        assert!(kernel_meets_min((6, 1)));
        assert!(!kernel_meets_min((4, 19)));
    }

    #[test]
    fn lsm_token_matching() {
        assert!(lsm_list_has_bpf("lockdown,capability,yama,apparmor,bpf"));
        assert!(lsm_list_has_bpf("bpf"));
        assert!(!lsm_list_has_bpf("lockdown,capability,yama,apparmor"));
        // Must be an exact token, not a substring of another LSM name.
        assert!(!lsm_list_has_bpf("lockdown,bpfcontain,yama"));
    }

    #[test]
    fn unsupported_when_all_fail() {
        let s = Support {
            kernel_ok: false,
            bpf_lsm_active: false,
            cgroup_v2: false,
            offsets_ok: false,
            kernel_version: Some((5, 4)),
            reason: Some("old kernel".into()),
        };
        assert!(!s.is_supported());
    }
}
