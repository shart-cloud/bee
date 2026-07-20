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

use std::io;

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

/// `setrlimit(RLIMIT_CORE, 0, 0)`.
pub fn disable_core_dumps() -> io::Result<()> {
    let lim = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
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
        assert_eq!(got, vec!["LD_LIBRARY_PATH".to_string(), "LD_PRELOAD".to_string()]);
    }

    #[test]
    fn core_dumps_and_dumpable_apply() {
        // These affect the test process itself; both should succeed on Linux.
        disable_core_dumps().expect("setrlimit RLIMIT_CORE");
        set_non_dumpable().expect("prctl PR_SET_DUMPABLE");
    }
}
