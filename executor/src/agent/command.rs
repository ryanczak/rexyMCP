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
