//! 016-native-tools — the fail-closed gate (Constitution I; FR-012, SC-002).
//!
//! The requirement these guard is one sentence: **a tool that cannot run must never look like a tool
//! that ran and found nothing.** A missing scanner reporting "no issues" is the worst failure mode a
//! security harness has, because the operator reads the silence as assurance.
//!
//! The constitution's own words: "A denial that no test exercises is not considered enforced." These
//! are that exercise for the tool layer, the same way `tests/fail_closed.rs` is for the kernel layer.

use bee::tools::outcome::{
    clean_scan, ToolOutcome, UnavailableReason, CLEAN_PREFIX, FAILED_PREFIX, UNAVAILABLE_PREFIX,
};
use bee::tools::{is_known_tool, sec_tool_family, SCANNER_TOOLS, SEC_TOOLS};

/// Every `UnavailableReason` variant, so a new one cannot be added without deciding how it renders.
fn every_reason() -> Vec<UnavailableReason> {
    vec![
        UnavailableReason::NotCompiledIn {
            family: "scanners".into(),
        },
        UnavailableReason::BinaryMissing {
            path: "/usr/bin/nope".into(),
        },
        UnavailableReason::NotGranted {
            name: "opengrep".into(),
        },
        UnavailableReason::PinMismatch {
            path: "/usr/bin/opengrep".into(),
        },
        UnavailableReason::BundleMismatch {
            expected: "codeql-bundle-v2.26.1".into(),
            found: Some("codeql-bundle-v2.20.0".into()),
        },
        UnavailableReason::LanguageUnsupported {
            lang: "python".into(),
            compiled_in: vec!["rust".into()],
        },
        UnavailableReason::NotARepository {
            path: "/tmp/notarepo".into(),
        },
    ]
}

fn render_empty(outcome: ToolOutcome<Vec<String>>) -> bee::tools::ToolResult {
    outcome.into_tool_result(|v| clean_scan(v.len(), "files"))
}

#[test]
fn no_unavailable_rendering_can_be_mistaken_for_a_clean_scan() {
    // A clean scan is the baseline: not an error, and carrying the clean phrase.
    let clean = render_empty(ToolOutcome::completed(Vec::new()));
    assert!(!clean.is_error);
    assert!(clean.content.contains(CLEAN_PREFIX), "{}", clean.content);

    for reason in every_reason() {
        let rendered = render_empty(ToolOutcome::unavailable(reason.clone()));

        assert!(
            rendered.is_error,
            "{reason:?} rendered as a non-error, which the loop would treat as success"
        );
        assert!(
            !rendered.content.contains(CLEAN_PREFIX),
            "{reason:?} rendered containing the clean-scan phrase {CLEAN_PREFIX:?}: {}",
            rendered.content
        );
        assert!(
            rendered.content.starts_with(UNAVAILABLE_PREFIX),
            "{reason:?} did not announce itself as a non-run: {}",
            rendered.content
        );
        // The rendering must be actionable, not just negative — an operator has to know what to fix.
        assert!(
            rendered.content.len() > UNAVAILABLE_PREFIX.len() + 8,
            "{reason:?} rendered without a usable detail: {}",
            rendered.content
        );
    }
}

#[test]
fn a_failed_run_is_an_error_and_reads_as_one() {
    let rendered = render_empty(ToolOutcome::failed("unparseable report"));
    assert!(rendered.is_error);
    assert!(rendered.content.starts_with(FAILED_PREFIX));
    assert!(!rendered.content.contains(CLEAN_PREFIX));
}

#[test]
fn the_three_states_are_mutually_distinguishable() {
    let clean = render_empty(ToolOutcome::completed(Vec::new()));
    let unavailable = render_empty(ToolOutcome::unavailable(UnavailableReason::NotGranted {
        name: "opengrep".into(),
    }));
    let failed = render_empty(ToolOutcome::failed("timeout"));

    // Pairwise distinct content, and the two refusals are errors while the clean run is not.
    assert_ne!(clean.content, unavailable.content);
    assert_ne!(clean.content, failed.content);
    assert_ne!(unavailable.content, failed.content);
    assert!(!clean.is_error && unavailable.is_error && failed.is_error);
}

#[test]
fn a_refusal_never_renders_a_payload() {
    // Structural form of "Completed-with-empty is unreachable from an error path": the renderer
    // closure is the only way a payload becomes text, and it must not run on a refusal.
    for reason in every_reason() {
        let outcome: ToolOutcome<Vec<String>> = ToolOutcome::unavailable(reason.clone());
        let _ = outcome.into_tool_result(|_| panic!("renderer ran for {reason:?}"));
    }
    let outcome: ToolOutcome<Vec<String>> = ToolOutcome::failed("boom");
    let _ = outcome.into_tool_result(|_| panic!("renderer ran on Failed"));
}

#[test]
fn truncation_is_announced_before_the_results() {
    let outcome = ToolOutcome::completed_truncated(vec!["finding one".to_string()], true);
    let rendered = outcome.into_tool_result(|v| v.join("\n"));
    assert!(
        !rendered.is_error,
        "a bounded result is still a real result"
    );
    let first = rendered.content.lines().next().unwrap();
    assert!(
        first.contains("incomplete"),
        "truncation must be the first thing read, got: {first}"
    );
}

#[test]
fn only_grant_relevant_refusals_are_flagged_security_relevant() {
    // PinMismatch and NotGranted mean an authorisation boundary was touched. The rest mean the
    // machine is not set up. Conflating them would drown the signal.
    for reason in every_reason() {
        let expected = matches!(
            reason,
            UnavailableReason::PinMismatch { .. } | UnavailableReason::NotGranted { .. }
        );
        assert_eq!(
            reason.is_security_relevant(),
            expected,
            "{reason:?} classified wrongly"
        );
    }
}

#[test]
fn every_reason_has_a_distinct_audit_slug() {
    let mut slugs: Vec<&str> = every_reason().iter().map(|r| r.slug()).collect();
    let before = slugs.len();
    slugs.sort_unstable();
    slugs.dedup();
    assert_eq!(before, slugs.len(), "two reasons share an audit slug");
}

#[test]
fn sec_tool_names_are_known_so_scenarios_do_not_drop_them() {
    // A 016 tool must be *known* even when its feature is compiled out, so a scenario referencing it
    // fails loudly at the call rather than silently at validation.
    for name in SEC_TOOLS.iter().chain(SCANNER_TOOLS.iter()) {
        assert!(is_known_tool(name), "{name} is not a known tool name");
        assert!(
            sec_tool_family(name).is_some(),
            "{name} has no feature family, so its NotCompiledIn message cannot name a fix"
        );
    }
}

#[test]
fn a_compiled_out_family_refuses_rather_than_returning_nothing() {
    // Exercises the registered stub end to end for whichever families this build has off. When every
    // family is on, the loop body does not run and the test still documents the requirement.
    let build_has = |family: &str| match family {
        "astgrep" => cfg!(feature = "astgrep"),
        "gitlog" => cfg!(feature = "gitlog"),
        "cvss" => cfg!(feature = "cvss"),
        "findings" => cfg!(feature = "findings"),
        "scanners" => cfg!(feature = "scanners"),
        _ => unreachable!("unknown family"),
    };

    for name in SEC_TOOLS.iter().chain(SCANNER_TOOLS.iter()) {
        let family = sec_tool_family(name).unwrap();
        if build_has(family) {
            continue;
        }
        let reason = UnavailableReason::NotCompiledIn {
            family: family.to_string(),
        };
        let rendered = render_empty(ToolOutcome::unavailable(reason));
        assert!(rendered.is_error, "{name} did not refuse");
        assert!(
            !rendered.content.contains(CLEAN_PREFIX),
            "{name} read clean"
        );
        // The message must name the feature that would fix it.
        assert!(
            rendered.content.contains(family),
            "{name} did not name the missing feature: {}",
            rendered.content
        );
    }
}
