# Contract: External Scanner Adapter

The external tier. An adapter wraps a third-party scanner whose value is its rule corpus
(research R2). bee never reimplements the corpus and never hands the model a command line.

## The trait

```rust
pub trait ScannerAdapter: Send + Sync {
    /// Stable name, used in policy grants and in `FindingSource::Scanner(name)`.
    fn name(&self) -> &'static str;

    /// Can this scanner run right now? Checks existence, pin, and any provisioned artefact.
    /// Runs **before** argv construction, so unavailability is reported without spawning.
    fn probe(&self, grant: &ScannerGrant) -> Result<(), UnavailableReason>;

    /// Build the child's argv from typed, validated inputs. The ONLY place argv is authored.
    /// Returns an error — never a partially-built command — if any input fails validation.
    fn argv(&self, grant: &ScannerGrant, req: &ScanRequest, out: &Path) -> Result<Vec<String>, String>;

    /// Where this scanner writes its report, relative to the scratch dir handed to it.
    fn report_kind(&self) -> ReportKind;   // Sarif for both Opengrep and CodeQL
}

/// Typed scan inputs. Note what is absent: no flags, no shell string, no rule *content*.
pub struct ScanRequest {
    /// Directory or file to scan. Validated to resolve inside the scope.
    pub target: PathBuf,
    /// Language hint, where the scanner needs one.
    pub lang: Option<String>,
    /// Wall-clock budget. Exceeded ⇒ `Failed`, never a partial `Completed`.
    pub timeout: Duration,
}
```

## FR-007: the model never authors a command line

`ScanRequest` has three fields and none of them is free-form. There is deliberately no
`extra_args: Vec<String>`, no `config_inline: String`, and no passthrough of any kind. The rule
config comes from the **grant** (operator-set), not the request (model-set) — so the model can choose
*what to scan*, never *how the scanner is configured*.

This is the discipline `src/search.rs` already states for ripgrep, generalised:

> None of ripgrep's command-executing options (`--pre`, `--search-zip`) are exposed — the model
> drives this only through `SearchArgs`, and there is nothing here that runs another program.

Validation performed in `argv()`, all of which return `Err` rather than a degraded command:
- `target` resolves inside the scope after symlink resolution
- `grant.rules`, when set, resolves inside the scope and is not `auto` (research R6)
- no argument begins with `-` unless the adapter itself emitted it
- the output path is bee-chosen, never caller-influenced

## The two-child pipeline

Forced by measurement (research R4): an Opengrep SARIF over a single file is **1,912,546 bytes**
against a **102,400-byte** `DEFAULT_OUTPUT_CAP`. Capturing it on stdout truncates it into unparseable
JSON, which would surface as "the scanner found nothing" — precisely the failure FR-012 forbids.

```text
                      ┌─────────────────────────────────────────┐
  ScannerGrant ──────▶│ child 1: the scanner                    │
  (inode-pinned)      │   argv built by adapter, never by model │
                      │   joined to scope cgroup, hardened,     │
                      │   env-stripped — like any tool child    │
                      └──────────────┬──────────────────────────┘
                                     │ writes
                                     ▼
                       <scratch>/report.sarif        (inside the scope)
                                     │
                      ┌──────────────┴──────────────────────────┐
                      │ child 2: `bee sarif-worker`             │
                      │   reads the report IN SCOPE             │
                      │   normalises to Finding                 │
                      │   bounds the set BEFORE the output cap  │
                      └──────────────┬──────────────────────────┘
                                     │ bounded, structured stdout
                                     ▼
                              ToolOutcome<Vec<Finding>>
```

Both children run through the existing `run_child` path, so both are scope-joined and LSM-mediated.
**The harness never opens the report file** — doing so would read around the sandbox and violate
Constitution III. That the normaliser is a second child rather than harness code is the whole point.

## Success determination

**Never the exit status.** Measured: `opengrep scan --sarif --quiet` exited **0 with a finding
present**; it only returns non-zero on findings when `--error` is passed (research R6). Exit status
conflates "findings exist" with "run failed" and is wrong in both directions.

Success is determined by, in order:

1. Child 1 exited without a signal and within `timeout` — else `Failed`.
2. The report file exists and parses — else `Failed { "unparseable report" }`.
3. `runs[].invocations[].executionSuccessful` is true — else `Failed`.
4. Then, and only then, `Completed { value: findings }` — where `findings` may legitimately be empty.

## Bounding

The normaliser caps the finding set (`MAX_FINDINGS_PER_SCAN`, initially 200 — matching `search`'s
`MATCH_LIMIT` precedent). When the cap bites it sets `ToolOutcome::truncated`, so incompleteness is
stated rather than inferred. Bounding happens **before** rendering, so the transport cap never has to
cut a structured result mid-object.

The worker emits **JSONL** — one finding per line — rather than a single array, and belt-and-braces
with the bound above: if a cut ever did happen at the transport, it costs the last record instead of
the whole parse. An array would fail to parse entirely, and an unparseable result is one step from
reading as "the scanner found nothing", which is the failure this whole feature is built against.

## Grant and pin

A grant is an ordinary `ExecPolicy.allow` entry using the existing `!`-prefix inode pin
(`crates/core/src/policy.rs:81`):

```toml
[exec]
allow = ["!/home/jg/.local/bin/opengrep"]
```

- **FR-008**: issued through the existing derive/ceiling path, so it is attenuation-checked like any
  other capability. Presence of a binary on `PATH` confers nothing — `probe()` returns
  `NotGranted` regardless of what is installed.
- **FR-009 / SC-010**: `probe()` re-checks the pin immediately before spawning. A binary substituted
  between grant issuance and run yields `PinMismatch` and is never executed.

**Where `grant.rules` comes from.** Not from policy: a ruleset path is configuration, not a
capability — it says *how* a granted scanner is configured, never *what* the episode may reach — so
admitting it to `ExecPolicy` would widen the policy surface for something that grants nothing
(Constitution IV). It lives in a `[scanners.<name>]` block in bee's own config, while the exec grant
stays exactly the `!`-pinned entry above. The model still cannot influence it either way: it is
operator-set in both halves.

**When grants are resolved.** Once, at registry construction (`ScanTool::new(grants, run_id,
ledger_dir)`) — `Tool::call` receives only a `&Sandbox`, from which the policy is not reachable. The
consequence is bounded and worth stating: a scanner granted mid-episode through 007 escalation is not
visible until the next episode. What is *not* deferred is the identity check — `probe()` re-stats the
pin on every call, so this trades freshness of the grant set, never freshness of the pin.

## Opengrep adapter

```text
argv = ["scan", "--sarif", "--sarif-output=<out>", "--quiet",
        "--config", <grant.rules>, "--timeout", <secs>, <target>]
```

`--config auto` is **refused at argv construction**, not at runtime: it fetches the rule registry over
the network (cached under `~/.opengrep/cli`), and a scanning scope has no egress, so `auto` would fail
opaquely. Measured: the same target scanned with a local ruleset produced a ~600-byte report versus
1.9 MB with `auto`.

## CodeQL adapter (US6, deferred)

Consumes an operator-provisioned, version-pinned **bundle** — `codeql-bundle-<platform>.tar.zst` from
`github/codeql-action` releases, which ships the CLI, matching queries, and precompiled query packs.
`codeql-action` v4.37.3 pins `codeql-bundle-v2.26.1` / CLI `2.26.1` (`src/defaults.json`).

```text
probe : <bundle>/codeql/codeql version --format=json  →  compare against grant.bundle_version
        mismatch or absent ⇒ Unavailable { BundleMismatch { expected, found } }

argv  : database create <db> --language=<lang> --build-mode=none --source-root=<target>
        database analyze <db> --format=sarif-latest --output=<out> <query-suite>
```

**Only `build-mode: none` languages are supported.** `databaseInitCluster`
(`codeql-action/src/codeql.ts:547`) pushes `--begin-tracing` and `--trace-process-name` for compiled
languages, because extraction works by **intercepting the build's process spawns**. That requires
admitting every compiler, linker, and build tool the target's build happens to invoke — an unbounded,
un-attenuable widening that surrenders SC-006 and violates Constitution II. A request for a traced
language is declined explicitly (`Unavailable`), because half-support is worse than none.

`rust` is a builtin CodeQL language (`codeql-action/src/languages/builtin.json`), so bee can
eventually analyse itself.

## Untrusted output (FR-015/016)

`message.text` and `region.snippet.text` are verbatim attacker-controlled source. They:

- enter the ledger as **data**, never as instruction;
- pass through `safe_text` before reaching any front-end, so control and bidi characters cannot alter
  the operator's terminal (the 014 property, extended to a new input channel);
- never become `Severity` — a scanner's `level` is recorded as advisory only, because severity comes
  from the computed path alone (FR-005).
