//! The `ast_grep` tool's worker: a structural, syntax-aware search that runs inside the scope.
//!
//! The security seam is the one every file tool relies on and [`crate::search`] documents: a tool
//! never reads files in the harness — it does the work in a *sandboxed child* so the eBPF LSM
//! mediates each open. Like `search`, this worker's "binary" is bee itself: [`crate::tools::astgrep`]
//! execs `bee astgrep-worker …` through `run_child`, so every file parsed here passes through the
//! scope's `file_open` policy exactly as a `cat` child would.
//!
//! **Why this exists beside `search`.** `search` matches bytes; this matches the parse tree. The
//! fixture at `tests/fixtures/astgrep/decoy.rs` holds the text `v.unwrap()` three times — as a call,
//! in a comment, and in a string literal. A regex returns all three and the reader has to sort them
//! out. Matching structurally returns one (SC-007).
//!
//! **Read-only by construction.** `ast-grep-core` can also rewrite (`Root::replace`). Rewriting is a
//! mutation with its own policy consequences, so this worker exposes no path to it — the argument
//! surface below has no replacement field, and nothing here calls `replace`.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use clap::Args;

/// An `ast_grep` request, shared between the model-facing tool (which fills it from the tool call
/// and serializes it onto the worker's argv) and the worker subcommand (which parses it back).
#[derive(Args, Debug, Clone)]
pub struct AstGrepArgs {
    /// The language whose grammar parses the target — `rust`, `python`, … Must be compiled into
    /// this build; see [`compiled_in`]. An unsupported value is refused, never silently skipped.
    #[arg(long)]
    pub lang: String,
    /// The directory (or file) to search under. Resolved and opened inside the scope, so the LSM,
    /// not this process, decides what is readable.
    #[arg(long, default_value = ".")]
    pub path: PathBuf,
    /// Stop after this many matches. Bounds both output size and walk time on a large tree — the
    /// same guard, and the same default, as `search`'s `MATCH_LIMIT`.
    #[arg(long, default_value_t = 200)]
    pub limit: usize,
    /// The ast-grep pattern, e.g. `$X.unwrap()`. `allow_hyphen_values` so a pattern that opens with
    /// an operator is not parsed as a flag.
    #[arg(allow_hyphen_values = true)]
    pub pattern: String,
}

/// Turn an [`AstGrepArgs`] into the argv tail that reconstructs it under `bee astgrep-worker`. The
/// caller prepends the program (the bee executable) and the subcommand name.
pub fn worker_argv(args: &AstGrepArgs) -> Vec<String> {
    vec![
        "astgrep-worker".to_string(),
        "--lang".to_string(),
        args.lang.clone(),
        "--path".to_string(),
        args.path.to_string_lossy().into_owned(),
        "--limit".to_string(),
        args.limit.to_string(),
        // `--` so a pattern beginning with `-` is never read as a flag.
        "--".to_string(),
        args.pattern.clone(),
    ]
}

/// One language bee can parse in this build: the name the model uses, the file extensions it claims,
/// and the Cargo feature that brings its grammar in.
struct LangEntry {
    name: &'static str,
    exts: &'static [&'static str],
}

/// The languages compiled into **this** binary.
///
/// This table is load-bearing rather than cosmetic. `ast_grep_language::SupportLang` keeps every
/// variant regardless of which grammar features are enabled, and its parser lookup is
/// `unimplemented!()` when the feature is off — so constructing one for an uncompiled language
/// **panics**. bee therefore answers "is this language available?" from its own registry and refuses
/// before ever touching `SupportLang` (research R12). A panic here would be a refusal that crashed
/// the worker instead of reporting, which Constitution I and FR-016 both forbid.
const LANGS: &[LangEntry] = &[
    #[cfg(feature = "astgrep-rust")]
    LangEntry {
        name: "rust",
        exts: &["rs"],
    },
    #[cfg(feature = "astgrep-python")]
    LangEntry {
        name: "python",
        exts: &["py", "pyi"],
    },
    #[cfg(feature = "astgrep-javascript")]
    LangEntry {
        name: "javascript",
        exts: &["js", "mjs", "cjs", "jsx"],
    },
    #[cfg(feature = "astgrep-typescript")]
    LangEntry {
        name: "typescript",
        exts: &["ts"],
    },
    #[cfg(feature = "astgrep-go")]
    LangEntry {
        name: "go",
        exts: &["go"],
    },
    #[cfg(feature = "astgrep-c")]
    LangEntry {
        name: "c",
        exts: &["c", "h"],
    },
    #[cfg(feature = "astgrep-cpp")]
    LangEntry {
        name: "cpp",
        exts: &["cc", "cpp", "cxx", "hpp", "hh"],
    },
    #[cfg(feature = "astgrep-java")]
    LangEntry {
        name: "java",
        exts: &["java"],
    },
    #[cfg(feature = "astgrep-ruby")]
    LangEntry {
        name: "ruby",
        exts: &["rb"],
    },
];

/// The language names available in this build, for the `LanguageUnsupported` refusal. Naming what
/// *is* here turns a dead end into a retry the caller can actually make.
pub fn compiled_in() -> Vec<String> {
    LANGS.iter().map(|l| l.name.to_string()).collect()
}

/// Whether `name` is parseable in this build. Callers MUST consult this before parsing.
pub fn is_compiled_in(name: &str) -> bool {
    LANGS.iter().any(|l| l.name == name)
}

fn entry(name: &str) -> Option<&'static LangEntry> {
    LANGS.iter().find(|l| l.name == name)
}

/// One structural match: where it is and what it covers.
#[derive(Debug, Clone, PartialEq)]
pub struct Match {
    pub path: String,
    /// 1-indexed, matching every other tool bee has (ast-grep counts from 0).
    pub line: usize,
    pub end_line: usize,
    /// The matched source text, first line only — enough to recognise, bounded for output.
    pub text: String,
}

impl std::fmt::Display for Match {
    /// `path:line:text`, the same shape `search` prints, so the two tools read alike.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}:{}", self.path, self.line, self.text)
    }
}

/// Why a search could not be performed. Distinct from "performed, found nothing" — the worker maps
/// these to a non-zero exit with a diagnostic on stderr, and the tool wrapper maps them to
/// `Unavailable`/`Failed` (contract `tool-outcome.md`).
#[derive(Debug)]
pub enum AstGrepError {
    /// The grammar for this language is not in this build.
    LanguageUnsupported {
        lang: String,
        compiled_in: Vec<String>,
    },
    /// The pattern does not parse against that grammar.
    BadPattern(String),
    /// The tree could not be walked at all.
    Io(io::Error),
}

impl std::fmt::Display for AstGrepError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AstGrepError::LanguageUnsupported { lang, compiled_in } => {
                if compiled_in.is_empty() {
                    write!(
                        f,
                        "language `{lang}` is unsupported; no grammars are compiled into this build"
                    )
                } else {
                    write!(
                        f,
                        "language `{lang}` is unsupported; compiled in: {}",
                        compiled_in.join(", ")
                    )
                }
            }
            AstGrepError::BadPattern(detail) => write!(f, "invalid pattern: {detail}"),
            AstGrepError::Io(e) => write!(f, "{e}"),
        }
    }
}

/// Run the structural search. Returns the bounded match list, or an error that is explicitly *not*
/// an empty result.
pub fn search(args: &AstGrepArgs) -> Result<Vec<Match>, AstGrepError> {
    let Some(entry) = entry(&args.lang) else {
        // Refuse from our own table — see the note on LANGS. Reaching SupportLang here would panic.
        return Err(AstGrepError::LanguageUnsupported {
            lang: args.lang.clone(),
            compiled_in: compiled_in(),
        });
    };

    // Safe now: the feature that gates this name also gates the grammar the parser looks up.
    let lang: ast_grep_language::SupportLang =
        entry
            .name
            .parse()
            .map_err(|_| AstGrepError::LanguageUnsupported {
                lang: args.lang.clone(),
                compiled_in: compiled_in(),
            })?;

    let mut out = Vec::new();
    let mut pattern_checked = false;

    for file in walk(&args.path, entry.exts)? {
        if out.len() >= args.limit {
            break;
        }
        // A file that cannot be read inside the scope is skipped, not fatal: the LSM denying one
        // path is a normal, audited outcome of scanning a tree, not a failure of the search.
        let Ok(src) = std::fs::read_to_string(&file) else {
            continue;
        };

        let matches = match_file(&lang, &src, &args.pattern)?;
        pattern_checked = true;

        for m in matches {
            if out.len() >= args.limit {
                break;
            }
            out.push(Match {
                path: display_path(&file),
                ..m
            });
        }
    }

    // If the walk found nothing to parse, the pattern was never validated. Validate it now so a
    // typo'd pattern over an empty tree still reports as a bad pattern rather than a clean scan.
    if !pattern_checked {
        match_file(&lang, "", &args.pattern)?;
    }

    Ok(out)
}

/// Parse one source string and collect matches. Split out so the pattern-validity error surfaces
/// identically whether or not the tree had files.
fn match_file(
    lang: &ast_grep_language::SupportLang,
    src: &str,
    pattern: &str,
) -> Result<Vec<Match>, AstGrepError> {
    use ast_grep_language::LanguageExt;

    let root = lang.ast_grep(src);
    let pat = ast_grep_core::matcher::Pattern::try_new(pattern, *lang)
        .map_err(|e| AstGrepError::BadPattern(e.to_string()))?;
    check_pattern_parses_cleanly(&pat, pattern)?;

    let mut out = Vec::new();
    for m in root.root().find_all(&pat) {
        let start = m.start_pos();
        let end = m.end_pos();
        let text = m.text();
        out.push(Match {
            path: String::new(), // filled by the caller, which knows the path
            // ast-grep counts lines from 0; every other bee tool counts from 1.
            line: start.line() + 1,
            end_line: end.line() + 1,
            text: text.lines().next().unwrap_or("").trim().to_string(),
        });
    }
    Ok(out)
}

/// Reject a pattern whose own parse tree contains error or missing nodes.
///
/// `Pattern::try_new` is more permissive than it looks: tree-sitter recovers from broken input, so
/// `fn $(((` builds a pattern happily and then matches nothing. Only genuinely degenerate patterns —
/// empty, whitespace, a standalone `$$$` — are rejected outright.
///
/// That permissiveness is a fail-closed problem, not a convenience. A typo'd pattern would return
/// zero matches and exit 0, which reads exactly like "parsed the tree, found nothing" — the
/// confusion FR-012 exists to prevent, arriving through the one input the model authors freely. So
/// bee checks the pattern's tree itself and refuses when the grammar could not make sense of it.
fn check_pattern_parses_cleanly(
    pat: &ast_grep_core::matcher::Pattern,
    pattern: &str,
) -> Result<(), AstGrepError> {
    if pat.has_error() {
        return Err(AstGrepError::BadPattern(format!(
            "the grammar could not parse `{pattern}`. Patterns must be a syntactically valid \
             fragment of the target language, with `$VAR` standing in for a subexpression and \
             `$$$` for a run of them — for example `$X.unwrap()` or `fn $NAME($$$) {{ $$$ }}`."
        )));
    }
    Ok(())
}

/// Files under `root` whose extension the language claims. Deliberately narrow: feeding a `.txt` to
/// the Rust grammar produces noise, not findings.
fn walk(root: &Path, exts: &[&str]) -> Result<Vec<PathBuf>, AstGrepError> {
    let mut found = Vec::new();
    let claims = |p: &Path| {
        p.extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| exts.contains(&e))
    };

    if root.is_file() {
        if claims(root) {
            found.push(root.to_path_buf());
        }
        return Ok(found);
    }

    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        // A directory the scope denies is skipped, not fatal — same reasoning as the read above.
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_dir() {
                // Don't descend into VCS metadata or build output; both are noise and `target/` in
                // particular can dwarf the source tree.
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if name == ".git" || name == "target" || name == "node_modules" {
                    continue;
                }
                stack.push(path);
            } else if ft.is_file() && claims(&path) {
                found.push(path);
            }
        }
    }
    // Stable output: the walk order of read_dir is not defined, and a tool whose results reshuffle
    // between identical runs is one an operator cannot diff.
    found.sort();
    Ok(found)
}

fn display_path(p: &Path) -> String {
    let cwd = std::env::current_dir().unwrap_or_default();
    p.strip_prefix(&cwd)
        .unwrap_or(p)
        .to_string_lossy()
        .into_owned()
}

/// The `bee astgrep-worker` entry point. Prints one `path:line:text` per match, exits non-zero with
/// a diagnostic on stderr when the search could not be performed at all.
pub fn run_worker(args: &AstGrepArgs) -> Result<(), AstGrepError> {
    let matches = search(args)?;
    let stdout = io::stdout();
    let mut w = stdout.lock();
    for m in &matches {
        writeln!(w, "{m}").map_err(AstGrepError::Io)?;
    }
    w.flush().map_err(AstGrepError::Io)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_argv_round_trips_the_request() {
        let args = AstGrepArgs {
            lang: "rust".into(),
            path: PathBuf::from("/tmp/x"),
            limit: 50,
            pattern: "$X.unwrap()".into(),
        };
        let argv = worker_argv(&args);
        assert_eq!(argv[0], "astgrep-worker");
        assert!(argv.contains(&"--lang".to_string()));
        assert!(argv.contains(&"rust".to_string()));
        assert!(argv.contains(&"50".to_string()));
        // The pattern must sit after `--` so a leading `-` cannot be read as a flag.
        let dashdash = argv.iter().position(|a| a == "--").unwrap();
        assert_eq!(argv[dashdash + 1], "$X.unwrap()");
    }

    #[test]
    fn a_pattern_opening_with_a_hyphen_survives_argv() {
        let args = AstGrepArgs {
            lang: "rust".into(),
            path: PathBuf::from("."),
            limit: 200,
            pattern: "-$X".into(),
        };
        let argv = worker_argv(&args);
        assert_eq!(argv.last().unwrap(), "-$X");
    }

    #[test]
    fn an_uncompiled_language_is_refused_not_parsed() {
        let args = AstGrepArgs {
            lang: "cobol".into(),
            path: PathBuf::from("."),
            limit: 10,
            pattern: "$X".into(),
        };
        // Must be an error, and must NOT panic — the whole reason LANGS exists.
        match search(&args) {
            Err(AstGrepError::LanguageUnsupported { lang, .. }) => assert_eq!(lang, "cobol"),
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[cfg(feature = "astgrep-rust")]
    #[test]
    fn rust_is_compiled_in_when_its_feature_is() {
        assert!(is_compiled_in("rust"));
        assert!(compiled_in().contains(&"rust".to_string()));
    }
}
