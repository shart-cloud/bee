# Contract: Finding Ledger

Append-only event log, folded into a materialised view. The substrate every tool in this feature
writes into (spec US2).

## Layout

```text
.bee/findings/
├── ledger.jsonl     # append-only, one LedgerEvent per line — the source of truth
└── view.json        # folded state; a cache, regenerable, never hand-edited
```

Both are text, diffable, and reviewable in a pull request (FR-023, Constitution IV). The log is never
rewritten in place; the view is derived and disposable.

## Events

```rust
#[derive(Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum LedgerEvent {
    /// A finding was observed. Carries the full record on first sight; a bare sighting after.
    Observed {
        id: FindingId,
        run_id: String,
        at: OffsetDateTime,
        sighting: Sighting,
        /// Present only when this run is the first to see `id`.
        finding: Option<Box<Finding>>,
    },
    /// A human adjudicated. Only a later `Adjudicated` supersedes an earlier one.
    Adjudicated { id: FindingId, verdict: Verdict },
    /// A severity vector was scored. Written only by the scoring path (FR-005).
    Scored { id: FindingId, severity: Severity },
}
```

## Folding

`fold(log) -> view` is a left fold keyed by `FindingId`:

| Event | Effect on the view |
|---|---|
| `Observed` (id unseen) | Insert the record with its first sighting |
| `Observed` (id known) | Append the sighting. **Do not** overwrite `title`, `evidence`, or `path` |
| `Adjudicated` | Set `verdict`; a later event replaces an earlier one |
| `Scored` | Set `severity` |

Folding is **pure and total**: it never fails on an unknown event kind (forward compatibility — an
unrecognised `event` tag is skipped and counted), and it never partially applies an event.

## The four requirements this shape satisfies

| Requirement | Mechanism |
|---|---|
| **FR-018** merge, never overwrite | Folding is the only path to the view; the log is append-only |
| **FR-019** no duplicates | Two sightings of one `FindingId` fold into one entry with two sightings |
| **FR-020** verdicts survive | A verdict is an event; `Observed` has no power to clear one |
| **FR-022** concurrent writers | Appends are independent — no read-modify-write cycle to lose |

## Identity

```text
FindingId = hash(normalise_path(path) || class || normalise_title(title))
```

`normalise_title` lowercases, collapses runs of whitespace, and strips a trailing period.

**The line number is deliberately not part of identity.** Code moves between runs; a finding whose
line shifted but whose substance did not must merge, not duplicate (spec Edge Case "a finding's
location moves"). Lines are attributes of a `Sighting`.

The trade is explicit: two genuinely distinct defects of the same class, in the same file, with
identically-normalising titles will collide into one entry. That is the safer direction to err —
an over-merge is visible in the sightings list and correctable by a human, whereas a duplicate
storm makes the ledger useless.

## Concurrency (FR-022)

Writers `O_APPEND` a single serialised line. There is no lock and no read-modify-write.

Two constraints make this safe rather than hopeful:

1. **Bounded records.** A serialised event is capped (`MAX_EVENT_BYTES`, 4096). An event whose
   payload would exceed it has `evidence` truncated with a marker *before* serialisation, so no line
   is ever cut mid-write. Records under `PIPE_BUF` append atomically on Linux for regular files
   opened `O_APPEND`.
2. **The view is a cache.** A concurrent fold producing a stale view is recoverable — refold. A lost
   *event* would not be, which is why the log, not the view, is the source of truth.

Carried to tasks as an open question (research R11 §1): whether the first slice adds an explicit
concurrent-append test, or records that concurrent episodes sharing one ledger is out of scope until
the `concurrent` feature path needs it.

## Validation (FR-021)

An `Observed` carrying a `finding` is rejected — with the ledger unchanged — unless:

- `path` is non-empty and scope-relative;
- `class`, `title`, and `evidence` are each non-empty;
- exactly one sighting accompanies a first sight.

Rejection is a `Failed` outcome with a diagnostic naming the missing field. A rejected event is
never partially written.

## Severity may not be asserted (FR-005)

`Scored` is emitted only by the scoring path, which computes `score` and `band` from `vector` via the
`cvss` crate. A `record_finding` call carrying a pre-populated `Severity.score` is **rejected**, not
silently recomputed — silently accepting it would hide the violation from the caller and from review.

## Model-facing surface

Two tools, both feature-gated behind `findings`:

```text
record_finding { path, class, title, evidence, line?, rule_id?, severity_vector? }
    → ToolOutcome<FindingId>
    Appends `Observed` (+ `Scored` when a vector is supplied and scores cleanly).

list_findings { class?, verdict?, limit? }
    → ToolOutcome<Vec<Finding>>
    Folds the log and returns the view, bounded. Every text field escaped via `safe_text`
    at the front-end (FR-015); the model sees the raw record, which is data, not instruction
    (FR-016).
```
