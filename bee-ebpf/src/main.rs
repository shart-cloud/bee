#![no_std]
#![no_main]

//! bee eBPF LSM programs.
//!
//! Programs are attached globally (`BPF_LSM_MAC`) and scope enforcement per cgroup by filtering on
//! `bpf_get_current_cgroup_id()`. Return `0` to allow, a negative errno to deny.
//!
//! Two hooks:
//! * `socket_connect` — deny outbound connections for a bee-managed cgroup (no struct reads).
//! * `file_open` — resolve the path with `bpf_d_path` and apply the per-scope rule list: deny
//!   subtrees/patterns, and enforce read-only (`read`-marked paths block write-opens) plus
//!   deny-by-default writes for scopes that declare a writable surface. The requested access is the
//!   `FMODE_WRITE` bit of `file->f_mode`, read by a direct load at a constant offset.
//!
//! `bpf_d_path` needs a `*mut path`; we get it as `&file->f_path`. The `file`/`path`/`linux_binprm`
//! structs in `aya-ebpf-bindings` are opaque, so we read fields at compile-time-constant offsets (the
//! BPF verifier requires *constant* offsets into a BTF pointer, so these cannot be made
//! runtime-configurable — full kernel portability requires CO-RE relocation of these offsets).
//!
//! The offsets live in [`bee_common::offsets`] (shared source of truth) and target kernel 6.8 x86_64.
//! Because a wrong offset reads the wrong field and fails **open**, user space validates them against
//! the running kernel's BTF at startup and refuses to enforce on a mismatch (`bee-userspace::kbtf`).
//! To retarget another kernel, update `bee_common::offsets` from its BTF: `pahole -C file | grep -E
//! 'f_mode|f_path'` and `pahole -C linux_binprm | grep file`.
//!
//! NOTE on CO-RE: true auto-relocating CO-RE is not currently achievable here. `aya-tool`/bindgen
//! generate plain `#[repr(C)]` bindings with **no `preserve_access_index`**, so aya-ebpf field access
//! compiles to fixed offsets and emits no BPF_CORE_FIELD relocations — functionally identical to
//! these constants. (aya's *loader* can apply CO-RE relocations, but the probe side doesn't emit
//! them.) Porting across kernels therefore means updating these two constants until the Rust eBPF
//! toolchain gains `preserve_access_index` field-relocation support.

use core::ffi::c_void;

use aya_ebpf::{
    bindings::path,
    helpers::{
        bpf_d_path, bpf_get_current_cgroup_id, bpf_get_current_pid_tgid, bpf_ktime_get_ns,
        bpf_probe_read_kernel,
    },
    macros::{lsm, map},
    maps::{HashMap, PerCpuArray, RingBuf},
    programs::LsmContext,
};
use bee_common::{
    layout::{DenyList, DenyRule, NetKey, ScopeMeta, FS_KIND_POSTFIX, FS_KIND_SUBTREE},
    AccessMode, AuditRecord, Decision, Op, ScopeMode, DENY_MAX_RULES, DENY_PREFIX_MAX, PATH_MAX,
    TARGET_MAX,
};

const EPERM: i32 = 1;
const EACCES: i32 = 13;

/// Kernel struct field offsets (target 6.8 x86_64), shared with user space so its startup guard can
/// validate them against the running kernel's BTF (see `bee_common::offsets` and module docs).
use bee_common::offsets::{BINPRM_FILE as BINPRM_FILE_OFF, FILE_F_MODE as FILE_F_MODE_OFF, FILE_F_PATH as FILE_F_PATH_OFF};

/// `FMODE_WRITE` (UAPI-stable): the open requests write access. Set for `O_WRONLY`/`O_RDWR`.
const FMODE_WRITE: u32 = 0x2;

use bee_common::{FLAG_FS_WRITE_DEFAULT_DENY, FLAG_NET_ENFORCED};

// UAPI address families (stable ABI).
const AF_INET: u16 = 2;
const AF_INET6: u16 = 10;

/// Cgroups bee manages, keyed by cgroup id → mode/flags. Absence ⇒ not ours ⇒ do not enforce.
#[map]
static SCOPES: HashMap<u64, ScopeMeta> = HashMap::with_max_entries(1024, 0);

/// Per-cgroup file deny prefixes.
#[map]
static FS_DENY: HashMap<u64, DenyList> = HashMap::with_max_entries(1024, 0);

/// Allowed network destinations, keyed by `{cgroup_id, family, port, addr}`. Presence ⇒ allowed.
#[map]
static NET_ALLOW: HashMap<NetKey, u8> = HashMap::with_max_entries(4096, 0);

/// Per-cgroup executable allowlist (path prefixes). Presence of an entry ⇒ exec is enforced.
#[map]
static EXEC_ALLOW: HashMap<u64, DenyList> = HashMap::with_max_entries(1024, 0);

/// Scratch buffer for `bpf_d_path` output (PATH_MAX won't fit on the 512-byte stack).
#[map]
static PATHBUF: PerCpuArray<[u8; PATH_MAX]> = PerCpuArray::with_max_entries(1, 0);

/// Audit records streamed to user space.
#[map]
static AUDIT_RB: RingBuf = RingBuf::with_byte_size(256 * 1024, 0);

#[lsm(hook = "socket_connect")]
pub fn socket_connect(ctx: LsmContext) -> i32 {
    let cgid = unsafe { bpf_get_current_cgroup_id() };
    let meta = match unsafe { SCOPES.get(&cgid) } {
        Some(m) => *m,
        None => return 0,
    };
    if meta.flags & FLAG_NET_ENFORCED == 0 {
        return 0; // network not enforced for this scope
    }

    // arg1 of socket_connect is `struct sockaddr *address` (UAPI-stable layout).
    let addr: *const u8 = ctx.arg(1);
    if addr.is_null() {
        return 0;
    }
    let family = unsafe { bpf_probe_read_kernel(addr as *const u16).unwrap_or(0) };

    let mut key = NetKey { cgroup_id: cgid, family, port: 0, _pad: [0; 4], addr: [0; 16] };
    match family {
        AF_INET => {
            // sockaddr_in: sin_port @2 (be16), sin_addr @4 (4 bytes).
            let port_be = unsafe { bpf_probe_read_kernel((addr as usize + 2) as *const u16).unwrap_or(0) };
            let a = unsafe { bpf_probe_read_kernel((addr as usize + 4) as *const [u8; 4]).unwrap_or([0; 4]) };
            key.port = u16::from_be(port_be);
            key.addr[..4].copy_from_slice(&a);
        }
        AF_INET6 => {
            // sockaddr_in6: sin6_port @2 (be16), sin6_addr @8 (16 bytes).
            let port_be = unsafe { bpf_probe_read_kernel((addr as usize + 2) as *const u16).unwrap_or(0) };
            let a = unsafe { bpf_probe_read_kernel((addr as usize + 8) as *const [u8; 16]).unwrap_or([0; 16]) };
            key.port = u16::from_be(port_be);
            key.addr = a;
        }
        _ => return 0, // non-IP (unix, netlink, …) — not enforced here
    }

    if unsafe { NET_ALLOW.get(&key) }.is_some() {
        return 0; // destination is on the allowlist
    }

    let observe = meta.mode == ScopeMode::Observe as u8;
    emit_audit(cgid, Op::Connect, observe, -EPERM, None);
    if observe {
        0
    } else {
        -EPERM
    }
}

#[lsm(hook = "file_open")]
pub fn file_open(ctx: LsmContext) -> i32 {
    let cgid = unsafe { bpf_get_current_cgroup_id() };
    let meta = match unsafe { SCOPES.get(&cgid) } {
        Some(m) => *m,
        None => return 0,
    };
    let list = match unsafe { FS_DENY.get(&cgid) } {
        Some(l) => l,
        None => return 0, // scope has no file deny rules
    };

    // Resolve the path into the per-CPU buffer.
    let buf_ptr = match PATHBUF.get_ptr_mut(0) {
        Some(p) => p,
        None => return 0,
    };
    // arg0 of file_open is `struct file *`.
    let file: *const c_void = ctx.arg(0);
    if file.is_null() {
        return 0;
    }
    // Requested access: direct load of `file->f_mode` (a scalar field on a trusted LSM BTF pointer,
    // so the verifier maps offset 20 to the u32 field — same mechanism as `bprm->file` below).
    let f_mode = unsafe { *((file as usize + FILE_F_MODE_OFF) as *const u32) };
    let is_write = f_mode & FMODE_WRITE != 0;

    let path_ptr = (file as usize + FILE_F_PATH_OFF) as *mut path;
    // SAFETY: bpf_d_path writes up to PATH_MAX bytes into buf_ptr and returns the length (incl. NUL).
    let ret = unsafe { bpf_d_path(path_ptr, buf_ptr as *mut i8, PATH_MAX as u32) };
    if ret <= 0 {
        return 0; // could not resolve — do not block
    }
    let plen = resolved_len(ret);

    // SAFETY: buf_ptr points at a valid [u8; PATH_MAX] map value.
    let buf = unsafe { &*buf_ptr };
    let write_default_deny = meta.flags & FLAG_FS_WRITE_DEFAULT_DENY != 0;
    if !fs_should_block(buf, plen, list, is_write, write_default_deny) {
        return 0; // access permitted by the rule list
    }

    let observe = meta.mode == ScopeMode::Observe as u8;
    emit_audit(cgid, Op::FileOpen, observe, -EACCES, Some(buf));
    if observe {
        0
    } else {
        -EACCES
    }
}

#[lsm(hook = "bprm_check_security")]
pub fn bprm_check_security(ctx: LsmContext) -> i32 {
    let cgid = unsafe { bpf_get_current_cgroup_id() };
    let meta = match unsafe { SCOPES.get(&cgid) } {
        Some(m) => *m,
        None => return 0,
    };
    // No exec allowlist for this scope ⇒ exec is not enforced.
    let allow = match unsafe { EXEC_ALLOW.get(&cgid) } {
        Some(a) => a,
        None => return 0,
    };

    // arg0 of bprm_check_security is `struct linux_binprm *`; read bprm->file (a `struct file*`).
    let bprm: *const u8 = ctx.arg(0);
    if bprm.is_null() {
        return 0;
    }
    // Direct load of bprm->file. Because `bprm` is a trusted LSM BTF pointer, the verifier maps
    // offset 64 to the `file*` field and keeps the loaded value a trusted pointer (which
    // `bpf_d_path` requires) — unlike `bpf_probe_read`, which would yield an untyped scalar.
    let file_val = unsafe { *((bprm as usize + BINPRM_FILE_OFF) as *const usize) };
    if file_val == 0 {
        return 0;
    }
    let buf_ptr = match PATHBUF.get_ptr_mut(0) {
        Some(p) => p,
        None => return 0,
    };
    let path_ptr = (file_val + FILE_F_PATH_OFF) as *mut path;
    let ret = unsafe { bpf_d_path(path_ptr, buf_ptr as *mut i8, PATH_MAX as u32) };
    if ret <= 0 {
        return 0; // cannot resolve — do not block
    }
    let plen = resolved_len(ret);
    let buf = unsafe { &*buf_ptr };
    if matches_any(buf, plen, allow) {
        return 0; // executable is allowlisted
    }

    let observe = meta.mode == ScopeMode::Observe as u8;
    emit_audit(cgid, Op::Exec, observe, -EACCES, Some(buf));
    if observe {
        0
    } else {
        -EACCES
    }
}

/// Convert `bpf_d_path`'s return (bytes written, including the trailing NUL) to the path length,
/// clamped into `[0, PATH_MAX-1]` so downstream indexing stays in bounds.
fn resolved_len(ret: i64) -> usize {
    let n = if ret > 0 { ret as usize } else { 0 };
    let n = n.saturating_sub(1); // drop the trailing NUL
    if n < PATH_MAX {
        n
    } else {
        PATH_MAX - 1
    }
}

/// Does `buf` (a resolved path of length `plen`) match `rule`, dispatched on its kind? `plen` comes
/// from `bpf_d_path`'s return value — scanning the 4KB buffer for a NUL blows the verifier's
/// instruction budget, so we never do that. Segment / bounded-star kinds never reach a scope's list
/// (user space fails closed on them), so an unknown kind is treated as no-match.
fn rule_matches(buf: &[u8; PATH_MAX], plen: usize, rule: &DenyRule) -> bool {
    match rule.kind {
        FS_KIND_SUBTREE => match_subtree(buf, plen, rule),
        FS_KIND_POSTFIX => match_postfix(buf, plen, rule),
        _ => false,
    }
}

/// Does any rule in `list` match `buf`? Used for the executable allowlist (pure membership, mode
/// ignored). Bounded, verifier-safe.
fn matches_any(buf: &[u8; PATH_MAX], plen: usize, list: &DenyList) -> bool {
    let count = if (list.count as usize) < DENY_MAX_RULES {
        list.count as usize
    } else {
        DENY_MAX_RULES
    };
    let mut i = 0usize;
    while i < DENY_MAX_RULES {
        if i >= count {
            break;
        }
        if rule_matches(buf, plen, &list.rules[i]) {
            return true;
        }
        i += 1;
    }
    false
}

/// The `file_open` access decision (returns `true` ⇒ block). Verifier-safe, buffer-indexed mirror
/// of `bee_common::matcher::fs_open_blocked` (see there and `contracts/policy.schema.md` / R13 for
/// the precedence).
///
/// The rule list is **pre-sorted most-specific-first (deny breaking ties)** by user space
/// (`bee_common::matcher::rule_sort_key`), so the **first matching rule is the most specific** and
/// this is a single **early-returning** scan — the same shape as the exec allowlist match, which is
/// why it stays within the verifier's 1M-instruction budget. Tracking a "best specificity" scalar
/// across all rules instead (no early return) explodes the verifier state and overruns the budget.
///
/// First matching rule decides: `deny` ⇒ block; a grant ⇒ block iff it does not grant the requested
/// access (read for a read-open, write for a write-open — grants always include read, so reads are
/// only ever blocked by a `deny`). With no matching rule: reads are allowed; writes are denied only
/// where the scope manages its write surface (`write_default_deny`).
fn fs_should_block(
    buf: &[u8; PATH_MAX],
    plen: usize,
    list: &DenyList,
    is_write: bool,
    write_default_deny: bool,
) -> bool {
    let count = if (list.count as usize) < DENY_MAX_RULES {
        list.count as usize
    } else {
        DENY_MAX_RULES
    };

    let mut i = 0usize;
    while i < DENY_MAX_RULES {
        if i >= count {
            break;
        }
        let rule = &list.rules[i];
        if rule_matches(buf, plen, rule) {
            let mode = AccessMode(rule.mode);
            if mode.is_deny() {
                return true;
            }
            let requested = if is_write { AccessMode::WRITE } else { AccessMode::READ };
            return !mode.contains(requested);
        }
        i += 1;
    }

    // No rule matched: allow reads; deny writes only where the scope manages its write surface.
    if !is_write {
        return false;
    }
    write_default_deny
}

/// Path ends with `rule.bytes[..len]` (compiled from `*.ext`). Compares from the end; buffer indices
/// are masked with `& (PATH_MAX-1)` so the verifier can bound the (runtime-derived) access.
fn match_postfix(buf: &[u8; PATH_MAX], plen: usize, rule: &DenyRule) -> bool {
    let rl = rule.len as usize;
    if rl == 0 || rl > DENY_PREFIX_MAX || plen < rl {
        return false;
    }
    let mut i = 0usize;
    while i < DENY_PREFIX_MAX {
        if i >= rl {
            break;
        }
        // plen > i (since plen >= rl > i), so these are non-negative; mask keeps the verifier happy.
        let pidx = (plen - 1 - i) & (PATH_MAX - 1);
        let ridx = (rl - 1 - i) & (DENY_PREFIX_MAX - 1);
        if buf[pidx] != rule.bytes[ridx] {
            return false;
        }
        i += 1;
    }
    true
}

/// Subtree/exact match (verifier-safe): does `buf` start with `rule.bytes[..rule.len]` at a path
/// boundary (exact match or the next byte is `/`)?
fn match_subtree(buf: &[u8; PATH_MAX], plen: usize, rule: &DenyRule) -> bool {
    let len = rule.len as usize;
    if len == 0 || len > DENY_PREFIX_MAX || len > plen {
        return false;
    }
    let mut i = 0usize;
    while i < DENY_PREFIX_MAX {
        if i >= len {
            break;
        }
        if buf[i] != rule.bytes[i] {
            return false;
        }
        i += 1;
    }
    // Boundary: exact match (len == plen) or the next byte is a separator.
    len == plen || buf[len & (PATH_MAX - 1)] == b'/'
}

fn emit_audit(cgid: u64, op: Op, observe: bool, errno: i32, path: Option<&[u8; PATH_MAX]>) {
    if let Some(mut slot) = AUDIT_RB.reserve::<AuditRecord>(0) {
        let pidtgid = bpf_get_current_pid_tgid();
        let mut target = [0u8; TARGET_MAX];
        let mut tlen = 0u16;
        if let Some(p) = path {
            let mut i = 0usize;
            while i < TARGET_MAX - 1 {
                let b = p[i];
                if b == 0 {
                    break;
                }
                target[i] = b;
                i += 1;
            }
            tlen = i as u16;
        }
        let rec = AuditRecord {
            ts_ns: unsafe { bpf_ktime_get_ns() },
            cgroup_id: cgid,
            pid: pidtgid as u32,
            tgid: (pidtgid >> 32) as u32,
            op: op as u8,
            decision: if observe { Decision::Observed as u8 } else { Decision::Denied as u8 },
            _pad: [0; 2],
            errno: if observe { 0 } else { errno },
            target_len: tlen,
            _pad2: [0; 6],
            target,
        };
        // SAFETY: the reserved slot is sized for AuditRecord.
        unsafe { core::ptr::write_unaligned(slot.as_mut_ptr(), rec) };
        slot.submit(0);
    }
}

#[link_section = "license"]
#[used]
static LICENSE: [u8; 4] = *b"GPL\0";

#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
