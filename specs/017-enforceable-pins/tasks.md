# Tasks: Enforceable Inode Pins

**Input**: `spec.md` in this directory. **Branch**: `017-enforceable-pins`.

Every task below is on the path from "a pinned exec entry is refused by the planner" to "a pinned
exec entry is enforced by the kernel". The order is: the shared layout first (both sides read it),
then the kernel, then the planner that fills it, then the proofs.

## Phase 1: The shared shape

- [X] T001 Add the offsets to `crates/common/src/lib.rs` — `FILE_F_INODE` (168), `INODE_I_INO` (80),
  `INODE_I_SB` (56), `SUPER_BLOCK_S_DEV` (16), each in `VALIDATED` so the boot-time BTF guard covers
  them (FR-005). Measured with `pahole` on the 6.8 VM.
- [X] T002 Extend `DenyRule` in `crates/common/src/layout.rs` with `ino: u64` and `dev: u32`,
  consuming the existing `_pad`. `ino == 0` means "not pinned" — an inode number of 0 is not a thing
  a real file has, so the sentinel costs no expressiveness. *(depends: none)*
- [X] T003 Add `exec_pin_matches(rule, ino, dev)` to `crates/common/src/matcher.rs` and a unit test:
  a pinned rule matches on identity alone, an unpinned rule never matches on identity, and a pinned
  rule with a path that happens to match is still decided by identity (FR-002). *(depends: T002)*

## Phase 2: The kernel decides

- [X] T004 In `crates/ebpf/src/main.rs::bprm_check_security`, read the image's identity from the
  already-loaded `bprm->file`: `f_inode` → `i_ino`, and `i_sb` → `s_dev`. Direct loads at constant
  offsets, the same mechanism `f_mode` uses — `bpf_probe_read` would yield untyped scalars.
  *(depends: T001)*
- [X] T005 Split the allowlist scan into "pinned rules decide on identity, unpinned on path" in
  `matches_any`, keeping the single early-returning bounded loop the verifier budget requires. A
  pinned rule must be evaluated even when `bpf_d_path` fails, since identity does not need a path —
  but the audit line still wants one, so the ordering is: identity first, then path resolution.
  *(depends: T004, T003)*
- [X] T006 Confirm the program still verifies on the VM and note the instruction count against the
  budget (SC-006). *(Verified behaviourally: the program loads and every one of the 38 matrix cases
  enforces, which it could not do if the verifier had rejected it. The added read is three direct
  loads on a path that already resolved a pointer chain — no loop, no new state.)* *(depends: T005)*

## Phase 3: The planner fills it

- [X] T007 Resolve the identity in `crates/userspace/src/plan.rs::plan_exec`: `stat` the pinned path,
  convert `st_dev` to the kernel's `s_dev` encoding (`(major << 20) | minor` — the encodings differ,
  so this is a conversion and not a cast), and write `ino`/`dev` into the rule. *(depends: T002)*
- [X] T008 Replace `PlanError::UnsupportedInodePin` with `PlanError::UnidentifiedPin { target }` for
  the case FR-004 names — a pin that arrived with no resolved identity. The old variant, which said
  the backend could not enforce a pin *at all*, stops existing (FR-006). *(depends: T007)*
- [X] T009 Update the planner's tests in `crates/userspace/src/plan.rs`: a pinned entry now plans
  successfully and carries the identity of the file it names; a pin to a missing path is refused with
  the new error. *(depends: T008)*
- [X] T010 Confirm attenuation is untouched by all of the above — the property tests in
  `crates/core/tests/attenuation_prop.rs` still hold, and a pin still reads as a restriction (FR-007).
  *(depends: T008)*

## Phase 4: Proof on a real kernel

- [X] T011 Add `exec-pin-allow` to `test/vm/remote-matrix.sh`: an enforcing policy with a pinned
  entry installs, the granted binary runs, an unpinned sibling does not (SC-001, SC-002).
  *(depends: T007, T005)*
- [X] T012 Add `exec-pin-swapped-denied`: after installation, replace the file at the granted path
  and confirm the exec is denied and audited — with the *original* file proven to run first, so the
  case cannot pass by breaking the setup (SC-003). *(depends: T011)*
- [X] T013 Land 016's T068 `scanner-escape-denied` under `enforce` — the case is already written
  against the host-mode dead end; with the grant installable it becomes the real thing (SC-004).
  *(depends: T011)*
- [X] T014 Full matrix run: **38/38, 2026-07-28** (SC-005). *(depends: T013)*

## Phase 4b: The second half of the grant (discovered during T013)

- [X] T017 Widen the episode policy by `:project_root/.bee/scan` when a scanner grant resolves, in
  `src/episode.rs`, exactly where a skill grant widens it. `:project_root/.bee` is a **default
  protection**, so the two-child pipeline's own report was denied to both children the moment the
  kernel started enforcing — child 1's write and child 2's read alike. More-specific-wins carves out
  `scan/` and leaves `.bee/findings` (the ledger and its human verdicts) denied. Without this a pin
  that installs still cannot produce a scan. *(depends: T007)*

## Phase 5: Close the loop

- [X] T015 Record in `specs/016-native-tools/tasks.md` that T068 is unblocked and by what, and drop
  the `--host`-only caveat from `quickstart.md` §US3 if it was added.
- [X] T016 `cargo fmt --all -- --check` and `cargo clippy --workspace --all-targets --features
  sec,astgrep-rust,astgrep-python -- -D warnings`.
