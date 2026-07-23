# The `bee` application package and `bee exec`

Status: complete

Part of [Bee application and workspace consolidation](../PRD.md).

## Problem Statement

`bee run --policy P -- COMMAND` runs a single host command inside a scope. It is the workhorse of
the live enforcement matrix — `test/vm/remote-matrix.sh` routes nearly every case through it,
including the segment-glob rejection, the attenuation refusal, and both network destination checks —
and its value is precisely that it exercises enforcement without an LLM anywhere in the picture.

It is also sitting on the name the product needs. ADR-0002 puts the headless harness session at
`bee run`, and nothing else can move until the raw runner has vacated it. Doing that rename first,
on its own, means no later issue has to reason about `run` meaning two things, and no window exists
in which a VM script silently runs the wrong thing.

The package that owns the `bee` binary is `bee-cli`, described in its own manifest as a "command-line
wrapper". Under ADR-0002 it stops being a wrapper and becomes the application — the package every
other host-facing command will land in.

## Solution

Rename the raw runner from `run` to `exec`. Behaviour, flags, and exit codes are unchanged: the
policy and parent-policy arguments, the mode override, the parent cgroup, and the trailing command
after `--` all stay exactly as they are, as do the compile-then-plan-then-attach ordering and the
four exit codes (policy error, unsupported, privileged target, internal). This is a rename plus
help-text reframing, not a rewrite. The command's help describes it as a diagnostic for exercising a
policy against a single host process, so nobody mistakes it for a primary journey.

Reframe `bee-cli` as the application package. Its description changes from "command-line wrapper" to
the application, and its command tree becomes the top-level tree the rest of the consolidation will
grow into: `check`, `validate`, and `exec` today, with `run`, `repl`, and `metrics` arriving in later
issues. Bare `bee` prints help rather than defaulting into any command — starting a session is
always something the operator asked for.

The package keeps its directory name for now. Renaming the directory is part of the workspace
reorganisation and does not belong in a behaviour-adjacent change.

Update every caller in the same commit. There are no compatibility shims: the workspace is `0.1.0`
and unreleased, and a shim for a command nobody has installed yet is pure carrying cost.

## Commits

1. **Rename the subcommand.** Change the `Run` variant to `Exec` in `bee-cli/src/main.rs`, keeping
   `cmd_run`'s body and its `enforce_run` helper intact. Update the help text to describe a
   diagnostic single-command run. Verify the existing `bee-cli/tests/validate.rs` still passes — it
   exercises `check` and `validate`, so it should be untouched by this.

2. **Add command-surface tests.** Extend `bee-cli/tests/validate.rs` (or add a sibling) with tests
   that assert the top-level command tree: bare `bee` exits with help, `bee run` is not a
   recognised subcommand, and `bee exec` without a policy fails as a usage error. These are the
   tests that will catch a later issue accidentally reintroducing the old name.

3. **Update the VM matrix.** Change the five `bee run` invocations in `test/vm/remote-matrix.sh` —
   the `run_case` helper and the segment-glob, attenuation-refusal, and two network-destination
   cases — to `bee exec`. The `bee check` invocation is unaffected.

4. **Reframe the package.** Update the `description` in `bee-cli/Cargo.toml` and the module doc
   comment at the top of `bee-cli/src/main.rs` to describe the application rather than a wrapper.
   Update the `bee-cli` row in the README workspace table and the CLI example block, which currently
   shows `bee check` and `bee validate` and should now also show `bee exec`.

## Verification

- `cargo test` and `cargo clippy --workspace --all-targets` green.
- `cargo run --bin bee` prints help and exits without running anything.
- `cargo run --bin bee -- validate --policy policies/cargo-test.toml` still succeeds — proof the
  rename touched nothing but the one subcommand.
- The live VM matrix (`test/vm/matrix.sh`) passes with the same case results as before the rename.
  This is the real gate: the matrix is the only thing that proves `exec` still enforces.

## Outcome

Landed as written.
