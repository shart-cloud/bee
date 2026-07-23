//! Process hardening applied before a sandboxed program runs (FR-011).
//!
//! The **launcher** calls [`apply_hardening`] in the forked child *between fork and exec* — this is
//! the general path that works for arbitrary tool binaries (`cargo`, `rustc`, …) that do not link
//! bee. With the `ctor` feature, an additional constructor hardens the current process before
//! `main()` for agents that embed bee in-process.
//!
//! Hardening steps:
//! * `RLIMIT_CORE = 0` — no core dumps that could leak memory.
//! * `PR_SET_DUMPABLE = 0` — process is non-dumpable (blocks ptrace/`/proc/pid/mem` by non-root).
//! * strip `LD_*` environment variables — defeat `LD_PRELOAD`/`LD_LIBRARY_PATH` injection.
//! * [`drop_privileges`] — `PR_SET_NO_NEW_PRIVS` plus an escape-capability drop, for tool children.
//!
//! The privilege drop is deliberately **not** part of [`apply_hardening`]: bee itself needs
//! `CAP_BPF`/`CAP_SYS_ADMIN` to load the LSM programs, so only the forked tool child sheds them.

use std::io;

/// The only capabilities a root tool child keeps: `CAP_DAC_OVERRIDE` (1) and `CAP_DAC_READ_SEARCH`
/// (2). Everything else — `CAP_SYS_ADMIN`, `CAP_BPF`, `CAP_SYS_PTRACE`, `CAP_SYS_MODULE`, … — is
/// dropped from the bounding set so a root child cannot detach bee's own LSM programs, remount, or
/// otherwise escape the scope.
///
/// The DAC pair stays for a specific reason: bee's **eBPF LSM policy is the file-access arbiter**,
/// and its audit trail is only complete if every open reaches the LSM hook. `security_file_open`
/// runs *after* the kernel's DAC check, so a root child stripped of DAC override is denied at DAC
/// before the policy is ever consulted — the operator's rule is never evaluated and no audit record
/// is produced. Keeping DAC override lets root traverse and read/write as before; the LSM then makes
/// the actual allow/deny/observe decision and records it.
const KEEP_CAPS: [i32; 2] = [
    1, /* CAP_DAC_OVERRIDE */
    2, /* CAP_DAC_READ_SEARCH */
];

/// Apply all hardening steps to the **current process** (for the embedded / `#[ctor]` case, or any
/// context that is *not* between `fork` and `exec`). Strips `LD_*` from the live environment, which
/// takes locks and allocates — do NOT call this from a `pre_exec` hook. Launchers should instead
/// strip `LD_*` on the `Command` in the parent and call [`pre_exec_hardening`] in the child.
pub fn apply_hardening() -> io::Result<()> {
    disable_core_dumps()?;
    set_non_dumpable()?;
    strip_ld_env();
    Ok(())
}

/// The fork-safe subset of hardening, for use inside a `pre_exec` hook (between `fork` and `execve`).
/// Only issues the `setrlimit`/`prctl` syscalls — both async-signal-safe — and does **not** touch the
/// environment. The launcher strips `LD_*` on the parent's `Command` before forking.
pub fn pre_exec_hardening() -> io::Result<()> {
    disable_core_dumps()?;
    set_non_dumpable()?;
    Ok(())
}

/// Shed every privilege a tool child could use to leave its scope, without disturbing the file
/// access bee's LSM policy is meant to arbitrate (FR-015). Fork-safe: raw `prctl` syscalls only, no
/// allocation.
///
/// The uid is deliberately left alone — bee runs as root under `--features enforce`, and the
/// scenario workdir it materializes is root-owned, so a uid change would leave the child unable to
/// read its own task. Two steps instead:
///
/// 1. `PR_SET_NO_NEW_PRIVS` — an `execve` can no longer gain privilege from a setuid/setgid image
///    or from file capabilities. This is what closes the "we only checked the first executable"
///    hole: `sh -c` can no longer reach a privileged binary even though `sh` itself passed the
///    [`crate::spawn::is_privileged_target`] check.
/// 2. Drop every capability from the **bounding set** except [`KEEP_CAPS`]. `SECBIT_NOROOT` is
///    deliberately *not* set: with it off, the kernel's root-magic re-derives the child's post-exec
///    capabilities from the bounding set, so bounding-dropping `CAP_SYS_ADMIN`/`CAP_BPF`/… removes
///    them from the running child while the retained DAC pair still comes through. That keeps the
///    LSM — not DAC — the file-access decision point (see [`KEEP_CAPS`]). There is no window: the
///    full set only exists between this call and the immediately following `execve` of the tool.
///
/// Step 2 is best-effort: an unprivileged launcher (the default host build) has no `CAP_SETPCAP`,
/// so the `PR_CAPBSET_DROP`s are no-ops and there is nothing to drop anyway. Step 1 is always
/// permitted by the kernel, so a failure there is real and fails the spawn closed.
pub fn drop_privileges() -> io::Result<()> {
    // 1. No exec may ever gain privilege from here on. Always permitted; a failure is real.
    // SAFETY: PR_SET_NO_NEW_PRIVS takes one integer argument; the rest are ignored.
    if unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0 {
        return Err(io::Error::last_os_error());
    }

    // 2. Empty the bounding set except for the DAC pair. Walk past the kernel's CAP_LAST_CAP and let
    // the surplus fail with EINVAL rather than read /proc to find the limit (not fork-safe).
    for cap in 0..=63 {
        if KEEP_CAPS.contains(&cap) {
            continue;
        }
        // SAFETY: PR_CAPBSET_DROP takes one integer argument; out-of-range values return EINVAL.
        unsafe { libc::prctl(libc::PR_CAPBSET_DROP, cap, 0, 0, 0) };
    }
    Ok(())
}

/// `setrlimit(RLIMIT_CORE, 0, 0)`.
pub fn disable_core_dumps() -> io::Result<()> {
    let lim = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `lim` is a valid, fully-initialized rlimit for the duration of the call.
    let rc = unsafe { libc::setrlimit(libc::RLIMIT_CORE, &lim) };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// `prctl(PR_SET_DUMPABLE, 0)`.
pub fn set_non_dumpable() -> io::Result<()> {
    // SAFETY: PR_SET_DUMPABLE takes a single integer argument; remaining args are ignored.
    let rc = unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0) };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Remove every `LD_*` variable from the current process environment.
pub fn strip_ld_env() {
    for key in ld_env_keys(std::env::vars().map(|(k, _)| k)) {
        std::env::remove_var(key);
    }
}

/// Pure helper: given an iterator of env var names, return those that must be stripped (`LD_*`).
/// Separated out so the policy is unit-testable without mutating the real environment.
pub fn ld_env_keys<I: IntoIterator<Item = String>>(vars: I) -> Vec<String> {
    vars.into_iter().filter(|k| k.starts_with("LD_")).collect()
}

/// Harden the current process before `main()`. Enabled with the `ctor` feature for embedded use.
#[cfg(feature = "ctor")]
#[ctor::ctor]
fn harden_before_main() {
    // Best-effort: a failure here must not abort process startup, but we still strip LD_* which is
    // the injection-relevant step.
    let _ = disable_core_dumps();
    let _ = set_non_dumpable();
    strip_ld_env();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_only_ld_vars() {
        let vars = vec![
            "PATH".to_string(),
            "LD_PRELOAD".to_string(),
            "LD_LIBRARY_PATH".to_string(),
            "HOME".to_string(),
            "LDFLAGS".to_string(), // not LD_ prefixed — must NOT be stripped
        ];
        let mut got = ld_env_keys(vars);
        got.sort();
        assert_eq!(
            got,
            vec!["LD_LIBRARY_PATH".to_string(), "LD_PRELOAD".to_string()]
        );
    }

    #[test]
    fn core_dumps_and_dumpable_apply() {
        // These affect the test process itself; both should succeed on Linux.
        disable_core_dumps().expect("setrlimit RLIMIT_CORE");
        set_non_dumpable().expect("prctl PR_SET_DUMPABLE");
    }
}
