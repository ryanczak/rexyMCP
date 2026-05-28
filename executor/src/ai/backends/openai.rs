use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedSender;

use super::super::types::{AiEvent, Message, TokenBreakdown, ToolSchema};
use super::super::{AiClient, http, send_with_retry, stream_next_with_timeout};

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

pub struct OpenAiClient {
    api_key: String,
    model: String,
    base_url: String,
}

impl OpenAiClient {
    pub fn new(api_key: String, model: String, base_url: String) -> Self {
        let resolved_url = if base_url.is_empty() {
            "https://api.openai.com/v1".to_string()
        } else {
            base_url
        };
        OpenAiClient {
            api_key,
            model,
            base_url: resolved_url,
        }
    }

    fn convert_messages(&self, messages: Vec<Message>) -> Vec<Value> {
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

#[async_trait]
impl AiClient for OpenAiClient {
    async fn chat(
        &self,
        system: &str,
        messages: Vec<Message>,
        tx: UnboundedSender<AiEvent>,
        tools: Option<&[ToolSchema]>,
    ) -> Result<()> {
        let converted = self.convert_messages(messages);

        let mut combined_system = String::from(system);
        let mut non_system = Vec::with_capacity(converted.len());
        for msg in converted {
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
        full_messages.extend(non_system);

        let mut body = json!({
            "model": self.model.clone(),
            "max_tokens": 4096,
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

        const MAX_LEFTOVER_BYTES: usize = 1 << 20;

        'outer: while let Some(result) = stream_next_with_timeout(&mut stream).await {
            let bytes = result?;
            leftover.push_str(&String::from_utf8_lossy(&bytes));
            if leftover.len() > MAX_LEFTOVER_BYTES {
                return Err(anyhow::anyhow!(
                    "SSE stream leftover buffer exceeded {} bytes without a newline; \
                     aborting to prevent memory exhaustion",
                    MAX_LEFTOVER_BYTES
                ));
            }

            while let Some(pos) = leftover.find('\n') {
                let line = leftover[..pos].trim().to_string();
                leftover = leftover[pos + 1..].to_string();

                if let Some(data) = line.strip_prefix("data: ") {
                    if data == "[DONE]" {
                        break 'outer;
                    }
                    if let Ok(v) = serde_json::from_str::<Value>(data) {
                        if let Some(delta) =
                            v["choices"].get(0).and_then(|c| c["delta"].as_object())
                        {
                            let reasoning_chunk = delta
                                .get("reasoning")
                                .or_else(|| delta.get("reasoning_content"))
                                .and_then(|r| r.as_str())
                                .filter(|r| !r.is_empty());
                            if let Some(chunk) = reasoning_chunk {
                                if !in_reasoning {
                                    let _ = tx.send(AiEvent::Token("</think>".to_string()));
                                    in_reasoning = true;
                                }
                                let _ = tx.send(AiEvent::Token(chunk.to_string()));
                            }
                            if let Some(content) = delta.get("content").and_then(|c| c.as_str())
                                && !content.is_empty()
                            {
                                if in_reasoning {
                                    let _ = tx.send(AiEvent::Token("</think>\n".to_string()));
                                    in_reasoning = false;
                                }
                                let _ = tx.send(AiEvent::Token(content.to_string()));
                            }
                            if let Some(tool_calls) =
                                delta.get("tool_calls").and_then(|t| t.as_array())
                                && let Some(tc) = tool_calls.first()
                            {
                                if in_reasoning {
                                    let _ = tx.send(AiEvent::Token("</think>\n".to_string()));
                                    in_reasoning = false;
                                }
                                if let Some(id) = tc.get("id").and_then(|i| i.as_str()) {
                                    if !tool_id.is_empty() && tool_id != id {
                                        emit_tool_call_generic(
                                            &tx, &tool_id, &tool_name, &tool_args,
                                        );
                                    }
                                    tool_id = id.to_string();
                                    tool_args.clear();
                                }
                                if let Some(f) = tc.get("function") {
                                    if let Some(n) = f.get("name").and_then(|n| n.as_str())
                                        && !n.is_empty()
                                    {
                                        tool_name = n.to_string();
                                    }
                                    if let Some(args) = f.get("arguments").and_then(|a| a.as_str())
                                    {
                                        tool_args.push_str(args);
                                    }
                                }
                            }
                        }
                        if let Some(u) = v.get("usage").and_then(|u| u.as_object()) {
                            usage = parse_openai_usage(u);
                        }
                    }
                }
            }
        }

        if in_reasoning {
            let _ = tx.send(AiEvent::Token("</think>\n".to_string()));
        }

        if !tool_id.is_empty() {
            emit_tool_call_generic(&tx, &tool_id, &tool_name, &tool_args);
        }

        let _ = tx.send(AiEvent::Done(usage));
        Ok(())
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

#[cfg(test)]
mod tests {
    use super::super::super::types::{AiEvent, ToolSchema};
    use super::{emit_tool_call_generic, parse_openai_usage, render_openai_tools};
    use serde_json::{Value, json};
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
}
