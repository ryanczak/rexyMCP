use crate::agent::tasks::Task;
use crate::store::sessions::event::TaskState;
use crate::tools::registry::{Tool, ToolResult};
use anyhow::Result;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

#[derive(Deserialize)]
struct UpdateTaskArgs {
    id: String,
    state: String,
}

pub struct UpdateTask {
    tasks: Mutex<Vec<Task>>,
}

pub fn update_task(tasks: Vec<Task>) -> Arc<dyn Tool> {
    Arc::new(UpdateTask {
        tasks: Mutex::new(tasks),
    })
}

fn advisory(msg: &str) -> ToolResult {
    ToolResult {
        output: String::new(),
        error: Some(msg.to_string()),
        metadata: None,
    }
}

#[async_trait]
impl Tool for UpdateTask {
    fn name(&self) -> &str {
        "update_task"
    }

    fn description(&self) -> &str {
        "Record progress on a tracked task from the phase checklist. Set a task `active` when you start it and `done` when it is complete. `id` is the Spec item number; `state` is one of `active`, `done`, `pending`."
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "Task id (the Spec item number, e.g. \"2\")."
                },
                "state": {
                    "type": "string",
                    "enum": ["active", "done", "pending"],
                    "description": "New state for the task."
                }
            },
            "required": ["id", "state"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let parsed = match serde_json::from_value::<UpdateTaskArgs>(args) {
            Ok(v) => v,
            Err(_) => {
                return Ok(advisory(
                    "update_task: invalid arguments — expected {id, state}",
                ));
            }
        };

        let new_state = match parsed.state.as_str() {
            "pending" => TaskState::Pending,
            "active" => TaskState::Active,
            "done" => TaskState::Done,
            other => {
                return Ok(advisory(&format!(
                    "update_task: invalid state \"{other}\" — expected one of: active, done, pending"
                )));
            }
        };

        let (id, title, state_value) = {
            let mut tasks = self.tasks.lock().unwrap_or_else(|e| e.into_inner());
            let task = match tasks.iter_mut().find(|t| t.id == parsed.id) {
                Some(t) => t,
                None => {
                    return Ok(advisory(&format!(
                        "update_task: no task with id \"{}\"",
                        parsed.id
                    )));
                }
            };
            let title = task.title.clone();
            let id = task.id.clone();
            task.state = new_state;
            let state_value = serde_json::to_value(task.state)?;
            (id, title, state_value)
        };

        Ok(ToolResult {
            output: format!("task {} \"{}\" → {}", id, title, parsed.state),
            error: None,
            metadata: Some(json!({
                "task_update": {
                    "id": id,
                    "title": title,
                    "state": state_value
                }
            })),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Arc;

    fn make_tasks() -> Vec<Task> {
        vec![Task {
            id: "1".to_string(),
            title: "First task".to_string(),
            state: TaskState::Pending,
        }]
    }

    #[tokio::test]
    async fn flips_pending_task_to_active() {
        let tool: Arc<dyn Tool> = update_task(make_tasks());
        let result = tool
            .execute(json!({ "id": "1", "state": "active" }))
            .await
            .unwrap();

        assert!(result.error.is_none());
        let meta = result.metadata.as_ref().unwrap();
        let state = meta["task_update"]["state"].as_str().unwrap();
        assert_eq!(state, "active");
        assert_eq!(meta["task_update"]["id"].as_str().unwrap(), "1");
        assert_eq!(meta["task_update"]["title"].as_str().unwrap(), "First task");
    }

    #[tokio::test]
    async fn flips_active_task_to_done() {
        let mut tasks = make_tasks();
        tasks[0].state = TaskState::Active;
        let tool: Arc<dyn Tool> = update_task(tasks);
        let result = tool
            .execute(json!({ "id": "1", "state": "done" }))
            .await
            .unwrap();

        assert!(result.error.is_none());
        let meta = result.metadata.as_ref().unwrap();
        assert_eq!(meta["task_update"]["state"].as_str().unwrap(), "done");
    }

    #[tokio::test]
    async fn success_output_names_task() {
        let tool: Arc<dyn Tool> = update_task(make_tasks());
        let result = tool
            .execute(json!({ "id": "1", "state": "active" }))
            .await
            .unwrap();

        assert!(result.output.contains("1"));
        assert!(result.output.contains("First task"));
    }

    #[tokio::test]
    async fn unknown_id_returns_advisory_error() {
        let tool: Arc<dyn Tool> = update_task(make_tasks());
        let result = tool
            .execute(json!({ "id": "99", "state": "done" }))
            .await
            .unwrap();

        assert!(result.error.is_some());
        assert!(result.error.as_ref().unwrap().contains("99"));
        assert!(result.metadata.is_none());
    }

    #[tokio::test]
    async fn invalid_state_returns_advisory_error() {
        let tool: Arc<dyn Tool> = update_task(make_tasks());
        let result = tool
            .execute(json!({ "id": "1", "state": "frobnicate" }))
            .await
            .unwrap();

        assert!(result.error.is_some());
        assert!(result.error.as_ref().unwrap().contains("frobnicate"));
    }

    #[tokio::test]
    async fn malformed_args_returns_advisory_error() {
        let tool: Arc<dyn Tool> = update_task(make_tasks());
        let result = tool.execute(json!({ "id": 1 })).await.unwrap();

        assert!(result.error.is_some());
    }
}
