# Feature Specification: Harness-Native Security Tooling

**Feature Branch**: `016-native-tools`

**Created**: 2026-07-26

**Status**: Draft

**Input**: User description: "Harness-native security tooling for bee. Give the model a set of security-analysis tools that are implemented in Rust inside bee wherever a maintained crate is the actual engine, and that shell out to an external binary only where the value is someone else's curated rule corpus or a heavy non-Rust runtime. Two tiers with one shared contract."

## Overview

bee can already confine what a model does. It cannot yet give a model much worth doing in a
security investigation: the tool surface is `bash`, file access, and regex search. Anything
more capable today means the operator installs a pile of command-line scanners and widens the
executable allowlist to admit each one.

This feature gives bee a security-analysis tool surface organised by a single rule:

> **Build it into the harness when a maintained library is the actual engine. Shell out only
> when the value is a curated rule corpus or a heavy non-native runtime that would be absurd
> to re-derive.**

Two tiers follow from that rule, and they share one contract. The **native tier** runs inside
the episode's scope as a bee subcommand, so every file it opens is mediated exactly like a
`cat` child. The **external tier** wraps a third-party scanner as an adapter: bee builds the
argument list from typed inputs, the binary is admitted to the allowlist by pinned identity,
and the scanner's report is normalised into bee's own finding shape before anyone sees it.

The payoff is threefold. An operator gets a working security agent from a single binary
instead of an install ritual. The executable allowlist stays near-empty — one entry for bee
itself, plus one pinned entry per scanner the operator has deliberately granted. And findings
stop evaporating: they accumulate in a ledger that merges across runs instead of being
re-derived from scratch every session.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Search code by structure, not by text (Priority: P1)

An analyst is auditing a codebase for a pattern that regex handles badly — a call whose
receiver matters, an argument in a particular position, a construct that also appears
harmlessly inside comments and string literals. They ask the model to find every occurrence,
and get matches based on the code's actual syntactic shape, with the noise from comments and
strings gone.

**Why this priority**: It is the single largest capability increase over what exists today,
it is useful entirely on its own, and it exercises the sandbox seam that every other native
tool will reuse. Shipping only this leaves bee meaningfully better at code review.

**Independent Test**: On a checkout with no security tooling installed, ask for a structural
pattern that has decoy occurrences inside comments and string literals. Verify the real
matches are returned with locations and the decoys are not. Verify a file the policy denies
is not readable through the tool.

**Acceptance Scenarios**:

1. **Given** a source tree containing a target construct in live code and the same text inside a
   comment and a string literal, **When** the analyst searches for that construct structurally,
   **Then** only the live-code occurrences are returned, each with a file location and line span.
2. **Given** a scope whose policy denies reading a subdirectory, **When** a structural search is
   run over the parent directory, **Then** the denied subdirectory contributes no matches and the
   denial is recorded as an audit event.
3. **Given** a syntactically invalid search pattern, **When** the analyst runs it, **Then** the
   tool returns a diagnostic naming the problem and the episode continues.
4. **Given** a language whose support was not built into this binary, **When** a search targets it,
   **Then** the tool reports the language as unsupported rather than returning an empty result.

---

### User Story 2 - Findings survive the session (Priority: P2)

An analyst runs an investigation, the model surfaces several issues, and the session ends.
On the next run, the previous findings are still there. Re-running the same analysis does not
duplicate them; it adds what is new and leaves what was already recorded intact, including any
verdict a human has since attached.

**Why this priority**: It is the substrate every later stage writes into, and without it each
run starts from nothing and the same shallow issues are rediscovered forever. It also delivers
standalone value: the harness stops losing work to context compaction.

**Independent Test**: Record several findings, end the episode, start a new one, record an
overlapping set. Verify the union is present, that the repeats did not duplicate, and that a
human-applied verdict from the first run survived the second.

**Acceptance Scenarios**:

1. **Given** an empty ledger, **When** the model records a finding with a location, a category, and
   supporting evidence, **Then** the finding is durably stored and readable in a later episode.
2. **Given** a ledger holding a finding, **When** a later run records a finding matching the same
   location and category, **Then** the entry is merged rather than duplicated and its history
   records both sightings.
3. **Given** a finding a human has marked as a false positive, **When** a later run rediscovers it,
   **Then** the human verdict is preserved and is visible to the model.
4. **Given** a finding submitted with a malformed or incomplete record, **When** it is recorded,
   **Then** it is rejected with a diagnostic and the ledger is left unchanged.

---

### User Story 3 - Borrow someone else's rule corpus (Priority: P3)

An operator has a third-party scanner installed and wants its curated rules available to the
model without bee reimplementing any of them. They grant the scanner to an episode. The model
runs it over a chosen path, and the results come back in the same shape as every other
finding. If the scanner is not installed or cannot run, that is reported plainly — never as
a clean bill of health.

**Why this priority**: It proves the external tier end to end and buys a large rule corpus for
a small amount of work. It depends on the ledger existing, which is why it follows US2.

**Independent Test**: With the scanner installed and granted, run it over a directory with a
known-vulnerable file and confirm the finding lands in the ledger. Then remove the grant, or
point at a machine without the scanner, and confirm the result is an explicit unavailability
report and that nothing is written to the ledger.

**Acceptance Scenarios**:

1. **Given** a granted, installed scanner, **When** the model runs it over a path, **Then** its
   results are normalised into bee's finding shape and merged into the ledger.
2. **Given** a scanner that is not installed, **When** the model tries to run it, **Then** the
   result states the scanner is unavailable, is distinguishable from a clean scan, and no findings
   are recorded.
3. **Given** a scanner that has not been granted to this episode, **When** the model tries to run
   it, **Then** the attempt is refused and audited, and the refusal names the missing grant.
4. **Given** scanner output containing terminal control or bidirectional-text characters drawn from
   the scanned source, **When** the results are displayed, **Then** those characters are rendered
   inert and cannot alter the operator's terminal.
5. **Given** a granted scanner binary that is replaced on disk after the grant is issued, **When**
   the model runs it, **Then** the substituted binary is refused rather than executed.
6. **Given** a scanner that exceeds its time budget or emits an unparseable report, **When** it
   runs, **Then** the failure is reported as a failure and no partial results are silently
   presented as complete.

---

### User Story 4 - Severity is computed, not guessed (Priority: P4)

An analyst wants a defensible severity score on a finding. The model supplies its judgement as
a structured severity vector — the part it is good at — and the score is calculated
arithmetically rather than estimated.

**Why this priority**: Small, self-contained, and it removes a known and well-documented source
of error. It has no dependencies beyond the ledger it annotates.

**Independent Test**: Submit a set of severity vectors with independently known scores and
confirm every computed score matches. Submit a malformed vector and confirm it is rejected.

**Acceptance Scenarios**:

1. **Given** a well-formed severity vector, **When** it is scored, **Then** the numeric score and
   its qualitative band are returned and match the published calculation.
2. **Given** a malformed severity vector, **When** it is scored, **Then** it is rejected with a
   diagnostic and no score is produced.
3. **Given** a finding in the ledger, **When** a severity vector is attached to it, **Then** the
   stored score is the computed one and not a value supplied by the model.

---

### User Story 5 - Read the repository's history (Priority: P5)

An analyst asks when a suspicious line was introduced, by whom, and whether the surrounding
code has been touched since. The answer comes from the repository itself, without a version
control binary in the allowlist.

**Why this priority**: History is strong corroborating evidence — it distinguishes a live
defect from one already fixed, and it points at the people who can confirm intent — but every
preceding story stands without it.

**Independent Test**: Against a repository with known history, ask for the origin of a specific
line and the recent changes to a file, and confirm both match what the history actually records.

**Acceptance Scenarios**:

1. **Given** a file in a repository, **When** the analyst asks for its recent change history,
   **Then** the commits are returned with their authors, dates, and messages.
2. **Given** a specific line, **When** the analyst asks when it was introduced, **Then** the
   introducing commit is identified.
3. **Given** a path that is not inside a repository, **When** history is requested, **Then** the
   tool reports that no history is available rather than returning an empty history.

---

### User Story 6 - Deep whole-program analysis (Priority: P6)

An operator has provisioned a heavyweight analysis bundle and wants its whole-program queries
run against a codebase, with results arriving in the same finding shape as everything else.

**Why this priority**: The highest-value corpus available, but also the heaviest to provision
and the slowest to run, and it depends on the entire external tier being proven first.

**Independent Test**: With a provisioned bundle, analyse a codebase in a language that requires
no build step and confirm findings land in the ledger. Confirm an absent or version-mismatched
bundle is reported as unavailable.

**Acceptance Scenarios**:

1. **Given** a provisioned bundle of the expected version, **When** the model analyses a codebase in
   a supported language that needs no build, **Then** results are normalised and merged into the
   ledger.
2. **Given** no bundle or a bundle of an unexpected version, **When** analysis is attempted, **Then**
   the result reports unavailability and names the expected version.
3. **Given** a codebase whose language requires a build to analyse, **When** analysis is attempted,
   **Then** the tool declines with an explanation rather than producing partial results.

---

### Edge Cases

- **A tool cannot run.** Not installed, not granted, wrong version, missing bundle, times out,
  crashes. Every one of these is an explicit unavailability or failure result. None of them may
  be presented as, or be indistinguishable from, a scan that found nothing.
- **Scanner output is hostile.** Reports embed source text from the code under analysis, which is
  attacker-controlled. Control characters, bidirectional overrides, and text that reads as
  instructions must be inert on display and treated as data everywhere.
- **Output is enormous.** A scan over a large tree can produce far more than fits in a model's
  context or an operator's terminal. Results are bounded, and truncation is stated rather than
  silent.
- **The scanner tries to escape.** A scanner that writes outside the paths the scope permits, or
  reaches the network, is subject to the same policy as any other child; denials are audited.
- **The binary changes underneath the grant.** A scanner replaced between the grant and the run
  must not execute.
- **Two runs write findings at once.** Concurrent episodes recording into one ledger must not lose
  or corrupt entries.
- **A finding's location moves.** Code shifts between runs; a finding whose line moved but whose
  substance is unchanged should merge rather than duplicate.
- **The build was slimmed down.** A tool family not compiled into this binary reports itself
  absent, distinguishably from present-but-found-nothing.

## Requirements *(mandatory)*

### Functional Requirements

#### Tool surface

- **FR-001**: The harness MUST provide security-analysis tools in two tiers — implemented in-process,
  or wrapping an external binary — presenting a single uniform interface to the model, which is
  not required to know which tier a given tool belongs to.
- **FR-002**: In-process tools that read files MUST perform those reads inside the episode's
  enforced scope, so that every file access is mediated by the same policy that governs any other
  tool child.
- **FR-003**: The harness MUST provide structural, syntax-aware code search that distinguishes live
  code from comments and string literals, and MUST report the languages it supports in the running
  build.
- **FR-004**: The harness MUST provide repository history queries — change history for a path, and
  origin attribution for a line — without requiring an external version-control binary.
- **FR-005**: The harness MUST compute severity scores arithmetically from a structured severity
  vector. It MUST NOT accept a numeric score asserted by the model in place of a computed one.
- **FR-006**: Each tool family MUST be independently selectable at build time, so a build includes
  only the families it needs and pays no cost for the rest.

#### External scanners

- **FR-007**: The harness MUST construct every external scanner's argument list itself from typed,
  validated inputs. The model MUST NOT be able to supply a raw command line, shell string, or
  arbitrary flags.
- **FR-008**: An external scanner MUST be usable only when explicitly granted to the episode, and
  that grant MUST pass the same attenuation checks as any other capability. Discovery of a binary
  on the host MUST NOT by itself confer the ability to run it.
- **FR-009**: An external scanner grant MUST pin the binary's identity, and execution MUST be
  refused if the binary at that path no longer matches the pinned identity.
- **FR-010**: The harness MUST normalise external scanner reports into its own finding shape before
  those results reach the model, the ledger, or any display surface.
- **FR-011**: Where an external scanner requires a provisioned artefact rather than a mere binary,
  the harness MUST verify the artefact's presence and expected version before running it, and MUST
  report a mismatch as unavailability.

#### Fail-closed reporting

- **FR-012**: A tool that cannot execute MUST return an explicit unavailability or failure result.
  It MUST NOT return an empty finding set, and callers MUST be able to distinguish "could not run"
  from "ran and found nothing".
- **FR-013**: A tool whose output is truncated, incomplete, or produced by a run that failed partway
  MUST say so, and MUST NOT present partial results as complete.
- **FR-014**: Every refusal, denial, and unavailability MUST emit a structured audit event.

#### Untrusted output

- **FR-015**: All text originating from analysed source or from a scanner's report MUST be treated
  as untrusted data. It MUST be rendered inert before reaching any front-end, such that it cannot
  emit terminal control sequences or alter surrounding presentation.
- **FR-016**: Untrusted scanner text MUST NOT be presented to the model as instruction; it enters
  the record as evidence only.

#### The finding ledger

- **FR-017**: The harness MUST record findings in a durable store that persists across episodes.
- **FR-018**: The ledger MUST be append-only in effect: a later run MUST merge with existing entries
  rather than overwrite them, and MUST preserve the history of when a finding was seen.
- **FR-019**: Re-recording a finding that matches an existing entry MUST merge into that entry rather
  than create a duplicate.
- **FR-020**: A verdict or annotation applied by a human MUST survive subsequent runs and MUST NOT be
  overwritten by automated re-discovery.
- **FR-021**: A finding record MUST be rejected if it lacks the evidence fields required to review it.
- **FR-022**: Concurrent writers MUST NOT lose or corrupt ledger entries.
- **FR-023**: The ledger MUST be human-readable and reviewable as text, consistent with the project's
  treatment of policy as reviewable data.

### Key Entities

- **Finding**: A single reviewable observation about the code — where it is, what class of issue it
  is, the evidence supporting it, an optional computed severity, the history of when it was seen,
  and any human verdict attached to it.
- **Finding Ledger**: The durable, human-readable, merge-on-write collection of findings for a
  project, shared across episodes and across tools.
- **Tool Family**: A group of related capabilities that a build either includes or omits as a unit.
- **Scanner Grant**: An operator's explicit, attenuation-checked authorisation for one episode to run
  one externally-provided analysis binary, bound to that binary's pinned identity.
- **Scanner Report**: A third-party scanner's raw output, always untrusted, always normalised into
  findings before use.
- **Severity Vector**: The structured, model-supplied characterisation of a finding's exploitability
  and impact, from which a score is computed.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: An operator with only the bee binary and a source checkout — no security tooling
  installed — can run structural search, history queries, severity scoring, and finding recording,
  with zero additional installation steps.
- **SC-002**: In every case where a tool cannot run, the result is reported as unavailable or failed;
  in no case is it reported as, or indistinguishable from, a scan that found nothing.
- **SC-003**: Running the same analysis twice against unchanged code produces no duplicate findings
  in the ledger.
- **SC-004**: Human verdicts survive 100% of subsequent automated runs over the same code.
- **SC-005**: No text originating from analysed source or a scanner report can alter the operator's
  terminal, as demonstrated by tests using control and bidirectional character payloads.
- **SC-006**: The set of executables a scanning episode is authorised to run is bee itself plus only
  those scanners the operator explicitly granted — verifiable by inspecting the episode's policy.
- **SC-007**: Structural search returns no matches drawn from comments or string literals for
  patterns that target live code.
- **SC-008**: Every computed severity score matches the published calculation for its vector across a
  reference set of vectors with independently known scores.
- **SC-009**: A build that selects no optional tool families is no larger and no slower to build than
  the equivalent build today.
- **SC-010**: A scanner binary substituted after its grant is issued is never executed.
- **SC-011**: The first-slice tools are fully exercisable on a developer machine without kernel
  enforcement, so contributors can work on them without the enforcement test environment.

## Assumptions

- **Scope of this feature is discovery and recording, not exploitation.** Tools that execute a
  candidate exploit and verify it against kernel-observed behaviour are a separate, later feature;
  they need the enforcement test environment, whereas everything here is host-testable.
- **First slice.** Structural search, the finding ledger, one external scanner adapter, severity
  scoring, and the report normaliser. Repository history and the heavyweight bundle adapter follow;
  binary inspection, crash symbolisation, and patch generation are out of scope entirely.
- **Structural search is read-only in this feature.** The underlying capability can also rewrite
  code; rewriting is a mutation with its own policy consequences and is deliberately excluded.
- **External scanners are operator-provisioned.** bee does not download, install, update, or manage
  the lifecycle of a third-party scanner. It detects, verifies, grants, and runs what the operator
  has already put on the machine.
- **Whole-program analysis is limited to languages needing no build step.** Analysing a compiled
  language requires the analyser to intercept the build's process spawns, which is fundamentally at
  odds with a narrow executable allowlist. That combination is out of scope and must be declined
  explicitly rather than half-supported.
- **The interchange format for external scanner reports is the industry-standard static-analysis
  results format**, which the intended scanners already emit.
- **The ledger is project-scoped**, living with the project under analysis rather than inside a
  single episode's transcript, since merging across runs is its entire purpose.
- **Finding identity** is derived from location plus issue class plus a normalised description, so
  that code movement alone does not create duplicates.
- **Existing machinery is reused rather than rebuilt**: the sandboxed-worker seam already used by
  the current search tool, the existing capability-grant and attenuation system, the existing
  executable identity pinning, the existing untrusted-text neutralisation, and the existing audit
  event stream.
