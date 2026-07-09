use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// Returns `(input_per_mtok, output_per_mtok)` in USD/MTok for known Claude
/// model IDs. Used by both `DashboardConfig` and `ArchitectConfig` so the
/// rate table lives in one place.
pub fn known_model_rates(model: &str) -> Option<(f64, f64)> {
    match model {
        "claude-fable-5" | "claude-mythos-5" => Some((10.0, 50.0)),
        "claude-opus-4-8" | "claude-opus-4-7" | "claude-opus-4-6" => Some((5.0, 25.0)),
        "claude-sonnet-4-6" => Some((3.0, 15.0)),
        "claude-haiku-4-5" | "claude-haiku-4-5-20251001" => Some((1.0, 5.0)),
        _ => None,
    }
}

/// Executor capability tier. Set via `rexymcp calibrate` and recorded in
/// `[executor].tier`. Controls default `max_turns` and `gate_retries`
/// (wired M26).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Tier {
    Large,
    Medium,
    Small,
}

impl Tier {
    /// Default `max_turns` for this tier when not explicitly set in `[budget]`.
    pub fn default_max_turns(self) -> u32 {
        match self {
            Tier::Large => 400,
            Tier::Medium => 250,
            Tier::Small => 100,
        }
    }

    /// Default `gate_retries` for this tier when not explicitly set in `[budget]`.
    /// `u32::MAX` means retry until `max_turns` is exhausted (LARGE behavior).
    pub fn default_gate_retries(self) -> u32 {
        match self {
            Tier::Large => u32::MAX,
            Tier::Medium => 2,
            Tier::Small => 1,
        }
    }
}

/// Escalation budget for the architect-side autonomous loop
/// (`/rexymcp:auto`, M27). `max_assists` is the number of autonomous
/// assist round-trips (refine + re-dispatch, or resume) the loop may
/// spend on one phase before stopping for the human. Tier-independent;
/// consumed by the plugin skill layer, never by the executor loop.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct EscalationConfig {
    /// Maximum autonomous mid-phase Architect assists before hard_fail.
    pub max_assists: u32,
}

impl Default for EscalationConfig {
    fn default() -> Self {
        Self { max_assists: 3 }
    }
}

/// The model used for Architect escalation assists. Separate from `[dashboard]`
/// which is the hypothetical cloud baseline — this is a real cost center.
/// When `model` matches a known Claude model ID, rates are auto-filled.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ArchitectConfig {
    /// Claude model ID for Architect assists (e.g. `"claude-opus-4-8"`).
    /// When recognised, auto-fills `input_per_mtok` / `output_per_mtok`.
    pub model: Option<String>,
    /// USD per million input tokens (overridden by `model` when recognised).
    pub input_per_mtok: f64,
    /// USD per million output tokens (overridden by `model` when recognised).
    pub output_per_mtok: f64,
    /// USD per million cache-**read** input tokens (overridden by `model` when
    /// recognised: 0.1× the input rate).
    pub cache_read_per_mtok: f64,
    /// USD per million cache-**creation** input tokens (overridden by `model`
    /// when recognised: 1.25× the input rate).
    pub cache_creation_per_mtok: f64,
}

impl Default for ArchitectConfig {
    fn default() -> Self {
        Self {
            model: None,
            input_per_mtok: 0.0,
            output_per_mtok: 0.0,
            cache_read_per_mtok: 0.0,
            cache_creation_per_mtok: 0.0,
        }
    }
}

impl ArchitectConfig {
    /// Resolved `(input_per_mtok, output_per_mtok)`: model lookup wins when
    /// the model ID is recognised; explicit fields win otherwise.
    pub fn effective_rates(&self) -> (f64, f64) {
        self.model
            .as_deref()
            .and_then(known_model_rates)
            .unwrap_or((self.input_per_mtok, self.output_per_mtok))
    }

    /// Resolved per-class architect rates. When `model` is recognised, cache
    /// rates derive from its input rate (0.1× read, 1.25× creation); otherwise the
    /// explicit `cache_*_per_mtok` fields apply. Reuses `effective_rates` for the
    /// input/output pair.
    pub fn effective_architect_rates(&self) -> crate::store::telemetry::ArchitectRates {
        use crate::store::telemetry::{
            ArchitectRates, CACHE_CREATION_RATE_MULTIPLIER, CACHE_READ_RATE_MULTIPLIER,
        };
        let (input, output) = self.effective_rates();
        let model_known = self.model.as_deref().and_then(known_model_rates).is_some();
        let (cache_read, cache_creation) = if model_known {
            (
                input * CACHE_READ_RATE_MULTIPLIER,
                input * CACHE_CREATION_RATE_MULTIPLIER,
            )
        } else {
            (self.cache_read_per_mtok, self.cache_creation_per_mtok)
        };
        ArchitectRates {
            input_per_mtok: input,
            cache_creation_per_mtok: cache_creation,
            cache_read_per_mtok: cache_read,
            output_per_mtok: output,
        }
    }
}

/// Live-dashboard settings. The "$ saved" baseline: cloud $/million-tokens the
/// local run is priced against. Default 0.0 → the dashboard shows "—" (unset).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DashboardConfig {
    /// USD per million **input** tokens for the cloud baseline (used when
    /// `saved_model` is not set or not recognised).
    pub saved_input_per_mtok: f64,
    /// USD per million **output** tokens for the cloud baseline (used when
    /// `saved_model` is not set or not recognised).
    pub saved_output_per_mtok: f64,
    /// Optional Claude model name; when set and recognised, auto-fills
    /// `saved_input_per_mtok` / `saved_output_per_mtok` with current pricing.
    /// Recognised values: `claude-fable-5`, `claude-mythos-5`,
    /// `claude-opus-4-8`/`4-7`/`4-6`, `claude-sonnet-4-6`, `claude-haiku-4-5`.
    pub saved_model: Option<String>,
}

impl Default for DashboardConfig {
    fn default() -> Self {
        Self {
            saved_input_per_mtok: 0.0,
            saved_output_per_mtok: 0.0,
            saved_model: None,
        }
    }
}

impl DashboardConfig {
    /// Resolved `(input_per_mtok, output_per_mtok)` for the cloud baseline.
    /// `saved_model` lookup wins; explicit fields win otherwise.
    pub fn effective_rates(&self) -> (f64, f64) {
        self.saved_model
            .as_deref()
            .and_then(known_model_rates)
            .unwrap_or((self.saved_input_per_mtok, self.saved_output_per_mtok))
    }
}

/// Context-optimization settings (M10). `output_filter` is the kill-switch for
/// boundary output filtering — default on; set false to restore raw head+tail
/// truncation with no recovery file.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct ContextConfig {
    pub output_filter: bool,
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            output_filter: true,
        }
    }
}

/// Governor hard-fail thresholds. Tune these to match your model's cadence.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct GovernorConfig {
    /// Consecutive identical tool calls before `IdenticalToolCallRepetition`
    /// hard-fail. Default 6.
    pub identical_call_threshold: usize,
    /// Consecutive turns with author-attributed verifier errors before
    /// `VerifierFailurePersistent` hard-fail. Default 6.
    pub verifier_persistence_threshold: usize,
    /// Single tool-output bytes before `RunawayOutput` hard-fail.
    /// Default 102400 (100 KB).
    pub runaway_output_bytes: usize,
    /// Consecutive empty completions before `EmptyCompletionStall` hard-fail.
    /// Default 3.
    pub empty_completion_threshold: usize,
    /// Consecutive byte-identical gate-feedback re-injections before
    /// `StuckGateFeedback` hard-fail. Default 5.
    pub gate_feedback_repeat_threshold: usize,
    /// Sliding window of recent tool calls examined for oscillation. When the
    /// distinct `(tool, arguments)` count in the last `oscillation_window` calls
    /// is in `2..=oscillation_distinct_max`, the loop hard-fails with
    /// `Oscillation`. `0` disables the detector. Default 8.
    pub oscillation_window: usize,
    /// Max distinct calls in the oscillation window still treated as a stuck
    /// cycle. Default 2 (an A,B,A,B alternation).
    pub oscillation_distinct_max: usize,
    /// Sliding window of recent tool outputs summed for the cumulative-output
    /// flood check. `0` disables the detector. Default 6.
    pub output_window: usize,
    /// Total bytes across the last `output_window` tool outputs before
    /// `CumulativeOutputFlood`. Catches multi-call floods each below
    /// `runaway_output_bytes`. Default 262144 (256 KB).
    pub output_window_bytes: usize,
}

impl Default for GovernorConfig {
    fn default() -> Self {
        Self {
            identical_call_threshold: 6,
            verifier_persistence_threshold: 6,
            runaway_output_bytes: 100 * 1024,
            empty_completion_threshold: 3,
            gate_feedback_repeat_threshold: 5,
            oscillation_window: 8,
            oscillation_distinct_max: 2,
            output_window: 6,
            output_window_bytes: 256 * 1024,
        }
    }
}

/// Per-model knob overrides. Each `Some` field replaces the corresponding global
/// `[executor]`/`[governor]` default when this model is the active executor
/// model; each `None` field inherits the global value. Keyed by exact model id
/// in the `[models]` table (e.g. `[models."Qwen/Qwen3.6-27B-FP8"]`).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ModelOverride {
    pub task_tracking: Option<bool>,
    pub temperature: Option<f64>,
    pub seed: Option<u64>,
    pub max_tokens: Option<u32>,
    pub enable_thinking: Option<bool>,
    pub identical_call_threshold: Option<usize>,
    pub verifier_persistence_threshold: Option<usize>,
    pub runaway_output_bytes: Option<usize>,
    pub empty_completion_threshold: Option<usize>,
    pub gate_feedback_repeat_threshold: Option<usize>,
    pub oscillation_window: Option<usize>,
    pub oscillation_distinct_max: Option<usize>,
    pub output_window: Option<usize>,
    pub output_window_bytes: Option<usize>,
}

/// Per-project identity. The `id` UUID is generated by `rexymcp init` and
/// stored in the project's `rexymcp.toml`. Used to scope telemetry records to
/// the project regardless of filesystem path.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ProjectConfig {
    pub id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub project: ProjectConfig,
    pub executor: ExecutorConfig,
    pub commands: CommandConfig,
    pub budget: BudgetConfig,
    pub telemetry: TelemetryConfig,
    pub dashboard: DashboardConfig,
    pub context: ContextConfig,
    pub governor: GovernorConfig,
    pub models: HashMap<String, ModelOverride>,
    #[serde(default)]
    pub escalation: EscalationConfig,
    #[serde(default)]
    pub architect: ArchitectConfig,
}

/// Cross-project telemetry store. `None` disables telemetry emission.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TelemetryConfig {
    pub dir: Option<PathBuf>,
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
    /// Budget (seconds) for the wait before the first token of a completion (prefill).
    #[serde(default = "default_first_token_timeout_secs")]
    pub first_token_timeout_secs: u64,
    /// Budget (seconds) for the gap between tokens once streaming has begun.
    #[serde(default = "default_stream_idle_timeout_secs")]
    pub stream_idle_timeout_secs: u64,
    /// Sampling temperature sent on every chat request. `None` omits the key,
    /// letting the endpoint apply its own default.
    #[serde(default)]
    pub temperature: Option<f64>,
    /// Deterministic sampling seed sent on every chat request. `None` omits it.
    #[serde(default)]
    pub seed: Option<u64>,
    /// Per-response output-token ceiling (`max_tokens`) sent on every chat
    /// request. Carved out of the remaining context window — `prompt + max_tokens`
    /// must fit in the model's context length. The prior hardcoded 4096 truncated
    /// thinking models mid-reasoning before they reached a tool call; 8192 leaves
    /// headroom for a full reasoning block + tool call while keeping a runaway turn
    /// bounded.
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    /// Whether the served model's chat template renders its reasoning block.
    /// Default **false** (thinking off) to stop thinking models from burning the
    /// output budget on `<think>` reasoning before reaching a tool call. When
    /// false, the backend sends `chat_template_kwargs.enable_thinking = false`;
    /// when true, the key is omitted and the endpoint applies its own default.
    #[serde(default = "default_enable_thinking")]
    pub enable_thinking: bool,
    /// Whether the loop seeds a per-session task list from the phase doc's
    /// `## Spec` and emits `TaskUpdate` events as the executor flips state
    /// (M12 Arc A). Default on; set false for a control run with no task
    /// tracking (the seeding emit is byte-for-byte suppressed).
    #[serde(default = "default_task_tracking")]
    pub task_tracking: bool,
    /// Executor capability tier. `None` = no tier configured (behavior unchanged
    /// from pre-M20). Set via `rexymcp calibrate`.
    #[serde(default)]
    pub tier: Option<Tier>,
}

fn default_first_token_timeout_secs() -> u64 {
    600
}

fn default_stream_idle_timeout_secs() -> u64 {
    240
}

fn default_enable_thinking() -> bool {
    false
}

fn default_task_tracking() -> bool {
    true
}

fn default_max_tokens() -> u32 {
    8192
}

impl Default for ExecutorConfig {
    fn default() -> Self {
        Self {
            provider: "openai".into(),
            model: String::new(),
            base_url: "http://localhost:1234/v1".into(),
            api_key: None,
            first_token_timeout_secs: default_first_token_timeout_secs(),
            stream_idle_timeout_secs: default_stream_idle_timeout_secs(),
            temperature: None,
            seed: None,
            max_tokens: default_max_tokens(),
            enable_thinking: default_enable_thinking(),
            task_tracking: default_task_tracking(),
            tier: None,
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
    pub lint_fix: Option<String>,
    pub format_fix: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetConfig {
    /// Model's context-window size in tokens.
    pub context_length: usize,
    /// % of the model's context window the loop may fill before compacting.
    pub max_context_pct: u8,
    /// Hard cap on executor turns in one phase before budget_exceeded.
    pub max_turns: u32,
    /// Max gate-retry loops at completion time before escalation. `None` = derive
    /// from `executor.tier`; if tier is also `None`, unlimited (bounded by
    /// `max_turns`). Set explicitly to override tier default.
    #[serde(default)]
    pub gate_retries: Option<u32>,
    /// Optional wall-clock ceiling in seconds. When > 0, a run whose elapsed
    /// wall-clock time reaches this value terminates as `budget_exceeded` at
    /// the next turn boundary. `0` (the default) disables the ceiling.
    #[serde(default)]
    pub wall_clock_secs: u64,
}

impl Default for BudgetConfig {
    fn default() -> Self {
        Self {
            context_length: 32768,
            max_context_pct: 70,
            max_turns: 200,
            gate_retries: None,
            wall_clock_secs: 0,
        }
    }
}

impl BudgetConfig {
    /// Resolved gate_retries: explicit field wins; falls back to tier default;
    /// falls back to `u32::MAX` (unlimited, bounded by `max_turns`).
    pub fn effective_gate_retries(&self, tier: Option<Tier>) -> u32 {
        self.gate_retries
            .or_else(|| tier.map(|t| t.default_gate_retries()))
            .unwrap_or(u32::MAX)
    }

    /// Resolved max_turns: explicit field wins; falls back to tier default.
    /// `BudgetConfig.max_turns` always has a value (it has no `Option` wrapper)
    /// so this only matters when the TOML omits `max_turns` entirely. Current
    /// configs always set it, but future `/calibrate` writes will omit it and
    /// rely on this resolution.
    pub fn effective_max_turns(&self, tier: Option<Tier>) -> u32 {
        // max_turns is already resolved from TOML (or its Default impl).
        // This helper is the future hook for tier-derived defaults; for now
        // it just returns the stored value.
        let _ = tier; // reserved for M21 when calibrate stops writing max_turns
        self.max_turns
    }
}

fn expand_tilde(path: PathBuf) -> PathBuf {
    let s = path.to_string_lossy();
    if let Some(rest) = s.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(rest);
        }
    } else if s == "~"
        && let Ok(home) = std::env::var("HOME")
    {
        return PathBuf::from(home);
    }
    path
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

        config.telemetry.dir = config.telemetry.dir.map(expand_tilde);

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

    /// Apply the per-model override for `model` (exact-match lookup in
    /// `[models]`) on top of the global `[executor]`/`[governor]` defaults,
    /// mutating `self` so downstream reads see resolved values. A model with no
    /// `[models]` entry leaves every global untouched.
    pub fn resolve_for_model(&mut self, model: &str) {
        let Some(over) = self.models.get(model).cloned() else {
            return;
        };
        if let Some(v) = over.task_tracking {
            self.executor.task_tracking = v;
        }
        if let Some(v) = over.temperature {
            self.executor.temperature = Some(v);
        }
        if let Some(v) = over.seed {
            self.executor.seed = Some(v);
        }
        if let Some(v) = over.max_tokens {
            self.executor.max_tokens = v;
        }
        if let Some(v) = over.enable_thinking {
            self.executor.enable_thinking = v;
        }
        if let Some(v) = over.identical_call_threshold {
            self.governor.identical_call_threshold = v;
        }
        if let Some(v) = over.verifier_persistence_threshold {
            self.governor.verifier_persistence_threshold = v;
        }
        if let Some(v) = over.runaway_output_bytes {
            self.governor.runaway_output_bytes = v;
        }
        if let Some(v) = over.empty_completion_threshold {
            self.governor.empty_completion_threshold = v;
        }
        if let Some(v) = over.gate_feedback_repeat_threshold {
            self.governor.gate_feedback_repeat_threshold = v;
        }
        if let Some(v) = over.oscillation_window {
            self.governor.oscillation_window = v;
        }
        if let Some(v) = over.oscillation_distinct_max {
            self.governor.oscillation_distinct_max = v;
        }
        if let Some(v) = over.output_window {
            self.governor.output_window = v;
        }
        if let Some(v) = over.output_window_bytes {
            self.governor.output_window_bytes = v;
        }
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

    #[test]
    fn telemetry_default_is_none() {
        let cfg = Config::default();
        assert_eq!(cfg.telemetry.dir, None);
    }

    #[test]
    fn telemetry_absent_section_is_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"[executor]
provider = "openai"
model = "m"
base_url = "http://localhost:1234/v1"

[commands]

[budget]
context_length = 32768
max_context_pct = 70
max_turns = 40
"#,
        )
        .unwrap();

        let cfg = Config::load(&path).unwrap();
        assert_eq!(cfg.telemetry.dir, None);
    }

    #[test]
    fn telemetry_explicit_dir_is_some() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"[executor]
provider = "openai"
model = "m"
base_url = "http://localhost:1234/v1"

[commands]

[budget]
context_length = 32768
max_context_pct = 70
max_turns = 40

[telemetry]
dir = "/var/lib/rexymcp"
"#,
        )
        .unwrap();

        let cfg = Config::load(&path).unwrap();
        assert_eq!(cfg.telemetry.dir, Some(PathBuf::from("/var/lib/rexymcp")));
    }

    #[test]
    fn telemetry_tilde_dir_expands_to_home() {
        let home = std::env::var("HOME").expect("HOME must be set");
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"[executor]
provider = "openai"
model = "m"
base_url = "http://localhost:1234/v1"

[budget]
context_length = 32768
max_context_pct = 70
max_turns = 40

[telemetry]
dir = "~/.rexymcp/telemetry"
"#,
        )
        .unwrap();

        let cfg = Config::load(&path).unwrap();
        let expected = PathBuf::from(&home).join(".rexymcp/telemetry");
        assert_eq!(cfg.telemetry.dir, Some(expected));
    }

    #[test]
    fn config_defaults_first_token_and_idle_timeouts() {
        let cfg = ExecutorConfig::default();
        assert_eq!(cfg.first_token_timeout_secs, 600);
        assert_eq!(cfg.stream_idle_timeout_secs, 240);
    }

    #[test]
    fn config_loads_overridden_timeouts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"[executor]
provider = "openai"
model = "m"
base_url = "http://localhost:1234/v1"
first_token_timeout_secs = 300
stream_idle_timeout_secs = 45

[commands]

[budget]
context_length = 32768
max_context_pct = 70
max_turns = 40
"#,
        )
        .unwrap();

        let cfg = Config::load(&path).unwrap();
        assert_eq!(cfg.executor.first_token_timeout_secs, 300);
        assert_eq!(cfg.executor.stream_idle_timeout_secs, 45);
    }

    #[test]
    fn config_omits_timeouts_keeps_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"[executor]
provider = "openai"
model = "m"
base_url = "http://localhost:1234/v1"

[commands]

[budget]
context_length = 32768
max_context_pct = 70
max_turns = 40
"#,
        )
        .unwrap();

        let cfg = Config::load(&path).unwrap();
        assert_eq!(cfg.executor.first_token_timeout_secs, 600);
        assert_eq!(cfg.executor.stream_idle_timeout_secs, 240);
    }

    #[test]
    fn config_defaults_sampling_settings_to_none() {
        let cfg = ExecutorConfig::default();
        assert_eq!(cfg.temperature, None);
        assert_eq!(cfg.seed, None);
    }

    #[test]
    fn config_loads_sampling_settings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"[executor]
provider = "openai"
model = "m"
base_url = "http://localhost:1234/v1"
temperature = 0.2
seed = 42

[commands]

[budget]
context_length = 32768
max_context_pct = 70
max_turns = 40
"#,
        )
        .unwrap();

        let cfg = Config::load(&path).unwrap();
        assert_eq!(cfg.executor.temperature, Some(0.2));
        assert_eq!(cfg.executor.seed, Some(42));
    }

    #[test]
    fn config_omits_sampling_settings_keeps_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"[executor]
provider = "openai"
model = "m"
base_url = "http://localhost:1234/v1"

[commands]

[budget]
context_length = 32768
max_context_pct = 70
max_turns = 40
"#,
        )
        .unwrap();

        let cfg = Config::load(&path).unwrap();
        assert_eq!(cfg.executor.temperature, None);
        assert_eq!(cfg.executor.seed, None);
    }

    #[test]
    fn dashboard_config_defaults_to_zero() {
        let cfg = Config::default();
        assert_eq!(cfg.dashboard.saved_input_per_mtok, 0.0);
        assert_eq!(cfg.dashboard.saved_output_per_mtok, 0.0);
    }

    #[test]
    fn config_loads_dashboard_rates() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"[executor]
provider = "openai"
model = "m"
base_url = "http://localhost:1234/v1"

[commands]

[budget]
context_length = 32768
max_context_pct = 70
max_turns = 40

[dashboard]
saved_input_per_mtok = 3.0
saved_output_per_mtok = 15.0
"#,
        )
        .unwrap();

        let cfg = Config::load(&path).unwrap();
        assert_eq!(cfg.dashboard.saved_input_per_mtok, 3.0);
        assert_eq!(cfg.dashboard.saved_output_per_mtok, 15.0);
    }

    #[test]
    fn lint_fix_field_defaults_to_none_when_absent() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("rexymcp.toml");
        std::fs::write(
            &path,
            r#"
[executor]
provider = "openai"
model = "m"
base_url = "http://localhost:1234/v1"

[commands]
format = "cargo fmt"
"#,
        )
        .unwrap();

        let cfg = Config::load(&path).unwrap();
        assert_eq!(
            cfg.commands.lint_fix, None,
            "lint_fix must default to None when absent from [commands]"
        );
        assert_eq!(cfg.commands.format.as_deref(), Some("cargo fmt"));
    }

    #[test]
    fn format_fix_field_defaults_to_none_when_absent() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("rexymcp.toml");
        std::fs::write(
            &path,
            r#"
[executor]
provider = "openai"
model = "m"
base_url = "http://localhost:1234/v1"

[commands]
format = "cargo fmt"
"#,
        )
        .unwrap();

        let cfg = Config::load(&path).unwrap();
        assert_eq!(
            cfg.commands.format_fix, None,
            "format_fix must default to None when absent from [commands]"
        );
        assert_eq!(cfg.commands.format.as_deref(), Some("cargo fmt"));
    }

    #[test]
    fn context_config_defaults_output_filter_on() {
        let cfg = ContextConfig::default();
        assert!(cfg.output_filter, "output_filter should default to true");

        // A Config parsed from TOML with no [context] section should also default to true
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"[executor]
provider = "openai"
model = "m"
base_url = "http://localhost:1234/v1"

[commands]

[budget]
context_length = 32768
max_context_pct = 70
max_turns = 40
"#,
        )
        .unwrap();
        let cfg = Config::load(&path).unwrap();
        assert!(
            cfg.context.output_filter,
            "context.output_filter should default to true when [context] section is absent"
        );
    }

    #[test]
    fn context_output_filter_can_be_disabled() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"[executor]
provider = "openai"
model = "m"
base_url = "http://localhost:1234/v1"

[commands]

[budget]
context_length = 32768
max_context_pct = 70
max_turns = 40

[context]
output_filter = false
"#,
        )
        .unwrap();
        let cfg = Config::load(&path).unwrap();
        assert!(
            !cfg.context.output_filter,
            "output_filter should be false when explicitly set"
        );
    }

    #[test]
    fn governor_config_round_trips_from_toml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"[executor]
provider = "openai"
model = "m"
base_url = "http://localhost:1234/v1"

[commands]

[budget]
context_length = 32768
max_context_pct = 70
max_turns = 40

[governor]
identical_call_threshold = 10
verifier_persistence_threshold = 8
runaway_output_bytes = 204800
"#,
        )
        .unwrap();
        let cfg = Config::load(&path).unwrap();
        assert_eq!(cfg.governor.identical_call_threshold, 10);
        assert_eq!(cfg.governor.verifier_persistence_threshold, 8);
        assert_eq!(cfg.governor.runaway_output_bytes, 204800);
    }

    #[test]
    fn executor_task_tracking_defaults_on() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"[executor]
provider = "openai"
model = "m"
base_url = "http://localhost:1234/v1"

[commands]

[budget]
context_length = 32768
max_context_pct = 70
max_turns = 40
"#,
        )
        .unwrap();
        let cfg = Config::load(&path).unwrap();
        assert!(
            cfg.executor.task_tracking,
            "executor.task_tracking should default to true when absent"
        );
    }

    #[test]
    fn executor_task_tracking_can_be_disabled() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"[executor]
provider = "openai"
model = "m"
base_url = "http://localhost:1234/v1"
task_tracking = false

[commands]

[budget]
context_length = 32768
max_context_pct = 70
max_turns = 40
"#,
        )
        .unwrap();
        let cfg = Config::load(&path).unwrap();
        assert!(
            !cfg.executor.task_tracking,
            "executor.task_tracking should be false when explicitly set"
        );
    }

    #[test]
    fn model_override_section_parses_from_toml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"[executor]
provider = "openai"
model = "m"
base_url = "http://localhost:1234/v1"

[commands]

[budget]
context_length = 32768
max_context_pct = 70
max_turns = 40

[models."Qwen/Qwen3.6-27B-FP8"]
temperature = 0.2
task_tracking = false
"#,
        )
        .unwrap();

        let cfg = Config::load(&path).unwrap();
        assert!(
            cfg.models.contains_key("Qwen/Qwen3.6-27B-FP8"),
            "models map must contain the key from the TOML section"
        );
        let over = cfg.models.get("Qwen/Qwen3.6-27B-FP8").unwrap();
        assert_eq!(over.temperature, Some(0.2));
        assert_eq!(over.task_tracking, Some(false));
    }

    #[test]
    fn models_section_absent_is_empty_map() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"[executor]
provider = "openai"
model = "m"
base_url = "http://localhost:1234/v1"

[commands]

[budget]
context_length = 32768
max_context_pct = 70
max_turns = 40
"#,
        )
        .unwrap();

        let cfg = Config::load(&path).unwrap();
        assert!(
            cfg.models.is_empty(),
            "models map must be empty when no [models] section is present"
        );
    }

    #[test]
    fn resolve_for_model_applies_matching_override() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"[executor]
provider = "openai"
model = "m"
base_url = "http://localhost:1234/v1"
task_tracking = true
temperature = 0.8
seed = 1

[commands]

[budget]
context_length = 32768
max_context_pct = 70
max_turns = 40

[governor]
identical_call_threshold = 6
verifier_persistence_threshold = 6
runaway_output_bytes = 102400

[models."Qwen/Qwen3.6-27B-FP8"]
task_tracking = false
temperature = 0.2
seed = 7
identical_call_threshold = 8
"#,
        )
        .unwrap();

        let mut cfg = Config::load(&path).unwrap();
        cfg.resolve_for_model("Qwen/Qwen3.6-27B-FP8");

        assert!(!cfg.executor.task_tracking);
        assert_eq!(cfg.executor.temperature, Some(0.2));
        assert_eq!(cfg.executor.seed, Some(7));
        assert_eq!(cfg.governor.identical_call_threshold, 8);
    }

    #[test]
    fn resolve_for_model_leaves_unset_fields_global() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"[executor]
provider = "openai"
model = "m"
base_url = "http://localhost:1234/v1"
task_tracking = true
seed = 99

[commands]

[budget]
context_length = 32768
max_context_pct = 70
max_turns = 40

[governor]
identical_call_threshold = 6
verifier_persistence_threshold = 6
runaway_output_bytes = 102400

[models."test-model"]
temperature = 0.1
"#,
        )
        .unwrap();

        let mut cfg = Config::load(&path).unwrap();
        cfg.resolve_for_model("test-model");

        // temperature was overridden
        assert_eq!(cfg.executor.temperature, Some(0.1));
        // all others remain at their global values
        assert!(cfg.executor.task_tracking);
        assert_eq!(cfg.executor.seed, Some(99));
        assert_eq!(cfg.governor.identical_call_threshold, 6);
        assert_eq!(cfg.governor.verifier_persistence_threshold, 6);
        assert_eq!(cfg.governor.runaway_output_bytes, 102400);
    }

    #[test]
    fn resolve_for_model_unknown_model_is_noop() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"[executor]
provider = "openai"
model = "m"
base_url = "http://localhost:1234/v1"
task_tracking = true
temperature = 0.5

[commands]

[budget]
context_length = 32768
max_context_pct = 70
max_turns = 40

[governor]
identical_call_threshold = 6
verifier_persistence_threshold = 6
runaway_output_bytes = 102400

[models."some-other-model"]
task_tracking = false
temperature = 0.1
"#,
        )
        .unwrap();

        let mut cfg = Config::load(&path).unwrap();
        cfg.resolve_for_model("unknown-model");

        assert!(cfg.executor.task_tracking);
        assert_eq!(cfg.executor.temperature, Some(0.5));
        assert_eq!(cfg.governor.identical_call_threshold, 6);
        assert_eq!(cfg.governor.verifier_persistence_threshold, 6);
        assert_eq!(cfg.governor.runaway_output_bytes, 102400);
    }

    #[test]
    fn resolve_for_model_is_exact_match() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"[executor]
provider = "openai"
model = "m"
base_url = "http://localhost:1234/v1"
task_tracking = true

[commands]

[budget]
context_length = 32768
max_context_pct = 70
max_turns = 40

[models."qwen"]
task_tracking = false
"#,
        )
        .unwrap();

        let mut cfg = Config::load(&path).unwrap();
        cfg.resolve_for_model("qwen2.5-coder");

        assert!(
            cfg.executor.task_tracking,
            "prefix 'qwen' must not match model 'qwen2.5-coder'"
        );
    }

    #[test]
    fn known_model_rates_returns_opus_rates() {
        let (i, o) = known_model_rates("claude-opus-4-8").expect("opus must be known");
        assert_eq!(i, 5.0);
        assert_eq!(o, 25.0);
    }

    #[test]
    fn known_model_rates_returns_none_for_unknown() {
        assert!(known_model_rates("some-local-llm").is_none());
    }

    #[test]
    fn tier_default_max_turns_correct() {
        assert_eq!(Tier::Large.default_max_turns(), 400);
        assert_eq!(Tier::Medium.default_max_turns(), 250);
        assert_eq!(Tier::Small.default_max_turns(), 100);
    }

    #[test]
    fn tier_default_gate_retries_correct() {
        assert_eq!(Tier::Large.default_gate_retries(), u32::MAX);
        assert_eq!(Tier::Medium.default_gate_retries(), 2);
        assert_eq!(Tier::Small.default_gate_retries(), 1);
    }

    #[test]
    fn budget_effective_gate_retries_explicit_wins() {
        let b = BudgetConfig {
            gate_retries: Some(5),
            ..BudgetConfig::default()
        };
        assert_eq!(b.effective_gate_retries(Some(Tier::Small)), 5);
    }

    #[test]
    fn budget_effective_gate_retries_falls_back_to_tier() {
        let b = BudgetConfig {
            gate_retries: None,
            ..BudgetConfig::default()
        };
        assert_eq!(b.effective_gate_retries(Some(Tier::Medium)), 2);
    }

    #[test]
    fn budget_effective_gate_retries_unlimited_when_no_tier() {
        let b = BudgetConfig {
            gate_retries: None,
            ..BudgetConfig::default()
        };
        assert_eq!(b.effective_gate_retries(None), u32::MAX);
    }

    #[test]
    fn budget_default_wall_clock_secs_is_zero() {
        assert_eq!(BudgetConfig::default().wall_clock_secs, 0);
    }

    #[test]
    fn budget_parses_wall_clock_secs_from_toml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.toml");
        std::fs::write(
            &path,
            r#"
[executor]
provider = "openai"
model = "m"
base_url = "http://localhost:1234/v1"

[budget]
context_length = 32768
max_context_pct = 70
max_turns = 200
wall_clock_secs = 30
"#,
        )
        .unwrap();
        let cfg = Config::load(&path).unwrap();
        assert_eq!(cfg.budget.wall_clock_secs, 30);

        let path2 = dir.path().join("c2.toml");
        std::fs::write(
            &path2,
            r#"
[executor]
provider = "openai"
model = "m"
base_url = "http://localhost:1234/v1"

[budget]
context_length = 32768
max_context_pct = 70
max_turns = 200
"#,
        )
        .unwrap();
        let cfg2 = Config::load(&path2).unwrap();
        assert_eq!(cfg2.budget.wall_clock_secs, 0);
    }

    #[test]
    fn architect_effective_rates_uses_known_model() {
        let a = ArchitectConfig {
            model: Some("claude-opus-4-8".into()),
            input_per_mtok: 0.0,
            output_per_mtok: 0.0,
            cache_read_per_mtok: 0.0,
            cache_creation_per_mtok: 0.0,
        };
        assert_eq!(a.effective_rates(), (5.0, 25.0));
    }

    #[test]
    fn architect_effective_rates_falls_back_to_explicit() {
        let a = ArchitectConfig {
            model: Some("unknown-model".into()),
            input_per_mtok: 2.5,
            output_per_mtok: 12.5,
            cache_read_per_mtok: 0.0,
            cache_creation_per_mtok: 0.0,
        };
        assert_eq!(a.effective_rates(), (2.5, 12.5));
    }

    #[test]
    fn config_parses_tier_from_toml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.toml");
        std::fs::write(
            &path,
            r#"
[executor]
provider = "openai"
model = "m"
base_url = "http://localhost:1234/v1"
tier = "MEDIUM"

[budget]
context_length = 32768
max_context_pct = 70
max_turns = 40
"#,
        )
        .unwrap();
        let cfg = Config::load(&path).unwrap();
        assert_eq!(cfg.executor.tier, Some(Tier::Medium));
    }

    #[test]
    fn config_tier_absent_is_none() {
        let cfg = Config::default();
        assert_eq!(cfg.executor.tier, None);
    }

    #[test]
    fn config_parses_escalation_and_architect_sections() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.toml");
        std::fs::write(
            &path,
            r#"
[executor]
provider = "openai"
model = "m"
base_url = "http://localhost:1234/v1"

[budget]
context_length = 32768
max_context_pct = 70
max_turns = 40

[escalation]
max_assists = 5

[architect]
model = "claude-opus-4-8"
"#,
        )
        .unwrap();
        let cfg = Config::load(&path).unwrap();
        assert_eq!(cfg.escalation.max_assists, 5);
        assert_eq!(cfg.architect.model.as_deref(), Some("claude-opus-4-8"));
        assert_eq!(cfg.architect.effective_rates(), (5.0, 25.0));
    }

    #[test]
    fn config_escalation_absent_uses_default() {
        let cfg = Config::default();
        assert_eq!(cfg.escalation.max_assists, 3);
    }

    #[test]
    fn dashboard_effective_rates_uses_known_model() {
        let d = DashboardConfig {
            saved_model: Some("claude-sonnet-4-6".into()),
            ..DashboardConfig::default()
        };
        assert_eq!(d.effective_rates(), (3.0, 15.0));
    }

    #[test]
    fn loads_default_max_tokens_when_absent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"[executor]
provider = "openai"
model = "m"
base_url = "http://localhost:1234/v1"

[commands]

[budget]
context_length = 32768
max_context_pct = 70
max_turns = 40

[governor]
identical_call_threshold = 6
verifier_persistence_threshold = 6
runaway_output_bytes = 102400
"#,
        )
        .unwrap();

        let cfg = Config::load(&path).unwrap();
        assert_eq!(cfg.executor.max_tokens, 8192);
    }

    #[test]
    fn loads_max_tokens_from_toml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"[executor]
provider = "openai"
model = "m"
base_url = "http://localhost:1234/v1"
max_tokens = 2048

[commands]

[budget]
context_length = 32768
max_context_pct = 70
max_turns = 40

[governor]
identical_call_threshold = 6
verifier_persistence_threshold = 6
runaway_output_bytes = 102400
"#,
        )
        .unwrap();

        let cfg = Config::load(&path).unwrap();
        assert_eq!(cfg.executor.max_tokens, 2048);
    }

    #[test]
    fn resolve_for_model_applies_max_tokens_override() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"[executor]
provider = "openai"
model = "m"
base_url = "http://localhost:1234/v1"
max_tokens = 8192

[commands]

[budget]
context_length = 32768
max_context_pct = 70
max_turns = 40

[governor]
identical_call_threshold = 6
verifier_persistence_threshold = 6
runaway_output_bytes = 102400

[models."m"]
max_tokens = 2048
"#,
        )
        .unwrap();

        let mut cfg = Config::load(&path).unwrap();
        cfg.resolve_for_model("m");
        assert_eq!(cfg.executor.max_tokens, 2048);
    }

    #[test]
    fn resolve_for_model_leaves_max_tokens_when_override_absent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"[executor]
provider = "openai"
model = "m"
base_url = "http://localhost:1234/v1"
max_tokens = 8192

[commands]

[budget]
context_length = 32768
max_context_pct = 70
max_turns = 40

[governor]
identical_call_threshold = 6
verifier_persistence_threshold = 6
runaway_output_bytes = 102400

[models."m"]
temperature = 0.1
"#,
        )
        .unwrap();

        let mut cfg = Config::load(&path).unwrap();
        cfg.resolve_for_model("m");
        assert_eq!(cfg.executor.max_tokens, 8192);
    }

    #[test]
    fn enable_thinking_defaults_false() {
        assert!(!ExecutorConfig::default().enable_thinking);
    }

    #[test]
    fn loads_enable_thinking_from_toml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"[executor]
provider = "openai"
model = "m"
base_url = "http://localhost:1234/v1"
enable_thinking = true

[commands]

[budget]
context_length = 32768
max_context_pct = 70
max_turns = 40

[governor]
identical_call_threshold = 6
verifier_persistence_threshold = 6
runaway_output_bytes = 102400
"#,
        )
        .unwrap();

        let cfg = Config::load(&path).unwrap();
        assert!(cfg.executor.enable_thinking);
    }

    #[test]
    fn enable_thinking_absent_keeps_default_false() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"[executor]
provider = "openai"
model = "m"
base_url = "http://localhost:1234/v1"

[commands]

[budget]
context_length = 32768
max_context_pct = 70
max_turns = 40

[governor]
identical_call_threshold = 6
verifier_persistence_threshold = 6
runaway_output_bytes = 102400
"#,
        )
        .unwrap();

        let cfg = Config::load(&path).unwrap();
        assert!(!cfg.executor.enable_thinking);
    }

    #[test]
    fn resolve_for_model_applies_enable_thinking_override() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"[executor]
provider = "openai"
model = "m"
base_url = "http://localhost:1234/v1"
enable_thinking = false

[commands]

[budget]
context_length = 32768
max_context_pct = 70
max_turns = 40

[governor]
identical_call_threshold = 6
verifier_persistence_threshold = 6
runaway_output_bytes = 102400

[models."m"]
enable_thinking = true
"#,
        )
        .unwrap();

        let mut cfg = Config::load(&path).unwrap();
        cfg.resolve_for_model("m");
        assert!(cfg.executor.enable_thinking);
    }

    #[test]
    fn resolve_for_model_leaves_enable_thinking_when_override_absent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"[executor]
provider = "openai"
model = "m"
base_url = "http://localhost:1234/v1"
enable_thinking = true

[commands]

[budget]
context_length = 32768
max_context_pct = 70
max_turns = 40

[governor]
identical_call_threshold = 6
verifier_persistence_threshold = 6
runaway_output_bytes = 102400

[models."m"]
temperature = 0.1
"#,
        )
        .unwrap();

        let mut cfg = Config::load(&path).unwrap();
        cfg.resolve_for_model("m");
        assert!(cfg.executor.enable_thinking);
    }

    #[test]
    fn load_ignores_retired_escalation_slots_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rexymcp.toml");
        std::fs::write(
            &path,
            r#"
[executor]
provider = "openai"
model = "m"
base_url = "http://localhost:1234/v1"

[budget]
context_length = 32768
max_context_pct = 70
max_turns = 100
escalation_slots = 1

[governor]
identical_call_threshold = 6
verifier_persistence_threshold = 6
runaway_output_bytes = 102400
"#,
        )
        .unwrap();

        let cfg = Config::load(&path).unwrap();
        assert_eq!(cfg.budget.max_turns, 100);
    }

    #[test]
    fn effective_architect_rates_derives_cache_from_known_model() {
        let cfg = ArchitectConfig {
            model: Some("claude-opus-4-8".to_string()),
            input_per_mtok: 0.0,
            output_per_mtok: 0.0,
            cache_read_per_mtok: 0.0,
            cache_creation_per_mtok: 0.0,
        };
        let rates = cfg.effective_architect_rates();
        assert_eq!(rates.input_per_mtok, 5.0);
        assert_eq!(rates.output_per_mtok, 25.0);
        assert_eq!(rates.cache_read_per_mtok, 0.5);
        assert_eq!(rates.cache_creation_per_mtok, 6.25);
    }

    #[test]
    fn effective_architect_rates_uses_explicit_when_model_unknown() {
        let cfg = ArchitectConfig {
            model: Some("unknown-model".to_string()),
            input_per_mtok: 8.0,
            output_per_mtok: 40.0,
            cache_read_per_mtok: 2.0,
            cache_creation_per_mtok: 9.0,
        };
        let rates = cfg.effective_architect_rates();
        assert_eq!(rates.input_per_mtok, 8.0);
        assert_eq!(rates.output_per_mtok, 40.0);
        assert_eq!(rates.cache_read_per_mtok, 2.0);
        assert_eq!(rates.cache_creation_per_mtok, 9.0);
    }
}
