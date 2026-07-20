use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
    #[serde(skip)]
    pub thought_signature: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub tool_call_id: String,
    pub tool_name: String,
    pub content: String,
}

/// Provider-neutral description of a tool the model may call.
/// Backends render this into their provider-specific request shape
/// in `AiClient::chat`.
#[derive(Debug, Clone)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_results: Option<Vec<ToolResult>>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub turn: Option<usize>,
}

/// Four-bucket token breakdown for a single AI call.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenBreakdown {
    #[serde(default)]
    pub input_tokens: u32,
    #[serde(default)]
    pub output_tokens: u32,
    #[serde(default)]
    pub cache_read_tokens: u32,
    #[serde(default)]
    pub cache_write_tokens: u32,
}

impl TokenBreakdown {
    pub fn total(&self) -> u32 {
        self.input_tokens
            .saturating_add(self.output_tokens)
            .saturating_add(self.cache_read_tokens)
            .saturating_add(self.cache_write_tokens)
    }

    pub fn uncached_input_tokens(&self) -> u32 {
        self.input_tokens
    }
}

#[derive(Debug)]
pub enum AiEvent {
    Token(String),
    ToolCallGeneric {
        id: String,
        name: String,
        args: serde_json::Value,
        thought_signature: Option<String>,
    },
    Done(TokenBreakdown),
    /// Per-completion provenance, emitted once just before `Done`. Either field may be `None`.
    Completion {
        finish_reason: Option<String>,
        model: Option<String>,
    },
    Error(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user_msg(content: &str) -> Message {
        Message {
            role: "user".to_string(),
            content: content.to_string(),
            tool_calls: None,
            tool_results: None,
            turn: None,
        }
    }

    #[test]
    fn message_roundtrip_plain() {
        let msg = user_msg("test content");
        let json = serde_json::to_string(&msg).unwrap();
        let back: Message = serde_json::from_str(&json).unwrap();
        assert_eq!(back.role, "user");
        assert_eq!(back.content, "test content");
        assert!(back.tool_calls.is_none());
        assert!(back.tool_results.is_none());
    }

    #[test]
    fn message_tool_calls_skipped_when_none() {
        let msg = user_msg("hi");
        let json = serde_json::to_string(&msg).unwrap();
        assert!(!json.contains("tool_calls"));
        assert!(!json.contains("tool_results"));
    }

    #[test]
    fn tool_call_roundtrip() {
        let tc = ToolCall {
            id: "tc_99".to_string(),
            name: "run_terminal_command".to_string(),
            arguments: r#"{"command":"echo hi","background":true}"#.to_string(),
            thought_signature: None,
        };
        let json = serde_json::to_string(&tc).unwrap();
        let back: ToolCall = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, "tc_99");
        assert_eq!(back.name, "run_terminal_command");
    }

    #[test]
    fn token_breakdown_total_sums_all_buckets() {
        let tb = TokenBreakdown {
            input_tokens: 1000,
            output_tokens: 500,
            cache_read_tokens: 800,
            cache_write_tokens: 200,
        };
        assert_eq!(tb.total(), 2500);
    }

    #[test]
    fn token_breakdown_zero_tokens_is_zero() {
        let tb = TokenBreakdown::default();
        assert_eq!(tb.total(), 0);
        assert_eq!(tb.uncached_input_tokens(), 0);
    }

    #[test]
    fn token_breakdown_uncached_input_tokens_returns_input_field() {
        let tb = TokenBreakdown {
            input_tokens: 300,
            output_tokens: 100,
            cache_read_tokens: 500,
            cache_write_tokens: 100,
        };
        assert_eq!(tb.uncached_input_tokens(), 300);
    }

    #[test]
    fn token_breakdown_serializes_all_fields() {
        let tb = TokenBreakdown {
            input_tokens: 10,
            output_tokens: 20,
            cache_read_tokens: 30,
            cache_write_tokens: 40,
        };
        let json = serde_json::to_string(&tb).unwrap();
        assert!(json.contains("\"input_tokens\":10"));
        assert!(json.contains("\"output_tokens\":20"));
        assert!(json.contains("\"cache_read_tokens\":30"));
        assert!(json.contains("\"cache_write_tokens\":40"));
    }

    #[test]
    fn legacy_usage_shape_no_longer_mapped() {
        let legacy_json = r#"{"prompt_tokens":1500,"completion_tokens":800}"#;
        let tb: TokenBreakdown = serde_json::from_str(legacy_json).unwrap();
        assert_eq!(tb.input_tokens, 0);
        assert_eq!(tb.output_tokens, 0);
        assert_eq!(tb.cache_read_tokens, 0);
        assert_eq!(tb.cache_write_tokens, 0);
    }

    #[test]
    fn token_breakdown_current_shape_roundtrips() {
        let json = r#"{"input_tokens":200,"output_tokens":100,"cache_read_tokens":500,"cache_write_tokens":50}"#;
        let tb: TokenBreakdown = serde_json::from_str(json).unwrap();
        assert_eq!(tb.input_tokens, 200);
        assert_eq!(tb.output_tokens, 100);
        assert_eq!(tb.cache_read_tokens, 500);
        assert_eq!(tb.cache_write_tokens, 50);
    }

    #[test]
    fn token_breakdown_zero_cache_when_provider_omits_field() {
        let partial_json = r#"{"input_tokens":100,"output_tokens":50}"#;
        let tb: TokenBreakdown = serde_json::from_str(partial_json).unwrap();
        assert_eq!(tb.cache_read_tokens, 0);
        assert_eq!(tb.cache_write_tokens, 0);
        assert_eq!(tb.input_tokens, 100);
        assert_eq!(tb.output_tokens, 50);
    }

    #[test]
    fn tool_schema_roundtrip() {
        let schema = ToolSchema {
            name: "test_tool".to_string(),
            description: "A test tool".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string" },
                    "limit": { "type": "integer" }
                },
                "required": ["query"]
            }),
        };
        let cloned = schema.clone();
        assert_eq!(schema.name, cloned.name);
        assert_eq!(schema.description, cloned.description);
        assert_eq!(schema.parameters, cloned.parameters);
        let debug_str = format!("{:?}", schema);
        assert!(debug_str.contains("test_tool"));
    }
}
