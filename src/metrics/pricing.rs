//! Per-model pricing and cost computation.
//!
//! Rates are provider **list prices per 1M tokens**, current as of [`PRICING_VERSION`]. Records
//! always store raw token counts as the source of truth; cost is derived here and can be recomputed
//! if the table changes, so a later price update never rewrites history. Models not in the table
//! (local `openai-compat`/Ollama endpoints, the `mock` provider, or anything unrecognized) are
//! **unpriced** — [`cost`] returns `None` rather than guessing.
//!
//! Cache and reasoning tokens follow Anthropic's billing: `input_tokens` is the *uncached* remainder,
//! so cache-read and cache-write tokens are additive (not double-counted); cache reads bill at 0.1×
//! input, cache writes at 1.25× input (the 5-minute-TTL rate), and reasoning tokens bill as output.

use crate::provider::Usage;

/// The date these rates were captured. Stamped on every record for reproducibility.
pub const PRICING_VERSION: &str = "2026-07-20";

/// Cache-read discount and cache-write premium, as multiples of the input rate.
const CACHE_READ_MULT: f64 = 0.1;
const CACHE_WRITE_MULT: f64 = 1.25;

/// Per-1M-token input/output rates for one model.
struct Rates {
    input: f64,
    output: f64,
}

/// Look up rates by model id (the provider prefix, if any, is ignored). `None` ⇒ unpriced.
fn rates(model_id: &str) -> Option<Rates> {
    let m = model_id.rsplit('/').next().unwrap_or(model_id);
    let r = |input, output| Some(Rates { input, output });
    match m {
        "claude-opus-4-8" | "claude-opus-4-7" | "claude-opus-4-6" | "claude-opus-4-5" => {
            r(5.0, 25.0)
        }
        // Sonnet 5 has an intro rate ($2/$10 through 2026-08-31); list price is used here so cost is
        // never understated — recompute against a dated table if exact intro pricing is needed.
        "claude-sonnet-5" | "claude-sonnet-4-6" | "claude-sonnet-4-5" => r(3.0, 15.0),
        "claude-haiku-4-5" => r(1.0, 5.0),
        "claude-fable-5" | "claude-mythos-5" => r(10.0, 50.0),
        _ => None,
    }
}

/// USD cost of one call's `usage` on `model_id`, or `None` when the model is unpriced.
pub fn cost(model_id: &str, usage: &Usage) -> Option<f64> {
    let Rates { input, output } = rates(model_id)?;
    let per = |tokens: u32, rate: f64| tokens as f64 * rate / 1_000_000.0;
    Some(
        per(usage.input_tokens, input)
            + per(usage.output_tokens, output)
            + per(usage.reasoning_tokens, output)
            + per(usage.cache_read_tokens, input * CACHE_READ_MULT)
            + per(usage.cache_write_tokens, input * CACHE_WRITE_MULT),
    )
}

/// Whether a model id is priced (in the table). Handy for reports that separate priced vs unpriced.
pub fn is_priced(model_id: &str) -> bool {
    rates(model_id).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(input: u32, output: u32) -> Usage {
        Usage {
            input_tokens: input,
            output_tokens: output,
            ..Default::default()
        }
    }

    #[test]
    fn opus_plain_cost() {
        // 1M input @ $5 + 1M output @ $25 = $30.
        let c = cost("anthropic/claude-opus-4-8", &usage(1_000_000, 1_000_000)).unwrap();
        assert!((c - 30.0).abs() < 1e-9, "{c}");
    }

    #[test]
    fn cache_and_reasoning_are_priced() {
        let u = Usage {
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 1_000_000,  // 0.1 × $5 = $0.50
            cache_write_tokens: 1_000_000, // 1.25 × $5 = $6.25
            reasoning_tokens: 1_000_000,   // billed as output: $25
        };
        let c = cost("claude-opus-4-8", &u).unwrap();
        assert!((c - (0.5 + 6.25 + 25.0)).abs() < 1e-9, "{c}");
    }

    #[test]
    fn provider_prefix_is_ignored() {
        assert_eq!(
            cost("anthropic/claude-haiku-4-5", &usage(1_000_000, 0)),
            cost("claude-haiku-4-5", &usage(1_000_000, 0))
        );
    }

    #[test]
    fn local_and_mock_are_unpriced() {
        assert_eq!(
            cost("openai-compat/qwen2.5-coder", &usage(1000, 1000)),
            None
        );
        assert_eq!(cost("mock/scripted", &usage(1000, 1000)), None);
        assert!(!is_priced("mock/scripted"));
        assert!(is_priced("claude-sonnet-5"));
    }
}
