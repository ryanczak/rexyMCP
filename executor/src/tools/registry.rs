// Tool registry: name → executor, plus schema metadata for the LLM.

use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct ToolResult {
    pub output: String,
    pub error: Option<String>,
    pub metadata: Option<Value>,
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    /// JSON schema for tool parameters, in OpenAI function-calling shape.
    fn schema(&self) -> Value;
    async fn execute(&self, args: Value) -> Result<ToolResult>;
}

#[derive(Default, Clone)]
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        self.tools.insert(tool.name().to_string(), tool);
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).cloned()
    }

    pub fn all(&self) -> impl Iterator<Item = &Arc<dyn Tool>> {
        self.tools.values()
    }

    /// Look up a tool by name and run it. An unknown name is an
    /// advisory failure (the model receives `ToolResult { error:
    /// Some(_), output: "", metadata: None }`), not a Rust error —
    /// same pattern as every tool's own input-validation failures.
    pub async fn dispatch(&self, name: &str, args: Value) -> Result<ToolResult> {
        match self.get(name) {
            Some(tool) => tool.execute(args).await,
            None => Ok(ToolResult {
                output: String::new(),
                error: Some(format!("unknown tool: {name}")),
                metadata: None,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct EchoTool;

    #[async_trait]
    impl Tool for EchoTool {
        fn name(&self) -> &str {
            "echo"
        }

        fn description(&self) -> &str {
            "Echo tool for testing"
        }

        fn schema(&self) -> Value {
            json!({ "type": "object", "properties": {} })
        }

        async fn execute(&self, args: Value) -> Result<ToolResult> {
            Ok(ToolResult {
                output: args.to_string(),
                error: None,
                metadata: None,
            })
        }
    }

    #[tokio::test]
    async fn dispatch_of_registered_tool_returns_output() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(EchoTool));

        let result = registry
            .dispatch("echo", json!({ "msg": "hello" }))
            .await
            .unwrap();

        assert!(result.error.is_none());
        assert!(result.output.contains("hello"));
    }

    #[tokio::test]
    async fn dispatch_of_unknown_name_returns_advisory_error() {
        let registry = ToolRegistry::new();
        let result = registry.dispatch("nonexistent", json!({})).await.unwrap();

        assert!(result.error.is_some());
        assert!(
            result
                .error
                .as_ref()
                .unwrap()
                .contains("unknown tool: nonexistent")
        );
        assert!(result.output.is_empty());
    }
}
