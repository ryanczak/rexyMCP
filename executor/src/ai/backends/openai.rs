use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};
use std::time::Duration;
use tokio::sync::mpsc::UnboundedSender;

use crate::ai::SamplingParams;
use crate::ai::types::{AiEvent, Message, TokenBreakdown, ToolSchema};
use crate::ai::{AiClient, http, send_with_retry, stream_next_with_timeout};

pub(crate) fn parse_openai_usage(u: &serde_json::Map<String, Value>) -> TokenBreakdown {
    let total_prompt = u.get("prompt_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let output_tokens = u
        .get("completion_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;
    let cache_read = u
        .get("prompt_tokens_details")
        .and_then(|d| d.get("cached_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;
    TokenBreakdown {
        input_tokens: total_prompt.saturating_sub(cache_read),
        output_tokens,
        cache_read_tokens: cache_read,
        cache_write_tokens: 0,
    }
}

pub fn convert_messages(messages: Vec<Message>) -> Vec<Value> {
    let mut result = Vec::new();
    for m in messages {
        if let Some(trs) = m.tool_results {
            for tr in trs {
                result.push(json!({
                    "role": "tool",
                    "tool_call_id": tr.tool_call_id,
                    "content": tr.content
                }));
            }
        } else if let Some(tcs) = m.tool_calls {
            let mut tool_calls = Vec::new();
            for tc in tcs {
                tool_calls.push(json!({
                    "id": tc.id,
                    "type": "function",
                    "function": {
                        "name": tc.name,
                        "arguments": tc.arguments
                    }
                }));
            }
            result.push(json!({
                "role": "assistant",
                "content": m.content,
                "tool_calls": tool_calls
            }));
        } else {
            result.push(json!({
                "role": m.role,
                "content": m.content
            }));
        }
    }
    result
}

fn render_openai_tools(tools: &[ToolSchema]) -> Vec<Value> {
    tools
        .iter()
        .map(|t| {
            json!({
                "type": "function",
                "function": {
                    "name": t.name.clone(),
                    "description": t.description.clone(),
                    "parameters": t.parameters.clone(),
                }
            })
        })
        .collect()
}

pub fn build_chat_body(
    model: &str,
    system: &str,
    messages: Vec<Value>,
    tools: Option<&[ToolSchema]>,
    sampling: SamplingParams,
) -> Value {
    let mut combined_system = String::from(system);
    let mut non_system = Vec::with_capacity(messages.len());
    for msg in messages {
        if msg.get("role").and_then(|r| r.as_str()) == Some("system") {
            if let Some(content) = msg.get("content").and_then(|c| c.as_str()) {
                if !combined_system.is_empty() {
                    combined_system.push_str("\n\n");
                }
                combined_system.push_str(content);
            }
        } else {
            non_system.push(msg);
        }
    }
    let mut full_messages = vec![json!({"role": "system", "content": combined_system})];
    let needs_seed = non_system
        .first()
        .and_then(|m| m.get("role").and_then(|r| r.as_str()))
        .map_or(true, |r| r != "user");
    if needs_seed {
        // Some vLLM-served models (e.g. Qwen3) reject payloads that don't open
        // with a user turn after the system message.
        full_messages.push(json!({"role": "user", "content": "Begin."}));
    }
    full_messages.extend(non_system);

    let mut body = json!({
        "model": model,
        "max_tokens": sampling.max_tokens,
        "stream": true,
        "stream_options": { "include_usage": true },
        "messages": full_messages,
    });
    let tool_list = tools.unwrap_or(&[]);
    if !tool_list.is_empty() {
        let rendered = render_openai_tools(tool_list);
        body["tools"] = json!(rendered);
        body["tool_choice"] = json!("auto");
    } else {
        body["tool_choice"] = json!("none");
    }
    if let Some(t) = sampling.temperature {
        body["temperature"] = json!(t);
    }
    if let Some(s) = sampling.seed {
        body["seed"] = json!(s);
    }
    body
}

pub struct OpenAiClient {
    api_key: String,
    model: String,
    base_url: String,
    first_token_timeout: Duration,
    stream_idle_timeout: Duration,
    sampling: SamplingParams,
}

impl OpenAiClient {
    pub fn new(
        api_key: String,
        model: String,
        base_url: String,
        first_token_timeout: Duration,
        stream_idle_timeout: Duration,
        sampling: SamplingParams,
    ) -> Self {
        let resolved_url = if base_url.is_empty() {
            "https://api.openai.com/v1".to_string()
        } else {
            base_url
        };
        OpenAiClient {
            api_key,
            model,
            base_url: resolved_url,
            first_token_timeout,
            stream_idle_timeout,
            sampling,
        }
    }
}

#[async_trait]
impl AiClient for OpenAiClient {
    async fn chat(
        &self,
        system: &str,
        messages: Vec<Message>,
        tx: UnboundedSender<AiEvent>,
        tools: Option<&[ToolSchema]>,
    ) -> Result<()> {
        let converted = convert_messages(messages);
        let body = build_chat_body(&self.model, system, converted, tools, self.sampling);

        let mut first_token_seen = false;
        let mut retries = 0;
        let mut stream_retries = 0;
        const MAX_FIRST_TOKEN_RETRIES: u32 = 2;
        const MAX_STREAM_RETRIES: u32 = 3;

        loop {
            let response = send_with_retry(|| {
                http()
                    .post(format!("{}/chat/completions", self.base_url))
                    .bearer_auth(&self.api_key)
                    .json(&body)
            })
            .await?;

            let mut stream = response.bytes_stream();
            let mut tool_id = String::new();
            let mut tool_name = String::new();
            let mut tool_args = String::new();
            let mut leftover = String::new();
            let mut usage = TokenBreakdown::default();
            let mut in_reasoning = false;
            let mut served_model: Option<String> = None;
            let mut finish_reason: Option<String> = None;

            // Per-attempt buffers (discarded on retry):
            let mut out = String::new();
            let mut buffered_tool_calls: Vec<(String, String, String)> = Vec::new();

            const MAX_LEFTOVER_BYTES: usize = 1 << 20;

            let stall_result = loop {
                let timeout = select_timeout(
                    first_token_seen,
                    self.first_token_timeout,
                    self.stream_idle_timeout,
                );
                match stream_next_with_timeout(&mut stream, timeout).await {
                    Some(Ok(bytes)) => {
                        leftover.push_str(&String::from_utf8_lossy(&bytes));
                        if leftover.len() > MAX_LEFTOVER_BYTES {
                            break Err(anyhow::anyhow!(
                                "SSE stream leftover buffer exceeded {} bytes without a newline; \
                                 aborting to prevent memory exhaustion",
                                MAX_LEFTOVER_BYTES
                            ));
                        }

                        let mut done = false;
                        while let Some(pos) = leftover.find('\n') {
                            let line = leftover[..pos].trim().to_string();
                            leftover = leftover[pos + 1..].to_string();

                            if let Some(data) = line.strip_prefix("data: ") {
                                if data == "[DONE]" {
                                    done = true;
                                    break;
                                }
                                if let Ok(v) = serde_json::from_str::<Value>(data) {
                                    if let Some(delta) =
                                        v["choices"].get(0).and_then(|c| c["delta"].as_object())
                                    {
                                        if !first_token_seen && delta_carries_token(delta) {
                                            first_token_seen = true;
                                        }

                                        let reasoning_chunk = delta
                                            .get("reasoning")
                                            .or_else(|| delta.get("reasoning_content"))
                                            .and_then(|r| r.as_str())
                                            .filter(|r| !r.is_empty());
                                        if let Some(chunk) = reasoning_chunk {
                                            if !in_reasoning {
                                                out.push_str("</think>");
                                                in_reasoning = true;
                                            }
                                            out.push_str(chunk);
                                        }
                                        if let Some(content) =
                                            delta.get("content").and_then(|c| c.as_str())
                                            && !content.is_empty()
                                        {
                                            if in_reasoning {
                                                out.push_str("</think>\n");
                                                in_reasoning = false;
                                            }
                                            out.push_str(content);
                                        }
                                        if let Some(tool_calls) =
                                            delta.get("tool_calls").and_then(|t| t.as_array())
                                            && let Some(tc) = tool_calls.first()
                                        {
                                            if in_reasoning {
                                                out.push_str("</think>\n");
                                                in_reasoning = false;
                                            }
                                            if let Some(id) = tc.get("id").and_then(|i| i.as_str())
                                            {
                                                if !tool_id.is_empty() && tool_id != id {
                                                    buffered_tool_calls.push((
                                                        tool_id.clone(),
                                                        tool_name.clone(),
                                                        tool_args.clone(),
                                                    ));
                                                }
                                                tool_id = id.to_string();
                                                tool_args.clear();
                                            }
                                            if let Some(f) = tc.get("function") {
                                                if let Some(n) =
                                                    f.get("name").and_then(|n| n.as_str())
                                                    && !n.is_empty()
                                                {
                                                    tool_name = n.to_string();
                                                }
                                                if let Some(args) =
                                                    f.get("arguments").and_then(|a| a.as_str())
                                                {
                                                    tool_args.push_str(args);
                                                }
                                            }
                                        }
                                    }
                                    if let Some(u) = v.get("usage").and_then(|u| u.as_object()) {
                                        usage = parse_openai_usage(u);
                                    }
                                    if let Some(m) = v.get("model").and_then(|m| m.as_str()) {
                                        served_model = Some(m.to_string());
                                    }
                                    if let Some(fr) = v["choices"]
                                        .get(0)
                                        .and_then(|c| c.get("finish_reason"))
                                        .and_then(|f| f.as_str())
                                    {
                                        finish_reason = Some(fr.to_string());
                                    }
                                }
                            }
                        }
                        if done {
                            break Ok(());
                        }
                    }
                    Some(Err(e)) => break Err(e),
                    None => break Ok(()),
                }
            };

            match stall_result {
                Ok(()) => {
                    // Flush trailing reasoning close if still open.
                    if in_reasoning {
                        out.push_str("</think>\n");
                    }
                    // Emit consolidated token.
                    let _ = tx.send(AiEvent::Token(out));
                    // Emit buffered tool calls in arrival order.
                    for (id, name, args) in buffered_tool_calls {
                        emit_tool_call_generic(&tx, &id, &name, &args);
                    }
                    // Emit the final (in-progress) tool call if any.
                    if !tool_id.is_empty() {
                        emit_tool_call_generic(&tx, &tool_id, &tool_name, &tool_args);
                    }
                    let _ = tx.send(AiEvent::Completion {
                        finish_reason,
                        model: served_model,
                    });
                    let _ = tx.send(AiEvent::Done(usage));
                    return Ok(());
                }
                Err(e) => {
                    if is_retriable_transport(&e) && stream_retries < MAX_STREAM_RETRIES {
                        stream_retries += 1;
                        tokio::time::sleep(stream_retry_backoff(stream_retries)).await;
                        continue; // per-attempt buffer is discarded; request re-issued
                    }
                    if should_retry_stall(first_token_seen, retries, MAX_FIRST_TOKEN_RETRIES) {
                        retries += 1;
                        continue;
                    }
                    return Err(e);
                }
            }
        }
    }
}

fn emit_tool_call_generic(tx: &UnboundedSender<AiEvent>, id: &str, name: &str, args_raw: &str) {
    if name.is_empty() {
        return;
    }
    let args: Value = serde_json::from_str(args_raw).unwrap_or(Value::Null);
    let _ = tx.send(AiEvent::ToolCallGeneric {
        id: id.to_string(),
        name: name.to_string(),
        args,
        thought_signature: None,
    });
}

/// Select which timeout to use based on whether a token has been seen.
fn select_timeout(
    first_token_seen: bool,
    first_token_timeout: Duration,
    stream_idle_timeout: Duration,
) -> Duration {
    if first_token_seen {
        stream_idle_timeout
    } else {
        first_token_timeout
    }
}

/// Whether a first-token stall should be retried.
fn should_retry_stall(first_token_seen: bool, retries: u32, max_retries: u32) -> bool {
    !first_token_seen && retries < max_retries
}

/// A stream error worth retrying: a transport/body failure (the connection
/// dropped mid-stream), as opposed to a stall timeout or our own runaway-buffer
/// abort, which are synthetic `anyhow` errors that don't downcast.
fn is_retriable_transport(e: &anyhow::Error) -> bool {
    e.downcast_ref::<reqwest::Error>().is_some()
}

/// Bounded exponential backoff for mid-stream retries.
/// Returns 250ms, 500ms, 1s, capped at ~2s.
fn stream_retry_backoff(attempt: u32) -> Duration {
    let ms = (250 * 2u64.pow(attempt.saturating_sub(1))).min(2000);
    Duration::from_millis(ms)
}

/// Whether a delta carries a real token (non-empty content, reasoning, or tool calls).
fn delta_carries_token(delta: &serde_json::Map<String, Value>) -> bool {
    let has_content = delta
        .get("content")
        .and_then(|c| c.as_str())
        .is_some_and(|c| !c.is_empty());
    let has_reasoning = delta
        .get("reasoning")
        .or_else(|| delta.get("reasoning_content"))
        .and_then(|r| r.as_str())
        .is_some_and(|r| !r.is_empty());
    let has_tool_calls = delta
        .get("tool_calls")
        .and_then(|t| t.as_array())
        .is_some_and(|t| !t.is_empty());
    has_content || has_reasoning || has_tool_calls
}

/// Drains a stream with per-item timeout selection and bounded retry on
/// first-token stalls. Generic over item/error so it can be unit-tested
/// without reqwest specifics.
///
/// This is a test harness that exercises the same decision functions
/// (`select_timeout`, `should_retry_stall`) that production `chat()` uses.
/// The production loop lives in `OpenAiClient::chat`; this helper exists
/// so the retry/timeout logic can be tested without reqwest specifics.
///
/// `next_item` produces the next stream item. Returns `Ok` when the stream
/// ends naturally (`None`), or `Err` on a stall / transport error.
///
/// On a stall *before* any token has been seen, retries up to
/// `max_first_token_retries` times by calling `retry_fn` (which should
/// re-issue the request and return a fresh stream via `next_item`). After
/// the cap is exhausted, or if the stall happens after a token was emitted,
/// the error is returned immediately.
#[cfg(test)]
pub(crate) async fn drain_stream_with_retry<S, T, E, F, R>(
    mut next_item: F,
    mut retry_fn: R,
    first_token_timeout: Duration,
    stream_idle_timeout: Duration,
    max_first_token_retries: u32,
) -> Result<(), E>
where
    S: futures_util::Stream<Item = std::result::Result<T, E>> + Unpin,
    F: FnMut() -> S,
    R: FnMut() -> S,
    E: From<anyhow::Error>,
{
    use futures_util::StreamExt;

    let mut first_token_seen = false;
    let mut retries = 0u32;
    let mut stream = next_item();

    loop {
        let timeout = select_timeout(first_token_seen, first_token_timeout, stream_idle_timeout);

        match tokio::time::timeout(timeout, stream.next()).await {
            Ok(Some(Ok(_item))) => {
                if !first_token_seen {
                    first_token_seen = true;
                }
            }
            Ok(Some(Err(e))) => return Err(e),
            Ok(None) => return Ok(()),
            Err(_elapsed) => {
                if should_retry_stall(first_token_seen, retries, max_first_token_retries) {
                    retries += 1;
                    stream = retry_fn();
                    continue;
                }
                return Err(anyhow::anyhow!(
                    "SSE stream stalled — no data received for {}s (server may have dropped the connection)",
                    timeout.as_secs()
                ).into());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::types::{AiEvent, Message, ToolCall, ToolResult, ToolSchema};
    use super::{
        build_chat_body, convert_messages, delta_carries_token, drain_stream_with_retry,
        emit_tool_call_generic, is_retriable_transport, parse_openai_usage, render_openai_tools,
        select_timeout, should_retry_stall, stream_retry_backoff,
    };
    use crate::ai::SamplingParams;
    use futures_util::{StreamExt, stream};
    use serde_json::{Value, json};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Duration;
    use tokio::sync::mpsc;

    #[test]
    fn openai_parses_cached_tokens_from_details() {
        let usage_obj = serde_json::json!({
            "prompt_tokens": 2000,
            "completion_tokens": 500,
            "prompt_tokens_details": {
                "cached_tokens": 800
            }
        })
        .as_object()
        .cloned()
        .unwrap();

        let usage = parse_openai_usage(&usage_obj);

        assert_eq!(usage.input_tokens, 1200);
        assert_eq!(usage.cache_read_tokens, 800);
        assert_eq!(usage.cache_write_tokens, 0);
        assert_eq!(usage.output_tokens, 500);
        assert_eq!(usage.total(), 2500);
    }

    #[test]
    fn openai_parses_zero_cache_when_details_absent() {
        let usage_obj = serde_json::json!({
            "prompt_tokens": 1000,
            "completion_tokens": 300
        })
        .as_object()
        .cloned()
        .unwrap();

        let usage = parse_openai_usage(&usage_obj);

        assert_eq!(usage.input_tokens, 1000);
        assert_eq!(usage.cache_read_tokens, 0);
        assert_eq!(usage.cache_write_tokens, 0);
    }

    #[test]
    fn emit_tool_call_generic_sends_toolcall_generic_event() {
        let (tx, mut rx) = mpsc::unbounded_channel::<AiEvent>();
        emit_tool_call_generic(&tx, "tc_42", "read_file", r#"{"path": "/tmp/x"}"#);

        let event = rx.try_recv().expect("expected an event");
        match event {
            AiEvent::ToolCallGeneric {
                id,
                name,
                args,
                thought_signature,
            } => {
                assert_eq!(id, "tc_42");
                assert_eq!(name, "read_file");
                assert_eq!(args, json!({"path": "/tmp/x"}));
                assert_eq!(thought_signature, None);
            }
            other => panic!("expected ToolCallGeneric, got {:?}", other),
        }
        assert!(rx.try_recv().is_err(), "expected exactly one event");
    }

    #[test]
    fn emit_tool_call_generic_degrades_args_to_null_on_parse_failure() {
        let (tx, mut rx) = mpsc::unbounded_channel::<AiEvent>();
        emit_tool_call_generic(&tx, "tc_1", "bash", "not valid json {");

        let event = rx.try_recv().expect("expected an event");
        match event {
            AiEvent::ToolCallGeneric { args, .. } => {
                assert_eq!(args, Value::Null);
            }
            other => panic!("expected ToolCallGeneric, got {:?}", other),
        }
    }

    #[test]
    fn emit_tool_call_generic_empty_name_emits_nothing() {
        let (tx, rx) = mpsc::unbounded_channel::<AiEvent>();
        emit_tool_call_generic(&tx, "tc_0", "", r#"{}"#);

        assert!(rx.is_empty(), "expected no events for empty name");
    }

    #[test]
    fn render_openai_tools_wraps_in_function_envelope() {
        let tools = vec![ToolSchema {
            name: "foo".to_string(),
            description: "A foo tool".to_string(),
            parameters: json!({ "type": "object", "properties": { "x": { "type": "string" } } }),
        }];
        let rendered = render_openai_tools(&tools);
        assert_eq!(rendered.len(), 1);
        assert_eq!(rendered[0]["type"], "function");
        assert_eq!(rendered[0]["function"]["name"], "foo");
        assert_eq!(rendered[0]["function"]["description"], "A foo tool");
        assert_eq!(
            rendered[0]["function"]["parameters"],
            json!({ "type": "object", "properties": { "x": { "type": "string" } } })
        );
    }

    #[test]
    fn build_chat_body_has_stream_true_and_model() {
        let body = build_chat_body(
            "qwen2.5",
            "system prompt",
            vec![],
            None,
            SamplingParams::default(),
        );
        assert_eq!(body["stream"], true);
        assert_eq!(body["model"], "qwen2.5");
    }

    #[test]
    fn build_chat_body_tool_choice_none_when_no_tools() {
        let body = build_chat_body("m", "sys", vec![], None, SamplingParams::default());
        assert_eq!(body["tool_choice"], "none");

        let body = build_chat_body("m", "sys", vec![], Some(&[]), SamplingParams::default());
        assert_eq!(body["tool_choice"], "none");
    }

    #[test]
    fn build_chat_body_tool_choice_auto_when_tools_present() {
        let tools = vec![ToolSchema {
            name: "foo".into(),
            description: "bar".into(),
            parameters: json!({}),
        }];
        let body = build_chat_body("m", "sys", vec![], Some(&tools), SamplingParams::default());
        assert_eq!(body["tool_choice"], "auto");
        assert!(body.get("tools").is_some());
    }

    #[test]
    fn build_chat_body_includes_temperature_and_seed_when_set() {
        let body = build_chat_body(
            "m",
            "sys",
            vec![],
            None,
            SamplingParams {
                temperature: Some(0.2),
                seed: Some(42),
                max_tokens: 8192,
            },
        );
        assert_eq!(body["temperature"], 0.2);
        assert_eq!(body["seed"], 42);
    }

    #[test]
    fn build_chat_body_omits_sampling_keys_when_none() {
        let body = build_chat_body("m", "sys", vec![], None, SamplingParams::default());
        assert!(body.get("temperature").is_none());
        assert!(body.get("seed").is_none());
    }

    #[test]
    fn build_chat_body_omits_only_unset_key() {
        let body = build_chat_body(
            "m",
            "sys",
            vec![],
            None,
            SamplingParams {
                temperature: Some(0.7),
                seed: None,
                max_tokens: 8192,
            },
        );
        assert_eq!(body["temperature"], 0.7);
        assert!(body.get("seed").is_none());
    }

    #[test]
    fn build_chat_body_uses_configured_max_tokens() {
        let body = build_chat_body("m", "sys", vec![], None, SamplingParams::default());
        assert_eq!(body["max_tokens"], 8192);
    }

    #[test]
    fn build_chat_body_max_tokens_reflects_arg_not_default() {
        let body = build_chat_body(
            "m",
            "sys",
            vec![],
            None,
            SamplingParams {
                max_tokens: 1234,
                ..SamplingParams::default()
            },
        );
        assert_eq!(body["max_tokens"], 1234);
    }

    #[test]
    fn convert_messages_plain_user_message() {
        let msgs = vec![Message {
            role: "user".into(),
            content: "hello".into(),
            tool_calls: None,
            tool_results: None,
            turn: None,
        }];
        let out = convert_messages(msgs);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["role"], "user");
        assert_eq!(out[0]["content"], "hello");
    }

    #[test]
    fn convert_messages_tool_results_become_role_tool() {
        let msgs = vec![Message {
            role: "user".into(),
            content: String::new(),
            tool_calls: None,
            tool_results: Some(vec![ToolResult {
                tool_call_id: "tc_1".into(),
                tool_name: "read_file".into(),
                content: "file contents".into(),
            }]),
            turn: None,
        }];
        let out = convert_messages(msgs);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["role"], "tool");
        assert_eq!(out[0]["tool_call_id"], "tc_1");
        assert_eq!(out[0]["content"], "file contents");
    }

    #[test]
    fn convert_messages_tool_calls_become_role_assistant() {
        let msgs = vec![Message {
            role: "assistant".into(),
            content: "let me help".into(),
            tool_calls: Some(vec![ToolCall {
                id: "tc_2".into(),
                name: "bash".into(),
                arguments: r#"{"cmd":"ls"}"#.into(),
                thought_signature: None,
            }]),
            tool_results: None,
            turn: None,
        }];
        let out = convert_messages(msgs);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["role"], "assistant");
        assert_eq!(out[0]["content"], "let me help");
        let tcs = out[0]["tool_calls"].as_array().unwrap();
        assert_eq!(tcs.len(), 1);
        assert_eq!(tcs[0]["id"], "tc_2");
        assert_eq!(tcs[0]["function"]["name"], "bash");
    }

    #[test]
    fn select_timeout_returns_first_token_budget_before_token_seen() {
        let first = Duration::from_secs(600);
        let idle = Duration::from_secs(90);
        assert_eq!(select_timeout(false, first, idle), first);
    }

    #[test]
    fn select_timeout_returns_idle_budget_after_token_seen() {
        let first = Duration::from_secs(600);
        let idle = Duration::from_secs(90);
        assert_eq!(select_timeout(true, first, idle), idle);
    }

    #[test]
    fn should_retry_stall_returns_true_before_token_seen_under_cap() {
        assert!(should_retry_stall(false, 0, 2));
        assert!(should_retry_stall(false, 1, 2));
    }

    #[test]
    fn should_retry_stall_returns_false_at_cap() {
        assert!(!should_retry_stall(false, 2, 2));
    }

    #[test]
    fn should_retry_stall_returns_false_after_token_seen() {
        assert!(!should_retry_stall(true, 0, 2));
    }

    #[test]
    fn delta_carries_token_with_non_empty_content() {
        let delta = json!({ "content": "hello" }).as_object().cloned().unwrap();
        assert!(delta_carries_token(&delta));
    }

    #[test]
    fn delta_carries_token_with_non_empty_reasoning() {
        let delta = json!({ "reasoning": "let me think" })
            .as_object()
            .cloned()
            .unwrap();
        assert!(delta_carries_token(&delta));
    }

    #[test]
    fn delta_carries_token_with_non_empty_reasoning_content() {
        let delta = json!({ "reasoning_content": "thinking..." })
            .as_object()
            .cloned()
            .unwrap();
        assert!(delta_carries_token(&delta));
    }

    #[test]
    fn delta_carries_token_with_non_empty_tool_calls() {
        let delta = json!({ "tool_calls": [{ "id": "tc_1", "function": { "name": "foo", "arguments": "{}" } }] })
            .as_object()
            .cloned()
            .unwrap();
        assert!(delta_carries_token(&delta));
    }

    #[test]
    fn delta_carries_token_false_on_empty_content() {
        let delta = json!({ "content": "" }).as_object().cloned().unwrap();
        assert!(!delta_carries_token(&delta));
    }

    #[test]
    fn delta_carries_token_false_on_empty_delta() {
        let delta = json!({}).as_object().cloned().unwrap();
        assert!(!delta_carries_token(&delta));
    }

    #[test]
    fn delta_carries_token_false_on_empty_tool_calls_array() {
        let delta = json!({ "tool_calls": [] }).as_object().cloned().unwrap();
        assert!(!delta_carries_token(&delta));
    }

    #[tokio::test]
    async fn first_token_stall_retries_then_succeeds() {
        let call_count = Arc::new(AtomicU32::new(0));
        let call_count_clone = call_count.clone();

        let result = drain_stream_with_retry(
            move || {
                let c = call_count_clone.clone();
                c.fetch_add(1, Ordering::SeqCst);
                if c.load(Ordering::SeqCst) <= 1 {
                    stream::pending::<Result<String, anyhow::Error>>().boxed()
                } else {
                    stream::iter(vec![Ok("token".to_string())]).boxed()
                }
            },
            move || {
                let c = call_count.clone();
                c.fetch_add(1, Ordering::SeqCst);
                stream::iter(vec![Ok("token".to_string())]).boxed()
            },
            Duration::from_secs(1),
            Duration::from_secs(1),
            2,
        )
        .await;

        assert!(result.is_ok(), "should succeed after retry: {result:?}");
    }

    #[tokio::test]
    async fn first_token_stall_exhausts_retries_then_errors() {
        let result = drain_stream_with_retry(
            || stream::pending::<Result<String, anyhow::Error>>().boxed(),
            || stream::pending::<Result<String, anyhow::Error>>().boxed(),
            Duration::from_secs(1),
            Duration::from_secs(1),
            2,
        )
        .await;

        assert!(result.is_err(), "should error after exhausting retries");
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("1s"),
            "error should report the first-token budget: {err_msg}"
        );
    }

    #[tokio::test]
    async fn midstream_stall_is_not_retried() {
        let request_count = Arc::new(AtomicU32::new(0));
        let stall_after_first = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stall_flag = stall_after_first.clone();
        let request_count_check = request_count.clone();

        let result = drain_stream_with_retry(
            move || {
                let c = request_count.clone();
                let s = stall_flag.clone();
                c.fetch_add(1, Ordering::SeqCst);
                let first_call = c.load(Ordering::SeqCst) == 1;
                if first_call {
                    stream::iter(vec![Ok("first_token".to_string())])
                        .chain(stream::pending())
                        .boxed()
                } else {
                    s.store(true, Ordering::SeqCst);
                    stream::pending::<Result<String, anyhow::Error>>().boxed()
                }
            },
            move || stream::pending::<Result<String, anyhow::Error>>().boxed(),
            Duration::from_secs(1),
            Duration::from_secs(1),
            2,
        )
        .await;

        assert!(result.is_err(), "should error on mid-stream stall");
        assert_eq!(
            request_count_check.load(Ordering::SeqCst),
            1,
            "should not have retried after first token was seen"
        );
    }

    #[test]
    fn is_retriable_transport_true_for_reqwest_error() {
        // Construct a reqwest::Error synchronously from an unparseable URL —
        // no network I/O.  .build() fails at request-build time with a
        // reqwest::Error, which is all we need for the downcast check.
        let reqwest_err = reqwest::Client::new().get("not-a-url").build().unwrap_err();
        let wrapped: anyhow::Error = reqwest_err.into();
        assert!(
            is_retriable_transport(&wrapped),
            "reqwest transport error should be retriable"
        );
    }

    #[test]
    fn is_retriable_transport_false_for_synthetic_stall() {
        let err = anyhow::anyhow!("SSE stream stalled — no data received");
        assert!(
            !is_retriable_transport(&err),
            "synthetic stall error should not be retriable"
        );
    }

    #[test]
    fn is_retriable_transport_false_for_runaway_abort() {
        let err = anyhow::anyhow!(
            "SSE stream leftover buffer exceeded 1048576 bytes without a newline; \
             aborting to prevent memory exhaustion"
        );
        assert!(
            !is_retriable_transport(&err),
            "runaway-buffer abort should not be retriable"
        );
    }

    #[test]
    fn stream_retry_backoff_is_bounded_and_increasing() {
        let b1 = stream_retry_backoff(1);
        let b2 = stream_retry_backoff(2);
        let b3 = stream_retry_backoff(3);
        let b10 = stream_retry_backoff(10);

        assert_eq!(b1, Duration::from_millis(250), "attempt 1 = 250ms");
        assert_eq!(b2, Duration::from_millis(500), "attempt 2 = 500ms");
        assert_eq!(b3, Duration::from_millis(1000), "attempt 3 = 1s");
        assert_eq!(b10, Duration::from_millis(2000), "attempt 10 capped at 2s");
        assert!(b1 < b2 && b2 < b3, "backoff should be increasing");
        assert!(
            b3 < b10 || b3 == Duration::from_millis(1000),
            "should grow then cap"
        );
    }
}
