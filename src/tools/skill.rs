//! The `skill` tool (006-skills) — progressive disclosure of skill instructions.
//!
//! One tool advertises every model-facing skill in its **schema description** (name + trigger text,
//! re-derived each turn) and, when called, returns that skill's markdown body as the tool result so
//! the instructions enter the model's context on demand. It is **instructions-only**: loading a
//! skill reads a file host-side and changes nothing about the episode's capabilities, so — like
//! [`crate::mcp::proxy::McpToolProxy`] — it ignores the [`Sandbox`]. A skill's declared `requires`
//! block (capability grants) is honored by a separate, consent-gated slice, not here.

use std::sync::Arc;

use serde_json::json;

use crate::provider::ToolSchema;
use crate::sandbox::Sandbox;
use crate::skills::SkillRegistry;
use crate::tools::{Tool, ToolResult};

/// The model-facing `skill` tool, backed by a shared [`SkillRegistry`].
pub struct SkillTool {
    skills: Arc<SkillRegistry>,
}

impl SkillTool {
    /// Build the tool over a discovered registry. Register it only when the registry has at least one
    /// model-facing skill (an empty catalog would advertise a useless tool).
    pub fn new(skills: Arc<SkillRegistry>) -> Self {
        SkillTool { skills }
    }

    /// The names of skills the model may load, in stable order.
    fn model_facing_names(&self) -> Vec<String> {
        self.skills.model_facing().map(|s| s.name.clone()).collect()
    }
}

/// The schema description: a short instruction plus the catalog of loadable skills. Kept in the
/// schema (not the system prompt) so it re-derives from the live registry each turn and never
/// duplicates the per-skill trigger text.
fn catalog_description(skills: &SkillRegistry) -> String {
    let mut d = String::from(
        "Load a skill: returns that skill's instructions so you can follow them for a specialized \
         task. Consult this list and invoke the tool *before* starting work a skill covers, then \
         follow the returned instructions. Loading a skill only adds guidance — it grants no new \
         capabilities.\n\nAvailable skills:\n",
    );
    for s in skills.model_facing() {
        // The description is the trigger text — inject it verbatim, never truncated.
        d.push_str("- ");
        d.push_str(&s.name);
        d.push_str(": ");
        d.push_str(&s.description);
        d.push('\n');
    }
    d
}

#[async_trait::async_trait]
impl Tool for SkillTool {
    fn name(&self) -> &'static str {
        "skill"
    }

    fn schema(&self) -> ToolSchema {
        let names = self.model_facing_names();
        let mut name_prop = json!({
            "type": "string",
            "description": "The name of the skill to load."
        });
        // Constrain to the known names when there are any, so the model can't invent one.
        if !names.is_empty() {
            name_prop["enum"] = json!(names);
        }
        ToolSchema {
            name: "skill".to_string(),
            description: catalog_description(&self.skills),
            parameters: json!({
                "type": "object",
                "properties": {
                    "name": name_prop,
                    "input": {
                        "type": "string",
                        "description": "Optional input or arguments to pass along to the skill."
                    }
                },
                "required": ["name"]
            }),
        }
    }

    async fn call(&self, arguments: serde_json::Value, _sandbox: &Sandbox) -> ToolResult {
        let name = match arguments.get("name").and_then(|v| v.as_str()) {
            Some(n) => n,
            None => return ToolResult::invalid_args("skill", "missing string field 'name'"),
        };

        let skill = match self.skills.get(name) {
            // A user-only skill exists but is deliberately hidden from the model — treat as unknown
            // rather than leak it, and steer the model back to the advertised set.
            Some(s) if s.model_invocable => s,
            _ => {
                let available = self.model_facing_names().join(", ");
                return ToolResult::error(format!(
                    "unknown skill '{name}'. Available skills: {available}"
                ));
            }
        };

        match skill.body() {
            Ok(body) => ToolResult::ok(load_message(&skill.name, &body, &arguments)),
            Err(e) => ToolResult::error(format!("skill '{name}': could not read body: {e}")),
        }
    }
}

/// Frame the returned body so the model reads it as loaded skill instructions, appending any caller
/// `input`. Shared with the REPL `/skill` command so a skill loads identically either way.
pub fn load_message(name: &str, body: &str, arguments: &serde_json::Value) -> String {
    let mut out = format!("# Skill loaded: {name}\n\n{body}");
    if let Some(input) = arguments.get("input").and_then(|v| v.as_str()) {
        if !input.trim().is_empty() {
            out.push_str("\n\n## Input\n");
            out.push_str(input);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn registry_with(skills_md: &[(&str, &str)]) -> Arc<SkillRegistry> {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let root: PathBuf =
            std::env::temp_dir().join(format!("bee-skilltool-test-{}-{}", std::process::id(), n));
        for (dir, contents) in skills_md {
            let d = root.join(dir);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("SKILL.md"), contents).unwrap();
        }
        Arc::new(SkillRegistry::discover(&[root]))
    }

    fn host_sandbox() -> Sandbox {
        Sandbox::host(Vec::new())
    }

    #[tokio::test]
    async fn schema_lists_model_facing_skills_with_enum() {
        let reg = registry_with(&[
            ("alpha", "---\nname: alpha\ndescription: does alpha\n---\nAlpha body.\n"),
            (
                "beta",
                "---\nname: beta\ndescription: does beta\ndisable-model-invocation: true\n---\nBeta body.\n",
            ),
        ]);
        let tool = SkillTool::new(reg);
        let schema = tool.schema();
        assert!(schema.description.contains("alpha: does alpha"));
        // The model-hidden skill is not advertised.
        assert!(!schema.description.contains("beta"));
        let names = schema.parameters["properties"]["name"]["enum"]
            .as_array()
            .unwrap();
        assert_eq!(names.len(), 1);
        assert_eq!(names[0], "alpha");
    }

    #[tokio::test]
    async fn call_returns_body_framed_with_input() {
        let reg = registry_with(&[(
            "alpha",
            "---\nname: alpha\ndescription: d\n---\nStep 1. Do the thing.\n",
        )]);
        let tool = SkillTool::new(reg);
        let sbox = host_sandbox();
        let res = tool
            .call(json!({"name": "alpha", "input": "target=/tmp"}), &sbox)
            .await;
        assert!(!res.is_error);
        assert!(res.content.contains("# Skill loaded: alpha"));
        assert!(res.content.contains("Step 1. Do the thing."));
        assert!(res.content.contains("## Input\ntarget=/tmp"));
    }

    #[tokio::test]
    async fn unknown_skill_is_an_error_not_a_panic() {
        let reg = registry_with(&[("alpha", "---\nname: alpha\ndescription: d\n---\nx\n")]);
        let tool = SkillTool::new(reg);
        let sbox = host_sandbox();
        let res = tool.call(json!({"name": "ghost"}), &sbox).await;
        assert!(res.is_error);
        assert!(res.content.contains("unknown skill 'ghost'"));
        assert!(res.content.contains("alpha"));
    }

    #[tokio::test]
    async fn model_hidden_skill_cannot_be_loaded_by_the_model() {
        let reg = registry_with(&[(
            "secret",
            "---\nname: secret\ndescription: d\ndisable-model-invocation: true\n---\nx\n",
        )]);
        let tool = SkillTool::new(reg);
        let sbox = host_sandbox();
        let res = tool.call(json!({"name": "secret"}), &sbox).await;
        assert!(res.is_error);
    }

    #[tokio::test]
    async fn missing_name_is_invalid_args() {
        let reg = registry_with(&[("alpha", "---\nname: alpha\ndescription: d\n---\nx\n")]);
        let tool = SkillTool::new(reg);
        let sbox = host_sandbox();
        let res = tool.call(json!({}), &sbox).await;
        assert!(res.is_error);
    }
}
