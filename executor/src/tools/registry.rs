// Tool registry: name → executor, plus schema metadata for the LLM.

use crate::tools::router::{self, Category};
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

    /// Return the distinct categories present among registered tools,
    /// in stable enum-declaration order (Read, Write, Search, Run).
    pub fn categories(&self) -> Vec<Category> {
        let present: std::collections::HashSet<Category> = self
            .tools
            .keys()
            .filter_map(|name| router::categorize(name))
            .collect();

        [
            Category::Read,
            Category::Write,
            Category::Search,
            Category::Run,
        ]
        .into_iter()
        .filter(|c| present.contains(c))
        .collect()
    }

    /// Return the registered tools in the given category, sorted by
    /// tool name. Returns an empty `Vec` if no tools are in the category.
    pub fn tools_in(&self, category: Category) -> Vec<Arc<dyn Tool>> {
        let mut tools: Vec<Arc<dyn Tool>> = self
            .tools
            .values()
            .filter(|tool| router::categorize(tool.name()) == Some(category))
            .cloned()
            .collect();
        tools.sort_by_key(|tool| tool.name().to_string());
        tools
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

    struct NamedTool(&'static str);

    #[async_trait]
    impl Tool for NamedTool {
        fn name(&self) -> &str {
            self.0
        }

        fn description(&self) -> &str {
            "test tool"
        }

        fn schema(&self) -> Value {
            json!({ "type": "object", "properties": {} })
        }

        async fn execute(&self, _args: Value) -> Result<ToolResult> {
            Ok(ToolResult {
                output: String::new(),
                error: None,
                metadata: None,
            })
        }
    }

    fn tool(name: &'static str) -> Arc<dyn Tool> {
        Arc::new(NamedTool(name))
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

    #[test]
    fn categories_returns_stable_enum_order_with_one_per_category() {
        let mut registry = ToolRegistry::new();
        registry.register(tool("bash"));
        registry.register(tool("search"));
        registry.register(tool("write_file"));
        registry.register(tool("read_file"));

        let cats = registry.categories();
        assert_eq!(
            cats,
            vec![
                Category::Read,
                Category::Write,
                Category::Search,
                Category::Run
            ]
        );
    }

    #[test]
    fn categories_returns_only_present_categories() {
        let mut registry = ToolRegistry::new();
        registry.register(tool("read_file"));
        registry.register(tool("symbols"));

        let cats = registry.categories();
        assert_eq!(cats, vec![Category::Read]);
    }

    #[test]
    fn tools_in_returns_name_sorted_tools_for_category() {
        let mut registry = ToolRegistry::new();
        registry.register(tool("symbols"));
        registry.register(tool("read_file"));

        let tools = registry.tools_in(Category::Read);
        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert_eq!(names, vec!["read_file", "symbols"]);
    }

    #[test]
    fn tools_in_returns_empty_for_category_with_no_tools() {
        let registry = ToolRegistry::new();
        let tools = registry.tools_in(Category::Write);
        assert!(tools.is_empty());
    }

    #[test]
    fn registration_order_does_not_affect_categories_output() {
        let mut reg_a = ToolRegistry::new();
        reg_a.register(tool("read_file"));
        reg_a.register(tool("bash"));
        reg_a.register(tool("search"));

        let mut reg_b = ToolRegistry::new();
        reg_b.register(tool("search"));
        reg_b.register(tool("bash"));
        reg_b.register(tool("read_file"));

        assert_eq!(reg_a.categories(), reg_b.categories());
    }

    #[test]
    fn registration_order_does_not_affect_tools_in_output() {
        let mut reg_a = ToolRegistry::new();
        reg_a.register(tool("symbols"));
        reg_a.register(tool("read_file"));

        let mut reg_b = ToolRegistry::new();
        reg_b.register(tool("read_file"));
        reg_b.register(tool("symbols"));

        let tools_a = reg_a.tools_in(Category::Read);
        let tools_b = reg_b.tools_in(Category::Read);
        let names_a: Vec<&str> = tools_a.iter().map(|t| t.name()).collect();
        let names_b: Vec<&str> = tools_b.iter().map(|t| t.name()).collect();
        assert_eq!(names_a, names_b);
    }
}
