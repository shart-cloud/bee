//! cgroup v2 scope lifecycle (FR-004, research R4).
//!
//! bee creates one cgroup per scope under a configurable parent and identifies it by the kernel
//! cgroup id — the value `bpf_get_current_cgroup_id()` returns in-kernel, which on cgroupfs equals
//! the directory's inode number. The id is the key for every per-scope BPF map.

use std::ffi::CString;
use std::io;
use std::mem::MaybeUninit;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

/// Default parent cgroup under which bee creates scope cgroups.
pub const DEFAULT_PARENT: &str = "/sys/fs/cgroup/bee";

/// The cgroup id (== inode number on cgroupfs) of `path`. Matches `bpf_get_current_cgroup_id()`.
// `st_ino` is `u64` on glibc/Linux but not on every target; the cast keeps this portable.
#[allow(clippy::unnecessary_cast)]
pub fn cgroup_id(path: &Path) -> io::Result<u64> {
    let c = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains NUL"))?;
    let mut st = MaybeUninit::<libc::stat>::uninit();
    // SAFETY: `c` is a valid NUL-terminated path; `st` is writable for the syscall's duration.
    let rc = unsafe { libc::stat(c.as_ptr(), st.as_mut_ptr()) };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: stat succeeded, so `st` is initialized.
    let st = unsafe { st.assume_init() };
    Ok(st.st_ino as u64)
}

/// Create the scope cgroup directory `<parent>/<scope_id>`. Requires delegated cgroup v2 controllers
/// or root. Returns the created path.
pub fn create_scope_cgroup(parent: &str, scope_id: &str) -> io::Result<PathBuf> {
    let path = Path::new(parent).join(scope_id);
    std::fs::create_dir_all(&path)?;
    Ok(path)
}

/// Remove a scope cgroup directory (must be empty of processes).
pub fn teardown_scope_cgroup(cgroup: &Path) -> io::Result<()> {
    std::fs::remove_dir(cgroup)
}

/// The `cgroup.procs` path for a scope, as a NUL-terminated C string ready for a fork-safe join.
pub fn procs_path_c(cgroup: &Path) -> io::Result<std::ffi::CString> {
    std::ffi::CString::new(cgroup.join("cgroup.procs").as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains NUL"))
}

/// Join the *current* process to a cgroup by writing `0` to `cgroup.procs`, using only raw
/// open/write/close syscalls so it is safe to call from a `pre_exec` hook (between fork and exec).
///
/// # Safety
/// Must be called in a context where issuing raw syscalls is sound (e.g. a `pre_exec` child).
pub fn raw_join_self(procs_path: &std::ffi::CStr) -> io::Result<()> {
    // SAFETY: open/write/close are async-signal-safe; `procs_path` is a valid NUL-terminated path.
    unsafe {
        let fd = libc::open(procs_path.as_ptr(), libc::O_WRONLY | libc::O_CLOEXEC);
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let buf = b"0\n";
        let n = libc::write(fd, buf.as_ptr() as *const libc::c_void, buf.len());
        let werr = io::Error::last_os_error();
        libc::close(fd);
        if n < 0 {
            return Err(werr);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cgroup_id_reads_inode() {
        // The root filesystem always exists; its inode is a stable nonzero id.
        let id = cgroup_id(Path::new("/")).expect("stat /");
        assert_ne!(id, 0);
    }

    #[test]
    fn cgroup_id_errors_on_missing() {
        assert!(cgroup_id(Path::new("/nonexistent/bee/scope")).is_err());
    }
}
