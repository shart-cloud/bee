# Absorb `bee-hardening` into `bee-userspace`

Status: ready-for-agent

Part of [Bee application and workspace consolidation](../PRD.md).

## Problem Statement

`bee-hardening` is a package with one source file, one consumer, and no constraint that justifies
its separation. It provides pre-exec and pre-main process hardening — no core dumps, non-dumpable,
`LD_*` stripping — and `bee-userspace`'s spawn path is the only thing that calls it, using two
functions: the environment-key filter and the pre-exec hardening hook.

The other four packages are separate for reasons that pay for themselves. `bee-core` must stay
runtime-free so it can be audited and embedded without a UI toolchain, and there is a test that
enforces it. `bee-common` is `no_std` because the eBPF side shares its layouts. `bee-ebpf` targets a
different architecture on a different toolchain. `bee-hardening` has none of these: it is ordinary
host code using the same syscall crates `bee-userspace` already depends on, sitting one directory
away from its only caller.

It also carries an optional feature that installs a constructor to harden the process before `main`,
for agents embedding bee in-process. Nothing in the workspace enables it. It is a public capability
of the package, and it needs to survive the move as a public capability rather than being quietly
dropped.

## Solution

Move the hardening source into `bee-userspace` as a module, re-exported so the two call sites in the
spawn path change import prefix and nothing else. Carry the optional constructor feature across as
an optional feature of `bee-userspace` with the same name and the same behaviour, so an embedder
that wants pre-main hardening still has a way to ask for it.

Remove the package from the workspace members and from the shared dependency table. Nothing outside
the workspace depends on it — it is unpublished, at `0.1.0`, and referenced only by path.

Move its tests with it. The hardening logic is the kind that is easy to move and hard to notice
breaking, since its failure mode is a process that is slightly less hardened than it claims and
behaves identically otherwise. The tests are the only thing that will notice.

## Commits

1. **Move the source and tests.** Relocate the module into `bee-userspace`, update the two call
   sites in its spawn path, and re-export the public surface so the module is reachable from outside
   the crate. Add the optional constructor feature. Everything else stays byte-identical, including
   the doc comments recording which requirement each behaviour satisfies.

2. **Remove the package.** Delete the directory, the workspace member entry, the shared dependency
   table entry, and the direct path dependency in `bee-userspace`'s manifest. Update the README
   workspace table, which lists the package and its status.

## Verification

- `cargo test` and `cargo clippy --workspace --all-targets` green, and with the constructor feature
  enabled — that feature is not exercised by any default build, so it needs its own compile check.
- The moved tests pass unchanged. Any edit needed beyond the import prefix means something else
  moved with the code and deserves a second look.
- The credential-isolation integration test in the harness passes. It is the end-to-end proof that
  the hardening still applies to spawned tool children, and it exercises the path this issue
  touches from the far end.
- The live VM matrix (`test/vm/matrix.sh`) passes — the spawn path is the enforcement path.
