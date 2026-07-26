//! The `cvss` tool (016-native-tools US4): severity **computed**, never asserted.
//!
//! The division of labour is the point (FR-005). The model supplies the *vector* — characterising
//! attack vector, complexity, privileges required, user interaction, scope, and the CIA impacts —
//! which is judgement it is good at. The *score* is a multi-step floating-point calculation with
//! scope-conditional coefficients, which it is not good at, and which a library gets right every
//! time. So the model never gets to hand over a number: this tool takes a vector string and returns
//! the arithmetic.
//!
//! Unlike every other file-touching tool in 016, this one has **no worker and no sandboxed child**.
//! It opens no files and executes nothing — it is pure computation over a string — so the
//! sandboxed-child seam has nothing to protect and would only add a process spawn.

use serde::Deserialize;
use serde_json::json;

use crate::provider::ToolSchema;
use crate::sandbox::Sandbox;
use crate::tools::outcome::ToolOutcome;
use crate::tools::{Tool, ToolResult};

#[derive(Deserialize)]
struct Args {
    vector: String,
}

/// A scored severity: the vector as given, plus the computed number and its band.
#[derive(Debug, Clone, PartialEq)]
pub struct Scored {
    pub vector: String,
    pub score: f64,
    pub band: String,
}

impl std::fmt::Display for Scored {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:.1} ({}) — {}", self.score, self.band, self.vector)
    }
}

/// Parse and score a CVSS vector. Accepts v3.1 (`CVSS:3.1/…`) and v4.0 (`CVSS:4.0/…`).
///
/// Returns `Err` with the parse diagnostic for anything malformed — deliberately **not** a zero
/// score, which would be a guess wearing the costume of a measurement.
pub fn score(vector: &str) -> Result<Scored, String> {
    let trimmed = vector.trim();
    if trimmed.is_empty() {
        return Err("empty vector".to_string());
    }

    if trimmed.starts_with("CVSS:4") {
        let v: cvss::v4::Vector = trimmed.parse().map_err(|e| format!("{e}"))?;
        // `Score::value` and `Score::severity` both take `self` and `Score` is not `Copy`, so each
        // reading needs its own score. `Vector::score` borrows, so computing twice is cheap.
        return Ok(Scored {
            vector: trimmed.to_string(),
            score: v.score().value(),
            band: v.score().severity().to_string(),
        });
    }

    // v3.1 (and v3.0, which parses the same shape).
    let base: cvss::v3::Base = trimmed.parse().map_err(|e| format!("{e}"))?;
    Ok(Scored {
        vector: trimmed.to_string(),
        score: base.score().value(),
        band: base.severity().to_string(),
    })
}

/// `{ "vector": string }` → the computed score and band.
pub struct CvssTool;

#[async_trait::async_trait]
impl Tool for CvssTool {
    fn name(&self) -> &'static str {
        "cvss"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "cvss".to_string(),
            description: "Compute a CVSS score from a severity vector. Supply the vector — the \
                          metrics describing how the issue is reached and what it costs — and this \
                          returns the numeric score and its qualitative band. Supports CVSS v3.1 \
                          and v4.0. Do NOT compute the score yourself: state the vector and let \
                          this calculate it."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "vector": {
                        "type": "string",
                        "description": "A CVSS vector string, e.g. \"CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H\"."
                    }
                },
                "required": ["vector"]
            }),
        }
    }

    async fn call(&self, arguments: serde_json::Value, _sandbox: &Sandbox) -> ToolResult {
        let args: Args = match serde_json::from_value(arguments) {
            Ok(a) => a,
            Err(e) => return ToolResult::invalid_args("cvss", e),
        };

        match score(&args.vector) {
            Ok(scored) => ToolOutcome::completed(scored).into_tool_result(|s| s.to_string()),
            Err(detail) => {
                let reason = format!("could not parse `{}`: {detail}", args.vector);
                crate::tools::outcome::audit_failure("cvss", &reason);
                ToolOutcome::<Scored>::failed(reason).into_tool_result(|s| s.to_string())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reference vectors with independently known scores (SC-008).
    #[test]
    fn reference_vectors_compute_to_their_published_scores() {
        let cases = [
            (
                "CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H",
                9.8,
                "critical",
            ),
            ("CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:N/I:N/A:H", 7.5, "high"),
            // Hand-checked: exploitability 8.22 × 0.55(AV:L) × 0.44(AC:H) × 0.27(PR:H) × 0.62(UI:R)
            // = 0.333; impact 6.42 × 0.22 = 1.412; roundup(1.745) = 1.8.
            ("CVSS:3.1/AV:L/AC:H/PR:H/UI:R/S:U/C:L/I:N/A:N", 1.8, "low"),
            (
                "CVSS:3.1/AV:N/AC:L/PR:L/UI:N/S:U/C:L/I:L/A:N",
                5.4,
                "medium",
            ),
        ];
        for (vector, expected, band) in cases {
            let got = score(vector).unwrap_or_else(|e| panic!("{vector}: {e}"));
            assert!(
                (got.score - expected).abs() < 0.05,
                "{vector}: expected {expected}, got {}",
                got.score
            );
            assert_eq!(got.band.to_lowercase(), band, "{vector}");
        }
    }

    #[test]
    fn a_malformed_vector_is_rejected_not_guessed() {
        for bad in [
            "",
            "   ",
            "CVSS:3.1/AV:X/nonsense",
            "not a vector at all",
            "CVSS:3.1/",
        ] {
            let got = score(bad);
            assert!(got.is_err(), "{bad:?} should not parse, got {got:?}");
        }
    }

    #[test]
    fn a_rejected_vector_never_yields_a_zero_score() {
        // The failure mode this guards: returning 0.0 for unparseable input, which reads as a real
        // measurement of "no severity" rather than as a refusal.
        assert!(score("CVSS:3.1/AV:X/nonsense").is_err());
    }
}
