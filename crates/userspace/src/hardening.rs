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
//! * [`drop_privileges`] — `PR_SET_NO_NEW_PRIVS` plus a full capability drop, for tool children only.
//!
//! The privilege drop is deliberately **not** part of [`apply_hardening`]: bee itself needs
//! `CAP_BPF`/`CAP_SYS_ADMIN` to load the LSM programs, so only the forked tool child sheds them.

use std::io;

/// `_LINUX_CAPABILITY_VERSION_3` — the 64-bit capability ABI (two 32-bit data words).
const CAP_VERSION_3: u32 = 0x2008_0522;

/// Header for `capset(2)`. Matches `struct __user_cap_header_struct`.
#[repr(C)]
struct CapHeader {
    version: u32,
    pid: libc::c_int,
}

/// One 32-bit slice of the three capability sets. Matches `struct __user_cap_data_struct`.
#[repr(C)]
#[derive(Default, Clone, Copy)]
struct CapData {
    effective: u32,
    permitted: u32,
    inheritable: u32,
}

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

/// Shed every privilege a tool child could use to leave its scope, then forbid regaining any
/// (FR-015). Fork-safe: raw `prctl`/`capset` syscalls only, no allocation.
///
/// The uid is deliberately left alone — bee runs as root under `--features enforce`, and the
/// scenario workdir it materializes is root-owned, so a uid change would leave the child unable to
/// read its own task. What goes instead is everything that *makes* root powerful:
///
/// 1. `PR_SET_NO_NEW_PRIVS` — an `execve` can no longer gain privilege from a setuid/setgid image
///    or from file capabilities. This is what closes the "we only checked the first executable"
///    hole: `sh -c` can no longer reach a privileged binary even though `sh` itself passed the
///    [`crate::spawn::is_privileged_target`] check.
/// 2. `SECBIT_NOROOT` (+ locks) — uid 0 no longer implies a full capability set across `execve`.
///    Without this, steps 3–5 would be undone by the very next exec.
/// 3. Ambient set cleared, 4. bounding set emptied, 5. permitted/effective/inheritable zeroed —
///    so the running child holds no capability, cannot pass one across exec, and cannot re-raise.
///
/// Order matters: `PR_SET_SECUREBITS` and `PR_CAPBSET_DROP` both need `CAP_SETPCAP`, so they must
/// run before the `capset` that discards it.
///
/// Steps 2–4 are best-effort: an unprivileged launcher (the default host build) has no `CAP_SETPCAP`
/// and nothing to drop, and failing the spawn there would break every non-root use of bee. Steps 1
/// and 5 are always permitted by the kernel, so a failure there is real and fails the spawn closed.
pub fn drop_privileges() -> io::Result<()> {
    // 1. No exec may ever gain privilege from here on. Always permitted; a failure is real.
    // SAFETY: PR_SET_NO_NEW_PRIVS takes one integer argument; the rest are ignored.
    if unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0 {
        return Err(io::Error::last_os_error());
    }

    // 2. Stop uid 0 from re-acquiring capabilities on exec, and lock the bits so the child cannot
    // clear them again. Needs CAP_SETPCAP — best-effort (see above).
    let secbits = libc::SECBIT_NOROOT
        | libc::SECBIT_NOROOT_LOCKED
        | libc::SECBIT_NO_SETUID_FIXUP
        | libc::SECBIT_NO_SETUID_FIXUP_LOCKED
        | libc::SECBIT_NO_CAP_AMBIENT_RAISE
        | libc::SECBIT_NO_CAP_AMBIENT_RAISE_LOCKED;
    // SAFETY: PR_SET_SECUREBITS takes one integer argument; the rest are ignored.
    unsafe { libc::prctl(libc::PR_SET_SECUREBITS, secbits, 0, 0, 0) };

    // 3. Drop the ambient set, which would otherwise survive exec on its own.
    // SAFETY: PR_CAP_AMBIENT_CLEAR_ALL takes no further arguments.
    unsafe {
        libc::prctl(
            libc::PR_CAP_AMBIENT,
            libc::PR_CAP_AMBIENT_CLEAR_ALL,
            0,
            0,
            0,
        )
    };

    // 4. Empty the bounding set so no file capability can ever be picked up. Walk past the kernel's
    // CAP_LAST_CAP and let the surplus fail with EINVAL rather than read /proc to find the limit
    // (which is not fork-safe).
    for cap in 0..=63 {
        // SAFETY: PR_CAPBSET_DROP takes one integer argument; out-of-range values return EINVAL.
        unsafe { libc::prctl(libc::PR_CAPBSET_DROP, cap, 0, 0, 0) };
    }

    // 5. Zero the three per-thread sets. Dropping is always permitted, so a failure here is real.
    let header = CapHeader {
        version: CAP_VERSION_3,
        pid: 0, // 0 == the calling thread
    };
    let data = [CapData::default(); 2];
    // SAFETY: `header` and `data` are valid, fully-initialized structures matching the v3 capability
    // ABI, and `capset` reads (never writes) them for the duration of the call.
    let rc = unsafe { libc::syscall(libc::SYS_capset, &header as *const CapHeader, data.as_ptr()) };
    if rc != 0 {
        return Err(io::Error::last_os_error());
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
