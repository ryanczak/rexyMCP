// bash: run a shell command with a timeout, cwd-pinned to the scope root and
// env-stripped to a safe allowlist. Commands are classified before execution;
// dangerous shapes are refused outright.
//
// Unlike read_file / write_file / patch, bash is not path-confined by Scope —
// Scope only sets the default cwd; a command can still `cd /` or use absolute
// paths. cwd-pin + env-strip + classifier are defense-in-depth, not a jail.

use crate::security::scope::Scope;
use crate::security::{Severity, classify};
use crate::tools::registry::{Tool, ToolResult};

use anyhow::Result;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::process::Command;
use tokio::time::Duration;

const ALLOWED_ENV_KEYS: &[&str] = &[
    "PATH", "HOME", "USER", "LOGNAME", "SHELL", "LANG", "TERM", "TZ", "PWD",
];

#[derive(Deserialize)]
struct BashArgs {
    command: String,
    timeout_secs: Option<u32>,
}

pub struct Bash {
    scope: Scope,
    default_timeout_secs: u32,
}

#[async_trait]
impl Tool for Bash {
    fn name(&self) -> &str {
        "bash"
    }

    fn description(&self) -> &str {
        "Run a shell command in the project root. Non-zero exit codes appear in \
         the status line of the output body, not as advisory failures."
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "Full shell command. Passed to sh -c. Runs in the project root."
                },
                "timeout_secs": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Wall-clock timeout. Overrides the default if set."
                }
            },
            "required": ["command"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let parsed = match serde_json::from_value::<BashArgs>(args) {
            Ok(a) => a,
            Err(e) => {
                return Ok(ToolResult {
                    output: String::new(),
                    error: Some(format!("invalid arguments: {e}")),
                    metadata: None,
                });
            }
        };

        if parsed.command.is_empty() {
            return Ok(ToolResult {
                output: String::new(),
                error: Some("command must not be empty".to_string()),
                metadata: None,
            });
        }

        let timeout_secs = parsed.timeout_secs.unwrap_or(self.default_timeout_secs);

        if timeout_secs == 0 {
            return Ok(ToolResult {
                output: String::new(),
                error: Some("timeout_secs must be >= 1".to_string()),
                metadata: None,
            });
        }

        if classify(&parsed.command) == Severity::Block {
            return Ok(ToolResult {
                output: String::new(),
                error: Some(
                    "refused: command matches a blocked-command pattern \
                     (rm -rf, sudo, git push, curl | sh, …) — rephrase or narrow the operation"
                        .to_string(),
                ),
                metadata: None,
            });
        }

        let mut cmd = Command::new("sh");
        cmd.arg("-c")
            .arg(&parsed.command)
            .current_dir(self.scope.root())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        cmd.env_clear();
        for (key, value) in std::env::vars() {
            if is_allowed_env_key(&key) {
                cmd.env(&key, &value);
            }
        }

        let child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                return Ok(ToolResult {
                    output: String::new(),
                    error: Some(format!("failed to spawn shell: {e}")),
                    metadata: None,
                });
            }
        };

        let start = std::time::Instant::now();
        let child_id = child.id();

        let result = tokio::time::timeout(
            Duration::from_secs(timeout_secs as u64),
            child.wait_with_output(),
        )
        .await;

        let elapsed = start.elapsed();

        match result {
            Ok(Ok(output)) => {
                let stdout_str = String::from_utf8_lossy(&output.stdout);
                let stderr_str = String::from_utf8_lossy(&output.stderr);

                let mut combined = String::new();
                if !stdout_str.is_empty() {
                    combined.push_str(&stdout_str);
                }
                if !stderr_str.is_empty() {
                    if !combined.is_empty() {
                        combined.push('\n');
                    }
                    combined.push_str(&stderr_str);
                }

                let (body, truncated) = truncate_output(&combined);

                let exit_code = output.status.code();
                let status_line = match exit_code {
                    Some(0) => format!("✓ exit 0 ({:.1}s)", elapsed.as_secs_f64()),
                    Some(n) => format!("✗ exit {n} ({:.1}s)", elapsed.as_secs_f64()),
                    None => format!("✗ exit signal ({:.1}s)", elapsed.as_secs_f64()),
                };

                let output_body = format!("{status_line}\n\n{body}");

                let metadata = json!({
                    "exit_code": exit_code,
                    "duration_ms": elapsed.as_millis(),
                    "stdout_bytes": output.stdout.len(),
                    "stderr_bytes": output.stderr.len(),
                    "truncated": truncated,
                    "timed_out": false,
                });

                Ok(ToolResult {
                    output: output_body,
                    error: None,
                    metadata: Some(metadata),
                })
            }
            Ok(Err(e)) => Ok(ToolResult {
                output: String::new(),
                error: Some(format!("command execution failed: {e}")),
                metadata: None,
            }),
            Err(_) => {
                if let Some(id) = child_id {
                    let _ = std::process::Command::new("kill")
                        .args(["-9", &id.to_string()])
                        .output();
                }
                let elapsed_ms = start.elapsed().as_millis();

                let metadata = json!({
                    "exit_code": null,
                    "duration_ms": elapsed_ms,
                    "stdout_bytes": 0,
                    "stderr_bytes": 0,
                    "truncated": false,
                    "timed_out": true,
                    "timeout_secs": timeout_secs,
                });

                Ok(ToolResult {
                    output: String::new(),
                    error: Some(format!(
                        "command timed out after {timeout_secs}s and was killed"
                    )),
                    metadata: Some(metadata),
                })
            }
        }
    }
}

fn truncate_output(body: &str) -> (String, bool) {
    let lines: Vec<&str> = body.lines().collect();
    let total = lines.len();

    if total <= 100 {
        return (body.to_string(), false);
    }

    let head_count = 20;
    let tail_count = 80;
    let omitted = total - head_count - tail_count;

    let mut result = String::new();
    for line in &lines[..head_count] {
        result.push_str(line);
        result.push('\n');
    }
    result.push_str(&format!(
        "[… {omitted} lines omitted; full output not retained in this run …]\n"
    ));
    for line in &lines[total - tail_count..] {
        result.push_str(line);
        result.push('\n');
    }

    (result, true)
}

pub fn is_allowed_env_key(key: &str) -> bool {
    ALLOWED_ENV_KEYS.contains(&key) || key.starts_with("LC_")
}

pub fn bash(scope: Scope, default_timeout_secs: u32) -> Arc<dyn Tool> {
    Arc::new(Bash {
        scope,
        default_timeout_secs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::Path;

    fn make_bash(dir: &Path, timeout: u32) -> Arc<dyn Tool> {
        let scope = Scope::new(dir).unwrap();
        bash(scope, timeout)
    }

    #[tokio::test]
    async fn runs_zero_exit_command() {
        let dir = tempfile::TempDir::new().unwrap();
        let tool = make_bash(dir.path(), 30);
        let result = tool
            .execute(json!({ "command": "echo hello" }))
            .await
            .unwrap();

        assert!(result.error.is_none());
        assert!(result.output.contains("✓ exit 0"));
        assert!(result.output.contains("hello"));
        let meta = result.metadata.unwrap();
        assert_eq!(meta["exit_code"], 0);
    }

    #[tokio::test]
    async fn non_zero_exit_appears_in_status_line() {
        let dir = tempfile::TempDir::new().unwrap();
        let tool = make_bash(dir.path(), 30);
        let result = tool
            .execute(json!({ "command": "sh -c 'exit 3'" }))
            .await
            .unwrap();

        assert!(result.error.is_none());
        assert!(result.output.starts_with("✗ exit 3"));
        let meta = result.metadata.unwrap();
        assert_eq!(meta["exit_code"], 3);
    }

    #[tokio::test]
    async fn captures_stderr() {
        let dir = tempfile::TempDir::new().unwrap();
        let tool = make_bash(dir.path(), 30);
        let result = tool
            .execute(json!({ "command": "sh -c 'echo to-stderr >&2'" }))
            .await
            .unwrap();

        assert!(result.error.is_none());
        assert!(result.output.contains("to-stderr"));
        let meta = result.metadata.unwrap();
        assert!(meta["stderr_bytes"].as_u64().unwrap() > 0);
    }

    #[tokio::test]
    async fn captures_both_streams_together() {
        let dir = tempfile::TempDir::new().unwrap();
        let tool = make_bash(dir.path(), 30);
        let result = tool
            .execute(json!({ "command": "sh -c 'echo out; echo err >&2'" }))
            .await
            .unwrap();

        assert!(result.error.is_none());
        assert!(result.output.contains("out"));
        assert!(result.output.contains("err"));
    }

    #[tokio::test]
    async fn truncates_long_output() {
        let dir = tempfile::TempDir::new().unwrap();
        let tool = make_bash(dir.path(), 30);
        let result = tool
            .execute(json!({
                "command": "sh -c 'for i in $(seq 1 200); do echo \"line $i\"; done'"
            }))
            .await
            .unwrap();

        assert!(result.error.is_none());
        assert!(result.output.contains("line 1"));
        assert!(result.output.contains("line 200"));
        assert!(result.output.contains("omitted"));
        let meta = result.metadata.unwrap();
        assert!(meta["truncated"].as_bool().unwrap());
    }

    #[tokio::test]
    async fn does_not_truncate_short_output() {
        let dir = tempfile::TempDir::new().unwrap();
        let tool = make_bash(dir.path(), 30);
        let result = tool
            .execute(json!({
                "command": "sh -c 'for i in $(seq 1 50); do echo \"line $i\"; done'"
            }))
            .await
            .unwrap();

        assert!(result.error.is_none());
        assert!(result.output.contains("line 25"));
        assert!(!result.output.contains("omitted"));
        let meta = result.metadata.unwrap();
        assert!(!meta["truncated"].as_bool().unwrap());
    }

    #[tokio::test]
    async fn times_out_advisory_failure() {
        let dir = tempfile::TempDir::new().unwrap();
        let tool = make_bash(dir.path(), 30);
        let result = tool
            .execute(json!({
                "command": "sleep 5",
                "timeout_secs": 1
            }))
            .await
            .unwrap();

        assert!(result.error.is_some());
        let err = result.error.as_ref().unwrap();
        assert!(err.contains("timed out"));
        let meta = result.metadata.unwrap();
        assert!(meta["timed_out"].as_bool().unwrap());
        assert_eq!(meta["timeout_secs"], 1);
    }

    #[tokio::test]
    async fn default_timeout_used_when_arg_absent() {
        let dir = tempfile::TempDir::new().unwrap();
        let tool = make_bash(dir.path(), 1);
        let result = tool.execute(json!({ "command": "sleep 5" })).await.unwrap();

        assert!(result.error.is_some());
        assert!(result.error.as_ref().unwrap().contains("timed out"));
        let meta = result.metadata.unwrap();
        assert!(meta["timed_out"].as_bool().unwrap());
    }

    #[tokio::test]
    async fn arg_timeout_overrides_constructor_default() {
        let dir = tempfile::TempDir::new().unwrap();
        let tool = make_bash(dir.path(), 30);
        let result = tool
            .execute(json!({
                "command": "sleep 5",
                "timeout_secs": 1
            }))
            .await
            .unwrap();

        assert!(result.error.is_some());
        assert!(result.error.as_ref().unwrap().contains("timed out"));
        let meta = result.metadata.unwrap();
        assert!(meta["timed_out"].as_bool().unwrap());
    }

    #[tokio::test]
    async fn rejects_empty_command() {
        let dir = tempfile::TempDir::new().unwrap();
        let tool = make_bash(dir.path(), 30);
        let result = tool.execute(json!({ "command": "" })).await.unwrap();

        assert!(result.error.is_some());
        assert!(result.error.as_ref().unwrap().contains("empty"));
    }

    #[tokio::test]
    async fn rejects_malformed_args() {
        let dir = tempfile::TempDir::new().unwrap();
        let tool = make_bash(dir.path(), 30);
        let result = tool.execute(json!({ "timeout_secs": 5 })).await.unwrap();

        assert!(result.error.is_some());
        assert!(result.error.as_ref().unwrap().contains("invalid arguments"));
    }

    #[tokio::test]
    async fn blocked_command_is_not_executed() {
        let dir = tempfile::TempDir::new().unwrap();
        let tool = make_bash(dir.path(), 30);
        let result = tool
            .execute(json!({ "command": "sudo touch should_not_exist.txt" }))
            .await
            .unwrap();

        assert!(result.error.is_some());
        let err = result.error.as_ref().unwrap();
        assert!(err.contains("refused"));
        assert!(err.contains("blocked-command pattern"));

        let test_file = dir.path().join("should_not_exist.txt");
        assert!(
            !test_file.exists(),
            "blocked command must not have been executed"
        );
    }

    #[tokio::test]
    async fn cwd_is_pinned_to_scope_root() {
        let dir = tempfile::TempDir::new().unwrap();
        let tool = make_bash(dir.path(), 30);
        let result = tool.execute(json!({ "command": "pwd" })).await.unwrap();

        assert!(result.error.is_none());
        let expected_root = dir.path().canonicalize().unwrap();
        assert!(
            result
                .output
                .contains(expected_root.to_string_lossy().as_ref()),
            "pwd output should contain scope root: {}",
            result.output
        );
    }

    #[test]
    fn is_allowed_env_key_allows_whitelisted() {
        assert!(is_allowed_env_key("PATH"));
        assert!(is_allowed_env_key("HOME"));
        assert!(is_allowed_env_key("USER"));
        assert!(is_allowed_env_key("LOGNAME"));
        assert!(is_allowed_env_key("SHELL"));
        assert!(is_allowed_env_key("LANG"));
        assert!(is_allowed_env_key("TERM"));
        assert!(is_allowed_env_key("TZ"));
        assert!(is_allowed_env_key("PWD"));
    }

    #[test]
    fn is_allowed_env_key_allows_lc_prefix() {
        assert!(is_allowed_env_key("LC_ALL"));
        assert!(is_allowed_env_key("LC_CTYPE"));
        assert!(is_allowed_env_key("LC_MESSAGES"));
    }

    #[test]
    fn is_allowed_env_key_rejects_others() {
        assert!(!is_allowed_env_key("AWS_SECRET_ACCESS_KEY"));
        assert!(!is_allowed_env_key("SOME_RANDOM_VAR"));
        assert!(!is_allowed_env_key("SECRET_KEY"));
        assert!(!is_allowed_env_key("FOO"));
    }
}
