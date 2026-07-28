# Feature Specification: Enforceable Inode Pins

**Feature Branch**: `017-enforceable-pins`

**Created**: 2026-07-28

**Status**: Draft

**Input**: The 016 VM matrix case T068 could not be written. A scanner grant is by definition an
inode-pinned executable entry (016 FR-008), and the eBPF backend refuses to install any policy
containing one — so `scan` is unreachable in every kernel-enforced episode.

## Overview

bee's policy language can say "this exact file may be executed" — an `exec.allow` entry prefixed
`!`. The compiler carries the pin (`CompiledExec.pin_inode`), user space re-checks it immediately
before spawn, and 016 built the whole external-scanner grant on it: *a binary merely present on
`PATH` is not a grant; only a pinned entry is.*

The kernel cannot express it. `crates/userspace/src/plan.rs` refuses outright:

```
PlanError::UnsupportedInodePin — "eBPF backend cannot enforce inode-pinned executable '<path>'"
```

That refusal is correct fail-closed behaviour for a backend whose exec allowlist matches resolved
paths, and it went unnoticed because the two layers never meet in one test. Measured on the
6.8 BPF-LSM VM, 2026-07-28:

| Policy | Result |
|---|---|
| `exec.allow = ["!…/opengrep"]` | `infra_error: eBPF backend cannot enforce inode-pinned executable` — the episode never starts, in **`enforce` and `observe` alike** |
| `exec.allow = ["…/opengrep"]` | episode completes; `scan` reports `did not run: scanner `opengrep` is not granted to this episode` |

The two are mutually exclusive, so on the enforcing backend the external tier is dead: `scan` runs
only under `--host`, where no plan is built at all. Meanwhile the unit test
`the_compiled_exec_surface_is_bee_plus_the_granted_scanner_and_nothing_else` passes, asserting that
every entry in the compiled allowlist is inode-pinned — the layer above is satisfied by exactly the
policy the layer below rejects.

This feature makes the pin mean something in the kernel: the exec hook reads the identity of the
file actually being executed and matches a pinned rule against **that**, not against its path.

Doing so also makes the pin stronger than the user-space check it replaces. Checking an inode and
then exec'ing a path is a time-of-check/time-of-use window — the file can be swapped in between.
Matching identity at `bprm_check_security` closes it: the decision is made about the very image the
kernel is loading.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - A granted scanner runs under enforcement (Priority: P1) 🎯 MVP

An operator grants Opengrep to a scanning episode with `exec.allow = ["!/usr/bin/opengrep"]` and a
policy in `enforce` mode. The episode starts, the scan runs, findings reach the ledger.

**Why this priority**: it is the whole feature. Without it 016's external tier cannot be used with
enforcement at all.

**Acceptance scenarios**:

1. **Given** a policy whose `exec.allow` contains a pinned entry, **When** the episode starts under
   `enforce`, **Then** the plan installs and the episode runs — no `infra_error`.
2. **Given** that episode, **When** the granted binary is executed, **Then** the exec is permitted.
3. **Given** that episode, **When** any other binary is executed, **Then** the exec is denied.

### User Story 2 - A swapped binary is refused by the kernel (Priority: P2)

Between the grant and the exec, the granted path is replaced with a different file — a rebuilt
binary, a symlink flip, an attacker with write access to the directory. The kernel refuses it.

**Why this priority**: it is what a pin *is*. 016 SC-010 asserts it at the user-space seam; this
moves the guarantee to the arbiter that cannot be raced.

**Acceptance scenarios**:

1. **Given** a pinned grant, **When** the file at that path is replaced after the policy is
   installed, **Then** executing it is denied and audited.
2. **Given** a pinned grant, **When** the same inode is reached by a *different* path (hard link,
   bind mount), **Then** it is permitted — identity, not spelling, is what was granted.

### User Story 3 - The scanner child is inside the scope (Priority: P3)

016's T068: with the grant now installable, an episode's scanner child attempts a read the policy
denies, and the kernel refuses it.

**Why this priority**: the case 016 could not land. It is the only live proof that a third-party
scanner spawned by `scan` is confined like every other tool child rather than running beside the
sandbox.

**Acceptance scenarios**:

1. **Given** an enforcing scan episode, **When** the scanner reads a denied path, **Then** the read
   fails and nothing from it reaches the ledger.
2. **Given** the same setup with the denial removed, **When** the scanner reads that path, **Then**
   it succeeds — the control that proves the case is not vacuous.

### Edge Cases

- **The pinned path does not exist when the policy is compiled.** There is no identity to install, so
  the policy is refused with a diagnostic naming the path. A pin to nothing must never become "allow
  anything at this path".
- **The filesystem reports a device identifier that the kernel's `s_dev` will not match** (btrfs
  subvolumes and some network filesystems synthesise `st_dev`). Enforcement would then deny a binary
  the operator granted. This fails closed — a denied exec, audited — but the diagnostic has to be
  good enough that the operator can tell it from a swap.
- **A pinned entry on a kernel whose struct offsets disagree with the compiled ones.** Already
  handled by the boot-time BTF guard: bee refuses to enforce at all rather than reading a wrong
  field. The new offsets join that guard.
- **More pinned rules than the allowlist holds.** `DENY_MAX_RULES` (8) is unchanged; exceeding it is
  the existing `TooManyRules` refusal.
- **The scanner wants `/dev/null`.** A policy that declares any write grant arms deny-by-default for
  writes, and `/dev/null` is not a path anyone lists — so a scanner child redirecting to it is
  refused. Observed while building the T068 case (the stand-in's own `2>/dev/null` failed, taking its
  compound command with it). Left as-is: it is the write-surface model working as designed, and an
  operator who needs it can grant it. Recorded here because the symptom — a scanner that behaves as
  if a read failed — points nowhere near the cause.
- **Nothing grants bee itself.** The pipeline's second child is `bee sarif-worker`, so an enforcing
  scan policy has to carry bee's own entry alongside the scanner's. This is not new (016's SC-006
  test states the surface as "bee plus the granted scanner"), but `--host` never needed it, so no
  worked example showed it until the matrix case did.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The compiled exec allowlist MUST carry, for each pinned entry, the identity (inode
  number and device) of the file at that path at compile time.
- **FR-002**: The `bprm_check_security` hook MUST read the identity of the image being executed and
  match a pinned rule against it. A pinned rule MUST NOT match on path.
- **FR-003**: An unpinned entry MUST keep matching exactly as it does today (resolved-path subtree),
  so no existing policy changes meaning.
- **FR-004**: A pinned entry whose path cannot be resolved to an identity at compile time MUST be a
  refusal, never a widened rule.
- **FR-005**: The struct offsets the hook uses to reach the identity MUST be part of the boot-time
  BTF validation set, so a mismatch refuses to enforce rather than reading a wrong field.
- **FR-006**: `PlanError::UnsupportedInodePin` MUST cease to exist as a reachable outcome for exec
  rules; a pinned entry is installable.
- **FR-007a**: A resolved scanner grant MUST widen the episode policy by the report directory the
  two-child pipeline exchanges SARIF through (`:project_root/.bee/scan`), and by nothing else.
  `:project_root/.bee` is a default protection, so without this the grant installs and the scan still
  cannot run — child 1's write and child 2's read are both refused. `.bee/findings`, where the ledger
  and its human verdicts live, MUST stay denied.
- **FR-007**: Attenuation MUST treat a pin as it does today — a restriction, so a child naming the
  same executable unpinned inherits the pin, and no sequence of grants can widen past the ceiling.

### Success Criteria *(mandatory)*

- **SC-001**: On the BPF-LSM VM, an `enforce`-mode policy containing a pinned `exec.allow` entry
  installs and the episode completes. (Today: `infra_error`.)
- **SC-002**: On the VM, executing the granted inode is permitted and executing anything else is
  denied, with the denial in the audit trail.
- **SC-003**: On the VM, replacing the file at the granted path after installation makes the exec
  denied — proven against the *same binary* that permits the original, so the difference is the
  file's identity and nothing else.
- **SC-004**: 016 T068 lands: an enforcing scan episode's scanner child is refused an out-of-scope
  read, with a control run proving the read succeeds when permitted.
- **SC-005**: The existing matrix stays green — 35/35 before this feature, 38/38 after (SC-002,
  SC-003, SC-004 each add a case).
- **SC-006**: The eBPF program still verifies with margin on the target kernel; the added identity
  read costs no measurable headroom in the 1M-instruction budget.

## Assumptions

- The target remains 6.8 x86_64 with the compiled offsets validated at boot (`bee_common::offsets`).
  Measured with `pahole` on the VM, 2026-07-28: `file.f_inode` 168, `inode.i_ino` 80, `inode.i_sb`
  56, `super_block.s_dev` 16.
- Device identity is the kernel's `s_dev` encoding (`(major << 20) | minor`), which user space must
  derive from `st_dev` rather than pass through — the two encodings differ.
- Pinning applies to the executable allowlist only. Filesystem rules stay path-matched; a pinned
  *file* rule is a separate question this feature does not open.
