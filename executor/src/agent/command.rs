//! The final-command-set seam. The loop depends on this trait, not on
//! `tokio::process` directly, so tests inject a deterministic mock instead of
//! spawning real subprocesses (`cargo build`, `npm test`, …).

use std::path::Path;

use async_trait::async_trait;

/// Outcome of a final-command-set run: the captured output (for
/// `PhaseResult.command_outputs`) and whether it exited successfully (for the
/// `PhaseRun` gate booleans).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandResult {
    pub output: String,
    pub success: bool,
}

/// Runs one shell command in a working directory and reports its combined
/// stdout+stderr plus exit success. Capturing the result is the whole job — a
/// command that fails to spawn or exits non-zero yields `success: false`, never an
/// error to the loop.
#[async_trait]
pub trait CommandRunner: Send + Sync {
    async fn run(&self, command: &str, cwd: &Path) -> CommandResult;
}

/// Production runner: `sh -c <command>` via `tokio::process`, stdout then stderr.
pub struct RealCommandRunner;

#[async_trait]
impl CommandRunner for RealCommandRunner {
    async fn run(&self, command: &str, cwd: &Path) -> CommandResult {
        match tokio::process::Command::new("sh")
            .arg("-c")
            .arg(command)
            .current_dir(cwd)
            .output()
            .await
        {
            Ok(out) => {
                let mut combined = String::from_utf8_lossy(&out.stdout).into_owned();
                let stderr = String::from_utf8_lossy(&out.stderr);
                if !stderr.is_empty() {
                    combined.push_str(&stderr);
                }
                CommandResult {
                    output: combined,
                    success: out.status.success(),
                }
            }
            Err(e) => CommandResult {
                output: format!("failed to run `{command}`: {e}"),
                success: false,
            },
        }
    }
}

use crate::config::CommandConfig;
use crate::phase::CommandOutputs;
use crate::store::telemetry::Gates;

use super::progress::{EmitCtx, emit_progress};

/// Tail cap on each captured final-command-set output.
pub(super) const MAX_COMMAND_TAIL_CHARS: usize = 4_000;

pub(super) async fn run_command_set(
    runner: &dyn CommandRunner,
    commands: &CommandConfig,
    cwd: &Path,
    ctx: &EmitCtx<'_>,
) -> (CommandOutputs, Gates) {
    if commands.format.is_some() {
        emit_progress(ctx, "command:fmt".to_string());
    }
    let (format, fmt_ok) = run_one(runner, commands.format.as_deref(), cwd).await;
    if commands.build.is_some() {
        emit_progress(ctx, "command:build".to_string());
    }
    let (build, build_ok) = run_one(runner, commands.build.as_deref(), cwd).await;
    if commands.lint.is_some() {
        emit_progress(ctx, "command:lint".to_string());
    }
    let (lint, lint_ok) = run_one(runner, commands.lint.as_deref(), cwd).await;
    if commands.test.is_some() {
        emit_progress(ctx, "command:test".to_string());
    }
    let (test, test_ok) = run_one(runner, commands.test.as_deref(), cwd).await;
    (
        CommandOutputs {
            format,
            build,
            lint,
            test,
        },
        Gates {
            fmt: fmt_ok,
            build: build_ok,
            lint: lint_ok,
            test: test_ok,
        },
    )
}

pub(super) fn gate_failure_feedback(gates: &Gates, outputs: &CommandOutputs) -> Option<String> {
    let mut sections: Vec<String> = Vec::new();
    if gates.fmt == Some(false) {
        sections.push(format!(
            "FORMAT failed:\n{}",
            outputs.format.as_deref().unwrap_or("(no output captured)")
        ));
    }
    if gates.build == Some(false) {
        sections.push(format!(
            "BUILD failed:\n{}",
            outputs.build.as_deref().unwrap_or("(no output captured)")
        ));
    }
    if gates.lint == Some(false) {
        sections.push(format!(
            "LINT failed:\n{}",
            outputs.lint.as_deref().unwrap_or("(no output captured)")
        ));
    }
    if gates.test == Some(false) {
        sections.push(format!(
            "TEST failed:\n{}",
            outputs.test.as_deref().unwrap_or("(no output captured)")
        ));
    }
    if sections.is_empty() {
        return None;
    }
    Some(format!(
        "Pre-completion gate check failed — the phase is not done yet. \
         Fix the issues below, then re-emit your completion signal.\n\n{}",
        sections.join("\n\n")
    ))
}

pub(super) async fn run_post_write_hooks(
    runner: &dyn CommandRunner,
    commands: &CommandConfig,
    cwd: &Path,
) {
    if let Some(cmd) = commands.lint_fix.as_deref() {
        let _ = runner.run(cmd, cwd).await;
    }
    if let Some(cmd) = commands.format.as_deref() {
        let _ = runner.run(cmd, cwd).await;
    }
}

async fn run_one(
    runner: &dyn CommandRunner,
    command: Option<&str>,
    cwd: &Path,
) -> (Option<String>, Option<bool>) {
    match command {
        Some(cmd) => {
            let CommandResult { output, success } = runner.run(cmd, cwd).await;
            (Some(tail(&output, MAX_COMMAND_TAIL_CHARS)), Some(success))
        }
        None => (None, None),
    }
}

fn tail(s: &str, max_chars: usize) -> String {
    let count = s.chars().count();
    if count > max_chars {
        s.chars().skip(count - max_chars).collect()
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::phase::CommandOutputs;
    use crate::store::telemetry::Gates;

    fn outputs_with(format: &str, build: &str, lint: &str, test: &str) -> CommandOutputs {
        CommandOutputs {
            format: Some(format.to_string()),
            build: Some(build.to_string()),
            lint: Some(lint.to_string()),
            test: Some(test.to_string()),
        }
    }

    #[test]
    fn gate_failure_feedback_returns_none_when_all_pass() {
        let gates = Gates {
            fmt: Some(true),
            build: Some(true),
            lint: Some(true),
            test: Some(true),
        };
        assert!(gate_failure_feedback(&gates, &outputs_with("ok", "ok", "ok", "ok")).is_none());
    }

    #[test]
    fn gate_failure_feedback_returns_none_when_no_commands_configured() {
        // Gates::default() is all None — unconfigured commands are not failures.
        assert!(gate_failure_feedback(&Gates::default(), &CommandOutputs::default()).is_none());
    }

    #[test]
    fn gate_failure_feedback_includes_failing_gates_and_omits_passing() {
        let gates = Gates {
            fmt: Some(false),
            build: Some(false),
            lint: Some(true),
            test: Some(false),
        };
        let outputs = outputs_with("fmt diff here", "build errors", "lint ok", "test failed");
        let msg = gate_failure_feedback(&gates, &outputs).expect("should be Some");
        assert!(msg.contains("FORMAT failed"), "missing FORMAT section");
        assert!(msg.contains("fmt diff here"), "missing FORMAT output");
        assert!(msg.contains("BUILD failed"), "missing BUILD section");
        assert!(msg.contains("build errors"), "missing BUILD output");
        assert!(!msg.contains("LINT"), "LINT should not appear (it passed)");
        assert!(msg.contains("TEST failed"), "missing TEST section");
    }
}
