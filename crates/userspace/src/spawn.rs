//! Launching a command into a scope: hardening (FR-011) and privileged-target refusal (FR-015).
//!
//! The process-launch machinery here is independent of the eBPF layer and is verifiable without a
//! BPF-LSM kernel. Under the `enforce` feature the caller also moves the child into the scope cgroup
//! before exec (see [`crate::cgroup`]); the hardening + refusal logic below is identical either way.

use std::ffi::CString;
use std::io;
use std::mem::MaybeUninit;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum SpawnError {
    #[error("refusing to sandbox a privileged target (setuid/setgid or file capabilities): {0}")]
    PrivilegedTarget(String),
    #[error("cannot resolve command '{0}' on PATH")]
    NotFound(String),
    #[error("spawn failed: {0}")]
    Io(#[from] io::Error),
}

/// Resolve a command name to an absolute path via `PATH` (or return it if already absolute).
pub fn resolve_in_path(cmd: &str) -> Option<PathBuf> {
    let p = Path::new(cmd);
    if p.is_absolute() {
        return p.exists().then(|| p.to_path_buf());
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(cmd))
        .find(|c| c.exists())
}

/// True if `path` is setuid/setgid or carries file capabilities — bee must refuse these (FR-015).
pub fn is_privileged_target(path: &Path) -> io::Result<bool> {
    let c = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains NUL"))?;
    let mut st = MaybeUninit::<libc::stat>::uninit();
    // SAFETY: valid path + writable stat buffer.
    let rc = unsafe { libc::stat(c.as_ptr(), st.as_mut_ptr()) };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: stat succeeded.
    let st = unsafe { st.assume_init() };
    let setid = st.st_mode & (libc::S_ISUID | libc::S_ISGID) != 0;
    Ok(setid || has_file_caps(&c))
}

/// Detect a non-empty `security.capability` xattr (file capabilities).
fn has_file_caps(path: &CString) -> bool {
    let name = c"security.capability";
    // SAFETY: null value pointer with size 0 asks only for the attribute size; -1/ENODATA if absent.
    let rc = unsafe { libc::getxattr(path.as_ptr(), name.as_ptr(), std::ptr::null_mut(), 0) };
    rc > 0
}

/// Build a `Command` that hardens the child (in `pre_exec`, between fork and exec) after refusing a
/// privileged target. The optional `cgroup_move` closure runs in the child too (used under `enforce`
/// to join the scope cgroup). The caller runs/waits on the returned `Command`.
pub fn hardened_command<F>(
    program: &str,
    args: &[String],
    cgroup_move: Option<F>,
) -> Result<Command, SpawnError>
where
    F: Fn() -> io::Result<()> + Send + Sync + 'static,
{
    let resolved = resolve_in_path(program).ok_or_else(|| SpawnError::NotFound(program.into()))?;
    if is_privileged_target(&resolved)? {
        return Err(SpawnError::PrivilegedTarget(resolved.display().to_string()));
    }

    let mut cmd = Command::new(&resolved);
    cmd.args(args);
    // Strip LD_* in the PARENT (fork-safe): remove them from the child's environment before exec.
    // Doing this in pre_exec would deadlock — env access takes locks and allocates, which is unsafe
    // after fork() in a multithreaded process.
    for key in crate::hardening::ld_env_keys(std::env::vars().map(|(k, _)| k)) {
        cmd.env_remove(key);
    }
    // SAFETY: the pre_exec closure runs in the forked child before exec and issues only
    // async-signal-safe operations: the raw setrlimit/prctl/capset syscalls in `pre_exec_hardening`
    // and `drop_privileges`, plus the caller-supplied cgroup join (which, under the enforce feature,
    // must itself use raw open/write syscalls — not std::fs — to stay fork-safe).
    unsafe {
        cmd.pre_exec(move || {
            // The cgroup join goes FIRST: writing the scope's `cgroup.procs` needs the launcher's
            // authority, and `drop_privileges` is about to discard it. Joining afterwards would
            // fail with EACCES and — because the error propagates — turn every enforced spawn into
            // a spawn failure.
            if let Some(join) = &cgroup_move {
                join()?;
            }
            crate::hardening::pre_exec_hardening()?;
            // Only now shed privilege, so the child cannot exec its way to a capability the policy
            // never granted or wield CAP_SYS_ADMIN/CAP_BPF to detach bee's own LSM (FR-015). The DAC
            // pair is kept so file access still flows through the LSM hook — see `drop_privileges`.
            crate::hardening::drop_privileges()?;
            Ok(())
        });
    }
    Ok(cmd)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_absolute_and_path() {
        assert!(resolve_in_path("/bin/sh").is_some() || resolve_in_path("/usr/bin/sh").is_some());
        assert!(resolve_in_path("definitely-not-a-real-binary-xyz").is_none());
    }

    #[test]
    fn plain_binary_is_not_privileged() {
        let sh = resolve_in_path("sh").expect("sh exists");
        assert!(
            !is_privileged_target(&sh).unwrap(),
            "sh should not be setuid"
        );
    }

    #[test]
    fn refuses_setuid_target_if_present() {
        // Find a setuid binary in common locations; skip if none (environment-dependent).
        for cand in [
            "/usr/bin/sudo",
            "/bin/su",
            "/usr/bin/passwd",
            "/usr/bin/mount",
        ] {
            let p = Path::new(cand);
            if p.exists() && is_privileged_target(p).unwrap() {
                let err = hardened_command::<fn() -> io::Result<()>>(cand, &[], None).unwrap_err();
                assert!(matches!(err, SpawnError::PrivilegedTarget(_)));
                return;
            }
        }
        eprintln!("no setuid binary available in this environment; refusal path not exercised");
    }

    /// The child must come out the other side of `pre_exec` unable to gain privilege, and — when the
    /// launcher was root — holding only the DAC capabilities bee keeps so its LSM stays the file
    /// arbiter. Reading it back from `/proc/self/status` in the child is the only honest check: the
    /// parent's own bits are untouched, which is the point.
    #[test]
    fn child_has_no_new_privs_and_only_the_dac_caps() {
        let mut cmd = hardened_command::<fn() -> io::Result<()>>(
            "cat",
            &["/proc/self/status".to_string()],
            None,
        )
        .expect("build cmd");
        let out = cmd.output().expect("run cat");
        let status = String::from_utf8_lossy(&out.stdout);

        let field = |name: &str| {
            status
                .lines()
                .find_map(|l| l.strip_prefix(name)?.split_whitespace().next())
                .unwrap_or_else(|| panic!("{name} missing from /proc/self/status"))
                .to_string()
        };

        assert_eq!(field("NoNewPrivs:"), "1", "PR_SET_NO_NEW_PRIVS not applied");
        // Bounding-set restriction needs CAP_SETPCAP, so only assert it when we ran as root. The
        // kept set is exactly CAP_DAC_OVERRIDE (bit 1) | CAP_DAC_READ_SEARCH (bit 2) == 0x6, and
        // CAP_SYS_ADMIN (bit 21) must be gone.
        if unsafe { libc::geteuid() } == 0 {
            assert_eq!(
                field("CapBnd:"),
                "0000000000000006",
                "bounding set should keep only the DAC pair"
            );
            let eff = u64::from_str_radix(&field("CapEff:"), 16).expect("hex CapEff");
            assert_eq!(eff & (1 << 21), 0, "CAP_SYS_ADMIN still effective");
        }
    }

    #[test]
    fn runs_hardened_true() {
        let mut cmd =
            hardened_command::<fn() -> io::Result<()>>("true", &[], None).expect("build cmd");
        let status = cmd.status().expect("run true");
        assert!(status.success());
    }
}
