//! The CodeQL adapter (016-native-tools US6).
//!
//! CodeQL is the deepest corpus bee can borrow — whole-program dataflow rather than pattern matching
//! — and it is also the one that asks the most of the operator: a provisioned, version-pinned
//! bundle, and a language whose facts can be extracted without watching a build.
//!
//! ## Why build-mode `none`, and nothing else
//!
//! CodeQL extracts a compiled language by **intercepting the build's process spawns**
//! (`--begin-tracing` / `--trace-process-name`, `codeql-action/src/codeql.ts:557`). Admitting that
//! means admitting every compiler, linker, and build tool the target's build happens to invoke —
//! an unbounded widening bee cannot attenuate and would not be able to describe to the operator it
//! asked. So bee builds databases one way, `--build-mode=none`, and declines the rest explicitly
//! (research R7, FR-011). Half-support would be worse than none: a traced language analysed without
//! tracing yields a thin database, and a thin database yields few findings, which reads exactly like
//! clean code.
//!
//! ## The supported set is a list, and it is deliberately short
//!
//! CodeQL itself decides tracedness by a filesystem fact — `isTracedLanguage`
//! (`codeql-action/src/codeql.ts:535`) stats `<extractor>/tools/tracing-config.lua` inside the
//! provisioned distribution. bee cannot read that from the harness (Constitution III), and asking
//! the CLI would cost two more children per scan, so [`BUILDLESS_LANGUAGES`] is a static list
//! instead. That trades freshness for simplicity, and it is safe to trade because it **fails
//! closed**: a language bee does not list is declined, so the list going stale costs coverage and
//! never correctness. Adding to it is a code change with a bundle to check against.
//!
//! Measured against **`codeql-bundle-v2.26.1`** / CLI **2.26.1**, the pin `codeql-action` v4.37.3
//! carries in `src/defaults.json`.

use std::path::{Path, PathBuf};

use super::{probe_binary, validate_target, ScanRequest, ScannerAdapter, ScannerGrant};
use crate::tools::outcome::{UnavailableReason, UNPINNED};

pub struct CodeQl;

/// The languages bee will build a database for, because none of them needs a build observed.
///
/// Evidence, all from `codeql-action` v4.37.3:
///
/// * `actions`, `javascript`, `python`, `ruby` are *scanned* languages — never traced under any
///   build mode, so `--build-mode=none` is the only mode they have.
/// * `java` and `csharp` have first-class `build-mode: none` extractors; the action carries
///   dedicated handling for where each puts its resolved dependencies (`src/analyze.ts:129-147`).
///
/// Absent, and each for a reason: `cpp` and `swift` are traced; `go` is documented as not yet
/// supporting build-mode none (`src/config-utils.ts:849`); `rust` is a builtin language and very
/// likely buildless, but "likely" is not evidence, and the cost of being wrong is a scan that
/// quietly under-reports. It goes in when a real bundle says so.
pub const BUILDLESS_LANGUAGES: &[&str] =
    &["actions", "csharp", "java", "javascript", "python", "ruby"];

/// Every language CodeQL itself knows, so bee can tell "I decline that" from "there is no such
/// thing" (`codeql-action/src/languages/builtin.json`).
const BUILTIN_LANGUAGES: &[&str] = &[
    "actions",
    "cpp",
    "csharp",
    "go",
    "java",
    "javascript",
    "python",
    "ruby",
    "rust",
    "swift",
];

/// CodeQL's own spellings for the same extractor, from the `aliases` map in `builtin.json`. Applied
/// before the supported-set check so a caller asking for `typescript` is not told TypeScript is
/// unanalysable when `javascript` is the extractor that handles it.
const ALIASES: &[(&str, &str)] = &[
    ("c", "cpp"),
    ("c-c++", "cpp"),
    ("c-cpp", "cpp"),
    ("c#", "csharp"),
    ("c++", "cpp"),
    ("java-kotlin", "java"),
    ("javascript-typescript", "javascript"),
    ("kotlin", "java"),
    ("typescript", "javascript"),
];

/// Resolve a caller's language name to the extractor CodeQL would use.
fn canonical(lang: &str) -> String {
    let lower = lang.trim().to_ascii_lowercase();
    ALIASES
        .iter()
        .find(|(from, _)| *from == lower)
        .map(|(_, to)| (*to).to_string())
        .unwrap_or(lower)
}

fn supported() -> Vec<String> {
    BUILDLESS_LANGUAGES.iter().map(|s| s.to_string()).collect()
}

/// Reduce a bundle version to the CLI version it contains, so an operator may pin either spelling.
///
/// `codeql-action` carries both — `"bundleVersion": "codeql-bundle-v2.26.1"` alongside
/// `"cliVersion": "2.26.1"` — and an operator provisioning a bundle has the first to hand while the
/// CLI only ever reports the second. Accepting one and rejecting the other would turn a correct pin
/// into a mismatch.
fn cli_version_of(pin: &str) -> &str {
    let pin = pin.trim();
    let pin = pin.strip_prefix("codeql-bundle-").unwrap_or(pin);
    pin.strip_prefix('v').unwrap_or(pin)
}

impl ScannerAdapter for CodeQl {
    fn name(&self) -> &'static str {
        "codeql"
    }

    /// Four questions, none of which costs a process: is the pinned binary still the pinned binary,
    /// was a version pinned at all, is the bundle where the operator said, and is this a language
    /// bee will analyse. Each is a reason the scan *could not run* — so each is `Unavailable`, and
    /// none of them is reported as a scan that ran and broke.
    fn probe(&self, grant: &ScannerGrant, req: &ScanRequest) -> Result<(), UnavailableReason> {
        probe_binary(grant)?;

        // Was a version pinned at all. This is a refusal to *run*, not a run that broke — nothing
        // has executed at this point — so it is `Unavailable`, and it belongs here rather than in
        // `steps`, which is reached only after this method has already passed the request as
        // answerable (contract `scanner-adapter.md`).
        if grant.bundle_version.is_none() {
            return Err(UnavailableReason::BundleMismatch {
                expected: UNPINNED.to_string(),
                found: None,
            });
        }

        // A bundle path is optional — the granted binary is the bundle's CLI, so the pin already
        // covers what actually runs. When the operator *does* name one, it has to be the bundle the
        // granted binary came out of, or the version verified below belongs to a different CodeQL
        // than the one that will do the scanning.
        if let Some(bundle) = &grant.bundle {
            // Unwrap-free: the no-pin case returned above, so a pin is present by construction.
            let expected = grant
                .bundle_version
                .clone()
                .unwrap_or_else(|| UNPINNED.to_string());
            if !bundle.exists() {
                return Err(UnavailableReason::BundleMismatch {
                    expected,
                    found: None,
                });
            }
            if !grant.path.starts_with(bundle) {
                return Err(UnavailableReason::BundleMismatch {
                    expected,
                    found: Some(format!(
                        "the granted binary {} is not inside the configured bundle {}",
                        grant.path.display(),
                        bundle.display()
                    )),
                });
            }
        }

        let Some(lang) = req.lang.as_deref() else {
            return Err(UnavailableReason::LanguageUnsupported {
                lang: "(none specified)".to_string(),
                compiled_in: supported(),
            });
        };
        let lang = canonical(lang);
        if BUILDLESS_LANGUAGES.contains(&lang.as_str()) {
            return Ok(());
        }
        // A language CodeQL has an extractor for, which bee declines on purpose, is a different
        // answer from one nothing has ever heard of — and the caller can act on the difference.
        if BUILTIN_LANGUAGES.contains(&lang.as_str()) {
            return Err(UnavailableReason::LanguageRequiresBuild {
                lang,
                supported: supported(),
            });
        }
        Err(UnavailableReason::LanguageUnsupported {
            lang,
            compiled_in: supported(),
        })
    }

    /// Ask the CLI its version. This is a child like any other, in scope, because asking a binary
    /// what it is means running it — and running it from the harness would run it around the
    /// sandbox (Constitution III).
    ///
    /// Returns `None` when no version was pinned: there is then nothing to compare against, and
    /// [`ScannerAdapter::steps`] refuses to build a command at all, so an unverified bundle is never
    /// reached by a different route.
    fn preflight(&self, grant: &ScannerGrant) -> Option<Vec<String>> {
        grant.bundle_version.as_ref()?;
        Some(vec!["version".to_string(), "--format=json".to_string()])
    }

    fn verify_preflight(
        &self,
        grant: &ScannerGrant,
        stdout: &str,
    ) -> Result<(), UnavailableReason> {
        let Some(pin) = grant.bundle_version.as_deref() else {
            return Ok(()); // no preflight was run; `steps` refuses below
        };
        let expected = pin.to_string();

        // Unreadable output is a mismatch, not a pass. A CLI that cannot say what version it is has
        // not said it is the right one (Constitution I).
        let found = serde_json::from_str::<serde_json::Value>(stdout)
            .ok()
            .and_then(|v| v["version"].as_str().map(str::to_string));
        let Some(found) = found else {
            return Err(UnavailableReason::BundleMismatch {
                expected,
                found: None,
            });
        };

        if cli_version_of(&found) == cli_version_of(pin) {
            Ok(())
        } else {
            Err(UnavailableReason::BundleMismatch {
                expected,
                found: Some(found),
            })
        }
    }

    /// Two children, because a CodeQL analysis is two operations:
    ///
    /// ```text
    /// database create  <db> --language=<lang> --build-mode=none --source-root=<target>
    /// database analyze <db> --format=sarif-latest --output=<out> <suite>
    /// ```
    ///
    /// The database goes in the per-call scratch directory, so two scans in one episode cannot land
    /// on each other and nothing survives the call. Every argument is a constant, a bee-chosen path,
    /// or a value already checked against [`BUILDLESS_LANGUAGES`] — the model's `lang` cannot reach
    /// the command line as anything but one of six known words.
    fn steps(
        &self,
        grant: &ScannerGrant,
        req: &ScanRequest,
        scratch: &Path,
        out: &Path,
    ) -> Result<Vec<Vec<String>>, String> {
        validate_target(&req.target)?;

        // `probe` refuses an unpinned bundle as `Unavailable` before anything reaches here, which is
        // where that refusal is *stated* — this is the same belt-and-braces the language check below
        // gets, for the same reason: `steps` authors a command line, so it re-checks what it is
        // about to write rather than trusting a caller to have asked first.
        if grant.bundle_version.is_none() {
            return Err(
                "no bundle version pinned for codeql: refusing to build a command for an \
                 unverified analysis bundle"
                    .to_string(),
            );
        }

        // `probe` has already resolved and accepted this, but `steps` does not take that on trust:
        // it is the function that authors the command line, so it checks what it is about to write.
        let lang = canonical(req.lang.as_deref().unwrap_or_default());
        if !BUILDLESS_LANGUAGES.contains(&lang.as_str()) {
            return Err(format!(
                "`{lang}` is not analysable without a build; refusing to build the command"
            ));
        }

        let db: PathBuf = scratch.join("db");

        // The query suite. Defaulting to the language's own pack is what makes the bundle worth
        // provisioning — its precompiled queries are the corpus. An operator who wants a narrower
        // set names one, and it is checked like any other configured path.
        let suite = match &grant.rules {
            Some(rules) => {
                let s = rules.to_string_lossy();
                if s.starts_with('-') {
                    return Err(format!("query suite `{s}` would be read as a flag"));
                }
                if !rules.exists() {
                    return Err(format!("query suite `{s}` does not exist"));
                }
                s.into_owned()
            }
            None => format!("codeql/{lang}-queries"),
        };

        Ok(vec![
            vec![
                "database".to_string(),
                "create".to_string(),
                db.to_string_lossy().into_owned(),
                format!("--language={lang}"),
                "--build-mode=none".to_string(),
                format!("--source-root={}", req.target.display()),
            ],
            vec![
                "database".to_string(),
                "analyze".to_string(),
                db.to_string_lossy().into_owned(),
                "--format=sarif-latest".to_string(),
                format!("--output={}", out.display()),
                suite,
            ],
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        _tmp: tempfile::TempDir,
        grant: ScannerGrant,
        target: PathBuf,
        scratch: PathBuf,
        out: PathBuf,
    }

    /// A grant that would work: the binary exists and is pinned, and a version is set.
    fn fixture() -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let bin = tmp.path().join("codeql");
        std::fs::write(&bin, "#!/bin/sh\n").unwrap();
        let target = tmp.path().join("src");
        std::fs::create_dir_all(&target).unwrap();
        let scratch = tmp.path().join("scratch");
        std::fs::create_dir_all(&scratch).unwrap();
        let out = tmp.path().join("report.sarif");
        let mut grant = ScannerGrant::for_test("codeql", bin, None);
        grant.bundle_version = Some("codeql-bundle-v2.26.1".to_string());
        Fixture {
            _tmp: tmp,
            grant,
            target,
            scratch,
            out,
        }
    }

    fn req(f: &Fixture, lang: &str) -> ScanRequest {
        let mut r = ScanRequest::new(f.target.clone());
        r.lang = Some(lang.to_string());
        r
    }

    #[test]
    fn a_bundle_and_a_cli_version_are_the_same_pin() {
        assert_eq!(cli_version_of("codeql-bundle-v2.26.1"), "2.26.1");
        assert_eq!(cli_version_of("v2.26.1"), "2.26.1");
        assert_eq!(cli_version_of("2.26.1"), "2.26.1");
    }

    #[test]
    fn an_alias_resolves_to_the_extractor_that_handles_it() {
        assert_eq!(canonical("TypeScript"), "javascript");
        assert_eq!(canonical("kotlin"), "java");
        assert_eq!(canonical("C#"), "csharp");
        assert_eq!(canonical("python"), "python");
    }

    #[test]
    fn typescript_is_analysable_because_javascript_is() {
        let f = fixture();
        assert!(CodeQl.probe(&f.grant, &req(&f, "typescript")).is_ok());
    }

    #[test]
    fn a_traced_language_is_declined_and_says_what_is_analysable() {
        let f = fixture();
        for lang in ["cpp", "c++", "go", "swift", "rust"] {
            let err = CodeQl.probe(&f.grant, &req(&f, lang)).unwrap_err();
            let UnavailableReason::LanguageRequiresBuild { supported, .. } = &err else {
                panic!("{lang} should be declined as needing a build, got {err:?}");
            };
            assert!(supported.contains(&"python".to_string()));
            // The refusal explains itself rather than just saying no.
            let shown = err.to_string();
            assert!(shown.contains("build"), "{shown}");
        }
    }

    #[test]
    fn a_language_codeql_has_never_heard_of_is_a_different_answer() {
        let f = fixture();
        assert!(matches!(
            CodeQl.probe(&f.grant, &req(&f, "cobol")),
            Err(UnavailableReason::LanguageUnsupported { .. })
        ));
    }

    #[test]
    fn no_language_at_all_is_refused_rather_than_guessed() {
        let f = fixture();
        let r = ScanRequest::new(f.target.clone());
        assert!(matches!(
            CodeQl.probe(&f.grant, &r),
            Err(UnavailableReason::LanguageUnsupported { .. })
        ));
    }

    #[test]
    fn a_bundle_that_is_not_there_is_a_mismatch_naming_the_expected_version() {
        let mut f = fixture();
        f.grant.bundle = Some(PathBuf::from("/nonexistent/codeql-bundle"));
        let err = CodeQl.probe(&f.grant, &req(&f, "python")).unwrap_err();
        let UnavailableReason::BundleMismatch { expected, found } = &err else {
            panic!("expected a bundle mismatch, got {err:?}");
        };
        assert_eq!(expected, "codeql-bundle-v2.26.1");
        assert!(found.is_none());
        assert!(err.to_string().contains("2.26.1"));
    }

    /// The refusal an unpinned bundle earns is "could not run", not "ran and broke". Nothing has
    /// executed when it fires, and the two render with different prefixes and different audit slugs
    /// — so the distinction is the operator's, not a detail of where the check happened to live.
    #[test]
    fn an_unpinned_bundle_could_not_run_rather_than_ran_and_broke() {
        let mut f = fixture();
        f.grant.bundle_version = None;
        let err = CodeQl.probe(&f.grant, &req(&f, "python")).unwrap_err();
        let UnavailableReason::BundleMismatch { expected, found } = &err else {
            panic!("expected a bundle mismatch, got {err:?}");
        };
        assert_eq!(expected, UNPINNED);
        assert!(found.is_none());
        assert_eq!(err.slug(), "bundle_mismatch");
        assert!(err.to_string().contains("bundle_version"), "{err}");
    }

    #[test]
    fn a_binary_outside_the_configured_bundle_is_refused() {
        let mut f = fixture();
        // A real directory, but not the one the granted binary lives in: the version this would
        // verify belongs to a different CodeQL than the one that would run.
        let other = f.grant.path.parent().unwrap().join("other-bundle");
        std::fs::create_dir_all(&other).unwrap();
        f.grant.bundle = Some(other);
        assert!(matches!(
            CodeQl.probe(&f.grant, &req(&f, "python")),
            Err(UnavailableReason::BundleMismatch { .. })
        ));
    }

    #[test]
    fn the_version_child_is_asked_for_machine_readable_output() {
        let f = fixture();
        let argv = CodeQl.preflight(&f.grant).unwrap();
        assert_eq!(argv, vec!["version", "--format=json"]);
    }

    #[test]
    fn a_matching_version_passes_in_either_spelling() {
        let f = fixture();
        assert!(CodeQl
            .verify_preflight(&f.grant, r#"{"version":"2.26.1"}"#)
            .is_ok());
    }

    #[test]
    fn a_different_version_is_a_mismatch_that_names_both() {
        let f = fixture();
        let err = CodeQl
            .verify_preflight(&f.grant, r#"{"version":"2.20.0"}"#)
            .unwrap_err();
        let UnavailableReason::BundleMismatch { expected, found } = &err else {
            panic!("expected a bundle mismatch, got {err:?}");
        };
        assert_eq!(expected, "codeql-bundle-v2.26.1");
        assert_eq!(found.as_deref(), Some("2.20.0"));
    }

    #[test]
    fn a_cli_that_cannot_say_its_version_is_not_given_the_benefit_of_the_doubt() {
        let f = fixture();
        for stdout in ["", "not json", "{}", r#"{"version":null}"#] {
            assert!(
                matches!(
                    CodeQl.verify_preflight(&f.grant, stdout),
                    Err(UnavailableReason::BundleMismatch { found: None, .. })
                ),
                "output {stdout:?} should not pass the pin"
            );
        }
    }

    #[test]
    fn the_two_commands_are_exactly_what_the_contract_says() {
        let f = fixture();
        let steps = CodeQl
            .steps(&f.grant, &req(&f, "python"), &f.scratch, &f.out)
            .unwrap();
        assert_eq!(steps.len(), 2);

        let create = &steps[0];
        assert_eq!(create[0..2], ["database".to_string(), "create".to_string()]);
        assert!(create.contains(&"--language=python".to_string()));
        assert!(create.contains(&"--build-mode=none".to_string()));
        assert!(create
            .iter()
            .any(|a| a.starts_with("--source-root=") && a.ends_with("src")));

        let analyze = &steps[1];
        assert_eq!(
            analyze[0..2],
            ["database".to_string(), "analyze".to_string()]
        );
        assert!(analyze.contains(&"--format=sarif-latest".to_string()));
        assert!(analyze
            .iter()
            .any(|a| a.starts_with("--output=") && a.contains("report.sarif")));
        assert_eq!(analyze.last().unwrap(), "codeql/python-queries");

        // Both steps name the same database, and it is inside the scratch bee chose.
        assert_eq!(create[2], analyze[2]);
        assert!(create[2].starts_with(&f.scratch.to_string_lossy().into_owned()));
    }

    #[test]
    fn nothing_here_ever_asks_for_tracing() {
        let f = fixture();
        let steps = CodeQl
            .steps(&f.grant, &req(&f, "java"), &f.scratch, &f.out)
            .unwrap();
        for argv in &steps {
            for arg in argv {
                assert!(
                    !arg.contains("trace")
                        && !arg.contains("autobuild")
                        && !arg.contains("command"),
                    "argv must never ask CodeQL to observe a build: {arg}"
                );
            }
        }
    }

    #[test]
    fn an_unpinned_bundle_yields_no_command_and_no_preflight() {
        let mut f = fixture();
        f.grant.bundle_version = None;
        // Nothing to verify against...
        assert!(CodeQl.preflight(&f.grant).is_none());
        // ...and therefore nothing to run. The sentence naming `bundle_version` belongs to the
        // `Unavailable` that `probe` returns (asserted above); what `steps` owes is a refusal to
        // author a command line, whichever route reached it.
        let err = CodeQl
            .steps(&f.grant, &req(&f, "python"), &f.scratch, &f.out)
            .unwrap_err();
        assert!(err.contains("unverified analysis bundle"), "{err}");
    }

    #[test]
    fn a_configured_suite_replaces_the_default_pack_and_is_checked() {
        let mut f = fixture();
        let suite = f.scratch.join("custom.qls");
        std::fs::write(&suite, "- queries: .\n").unwrap();
        f.grant.rules = Some(suite.clone());
        let steps = CodeQl
            .steps(&f.grant, &req(&f, "python"), &f.scratch, &f.out)
            .unwrap();
        assert_eq!(steps[1].last().unwrap(), &suite.to_string_lossy());

        f.grant.rules = Some(PathBuf::from("/nonexistent/custom.qls"));
        assert!(CodeQl
            .steps(&f.grant, &req(&f, "python"), &f.scratch, &f.out)
            .is_err());
        f.grant.rules = Some(PathBuf::from("--rerun"));
        assert!(CodeQl
            .steps(&f.grant, &req(&f, "python"), &f.scratch, &f.out)
            .is_err());
    }

    #[test]
    fn steps_does_not_take_probes_word_for_the_language() {
        // The two are separately reachable, so the one that writes the command line checks too.
        let f = fixture();
        assert!(CodeQl
            .steps(&f.grant, &req(&f, "cpp"), &f.scratch, &f.out)
            .is_err());
    }
}
