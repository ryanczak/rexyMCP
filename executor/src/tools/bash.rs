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
    filter: bool,
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
        let required = ["command"];
        let present: Vec<&str> = args
            .as_object()
            .map(|m| {
                required
                    .iter()
                    .copied()
                    .filter(|k| m.contains_key(*k))
                    .collect()
            })
            .unwrap_or_default();
        let parsed = match serde_json::from_value::<BashArgs>(args) {
            Ok(a) => a,
            Err(_) => {
                return Ok(ToolResult {
                    output: String::new(),
                    error: Some(super::registry::missing_args_hint(
                        "bash", &required, &present,
                    )),
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

                let (body, truncated) = if self.filter {
                    crate::context::output_filter::filter_for_command(
                        &parsed.command,
                        &combined,
                        self.scope.root(),
                    )
                } else {
                    truncate_output(&combined)
                };

                let exit_code = output.status.code();
                let status_line = match exit_code {
                    Some(0) => format!("✓ exit 0 ({:.1}s)", elapsed.as_secs_f64()),
                    Some(n) => format!("✗ exit {n} ({:.1}s)", elapsed.as_secs_f64()),
                    None => format!("✗ exit signal ({:.1}s)", elapsed.as_secs_f64()),
                };

                let output_body = format!("{status_line}\n\n{body}");

                let mut metadata = json!({
                    "exit_code": exit_code,
                    "duration_ms": elapsed.as_millis(),
                    "stdout_bytes": output.stdout.len(),
                    "stderr_bytes": output.stderr.len(),
                    "truncated": truncated,
                    "timed_out": false,
                });
                if self.filter {
                    let filter_label =
                        if crate::context::output_filter::is_cargo_command(&parsed.command) {
                            "cargo"
                        } else {
                            "generic"
                        };
                    metadata["output_filter"] = json!({
                        "tokens_before": crate::context::tokens::count(&combined),
                        "tokens_after": crate::context::tokens::count(&body),
                        "filter": filter_label,
                    });
                }

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
    bash_with_filter(scope, default_timeout_secs, true)
}

pub fn bash_with_filter(scope: Scope, default_timeout_secs: u32, filter: bool) -> Arc<dyn Tool> {
    Arc::new(Bash {
        scope,
        default_timeout_secs,
        filter,
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

        let err = result.error.as_ref().unwrap();
        assert!(
            err.contains("command"),
            "should name the missing field: {err}"
        );
        assert!(
            !err.contains("invalid arguments: missing field"),
            "no raw serde text: {err}"
        );
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

    #[tokio::test]
    async fn filtered_bash_truncation_writes_recovery_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let tool = bash_with_filter(scope, 30, true);
        let result = tool
            .execute(json!({
                "command": "sh -c 'for i in $(seq 1 200); do echo \"line $i\"; done'"
            }))
            .await
            .unwrap();

        assert!(result.error.is_none());
        assert!(result.output.contains("omitted"));
        assert!(
            result
                .output
                .contains("full output: .rexymcp/output/cmd-output-"),
            "marker should reference recovery file"
        );

        // Recovery file should exist
        let recovery_dir = dir.path().join(".rexymcp/output");
        assert!(recovery_dir.exists(), "recovery dir should exist");
        let files: Vec<_> = std::fs::read_dir(&recovery_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert!(
            !files.is_empty(),
            "at least one cmd-output-*.log should exist"
        );
    }

    #[tokio::test]
    async fn kill_switch_off_uses_legacy_truncation_without_recovery() {
        let dir = tempfile::TempDir::new().unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let tool = bash_with_filter(scope, 30, false);
        let result = tool
            .execute(json!({
                "command": "sh -c 'for i in $(seq 1 200); do echo \"line $i\"; done'"
            }))
            .await
            .unwrap();

        assert!(result.error.is_none());
        assert!(
            result.output.contains("full output not retained"),
            "legacy marker should appear"
        );
        assert!(
            !dir.path().join(".rexymcp/output").exists(),
            "no recovery dir should be created when filter is off"
        );
    }

    #[tokio::test]
    async fn cargo_command_output_is_filtered_through_cargo_filter() {
        let dir = tempfile::TempDir::new().unwrap();
        // Create a minimal Cargo project in the TempDir
        std::fs::write(
            dir.path().join("Cargo.toml"),
            r#"[package]
name = "scratch"
version = "0.1.0"
edition = "2021"
"#,
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/lib.rs"),
            r#"#[test]
fn passes() {}
#[test]
fn fails() { panic!("oh no"); }
"#,
        )
        .unwrap();

        let scope = Scope::new(dir.path()).unwrap();
        let tool = bash_with_filter(scope, 60, true);
        let result = tool
            .execute(json!({ "command": "cargo test 2>&1" }))
            .await
            .unwrap();

        assert!(
            result.error.is_none(),
            "cargo test should succeed as a tool call: {}",
            result
                .error
                .as_ref()
                .map_or("none".to_string(), |e| e.clone())
        );
        let output = &result.output;

        // (a) The failing test name should appear
        assert!(
            output.contains("fails"),
            "failing test name should appear in filtered output: {output}"
        );

        // (b) Passing-test `... ok` lines should be absent
        assert!(
            !output.contains("test passes ... ok"),
            "passing test line should be filtered out: {output}"
        );

        // (c) `test result:` summary should appear
        assert!(
            output.contains("test result:"),
            "test result summary should appear: {output}"
        );
    }

    #[tokio::test]
    async fn filter_on_records_output_filter_metadata() {
        let dir = tempfile::TempDir::new().unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let tool = bash_with_filter(scope, 30, true);
        let result = tool
            .execute(json!({
                "command": "sh -c 'for i in $(seq 1 200); do echo \"line $i\"; done'"
            }))
            .await
            .unwrap();

        assert!(result.error.is_none());
        let meta = result
            .metadata
            .as_ref()
            .expect("metadata should be present");
        let of = meta
            .get("output_filter")
            .expect("output_filter should be present when filter is on");
        let before = of.get("tokens_before").and_then(|v| v.as_u64()).unwrap();
        let after = of.get("tokens_after").and_then(|v| v.as_u64()).unwrap();
        let filter = of.get("filter").and_then(|v| v.as_str()).unwrap();
        assert!(
            after < before,
            "tokens_after ({after}) should be less than tokens_before ({before})"
        );
        assert_eq!(filter, "generic");
    }

    #[tokio::test]
    async fn cargo_command_records_cargo_filter_label() {
        let dir = tempfile::TempDir::new().unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let tool = bash_with_filter(scope, 30, true);
        // Command string starts with "cargo" — it may fail to run (no cargo installed),
        // but the filter label is derived from the command string, not the exit code.
        let result = tool
            .execute(json!({ "command": "cargo build" }))
            .await
            .unwrap();

        let meta = result
            .metadata
            .as_ref()
            .expect("metadata should be present");
        let of = meta
            .get("output_filter")
            .expect("output_filter should be present when filter is on");
        let filter = of.get("filter").and_then(|v| v.as_str()).unwrap();
        assert_eq!(filter, "cargo");
    }

    #[tokio::test]
    async fn filter_off_records_no_output_filter_metadata() {
        let dir = tempfile::TempDir::new().unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let tool = bash_with_filter(scope, 30, false);
        let result = tool
            .execute(json!({
                "command": "sh -c 'for i in $(seq 1 200); do echo \"line $i\"; done'"
            }))
            .await
            .unwrap();

        assert!(result.error.is_none());
        let meta = result
            .metadata
            .as_ref()
            .expect("metadata should be present");
        assert!(
            meta.get("output_filter").is_none(),
            "output_filter should not be present when filter is off"
        );
    }
}
