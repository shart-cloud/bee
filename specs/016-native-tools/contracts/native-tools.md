# Contract: Native Tools

The native tier. Each tool's engine is a maintained Rust crate; each follows the three-piece shape
that `search` already proves — a `Tool` impl, a worker module, and a `*-worker` subcommand.

## The established seam

From `src/search.rs`, which this tier copies verbatim in structure:

> The security seam is the same one every file tool relies on: a tool never reads files in the
> harness — it does the work in a *sandboxed child* so the eBPF LSM mediates each open. `search` is
> the one tool whose "binary" is bee itself.

So for every native tool that touches the filesystem:

```text
src/tools/<x>.rs   Tool impl — builds typed args, calls run_child(bee, ["<x>-worker", …])
src/<x>.rs         worker    — the crate-backed work, running INSIDE the scope
src/main.rs        subcommand `<x>-worker`, wired like `search-worker` (src/main.rs:112)
```

`cvss` is the exception and proves the rule: it opens no files, so it needs no worker and no child.

---

## `ast_grep` — structural search (US1, feature `astgrep`)

Engine: `ast-grep-core` 0.45 + `ast-grep-language` 0.45.

```rust
#[derive(clap::Args, Debug, Clone)]
pub struct AstGrepArgs {
    /// Language whose grammar parses the target. Must be compiled into this build.
    #[arg(long)]
    pub lang: String,
    /// Root to search under. Resolved and opened inside the scope.
    #[arg(long, default_value = ".")]
    pub path: PathBuf,
    /// Cap on returned matches. Bounds output and walk time. Mirrors search's MATCH_LIMIT.
    #[arg(long, default_value_t = 200)]
    pub limit: usize,
    /// The ast-grep pattern, e.g. `$X.unwrap()`. `allow_hyphen_values` so patterns
    /// beginning with an operator are not parsed as flags.
    #[arg(allow_hyphen_values = true)]
    pub pattern: String,
}
```

Returns `ToolOutcome<Vec<Match>>` where `Match` carries path, start/end line, and the matched text.

**Why this beats `search` (SC-007)**: matches come from the parse tree, so a construct appearing
inside a comment or a string literal is structurally distinct from the same text in live code and is
not returned. This is the acceptance criterion for US1 scenario 1.

**Language availability (FR-006)**: `ast-grep-language` is taken with `default-features = false`;
its `builtin-parser` default would pull all 27 tree-sitter grammars, every one of them compiled C
(research R9). bee exposes one feature per language. A request for a language not compiled in returns
`Unavailable { LanguageUnsupported { lang, compiled_in } }` — naming what *is* available, so the
caller can retry usefully rather than guess.

**Read-only.** `ast-grep-core` can also rewrite (`ast_grep.replace(…)`); rewriting is a mutation with
its own policy consequences and is excluded from this feature by the spec's Assumptions. The worker
exposes no replace path.

**MSRV**: `ast-grep-core` 0.45 declares Rust 1.88. The application package declares 1.88 to match;
the embeddable core crates keep their 1.85 promise, since `ast-grep` never enters their graph
(research R3). No toolchain change is needed — stable is already 1.96.

---

## `git_log` — repository history (US5, feature `gitlog`)

Engine: `gix` 0.86, `default-features = false`, features `revision` + `blame`.

```rust
#[derive(clap::Args, Debug, Clone)]
pub struct GitLogArgs {
    /// Path whose history is wanted. Resolved inside the scope.
    #[arg(long)]
    pub path: PathBuf,
    /// `log` → recent commits touching the path. `blame` → origin of a line.
    #[arg(long, default_value = "log")]
    pub mode: HistoryMode,
    /// 1-indexed line; required when mode = blame.
    #[arg(long)]
    pub line: Option<u32>,
    #[arg(long, default_value_t = 20)]
    pub limit: usize,
}
```

Returns `ToolOutcome<Vec<Commit>>` — id, author, time, summary.

**No `git` binary in the allowlist.** Shelling out would admit a binary whose flag surface includes
`-c core.pager=<cmd>` and similar, for what is a read-only local query (research R11).

**Not a repository** returns `Unavailable { NotARepository { path } }`, never an empty history —
US5 scenario 3, and the FR-012 invariant applied to the one case where "no history" is genuinely
ambiguous.

**Pin `gix` exactly.** Its API churns across minor versions more than the other crates here; expect
to touch call sites on upgrade.

---

## `cvss` — severity scoring (US4, feature `cvss`)

Engine: `cvss` 2.2, features `v3` + `v4`.

```rust
pub struct CvssArgs {
    /// e.g. "CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H" or a v4.0 vector.
    pub vector: String,
    /// Optional: attach the computed result to this finding (emits LedgerEvent::Scored).
    pub finding: Option<FindingId>,
}
```

Returns `ToolOutcome<Severity>` — the parsed vector, the **computed** score, and its band.

**No worker, no child.** It opens no files and executes nothing; it is pure computation over a
string, so the sandboxed-child seam has nothing to protect and would only add a process spawn.

**FR-005 is the whole point.** The model supplies the vector — characterising attack vector,
complexity, privileges, impact — which is judgement it is good at. The *arithmetic* is a multi-step
floating-point calculation with scope-conditional coefficients, which it is not good at. The crate
computes it.

A submitted `Severity` carrying a caller-supplied `score` is **rejected**, not silently recomputed.
Silent recomputation would make the violation invisible to both the caller and to review.

**Coverage**: `v3::Base` (CVSS v3.1) and `v4::Vector` (CVSS v4.0). Temporal and Environmental groups
are a TODO in the crate; Base scoring is what FR-005 requires (research R10).

A malformed vector returns `Failed` with the parse diagnostic — not a zero score, and not a guess.

---

## `sarif-worker` — the normaliser (feature `scanners`)

Not a model-facing tool. The second child of the scanner pipeline
([`scanner-adapter.md`](./scanner-adapter.md)).

```rust
pub struct SarifArgs {
    /// The report child 1 wrote. Read INSIDE the scope — the harness never opens it.
    #[arg(long)]
    pub report: PathBuf,
    /// Source name recorded on each finding: `scanner:<name>`.
    #[arg(long)]
    pub source: String,
    #[arg(long, default_value_t = 200)]
    pub limit: usize,
}
```

Reads the SARIF, projects the documented subset (see [`../data-model.md`](../data-model.md) §Scanner
Report), bounds the set, and writes structured findings to stdout.

**Minimal structs, not `serde-sarif`** (research R5): the full 2.1.0 schema would materialise all
1074 rule objects to reach 839 bytes of results. Serde ignores unknown fields for free.

**Bounding happens here**, before rendering and before the transport cap — so `DEFAULT_OUTPUT_CAP`
never has to cut a structured result mid-object, which is the failure R4 exists to prevent.

---

## Registration and feature gating

`DEFAULT_TOOLS` is unchanged. The new tools follow the `RENDER_TOOLS` / `SKILL_TOOLS` precedent in
`src/tools.rs` — named constants, opt-in per scenario or REPL config:

```rust
/// Native security analysis (016). Opt-in; each name is additionally feature-gated.
pub const SEC_TOOLS: &[&str] = &["ast_grep", "git_log", "cvss", "record_finding", "list_findings"];
/// External scanner tier (016). Requires a ScannerGrant in policy as well as this opt-in.
pub const SCANNER_TOOLS: &[&str] = &["scan"];
```

`is_known_tool` accepts them so scenario validation passes. A name whose feature is compiled out is
still *known* — it registers a stub returning `Unavailable { NotCompiledIn { family } }`, so a
scenario referencing it fails loudly at the tool call rather than silently at validation, and never
looks like a clean scan (spec Edge Case "the build was slimmed down").
