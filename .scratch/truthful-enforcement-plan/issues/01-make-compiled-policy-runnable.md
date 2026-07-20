# Make compiled Policy mean runnable

Status: complete

## Problem Statement

Bee currently calls the result of policy compilation a runtime representation, but that result can contain capabilities the eBPF backend cannot install. Segment and bounded-star filesystem rules compile successfully and fail only during Scope creation. Inode-pinned executables are accepted and represented but silently installed as ordinary path rules. Exfiltration settings are compiled but not enforced. The validation command therefore can report that a policy is valid even though running the same policy will fail later or omit requested behavior.

Backend constraints also leak into Scope creation: supported rule kinds, fixed map capacity, precedence ordering, write-default behavior, and map encoding are all discovered or assembled after compilation. This gives callers no small, truthful interface representing “safe to install on this backend.”

The desired outcome is a clean separation between a backend-neutral compiled Policy and an eBPF-specific Enforcement Plan. Compilation should normalize and resolve author intent without depending on an enforcement backend. Enforcement planning should either produce a complete, deterministic, installable plan or return a precise error before any kernel or cgroup state is touched.

## Solution

Keep Policy compilation in the runtime-free core and make its output explicitly backend-neutral. Rename the current compiled result so it no longer claims to be directly loadable. Preserve all author intent required for later backend decisions, including whether exfiltration was enabled and whether executable pinning was requested.

Add an eBPF-specific Enforcement Plan module in the userspace crate. It will be the sole module that knows the current backend's supported filesystem rule kinds, fixed capacities, precedence ordering, default-write flag, executable limitations, network map entries, and unsupported capabilities. Its interface will accept a compiled Policy plus the effective execution mode and return either a complete immutable plan or a typed planning error.

Scope installation will consume an Enforcement Plan. It will no longer inspect or reinterpret backend-neutral policy primitives. The validation command will compile and plan, including when validating an attenuated child, so a successful validation means the selected eBPF backend can run the effective policy.

Requested but unsupported capabilities will fail closed. This includes segment and bounded-star rules, inode-pinned executable rules, and enabled exfiltration detection. Disabled exfiltration metadata may remain in the compiled Policy because it requests no runtime behavior. Examples and documentation will stop claiming unsupported capabilities.

## Commits

1. **Characterize the backend-neutral compiler.** Add passing tests that record how supported prefixes and postfixes, unsupported-but-normalizable glob kinds, executable pin intent, network destinations, protected defaults, and exfiltration settings are represented after compilation. These tests establish the semantic input to enforcement planning without changing behavior.

2. **Rename the compiled policy result.** Replace the runtime-oriented `PolicySet` name with `CompiledPolicy` across the workspace. Keep behavior identical. Update public exports and documentation to describe it as normalized, resolved author intent rather than a map-ready value.

3. **Preserve exfiltration intent during compilation.** Extend the compiled representation so it retains whether exfiltration detection was enabled, not merely the list of sensitive paths. Add compiler tests for enabled, disabled, empty, and populated configurations. No enforcement behavior changes yet.

4. **Introduce the Enforcement Plan module and typed planning errors.** Add the immutable plan type, effective execution-mode input, and error categories without migrating callers. Support an empty/minimal compiled Policy first. Add tests proving a successful plan carries the effective mode and contains no implicit capabilities.

5. **Plan supported filesystem rules.** Move filesystem rule encoding, most-specific-first ordering, deny tie-breaking, and write-default flag calculation into the planner. Test externally visible plan behavior: prefix and postfix rules encode deterministically, nested precedence is preserved, deny wins an equal-specificity tie, and write intent controls the default-write flag.

6. **Reject unsupported filesystem capabilities during planning.** Return typed errors for segment and bounded-star rules. Include the rule and capability kind in diagnostics without leaking internal map details. Test each unsupported kind independently and in a mixed policy where an unsupported rule appears after valid rules.

7. **Enforce filesystem capacity during planning.** Move the fixed rule-capacity check into the planner and test the exact limit, one below it, and one above it. Account for injected protected defaults so errors describe the effective rule count rather than only author-written entries.

8. **Plan executable rules truthfully.** Encode ordinary executable path rules in the plan and reject every inode-pinned request until inode enforcement exists. Test ordinary paths, multiple paths, capacity limits, and pinning rejection. Remove any path that could silently discard the pinning request.

9. **Plan network rules and reject enabled exfiltration.** Encode resolved IPv4 and IPv6 destinations deterministically, set the network-enforced flag from the resulting entries, and reject exfiltration whenever the Policy requests it. Test empty network policy, both address families, duplicate resolved destinations if relevant to current semantics, disabled exfiltration metadata, and enabled exfiltration rejection.

10. **Give Scope installation a plan-based entry point.** Add an installation path that consumes only a completed Enforcement Plan. Move map population to read encoded plan data without inspecting compiled policy primitives. Keep the existing compiled-policy entry point temporarily as a compatibility delegate that plans before touching the kernel. Verify all host tests remain green.

11. **Make validation mean runnable.** Change both ordinary validation and parent/child attenuation validation to compile the effective Policy and build an Enforcement Plan. Add command-level tests proving supported policies succeed and each unsupported capability fails with a policy error before kernel support is probed.

12. **Migrate direct command execution.** Have the command runner build the Enforcement Plan explicitly before initializing the Engine. Pass the completed plan to Scope installation. Remove the duplicate compilation currently performed inside the execution path. Keep exit-code behavior stable except that unsupported capabilities now return the policy-error code earlier.

13. **Migrate single Episode and REPL execution.** Make both harness entry points compile and plan before creating an enforced Sandbox. Preserve host-only behavior, credential stripping, diagnostics, and transcript error classification. Add or adjust host tests for planning errors without requiring a live kernel.

14. **Migrate concurrent Episode execution.** Build each episode's Enforcement Plan during sequential setup, before creating its Scope. Preserve per-episode error isolation and input ordering. Add a concurrent test containing one unsupported Policy and valid peers to prove only the invalid episode fails.

15. **Remove the compiled-policy Scope entry point.** Once every caller supplies an Enforcement Plan, delete the compatibility delegate and any map-encoding branches that still inspect compiled policy primitives. Scope installation should have one interface for policy installation.

16. **Delete dormant kernel representations.** Remove unused map layouts, rule-kind constants, matcher functions, and public core Scope lifecycle types that represent no installed behavior. Keep backend-neutral compiled variants that are intentionally rejected by the current planner, since another backend may eventually support them. Confirm the shared kernel crate now exposes only layouts used by an actual map or hook.

17. **Remove false dependency edges and stale exports exposed by this refactor.** Prune dependencies and re-exports that no longer have callers after the plan migration. Keep this commit mechanical; do not fold in the separate hardening-authority or harness-interface refactors.

18. **Align examples and living documentation.** Update example Policies to request only capabilities the current backend can honor. Correct the feature table and validation documentation so backend-neutral compilation, eBPF planning, and kernel installation are distinct stages. Mark segment globbing, bounded-star globbing, executable pinning, and exfiltration as rejected until implemented rather than merely deferred or silently ignored.

19. **Run the complete verification gate.** Run formatting, all workspace host tests, linting in default and enforcement modes, the enforcement-feature release build, and the full VM matrix. Record the results in the issue comments. Do not close the refactor if a supported Policy changes kernel behavior or if any unsupported capability reaches Scope creation.

## Decision Document

- Policy compilation remains backend-neutral and runtime-free.
- The backend-neutral compiled result is called Compiled Policy and does not claim installability.
- The eBPF-specific Enforcement Plan is the only representation that means “ready to install.”
- The planner owns backend capability checks, fixed capacity, rule precedence, effective flags, and map-ready encoding.
- Planning is pure and occurs before Engine initialization or cgroup creation.
- Scope installation consumes a completed Enforcement Plan and does not reinterpret Policy semantics.
- Breaking public-interface changes are explicitly allowed; no compatibility shim remains at the end.
- Validation performs authoring validation, attenuation when requested, backend-neutral compilation, and eBPF planning.
- Successful validation means the effective Policy is runnable by the current eBPF backend.
- Segment filesystem rules are normalized by the core but rejected by the current eBPF planner.
- Bounded-star filesystem rules are normalized by the core but rejected by the current eBPF planner.
- Inode-pinned executable requests are retained as author intent but rejected until inode enforcement exists.
- Enabled exfiltration detection is rejected until an enforcing implementation exists.
- Disabled exfiltration metadata may remain because it requests no runtime behavior.
- Existing supported prefix, postfix, executable-path, and network behavior must not change.
- Transactional Scope lifetime, rollback, and Drop ownership are a separate refactor.

## Testing Decisions

- Tests assert behavior at the compiler, planner, command, and kernel interfaces rather than private helper structure.
- Compiler tests prove normalization and preservation of author intent without asserting eBPF support.
- Planner tests prove that every successful plan is complete and deterministic, and every unsupported request returns a typed error.
- Filesystem planner tests cover precedence, deny ties, protected defaults, write-default behavior, and the exact capacity limit.
- Executable planner tests prove path rules are encoded and pinning cannot be silently weakened.
- Network planner tests cover IPv4, IPv6, empty policy, and deterministic entries.
- Exfiltration tests distinguish disabled metadata from enabled unsupported behavior.
- Command-level tests invoke validation as a user would and assert exit category and diagnostic meaning.
- Harness tests use the existing Mock Model and host Sandbox seams to verify setup-error isolation without a BPF-capable host.
- Existing compiler, attenuation, matcher, Episode, batch, concurrent, and credential-isolation tests are prior art and remain regression coverage.
- The full VM matrix is a required completion gate because map encoding and the policy-to-kernel path change even though intended enforcement semantics do not.
- A good test observes accepted or rejected capability behavior through a module interface; it does not assert private field layout unless that layout is the shared kernel ABI itself.

## Out of Scope

- Implementing segment or bounded-star matching in eBPF.
- Implementing inode-pinned executable enforcement.
- Implementing exfiltration detection.
- Adding a second enforcement backend.
- Redesigning attenuation semantics.
- Changing filesystem precedence or deny-by-default behavior.
- Transactional Scope rollback, automatic Drop teardown, or audit ownership redesign.
- Unifying Episode and REPL Agent Turn logic.
- Consolidating command-line binaries.
- Reorganizing or merging crates solely to reduce folder count.
- General cleanup unrelated to truthful policy planning.

## Further Notes

The existing crate split should remain intact. The runtime-free core, userspace kernel adapter, shared kernel ABI, and eBPF target each satisfy a real seam. This refactor cleans inward by making the interfaces truthful rather than moving directories.

The deletion test guides cleanup: removing dormant kernel layouts and unused lifecycle types loses no implemented behavior, while removing backend-neutral Policy compilation or Scope installation would force their complexity into every caller. The former should be deleted; the latter should become deeper modules.

The recommended first implementation move is the mechanical rename and intent-preservation work. It sharpens the language before introducing the new planner and keeps the first commits easy to review.

## Comments

Implemented on 2026-07-20.

- Renamed the backend-neutral result to `CompiledPolicy` and preserved exfiltration and inode-pin intent.
- Added the pure eBPF `EnforcementPlan` with typed, fail-closed capability and capacity errors.
- Changed every Scope caller to plan before Engine initialization and made Scope installation consume only a completed plan.
- Added compiler, planner, CLI, and concurrent setup-isolation coverage.
- Removed dormant shared-kernel layouts, matcher helpers, lifecycle types, exports, and dependency edges.
- Updated examples, contracts, the data model, README, and research decisions to describe the two-stage compile/plan boundary truthfully.
- Corrected the VM gate so expected planner rejection uses the new diagnostic contract and the mandatory concurrent audit case is built rather than skipped.

Verification:

- `cargo test --workspace --tests`: passed.
- `cargo test -p bee-harness --features concurrent`: passed, including invalid-policy isolation before Engine initialization.
- `cargo clippy --workspace --all-targets`: passed without warnings.
- `cargo clippy -p bee-harness --all-targets --features concurrent`: passed without warnings.
- Enforcement and concurrent feature checks passed.
- Enforcement release builds passed for `bee` and concurrent `bee-episode`.
- `bee validate --policy policies/cargo-test.toml`: reported valid and runnable.
- `bash test/vm/matrix.sh`: 24 passed, 0 failed, 0 skipped.
- Every edited Rust file was formatted directly. The repository-wide formatter check still reports pre-existing drift in untouched files, so unrelated files were not bulk-formatted.
- No retired Rust symbols remain referenced.

The workspace has no usable Git metadata (`git status` reports that it is not a repository), so the planned tiny commits could not be created; the implementation is present directly in the working tree.
