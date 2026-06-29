pub mod backends;
pub mod testing;
pub mod types;

use anyhow::Result;
use async_trait::async_trait;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::mpsc::UnboundedSender;

pub use types::{AiEvent, Message, TokenBreakdown, ToolResult, ToolSchema};

pub use backends::openai::OpenAiClient;

use crate::config::ExecutorConfig;

static TOOL_CALL_ID: AtomicU64 = AtomicU64::new(1);
static HTTP_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

// ── Circuit breaker ──────────────────────────────────────────────────────

const CB_FAILURE_THRESHOLD: u32 = 5;
const CB_COOLDOWN: Duration = Duration::from_secs(60);

struct CircuitBreaker {
    consecutive_failures: AtomicU32,
    open_until: std::sync::Mutex<Option<Instant>>,
}

impl CircuitBreaker {
    fn new() -> Self {
        Self {
            consecutive_failures: AtomicU32::new(0),
            open_until: std::sync::Mutex::new(None),
        }
    }

    #[cfg(test)]
    fn state_str(&self) -> &'static str {
        let open_until = *self.open_until.lock().unwrap_or_else(|e| e.into_inner());
        match open_until {
            None => "closed",
            Some(t) if t > Instant::now() => "open",
            Some(_) => "half-open",
        }
    }

    fn allow(&self) -> bool {
        let open_until = *self.open_until.lock().unwrap_or_else(|e| e.into_inner());
        match open_until {
            None => true,
            Some(t) => Instant::now() >= t,
        }
    }

    fn record_success(&self) {
        self.consecutive_failures.store(0, Ordering::Relaxed);
        let _prev = self
            .open_until
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
    }

    fn record_failure(&self) {
        let failures = self.consecutive_failures.fetch_add(1, Ordering::Relaxed) + 1;
        if failures >= CB_FAILURE_THRESHOLD {
            let cooldown_until = Instant::now() + CB_COOLDOWN;
            *self.open_until.lock().unwrap_or_else(|e| e.into_inner()) = Some(cooldown_until);
        }
    }
}

static CIRCUIT_BREAKER: OnceLock<CircuitBreaker> = OnceLock::new();

fn circuit() -> &'static CircuitBreaker {
    CIRCUIT_BREAKER.get_or_init(CircuitBreaker::new)
}

#[async_trait]
pub trait AiClient: Send + Sync {
    /// Stream a chat completion.
    ///
    /// `tools = None` for pure-text analysis calls where the model must not
    /// emit tool/function calls. `tools = Some(&[])` is equivalent to `None`
    /// (backends should treat both identically).
    ///
    /// `tools = Some(slice)` sends the rendered tool schemas to the provider.
    async fn chat(
        &self,
        system_prompt: &str,
        messages: Vec<Message>,
        tx: UnboundedSender<AiEvent>,
        tools: Option<&[ToolSchema]>,
    ) -> Result<()>;
}

pub fn http() -> &'static reqwest::Client {
    HTTP_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .unwrap()
    })
}

pub fn next_tool_id() -> String {
    format!("tc_{}", TOOL_CALL_ID.fetch_add(1, Ordering::Relaxed))
}

async fn send_with_retry_inner(
    make_req: impl Fn() -> reqwest::RequestBuilder,
) -> Result<reqwest::Response> {
    let mut retries = 0;
    loop {
        let req = make_req();
        match req.send().await {
            Ok(resp) => {
                let status = resp.status();
                if status.is_success() {
                    return Ok(resp);
                }
                if status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
                    if retries >= 2 {
                        let bytes = resp.bytes().await.unwrap_or_default();
                        let text = String::from_utf8_lossy(&bytes);
                        anyhow::bail!("API error {}: {}", status, text);
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(2 << retries)).await;
                    retries += 1;
                    continue;
                }
                let bytes = resp.bytes().await.unwrap_or_default();
                let text = String::from_utf8_lossy(&bytes);
                anyhow::bail!("API error {}: {}", status, text);
            }
            Err(e) => {
                if retries >= 2 {
                    anyhow::bail!("Request failed: {}", e);
                }
                tokio::time::sleep(std::time::Duration::from_secs(2 << retries)).await;
                retries += 1;
            }
        }
    }
}

pub async fn send_with_retry(
    make_req: impl Fn() -> reqwest::RequestBuilder,
) -> Result<reqwest::Response> {
    if !circuit().allow() {
        anyhow::bail!(
            "AI backend circuit breaker is open — too many recent failures. \
             Retry in ~{}s.",
            CB_COOLDOWN.as_secs()
        );
    }
    match send_with_retry_inner(make_req).await {
        Ok(resp) => {
            circuit().record_success();
            Ok(resp)
        }
        Err(e) => {
            circuit().record_failure();
            Err(e)
        }
    }
}

pub async fn stream_next_with_timeout<B>(
    stream: &mut (impl futures_util::Stream<Item = Result<B, reqwest::Error>> + Unpin),
    timeout: Duration,
) -> Option<Result<B, anyhow::Error>> {
    use futures_util::StreamExt;
    match tokio::time::timeout(timeout, stream.next()).await {
        Ok(Some(Ok(bytes))) => Some(Ok(bytes)),
        Ok(Some(Err(e))) => Some(Err(e.into())),
        Ok(None) => None,
        Err(_elapsed) => Some(Err(anyhow::anyhow!(
            "SSE stream stalled — no data received for {}s (server may have dropped the connection)",
            timeout.as_secs()
        ))),
    }
}

/// Sampling knobs forwarded verbatim to every chat request.
#[derive(Debug, Clone, Copy)]
pub struct SamplingParams {
    pub temperature: Option<f64>,
    pub seed: Option<u64>,
    pub max_tokens: u32,
    pub enable_thinking: bool,
}

impl Default for SamplingParams {
    fn default() -> Self {
        Self {
            temperature: None,
            seed: None,
            max_tokens: 8192,
            enable_thinking: false,
        }
    }
}

pub fn make_client(cfg: &ExecutorConfig) -> Box<dyn AiClient> {
    Box::new(OpenAiClient::new(
        cfg.api_key.clone().unwrap_or_default(),
        cfg.model.clone(),
        cfg.base_url.clone(),
        Duration::from_secs(cfg.first_token_timeout_secs),
        Duration::from_secs(cfg.stream_idle_timeout_secs),
        SamplingParams {
            temperature: cfg.temperature,
            seed: cfg.seed,
            max_tokens: cfg.max_tokens,
            enable_thinking: cfg.enable_thinking,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::stream;

    #[test]
    fn circuit_breaker_closed_initially() {
        let cb = CircuitBreaker::new();
        assert_eq!(cb.state_str(), "closed");
        assert!(cb.allow());
    }

    #[test]
    fn circuit_breaker_opens_after_threshold() {
        let cb = CircuitBreaker::new();
        for _ in 0..CB_FAILURE_THRESHOLD {
            assert!(cb.allow(), "should still be allowed before threshold");
            cb.record_failure();
        }
        assert_eq!(cb.state_str(), "open");
        assert!(!cb.allow());
    }

    #[test]
    fn circuit_breaker_closes_on_success() {
        let cb = CircuitBreaker::new();
        for _ in 0..CB_FAILURE_THRESHOLD {
            cb.record_failure();
        }
        assert_eq!(cb.state_str(), "open");
        {
            let mut guard = cb.open_until.lock().unwrap();
            *guard = Some(Instant::now() - Duration::from_secs(1));
        }
        assert_eq!(cb.state_str(), "half-open");
        assert!(cb.allow());
        cb.record_success();
        assert_eq!(cb.state_str(), "closed");
        assert!(cb.allow());
    }

    #[test]
    fn make_client_openai() {
        let cfg = ExecutorConfig {
            provider: "openai".into(),
            model: "gpt-4o".into(),
            base_url: String::new(),
            api_key: Some("key".into()),
            first_token_timeout_secs: 600,
            stream_idle_timeout_secs: 90,
            temperature: None,
            seed: None,
            max_tokens: 8192,
            enable_thinking: false,
            task_tracking: true,
            tier: None,
        };
        let _c = make_client(&cfg);
    }

    #[test]
    fn make_client_ollama() {
        let cfg = ExecutorConfig {
            provider: "ollama".into(),
            model: "llama3.2".into(),
            base_url: "http://localhost:11434/v1".into(),
            api_key: Some("local".into()),
            first_token_timeout_secs: 600,
            stream_idle_timeout_secs: 90,
            temperature: None,
            seed: None,
            max_tokens: 8192,
            enable_thinking: false,
            task_tracking: true,
            tier: None,
        };
        let _c = make_client(&cfg);
    }

    #[test]
    fn make_client_lmstudio() {
        let cfg = ExecutorConfig {
            provider: "lmstudio".into(),
            model: "some-model".into(),
            base_url: "http://localhost:1234/v1".into(),
            api_key: Some("local".into()),
            first_token_timeout_secs: 600,
            stream_idle_timeout_secs: 90,
            temperature: None,
            seed: None,
            max_tokens: 8192,
            enable_thinking: false,
            task_tracking: true,
            tier: None,
        };
        let _c = make_client(&cfg);
    }

    #[tokio::test]
    async fn stream_next_uses_supplied_timeout() {
        let timeout = Duration::from_secs(1);
        let mut pending: futures_util::stream::Pending<Result<Vec<u8>, reqwest::Error>> =
            stream::pending();

        let result = stream_next_with_timeout(&mut pending, timeout).await;
        assert!(result.is_some(), "should return Some on timeout");
        let err_msg = result.unwrap().unwrap_err().to_string();
        assert!(
            err_msg.contains("1s"),
            "error should report the actual budget: {err_msg}"
        );
    }

    #[test]
    fn sampling_params_default_max_tokens_is_8192() {
        assert_eq!(SamplingParams::default().max_tokens, 8192);
    }

    #[test]
    fn sampling_params_default_enable_thinking_is_false() {
        assert!(!SamplingParams::default().enable_thinking);
    }
}
