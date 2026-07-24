//! The `search` tool's worker: a ripgrep-library search that runs inside the scope.
//!
//! The security seam is the same one every file tool relies on (contracts/tool-contracts.md): a
//! tool never reads files in the harness — it does the work in a *sandboxed child* so the eBPF LSM
//! mediates each open. `search` is the one tool whose "binary" is bee itself: [`crate::tools::search`]
//! execs `bee search-worker …` through [`crate::tools::exec::run_child`], so this code runs in a
//! process that has joined the scope cgroup, been hardened, and had its environment stripped. Every
//! path [`ripgrep_api`] opens here therefore passes through the scope's `file_open` policy exactly
//! like a `cat` or `rg` child would. Running the library in the harness instead would search *around*
//! the sandbox — reading whatever the (root) harness can reach — which is why it lives here.
//!
//! The worker's argument surface is deliberately narrow: a pattern, a root path, an optional glob,
//! and a case toggle. None of ripgrep's command-executing options (`--pre`, `--search-zip`) are
//! exposed — the model drives this only through [`SearchArgs`], and there is nothing here that runs
//! another program.

use std::io::{self, Write};
use std::path::PathBuf;

use clap::Args;

/// A `search` request, shared between the model-facing tool (which fills it from the tool call and
/// serializes it onto the worker's argv) and the worker subcommand (which parses it back).
#[derive(Args, Debug, Clone)]
pub struct SearchArgs {
    /// The directory (or file) to search under. Resolved and opened inside the scope, so the LSM,
    /// not this process, decides what is readable.
    #[arg(long, default_value = ".")]
    pub path: PathBuf,
    /// Restrict to files matching this glob (e.g. `**/*.rs`). Empty ⇒ no glob filter.
    #[arg(long, default_value = "")]
    pub glob: String,
    /// Case-insensitive match. Without it the search is smart-case (case-sensitive unless the
    /// pattern has an uppercase letter) — ripgrep's friendly default.
    #[arg(long)]
    pub ignore_case: bool,
    /// Stop after this many matching lines. Bounds both output size and walk time on a huge tree.
    #[arg(long, default_value_t = 200)]
    pub limit: usize,
    /// The regex to search for. `allow_hyphen_values` so a pattern like `-> Result` is not parsed
    /// as a flag.
    #[arg(allow_hyphen_values = true)]
    pub pattern: String,
}

/// Turn a [`SearchArgs`] into the argv tail that reconstructs it under `bee search-worker`. The
/// caller prepends the program (the bee executable) and the `search-worker` subcommand name.
pub fn worker_argv(args: &SearchArgs) -> Vec<String> {
    let mut v = vec![
        "search-worker".to_string(),
        "--path".to_string(),
        args.path.to_string_lossy().into_owned(),
        "--limit".to_string(),
        args.limit.to_string(),
    ];
    if !args.glob.is_empty() {
        v.push("--glob".to_string());
        v.push(args.glob.clone());
    }
    if args.ignore_case {
        v.push("--ignore-case".to_string());
    }
    // `--` guards a pattern that begins with `-`, alongside `allow_hyphen_values`.
    v.push("--".to_string());
    v.push(args.pattern.clone());
    v
}

/// Run the search and stream `path:line:text` to `out`, one match per line. Returns the number of
/// matches written. A regex/IO error from ripgrep is surfaced as an `Err` so the worker can exit
/// non-zero (which the tool reports as an error result); a *denied* file open is not an error here —
/// the LSM simply makes it unreadable, so it contributes no matches, exactly like `rg` skipping it.
pub fn run(args: &SearchArgs, out: &mut impl Write) -> io::Result<usize> {
    use bstr::ByteSlice;
    use ripgrep_api::SearchBuilder;

    let mut builder = SearchBuilder::new(&args.pattern)
        .path(&args.path)
        .limit(args.limit);
    if args.ignore_case {
        builder = builder.ignore_case();
    } else {
        builder = builder.smart_case();
    }
    if !args.glob.is_empty() {
        builder = builder.glob(&args.glob);
    }

    let mut count = 0usize;
    let mut io_err: Option<io::Error> = None;
    let result = builder.for_each(|m| {
        let line = m.line.unwrap_or(0);
        // `text` is arbitrary bytes; render lossily and drop the trailing newline ripgrep keeps so
        // our own `writeln!` owns the line ending.
        let text = m.text.to_str_lossy();
        let text = text.trim_end_matches(['\n', '\r']);
        if let Err(e) = writeln!(out, "{}:{}:{}", m.path.display(), line, text) {
            io_err = Some(e);
            return false; // stop the walk: the sink is gone (e.g. broken pipe)
        }
        count += 1;
        true
    });

    if let Some(e) = io_err {
        return Err(e);
    }
    if let Err(e) = result {
        return Err(io::Error::other(format!("search failed: {e}")));
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_argv_roundtrips_the_flags() {
        let args = SearchArgs {
            path: PathBuf::from("src"),
            glob: "**/*.rs".to_string(),
            ignore_case: true,
            limit: 50,
            pattern: "-> Result".to_string(),
        };
        let argv = worker_argv(&args);
        assert_eq!(argv[0], "search-worker");
        // The pattern is last and guarded by `--`, so a leading `-` is data, not a flag.
        assert_eq!(argv[argv.len() - 2], "--");
        assert_eq!(argv[argv.len() - 1], "-> Result");
        assert!(argv.iter().any(|a| a == "--ignore-case"));
        assert!(argv
            .windows(2)
            .any(|w| w[0] == "--glob" && w[1] == "**/*.rs"));
    }

    #[test]
    fn no_glob_or_case_flags_when_defaulted() {
        let args = SearchArgs {
            path: PathBuf::from("."),
            glob: String::new(),
            ignore_case: false,
            limit: 200,
            pattern: "needle".to_string(),
        };
        let argv = worker_argv(&args);
        assert!(!argv.iter().any(|a| a == "--glob"));
        assert!(!argv.iter().any(|a| a == "--ignore-case"));
    }

    #[test]
    fn finds_a_known_pattern_in_this_file() {
        // Search this crate's src for a string that certainly exists here.
        let args = SearchArgs {
            path: PathBuf::from("src"),
            glob: "**/*.rs".to_string(),
            ignore_case: false,
            limit: 10,
            pattern: "ripgrep-library search".to_string(),
        };
        let mut buf = Vec::new();
        let n = run(&args, &mut buf).expect("search runs");
        let out = String::from_utf8_lossy(&buf);
        assert!(n >= 1, "expected at least one match, got {n}: {out}");
        assert!(
            out.contains("search.rs"),
            "match should cite this file: {out}"
        );
    }
}
