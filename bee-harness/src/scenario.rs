//! Scenario definition (contracts/scenario-schema.md): the environment + task an episode runs. A
//! bee policy (feature 001 TOML) confines it; the workdir is materialized before the run.

use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::config::ConfigError;
use crate::tools::{is_known_tool, DEFAULT_TOOLS};

/// Scoring mode. `Ctf` (US3) plants a flag and enables the terminal `submit_flag`/`give_up` tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScoringMode {
    #[default]
    Standard,
    Ctf,
}

/// A file to create in the workdir before the run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileSpec {
    pub path: PathBuf,
    #[serde(default)]
    pub content: String,
}

/// A planted CTF flag (US3): a sentinel `value` written to `path` before the run. The agent's goal
/// is to read it and `submit_flag` the value.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlagSpec {
    pub path: PathBuf,
    pub value: String,
}

/// Files/dirs to materialize before the episode, plus an optional planted flag (US3).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkdirSetup {
    #[serde(default)]
    pub create_dirs: Vec<PathBuf>,
    #[serde(default)]
    pub create_files: Vec<FileSpec>,
    #[serde(default)]
    pub flag: Option<FlagSpec>,
}

/// A scenario: policy + task + limits + enabled tools.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scenario {
    pub id: String,
    /// bee security policy (feature 001 TOML) compiled for the scope.
    pub policy_path: PathBuf,
    pub system_prompt: String,
    pub task: String,
    /// Max tool-call rounds (FR-007).
    pub turn_limit: u32,
    /// Wall-clock cap in seconds (FR-007).
    pub timeout_secs: u64,
    #[serde(default = "default_tools")]
    pub tools: Vec<String>,
    #[serde(default)]
    pub mode: ScoringMode,
    #[serde(default)]
    pub workdir: WorkdirSetup,
}

fn default_tools() -> Vec<String> {
    DEFAULT_TOOLS.iter().map(|s| s.to_string()).collect()
}

#[derive(Deserialize)]
struct ScenarioFile {
    scenario: Scenario,
}

impl Scenario {
    /// The wall-clock deadline as a `Duration`.
    pub fn timeout(&self) -> Duration {
        Duration::from_secs(self.timeout_secs)
    }

    /// Load + validate a scenario TOML.
    pub fn from_path(path: &std::path::Path) -> Result<Scenario, ConfigError> {
        let text = std::fs::read_to_string(path).map_err(|e| ConfigError::read(path, e))?;
        let file: ScenarioFile = toml::from_str(&text).map_err(|e| ConfigError::parse(path, e))?;
        let mut s = file.scenario;
        // Resolve a relative policy_path against the scenario file's directory for portability.
        if s.policy_path.is_relative() {
            if let Some(dir) = path.parent() {
                let joined = dir.join(&s.policy_path);
                if joined.exists() {
                    s.policy_path = joined;
                }
            }
        }
        s.validate()?;
        Ok(s)
    }

    fn validate(&self) -> Result<(), ConfigError> {
        for (field, val) in [
            ("id", &self.id),
            ("system_prompt", &self.system_prompt),
            ("task", &self.task),
        ] {
            if val.trim().is_empty() {
                return Err(ConfigError::Invalid(format!("scenario.{field} is required")));
            }
        }
        if self.policy_path.as_os_str().is_empty() {
            return Err(ConfigError::Invalid("scenario.policy_path is required".into()));
        }
        if self.turn_limit < 1 {
            return Err(ConfigError::Invalid("scenario.turn_limit must be >= 1".into()));
        }
        if self.timeout_secs < 1 {
            return Err(ConfigError::Invalid("scenario.timeout_secs must be >= 1".into()));
        }
        // CTF mode (US3): a flag must be planted, and the terminal `submit_flag` tool must be
        // enabled (a CTF with no way to submit is unwinnable).
        match self.mode {
            ScoringMode::Ctf => {
                match &self.workdir.flag {
                    Some(f) if !f.path.as_os_str().is_empty() && !f.value.is_empty() => {}
                    _ => {
                        return Err(ConfigError::Invalid(
                            "mode = \"ctf\" requires [scenario.workdir.flag] with path and value"
                                .into(),
                        ))
                    }
                }
                if !self.tools.iter().any(|t| t == "submit_flag") {
                    return Err(ConfigError::Invalid(
                        "mode = \"ctf\" requires \"submit_flag\" in the tools list".into(),
                    ));
                }
            }
            // A flag on a non-CTF scenario is ignored (a warning would go here if we had a logger).
            ScoringMode::Standard => {}
        }
        // Reject tool names we don't ship, before the run.
        for t in &self.tools {
            if !is_known_tool(t) {
                return Err(ConfigError::Invalid(format!("unknown tool: {t}")));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(name: &str, body: &str) -> PathBuf {
        let tmp = std::env::temp_dir().join(format!("bee-scn-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let p = tmp.join("scenario.toml");
        std::fs::write(&p, body).unwrap();
        p
    }

    const OK: &str = r#"
[scenario]
id            = "hello"
policy_path   = "/etc/bee/policy.toml"
system_prompt = "You are a coding agent."
task          = "List the working dir."
turn_limit    = 5
timeout_secs  = 60
"#;

    #[test]
    fn loads_defaults() {
        let p = write("ok", OK);
        let s = Scenario::from_path(&p).unwrap();
        assert_eq!(s.tools, default_tools());
        assert_eq!(s.mode, ScoringMode::Standard);
        assert_eq!(s.timeout(), Duration::from_secs(60));
    }

    const CTF_FLAG: &str = r#"
[scenario.workdir.flag]
path  = "/secrets/flag.txt"
value = "FLAG{abc}"
"#;

    #[test]
    fn ctf_requires_flag() {
        // mode = "ctf" with submit_flag but no planted flag → rejected.
        let body =
            format!("{OK}mode = \"ctf\"\ntools = [\"bash\", \"submit_flag\"]\n");
        let p = write("ctf-noflag", &body);
        assert!(matches!(Scenario::from_path(&p), Err(ConfigError::Invalid(_))));
    }

    #[test]
    fn ctf_requires_submit_flag_tool() {
        // mode = "ctf" with a flag but no submit_flag tool → rejected.
        let body = format!("{OK}mode = \"ctf\"\ntools = [\"bash\"]\n{CTF_FLAG}");
        let p = write("ctf-notool", &body);
        assert!(matches!(Scenario::from_path(&p), Err(ConfigError::Invalid(_))));
    }

    #[test]
    fn ctf_mode_now_accepted() {
        // A valid CTF scenario parses (the slice-1 rejection is gone).
        let body = format!(
            "{OK}mode = \"ctf\"\ntools = [\"bash\", \"submit_flag\", \"give_up\"]\n{CTF_FLAG}"
        );
        let p = write("ctf-ok", &body);
        let s = Scenario::from_path(&p).expect("valid CTF scenario");
        assert_eq!(s.mode, ScoringMode::Ctf);
        assert_eq!(s.workdir.flag.as_ref().unwrap().value, "FLAG{abc}");
    }

    #[test]
    fn rejects_unknown_tool() {
        let p = write("badtool", &format!("{OK}tools = [\"bash\", \"nmap\"]\n"));
        assert!(matches!(Scenario::from_path(&p), Err(ConfigError::Invalid(_))));
    }

    #[test]
    fn rejects_zero_turn_limit() {
        let body = OK.replace("turn_limit    = 5", "turn_limit    = 0");
        let p = write("zero", &body);
        assert!(matches!(Scenario::from_path(&p), Err(ConfigError::Invalid(_))));
    }
}
