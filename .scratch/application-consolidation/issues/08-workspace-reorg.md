# Workspace reorganisation

Status: complete

Part of [Bee application and workspace consolidation](../PRD.md).

## Problem Statement

After issue 07 the workspace has five packages and one installed executable, but the layout still
reads like the old shape: six sibling `bee-*` directories at the repository root, one of which is
the application, one of which is a library that only the application uses, and one of which is not a
workspace member at all. Nothing about the arrangement tells a reader which of them is the product.

Two smaller things travel with the layout. The application's behaviour is split across two packages
— `bee-cli` owns the commands, `bee-harness` owns everything they do — and that split has no
remaining justification now that the harness library has exactly one consumer. And the BPF artifact
is a binary target named `bee`, so the object copied out of the eBPF build and embedded into
`bee-userspace` is a file called `bee` that is not the `bee` anyone installs. That was tolerable
when the host command surface was four binaries; it is confusing when there is exactly one.

This is a mechanical change, deliberately sequenced last. Everything before it altered behaviour and
was reviewable by reading a diff; this one moves thousands of lines without altering any behaviour,
and mixing the two would make both unreviewable.

## Solution

Adopt the target layout: the root manifest becomes both the workspace root and the `bee` application
package, with the application source at `src/`, and the remaining packages move to `crates/core`,
`crates/userspace`, `crates/common`, and `crates/ebpf`. The eBPF crate stays excluded from the
workspace and built out of band by the userspace build script, for the same reason as today — a
different target on a different toolchain would otherwise break every ordinary host build.

Merge the harness library into the application package. Its source, its roughly thirty integration
tests, its fixtures, and its feature flags all move; the root `tests/` directories already exist and
are empty. The features keep their names and their gating, because they are how the default build
stays free of a terminal backend, an MCP client, and the BPF toolchain.

Rename the BPF binary target from `bee` to `bee-lsm`, updating the object path the build script
copies from, the file name it copies to, and the embed in the loader. Three coordinated edits, and
the confusion goes away permanently.

Move files with `git mv` so history follows them. A reviewer should be able to confirm this issue by
reading the manifest diffs and the path fixes, and by trusting the test suite for the rest.

## Commits

1. **Move the leaf packages.** `git mv` `bee-core`, `bee-common`, and `bee-userspace` under
   `crates/`, updating the workspace members, the shared dependency table paths, and the relative
   path in the userspace build script that reaches the eBPF crate.

2. **Move the eBPF package and rename its artifact.** `git mv` it to `crates/ebpf`, update the
   workspace exclusion and the build script's path to it, and rename its binary target to `bee-lsm`
   — which means the object path the build script copies from, the name it copies into the output
   directory, and the embed in `bee-userspace`'s loader all change together. This is the one commit
   here that can fail at link time rather than compile time, so it wants building with the
   enforcement feature before moving on.

3. **Fix the dependency-boundary guards.** `core_deps_guard` resolves manifests by relative path and
   `mcp_dep_boundary` queries the dependency tree by package name. Both need updating for the new
   paths, and both must still fail when a forbidden dependency is introduced — verify by
   temporarily adding one, not by assuming.

4. **Promote the application to the root package.** Make the root manifest both the workspace root
   and the `bee` package, move the application source to `src/`, and merge the harness library into
   it: source, tests, fixtures, and feature flags. The harness package disappears; its features
   become the application's features.

5. **Sweep the references.** Doc comments across the tree name the old paths and package names,
   including several in `xtask` that explain, at length, why that crate deliberately does not depend
   on the harness. That reasoning is still correct and its wording needs updating rather than
   deleting. Update the README workspace table to the five-package layout.

## Verification

- `cargo test` and `cargo clippy --workspace --all-targets` green across the default, `enforce`,
  `concurrent`, `mcp`, and `tui` feature configurations. Every test that passed before this issue
  passes after it, with no test edited except for path constants — that equivalence is the whole
  claim.
- `cargo build --features enforce` succeeds on the nightly BPF toolchain, proving the renamed
  artifact is found, copied, and embedded.
- `cargo xtask viz-snapshot` still runs — `xtask` is its own workspace with its own lockfile and
  should be entirely unaffected, which is worth confirming rather than assuming.
- `git log --follow` on a moved file shows its history, confirming the moves were recorded as moves.
- The live VM matrix (`test/vm/matrix.sh`) passes.

## Outcome

Landed. Also fixed a pre-existing break: `src/concurrent.rs` had a test helper missing two `ProviderConfig` fields added in 008, so `cargo test --features concurrent` had not compiled since. Unrelated to the move, fixed in passing because it blocked the feature-matrix verification.
