# Contract: Tool Outcome (the fail-closed result type)

The single most load-bearing contract in this feature. It is what makes FR-012 structural rather
than a matter of discipline, and it is the direct descendant of `a32e156` — *a hook that cannot
evaluate an operation refuses it* — applied one layer up, at the tool surface.

## The type

```rust
/// Every security tool in 016 returns this. `T` is the tool's success payload
/// (`Vec<Finding>`, `Vec<Match>`, `Severity`, …).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ToolOutcome<T> {
    /// The tool ran to completion. An empty `value` means **ran and found nothing** —
    /// a real, trustworthy, negative result.
    Completed { value: T, truncated: bool },

    /// The tool could not run at all. Never conflatable with a clean scan.
    Unavailable { reason: UnavailableReason },

    /// The tool ran and broke partway. Any partial output is discarded, not reported.
    Failed { reason: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum UnavailableReason {
    /// The Cargo feature for this tool family was not compiled in.
    NotCompiledIn { family: &'static str },
    /// No such binary at the granted path.
    BinaryMissing { path: PathBuf },
    /// The episode holds no grant for this scanner.
    NotGranted { name: String },
    /// The binary at the granted path no longer matches its pinned identity (FR-009).
    PinMismatch { path: PathBuf },
    /// A required provisioned artefact is absent or the wrong version (FR-011).
    BundleMismatch { expected: String, found: Option<String> },
    /// The requested language has no grammar in this build.
    LanguageUnsupported { lang: String, compiled_in: Vec<String> },
    /// The path is not inside a repository (US5 scenario 3).
    NotARepository { path: PathBuf },
}
```

## The invariant

> `Completed { value: <empty>, .. }` is reachable **only** from a run that genuinely completed and
> genuinely found nothing.

Every error path produces `Unavailable` or `Failed`. There is no code path from an error to an empty
`Completed`.

This is enforced three ways:

1. **Type-level.** Construction sites for `Completed` are confined to the success arm of each tool's
   worker. `?` on any fallible step yields `Failed`; a probe failure yields `Unavailable`.
2. **Test.** `tests/scanner_adapter.rs` asserts each of the seven `UnavailableReason` variants is
   produced by its triggering condition and that none of them serialises to something a caller could
   mistake for a clean scan.
3. **Rendering.** The model-facing and operator-facing renderings of `Unavailable` and `Failed` never
   contain the phrase used for a clean result. A clean scan reads *"scanned N files, no findings"*;
   an unavailable one reads *"did not run: <reason>"*.

## Why `truncated` is on `Completed` and not a fourth variant

A truncated-but-valid result is still a completed run — the caller needs the partial findings *and*
needs to know the set is incomplete (FR-013). Making it a separate variant would force callers to
discard usable data. Making it a silent bool default would let incompleteness pass unnoticed, so the
field is non-defaulted in the model-facing rendering: when `truncated` is true, the rendering says so
in the first line, before the results.

## Mapping onto `ToolResult`

The existing `ToolResult` (`src/tools.rs`) is the transport. `ToolOutcome` maps onto it:

| Outcome | `is_error` | `content` |
|---|---|---|
| `Completed { truncated: false }` | `false` | rendered findings, or the explicit no-findings line |
| `Completed { truncated: true }` | `false` | truncation notice, then rendered findings |
| `Unavailable` | `true` | `did not run: <reason>` |
| `Failed` | `true` | `failed: <reason>` |

`ToolResult::truncated` continues to mean *the transport capped the string*, which is a different and
lower-level fact than `ToolOutcome::truncated` (*the result set was bounded before rendering*). Both
can be true; the rendering distinguishes them.

## Audit obligation (FR-014)

Every `Unavailable` and every `Failed` emits a structured audit event before returning. The event
carries the tool name, the reason variant, and — for `PinMismatch` and `NotGranted` — enough detail
to investigate, since those two are the security-relevant refusals rather than mere absence.
