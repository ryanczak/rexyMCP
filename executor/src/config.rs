use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::error::{Error, Result};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    pub executor: ExecutorConfig,
    pub commands: CommandConfig,
    pub budget: BudgetConfig,
}

/// The local LLM the executor drives.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutorConfig {
    /// "openai" | "ollama" | "lmstudio" — all OpenAI-compatible.
    pub provider: String,
    /// Model identifier as the endpoint knows it (e.g. "qwen2.5-coder").
    pub model: String,
    /// OpenAI-compatible base URL, e.g. "http://localhost:1234/v1".
    pub base_url: String,
    /// Optional API key; local endpoints usually ignore it.
    pub api_key: Option<String>,
}

impl Default for ExecutorConfig {
    fn default() -> Self {
        Self {
            provider: "openai".into(),
            model: String::new(),
            base_url: "http://localhost:1234/v1".into(),
            api_key: None,
        }
    }
}

/// Resolves the {FORMAT,BUILD,LINT,TEST}_COMMAND placeholders for the
/// target project the executor works in.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CommandConfig {
    pub format: Option<String>,
    pub build: Option<String>,
    pub lint: Option<String>,
    pub test: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetConfig {
    /// Model's context-window size in tokens.
    pub context_length: usize,
    /// % of the model's context window the loop may fill before compacting.
    pub max_context_pct: u8,
    /// Hard cap on executor turns in one phase before budget_exceeded.
    pub max_turns: u32,
    /// Escalation slots (briefings returned to the architect) per phase.
    pub escalation_slots: u32,
}

impl Default for BudgetConfig {
    fn default() -> Self {
        Self {
            context_length: 32768,
            max_context_pct: 70,
            max_turns: 40,
            escalation_slots: 1,
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let mut config = Config::default();

        if path.exists() {
            let content = std::fs::read_to_string(path)?;
            let loaded: Config =
                toml::from_str(&content).map_err(|e| Error::Config(e.to_string()))?;
            config = loaded;
        }

        Ok(config)
    }

    pub fn apply_overrides(&mut self, get: impl Fn(&str) -> Option<String>) {
        if let Some(v) = get("REXYMCP_PROVIDER") {
            self.executor.provider = v;
        }
        if let Some(v) = get("REXYMCP_MODEL") {
            self.executor.model = v;
        }
        if let Some(v) = get("REXYMCP_BASE_URL") {
            self.executor.base_url = v;
        }
        if let Some(v) = get("REXYMCP_API_KEY") {
            self.executor.api_key = Some(v);
        }
    }

    pub fn apply_env(&mut self) -> Result<()> {
        self.apply_overrides(|k| std::env::var(k).ok());
        Ok(())
    }

    pub fn load_with_env(path: &Path) -> Result<Self> {
        let mut config = Self::load(path)?;
        config.apply_env()?;
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn default_config_targets_local_lmstudio() {
        let cfg = Config::default();
        assert_eq!(cfg.executor.provider, "openai");
        assert_eq!(cfg.executor.base_url, "http://localhost:1234/v1");
        assert_eq!(cfg.budget.max_context_pct, 70);
    }

    #[test]
    fn load_missing_file_returns_default() {
        let cfg = Config::load(Path::new("/nonexistent/path.toml")).unwrap();
        let defaults = Config::default();
        assert_eq!(cfg.executor.provider, defaults.executor.provider);
        assert_eq!(cfg.executor.model, defaults.executor.model);
        assert_eq!(cfg.executor.base_url, defaults.executor.base_url);
        assert_eq!(cfg.budget.max_context_pct, defaults.budget.max_context_pct);
    }

    #[test]
    fn load_parses_toml_executor_block() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(
            f,
            r#"[executor]
provider = "ollama"
model = "qwen2.5-coder"
base_url = "http://localhost:11434/v1"

[commands]

[budget]
context_length = 128000
max_context_pct = 80
max_turns = 50
escalation_slots = 2
"#
        )
        .unwrap();
        drop(f);

        let cfg = Config::load(&path).unwrap();
        assert_eq!(cfg.executor.provider, "ollama");
        assert_eq!(cfg.executor.model, "qwen2.5-coder");
        assert_eq!(cfg.executor.base_url, "http://localhost:11434/v1");
        assert_eq!(cfg.budget.context_length, 128000);
        assert_eq!(cfg.budget.max_context_pct, 80);
        assert_eq!(cfg.budget.max_turns, 50);
        assert_eq!(cfg.budget.escalation_slots, 2);
    }

    #[test]
    fn load_malformed_toml_is_config_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.toml");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, "this is not valid toml {{{{").unwrap();
        drop(f);

        let result = Config::load(&path);
        assert!(result.is_err());
        match result.unwrap_err() {
            Error::Config(_) => {}
            other => panic!("expected Error::Config, got {other:?}"),
        }
    }

    #[test]
    fn env_override_beats_toml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(
            f,
            r#"[executor]
provider = "openai"
model = "model-a"
base_url = "http://localhost:1234/v1"

[commands]

[budget]
context_length = 32768
max_context_pct = 70
max_turns = 40
escalation_slots = 1
"#
        )
        .unwrap();
        drop(f);

        let mut cfg = Config::load(&path).unwrap();
        cfg.apply_overrides(|k| {
            if k == "REXYMCP_MODEL" {
                Some("model-b".into())
            } else {
                None
            }
        });

        assert_eq!(cfg.executor.model, "model-b");
    }
}
