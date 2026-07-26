# Phase 0 Research: Harness-Native Security Tooling

Findings that shaped the plan. Measurements were taken on this machine on 2026-07-26 against
`opengrep 1.22.0` and crates.io metadata as of that date.

---

## R1 — Reference harnesses: what to copy, what not to

**Decision**: Copy the *doctrine* from two existing tools, copy neither architecture wholesale.

Two reference implementations were studied.

**deepsec** (`vercel-labs/deepsec`, npm `deepsec@2.2.x`) is a wide, cheap, read-only sieve:
`scan → process → revalidate → enrich → export`. Its unit of work is a **source file**, each
represented by an append-only `FileRecord`. Re-scanning merges candidates; re-processing appends to
`analysisHistory`; revalidation tags findings with a verdict. Its prompt is *assembled from scanner
output* — the rendered prompt carries tech-stack threat highlights and per-slug reviewer notes only
for the slugs the free regex pass actually matched. Its system prompt states plainly:
"**Static analysis only.** Do NOT attempt to reproduce, exploit, or trigger any vulnerability."
It cannot execute, because it has no sandbox.

**defending-code-reference-harness** (Anthropic) is a narrow, deep, execution-verified pipeline:
`build → recon → find ×N → grade → judge → report → dedup → patch`. Its unit of work is a **crash**.
Find agents run in network-isolated gVisor containers and must emit a crashing input file, not a
report. The grader is a separate agent in a fresh container; only the PoC bytes cross the boundary.

**Adopted from deepsec**: append-only records that merge rather than overwrite (→ FR-018);
a separate revalidation pass as the false-positive killer; growing a matcher set only from confirmed
true positives.
**Adopted from DCRH**: a cheap programmatic gate before any expensive agentic one; severity derived
from preconditions rather than category; never letting the model compute a CVSS score (→ FR-005).
**Rejected**: deepsec's file-as-unit-of-work (bee's evidence is behavioural and does not key to a
file) and DCRH's crash-as-unit-of-work (bee is not limited to memory-safety bugs). bee's unit is the
**finding**.

**Alternatives considered**: adopting deepsec's `FileRecord` shape directly — rejected because a
finding produced by a whole-program scanner has no single owning file.

---

## R2 — Native vs. external: the decision rule

**Decision**: Build native when a maintained crate *is* the engine. Shell out when the value is a
curated rule corpus or a heavy non-Rust runtime.

**Rationale**: The criterion that actually discriminates is not implementation difficulty, it is
**who owns the corpus**. Opengrep's engine is not its value; its rule library is, and re-deriving it
would be absurd. ripgrep, ast-grep, and git are the reverse — the engine is a library, and the value
is in how bee drives it.

A second, security-side rationale reinforces it. Every external CLI is an `ExecPolicy.allow` entry,
a binary the policy must trust, and a flag surface where some option shells out — `rg --pre`,
`git -c core.pager=`, `find -exec`. The existing `src/search.rs` already documents this discipline:

> None of ripgrep's command-executing options (`--pre`, `--search-zip`) are exposed — the model
> drives this only through `SearchArgs`, and there is nothing here that runs another program.

Going native keeps the exec allowlist at one inode (bee itself) plus exactly the scanners an
operator deliberately granted (→ SC-006).

**Alternatives considered**: all-native (rejected — means rebuilding rule corpora); all-external
(rejected — reproduces the install-ritual problem this feature exists to remove).

---

## R3 — MSRV conflict: `ast-grep-core` needs 1.88, bee declares 1.85

**Decision**: Keep `[workspace.package] rust-version = "1.85"` for the embeddable core crates, and
override the **application package** (`Cargo.toml:76`) to `rust-version = "1.88"`.

**Measured**:

| Crate | Version | Declared MSRV | Edition | License |
|---|---|---|---|---|
| `ast-grep-core` | 0.45.0 | **1.88.0** | 2024 | MIT |
| `ast-grep-language` | 0.45.0 | **1.88.0** | 2024 | MIT |
| `cvss` | 2.2.0 | 1.85 | 2024 | Apache-2.0 OR MIT |
| `gix` | 0.86.0 | 1.85 | 2024 | MIT OR Apache-2.0 |
| `tree-sitter` | 0.26.11 | 1.77 | 2021 | MIT |
| `serde-sarif` | 0.8.0 | (none) | 2018 | MIT |

**Rationale**: First, nothing is blocked today. `rust-toolchain.toml` declares `channel = "stable"`
(unpinned) and CI installs `dtolnay/rust-toolchain@stable`, so local and CI builds already run 1.96.
No toolchain action is required. `rust-version` is purely a **compatibility promise** to consumers.

Second, that promise does not have to be uniform across the workspace. bee's packages have different
audiences:

| Package | Audience | Floor |
|---|---|---|
| `bee-core`, `bee-common`, `bee-userspace` | embedders — Constitution V's "library that agent frameworks embed" | **1.85**, unchanged |
| `bee` (application) | operators building the binary | **1.88** |

`crates/*/Cargo.toml` each carry `rust-version.workspace = true`, and `Cargo.toml:76` is the `bee`
application package inheriting the same value. Replacing that one line with an explicit
`rust-version = "1.88"` raises the floor exactly where `ast-grep` lives and nowhere else. The
embeddable core keeps its 1.85 promise; the binary states a truthful 1.88.

This is strictly better than a feature-conditional floor, which Cargo cannot express: under that
scheme a 1.85 user enabling `astgrep` gets a confusing dependency-resolution error instead of a
clear "this package requires 1.88".

**Alternatives considered**:
- *Feature-conditional floor, documented only* (the earlier draft of this decision) — rejected:
  unexpressible in the manifest, so the failure mode is a confusing error rather than a clear one.
- *Bumping `[workspace.package]` to 1.88* — rejected: imposes the floor on the embeddable core for
  the sake of an application-only, default-off feature.
- *Pinning an older `ast-grep-core` with a 1.85 floor* — rejected: accepts a stale matcher engine to
  preserve a number.

---

## R4 — Opengrep SARIF is 99.96% rules, and it does not fit stdout

**Decision**: Run external scanners as a **two-child pipeline** — the scanner writes SARIF to a path
inside the scope, then `bee sarif-worker` reads, normalises, and bounds it, also inside the scope.

**Measured** — `opengrep scan --config auto --sarif --quiet` over a directory containing a single
three-line Python file with one `subprocess.call(..., shell=True)`:

| Quantity | Value |
|---|---|
| SARIF version | 2.1.0 |
| Results | 1 |
| Rules embedded in `tool.driver.rules` | 1074 |
| Total document | 1,912,546 bytes |
| The `results` array alone | 839 bytes |
| **Share of document that is rules** | **99.96%** |
| `DEFAULT_OUTPUT_CAP` (`src/transcript.rs:12`) | 102,400 bytes |

The document is **19× the output cap**. Capturing it on stdout would truncate it mid-structure into
unparseable JSON — a failure that would surface as "scanner returned nothing", the exact failure
mode FR-012 exists to prevent.

**Rationale**: Writing to a file and normalising in a second in-scope child solves both problems at
once. Normalisation happens *before* the cap, so what reaches the model is bounded, structured, and
complete-or-explicitly-truncated. And it preserves the invariant `src/search.rs` states — the
harness never opens a target file; a scope-joined child does.

**Alternatives considered**: raising `DEFAULT_OUTPUT_CAP` for scanner tools (rejected — unbounded
model context, and the cap exists for good reasons); having the harness read the SARIF file directly
(rejected — the harness would read around the sandbox, violating Constitution III); streaming stdout
through an incremental parser (rejected — more machinery than writing a file, and still leaves the
harness holding an unbounded buffer).

---

## R5 — SARIF ingestion: minimal local structs, not `serde-sarif`

**Decision**: Define bee's own minimal `serde` structs covering `runs[].results[]`,
`locations[].physicalLocation`, `message.text`, `ruleId`, `level`, and
`invocations[].executionSuccessful`. Ignore everything else by default.

**Rationale**: `serde-sarif` 0.8 models the full 2.1.0 schema faithfully, which means materialising
all 1074 rule objects to reach 839 bytes of results (R4). A minimal struct set ignores unknown
fields for free, adds no dependency, and keeps peak memory proportional to what is actually used.
Schema fidelity is not needed on the ingest path — bee consumes a known subset from known producers.

`serde-sarif` remains reasonable for *validating test fixtures*, where fidelity matters and cost
does not.

**Alternatives considered**: `serde-sarif` on the hot path (rejected as above); hand-rolled
non-serde parsing (rejected — no benefit over serde with ignored fields).

---

## R6 — Opengrep invocation: local rules, and the exit code is not a signal

**Decision**: Invoke as `opengrep scan --sarif --quiet --config <local-path> <target>`. Refuse
`--config auto`. Determine success from `invocations[].executionSuccessful` and a parseable report,
never from the exit status.

**Measured**:
- `--config auto` fetches the rule registry over the network and caches under `~/.opengrep/cli`.
  A scanning scope has no egress, so `auto` cannot work there and must be rejected at argv
  construction rather than failing opaquely at runtime.
- A local rule file (`--config ./local.yml`) works fully offline and produced a 600-byte report for
  the same target — three orders of magnitude smaller than the `auto` run.
- **Exit status was `0` with a finding present.** Opengrep only returns non-zero on findings when
  `--error` is passed. Treating exit status as a findings signal would be wrong in both directions.

**Rationale**: The rule config becomes a typed, validated adapter input: a path, checked to resolve
inside the scope, never a passthrough string. This is the FR-007 discipline applied to the one
argument an operator genuinely needs to vary.

**Alternatives considered**: passing `--error` and reading exit status (rejected — conflates
"findings exist" with "run failed", and FR-012 needs those separated); allowing `--config auto` when
the scope happens to have egress (rejected — makes a security tool's behaviour depend on an
incidental policy detail).

---

## R7 — CodeQL: bundle-based, and compiled languages are out of scope

**Decision**: The CodeQL adapter consumes an operator-provisioned, version-pinned **bundle**, and
supports only `build-mode: none` languages. Traced (compiled) languages are declined explicitly.

**Evidence** — from `codeql-action` v4.37.3 (`src/`):
- `src/defaults.json` pins `"bundleVersion": "codeql-bundle-v2.26.1"`, `"cliVersion": "2.26.1"`.
  The bundle ships the CLI, matching queries, and **precompiled** query packs — which is why GitHub's
  own guidance is to use the bundle rather than a separate CLI plus a `github/codeql` checkout.
- `src/languages/builtin.json` lists `actions, cpp, csharp, go, java, javascript, python, ruby,
  rust, swift`. **`rust` is a builtin language**, so bee can eventually analyse itself.
- `src/util.ts:1081` defines `BuildMode = { None, Autobuild, Manual }`.
- `databaseInitCluster` (`src/codeql.ts:547`) pushes `--begin-tracing`, trap-caching extractor args,
  and `--trace-process-name=<...>` whenever indirect tracing is enabled.
- The lifecycle is `database init --db-cluster --source-root --language=X` → build/extract →
  `database finalize` → `database run-queries` → `database interpret-results --format=sarif-latest
  --output=<file>` (`src/codeql.ts:796`).

**Rationale**: Tracing works by **intercepting the build's process spawns**. That is fundamentally
opposed to a narrow, inode-pinned exec allowlist: it would require admitting every compiler, linker,
and build-tool process the target's build happens to invoke — effectively surrendering SC-006. A
`build-mode: none` analysis needs no such interception, so it fits bee's model cleanly. Half-support
would be worse than none, hence FR-011 and US6 scenario 3 requiring an explicit decline.

The final `interpret-results --output=<file>` step confirms the R4 pipeline generalises: CodeQL, like
Opengrep, naturally writes SARIF to a file rather than a stream.

**Alternatives considered**: supporting compiled languages by widening the allowlist during
extraction (rejected — the widening is unbounded and un-attenuable, violating Constitution II);
running CodeQL outside the scope (rejected — Constitution III).

---

## R8 — Ledger shape: append-only JSONL event log, folded to a view

**Decision**: `.bee/findings/ledger.jsonl` is an append-only log of observation and verdict events.
A derived `.bee/findings/view.json` is the folded current state, regenerable from the log.

**Rationale**: This satisfies four requirements at once and mirrors machinery bee already has (the
audit stream is append-only JSONL, and `crates/userspace` already writes one file per producer to
avoid contention).

| Requirement | How the event log satisfies it |
|---|---|
| FR-018 merge, never overwrite | Folding is the only write path to the view; the log is never rewritten |
| FR-019 no duplicates | Two sightings of one identity fold into one entry with two sighting events |
| FR-020 human verdicts survive | A verdict is an event; automated re-discovery appends a sighting, which cannot outrank it |
| FR-022 concurrent writers | Appends are independent; no reader-modify-write cycle to lose |
| FR-023 human-readable | One JSON object per line, diffable in review |

**Finding identity** is a stable hash of `(normalised path, issue class, normalised message)` and
deliberately **excludes the line number**, so that code movement alone does not mint a duplicate
(spec Assumptions, Edge Case "a finding's location moves"). The line number is an attribute of a
sighting, not of the identity.

**Alternatives considered**: one JSON file per finding, deepsec-style (rejected — a directory churn
per run, and merge becomes a read-modify-write race); a single mutable `findings.json` (rejected —
loses history, and concurrent writers clobber, violating FR-020 and FR-022); SQLite (rejected —
opaque to review, violating Constitution IV).

---

## R9 — Per-language grammar selection is available and cheap

**Decision**: Depend on `ast-grep-language` with `default-features = false`, exposing one bee feature
per language.

**Evidence**: `ast-grep-language` 0.45's `builtin-parser` feature (its default) enables 27
`tree-sitter-*` grammar crates — bash, c, cpp, c-sharp, css, dart, elixir, go, haskell, hcl, html,
java, javascript, json, kotlin, lua, md, nix, php, python, ruby, rust, scala, solidity, swift,
typescript, yaml. Each is an **optional** dependency, so disabling the default and selecting
individually works without patching.

**Rationale**: Every grammar is compiled C. Taking all 27 by default would make build time and binary
size the argument against ever enabling the feature — exactly what SC-009 guards against. `ast-grep-core`'s
own dependency set is lean (`bit-set`, `regex`, `thiserror`, optional `tree-sitter`), so the grammars
are the entire cost.

**Alternatives considered**: `builtin-parser` default (rejected — 27 C grammars for a feature most
builds want for one or two languages); dynamic grammar loading at runtime (rejected — would put a
`dlopen` of an unpinned shared object inside the sandbox, which is a far worse security trade than a
compile-time choice).

---

## R10 — `cvss` crate covers what FR-005 needs

**Decision**: `cvss` 2.2 with `v3` and `v4` features (both on by default).

**Evidence**: `src/lib.rs` documents `v3::Base` for CVSS v3.1 parsing/serialising/scoring and
`v4::Vector` as "a fully-featured implementation of CVSS v4.0". Temporal and Environmental groups for
v3.1 are explicitly a TODO in the crate — Base scoring is what FR-005 requires, so this is
sufficient. MSRV 1.85 matches bee exactly; the crate is `RustCrypto`-adjacent and dual-licensed.

**Rationale**: This is the cheapest requirement in the feature to satisfy correctly, and the one with
the best-documented failure mode if left to the model. The tool takes a vector string, returns score
and qualitative band, and rejects malformed input — the model never supplies a number.

**Alternatives considered**: computing the formula in bee (rejected — reimplementing a specified
floating-point calculation with scope-conditional coefficients for no benefit); letting the model
score (rejected outright by FR-005).

---

## R11 — `gix` blame is available behind a feature

**Decision**: `gix` 0.86 with `default-features = false`, enabling `blame` and `revision`.

**Evidence**: `gix` 0.86's manifest defines `blame`, `revision`, `blob-diff`, `status`, `basic`,
`comfort`, and `max-performance` features. `default` pulls substantially more than history queries
need.

**Rationale**: US5 needs exactly two operations — log for a path, and origin attribution for a line —
which map to `revision` and `blame`. Taking `default` would pull networking and status machinery into
a build that only reads local history.

**Caveat carried to tasks**: `gix`'s API surface churns more across minor versions than the other
crates here. Pin it exactly and expect to touch the call sites on upgrade.

**Alternatives considered**: `git2`/libgit2 (rejected — a C dependency, against the spirit of the
native tier); shelling out to `git` (rejected — puts a binary with `-c core.pager=`-class flag
surface into the exec allowlist for a read-only query).

---

## Open questions carried to `/speckit-tasks`

1. **FR-022 concurrency coverage.** The checklist flagged that the user stories are written
   single-writer. The event log makes concurrent append safe by construction, but the first slice
   should either add an explicit concurrent-append test or record that concurrent episodes writing one
   ledger is out of scope until the `concurrent` feature path needs it.
2. **Ledger location.** `.bee/findings/` inside the analysed project is assumed. If the project under
   analysis is read-only in policy, the ledger needs a configured writable location instead — worth
   confirming against how scenarios already configure workdirs.
