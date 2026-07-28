//! Shared layouts and matcher primitives for bee.
//!
//! This crate is `no_std` and dependency-light so it can be used from both the eBPF probe
//! crate (`bee-ebpf`) and the user-space loader (`bee-userspace`). It defines:
//!
//! * the `#[repr(C)]` structs that are the on-the-wire layout of the BPF maps (identical on
//!   both sides — see `data-model.md`), and
//! * the **verifier-safe matcher primitives** (`matcher`) that decide whether a resolved path
//!   matches a compiled rule. Keeping this logic here (rather than inside `bee-ebpf`) lets it be
//!   unit-tested on the host with an ordinary `cargo test` (analyze finding I1).

#![no_std]
#![forbid(unsafe_op_in_unsafe_fn)]

pub mod layout;
pub mod matcher;

pub use layout::*;

/// Maximum bytes of a path stored/compared in-kernel. Full `PATH_MAX` for `bpf_d_path` output.
pub const PATH_MAX: usize = 4096;
/// Max bytes of an audit target string (path or "addr:port"), truncated if longer.
pub const TARGET_MAX: usize = 256;

/// Compiled-in kernel struct field offsets, the single source of truth for both the eBPF probe
/// (which reads these fields at constant offsets) and the user-space startup guard (which validates
/// them against the running kernel's BTF and refuses to run on a mismatch — see
/// `bee-userspace::kbtf`). These are per-kernel: a wrong offset reads the wrong field and enforcement
/// fails **open**, so they MUST be validated at load time, never trusted blindly. Values below are
/// for the target kernel (6.8 x86_64). Retarget via `pahole -C <struct> | grep <field>`.
pub mod offsets {
    /// `offsetof(struct file, f_path)` — the `struct path` passed to `bpf_d_path`.
    pub const FILE_F_PATH: usize = 152;
    /// `offsetof(struct file, f_mode)` — `fmode_t` (u32); the requested-access bits.
    pub const FILE_F_MODE: usize = 20;
    /// `offsetof(struct linux_binprm, file)` — the `struct file *` of the program being exec'd.
    pub const BINPRM_FILE: usize = 64;

    // The chain that answers "which file is this, regardless of what it is called" (017). An
    // executable's identity is `(i_ino, s_dev)`, reached from the same `struct file *` the exec hook
    // already holds: file → f_inode → {i_ino, i_sb → s_dev}.
    /// `offsetof(struct file, f_inode)` — the `struct inode *` this file refers to.
    pub const FILE_F_INODE: usize = 168;
    /// `offsetof(struct inode, i_ino)` — the inode number (`unsigned long`, u64 on x86_64).
    pub const INODE_I_INO: usize = 80;
    /// `offsetof(struct inode, i_sb)` — the `struct super_block *` the inode lives on.
    pub const INODE_I_SB: usize = 56;
    /// `offsetof(struct super_block, s_dev)` — `dev_t` (u32), the kernel's device encoding.
    pub const SUPER_BLOCK_S_DEV: usize = 16;

    /// `(struct, member, compiled_offset)` triples the loader validates against the running kernel's
    /// BTF at startup. If any disagrees, bee refuses to enforce (fail-closed).
    pub const VALIDATED: &[(&str, &str, usize)] = &[
        ("file", "f_path", FILE_F_PATH),
        ("file", "f_mode", FILE_F_MODE),
        ("linux_binprm", "file", BINPRM_FILE),
        ("file", "f_inode", FILE_F_INODE),
        ("inode", "i_ino", INODE_I_INO),
        ("inode", "i_sb", INODE_I_SB),
        ("super_block", "s_dev", SUPER_BLOCK_S_DEV),
    ];
}

/// `ScopeMeta.flags` bit: this scope enforces network egress (its policy has ≥1 allow rule).
pub const FLAG_NET_ENFORCED: u8 = 0b0000_0001;
/// `ScopeMeta.flags` bit: this scope's policy declares ≥1 filesystem `write` grant, so it manages
/// its write surface — writes to paths no rule grants are denied (deny-by-default writes). When
/// unset, the scope declares no writable roots and writes are allowed unless an explicit `deny`/`read`
/// rule blocks them (bee is not managing this scope's write surface). See `matcher::fs_open_blocked`.
pub const FLAG_FS_WRITE_DEFAULT_DENY: u8 = 0b0000_0010;

/// File access mode bitflags. `DENY` is a hard sentinel that overrides grants at equal specificity.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct AccessMode(pub u8);

impl AccessMode {
    pub const NONE: AccessMode = AccessMode(0);
    pub const READ: AccessMode = AccessMode(0b0000_0001);
    pub const WRITE: AccessMode = AccessMode(0b0000_0010);
    pub const DENY: AccessMode = AccessMode(0b1000_0000);

    #[inline]
    pub const fn contains(self, other: AccessMode) -> bool {
        (self.0 & other.0) == other.0
    }
    #[inline]
    pub const fn is_deny(self) -> bool {
        self.0 & AccessMode::DENY.0 != 0
    }
    /// Does this mode grant the requested access? `DENY` never grants.
    #[inline]
    pub const fn grants(self, requested: AccessMode) -> bool {
        !self.is_deny() && self.contains(requested)
    }
    #[inline]
    pub const fn union(self, other: AccessMode) -> AccessMode {
        AccessMode(self.0 | other.0)
    }
}

/// Enforcement mode for a scope.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ScopeMode {
    /// Block denied operations (default).
    Enforce = 0,
    /// Allow but emit an "observed" audit record (dry-run, FR-016).
    Observe = 1,
}

/// Operation an LSM hook was evaluating, recorded in an audit event.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Op {
    FileOpen = 0,
    Exec = 1,
    Connect = 2,
    Exfil = 3,
}

/// Outcome recorded in an audit event.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Decision {
    /// Blocked (enforce mode).
    Denied = 0,
    /// Would-deny but allowed (observe / dry-run mode).
    Observed = 1,
}
