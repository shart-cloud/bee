# Implementation Plan: Harness-Native Security Tooling

**Branch**: `016-native-tools` | **Date**: 2026-07-26 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/016-native-tools/spec.md`

## Summary

Give bee a security-analysis tool surface in two tiers behind one contract.

The **native tier** adds tools whose engine is a maintained Rust crate, each running as a bee
subcommand inside the episode's scope exactly like the existing `search` tool: structural AST
search (`ast-grep-core` 0.45 + `ast-grep-language` 0.45, per-language grammar features),
repository history (`gix` 0.86), and severity scoring (`cvss` 2.2 — the model emits the vector,
the crate computes the number).

The **external tier** adds an adapter shape for scanners whose value is a curated rule corpus:
Opengrep first, a CodeQL bundle later. bee builds argv from typed inputs, the binary enters
`ExecPolicy.allow` as an inode-pinned (`!`-prefixed) entry issued through the existing
grant/attenuation path, and the scanner's SARIF is normalised into bee's finding shape.

Both tiers write into an append-only **finding ledger** — a JSONL event log folded into a
materialised view — so re-runs merge, human verdicts survive, and concurrent episodes append
without a lock.

Two structural decisions carry the security properties. First, an external scanner runs as a
**two-child pipeline**: the scanner writes SARIF to a path inside the scope, then a second
in-scope child (`bee sarif-worker`) reads, normalises, and bounds it. This is forced by
measurement — an Opengrep SARIF over one file is 1.9 MB against a 100 KiB `DEFAULT_OUTPUT_CAP`,
so stdout capture would truncate into unparseable JSON — and it preserves the invariant that the
harness never reads target files around the sandbox. Second, every tool returns a three-state
outcome (`Completed` / `Unavailable` / `Failed`), so "could not run" is structurally
distinguishable from "ran and found nothing" (FR-012, Constitution I).

## Technical Context

**Language/Version**: Rust, stable — already 1.96 locally and in CI (`rust-toolchain.toml` is
unpinned `stable`; CI uses `dtolnay/rust-toolchain@stable`), so no toolchain change is needed.
MSRV is split by audience: `[workspace.package] rust-version` stays **1.85** for the embeddable
core crates, and the **application package** overrides to **1.88** (`ast-grep-core` 0.45's floor).
One line at `Cargo.toml:76`. See research R3.

**Primary Dependencies** (all optional, all in the application package only):
`ast-grep-core` 0.45 + `ast-grep-language` 0.45 (MIT), `gix` 0.86 (`default-features = false`,
`blame` + `revision`), `cvss` 2.2 (`v3` + `v4`), plus `serde`/`serde_json` already present.
`serde-sarif` 0.8 is **rejected for the ingest path** (research R5) and used, if at all, only to
validate test fixtures.

**Storage**: Project-scoped ledger at `.bee/findings/ledger.jsonl` (append-only event log) plus a
derived `.bee/findings/view.json`. Text, diffable, reviewable in a pull request (Constitution IV).

**Testing**: `cargo test` host-side for everything in the first slice — this feature is
deliberately host-testable end to end (SC-011). New integration tests under `tests/`:
`astgrep_tool.rs`, `finding_ledger.rs`, `scanner_adapter.rs`, `cvss_tool.rs`. A follow-on VM case
(`scanner-escape-denied`) proves a scanner child's out-of-scope read is refused by the kernel;
it is not required for the first slice to land.

**Target Platform**: Linux, same as the rest of bee. Native tools function in host (non-`enforce`)
builds with policy mediation absent — which is what makes contributor iteration possible.

**Project Type**: Rust workspace, library-first. All new code is in the application package
(`src/`); `bee-core` and `bee-common` are untouched, enforced structurally by the existing
`tests/core_deps_guard.rs` (extended with the new crates).

**Performance Goals**: A structural search over bee's own tree completes within the same order as
the existing `search` tool. SARIF normalisation is linear in the report and bounded in output.
Ledger folding is linear in event count.

**Constraints**: Tool output obeys `DEFAULT_OUTPUT_CAP` (100 KiB) — normalisation happens
*before* the cap, never after, so truncation never produces unparseable structure. `--config auto`
is refused for Opengrep: it requires network egress, which a scanning scope does not have; rule
configs are operator-provided local paths. No async runtime is required by any native tool.

**Scale/Scope**: First slice is US1–US4 (structural search, ledger, one scanner adapter, severity
scoring). US5 (history) and US6 (CodeQL bundle) are planned here but deferred to follow-on tasks.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

- **I — Deny-by-Default & Fail-Closed** ✅ Every tool returns `Completed | Unavailable | Failed`;
  an absent binary, an ungranted scanner, an inode mismatch, a bundle version mismatch, a timeout,
  a non-zero exit with no parseable report, and a malformed SARIF all resolve to `Unavailable` or
  `Failed`, never to an empty `Completed`. Truncated output is flagged, never silently completed.
  **Gates**: `scanner_adapter.rs` asserts each failure mode is distinguishable from a clean scan;
  a test asserts no code path constructs `Completed` with an empty finding set from an error.
- **II — Capability Attenuation** ✅ A scanner grant is an `ExecPolicy.allow` entry issued through
  the existing derive/ceiling path; it is refused if it reaches outside the ceiling. Discovery of a
  binary on `PATH` confers nothing. **Gate**: a property test that no sequence of scanner grants
  yields an exec allowlist outside the ceiling, reusing the 007 harness.
- **III — Kernel Enforcement Is Authoritative** ✅ Native workers and scanner children both run as
  scope-joined children; the harness never opens a target file. The two-child SARIF pipeline exists
  precisely so normalisation also happens in-scope. **Gate**: host tests assert the harness makes no
  direct read of target paths; the follow-on VM case proves kernel refusal of an out-of-scope read.
- **IV — Policy-as-Data** ✅ The ledger is human-readable append-only JSONL, diffable and reviewable.
  Scanner grants are ordinary policy entries in the existing declarative format — no new imperative
  surface. **Gate**: a test asserts the ledger round-trips through a text diff without loss.
- **V — Library-First, Runtime-Free Core** ✅ All new dependencies are optional and confined to the
  application package. No new crate requires an async runtime. **Gate**: `core_deps_guard.rs`
  extended with `ast-grep-core`, `ast-grep-language`, `gix`, `cvss`, `tree-sitter`.

**Result: PASS.** No violations; Complexity Tracking is empty.

## Project Structure

### Documentation (this feature)

```text
specs/016-native-tools/
├── plan.md              # This file
├── research.md          # Phase 0 output
├── data-model.md        # Phase 1 output
├── quickstart.md        # Phase 1 output
├── contracts/           # Phase 1 output
│   ├── tool-outcome.md      # the three-state result every tool returns
│   ├── finding-ledger.md    # event log, folding, identity, verdicts
│   ├── scanner-adapter.md   # external tier: grant, argv, two-child pipeline
│   └── native-tools.md      # astgrep / gitlog / cvss tool + worker surfaces
├── checklists/
│   └── requirements.md  # spec quality checklist (already written)
└── tasks.md             # Phase 2 output (/speckit-tasks — NOT created here)
```

### Source Code (repository root)

```text
src/
├── tools.rs                   # registry: register new tools + feature gates
├── tools/
│   ├── astgrep.rs             # NEW  structural search tool  → bee astgrep-worker
│   ├── gitlog.rs              # NEW  history tool (US5)      → bee gitlog-worker
│   ├── cvss.rs                # NEW  severity scoring (no worker — no file access)
│   ├── finding.rs             # NEW  record_finding / list_findings tools
│   └── scanner.rs             # NEW  external scanner tool (opengrep, later codeql)
├── astgrep.rs                 # NEW  worker: ast-grep search inside the scope
├── gitlog.rs                  # NEW  worker: gix log/blame inside the scope
├── sarif.rs                   # NEW  worker: SARIF → findings, bounded, inside the scope
├── findings.rs                # NEW  ledger: event log, fold, identity, verdicts
├── scanners/                  # NEW  external tier
│   ├── mod.rs                 #      adapter trait, availability probe, grant check
│   ├── opengrep.rs            #      argv construction + rule-config validation
│   └── codeql.rs              #      bundle probe + version pin (US6, deferred)
└── main.rs                    # + astgrep-worker / gitlog-worker / sarif-worker subcommands

tests/
├── astgrep_tool.rs            # NEW  US1
├── finding_ledger.rs          # NEW  US2
├── scanner_adapter.rs         # NEW  US3
├── cvss_tool.rs               # NEW  US4
└── core_deps_guard.rs         # extended with the new crates (Constitution V)

test/vm/
└── remote-matrix.sh           # + scanner-escape-denied (follow-on, not first slice)
```

**Structure Decision**: Single-project layout, matching every prior bee feature. Each tool follows
the established three-piece shape proven by `search`: a `Tool` impl in `src/tools/`, a worker
module beside `src/search.rs`, and a `*-worker` subcommand in `main.rs`. The external tier gets its
own `src/scanners/` directory because adapters share an availability/grant/argv contract that the
native tools do not need.

### Feature flags

Following the existing `enforce` / `mcp` / `tui` / `concurrent` discipline (FR-006, SC-009):

| Feature | Pulls | Notes |
|---|---|---|
| `findings` | — | The ledger and its tools. No new deps; implied by `scanners`. |
| `astgrep` | `ast-grep-core`, `ast-grep-language` | Default-off. Needs the application package's 1.88 floor (R3). |
| `astgrep-<lang>` | one `tree-sitter-*` grammar | Per-language; `ast-grep-language`'s own `builtin-parser` default is disabled so a build pays only for the grammars it selects. |
| `gitlog` | `gix` (`default-features = false`) | US5. |
| `cvss` | `cvss` | Tiny; gated for consistency and to keep SC-009 honest. |
| `scanners` | `findings` | Adapter tier + SARIF normaliser. No new crates. |
| `sec` | all of the above | Convenience umbrella for a security-tooling build. |

Default features are unchanged, so `SC-009` holds by construction: a build that selects none of
these has a byte-identical dependency graph to today's.

## Complexity Tracking

> No Constitution Check violations. Section intentionally empty.
