//! The Opengrep adapter (016-native-tools US3).
//!
//! Opengrep is the first external scanner because of what it brings: a large, maintained corpus of
//! pattern rules across many languages, which is exactly the thing bee should borrow rather than
//! rebuild (research R2). What bee contributes is the part Opengrep does not — running it inside an
//! enforced scope, deciding its argv from typed inputs, and refusing to report a failed run as a
//! clean one.
//!
//! Measured against **Opengrep 1.22.0** (research R4/R6).

use std::path::Path;

use super::{probe_binary, validate_target, ScanRequest, ScannerAdapter, ScannerGrant};
use crate::tools::outcome::UnavailableReason;

pub struct Opengrep;

impl ScannerAdapter for Opengrep {
    fn name(&self) -> &'static str {
        "opengrep"
    }

    fn probe(&self, grant: &ScannerGrant, _req: &ScanRequest) -> Result<(), UnavailableReason> {
        // Nothing about the request can make Opengrep unavailable: it needs no provisioned bundle,
        // and its own rule corpus decides which languages it reads.
        probe_binary(grant)
    }

    /// One step, because Opengrep needs one:
    ///
    /// ```text
    /// scan --sarif --sarif-output=<out> --quiet --config <rules> --timeout <secs> <target>
    /// ```
    ///
    /// Every argument here is either a constant or a validated, typed input. There is no path by
    /// which a model-supplied string becomes a flag (FR-007). The scratch directory goes unused —
    /// Opengrep reads the tree and writes the report, with nothing in between to keep.
    fn steps(
        &self,
        grant: &ScannerGrant,
        req: &ScanRequest,
        _scratch: &Path,
        out: &Path,
    ) -> Result<Vec<Vec<String>>, String> {
        validate_target(&req.target)?;

        let rules = grant.rules.as_ref().ok_or_else(|| {
            // Not defaulting to `auto` is the whole point — see the refusal below. With no ruleset
            // there is nothing to scan *for*, and a scan with no rules would find nothing and look
            // exactly like a clean scan.
            "no ruleset configured for opengrep: set `[security.scanners.opengrep] rules` to a \
             local rule file or directory"
                .to_string()
        })?;

        let rules_str = rules.to_string_lossy();
        if rules_str == "auto" {
            return Err(
                "`--config auto` is refused: it fetches the rule registry over the network, and a \
                 scanning scope has no egress — the run would fail opaquely rather than cleanly. \
                 Configure a local ruleset path instead (research R6)."
                    .to_string(),
            );
        }
        if rules_str.starts_with('-') {
            return Err(format!(
                "ruleset path `{rules_str}` would be read as a flag"
            ));
        }
        if !rules.exists() {
            return Err(format!("ruleset `{rules_str}` does not exist"));
        }

        // `--quiet` because Opengrep also prints the report to stdout when `--sarif` is set, and the
        // copy bee reads is the file. `--timeout` is Opengrep's own per-rule budget; the wall-clock
        // budget the tool enforces around the child is the one that actually bounds the call.
        Ok(vec![vec![
            "scan".to_string(),
            "--sarif".to_string(),
            format!("--sarif-output={}", out.display()),
            "--quiet".to_string(),
            "--config".to_string(),
            rules_str.into_owned(),
            "--timeout".to_string(),
            req.timeout.as_secs().to_string(),
            req.target.to_string_lossy().into_owned(),
        ]])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct Fixture {
        _tmp: tempfile::TempDir,
        grant: ScannerGrant,
        target: PathBuf,
        scratch: PathBuf,
        out: PathBuf,
    }

    fn fixture() -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let rules = tmp.path().join("rules.yml");
        std::fs::write(&rules, "rules: []\n").unwrap();
        let target = tmp.path().join("src");
        std::fs::create_dir_all(&target).unwrap();
        let scratch = tmp.path().join("scratch");
        std::fs::create_dir_all(&scratch).unwrap();
        let grant = ScannerGrant::for_test("opengrep", tmp.path().join("opengrep"), Some(rules));
        let out = tmp.path().join("report.sarif");
        Fixture {
            _tmp: tmp,
            grant,
            target,
            scratch,
            out,
        }
    }

    /// Opengrep is a one-step adapter; every case below asserts over that single command.
    fn only_step(f: &Fixture, req: &ScanRequest) -> Result<Vec<String>, String> {
        let mut steps = Opengrep.steps(&f.grant, req, &f.scratch, &f.out)?;
        assert_eq!(steps.len(), 1, "opengrep runs exactly one child");
        Ok(steps.remove(0))
    }

    #[test]
    fn the_command_is_exactly_what_the_contract_says() {
        let f = fixture();
        let argv = only_step(&f, &ScanRequest::new(f.target.clone())).unwrap();
        assert_eq!(argv[0], "scan");
        assert!(argv.contains(&"--sarif".to_string()));
        assert!(argv.contains(&"--quiet".to_string()));
        assert!(argv
            .iter()
            .any(|a| a.starts_with("--sarif-output=") && a.contains("report.sarif")));
        assert_eq!(argv.last().unwrap(), &f.target.to_string_lossy());
        // Nothing that would make Opengrep run another program, and no way to ask for one.
        assert!(!argv.iter().any(|a| a.contains("--pro") || a.contains("-e")));
    }

    #[test]
    fn auto_is_refused_at_construction_with_the_reason() {
        let mut f = fixture();
        f.grant.rules = Some(PathBuf::from("auto"));
        let err = only_step(&f, &ScanRequest::new(f.target.clone())).unwrap_err();
        assert!(err.contains("auto"), "{err}");
        assert!(err.contains("network"), "{err}");
    }

    #[test]
    fn no_ruleset_means_no_command() {
        let mut f = fixture();
        f.grant.rules = None;
        assert!(only_step(&f, &ScanRequest::new(f.target.clone())).is_err());
    }

    #[test]
    fn a_missing_ruleset_is_refused_before_spawning() {
        let mut f = fixture();
        f.grant.rules = Some(PathBuf::from("/nonexistent/rules.yml"));
        assert!(only_step(&f, &ScanRequest::new(f.target.clone())).is_err());
    }

    #[test]
    fn the_output_path_is_bee_chosen_and_the_timeout_is_the_grants() {
        let f = fixture();
        let mut req = ScanRequest::new(f.target.clone());
        req.timeout = std::time::Duration::from_secs(42);
        let argv = only_step(&f, &req).unwrap();
        let i = argv.iter().position(|a| a == "--timeout").unwrap();
        assert_eq!(argv[i + 1], "42");
        // The report path never comes from the caller — it is the one bee handed in.
        assert_eq!(
            argv.iter()
                .filter(|a| a.starts_with("--sarif-output="))
                .count(),
            1
        );
    }
}
