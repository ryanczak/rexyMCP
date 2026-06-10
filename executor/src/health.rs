use schemars::JsonSchema;
use serde::Serialize;

use crate::ai::{http, send_with_retry};
use crate::config::ExecutorConfig;
use crate::error::{Error, Result};

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct Health {
    pub reachable: bool,
    pub base_url: String,
    pub models: Vec<String>,
}

fn build_models_url(base_url: &str) -> String {
    let base = base_url.trim_end_matches('/');
    format!("{base}/models")
}

fn parse_models_list(body: &str) -> Result<Vec<String>> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|e| Error::Backend(format!("parse error: {e}")))?;

    let data = value
        .get("data")
        .and_then(|d| d.as_array())
        .ok_or_else(|| Error::Backend("response missing 'data' array".into()))?;

    Ok(data
        .iter()
        .filter_map(|entry| entry.get("id").and_then(|id| id.as_str()).map(String::from))
        .collect())
}

/// Find the `max_model_len` for `model_id` in a `/v1/models` response body.
/// `None` if the body is unparseable, the model is absent, or the entry has no
/// `max_model_len`.
pub fn parse_model_max_len(body: &str, model_id: &str) -> Option<usize> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    let data = value.get("data")?.as_array()?;
    data.iter()
        .find(|e| e.get("id").and_then(|i| i.as_str()) == Some(model_id))
        .and_then(|e| e.get("max_model_len"))
        .and_then(|v| v.as_u64())
        .map(|n| n as usize)
}

/// Best-effort: fetch `/v1/models` and return `model`'s context window. Any error
/// (network, parse, missing field) yields `None` — never fails.
pub async fn fetch_context_window(cfg: &ExecutorConfig, model: &str) -> Option<usize> {
    let url = build_models_url(&cfg.base_url);
    let api_key = cfg.api_key.clone();
    let resp = send_with_retry(move || {
        let mut req = http().get(&url);
        if let Some(ref key) = api_key {
            req = req.bearer_auth(key);
        }
        req
    })
    .await
    .ok()?;
    let body = resp.text().await.ok()?;
    parse_model_max_len(&body, model)
}

pub async fn list_models(cfg: &ExecutorConfig) -> Result<Vec<String>> {
    let url = build_models_url(&cfg.base_url);
    let api_key = cfg.api_key.clone();

    let resp = send_with_retry({
        let url = url.clone();
        let api_key = api_key.clone();
        move || {
            let mut req = http().get(&url);
            if let Some(ref key) = api_key {
                req = req.bearer_auth(key);
            }
            req
        }
    })
    .await
    .map_err(|e| Error::Backend(e.to_string()))?;

    let body = resp
        .text()
        .await
        .map_err(|e| Error::Backend(format!("failed to read response body: {e}")))?;

    parse_models_list(&body)
}

pub async fn check(cfg: &ExecutorConfig) -> Health {
    match list_models(cfg).await {
        Ok(models) => Health {
            reachable: true,
            base_url: cfg.base_url.clone(),
            models,
        },
        Err(_) => Health {
            reachable: false,
            base_url: cfg.base_url.clone(),
            models: Vec::new(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_model_ids_from_openai_list_response() {
        let fixture = r#"{"object":"list","data":[{"id":"qwen2.5-coder"},{"id":"gemma2"}]}"#;
        let models = parse_models_list(fixture).unwrap();
        assert_eq!(models, vec!["qwen2.5-coder", "gemma2"]);
    }

    #[test]
    fn parse_rejects_non_list_body() {
        let body = r#"{"error":"something went wrong"}"#;
        let result = parse_models_list(body);
        assert!(result.is_err());
        match result.unwrap_err() {
            Error::Backend(_) => {}
            other => panic!("expected Error::Backend, got {other:?}"),
        }
    }

    #[test]
    fn joins_base_url_trailing_slash() {
        assert_eq!(
            build_models_url("http://localhost:1234/v1/"),
            "http://localhost:1234/v1/models"
        );
    }

    #[test]
    fn joins_base_url_no_trailing_slash() {
        assert_eq!(
            build_models_url("http://localhost:1234/v1"),
            "http://localhost:1234/v1/models"
        );
    }

    #[tokio::test]
    async fn check_returns_unreachable_on_connection_error() {
        let cfg = ExecutorConfig {
            provider: "openai".into(),
            model: "test".into(),
            base_url: "http://127.0.0.1:1".into(),
            api_key: None,
            first_token_timeout_secs: 600,
            stream_idle_timeout_secs: 90,
            temperature: None,
            seed: None,
            task_tracking: true,
        };
        let health = check(&cfg).await;
        assert!(!health.reachable);
        assert_eq!(health.base_url, "http://127.0.0.1:1");
        assert!(health.models.is_empty());
    }

    #[test]
    fn parse_model_max_len_finds_matching_model() {
        let fixture = r#"{"object":"list","data":[{"id":"qwen2.5-coder","max_model_len":32768},{"id":"gemma2","max_model_len":262144}]}"#;
        assert_eq!(parse_model_max_len(fixture, "gemma2"), Some(262144));
        assert_eq!(parse_model_max_len(fixture, "qwen2.5-coder"), Some(32768));
    }

    #[test]
    fn parse_model_max_len_none_for_absent_model() {
        let fixture = r#"{"object":"list","data":[{"id":"gemma2","max_model_len":262144}]}"#;
        assert_eq!(parse_model_max_len(fixture, "qwen2.5-coder"), None);
    }

    #[test]
    fn parse_model_max_len_none_when_field_missing() {
        let fixture = r#"{"object":"list","data":[{"id":"gemma2"}]}"#;
        assert_eq!(parse_model_max_len(fixture, "gemma2"), None);
    }
}
