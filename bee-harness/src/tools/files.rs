//! Structured file tools (FR-004): `read_file`, `write_file`, `list_directory`. Each takes typed
//! arguments but still executes as a sandboxed child (`cat`/`ls`/`sh`) so the LSM enforces the
//! operation (contracts/tool-contracts.md — tools never pre-check paths in user space).

use serde::Deserialize;
use serde_json::json;

use crate::provider::ToolSchema;
use crate::sandbox::Sandbox;
use crate::tools::exec::run_child;
use crate::tools::{Tool, ToolResult};
use crate::transcript::DEFAULT_OUTPUT_CAP;

fn string_prop(name: &str, desc: &str, required: &[&str]) -> serde_json::Value {
    json!({
        "type": "object",
        "properties": { name: { "type": "string", "description": desc } },
        "required": required
    })
}

#[derive(Deserialize)]
struct PathArgs {
    path: String,
}

/// `{ "path": string }` → `cat -- <path>`.
pub struct ReadFile;

#[async_trait::async_trait]
impl Tool for ReadFile {
    fn name(&self) -> &'static str {
        "read_file"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "read_file".to_string(),
            description: "Read a file's contents (UTF-8, lossy). Errors if the file is missing or \
                          access is denied."
                .to_string(),
            parameters: string_prop("path", "Path to the file to read.", &["path"]),
        }
    }
    async fn call(&self, arguments: serde_json::Value, sandbox: &Sandbox) -> ToolResult {
        let args: PathArgs = match serde_json::from_value(arguments) {
            Ok(a) => a,
            Err(e) => return ToolResult::invalid_args("read_file", e),
        };
        run_child(
            sandbox,
            "cat",
            &["--".to_string(), args.path],
            None,
            DEFAULT_OUTPUT_CAP,
        )
        .await
    }
}

#[derive(Deserialize)]
struct WriteArgs {
    path: String,
    #[serde(default)]
    content: String,
}

/// `{ "path": string, "content": string }` → writes `content` to `path` (truncating) in the scope.
pub struct WriteFile;

#[async_trait::async_trait]
impl Tool for WriteFile {
    fn name(&self) -> &'static str {
        "write_file"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "write_file".to_string(),
            description: "Write text to a file (truncating it). Errors if the write is denied by \
                          policy or fails."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Destination file path." },
                    "content": { "type": "string", "description": "Text to write." }
                },
                "required": ["path", "content"]
            }),
        }
    }
    async fn call(&self, arguments: serde_json::Value, sandbox: &Sandbox) -> ToolResult {
        let args: WriteArgs = match serde_json::from_value(arguments) {
            Ok(a) => a,
            Err(e) => return ToolResult::invalid_args("write_file", e),
        };
        let n = args.content.len();
        let path = args.path.clone();
        // `tee -- <path>`: no shell layer — the path is a direct argv element (so quotes/spaces/`$`
        // in it are inert), `--` guards a leading dash, and `tee` truncates + writes stdin to the
        // file inside the scope (its stdout echo is discarded). The LSM enforces the write-open.
        let res = run_child(
            sandbox,
            "tee",
            &["--".to_string(), args.path],
            Some(args.content.into_bytes()),
            DEFAULT_OUTPUT_CAP,
        )
        .await;
        if res.is_error {
            res
        } else {
            ToolResult::ok(format!("wrote {n} bytes to {path}"))
        }
    }
}

/// `{ "path": string }` → `ls -1Ap -- <path>`.
pub struct ListDirectory;

#[async_trait::async_trait]
impl Tool for ListDirectory {
    fn name(&self) -> &'static str {
        "list_directory"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "list_directory".to_string(),
            description: "List a directory's entries (one per line; trailing '/' marks subdirs). \
                          Errors if the directory is missing or access is denied."
                .to_string(),
            parameters: string_prop("path", "Directory path to list.", &["path"]),
        }
    }
    async fn call(&self, arguments: serde_json::Value, sandbox: &Sandbox) -> ToolResult {
        let args: PathArgs = match serde_json::from_value(arguments) {
            Ok(a) => a,
            Err(e) => return ToolResult::invalid_args("list_directory", e),
        };
        run_child(
            sandbox,
            "ls",
            &["-1Ap".to_string(), "--".to_string(), args.path],
            None,
            DEFAULT_OUTPUT_CAP,
        )
        .await
    }
}
