---
description: "Task list for 016-native-tools implementation"
---

# Tasks: Harness-Native Security Tooling

**Input**: Design documents from `/specs/016-native-tools/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md,
contracts/{tool-outcome,finding-ledger,scanner-adapter,native-tools}.md, quickstart.md

**Tests**: REQUIRED — the constitution mandates test-first for the security boundary
("Enforcement behavior … MUST have tests written and failing before implementation", "A denial that
no test exercises is not considered enforced"). Test tasks precede their implementation within each
phase.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: parallelizable (different files, no dependency on an incomplete task)
- **[Story]**: US1–US6 for user-story phases; Setup/Foundational/Polish carry no story label
- Paths are repo-relative.

**Scope note**: the **first slice is US1–US4** (plus Setup, Foundational, Polish) and is entirely
host-testable (SC-011). **US5 and US6 are deferred** — specified and planned, not built now.

---

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Manifest changes and module scaffolding. No behaviour yet.

- [X] T001 Override the application package MSRV — replace `rust-version.workspace = true` with `rust-version = "1.88"` in the `[package]` section of `Cargo.toml` (line 76), leaving `[workspace.package] rust-version = "1.85"` untouched so the embeddable core crates keep their floor (research R3).
- [X] T002 [P] Add optional dependencies to `Cargo.toml`: `ast-grep-core = { version = "0.45", optional = true }`, `ast-grep-language = { version = "0.45", default-features = false, optional = true }`, `gix = { version = "0.86", default-features = false, features = ["blame", "revision"], optional = true }`, `cvss = { version = "2.2", optional = true }`. Pin `gix` exactly — its API churns across minor versions (research R11).
- [X] T003 [P] Add the feature table to `Cargo.toml` per plan.md §Feature flags: `findings`, `astgrep`, `astgrep-<lang>` (one per grammar, forwarding to `ast-grep-language/tree-sitter-<lang>`), `gitlog`, `cvss`, `scanners` (implies `findings`), and the `sec` umbrella. Leave `default` unchanged.
- [X] T004 Create module scaffolding (stubs, `#[cfg(feature = …)]`-gated): `src/findings.rs`, `src/astgrep.rs`, `src/sarif.rs`, `src/scanners/mod.rs`, `src/scanners/opengrep.rs`, `src/tools/astgrep.rs`, `src/tools/cvss.rs`, `src/tools/finding.rs`, `src/tools/scanner.rs`; register each in `src/lib.rs` and `src/tools.rs`. *(depends: T002, T003)*
- [X] T005 [P] Extend the `FORBIDDEN` list in `tests/core_deps_guard.rs` with `ast-grep-core`, `ast-grep-language`, `tree-sitter`, `gix`, and `cvss` so none can reach `bee-core`/`bee-common` (Constitution V gate).

**Checkpoint**: `cargo build` (default) unchanged; `cargo build --features sec` compiles stubs.

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: The fail-closed outcome type and tool-registration machinery every story depends on.
**⚠️ No user story can start until this phase completes.**

### The outcome type (contract `tool-outcome.md`)

- [X] T006 Define `ToolOutcome<T>` (`Completed { value, truncated }` / `Unavailable { reason }` / `Failed { reason }`) and `UnavailableReason` with all seven variants in a new `src/tools/outcome.rs`, re-exported from `src/tools.rs`.
- [X] T007 Implement `ToolOutcome<T> → ToolResult` rendering in `src/tools/outcome.rs` per the mapping table in `contracts/tool-outcome.md`. A clean scan renders `scanned N files, no findings`; `Unavailable` renders `did not run: <reason>`; `Failed` renders `failed: <reason>`. When `ToolOutcome::truncated`, the truncation notice is the **first** line, before results (FR-013). *(depends: T006)*
- [X] T008 Emit a structured audit event on every `Unavailable` and `Failed` before returning, carrying tool name and reason variant; include the path for `PinMismatch` and the scanner name for `NotGranted`, since those are the security-relevant refusals rather than mere absence (FR-014). *(depends: T006)*

### The fail-closed gate (Constitution I — write these failing first)

- [X] T009 [P] Write failing tests in `tests/fail_closed_tools.rs` asserting that every `UnavailableReason` variant renders distinguishably from a clean scan, and that no rendering of `Unavailable`/`Failed` contains the clean-scan phrase (SC-002, FR-012).
- [X] T010 [P] Write a failing test in `tests/fail_closed_tools.rs` asserting `ToolOutcome::Completed` is never constructed from an error path — assert structurally by confirming each error branch in the scanner and worker paths yields `Unavailable` or `Failed` (Constitution I gate).

### Registration and feature gating

- [X] T011 Add `SEC_TOOLS` (`ast_grep`, `git_log`, `cvss`, `record_finding`, `list_findings`) and `SCANNER_TOOLS` (`scan`) constants to `src/tools.rs`, following the `RENDER_TOOLS`/`SKILL_TOOLS` precedent, and accept both in `is_known_tool` so scenario validation passes. *(depends: T006)*
- [X] T012 Register a stub for every tool whose feature is compiled out, returning `Unavailable { NotCompiledIn { family } }` — a compiled-out tool stays *known* so a scenario referencing it fails loudly at the call, never silently at validation (spec Edge Case "the build was slimmed down"). *(depends: T011)*
- [X] T013 [P] Write a test in `tests/fail_closed_tools.rs` asserting a compiled-out family returns `NotCompiledIn` and is distinguishable from an empty result. *(depends: T012)*

**Checkpoint**: Foundation ready — US1–US4 can now proceed; US1, US2, US4 in parallel.

---

## Phase 3: User Story 1 — Search code by structure, not by text (P1) 🎯 MVP

**Goal**: Syntax-aware structural search inside the scope, so matches come from the parse tree and
decoys in comments and string literals are excluded.

**Independent test**: On a checkout with no security tooling installed, search a fixture containing
the same construct in live code, in a comment, and in a string literal — only the live-code match
returns. A denied subdirectory contributes nothing.

### Tests first

- [X] T014 [P] [US1] Create the decoy fixture (`tests/fixtures/astgrep/decoy.rs`) with `v.unwrap()` in live code, in a comment, and inside a string literal — per `quickstart.md` §US1.
- [X] T015 [P] [US1] Write a failing test in `tests/astgrep_tool.rs`: searching `$X.unwrap()` over the fixture returns exactly one match at the live-code line; the comment and string-literal occurrences are absent (SC-007, US1 scenario 1).
- [X] T016 [P] [US1] Write a failing test in `tests/astgrep_tool.rs`: an unsupported language returns `Unavailable { LanguageUnsupported { lang, compiled_in } }` naming what *is* compiled in — never an empty result (US1 scenario 4, FR-012).
- [X] T017 [P] [US1] Write a failing test in `tests/astgrep_tool.rs`: a syntactically invalid pattern returns `Failed` with a diagnostic and the episode continues (US1 scenario 3).

### Implementation

- [X] T018 [US1] Define `AstGrepArgs` (`--lang`, `--path`, `--limit` default 200, positional `pattern` with `allow_hyphen_values`) and `worker_argv()` in `src/astgrep.rs`, mirroring `SearchArgs`/`worker_argv` in `src/search.rs` (contract `native-tools.md`). *(depends: T004)*
- [X] T019 [US1] Implement the compiled-in language registry in `src/astgrep.rs` — map `--lang` to an `ast-grep-language` grammar behind its `astgrep-<lang>` feature, and expose the compiled-in list for the `LanguageUnsupported` message. *(depends: T018)*
- [X] T020 [US1] Implement the worker in `src/astgrep.rs`: parse with `ast-grep-core`, match the pattern, bound results at `--limit`, emit structured matches (path, start/end line, matched text) on stdout. Expose **no** replace path — this feature is read-only (spec Assumptions). *(depends: T019)*
- [X] T021 [US1] Add the `astgrep-worker` subcommand to `src/main.rs`, wired exactly like `search-worker` (`src/main.rs:112`). *(depends: T020)*
- [X] T022 [US1] Implement the `Tool` impl in `src/tools/astgrep.rs` — schema, typed arg parsing, `run_child` exec of `bee astgrep-worker`, returning `ToolOutcome<Vec<Match>>`. *(depends: T021, T007)*

**Checkpoint**: US1 delivers standalone value — bee searches code better than regex. Ship-able alone.

---

## Phase 4: User Story 2 — Findings survive the session (P2)

**Goal**: A durable, project-scoped, append-only ledger that merges across runs and preserves human
verdicts.

**Independent test**: Record findings, end the episode, start a new one, record an overlapping set —
the union is present, repeats did not duplicate, and a human verdict from the first run survived.

### Tests first

- [X] T023 [P] [US2] Write a failing test in `tests/finding_ledger.rs`: recording the same finding across two runs yields one entry with two sightings, not two entries (FR-019, SC-003).
- [X] T024 [P] [US2] Write a failing test in `tests/finding_ledger.rs`: a finding marked `FalsePositive` keeps that verdict when automated re-discovery appends a sighting (FR-020, SC-004).
- [X] T025 [P] [US2] Write a failing test in `tests/finding_ledger.rs`: a record missing `path`, `class`, `title`, or `evidence` is rejected with a diagnostic naming the field, and `ledger.jsonl` is byte-identical afterwards (FR-021).
- [X] T026 [P] [US2] Write a failing test in `tests/finding_ledger.rs`: a finding whose line moved between runs but whose substance is unchanged merges rather than duplicating (identity excludes line number — data-model §FindingId).

### Implementation

- [X] T027 [US2] Define `Finding`, `FindingId`, `Sighting`, `Severity`, `SeverityBand`, `Verdict`, `VerdictState`, and `FindingSource` in `src/findings.rs` per `data-model.md`. `FindingId` = hash of normalised path ‖ class ‖ normalised title, **excluding** the line number. Two manifest additions the `findings` feature needs: `dep:sha2` (the workspace has no hash crate) and `time/parsing` + `time/serde-well-known` (the workspace `time` carries only `formatting`, and `Sighting.at` has to round-trip through the log). Separate the three hashed components with `␟` (U+001F) rather than concatenating bare: `("a/b", "c")` and `("a", "b/c")` must not collide into one id. *(depends: T004)*
- [X] T028 [US2] Implement validation in `src/findings.rs` — reject a record missing any required evidence field, returning `Failed` with the field named; never partially write. *(depends: T027)*
- [X] T029 [US2] Implement `LedgerEvent` and the append path in `src/findings.rs`: `O_APPEND` of one serialised line to `.bee/findings/ledger.jsonl`, with `MAX_EVENT_BYTES` (**4000**) enforced by truncating `evidence` with a marker *before* serialisation so no line is ever cut mid-write (contract `finding-ledger.md`). The cap is 4000 rather than the 4096 the contract first stated: `PIPE_BUF` **is** 4096, so a 4096-byte payload plus its `\n` is 4097 — one byte past the atomicity the whole lock-free design rests on. `contracts/finding-ledger.md` §Concurrency records the corrected value and the reasoning. *(depends: T027)*
- [X] T030 [US2] Implement `fold(log) -> view` in `src/findings.rs` and materialise `.bee/findings/view.json`. Folding is pure and total: an unrecognised `event` tag is skipped and counted, never fatal. `Observed` on a known id appends a sighting and must **not** overwrite `title`/`evidence`/`path`. *(depends: T029)*
- [X] T031 [US2] Implement the `record_finding` and `list_findings` tools in `src/tools/finding.rs`, returning `ToolOutcome<FindingId>` and `ToolOutcome<Vec<Finding>>`. Reject a caller-supplied `severity.score` outright rather than silently recomputing it (FR-005). Both need a `run_id` and a ledger directory that `Tool::call(&self, args, &Sandbox)` cannot supply, so inject them at construction — `RecordFinding::new(run_id, ledger_dir)`, `ListFindings::new(ledger_dir)` — following the `SubmitFlag::new(flag)` precedent. Add both names to `SEC_TOOLS` and `sec_tool_family` and add their `register_named` arms; the `NotCompiledIn` stub path then covers them automatically. *(depends: T030, T007)*
- [X] T031a [US2] Add the `bee findings {list,adjudicate}` subcommand in `src/main.rs` — `adjudicate <id> --state <confirmed|false-positive|fixed|duplicate|deferred> [--note]` appends a `LedgerEvent::Adjudicated`. This is the **only** writer of that event and is deliberately CLI-only: adjudication is the human's act, so it is never a model-facing tool. Without it FR-020 is tested but unreachable in practice, and `quickstart.md` §US2 already documents the command. *(depends: T029)*
- [X] T032 [US2] Apply `safe_text` to every finding text field at the front-end boundary in `src/repl/terminal.rs` and `src/tui/app.rs`, extending the 014 treatment to this new channel (FR-015). *(depends: T031)*
- [X] T033 [US2] Resolve open question 2 — decide and implement the ledger location when the analysed project is read-only in policy: either a configured writable location or an explicit `Failed`. Record the decision in `research.md`. *(depends: T029)*
- [X] T034 [US2] Resolve open question 1 by **writing the test**: add a concurrent-append case to `tests/finding_ledger.rs` proving two writers lose no events (FR-022) — N writers × M events each, assert N×M lines and that every one parses. It is the only thing that substantiates the claim the lock-free design rests on, it is ~20 lines, and the 4000-byte cap from T029 is what makes it pass. Deferring it (the alternative the open question offered) leaves an untested assertion at the centre of the ledger. *(depends: T029)*

**Checkpoint**: US2 delivers standalone value — findings stop evaporating between sessions.

---

## Phase 5: User Story 3 — Borrow someone else's rule corpus (P3)

**Goal**: Run an operator-granted external scanner, normalise its SARIF into bee findings, and fail
closed in every case where it cannot run.

**Depends on**: US2 (findings need a ledger to land in).

**Independent test**: With the scanner installed and granted, a known-vulnerable file produces a
finding in the ledger. Without the grant — or without the binary — the result is an explicit
unavailability and nothing is written.

### Tests first

- [X] T035 [P] [US3] Capture the measured Opengrep SARIF as a fixture at `tests/fixtures/sarif/opengrep-shell-true.json` (research R4: 1 result, 1074 embedded rules, 1,912,546 bytes — keep a trimmed variant plus a note recording the real proportions).
- [X] T036 [P] [US3] Write failing tests in `tests/scanner_adapter.rs` for each fail-closed case in `quickstart.md` §US3: no grant → `NotGranted`; missing binary → `BinaryMissing`; substituted binary → `PinMismatch` and never executed (SC-010); unparseable report → `Failed`; timeout → `Failed`; feature off → `NotCompiledIn`. Each must be distinguishable from a clean scan (SC-002).
- [X] T037 [P] [US3] Write a failing test in `tests/scanner_adapter.rs`: `--config auto` is refused at argv construction, not at runtime (research R6).
- [X] T038 [P] [US3] Write a failing test in `tests/scanner_adapter.rs`: a SARIF whose `message.text`/`snippet.text` carries terminal control and bidi characters renders inert and cannot alter the terminal (SC-005, FR-015).
- [X] T039 [P] [US3] Write a failing test in `tests/scanner_adapter.rs`: a scanner exiting `0` **with** findings is not treated as failure, and one exiting `0` with `executionSuccessful: false` **is** — success comes from the report, never the exit code (research R6).

### Implementation

- [X] T039a [US3] Add `run_child_timed` to `src/tools/exec.rs`: same hardened, scope-joined spawn as `run_child`, but returning the raw exit status with stdout and stderr **kept apart**, under a wall-clock budget. The existing `run_child` cannot serve the pipeline — it folds stderr into the content, caps at `DEFAULT_OUTPUT_CAP`, and takes no timeout, so T036's timeout case has nothing to trip and T047 has nothing clean to parse. `run_child` itself is untouched, so no existing tool changes behaviour. *(depends: none)*

- [X] T040 [US3] Define the minimal SARIF structs in `src/sarif.rs` covering only the documented subset (`executionSuccessful`, `ruleId`, `level`, `message.text`, `artifactLocation.uri`, `region.startLine`/`endLine`/`snippet.text`). Serde ignores unknown fields; `tool.driver.rules` is never materialised (research R5). *(depends: T004)*
- [X] T041 [US3] Implement SARIF → `Finding` normalisation in `src/sarif.rs`, bounded at `MAX_FINDINGS_PER_SCAN` (200), setting `ToolOutcome::truncated` when the cap bites. A scanner's `level` is recorded as advisory and never becomes `Severity` (FR-005), which needs somewhere to live: `Finding.advisory_level: Option<String>`, now carried in `data-model.md`. *(depends: T040, T027)*
- [X] T042 [US3] Add the `sarif-worker` subcommand to `src/main.rs` (`--report`, `--source`, `--limit`) so normalisation runs **inside the scope** — the harness never opens the report file (Constitution III). The worker emits **JSONL**: one finding per line, bounded to `MAX_FINDINGS_PER_SCAN` *before* printing. One object per line rather than one array means a transport cut costs the last record instead of the whole parse, which keeps a truncated read from degrading into "the scanner found nothing". *(depends: T041)*
- [X] T043 [US3] Define the `ScannerAdapter` trait (`name`, `probe`, `argv`, `report_kind`) and `ScanRequest` (`target`, `lang`, `timeout` — deliberately no free-form arg field) in `src/scanners/mod.rs` (contract `scanner-adapter.md`). *(depends: T006)*
- [X] T044 [US3] Implement `ScannerGrant` resolution in `src/scanners/mod.rs` — read the `!`-pinned `ExecPolicy.allow` entry via the existing grant/attenuation path; a binary merely present on `PATH` yields `NotGranted` (FR-008). Grants resolve **once, at registry construction** (`ScanTool::new(grants, run_id, ledger_dir)`), because `Tool::call` receives only a `&Sandbox` and the policy is not reachable from there — same injection shape as `SubmitFlag::new(flag)`. The accepted cost: a scanner granted mid-episode through 007 escalation is not visible until the next episode; the inode pin is still re-checked per call in T045, so this trades freshness of the *grant set*, never freshness of the *identity check*. `grant.rules` is populated from a new `[scanners.<name>]` block in `src/config.rs` — a ruleset path is configuration, not a capability, so policy keeps exactly the surface it has today (Constitution IV). *(depends: T043)*
- [X] T045 [US3] Implement `probe()` in `src/scanners/mod.rs`: existence, then inode-pin re-check immediately before spawn so a binary substituted after the grant is never executed (FR-009, SC-010). *(depends: T044)*
- [X] T046 [US3] Implement the Opengrep adapter in `src/scanners/opengrep.rs` — argv `["scan", "--sarif", "--sarif-output=<out>", "--quiet", "--config", <grant.rules>, "--timeout", <secs>, <target>]`, with validation rejecting `auto`, out-of-scope paths, and any caller-influenced flag (FR-007). *(depends: T043)*
- [X] T047 [US3] Implement the two-child pipeline in `src/tools/scanner.rs`: child 1 runs the scanner writing SARIF to `.bee/scan/<run_id>.sarif`, child 2 runs `bee sarif-worker` over it; both via `run_child_timed`. The report path is project-local rather than under `/tmp` because it has to be writable by child 1 *and* readable by child 2 while both are in-scope, and a scanning policy need not admit `/tmp` at all; remove it after normalisation. Determine success from exit-without-signal-and-within-budget → report parses → `executionSuccessful`, in that order (research R4, R6). *(depends: T042, T046, T045, T039a)*
- [X] T048 [US3] Merge normalised findings into the ledger as `Observed` events sourced `scanner:<name>` in `src/tools/scanner.rs`. *(depends: T047, T029)*

**Checkpoint**: US3 proves the external tier end to end and buys a large rule corpus.

---

## Phase 6: User Story 4 — Severity is computed, not guessed (P4)

**Goal**: The model supplies a severity vector; the `cvss` crate computes the score.

**Independent test**: Reference vectors with independently known scores all compute correctly; a
malformed vector is rejected; a caller-supplied score is rejected rather than recomputed.

### Tests first

- [X] T049 [P] [US4] Write a failing test in `tests/cvss_tool.rs` over a reference set of v3.1 and v4.0 vectors with independently known scores — including `CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H` → `9.8 (critical)` (SC-008).
- [X] T050 [P] [US4] Write a failing test in `tests/cvss_tool.rs`: a malformed vector returns `Failed` with the parse diagnostic — not a zero score and not a guess (US4 scenario 2).
- [X] T051 [P] [US4] Write a failing test in `tests/cvss_tool.rs`: submitting a `Severity` with a caller-supplied `score` is rejected, not silently recomputed (FR-005, US4 scenario 3).

### Implementation

- [X] T052 [US4] Implement the `cvss` tool in `src/tools/cvss.rs` — parse the vector via `cvss::v3::Base` / `cvss::v4::Vector`, return `ToolOutcome<Severity>` with the computed score and band. **No worker and no child**: it opens no files, so the sandboxed-child seam has nothing to protect (contract `native-tools.md`). *(depends: T004, T007)*
- [X] T053 [US4] Wire the optional `finding` argument to emit a `LedgerEvent::Scored`, the only path that may write `Severity.score` (FR-005). *(depends: T052, T029)*

**Checkpoint**: First slice complete — US1–US4 all shippable and host-testable.

---

## Phase 7: User Story 5 — Read the repository's history (P5) — DEFERRED

**Goal**: Log and blame from the repository itself, with no `git` binary in the allowlist.

> Not in the first slice. Specified and contracted; build after US1–US4 land.

- [ ] T054 [P] [US5] Write failing tests in `tests/gitlog_tool.rs`: log returns commits with author/date/summary; blame identifies the introducing commit; a non-repository path returns `Unavailable { NotARepository }`, never an empty history (US5 scenarios 1–3).
- [ ] T055 [US5] Define `GitLogArgs` (`--path`, `--mode`, `--line`, `--limit`) and `worker_argv()` in `src/gitlog.rs`. *(depends: T004)*
- [ ] T056 [US5] Implement the worker over `gix` 0.86 (`revision` for log, `blame` for line origin) in `src/gitlog.rs`. *(depends: T055)*
- [ ] T057 [US5] Add the `gitlog-worker` subcommand to `src/main.rs` and the `Tool` impl in `src/tools/gitlog.rs`. *(depends: T056, T007)*

---

## Phase 8: User Story 6 — Deep whole-program analysis (P6) — DEFERRED

**Goal**: Run a provisioned, version-pinned CodeQL bundle over build-mode-`none` languages.

> Not in the first slice. Depends on the whole external tier (US3) being proven.

- [ ] T058 [P] [US6] Write failing tests in `tests/codeql_adapter.rs`: an absent or version-mismatched bundle returns `Unavailable { BundleMismatch { expected, found } }` (US6 scenario 2); a traced (compiled) language is declined explicitly rather than half-supported (US6 scenario 3).
- [ ] T059 [US6] Implement bundle probing in `src/scanners/codeql.rs` — run `<bundle>/codeql/codeql version --format=json` and compare against `grant.bundle_version` (pin `codeql-bundle-v2.26.1` / CLI `2.26.1`, per `codeql-action` v4.37.3 `src/defaults.json`). *(depends: T043)*
- [ ] T060 [US6] Implement the argv builder in `src/scanners/codeql.rs`: `database create <db> --language=<lang> --build-mode=none --source-root=<target>` then `database analyze <db> --format=sarif-latest --output=<out> <suite>`. *(depends: T059)*
- [ ] T061 [US6] Refuse traced languages in `src/scanners/codeql.rs` with an explanatory `Unavailable` — extraction for compiled languages intercepts the build's process spawns (`--begin-tracing`, `--trace-process-name`), which would require admitting every compiler and linker the build invokes, an unbounded widening that surrenders SC-006 and violates Constitution II (research R7). *(depends: T060)*

---

## Phase 9: Polish & Cross-Cutting Concerns

- [X] T062 [P] Verify SC-009 — confirm the default build's dependency graph and binary size are unchanged from `main`, and that each feature builds standalone (`cargo build --features cvss`, `--features findings`, `--features astgrep,astgrep-rust`).
- [X] T063 [P] Verify SC-006 — assert a scanning episode's compiled exec allowlist contains bee's own inode plus only explicitly granted scanners, in `tests/scanner_adapter.rs`. *(Done in two halves: `a_grant_is_only_ever_an_inode_pinned_exec_entry` / `an_unpinned_allow_entry_is_not_a_scanner_grant` / `no_policy_at_all_grants_no_scanner` prove the grant path adds nothing to `exec.allow` and reads nothing from `PATH`; `the_compiled_exec_surface_is_bee_plus_the_granted_scanner_and_nothing_else` compiles the policy and asserts over the lowered allowlist the kernel is handed, including that every entry is inode-pinned.)*
- [X] T064 [P] Add a property test in `crates/core/tests/attenuation_prop.rs` (not `attenuation.rs` — that file holds the targeted unit cases, and the property tests have always lived beside it) asserting no sequence of scanner grants yields an exec allowlist outside the ceiling, reusing the 007 attenuation harness (Constitution II gate).
- [X] T065 [P] Add a test asserting `ledger.jsonl` round-trips through a text diff without loss (Constitution IV gate).
- [X] T066 Walk `quickstart.md` end to end on a clean checkout with no security tooling installed and confirm every stated expectation, including the measured SARIF proportions (SC-001, SC-011). *(Walked 2026-07-27 on a fresh clone. Every behavioural expectation holds: all 5 builds green; US1 returns the single live-code match where regex returns all three; US2's two-run round trip yields 2 findings with 2 sightings each and a surviving false-positive verdict; US3 scans live through a pinned grant and records `scanner:opengrep`; US4 computes 9.8/9.3 and refuses a supplied score. Four **documentation** defects found and fixed in `quickstart.md`: `bee scan` and `bee cvss` were never CLI subcommands (both are model-facing tools, driven here through a `mock`-provider scenario — and `--policy` is mutually exclusive with `--host`, so the scan path needs a scenario's own `policy_path`); `--test cvss_tool` named a target that does not exist (the cvss tests are unit tests); the US2 invocation was not a runnable command; and the `Cargo.lock` expectation is uncheckable from a clean checkout because the lockfile is gitignored. Three findings left for a decision rather than patched — see the note below.)*
- [X] T067 [P] Document the tool surface in `README.md` and the two-tier decision rule in `CONTEXT.md`; note the application package's 1.88 MSRV and the per-language `astgrep-<lang>` features.
- [X] T068 [P] Add the `scanner-escape-denied` case to `test/vm/remote-matrix.sh` — a scanner child's out-of-scope read is refused by the kernel (currently 35/35; not required for the first slice to land). *(Landed via **017-enforceable-pins**, which is what the attempt to write this case uncovered: a scanner grant is by definition an inode-pinned `exec.allow` entry, and the eBPF planner refused every policy containing one — `infra_error`, in `enforce` and `observe` alike — so `scan` was reachable only under `--host` and the case could not exist. 017 made the kernel match a pin on `(i_ino, s_dev)`, and widened a resolved grant to carry `.bee/scan`, the report channel a default protection was denying to both children. The case now runs under `enforce`, with a control run proving the read succeeds when nothing forbids it. Matrix: 38/38, 2026-07-28.)*
- [X] T069 `cargo fmt --all -- --check` and `cargo clippy --workspace --all-targets --features sec,astgrep-rust,astgrep-python -- -D warnings`.

### Open from the T066 walkthrough

Three things the walkthrough surfaced that are code decisions, not documentation fixes. None blocks
the slice; all are recorded in `quickstart.md` beside the check that found them.

- [X] T070 `Finding.advisory_level` is dead for the one shipped adapter. Opengrep reports severity in `tool.driver.rules[].defaultConfiguration.level`, not on the result, and `src/sarif.rs` deliberately never materialises `tool.driver.rules` — that array is the 99.96% of the report research R4 says not to read. Either reach the level without materialising the catalogue (a streaming id→level pass over the rules array in `sarif-worker`, which is in-scope and bounded), or drop the field and say why. Harmless today (an advisory level never becomes `Severity`, FR-005), but a field that can never be populated is a lie in the data model. *(Fixed by the first option: `retain_rule_levels` in `src/sarif.rs` is a streaming seq visitor that collapses the catalogue to `id → level` as it passes — two short strings per rule, capped at `MAX_RULE_LEVELS` (4096) — so help text, descriptions, and tags are still parsed and dropped rather than allocated. A result's own `level` wins over the rule default (SARIF §3.27.10); a rule past the cap costs only its advisory level, never the finding, because the visitor drains the sequence rather than aborting the deserializer and turning a big catalogue into "unparseable report". FR-005 is unmoved: `to_finding` carries the level and leaves `severity` `None`, asserted in `a_rule_level_reaches_the_ledger_without_becoming_a_severity`.)*
- [X] T071 The FR-005 score guard keys on the flat `severity_score`. A caller who nests `severity = { score = … }` has the object dropped by serde, so no score reaches the ledger — FR-005 is not violated — but the caller is *silently ignored* rather than refused, which is precisely the failure mode the guard's own doc-comment says it exists to prevent. *(Fixed: `#[serde(deny_unknown_fields)]` on `RecordArgs`, so there is no spelling of "here is my score" bee accepts quietly. Covered by `a_score_nested_under_another_name_is_refused_too`.)*
- [X] T072 `repl_command::no_provider_at_all_reports_what_is_missing` fails on any non-`enforce` build — the unenforced-session refusal fires before the configuration-completeness check the test asserts on. **Pre-existing on `main`**, not a 016 regression (verified by running the test on both branches at default features), and the only failure in the whole suite. It needs `--host`, or the `skip_on_enforcement_build()` treatment its neighbours have, inverted. *(Fixed with `--host`, which settles enforcement identically on both builds and lets the completeness failure be about the provider — an `enforce` build would otherwise have named the missing **policy** instead, so the assertion held on neither. No skip guard: this case exits at configuration and never starts a session. The fix exposed a second cause — the child inherited the developer's `~/.config/bee/config.toml`, where a provider silently completes the session the test means to leave incomplete — so `run_with_stdin` now runs in an empty directory with `HOME`/`XDG_CONFIG_HOME` pointed at it. Verified on both builds; it runs rather than skips under `enforce`.)*

---

## Dependencies

```text
Setup (T001–T005)
   └─▶ Foundational (T006–T013)     ← blocks every story
          ├─▶ US1  astgrep   (T014–T022)   independent  🎯 MVP
          ├─▶ US2  ledger    (T023–T034, +T031a)   independent
          │      ├─▶ US3  scanner (T035–T048, +T039a)   needs the ledger
          │      └─▶ US4  cvss    (T049–T053)   T053 needs the ledger; T052 does not
          ├─▶ US5  gitlog    (T054–T057)   DEFERRED, independent
          └─▶ US6  codeql    (T058–T061)   DEFERRED, needs US3
                 └─▶ Polish (T062–T069)
```

**Story independence**: US1, US2, and US4's scoring path are mutually independent and can proceed in
parallel once Foundational lands. US3 genuinely depends on US2 — findings need somewhere to go — and
US6 on US3. This is the one place the "all stories independent" ideal does not hold, and it is
inherent to the feature rather than an artefact of the plan.

## Parallel execution examples

**Foundational** — T009, T010 in parallel (same file, different test fns: coordinate or split).

**US1** — T014, T015, T016, T017 all parallel (fixture + three independent test fns), then
T018 → T019 → T020 → T021 → T022 strictly sequential (each builds on the last).

**US2** — T023, T024, T025, T026 all parallel; then T027 → T028/T029 → T030 → T031 → T031a → T032.
T033 and T034 are parallel with each other (and with T031a) once T029 lands.

**US3** — T035, T036, T037, T038, T039 all parallel, and T039a alongside them (it depends on nothing
in this phase); then T040 → T041 → T042 and T043 → T044 → T045 and T046 proceed as three parallel
chains, converging at T047.

**US4** — T049, T050, T051 all parallel; then T052 → T053.

**Cross-story** — once Foundational completes, one contributor can take US1 while another takes US2,
with US3 and US4 following behind US2.

## Implementation strategy

**MVP = Phase 1 + Phase 2 + Phase 3 (US1).** That is 22 tasks and delivers a real capability
increase on its own: bee searches code by syntactic structure instead of text, inside the enforced
scope, with no external tooling installed.

**Increment 2 = US2**, which turns bee from a tool that finds things into one that remembers them.

**Increment 3 = US3 + US4**, which proves the external tier and closes the severity gap.

**Then US5, US6** as separate slices, each already contracted.

Every increment through US4 is host-testable, so the whole first slice can be developed and reviewed
without the `ac-matrix-vm` enforcement environment (SC-011). The one VM task (T068) is polish and
explicitly not a blocker.
