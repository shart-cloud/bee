# `bee run` — the headless session

Status: complete

Part of [Bee application and workspace consolidation](../PRD.md).

## Problem Statement

The headless harness session — the journey ADR-0002 names as primary — is installed as
`bee-episode`, a name that describes an internal unit of work rather than a thing an operator wants
to do. Its startup is also the first of two hand-rolled copies of the same sequence: set the process
non-dumpable, resolve and install the theme, load the provider TOML, read the key from its named
environment variable, build the model, build the scenario, run. `bee-repl` does the same seven steps
in a slightly different order with a different prefix on every error message.

Consolidating the two under one command tree without first extracting that sequence would just move
the duplication. So this issue does both: it introduces the shared session construction and makes
`bee run` its first caller. Issue 04 then makes `bee repl` the second, which is the point at which
the enforcement path for a headless run and an interactive run become the same code rather than two
implementations that happen to agree.

## Solution

Extract session construction into a module in the application package: from an `EffectiveConfig`,
produce the pieces a session needs — non-dumpable process state, installed theme and visual gate,
provider config, model, tool registry, and sandbox. The pieces come from the harness library
unchanged; this is assembly, not new behaviour. `bee-repl`'s copy is the more complete of the two,
so it is the one to read while extracting, but the parts it owns that only an interactive session
needs — skills grant prompting, MCP bridge wiring, front-end selection — stay out until issue 04
places them.

Add `bee run` on top of it, carrying `bee-episode`'s full surface: a scenario file or an ad-hoc task
with its policy, system prompt, tool list, turn limit and timeout; batch mode across a set of
scenarios crossed with a set of providers; the concurrent variant of batch; the output path; the
quiet flag; the theme override. Keep the input expansion behaviour exactly as it is — a
comma-separated list, a directory of TOML files, or a single path — and keep the artifact contract:
the transcript JSON on stdout, progress and diagnostics on stderr, so piping is unaffected.

Keep every exit code: usage errors, model construction failure, transcript infrastructure errors,
API errors, output write errors, and the batch setup-error code. These are an interface; a harness
in someone's CI is reading them.

The application package needs to mirror the harness's feature flags, since the features gate whole
code paths rather than internals. `enforce` already exists there; `concurrent`, `mcp`, and `tui` are
added, each forwarding to the harness feature of the same name. Without `concurrent`, the concurrent
batch flag stays the same explicit error it is today rather than silently running sequentially.

`bee-episode` stays installed and unchanged through this issue. That is what makes the parity tests
possible, and issue 06 removes it once every journey has moved.

## Commits

1. **Add the harness dependency and features.** Depend on `bee-harness` from the application
   package and add the `concurrent`, `mcp`, and `tui` feature forwards alongside the existing
   `enforce`. Verify that `cargo build` with no features still produces a binary free of the async
   runtime paths the harness gates, and that `core_deps_guard` and `mcp_dep_boundary` still pass —
   the application package may pull these in, `bee-core` may not.

2. **Extract session construction.** Move the shared startup sequence into its own module,
   consuming an `EffectiveConfig` and returning the constructed session pieces. Error messages
   become command-neutral — prefixed by the running command rather than a hardcoded binary name.
   No caller yet; unit-test what can be tested without a provider.

3. **Add single-episode `bee run`.** Wire the scenario and ad-hoc-task paths, transcript emission to
   stdout or the output path, progress to stderr, and the single-run exit codes. Port the guard that
   refuses an ad-hoc task without a policy under the enforce feature, which exists because an
   enforced scope must have a real policy and the placeholder path would otherwise surface as a
   confusing parse error at run time.

4. **Add batch mode.** Port scenario/provider expansion, the cross product, per-pair transcript
   output into a directory or a JSON array on stdout, the status grid summary on stderr, and the
   setup-error exit code.

5. **Add concurrent batch.** Port the shared-engine concurrent path behind the `concurrent` feature,
   including the stricter input validation it uses — a load failure aborts the run rather than
   being collected as a per-pair error — and the explicit unavailable-feature error without it.

6. **Add parity tests.** Drive the same inputs through `bee-episode` and `bee run` and assert
   identical transcript JSON and identical exit codes. Cover the single mock-provider episode, the
   ad-hoc task, a batch of two scenarios by two providers, and at least two failure paths (a
   missing provider file and an unparseable scenario). The mock provider makes these deterministic
   and offline.

## Verification

- `cargo test` and `cargo clippy --workspace --all-targets` green, and with `--features concurrent`.
- Parity tests pass — the gate for this issue is that `bee run` and `bee-episode` are
  indistinguishable on identical input, including exit codes and stderr/stdout separation.
- `bee run` piped to a file still yields parseable transcript JSON with no progress lines mixed in.
- The live VM matrix (`test/vm/matrix.sh`) passes. Its episode cases still drive `bee-episode`, so
  this issue should not move them; the point is proving the shared extraction did not disturb the
  enforcement path.

## Outcome

Landed as written.
