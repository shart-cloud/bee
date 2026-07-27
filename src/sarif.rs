//! SARIF ingestion: a third-party scanner's report, normalised into bee findings
//! (016-native-tools US3).
//!
//! ## Why bee models SARIF itself instead of using `serde-sarif`
//!
//! Measured (research R4/R5): an Opengrep SARIF over a single three-line Python file is
//! **1,912,546 bytes**, of which **99.96% is the embedded rule catalogue** — 1074 rule objects
//! carrying descriptions, help text, and tags — to deliver **839 bytes** of results. A faithful
//! model of the 2.1.0 schema materialises all of it to reach the part that matters. The structs
//! below name only the fields bee reads; serde ignores the rest, so `tool.driver.rules` is never
//! allocated at all.
//!
//! ## Why this runs in a child rather than in the harness
//!
//! The same measurement forces the shape of the pipeline. 1.9 MB against a 102,400-byte
//! `DEFAULT_OUTPUT_CAP` means capturing the report on stdout truncates it into unparseable JSON —
//! which would surface as "the scanner found nothing", the exact failure FR-012 exists to prevent.
//! So the scanner writes a file and `bee sarif-worker` reads it **inside the scope**. Having the
//! harness read that file instead would be reading around the sandbox (Constitution III), so the
//! normaliser is a second scope-joined child and the output is bounded before it is ever rendered.
//!
//! ## Everything in here is untrusted
//!
//! `message.text` and `region.snippet.text` are verbatim attacker-controlled source. They enter the
//! ledger as **data**, are escaped by [`crate::safe_text`] at every front-end (FR-015), and a
//! scanner's `level` is recorded as advisory only — it never becomes a [`Severity`], because
//! severity comes from the computed path alone (FR-005).

use serde::Deserialize;

use crate::findings::{Finding, FindingSource};

/// The cap on findings one scan contributes. Matches `search`'s `MATCH_LIMIT` precedent, and bites
/// **before** rendering so the transport cap never has to cut a structured result mid-object.
pub const MAX_FINDINGS_PER_SCAN: usize = 200;

/// The path recorded for a finding the report gives no location for — a whole-program result.
/// Kept rather than dropped: a result with no location is still a result, and silently discarding
/// one would under-report a scan that ran correctly.
pub const WHOLE_PROGRAM: &str = "(whole program)";

// ── The subset bee reads (data-model.md §Scanner Report) ─────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct Report {
    #[serde(default)]
    runs: Vec<Run>,
}

#[derive(Debug, Deserialize)]
struct Run {
    #[serde(default)]
    invocations: Vec<Invocation>,
    #[serde(default)]
    results: Vec<SarifResult>,
}

#[derive(Debug, Deserialize)]
struct Invocation {
    /// Whether the scanner considers its own run to have succeeded. **This, not the exit status, is
    /// the signal** (research R6).
    #[serde(rename = "executionSuccessful", default)]
    execution_successful: bool,
}

#[derive(Debug, Deserialize)]
struct SarifResult {
    #[serde(rename = "ruleId", default)]
    rule_id: Option<String>,
    /// The scanner's own severity. Advisory only.
    #[serde(default)]
    level: Option<String>,
    #[serde(default)]
    message: Option<Message>,
    #[serde(default)]
    locations: Vec<Location>,
}

#[derive(Debug, Deserialize)]
struct Message {
    #[serde(default)]
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Location {
    #[serde(rename = "physicalLocation", default)]
    physical_location: Option<PhysicalLocation>,
}

#[derive(Debug, Deserialize)]
struct PhysicalLocation {
    #[serde(rename = "artifactLocation", default)]
    artifact_location: Option<ArtifactLocation>,
    #[serde(default)]
    region: Option<Region>,
}

#[derive(Debug, Deserialize)]
struct ArtifactLocation {
    #[serde(default)]
    uri: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Region {
    #[serde(rename = "startLine", default)]
    start_line: Option<u32>,
    #[serde(rename = "endLine", default)]
    end_line: Option<u32>,
    #[serde(default)]
    snippet: Option<Snippet>,
}

#[derive(Debug, Deserialize)]
struct Snippet {
    #[serde(default)]
    text: Option<String>,
}

// ── Normalised output ────────────────────────────────────────────────────────────────────────────

/// One normalised finding plus the location the report gave for it. The location travels beside the
/// finding rather than inside it because a line number is an attribute of a *sighting*, not of a
/// finding's identity (contract `finding-ledger.md`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, Deserialize)]
pub struct ScanFinding {
    pub path: String,
    pub class: String,
    pub title: String,
    pub evidence: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_line: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_id: Option<String>,
    /// The scanner's own `level`, recorded verbatim and never promoted to a severity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub advisory_level: Option<String>,
}

impl ScanFinding {
    /// The ledger record this represents, attributed to the scanner that produced it.
    pub fn to_finding(&self, scanner: &str) -> Finding {
        let mut f = Finding::new(
            self.path.clone(),
            self.class.clone(),
            self.title.clone(),
            self.evidence.clone(),
            FindingSource::Scanner(scanner.to_string()),
        );
        f.advisory_level = self.advisory_level.clone();
        f.reidentify();
        f
    }
}

/// What one report yielded.
#[derive(Debug, Clone, Default, serde::Serialize, Deserialize)]
pub struct ScanOutcome {
    /// Whether the scanner said its own run succeeded. **False when the report says nothing**: a
    /// missing `executionSuccessful` is not evidence of success (Constitution I).
    pub execution_successful: bool,
    pub findings: Vec<ScanFinding>,
    /// Set when [`MAX_FINDINGS_PER_SCAN`] bit, so incompleteness is stated rather than inferred.
    pub truncated: bool,
}

/// Parse a SARIF document and normalise its results, bounded at `limit`.
///
/// Returns `Err` with the parse diagnostic for a document that does not parse — deliberately not an
/// empty result, which would be indistinguishable from a scanner that ran cleanly.
pub fn normalise(raw: &str, _scanner: &str, limit: usize) -> Result<ScanOutcome, String> {
    let report: Report =
        serde_json::from_str(raw).map_err(|e| format!("unparseable report: {e}"))?;

    // Success is unanimous or it is not success: one run reporting failure taints the report, since
    // the results of a failed run are by definition partial.
    let invocations: Vec<&Invocation> = report.runs.iter().flat_map(|r| &r.invocations).collect();
    let execution_successful =
        !invocations.is_empty() && invocations.iter().all(|i| i.execution_successful);

    let mut findings = Vec::new();
    let mut truncated = false;
    for run in &report.runs {
        for result in &run.results {
            if findings.len() >= limit {
                truncated = true;
                break;
            }
            findings.push(normalise_result(result));
        }
        if truncated {
            break;
        }
    }

    Ok(ScanOutcome {
        execution_successful,
        findings,
        truncated,
    })
}

fn normalise_result(result: &SarifResult) -> ScanFinding {
    let physical = result
        .locations
        .first()
        .and_then(|l| l.physical_location.as_ref());
    let region = physical.and_then(|p| p.region.as_ref());

    let path = physical
        .and_then(|p| p.artifact_location.as_ref())
        .and_then(|a| a.uri.as_deref())
        .map(crate::findings::normalise_path)
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| WHOLE_PROGRAM.to_string());

    let title = result
        .message
        .as_ref()
        .and_then(|m| m.text.clone())
        .filter(|t| !t.trim().is_empty())
        // A result with no message still has to be recordable — validation requires a non-empty
        // title, and refusing the whole report over one terse result would lose the others.
        .unwrap_or_else(|| {
            format!(
                "{} reported an issue with no message",
                result.rule_id.as_deref().unwrap_or("the scanner")
            )
        });

    let evidence = region
        .and_then(|r| r.snippet.as_ref())
        .and_then(|s| s.text.clone())
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| match (&result.rule_id, &result.level) {
            (Some(rule), Some(level)) => format!("rule {rule} fired at {level} with no snippet"),
            (Some(rule), None) => format!("rule {rule} fired with no snippet"),
            _ => "the scanner reported no snippet".to_string(),
        });

    ScanFinding {
        // The rule id doubles as the issue class: it is the scanner's own taxonomy, which is more
        // specific than anything bee could infer, and it keeps two runs of the same rule merging.
        class: result
            .rule_id
            .clone()
            .unwrap_or_else(|| "unclassified".to_string()),
        path,
        title,
        evidence,
        line: region.and_then(|r| r.start_line),
        end_line: region.and_then(|r| r.end_line),
        rule_id: result.rule_id.clone(),
        advisory_level: result.level.clone(),
    }
}

// ── The worker (`bee sarif-worker`) ──────────────────────────────────────────────────────────────

/// Arguments for the `sarif-worker` subcommand.
#[derive(clap::Args, Debug, Clone)]
pub struct SarifArgs {
    /// The report to read. Opened **inside the scope**, which is the point of this subcommand.
    #[arg(long)]
    pub report: std::path::PathBuf,
    /// The scanner that produced it, for attribution.
    #[arg(long)]
    pub source: String,
    /// Cap on findings emitted.
    #[arg(long, default_value_t = MAX_FINDINGS_PER_SCAN)]
    pub limit: usize,
}

/// Read, normalise, and print — one JSON object per line.
///
/// JSONL rather than one array, deliberately. The set is already bounded, so the transport cap
/// should never bite; if it ever did, a cut array fails to parse *entirely* and the scan reads as
/// empty, whereas a cut stream of lines costs only the last record. The failure mode this feature
/// exists to prevent deserves two independent defences.
pub fn run_worker(args: &SarifArgs) -> Result<(), String> {
    let raw = std::fs::read_to_string(&args.report)
        .map_err(|e| format!("cannot read {}: {e}", args.report.display()))?;
    let outcome = normalise(&raw, &args.source, args.limit)?;

    // The header line carries what the findings cannot: whether the scan ran at all, and whether the
    // set was cut. A consumer that saw only findings could not tell an empty scan from a failed one.
    let header = serde_json::json!({
        "kind": "scan_outcome",
        "execution_successful": outcome.execution_successful,
        "truncated": outcome.truncated,
        "count": outcome.findings.len(),
    });
    println!("{header}");
    for f in &outcome.findings {
        match serde_json::to_string(f) {
            Ok(line) => println!("{line}"),
            Err(e) => return Err(format!("could not serialise a finding: {e}")),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_documented_subset_is_read() {
        // A report carrying a large rule catalogue normalises without materialising it. The proof
        // that matters is behavioural: unknown fields are ignored and the result is unaffected.
        let report = r#"{"version":"2.1.0","runs":[{
            "tool":{"driver":{"name":"Opengrep","rules":[{"id":"r1","help":{"text":"…"}}]}},
            "invocations":[{"executionSuccessful":true,"toolExecutionNotifications":[]}],
            "results":[{"ruleId":"r1","message":{"text":"m"},"fingerprints":{"x":"y"},
                "locations":[{"physicalLocation":{"artifactLocation":{"uri":"a.py","uriBaseId":"%SRCROOT%"},
                "region":{"startLine":2,"endLine":2,"snippet":{"text":"code"}}}}]}]}]}"#;
        let out = normalise(report, "opengrep", 10).unwrap();
        assert!(out.execution_successful);
        assert_eq!(out.findings.len(), 1);
        assert_eq!(out.findings[0].line, Some(2));
        assert_eq!(out.findings[0].evidence, "code");
    }

    #[test]
    fn an_unparseable_document_is_an_error_not_an_empty_scan() {
        assert!(normalise("{ not json", "opengrep", 10).is_err());
        assert!(normalise("", "opengrep", 10).is_err());
    }

    #[test]
    fn one_failed_run_taints_the_report() {
        let report = r#"{"runs":[
            {"invocations":[{"executionSuccessful":true}],"results":[]},
            {"invocations":[{"executionSuccessful":false}],"results":[]}]}"#;
        assert!(
            !normalise(report, "opengrep", 10)
                .unwrap()
                .execution_successful
        );
    }

    #[test]
    fn a_result_without_a_message_still_records_something_reviewable() {
        // Validation requires a non-empty title and evidence; a terse result must not make the
        // whole report unrecordable.
        let report = r#"{"runs":[{"invocations":[{"executionSuccessful":true}],
            "results":[{"ruleId":"r1"}]}]}"#;
        let out = normalise(report, "opengrep", 10).unwrap();
        let f = out.findings[0].to_finding("opengrep");
        assert!(crate::findings::validate(&f).is_ok(), "{f:?}");
    }

    #[test]
    fn the_scanner_level_is_advisory_and_never_a_severity() {
        let report = r#"{"runs":[{"invocations":[{"executionSuccessful":true}],
            "results":[{"ruleId":"r1","level":"error","message":{"text":"m"}}]}]}"#;
        let out = normalise(report, "opengrep", 10).unwrap();
        assert_eq!(out.findings[0].advisory_level.as_deref(), Some("error"));
        let f = out.findings[0].to_finding("opengrep");
        assert_eq!(f.advisory_level.as_deref(), Some("error"));
        assert!(f.severity.is_none(), "a level must never become a score");
    }

    #[test]
    fn a_scanner_relative_path_normalises_like_every_other_finding_path() {
        let report = r#"{"runs":[{"invocations":[{"executionSuccessful":true}],
            "results":[{"ruleId":"r1","message":{"text":"m"},
            "locations":[{"physicalLocation":{"artifactLocation":{"uri":"./src/a.py"}}}]}]}]}"#;
        let out = normalise(report, "opengrep", 10).unwrap();
        assert_eq!(out.findings[0].path, "src/a.py");
    }
}
