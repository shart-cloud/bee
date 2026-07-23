//! The `bash` tool: run a shell command (`sh -c <command>`) inside the scope.

use serde::Deserialize;
use serde_json::json;

use crate::provider::ToolSchema;
use crate::sandbox::Sandbox;
use crate::tools::exec::run_child;
use crate::tools::{Tool, ToolResult};
use crate::transcript::DEFAULT_OUTPUT_CAP;

#[derive(Deserialize)]
struct Args {
    command: String,
}

/// `{ "command": string }` → `sh -c <command>` in the sandbox.
pub struct Bash;

#[async_trait::async_trait]
impl Tool for Bash {
    fn name(&self) -> &'static str {
        "bash"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "bash".to_string(),
            description: "Run a shell command via `sh -c` inside the sandbox. Returns stdout, any \
                          stderr, and the exit code. A non-zero exit (including a kernel permission \
                          denial) is reported as an error."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "The shell command to execute." }
                },
                "required": ["command"]
            }),
        }
    }

    async fn call(&self, arguments: serde_json::Value, sandbox: &Sandbox) -> ToolResult {
        let args: Args = match serde_json::from_value(arguments) {
            Ok(a) => a,
            Err(e) => return ToolResult::invalid_args("bash", e),
        };
        run_child(
            sandbox,
            "sh",
            &["-c".to_string(), args.command],
            None,
            DEFAULT_OUTPUT_CAP,
        )
        .await
    }
}
