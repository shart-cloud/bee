//! Verifier-safe path matching primitives.
//!
//! These are the exact operations the eBPF hooks perform on a `bpf_d_path`-resolved path. They are
//! deliberately branch-simple and bounded (no recursion, no backtracking) so the same code compiles
//! under the BPF verifier and is unit-testable on the host (analyze finding I1). All operate on raw
//! byte slices — paths are compared as bytes, never as `str`.

/// Subtree (prefix) match: `path` is `dir` itself or lies beneath it.
///
/// `/a/b` matches dir `/a` and `/a/b` but NOT `/ab`. A trailing slash on `dir` is ignored.
/// `dir == "/"` matches every absolute path.
pub fn subtree_match(path: &[u8], dir: &[u8]) -> bool {
    let dir = trim_trailing_slash(dir);
    if dir.is_empty() {
        // dir was "/" (root) — matches any absolute path.
        return path.first() == Some(&b'/');
    }
    if path.len() < dir.len() {
        return false;
    }
    if &path[..dir.len()] != dir {
        return false;
    }
    // Boundary: exact match, or the next char is a path separator.
    path.len() == dir.len() || path[dir.len()] == b'/'
}

/// Postfix (suffix) match, e.g. `.log` compiled from `*.log`.
#[inline]
pub fn postfix_match(path: &[u8], suffix: &[u8]) -> bool {
    if suffix.is_empty() || path.len() < suffix.len() {
        return false;
    }
    &path[path.len() - suffix.len()..] == suffix
}

#[inline]
fn trim_trailing_slash(p: &[u8]) -> &[u8] {
    // Trim a single trailing '/'. Root ("/") becomes empty, which `subtree_match` treats as
    // "matches any absolute path".
    if !p.is_empty() && *p.last().unwrap() == b'/' {
        &p[..p.len() - 1]
    } else {
        p
    }
}

/// Does a rule of `kind` with pattern `bytes` match the resolved `path`?
///
/// Only the two in-kernel-enforceable kinds are consulted (`SUBTREE`, `POSTFIX`); segment and
/// bounded-star rules never reach a scope's rule list (user space fails closed on them), so any
/// other kind is treated as "no match".
#[inline]
fn rule_matches(path: &[u8], kind: u8, bytes: &[u8]) -> bool {
    match kind {
        crate::layout::FS_KIND_SUBTREE => subtree_match(path, bytes),
        crate::layout::FS_KIND_POSTFIX => postfix_match(path, bytes),
        _ => false,
    }
}

/// Specificity of a matching rule; higher is more specific. Postfix (extension-targeted) rules rank
/// above any subtree rule so an explicit `*.ext` mode governs the file even inside a broader granted
/// directory; within a kind, a longer pattern is more specific.
#[inline]
fn specificity(kind: u8, len: usize) -> u32 {
    match kind {
        crate::layout::FS_KIND_POSTFIX => 0x1_0000 + len as u32,
        _ => len as u32,
    }
}

/// Sort key for a file rule: **sort descending** to order a scope's rule list most-specific-first,
/// with `deny` winning ties at equal specificity. `file_open` then decides by the first matching
/// rule, which lets the kernel scan-and-early-return (bounded verifier state) instead of tracking a
/// running "best" across every rule (which overruns the verifier's instruction budget).
#[inline]
pub fn rule_sort_key(kind: u8, mode: u8, len: usize) -> u32 {
    let deny = if crate::AccessMode(mode).is_deny() {
        1
    } else {
        0
    };
    specificity(kind, len) * 2 + deny
}

/// The `file_open` access decision for one scope, as a reference implementation unit-tested on the
/// host and mirrored (in verifier-safe, buffer-indexed form) by `fs_should_block` in `bee-ebpf`.
///
/// Returns `true` if the open must be blocked. `is_write` is the `FMODE_WRITE` bit of the opened
/// file; `write_default_deny` is the scope's [`crate::FLAG_FS_WRITE_DEFAULT_DENY`] flag.
///
/// **The rule list MUST be pre-sorted most-specific-first (deny breaking ties) by
/// [`rule_sort_key`]**, so the first matching rule is the governing one. Precedence (see
/// `contracts/policy.schema.md` and research R13):
/// - **Most-specific match wins**; a `deny` at equal-or-greater specificity beats a grant.
/// - The first matching rule decides: `deny` ⇒ block; a grant ⇒ block iff it does not grant the
///   requested access (read for a read-open, write for a write-open). Grants always include read, so
///   a read is only ever blocked by a `deny` (allow-by-default reads keep toolchains working); a
///   `read`-only rule nested in a writable subtree blocks writes to it.
/// - **No matching rule**: reads are allowed; writes are denied iff the scope manages its write
///   surface (`write_default_deny`).
pub fn fs_open_blocked(
    path: &[u8],
    list: &crate::layout::DenyList,
    is_write: bool,
    write_default_deny: bool,
) -> bool {
    let count = (list.count as usize).min(crate::layout::DENY_MAX_RULES);

    for rule in &list.rules[..count] {
        if !rule_matches(path, rule.kind, &rule.bytes[..rule.len as usize]) {
            continue;
        }
        let mode = crate::AccessMode(rule.mode);
        if mode.is_deny() {
            return true;
        }
        let requested = if is_write {
            crate::AccessMode::WRITE
        } else {
            crate::AccessMode::READ
        };
        return !mode.contains(requested);
    }

    // No rule matched: allow reads; deny writes only where the scope manages its write surface.
    if !is_write {
        return false;
    }
    write_default_deny
}

/// Does `rule` admit the executable identified by `(ino, dev)` (017 FR-002)?
///
/// Only a pinned rule can answer yes here, and it answers on identity alone. The unpinned rules in
/// the same list are decided by path, elsewhere — the two are deliberately disjoint, so a pin can
/// never be *widened* by the path it happens to carry, and a path rule can never be *narrowed* by an
/// identity it never claimed.
pub fn exec_pin_matches(rule: &crate::layout::DenyRule, ino: u64, dev: u32) -> bool {
    rule.is_pinned() && rule.ino == ino && rule.dev == dev
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pinned(path: &[u8], ino: u64, dev: u32) -> DenyRule {
        let mut r = DenyRule::EMPTY;
        r.len = path.len() as u16;
        r.bytes[..path.len()].copy_from_slice(path);
        r.ino = ino;
        r.dev = dev;
        r
    }

    #[test]
    fn a_pin_is_decided_by_identity_and_not_by_the_path_it_carries() {
        let rule = pinned(b"/usr/bin/opengrep", 4242, 66_306);
        assert!(exec_pin_matches(&rule, 4242, 66_306));
        // The same path, a different file: this is the swap the pin exists to refuse.
        assert!(!exec_pin_matches(&rule, 9999, 66_306));
        // The same inode number on another device is another file entirely.
        assert!(!exec_pin_matches(&rule, 4242, 2049));
    }

    #[test]
    fn an_unpinned_rule_never_matches_on_identity() {
        // Not "matches everything" and not "matches nothing by accident": an unpinned rule is simply
        // not in the identity conversation, and `ino == 0` is what says so.
        let mut rule = pinned(b"/usr/bin/opengrep", 0, 0);
        rule.ino = 0;
        assert!(!rule.is_pinned());
        assert!(!exec_pin_matches(&rule, 0, 0));
        assert!(!exec_pin_matches(&rule, 4242, 66_306));
    }

    #[test]
    fn subtree_boundaries() {
        assert!(subtree_match(b"/a/b", b"/a"));
        assert!(subtree_match(b"/a", b"/a"));
        assert!(subtree_match(b"/a/b/c", b"/a/b"));
        assert!(subtree_match(b"/a/b", b"/a/")); // trailing slash ignored
        assert!(!subtree_match(b"/ab", b"/a")); // not a path-boundary match
        assert!(!subtree_match(b"/a", b"/a/b")); // parent is not under child
    }

    #[test]
    fn subtree_root() {
        assert!(subtree_match(b"/anything/here", b"/"));
        assert!(subtree_match(b"/x", b"/"));
    }

    #[test]
    fn postfix() {
        assert!(postfix_match(b"/var/log/app.log", b".log"));
        assert!(!postfix_match(b"/var/log/app.txt", b".log"));
        assert!(!postfix_match(b".lo", b".log"));
    }

    // ---- fs_open_blocked (file_open access decision) --------------------------------------------
    use crate::layout::{DenyList, DenyRule, FS_KIND_POSTFIX, FS_KIND_SUBTREE};
    use crate::AccessMode;

    /// Build a `DenyList` from `(kind, mode, pattern)` triples, sorted most-specific-first exactly as
    /// user space loads them (the precondition `fs_open_blocked` relies on).
    fn list(rules: &[(u8, AccessMode, &[u8])]) -> DenyList {
        let mut sorted = rules.to_vec();
        sorted.sort_by(|a, b| {
            let ka = rule_sort_key(a.0, a.1 .0, a.2.len().min(crate::layout::DENY_PREFIX_MAX));
            let kb = rule_sort_key(b.0, b.1 .0, b.2.len().min(crate::layout::DENY_PREFIX_MAX));
            kb.cmp(&ka) // descending
        });
        let mut l = DenyList::EMPTY;
        for (i, (kind, mode, bytes)) in sorted.iter().enumerate() {
            let n = bytes.len().min(crate::layout::DENY_PREFIX_MAX);
            l.rules[i] = DenyRule::EMPTY;
            l.rules[i].kind = *kind;
            l.rules[i].mode = mode.0;
            l.rules[i].len = n as u16;
            l.rules[i].bytes[..n].copy_from_slice(&bytes[..n]);
        }
        l.count = sorted.len() as u32;
        l
    }

    const RD: AccessMode = AccessMode::READ;
    const WR: AccessMode = AccessMode(AccessMode::WRITE.0 | AccessMode::READ.0); // write implies read
    const DN: AccessMode = AccessMode::DENY;
    const SUB: u8 = FS_KIND_SUBTREE;
    const PST: u8 = FS_KIND_POSTFIX;

    #[test]
    fn deny_blocks_read_and_write() {
        let l = list(&[(SUB, DN, b"/home/u/.ssh")]);
        assert!(fs_open_blocked(b"/home/u/.ssh/id_rsa", &l, false, false)); // read blocked
        assert!(fs_open_blocked(b"/home/u/.ssh/id_rsa", &l, true, false)); // write blocked
        assert!(!fs_open_blocked(b"/home/u/other", &l, false, false)); // unrelated read ok
    }

    #[test]
    fn read_only_subtree_blocks_writes_allows_reads() {
        // US2 AS-2: a `read`-marked source tree is readable but not writable.
        let l = list(&[(SUB, RD, b"/proj/src")]);
        assert!(!fs_open_blocked(b"/proj/src/lib.rs", &l, false, true)); // read ok
        assert!(fs_open_blocked(b"/proj/src/lib.rs", &l, true, true)); // write blocked
    }

    #[test]
    fn write_grant_allows_write() {
        let l = list(&[(SUB, WR, b"/proj/scratch")]);
        assert!(!fs_open_blocked(b"/proj/scratch/out", &l, true, true));
        assert!(!fs_open_blocked(b"/proj/scratch/out", &l, false, true));
    }

    #[test]
    fn read_nested_in_write_is_most_specific() {
        // `.git` (read) is more specific than the writable project root: writes to `.git` blocked,
        // reads allowed, writes elsewhere in the project allowed.
        let l = list(&[(SUB, WR, b"/proj"), (SUB, RD, b"/proj/.git")]);
        assert!(fs_open_blocked(b"/proj/.git/config", &l, true, true)); // write .git blocked
        assert!(!fs_open_blocked(b"/proj/.git/config", &l, false, true)); // read .git ok
        assert!(!fs_open_blocked(b"/proj/README", &l, true, true)); // write elsewhere ok
                                                                    // Rule order must not matter: same list reversed decides identically.
        let l2 = list(&[(SUB, RD, b"/proj/.git"), (SUB, WR, b"/proj")]);
        assert!(fs_open_blocked(b"/proj/.git/config", &l2, true, true));
        assert!(!fs_open_blocked(b"/proj/README", &l2, true, true));
    }

    #[test]
    fn write_default_deny_gates_unmatched_writes() {
        let l = list(&[(SUB, WR, b"/proj/scratch"), (SUB, RD, b"/proj/src")]);
        // Armed: an unlisted path is read-allowed but write-denied.
        assert!(!fs_open_blocked(b"/tmp/x", &l, false, true));
        assert!(fs_open_blocked(b"/tmp/x", &l, true, true));
        // Unarmed (no write intent): the same unlisted write is allowed.
        assert!(!fs_open_blocked(b"/tmp/x", &l, true, false));
    }

    #[test]
    fn deny_beats_grant_at_equal_specificity() {
        // Explicit read + protected deny on the same path: deny wins (fail-closed).
        let l = list(&[(SUB, RD, b"/home/u/.ssh"), (SUB, DN, b"/home/u/.ssh")]);
        assert!(fs_open_blocked(b"/home/u/.ssh/id_rsa", &l, false, false));
    }

    #[test]
    fn postfix_deny_is_blanket() {
        let l = list(&[(PST, DN, b".secret"), (SUB, WR, b"/proj")]);
        assert!(fs_open_blocked(b"/proj/creds.secret", &l, false, true)); // read blocked everywhere
        assert!(!fs_open_blocked(b"/proj/creds.txt", &l, false, true)); // non-matching read ok
    }

    #[test]
    fn postfix_grant_outranks_subtree() {
        // `*.log` write governs log files even under a read-only project tree.
        let l = list(&[(SUB, RD, b"/proj"), (PST, WR, b".log")]);
        assert!(!fs_open_blocked(b"/proj/app.log", &l, true, true)); // write log ok
        assert!(fs_open_blocked(b"/proj/main.rs", &l, true, true)); // write non-log blocked (read-only)
    }
}
