# Retire the old binaries

Status: ready-for-agent

Part of [Bee application and workspace consolidation](../PRD.md).

## Problem Statement

After issues 03 and 04, every journey exists twice. `bee run` and `bee-episode` produce identical
transcripts; `bee repl` and `bee-repl` run identical sessions; the parity tests exist precisely to
prove it. That duplication was the point during the migration and is pure cost after it: two
installed names per journey, two sets of documentation examples, and a standing invitation for the
next change to land in only one of them.

`bee-metrics` is the one journey that has not moved. It is small — read the append-only JSONL log,
filter by project, model, or date, print the summary or the raw records — and it has no session, no
provider, and no sandbox. It is the last thing standing between the workspace and a single installed
executable.

The documentation debt is spread across two kinds of file. Living documents — the two READMEs, the
design-system doc, and the VM scripts — describe how bee works now and must be correct. The
`specs/00*/` quickstarts are records of what a feature shipped as, at the time it shipped; rewriting
them would falsify the record.

## Solution

Fold `bee-metrics` into `bee metrics`, carrying its flags unchanged: the log path override, the
project, model and date filters, the raw-JSON output mode, and the missing-path error. The default
path stays `~/.local/state/bee/metrics/events.jsonl` — that is state, and issue 02 deliberately left
it where it is.

Then delete the three binary targets from the harness manifest. The harness keeps its library, which
is where all of this behaviour lives; what goes away is three `[[bin]]` entries and three thin
`main` functions whose contents have already moved.

Update the living documents to describe the single executable, and give the historical spec
quickstarts a one-line note at the top pointing at the command rename rather than editing their
bodies. A reader arriving at a quickstart from a search result learns immediately that the command
names changed, and the record of what the feature shipped as stays intact.

## Commits

1. **Add `bee metrics`.** Port the flags, filters, output modes, and exit codes. Test the summary
   and raw-JSON paths against a fixture log, and test the error when no path can be determined.

2. **Delete the binary targets.** Remove the three `[[bin]]` entries from `bee-harness/Cargo.toml`
   and their source files under `bee-harness/src/bin/`. The parity tests from issues 03 and 04 go
   with them — they compare against binaries that no longer exist. Replace each with a
   single-command test asserting the surviving behaviour, so the coverage those tests provided is
   not simply dropped.

3. **Update the VM scripts.** `test/vm/matrix.sh` builds and ships a separate episode binary and
   makes both executable; `test/vm/remote-matrix.sh` locates it, skips cases when it is absent, and
   reports its absence in three places. Collapse all of it onto the single `bee` binary, which is
   already being built and shipped by the same script. This simplifies the matrix rather than
   merely renaming things in it.

4. **Update the living documents.** The README's workspace table, its CLI example block, and its
   full-screen TUI section; `bee-harness/README.md`; `docs/design-system.md`. Describe one
   executable with subcommands.

5. **Annotate the historical quickstarts.** Add a one-line note to each spec document that shows a
   retired command — the quickstarts under `specs/002`, `specs/003` (both), `specs/004`,
   `specs/008`, and `specs/009`, plus `specs/008-grid-tui/contracts/modes-and-cli.md` — recording
   that the commands they show were renamed by this consolidation, with the mapping. Do not edit
   their bodies.

## Verification

- `cargo test` and `cargo clippy --workspace --all-targets` green across the default, `enforce`,
  `concurrent`, `mcp`, and `tui` feature configurations.
- `cargo build --release` produces exactly one installed host executable. This is the issue's
  headline claim and worth asserting by listing the built binaries, not by inspection.
- Every command in the README and the two other living documents runs as written against a mock
  provider.
- The live VM matrix (`test/vm/matrix.sh`) passes with the same case results, now driving the single
  binary — including the concurrent-audit-isolation case, which previously skipped when the separate
  episode binary was missing or built without the concurrent feature.
