//! The `ast_grep` tool (016-native-tools US1): structural, syntax-aware code search.
//!
//! Like [`crate::tools::search`], this tool's "binary" is bee itself: it execs `bee astgrep-worker …`
//! through [`run_child`], so the tree-sitter parse in [`crate::astgrep`] runs in a process that has
//! joined the scope cgroup and is subject to the LSM. Every file the search opens goes through the
//! scope's `file_open` policy, same as any other tool child. Parsing in the harness instead would
//! read *around* the sandbox.
//!
//! The argument surface is deliberately narrow — a language, a pattern, a root, and nothing that can
//! rewrite. `ast-grep-core` supports replacement; this tool does not expose it (spec Assumptions).

use serde::Deserialize;
use serde_json::json;

use crate::astgrep::{compiled_in, is_compiled_in, worker_argv, AstGrepArgs};
use crate::provider::ToolSchema;
use crate::sandbox::Sandbox;
use crate::tools::exec::run_child;
use crate::tools::outcome::{audit_refusal, ToolOutcome, UnavailableReason};
use crate::tools::{Tool, ToolResult};
use crate::transcript::DEFAULT_OUTPUT_CAP;

/// The cap on matches a single call returns. Also bounds walk time on a large tree. Same value, and
/// same reasoning, as `search`'s `MATCH_LIMIT`.
const MATCH_LIMIT: usize = 200;

#[derive(Deserialize)]
struct Args {
    lang: String,
    pattern: String,
    #[serde(default = "default_path")]
    path: String,
}

fn default_path() -> String {
    ".".to_string()
}

/// `{ "lang": string, "pattern": string, "path"?: string }` → structural search.
pub struct AstGrepTool;

#[async_trait::async_trait]
impl Tool for AstGrepTool {
    fn name(&self) -> &'static str {
        "ast_grep"
    }

    fn schema(&self) -> ToolSchema {
        // Advertise the languages this build actually has. A model that can read the list will not
        // waste a turn asking for a grammar that is not here.
        let langs = compiled_in();
        ToolSchema {
            name: "ast_grep".to_string(),
            description: format!(
                "Search code by syntactic structure rather than text, using ast-grep patterns. \
                 Matches come from the parse tree, so occurrences inside comments and string \
                 literals are NOT returned — use this instead of `search` when the text would be \
                 ambiguous. Patterns use `$VAR` as a wildcard: `$X.unwrap()` matches any receiver, \
                 `fn $NAME($$$) {{ $$$ }}` matches any function. Returns `path:line:text`, capped at \
                 {MATCH_LIMIT}. Languages available in this build: {}.",
                if langs.is_empty() {
                    "(none)".to_string()
                } else {
                    langs.join(", ")
                }
            ),
            parameters: json!({
                "type": "object",
                "properties": {
                    "lang": {
                        "type": "string",
                        "description": "Language whose grammar parses the files.",
                        "enum": langs,
                    },
                    "pattern": {
                        "type": "string",
                        "description": "The ast-grep pattern, e.g. \"$X.unwrap()\"."
                    },
                    "path": {
                        "type": "string",
                        "description": "Directory or file to search under. Defaults to the current directory."
                    }
                },
                "required": ["lang", "pattern"]
            }),
        }
    }

    async fn call(&self, arguments: serde_json::Value, sandbox: &Sandbox) -> ToolResult {
        let args: Args = match serde_json::from_value(arguments) {
            Ok(a) => a,
            Err(e) => return ToolResult::invalid_args("ast_grep", e),
        };

        // Refuse an uncompiled language *here*, before spawning anything. Two reasons: it saves a
        // process, and — the load-bearing one — `SupportLang`'s parser lookup panics for a grammar
        // whose feature is off, so the refusal has to happen from bee's own registry (research R12).
        if !is_compiled_in(&args.lang) {
            let reason = UnavailableReason::LanguageUnsupported {
                lang: args.lang.clone(),
                compiled_in: compiled_in(),
            };
            audit_refusal("ast_grep", &reason);
            return ToolOutcome::<()>::unavailable(reason).into_tool_result(|_| String::new());
        }

        // The worker is this very executable, exec'd through the sandbox so it joins the scope. If we
        // cannot name our own binary there is no safe fallback — parsing here would search outside
        // the sandbox — so fail the call rather than escape the scope.
        let exe = match std::env::current_exe() {
            Ok(p) => p.to_string_lossy().into_owned(),
            Err(e) => {
                return ToolResult::error(format!("ast_grep: cannot locate bee executable: {e}"))
            }
        };

        let req = AstGrepArgs {
            lang: args.lang,
            path: args.path.into(),
            limit: MATCH_LIMIT,
            pattern: args.pattern,
        };
        run_child(sandbox, &exe, &worker_argv(&req), None, DEFAULT_OUTPUT_CAP).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_schema_advertises_only_compiled_in_languages() {
        let schema = AstGrepTool.schema();
        let enumerated = schema.parameters["properties"]["lang"]["enum"]
            .as_array()
            .expect("lang should enumerate available grammars")
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect::<Vec<_>>();
        assert_eq!(enumerated, compiled_in());
    }

    #[test]
    fn the_description_tells_the_model_what_this_does_that_search_cannot() {
        let schema = AstGrepTool.schema();
        assert!(schema.description.contains("comments"));
        assert!(schema.description.contains("string literals"));
    }
}
