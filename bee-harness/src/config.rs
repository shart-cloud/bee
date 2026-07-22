//! Provider configuration (contracts/scenario-schema.md). Declarative, Constitution IV. The key is
//! named by env var (`api_key_env`), never embedded — FR-018.

use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Errors from loading/validating a scenario or provider TOML.
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("cannot read {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("cannot parse {path}: {source}")]
    Parse {
        path: String,
        #[source]
        source: toml::de::Error,
    },
    #[error("invalid config: {0}")]
    Invalid(String),
}

impl ConfigError {
    pub(crate) fn read(path: &Path, source: std::io::Error) -> Self {
        ConfigError::Io {
            path: path.display().to_string(),
            source,
        }
    }
    pub(crate) fn parse(path: &Path, source: toml::de::Error) -> Self {
        ConfigError::Parse {
            path: path.display().to_string(),
            source,
        }
    }
}

/// Which provider shape the factory should build.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderType {
    Anthropic,
    #[serde(rename = "openai-compat")]
    OpenAiCompat,
    /// A deterministic scripted backend (no network / no key) — drives the offline demo and the VM
    /// enforcement case. Steps come from `provider.script`.
    Mock,
}

/// One scripted model turn for the `mock` provider: either a tool call or a final text turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MockStep {
    /// Tool name to call this turn (mutually exclusive with `text`).
    #[serde(default)]
    pub tool: Option<String>,
    /// Arguments for `tool` (JSON object).
    #[serde(default)]
    pub args: Option<serde_json::Value>,
    /// A text-only turn (ends the episode).
    #[serde(default)]
    pub text: Option<String>,
}

/// Anthropic **extended thinking** mode (008 / provider config). Maps to the `thinking` request
/// field. On current models (Opus 4.8/4.7, Sonnet 5, Fable 5) thinking is **off** unless set here —
/// omitting the field runs without reasoning. Anthropic-only; ignored for other providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingMode {
    /// `thinking = {type: "adaptive"}` — the model decides when and how much to reason (the only
    /// on-mode on current models).
    Adaptive,
    /// `thinking = {type: "disabled"}` — explicit off (accepted on Opus 4.8/4.7; rejected on Fable 5).
    Disabled,
}

/// Anthropic reasoning **effort** (008 / provider config). Sent as `output_config = {effort: …}`;
/// controls thinking depth and overall token spend. Anthropic-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Effort {
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}

/// A provider selection (which model / endpoint / key var).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub provider: ProviderType,
    /// Required for `openai-compat` (Ollama/OpenRouter/vLLM/OpenAI); ignored for Anthropic.
    #[serde(default)]
    pub base_url: Option<String>,
    /// Passed verbatim to Rig's `completion_model()` — use CURRENT ids (H8). Optional for `mock`.
    #[serde(default)]
    pub model: String,
    /// The **name** of the env var holding the key (never the key itself). Optional for `mock`.
    #[serde(default)]
    pub api_key_env: String,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub temperature: Option<f32>,
    /// Anthropic **extended thinking** (opt-in). `thinking = "adaptive"` lets the model decide when
    /// and how much to reason; omit to leave it off. Anthropic-only. Note: on Opus 4.8/4.7 (and
    /// Sonnet 5 / Fable 5) reasoning is **off** unless this is set. See [`ThinkingMode`].
    #[serde(default)]
    pub thinking: Option<ThinkingMode>,
    /// Anthropic reasoning **effort** (`low`|`medium`|`high`|`xhigh`|`max`). Sent as
    /// `output_config = {effort: …}`; omit for the provider default. Anthropic-only. See [`Effort`].
    #[serde(default)]
    pub effort: Option<Effort>,
    /// Enable Anthropic prompt caching — caches the system prompt, tool schemas, and the growing
    /// conversation prefix so repeated turns re-read them at ~0.1× cost instead of full price. On by
    /// default; ignored for non-Anthropic providers. Set `prompt_caching = false` in the provider
    /// TOML to disable.
    #[serde(default = "default_prompt_caching")]
    pub prompt_caching: bool,
    /// Scripted turns for the `mock` provider (ignored otherwise).
    #[serde(default)]
    pub script: Vec<MockStep>,
}

/// Default for [`ProviderConfig::prompt_caching`]: on. A free function because `#[serde(default)]`
/// on a `bool` would otherwise yield `false`, and we want caching enabled unless explicitly opted
/// out.
fn default_prompt_caching() -> bool {
    true
}

#[derive(Deserialize)]
struct ProviderFile {
    provider: ProviderConfig,
}

impl ProviderConfig {
    /// Load + validate a provider TOML.
    pub fn from_path(path: &Path) -> Result<ProviderConfig, ConfigError> {
        let cfg = Self::parse_unchecked(path)?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// Read + parse a provider TOML **without** semantic validation. The batch runner (US2) uses
    /// this so a config that parses but is semantically invalid (e.g. `openai-compat` with no
    /// `base_url`) surfaces at model construction as a per-episode error transcript rather than
    /// aborting the whole batch. A genuine *parse* failure (unreadable / malformed TOML) is still an
    /// error here — that becomes a `BatchError`, since we can't even name the model.
    pub fn parse_unchecked(path: &Path) -> Result<ProviderConfig, ConfigError> {
        let text = std::fs::read_to_string(path).map_err(|e| ConfigError::read(path, e))?;
        let file: ProviderFile = toml::from_str(&text).map_err(|e| ConfigError::parse(path, e))?;
        Ok(file.provider)
    }

    fn validate(&self) -> Result<(), ConfigError> {
        if self.provider == ProviderType::Mock {
            if self.script.is_empty() {
                return Err(ConfigError::Invalid(
                    "provider = \"mock\" requires a non-empty provider.script".into(),
                ));
            }
            return Ok(());
        }
        if self.model.trim().is_empty() {
            return Err(ConfigError::Invalid("provider.model is required".into()));
        }
        if self.api_key_env.trim().is_empty() {
            return Err(ConfigError::Invalid(
                "provider.api_key_env is required".into(),
            ));
        }
        if self.provider == ProviderType::OpenAiCompat
            && self.base_url.as_deref().unwrap_or("").trim().is_empty()
        {
            return Err(ConfigError::Invalid(
                "provider.base_url is required for openai-compat".into(),
            ));
        }
        // `thinking` / `effort` are Anthropic request fields; reject them on other providers rather
        // than silently ignoring the setting.
        if self.provider != ProviderType::Anthropic
            && (self.thinking.is_some() || self.effort.is_some())
        {
            return Err(ConfigError::Invalid(
                "provider.thinking / provider.effort apply only to the anthropic provider".into(),
            ));
        }
        Ok(())
    }

    /// The stable `"<provider>/<model>"` id recorded in the transcript.
    pub fn model_id(&self) -> String {
        let p = match self.provider {
            ProviderType::Anthropic => "anthropic",
            ProviderType::OpenAiCompat => "openai-compat",
            ProviderType::Mock => "mock",
        };
        let model = if self.model.is_empty() {
            "scripted"
        } else {
            &self.model
        };
        format!("{p}/{model}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(tmp: &Path, body: &str) -> std::path::PathBuf {
        let p = tmp.join("provider.toml");
        std::fs::write(&p, body).unwrap();
        p
    }

    #[test]
    fn anthropic_ok() {
        let tmp = std::env::temp_dir().join(format!("bee-cfg-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let p = write(
            &tmp,
            r#"
[provider]
provider    = "anthropic"
model       = "claude-opus-4-8"
api_key_env = "ANTHROPIC_API_KEY"
max_tokens  = 4096
"#,
        );
        let cfg = ProviderConfig::from_path(&p).unwrap();
        assert_eq!(cfg.provider, ProviderType::Anthropic);
        assert_eq!(cfg.model_id(), "anthropic/claude-opus-4-8");
        // Prompt caching defaults to on when the key is omitted.
        assert!(cfg.prompt_caching, "prompt caching should default to on");
    }

    #[test]
    fn prompt_caching_can_be_disabled() {
        let tmp = std::env::temp_dir().join(format!("bee-cfg-nocache-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let p = write(
            &tmp,
            r#"
[provider]
provider       = "anthropic"
model          = "claude-opus-4-8"
api_key_env    = "ANTHROPIC_API_KEY"
max_tokens     = 4096
prompt_caching = false
"#,
        );
        let cfg = ProviderConfig::from_path(&p).unwrap();
        assert!(
            !cfg.prompt_caching,
            "explicit opt-out should disable caching"
        );
    }

    #[test]
    fn openai_compat_requires_base_url() {
        let tmp = std::env::temp_dir().join(format!("bee-cfg2-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let p = write(
            &tmp,
            r#"
[provider]
provider    = "openai-compat"
model       = "qwen2.5-coder"
api_key_env = "OPENAI_API_KEY"
"#,
        );
        let err = ProviderConfig::from_path(&p).unwrap_err();
        assert!(matches!(err, ConfigError::Invalid(_)), "got: {err:?}");
    }

    #[test]
    fn thinking_and_effort_parse() {
        let tmp = std::env::temp_dir().join(format!("bee-cfg-think-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let p = write(
            &tmp,
            r#"
[provider]
provider    = "anthropic"
model       = "claude-opus-4-8"
api_key_env = "ANTHROPIC_API_KEY"
max_tokens  = 4096
thinking    = "adaptive"
effort      = "xhigh"
"#,
        );
        let cfg = ProviderConfig::from_path(&p).unwrap();
        assert_eq!(cfg.thinking, Some(ThinkingMode::Adaptive));
        assert_eq!(cfg.effort, Some(Effort::Xhigh));
    }

    #[test]
    fn thinking_rejected_on_non_anthropic() {
        let tmp = std::env::temp_dir().join(format!("bee-cfg-think-oai-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let p = write(
            &tmp,
            r#"
[provider]
provider    = "openai-compat"
model       = "qwen2.5-coder"
api_key_env = "OPENAI_API_KEY"
base_url    = "http://localhost:11434/v1"
thinking    = "adaptive"
"#,
        );
        let err = ProviderConfig::from_path(&p).unwrap_err();
        assert!(matches!(err, ConfigError::Invalid(_)), "got: {err:?}");
    }
}
