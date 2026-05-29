//! The final-command-set seam. The loop depends on this trait, not on
//! `tokio::process` directly, so tests inject a deterministic mock instead of
//! spawning real subprocesses (`cargo build`, `npm test`, …).

use std::path::Path;

use async_trait::async_trait;

/// Runs one shell command in a working directory and returns its combined
/// stdout+stderr. Capturing the output is the whole job — a command that fails to
/// spawn or exits non-zero yields a diagnostic string, never an error to the loop.
#[async_trait]
pub trait CommandRunner: Send + Sync {
    async fn run(&self, command: &str, cwd: &Path) -> String;
}

/// Production runner: `sh -c <command>` via `tokio::process`, stdout then stderr.
pub struct RealCommandRunner;

#[async_trait]
impl CommandRunner for RealCommandRunner {
    async fn run(&self, command: &str, cwd: &Path) -> String {
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
                combined
            }
            Err(e) => format!("failed to run `{command}`: {e}"),
        }
    }
}
