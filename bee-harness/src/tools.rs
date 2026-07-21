//! The tool trait, its result type, and the name→tool registry (contracts/tool-contracts.md).
//!
//! Every tool executes as a **sandboxed child inside the episode's bee scope** (via [`Sandbox`]) and
//! returns a [`ToolResult`]. A tool MUST NOT panic on bad arguments — it returns an error result
//! (FR-016). Concrete tools (`bash`, `read_file`, `write_file`, `list_directory`) land in T014/T015.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::provider::{ToolCall, ToolSchema};
use crate::sandbox::Sandbox;

pub mod bash;
pub mod ctf;
pub mod exec;
pub mod files;
pub mod render;

use crate::render_spec::RenderSpec;

/// The default tool set advertised to the model (contracts/scenario-schema.md). The CTF terminal
/// tools (`submit_flag`, `give_up`) are **not** here — a scenario opts into them via its `tools`
/// list when `mode = "ctf"`.
pub const DEFAULT_TOOLS: &[&str] = &["bash", "read_file", "write_file", "list_directory"];

/// The CTF terminal tools (US3), enabled only when a scenario lists them.
pub const CTF_TOOLS: &[&str] = &["submit_flag", "give_up"];

/// The visualization tools (003-visual-render, US6). **Not** in `DEFAULT_TOOLS` (NFR-003): a
/// scenario or the REPL config opts in by listing `"render"`.
pub const RENDER_TOOLS: &[&str] = &["render"];

/// Every tool name the harness knows how to build. A scenario may only list names from this set
/// (scenario validation rejects the rest before a run).
pub fn is_known_tool(name: &str) -> bool {
    DEFAULT_TOOLS.contains(&name) || CTF_TOOLS.contains(&name) || RENDER_TOOLS.contains(&name)
}

/// The outcome of one tool call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    /// stdout (+ formatted stderr/exit for `bash`), or the file/dir payload, or an error message.
    pub content: String,
    /// Process exit code, for process-backed tools.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    /// True for a kernel denial (`EACCES`), a non-zero exit, or a malformed-arg error.
    pub is_error: bool,
    /// True if `content` was capped (FR-015).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
    /// Pre-truncation byte length, when `truncated`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_len: Option<usize>,
    /// When true, this tool call ends the episode (the CTF terminal tools `submit_flag`/`give_up`,
    /// US3). The loop is otherwise tool-name-agnostic — it reads this flag, not the name.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub terminal: bool,
    /// The visualization to render, when this result came from the `render` tool (003-visual-render,
    /// FR-023). **Never sent to the model** — `content` carries the human-readable summary instead;
    /// recorded in the transcript for later re-rendering.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub render_spec: Option<RenderSpec>,
}

impl ToolResult {
    /// A successful, non-truncated text result.
    pub fn ok(content: impl Into<String>) -> Self {
        ToolResult {
            content: content.into(),
            exit_code: None,
            is_error: false,
            truncated: false,
            original_len: None,
            terminal: false,
            render_spec: None,
        }
    }

    /// An error result (bad args, denial, missing file, …). `is_error = true`.
    pub fn error(content: impl Into<String>) -> Self {
        ToolResult {
            content: content.into(),
            exit_code: None,
            is_error: true,
            truncated: false,
            original_len: None,
            terminal: false,
            render_spec: None,
        }
    }

    /// A successful `render`-tool result: `content` is the text summary, `render_spec` the widget
    /// (003-visual-render, FR-023).
    pub fn rendered(summary: impl Into<String>, spec: RenderSpec) -> Self {
        ToolResult { render_spec: Some(spec), ..ToolResult::ok(summary) }
    }

    /// A successful **terminal** result (a CTF tool that ends the episode, US3). `terminal = true`,
    /// `is_error = false`.
    pub fn terminal_ok(content: impl Into<String>) -> Self {
        ToolResult { terminal: true, ..ToolResult::ok(content) }
    }

    /// The FR-016 malformed-arguments result: `"<tool>: invalid arguments: <detail>"`.
    pub fn invalid_args(tool: &str, detail: impl std::fmt::Display) -> Self {
        ToolResult::error(format!("{tool}: invalid arguments: {detail}"))
    }
}

/// A tool the agent may call. Object-safe so tools live in a `Box<dyn Tool>` registry.
#[async_trait::async_trait]
pub trait Tool: Send + Sync {
    /// The tool's stable name (matches [`ToolCall::name`]).
    fn name(&self) -> &'static str;

    /// The JSON-Schema-bearing schema advertised to the model.
    fn schema(&self) -> ToolSchema;

    /// Execute inside `sandbox`. MUST NOT panic on bad `arguments` — return an error result.
    async fn call(&self, arguments: serde_json::Value, sandbox: &Sandbox) -> ToolResult;
}

/// A name→tool map plus the schema list handed to the model each turn.
#[derive(Default)]
pub struct ToolRegistry {
    tools: BTreeMap<&'static str, Box<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        ToolRegistry::default()
    }

    /// Register a tool (last registration under a name wins).
    pub fn insert(&mut self, tool: Box<dyn Tool>) {
        self.tools.insert(tool.name(), tool);
    }

    /// Register a tool, chaining.
    pub fn with(mut self, tool: Box<dyn Tool>) -> Self {
        self.insert(tool);
        self
    }

    /// True if a tool with `name` is registered.
    pub fn contains(&self, name: &str) -> bool {
        self.tools.contains_key(name)
    }

    /// The schemas for all registered tools, in stable name order.
    pub fn schemas(&self) -> Vec<ToolSchema> {
        self.tools.values().map(|t| t.schema()).collect()
    }

    /// Drop every `mcp__…` tool. Used to rebuild the MCP set after a `tools/list_changed`
    /// notification (004-mcp-client, FR-043) — the bridge then re-registers the current tools.
    pub fn remove_mcp_tools(&mut self) {
        self.tools.retain(|name, _| !name.starts_with("mcp__"));
    }

    /// Execute a model-requested call. An unknown tool name yields an error result (fed back to the
    /// model), never a panic (FR-016).
    pub async fn execute(&self, call: &ToolCall, sandbox: &Sandbox) -> ToolResult {
        match self.tools.get(call.name.as_str()) {
            Some(tool) => tool.call(call.arguments.clone(), sandbox).await,
            None => ToolResult::error(format!("unknown tool: {}", call.name)),
        }
    }
}

/// Build a registry holding just the `enabled` tools (validated names; unknown names are dropped —
/// [`crate::scenario::Scenario`] validation already rejects them before a run). `flag` is the CTF
/// sentinel value (US3): `submit_flag` needs it at construction to check submissions, so it is only
/// registered when `flag` is `Some`.
pub fn registry_for(enabled: &[String], flag: Option<&str>) -> ToolRegistry {
    let mut r = ToolRegistry::new();
    for name in enabled {
        match name.as_str() {
            "bash" => r.insert(Box::new(bash::Bash)),
            "read_file" => r.insert(Box::new(files::ReadFile)),
            "write_file" => r.insert(Box::new(files::WriteFile)),
            "list_directory" => r.insert(Box::new(files::ListDirectory)),
            "submit_flag" => {
                if let Some(value) = flag {
                    r.insert(Box::new(ctf::SubmitFlag::new(value.to_string())));
                }
            }
            "give_up" => r.insert(Box::new(ctf::GiveUp)),
            "render" => r.insert(Box::new(render::RenderTool::new())),
            _ => {}
        }
    }
    r
}
