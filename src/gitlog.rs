//! The `git_log` tool's worker: repository history, read inside the scope (016-native-tools US5).
//!
//! Same seam as [`crate::search`] and [`crate::astgrep`]: the library does its reading in a
//! *sandboxed child* — `bee gitlog-worker …`, exec'd through `run_child` — so every object file and
//! pack the walk opens passes the scope's `file_open` policy. A history query looks harmless, but
//! `.git` is a directory of files like any other, and reading it in the harness would be reading
//! around the sandbox.
//!
//! **Why `gix` rather than shelling out to `git`.** The two-tier rule (spec Overview): a maintained
//! Rust library is the engine here, so the harness owns it and the exec allowlist stays empty. It
//! also means the answer does not depend on which `git` happens to be installed, or on parsing the
//! output of a program that formats for humans.
//!
//! **Read-only by construction.** Nothing here writes an object, a ref, or a config entry. The two
//! modes answer "what happened" and "who wrote this line", which is the whole surface US5 asks for.

use std::path::{Path, PathBuf};

use clap::Args;

/// Exit code for "this path is not inside a repository".
///
/// A distinct code rather than a message the wrapper greps: [`crate::tools::gitlog`] maps it to
/// `Unavailable { NotARepository }`, and the difference between *that* and an empty history is the
/// entire point of the case (FR-012). Matching on prose would make the distinction depend on
/// wording that any later edit could break.
pub const EXIT_NOT_A_REPOSITORY: i32 = 3;

/// Exit code for a request the repository could not answer — a bad line number, an unreadable
/// object, a file that is not there. Maps to `Failed`, which is again not an empty history.
pub const EXIT_FAILED: i32 = 4;

/// What to ask the history.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Commits touching the path, newest first.
    Log,
    /// The commit that introduced one line of one file.
    Blame,
}

impl std::str::FromStr for Mode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "log" => Ok(Mode::Log),
            "blame" => Ok(Mode::Blame),
            other => Err(format!(
                "unknown mode `{other}` (expected `log` or `blame`)"
            )),
        }
    }
}

impl std::fmt::Display for Mode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Mode::Log => "log",
            Mode::Blame => "blame",
        })
    }
}

/// A `git_log` request, shared between the model-facing tool (which fills it from the tool call and
/// serialises it onto the worker's argv) and the worker subcommand (which parses it back).
#[derive(Args, Debug, Clone)]
pub struct GitLogArgs {
    /// The file or directory to ask about. The repository is discovered from here upwards, so a
    /// path deep inside a worktree works without naming the root.
    #[arg(long, default_value = ".")]
    pub path: PathBuf,
    /// `log` or `blame`.
    #[arg(long, default_value = "log")]
    pub mode: Mode,
    /// The 1-indexed line to attribute. Required by `blame`, ignored by `log`.
    #[arg(long)]
    pub line: Option<u32>,
    /// Stop after this many commits. Bounds output and walk time on a deep history — the same guard
    /// `search` and `ast_grep` use.
    #[arg(long, default_value_t = 50)]
    pub limit: usize,
}

/// Turn a [`GitLogArgs`] into the argv tail that reconstructs it under `bee gitlog-worker`.
pub fn worker_argv(args: &GitLogArgs) -> Vec<String> {
    let mut argv = vec![
        "gitlog-worker".to_string(),
        "--path".to_string(),
        args.path.to_string_lossy().into_owned(),
        "--mode".to_string(),
        args.mode.to_string(),
        "--limit".to_string(),
        args.limit.to_string(),
    ];
    if let Some(line) = args.line {
        argv.push("--line".to_string());
        argv.push(line.to_string());
    }
    argv
}

/// Why a history could not be read. Deliberately *not* an empty result — the worker maps each of
/// these to a non-zero exit with a diagnostic on stderr, and the tool wrapper maps the exit code to
/// `Unavailable`/`Failed` (contract `tool-outcome.md`).
#[derive(Debug)]
pub enum GitLogError {
    /// Nothing at or above the path is a repository.
    NotARepository { path: PathBuf },
    /// The repository is there but the request cannot be answered.
    Failed(String),
}

impl std::fmt::Display for GitLogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GitLogError::NotARepository { path } => write!(
                f,
                "{} is not inside a repository, so it has no history to report",
                path.display()
            ),
            GitLogError::Failed(detail) => f.write_str(detail),
        }
    }
}

impl GitLogError {
    /// The exit code the wrapper reads.
    pub fn exit_code(&self) -> i32 {
        match self {
            GitLogError::NotARepository { .. } => EXIT_NOT_A_REPOSITORY,
            GitLogError::Failed(_) => EXIT_FAILED,
        }
    }
}

/// One commit, in the shape both modes report it.
#[derive(Debug, Clone, PartialEq)]
pub struct Commit {
    /// Abbreviated object id — long enough to be unambiguous in any repository a person works in,
    /// short enough to read in a line.
    pub id: String,
    /// Author time, RFC 3339 in UTC. A repository records an offset per commit; normalising to UTC
    /// keeps two commits comparable at a glance, which is what a reader of a log actually does.
    pub date: String,
    pub author: String,
    /// First line of the message.
    pub summary: String,
    /// For `blame`: the line this attribution is about.
    pub line: Option<u32>,
}

impl std::fmt::Display for Commit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.line {
            Some(line) => write!(
                f,
                "{} {} {} line {}: {}",
                self.id, self.date, self.author, line, self.summary
            ),
            None => write!(
                f,
                "{} {} {} {}",
                self.id, self.date, self.author, self.summary
            ),
        }
    }
}

const SHORT_ID: usize = 8;

/// Answer the request, or fail in a way that cannot be mistaken for "nothing found".
pub fn query(args: &GitLogArgs) -> Result<Vec<Commit>, GitLogError> {
    // Discovery walks upwards, so `--path src/main.rs` finds the repository the file lives in. A
    // path with no repository above it is the refusal this whole module is careful about.
    let start = if args.path.is_file() {
        args.path.parent().unwrap_or(Path::new(".")).to_path_buf()
    } else {
        args.path.clone()
    };
    let repo = gix::discover(&start).map_err(|_| GitLogError::NotARepository {
        path: args.path.clone(),
    })?;

    match args.mode {
        Mode::Log => log(&repo, args),
        Mode::Blame => blame(&repo, args),
    }
}

/// Commits touching `path`, newest first.
///
/// A directory (or the repository root) reports the whole history; a file reports only the commits
/// that changed it, which is what a reader asking about one file means. The filter is done by
/// comparing the blob the path resolves to across each commit and its first parent — enough for the
/// question being asked, and it avoids materialising a full tree diff per commit on a deep history.
fn log(repo: &gix::Repository, args: &GitLogArgs) -> Result<Vec<Commit>, GitLogError> {
    let head = match repo.head_id() {
        Ok(h) => h,
        // A repository with no commits yet: an empty history, and genuinely so. Not a refusal.
        Err(_) => return Ok(Vec::new()),
    };

    let relative = relative_to_workdir(repo, &args.path);
    let mut out = Vec::new();

    for info in head.ancestors().all().map_err(fail)? {
        if out.len() >= args.limit {
            break;
        }
        let info = info.map_err(fail)?;
        let commit = info.object().map_err(fail)?;

        if let Some(rel) = &relative {
            if !touches(repo, &commit, rel) {
                continue;
            }
        }
        out.push(describe(&commit, None)?);
    }
    Ok(out)
}

/// The commit that introduced one line.
fn blame(repo: &gix::Repository, args: &GitLogArgs) -> Result<Vec<Commit>, GitLogError> {
    let Some(line) = args.line else {
        return Err(GitLogError::Failed(
            "blame needs a line number (`--line`)".to_string(),
        ));
    };
    if line == 0 {
        return Err(GitLogError::Failed("line numbers start at 1".to_string()));
    }
    let Some(relative) = relative_to_workdir(repo, &args.path) else {
        return Err(GitLogError::Failed(format!(
            "{} is not a file inside this repository's worktree",
            args.path.display()
        )));
    };

    let head = repo
        .head_id()
        .map_err(|e| GitLogError::Failed(format!("no commit to blame from: {e}")))?;

    // Blame the single line asked about rather than the whole file: the answer is one hunk, and
    // walking the rest is work whose result is thrown away.
    let ranges = gix::blame::BlameRanges::from_one_based_inclusive_range(line..=line)
        .map_err(|e| GitLogError::Failed(format!("line {line} is not a blameable range: {e}")))?;
    let options = gix::repository::blame_file::Options {
        ranges,
        ..Default::default()
    };

    let outcome = repo
        .blame_file(
            gix::path::os_str_into_bstr(relative.as_os_str())
                .map_err(|e| GitLogError::Failed(format!("path is not valid UTF-8: {e}")))?,
            head.detach(),
            options,
        )
        .map_err(|e| GitLogError::Failed(format!("cannot blame {}: {e}", relative.display())))?;

    let Some(entry) = outcome.entries.first() else {
        // The file exists in the worktree but the line has no attribution — most often a line past
        // the end of the file. Refusing beats inventing a commit for a line that is not there.
        return Err(GitLogError::Failed(format!(
            "line {line} of {} has no attribution in the history (is the file that long, and \
             tracked?)",
            relative.display()
        )));
    };

    let object = repo
        .find_object(entry.commit_id)
        .map_err(|e| GitLogError::Failed(format!("cannot read the introducing commit: {e}")))?;
    let commit = object
        .try_into_commit()
        .map_err(|e| GitLogError::Failed(format!("the blamed object is not a commit: {e}")))?;
    Ok(vec![describe(&commit, Some(line))?])
}

/// Did `commit` change `path`, relative to its first parent? A root commit counts as touching every
/// path it contains.
fn touches(repo: &gix::Repository, commit: &gix::Commit<'_>, path: &Path) -> bool {
    let blob_at = |c: &gix::Commit<'_>| -> Option<gix::ObjectId> {
        let mut tree = c.tree().ok()?;
        let entry = tree.peel_to_entry_by_path(path).ok()??;
        Some(entry.object_id())
    };

    let here = blob_at(commit);
    let parent = commit
        .parent_ids()
        .next()
        .and_then(|id| repo.find_object(id).ok())
        .and_then(|o| o.try_into_commit().ok())
        .and_then(|p| blob_at(&p));

    match (here, parent) {
        // Present now, absent or different before: this commit changed it.
        (Some(a), Some(b)) => a != b,
        (Some(_), None) => true,
        // Deleted here. Still a change to the path, and a reader asking about a file that was
        // removed wants to see the removal.
        (None, Some(_)) => true,
        (None, None) => false,
    }
}

/// The path relative to the worktree root, or `None` when the path *is* the root (or lies outside
/// it, which `log` treats as "no filter" and `blame` refuses).
fn relative_to_workdir(repo: &gix::Repository, path: &Path) -> Option<PathBuf> {
    let workdir = repo.workdir()?;
    let workdir = workdir.canonicalize().ok()?;
    let absolute = if path.is_absolute() {
        path.canonicalize().ok()?
    } else {
        std::env::current_dir()
            .ok()?
            .join(path)
            .canonicalize()
            .ok()?
    };
    let rel = absolute.strip_prefix(&workdir).ok()?;
    if rel.as_os_str().is_empty() {
        None
    } else {
        Some(rel.to_path_buf())
    }
}

fn describe(commit: &gix::Commit<'_>, line: Option<u32>) -> Result<Commit, GitLogError> {
    let author = commit.author().map_err(fail)?;
    let message = commit.message().map_err(fail)?;
    Ok(Commit {
        id: commit.id().to_hex_with_len(SHORT_ID).to_string(),
        date: format_time(author.time().map_err(fail)?),
        author: author.name.to_string(),
        summary: message.summary().to_string(),
        line,
    })
}

/// RFC 3339 in UTC. `gix_date::Time` carries seconds since the epoch plus the author's offset; the
/// offset is deliberately dropped rather than rendered, so two commits from two timezones sort and
/// read the way a reader assumes they do.
fn format_time(t: gix::date::Time) -> String {
    time::OffsetDateTime::from_unix_timestamp(t.seconds)
        .map(|dt| {
            dt.format(&time::format_description::well_known::Rfc3339)
                .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
        })
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

fn fail(e: impl std::fmt::Display) -> GitLogError {
    GitLogError::Failed(e.to_string())
}

/// The `gitlog-worker` subcommand: print one commit per line, or exit non-zero with a diagnostic.
///
/// Returning the exit code rather than printing an error and returning `()` is what lets the wrapper
/// tell `NotARepository` from `Failed` from a genuinely empty history — three outcomes that all look
/// like "no output" from the outside.
pub fn run_worker(args: &GitLogArgs) -> i32 {
    match query(args) {
        Ok(commits) => {
            for c in &commits {
                println!("{c}");
            }
            0
        }
        Err(e) => {
            eprintln!("git_log: {e}");
            e.exit_code()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_argv_round_trips_a_blame_request() {
        let argv = worker_argv(&GitLogArgs {
            path: PathBuf::from("src/main.rs"),
            mode: Mode::Blame,
            line: Some(42),
            limit: 50,
        });
        assert_eq!(argv[0], "gitlog-worker");
        assert!(argv.contains(&"--line".to_string()));
        assert!(argv.contains(&"42".to_string()));
        assert!(argv.contains(&"blame".to_string()));
    }

    #[test]
    fn a_log_request_carries_no_line() {
        let argv = worker_argv(&GitLogArgs {
            path: PathBuf::from("."),
            mode: Mode::Log,
            line: None,
            limit: 10,
        });
        assert!(!argv.contains(&"--line".to_string()));
    }

    #[test]
    fn the_two_refusals_have_distinct_exit_codes() {
        // The wrapper distinguishes them by code, so they must never collide — and neither may be 0,
        // which is what an empty history exits with.
        let not_repo = GitLogError::NotARepository {
            path: PathBuf::from("/tmp"),
        };
        let failed = GitLogError::Failed("bad line".into());
        assert_ne!(not_repo.exit_code(), failed.exit_code());
        assert_ne!(not_repo.exit_code(), 0);
        assert_ne!(failed.exit_code(), 0);
    }

    #[test]
    fn a_commit_renders_differently_for_log_and_blame() {
        let base = Commit {
            id: "abcd1234".into(),
            date: "2026-07-28T00:00:00Z".into(),
            author: "Ada Lovelace".into(),
            summary: "first: add the file".into(),
            line: None,
        };
        assert!(!base.to_string().contains("line"));
        let blamed = Commit {
            line: Some(3),
            ..base
        };
        assert!(blamed.to_string().contains("line 3:"));
    }
}
