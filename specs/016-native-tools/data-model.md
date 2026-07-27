# Phase 1 Data Model: Harness-Native Security Tooling

Entities from the spec, given concrete shape. Rust types are indicative — the authoritative
signatures live in [`contracts/`](./contracts/).

---

## Finding

A reviewable observation about the code. The unit of work for this whole feature (research R1).

```rust
pub struct Finding {
    /// Stable identity — see `FindingId`. Not derived from the line number.
    pub id: FindingId,
    /// Repo-relative, normalised (no `..`, no symlink crossing, forward slashes).
    pub path: String,
    /// What kind of issue. Free-form slug, e.g. `sql-injection`, `path-traversal`.
    pub class: String,
    /// One-line statement of the defect. Untrusted — escaped at every front-end.
    pub title: String,
    /// The evidence: what makes this real. Untrusted.
    pub evidence: String,
    /// Who produced it: a native tool name, or `scanner:<name>` for the external tier.
    pub source: FindingSource,
    /// Computed only. Never a model-supplied number (FR-005).
    pub severity: Option<Severity>,
    /// A producing scanner's own `level`, kept verbatim as advisory context. Deliberately separate
    /// from `severity`: it is what someone else's rule asserted, not what bee computed, and only the
    /// scoring path may write a number (FR-005).
    pub advisory_level: Option<String>,
    /// Every time this finding has been seen, oldest first. Never empty.
    pub sightings: Vec<Sighting>,
    /// Human adjudication, if any. Survives automated re-discovery (FR-020).
    pub verdict: Option<Verdict>,
}
```

**Validation** (FR-021 — a record missing these is rejected, ledger unchanged):
`path` non-empty and scope-relative · `class` non-empty · `title` non-empty · `evidence` non-empty ·
at least one `Sighting`.

---

## FindingId

```rust
pub struct FindingId(String);   // hex digest, 16 bytes rendered
```

Derived as `hash(normalised_path || class || normalised_title)`.

**Deliberately excludes the line number.** Code movement between runs must not mint a duplicate
(research R8, spec Edge Case "a finding's location moves"). Line numbers are attributes of a
`Sighting`, not of identity.

`normalised_title` lowercases, collapses whitespace, and strips a trailing period, so that two
scanners phrasing the same defect slightly differently still collide.

---

## Sighting

One observation of a finding at one moment.

```rust
pub struct Sighting {
    pub run_id: String,
    pub at: OffsetDateTime,
    /// 1-indexed; `None` when the producer reports no line (whole-file or whole-program findings).
    pub line: Option<u32>,
    pub end_line: Option<u32>,
    /// The producing tool's own identifier for the rule that fired, when it has one.
    pub rule_id: Option<String>,
}
```

Appending a sighting is the *only* effect of re-discovering an existing finding (FR-019).

---

## Severity

```rust
pub struct Severity {
    /// The model's structured judgement — the part it is good at.
    pub vector: String,          // e.g. "CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H"
    /// Computed by the `cvss` crate. Never supplied by the model (FR-005).
    pub score: f32,
    pub band: SeverityBand,      // None | Low | Medium | High | Critical
}
```

**Invariant**: `score` and `band` are only ever written by the scoring path. A submitted `Severity`
carrying a caller-supplied score is rejected, not silently recomputed — silently accepting it would
make the violation invisible.

---

## Verdict

Human adjudication. An *event*, not a mutable field (research R8).

```rust
pub struct Verdict {
    pub state: VerdictState,     // Confirmed | FalsePositive | Fixed | Duplicate(FindingId) | Deferred
    pub note: Option<String>,
    pub by: String,
    pub at: OffsetDateTime,
}
```

**Precedence rule (FR-020)**: an automated `Sighting` never overrides a `Verdict`. When a finding
marked `FalsePositive` is re-discovered, the sighting appends and the verdict stands. Only a later
verdict event supersedes an earlier one.

---

## Finding Ledger

Append-only event log plus a derived view.

```text
.bee/findings/
├── ledger.jsonl     # append-only; one LedgerEvent per line; the source of truth
└── view.json        # folded current state; regenerable, never hand-edited
```

```rust
pub enum LedgerEvent {
    /// A finding was observed. Carries the full record on first sight, the sighting alone after.
    Observed { id: FindingId, run_id: String, at: OffsetDateTime, finding: Box<Finding> },
    /// A human adjudicated.
    Adjudicated { id: FindingId, verdict: Verdict },
    /// A severity vector was scored and attached.
    Scored { id: FindingId, severity: Severity },
}
```

**Folding** (log → view) is a left fold by `FindingId`:
`Observed` merges — union sightings, keep the earliest full record, never overwrite `title`/`evidence`
of an existing entry · `Adjudicated` sets the verdict, last event wins · `Scored` sets severity.

**Concurrency (FR-022)**: writers only ever `O_APPEND` a single line. There is no read-modify-write
cycle on the log, so two episodes appending concurrently cannot lose each other's events. The view is
a cache, rebuilt by folding; a torn or stale view is recoverable, a lost event would not be.

---

## Tool Outcome

The three-state result every tool in this feature returns. Full contract in
[`contracts/tool-outcome.md`](./contracts/tool-outcome.md).

```rust
pub enum ToolOutcome<T> {
    Completed { value: T, truncated: bool },
    Unavailable { reason: UnavailableReason },
    Failed { reason: String },
}
```

**The load-bearing invariant (FR-012, Constitution I)**: `Completed { value: <empty>, .. }` means
*ran and found nothing*. It is unreachable from any error path. Everything that could not run is
`Unavailable`; everything that ran and broke is `Failed`.

---

## Scanner Grant

An operator's authorisation for one episode to run one external analysis binary.

```rust
pub struct ScannerGrant {
    pub name: String,            // "opengrep" | "codeql"
    pub path: PathBuf,           // absolute
    pub pin: InodePin,           // bound identity — see below
    pub rules: Option<PathBuf>,  // operator-provided local ruleset; never `auto` (research R6)
    pub bundle_version: Option<String>,  // CodeQL only; verified before use (FR-011)
}
```

**Representation in policy**: an ordinary `ExecPolicy.allow` entry using the existing `!`-prefix
inode-pinning syntax already documented at `crates/core/src/policy.rs:81`:

```toml
[exec]
allow = ["!/home/jg/.local/bin/opengrep"]
```

No new policy surface is introduced (Constitution IV). The grant is issued through the existing
derive/ceiling path, so it is attenuation-checked like any other capability (FR-008, Constitution II).
Discovery of a binary on `PATH` confers nothing.

---

## Scanner Report

A third-party scanner's raw output. **Always untrusted** (FR-015/016).

Only this subset of SARIF 2.1.0 is modelled (research R5 — the full document is 99.96% rules):

```text
runs[].invocations[].executionSuccessful     → did it actually run
runs[].results[].ruleId                      → Sighting.rule_id
runs[].results[].level                       → advisory only; not trusted as severity
runs[].results[].message.text                → Finding.title  (untrusted)
runs[].results[].locations[0]
    .physicalLocation.artifactLocation.uri   → Finding.path
    .physicalLocation.region.startLine       → Sighting.line
    .physicalLocation.region.endLine         → Sighting.end_line
    .physicalLocation.region.snippet.text    → Finding.evidence (untrusted)
```

Unknown fields are ignored by serde. `tool.driver.rules` is never materialised.

**Trust boundary**: `message.text` and `snippet.text` are verbatim attacker-controlled source. They
enter the ledger as data and pass through `safe_text` before reaching any front-end. A scanner's
`level` is recorded as advisory and never becomes `Severity` — only the computed path writes that.

---

## Tool Family

A build-time unit of inclusion (FR-006, SC-009). Cargo features, per the table in
[`plan.md`](./plan.md#feature-flags): `findings`, `astgrep` (+ `astgrep-<lang>`), `gitlog`, `cvss`,
`scanners`, and the `sec` umbrella.

**Runtime obligation**: a family compiled out must report itself absent. The tool is either not
registered at all, or registered and answering `Unavailable { NotCompiledIn }` — never registered and
returning an empty `Completed` (spec Edge Case "the build was slimmed down").

---

## Entity relationships

```text
ScannerGrant ──authorises──▶ scanner child ──writes──▶ ScannerReport (SARIF file, in scope)
                                                             │
                                                    bee sarif-worker (in scope)
                                                             │ normalises + bounds
                                                             ▼
native tool ──────────────────────────────────────────▶ Finding
                                                             │
                                                    LedgerEvent::Observed
                                                             ▼
                                                    ledger.jsonl ──fold──▶ view.json
                                                             ▲
                        Verdict ──LedgerEvent::Adjudicated───┤
                        Severity ─LedgerEvent::Scored────────┘
                                     ▲
                          cvss crate │ (never the model)
```
