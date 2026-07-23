//! eBPF object loading + LSM attach (enforce feature only). Research R2/R3.

#![cfg(feature = "enforce")]

use aya::programs::Lsm;
use aya::{Btf, Ebpf};

use crate::EngineError;

/// The embedded BPF object, compiled by `build.rs` via aya-build.
pub static EBPF_OBJ: &[u8] = aya::include_bytes_aligned!(concat!(env!("OUT_DIR"), "/bee-lsm"));

/// LSM hooks bee attaches. Extend as hooks are implemented.
const HOOKS: &[&str] = &["socket_connect", "file_open", "bprm_check_security"];

/// Load the object and attach every LSM program. Returns the live `Ebpf` (holds the links; dropping
/// it detaches enforcement).
pub fn load_and_attach() -> Result<Ebpf, EngineError> {
    let mut ebpf =
        Ebpf::load(EBPF_OBJ).map_err(|e| EngineError::Load(format!("load object: {e}")))?;
    let btf = Btf::from_sys_fs().map_err(|e| EngineError::Load(format!("BTF from sysfs: {e}")))?;

    for hook in HOOKS {
        let prog: &mut Lsm = ebpf
            .program_mut(hook)
            .ok_or_else(|| EngineError::Load(format!("program '{hook}' missing from object")))?
            .try_into()
            .map_err(|e| EngineError::Load(format!("program '{hook}' not an LSM: {e}")))?;
        prog.load(hook, &btf)
            .map_err(|e| EngineError::Load(format!("load '{hook}': {e}")))?;
        prog.attach()
            .map_err(|e| EngineError::Load(format!("attach '{hook}': {e}")))?;
    }
    Ok(ebpf)
}
