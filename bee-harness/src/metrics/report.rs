//! Aggregation over the metrics log: totals, per-model / per-project / per-day rollups, latency and
//! time-to-first-token percentiles, and error rate. Pure functions over a slice of records so they
//! are trivially testable; the `bee-metrics` binary renders the result.

use std::collections::BTreeMap;

use super::CallRecord;

/// A per-group rollup (by model, project, or day).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GroupStat {
    pub key: String,
    pub calls: usize,
    pub cost: f64,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// avg / p50 / p95 over a set of samples (ms).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Percentiles {
    pub n: usize,
    pub avg: f64,
    pub p50: u64,
    pub p95: u64,
}

/// The full aggregation.
#[derive(Debug, Clone, Default)]
pub struct Report {
    pub calls: usize,
    pub errors: usize,
    pub total_cost: f64,
    pub priced_calls: usize,
    pub unpriced_calls: usize,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub reasoning_tokens: u64,
    pub by_model: Vec<GroupStat>,
    pub by_project: Vec<GroupStat>,
    pub by_day: Vec<GroupStat>,
    pub latency: Percentiles,
    pub ttft: Percentiles,
}

fn percentile(sorted: &[u64], q: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let idx = ((sorted.len() as f64 - 1.0) * q).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn summarize(mut samples: Vec<u64>) -> Percentiles {
    if samples.is_empty() {
        return Percentiles::default();
    }
    samples.sort_unstable();
    let n = samples.len();
    let avg = samples.iter().sum::<u64>() as f64 / n as f64;
    Percentiles {
        n,
        avg,
        p50: percentile(&samples, 0.5),
        p95: percentile(&samples, 0.95),
    }
}

/// Collapse a `key → GroupStat` map into a vector sorted by descending cost, then calls.
fn ranked(map: BTreeMap<String, GroupStat>) -> Vec<GroupStat> {
    let mut v: Vec<GroupStat> = map.into_values().collect();
    v.sort_by(|a, b| {
        b.cost
            .partial_cmp(&a.cost)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(b.calls.cmp(&a.calls))
            .then(a.key.cmp(&b.key))
    });
    v
}

/// Aggregate a slice of records into a [`Report`].
pub fn aggregate(records: &[CallRecord]) -> Report {
    let mut r = Report::default();
    let mut by_model: BTreeMap<String, GroupStat> = BTreeMap::new();
    let mut by_project: BTreeMap<String, GroupStat> = BTreeMap::new();
    let mut by_day: BTreeMap<String, GroupStat> = BTreeMap::new();
    let mut latencies = Vec::new();
    let mut ttfts = Vec::new();

    for rec in records {
        r.calls += 1;
        if rec.outcome.starts_with("error") {
            r.errors += 1;
        }
        match rec.cost_usd {
            Some(c) => {
                r.total_cost += c;
                r.priced_calls += 1;
            }
            None => r.unpriced_calls += 1,
        }
        r.input_tokens += rec.input_tokens as u64;
        r.output_tokens += rec.output_tokens as u64;
        r.cache_read_tokens += rec.cache_read_tokens as u64;
        r.cache_write_tokens += rec.cache_write_tokens as u64;
        r.reasoning_tokens += rec.reasoning_tokens as u64;
        latencies.push(rec.latency_ms);
        if let Some(t) = rec.ttft_ms {
            ttfts.push(t);
        }

        let day = rec.ts.get(0..10).unwrap_or(&rec.ts).to_string();
        for (map, key) in [
            (&mut by_model, rec.model_id.clone()),
            (&mut by_project, rec.project.clone()),
            (&mut by_day, day),
        ] {
            let g = map.entry(key.clone()).or_insert_with(|| GroupStat {
                key,
                ..Default::default()
            });
            g.calls += 1;
            g.cost += rec.cost_usd.unwrap_or(0.0);
            g.input_tokens += rec.input_tokens as u64;
            g.output_tokens += rec.output_tokens as u64;
        }
    }

    r.by_model = ranked(by_model);
    r.by_project = ranked(by_project);
    r.by_day = ranked(by_day);
    r.latency = summarize(latencies);
    r.ttft = summarize(ttfts);
    r
}

/// A compact token count: `840`, `12.3k`, `4.1M`.
fn human(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

/// Render a report as a plain-text summary.
pub fn render(r: &Report) -> String {
    if r.calls == 0 {
        return "no metrics recorded yet.".to_string();
    }
    let mut out = String::new();
    out.push_str(&format!(
        "{} calls · ${:.4} total cost · {} errors\n",
        r.calls, r.total_cost, r.errors
    ));
    if r.unpriced_calls > 0 {
        out.push_str(&format!(
            "  ({} priced, {} unpriced — local/mock endpoints excluded from cost)\n",
            r.priced_calls, r.unpriced_calls
        ));
    }
    out.push_str(&format!(
        "tokens: {} in · {} out · {} cache-read · {} cache-write · {} reasoning\n",
        human(r.input_tokens),
        human(r.output_tokens),
        human(r.cache_read_tokens),
        human(r.cache_write_tokens),
        human(r.reasoning_tokens),
    ));
    out.push_str(&format!(
        "latency: avg {:.0}ms · p50 {}ms · p95 {}ms",
        r.latency.avg, r.latency.p50, r.latency.p95
    ));
    if r.ttft.n > 0 {
        out.push_str(&format!(
            "   ttft (streaming): avg {:.0}ms · p50 {}ms · p95 {}ms",
            r.ttft.avg, r.ttft.p50, r.ttft.p95
        ));
    }
    out.push('\n');

    let section = |title: &str, groups: &[GroupStat]| -> String {
        let mut s = format!("\n{title}:\n");
        for g in groups.iter().take(10) {
            s.push_str(&format!(
                "  {:<40} {:>5} calls  ${:>9.4}  {:>7} in / {:>7} out\n",
                clip(&g.key, 40),
                g.calls,
                g.cost,
                human(g.input_tokens),
                human(g.output_tokens),
            ));
        }
        s
    };
    out.push_str(&section("by model", &r.by_model));
    out.push_str(&section("by project", &r.by_project));
    out.push_str(&section("by day", &r.by_day));
    out
}

/// Keep the last `max` chars of a key (paths read better tail-first).
fn clip(s: &str, max: usize) -> String {
    let n = s.chars().count();
    if n <= max {
        s.to_string()
    } else {
        let tail: String = s.chars().skip(n - (max - 1)).collect();
        format!("…{tail}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::CallRecord;

    fn rec(
        model: &str,
        cost: Option<f64>,
        latency: u64,
        ttft: Option<u64>,
        outcome: &str,
    ) -> CallRecord {
        CallRecord {
            ts: "2026-07-20T10:00:00Z".to_string(),
            session_id: "s".to_string(),
            kind: "repl".to_string(),
            project: "/proj".to_string(),
            provider: "anthropic".to_string(),
            model_id: model.to_string(),
            input_tokens: 100,
            output_tokens: 50,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            reasoning_tokens: 0,
            ttft_ms: ttft,
            latency_ms: latency,
            throughput_tok_s: None,
            stop_reason: "end_turn".to_string(),
            outcome: outcome.to_string(),
            cost_usd: cost,
            pricing_version: "test".to_string(),
        }
    }

    #[test]
    fn empty_report() {
        let r = aggregate(&[]);
        assert_eq!(r.calls, 0);
        assert!(render(&r).contains("no metrics"));
    }

    #[test]
    fn totals_and_grouping() {
        let records = vec![
            rec(
                "anthropic/claude-opus-4-8",
                Some(0.10),
                1000,
                Some(300),
                "ok",
            ),
            rec(
                "anthropic/claude-opus-4-8",
                Some(0.20),
                2000,
                Some(500),
                "ok",
            ),
            rec("mock/scripted", None, 50, None, "error:auth"),
        ];
        let r = aggregate(&records);
        assert_eq!(r.calls, 3);
        assert_eq!(r.errors, 1);
        assert_eq!(r.priced_calls, 2);
        assert_eq!(r.unpriced_calls, 1);
        assert!((r.total_cost - 0.30).abs() < 1e-9);
        // most-expensive model first
        assert_eq!(r.by_model[0].key, "anthropic/claude-opus-4-8");
        assert_eq!(r.by_model[0].calls, 2);
        // ttft only counts the two streaming calls
        assert_eq!(r.ttft.n, 2);
        assert_eq!(r.latency.n, 3);
    }

    #[test]
    fn percentiles_are_sane() {
        let records: Vec<CallRecord> = (1..=100)
            .map(|i| rec("claude-haiku-4-5", Some(0.001), i, None, "ok"))
            .collect();
        let r = aggregate(&records);
        assert_eq!(r.latency.p50, 51); // round(99*0.5)=50 → sorted[50]=51
        assert_eq!(r.latency.p95, 95); // round(99*0.95)=94 → sorted[94]=95
    }
}
