//! Post-edit verifier. Runs `cargo check`, `tsc`, or `ruff check` after every
//! successful `patch` / `write_file` tool dispatch. Emits structured
//! Diagnostics that the agent loop folds into the next `<tool_result>` message.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde::{Deserialize, Serialize};
use tokio::process::Command;

/// Severity of a single diagnostic. Mapped from the compiler's level field:
/// "error" → Error, "warning" → Warning, "note" → Note, "help" → Help.
/// Only errors are fed back to the model (warnings would be noise); the type
/// carries all four for future flexibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Severity {
    Error,
    Warning,
    Note,
    Help,
}

/// One compile/lint message. Populated from the compiler's structured output.
/// The shape is stable across languages — all supported compilers map into
/// this record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub path: PathBuf,
    /// 1-indexed line number.
    pub line: u32,
    /// 1-indexed column number; None when the diagnostic
    /// doesn't have a specific column (e.g., file-level
    /// warnings).
    pub column: Option<u32>,
    pub severity: Severity,
    pub message: String,
    /// Compiler's error code (e.g., "E0382" for Rust's
    /// borrow-of-moved-value). None for diagnostics that
    /// don't have codes.
    pub code: Option<String>,
}

/// A signature for matching a diagnostic against the baseline.
/// Deliberately ignores `line` and `column` because compiler
/// diagnostics shift those when surrounding code changes.
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct DiagnosticSignature {
    pub path: PathBuf,
    pub severity: Severity,
    pub code: Option<String>,
    pub message: String,
}

impl Diagnostic {
    pub fn signature(&self) -> DiagnosticSignature {
        DiagnosticSignature {
            path: self.path.clone(),
            severity: self.severity,
            code: self.code.clone(),
            message: self.message.clone(),
        }
    }
}

/// Outcome of a single `verify(...)` call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifierResult {
    /// The verifier ran successfully. `diagnostics` may be
    /// empty (everything compiled clean) or non-empty (real
    /// errors found).
    Checked { diagnostics: Vec<Diagnostic> },
    /// The file's extension isn't supported by any
    /// available checker (e.g., `.md`, `.toml`, an
    /// unrecognized language). Distinct from "ran and
    /// found no errors." The agent loop skips this case —
    /// no verifier output is appended to the tool_result.
    Unsupported,
    /// Infrastructure failure — process spawn failed, the
    /// output didn't parse, the file's crate root couldn't
    /// be found, etc. Distinct from "ran and found errors."
    /// The agent loop appends a brief "verifier failed: X"
    /// notice but doesn't treat this as the model's fault.
    Failed(String),
    /// A required checker binary isn't installed (not on PATH).
    /// Distinct from `Failed` (a genuine infra error) and from
    /// `Unsupported` (the file type has no checker at all). The
    /// agent loop surfaces this as a one-line advisory naming the
    /// binary and how to install it; it is NOT the model's fault
    /// and never counts toward verifier-persistence hard-fail.
    Skipped(String),
}

/// A set of diagnostic signatures present at session start.
/// Used to classify subsequent diagnostics as author or
/// ambient.
#[derive(Debug, Clone, Default)]
pub struct Baseline {
    pub signatures: HashSet<DiagnosticSignature>,
}

impl Baseline {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a diagnostic's signature to the baseline.
    pub fn record(&mut self, diagnostic: &Diagnostic) {
        self.signatures.insert(diagnostic.signature());
    }

    /// True if this diagnostic matches a signature already in
    /// the baseline (i.e., it's ambient).
    pub fn contains(&self, diagnostic: &Diagnostic) -> bool {
        self.signatures.contains(&diagnostic.signature())
    }

    /// Classify a list of diagnostics. Returns `(author,
    /// ambient)` partitions. Order preserved within each
    /// partition (insertion order from the input).
    pub fn partition<'a>(
        &self,
        diagnostics: &'a [Diagnostic],
    ) -> (Vec<&'a Diagnostic>, Vec<&'a Diagnostic>) {
        let mut author = Vec::new();
        let mut ambient = Vec::new();
        for d in diagnostics {
            if self.contains(d) {
                ambient.push(d);
            } else {
                author.push(d);
            }
        }
        (author, ambient)
    }
}

/// Capture a baseline by running the verifier on a set of
/// paths and collecting all reported diagnostic signatures.
/// Called at session start.
///
/// Each per-language checker (`cargo check`, `tsc`, `ruff
/// check`) inspects the whole project regardless of which file
/// is passed in, so this function deduplicates `paths` to one
/// representative path per (extension, project-root) group
/// before invoking the verifier. Without this dedup, startup on
/// a tree with N source files runs the checker N times — for a
/// rust workspace that's tens of seconds wasted on
/// cache-rebuilds; for a python project with a vendored venv
/// it's hundreds of `ruff` spawns.
pub async fn capture_baseline(paths: &[PathBuf]) -> Baseline {
    let mut baseline = Baseline::new();

    // Dedup key: (extension, project_root_or_path). For files
    // whose project root can't be resolved (e.g., a .rs file
    // outside any Cargo.toml ancestor), fall back to the file
    // path itself so each such file still runs individually.
    let mut seen: HashSet<(String, PathBuf)> = HashSet::new();
    let mut representatives: Vec<PathBuf> = Vec::new();
    for path in paths {
        let ext = match path.extension().and_then(|s| s.to_str()) {
            Some(e) => e.to_string(),
            None => continue,
        };
        let root = project_root_for(path, &ext).unwrap_or_else(|| path.clone());
        if seen.insert((ext, root)) {
            representatives.push(path.clone());
        }
    }

    for path in &representatives {
        match verify(path).await {
            VerifierResult::Checked { diagnostics } => {
                for d in &diagnostics {
                    baseline.record(d);
                }
            }
            VerifierResult::Unsupported
            | VerifierResult::Failed(_)
            | VerifierResult::Skipped(_) => {
                // No baseline diagnostics for files the
                // verifier can't check. Skip.
            }
        }
    }
    baseline
}

/// Resolve the project root used by the per-language verifier
/// for a given file. `.rs` → nearest `Cargo.toml` ancestor;
/// `.ts`/`.tsx` → nearest `tsconfig.json` ancestor; `.py` →
/// nearest `pyproject.toml` ancestor (or `setup.py`); falls
/// back to None when no marker is found, in which case the
/// caller treats the file itself as its own root.
fn project_root_for(path: &Path, ext: &str) -> Option<PathBuf> {
    match ext {
        "rs" => find_ancestor_with(path, "Cargo.toml"),
        "ts" | "tsx" => find_ancestor_with(path, "tsconfig.json"),
        "py" => find_ancestor_with(path, "pyproject.toml")
            .or_else(|| find_ancestor_with(path, "setup.py")),
        _ => None,
    }
}

fn find_ancestor_with(start: &Path, marker: &str) -> Option<PathBuf> {
    let mut current = if start.is_dir() {
        Some(start)
    } else {
        start.parent()
    };
    while let Some(dir) = current {
        if dir.join(marker).exists() {
            return Some(dir.to_path_buf());
        }
        current = dir.parent();
    }
    None
}

/// Top-level verifier entry point. Dispatches by file
/// extension to the appropriate per-language checker.
/// Supports `.rs`, `.ts`, `.tsx`, and `.py`.
pub async fn verify(path: &Path) -> VerifierResult {
    let ext = match path.extension().and_then(|s| s.to_str()) {
        Some(ext) => ext,
        None => return VerifierResult::Unsupported,
    };

    match ext {
        "rs" => verify_rust(path).await,
        "ts" | "tsx" => verify_typescript(path).await,
        "py" => verify_python(path).await,
        _ => VerifierResult::Unsupported,
    }
}

/// Map a checker-spawn `io::Error` to a VerifierResult. A
/// `NotFound` error means the toolchain binary isn't installed —
/// a `Skipped` advisory that names the remedy. Any other spawn
/// error is a genuine infrastructure `Failed`.
fn spawn_failure(tool: &str, install_hint: &str, err: &std::io::Error) -> VerifierResult {
    if err.kind() == std::io::ErrorKind::NotFound {
        VerifierResult::Skipped(format!(
            "{tool} not found on PATH — {install_hint}; \
             incremental verification is disabled this run"
        ))
    } else {
        VerifierResult::Failed(format!("{tool} spawn failed: {err}"))
    }
}

async fn verify_rust(path: &Path) -> VerifierResult {
    let crate_root = match find_crate_root(path) {
        Some(root) => root,
        None => {
            return VerifierResult::Failed(format!(
                "no Cargo.toml found at or above {}",
                path.display(),
            ));
        }
    };

    let output = match Command::new("cargo")
        .arg("check")
        .arg("--message-format=json")
        .current_dir(&crate_root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
    {
        Ok(o) => o,
        Err(e) => {
            return spawn_failure(
                "cargo",
                "install the Rust toolchain via https://rustup.rs",
                &e,
            );
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut diagnostics = Vec::new();
    for line in stdout.lines() {
        if let Some(diag) = parse_cargo_line(line, &crate_root) {
            diagnostics.push(diag);
        }
    }

    VerifierResult::Checked { diagnostics }
}

fn find_crate_root(start: &Path) -> Option<PathBuf> {
    let mut current = if start.is_dir() {
        start
    } else {
        start.parent()?
    };
    loop {
        if current.join("Cargo.toml").exists() {
            return Some(current.to_path_buf());
        }
        current = current.parent()?;
    }
}

/// Collect rustc machine-applicable suggested fixes from a compiler `message`
/// object. rustc attaches these to `help` child diagnostics whose spans carry a
/// string `suggested_replacement` and `suggestion_applicability ==
/// "MachineApplicable"`. Returns one model-facing line per suggestion. Only
/// MachineApplicable is surfaced — MaybeIncorrect / HasPlaceholders /
/// Unspecified are guesses and are excluded.
fn collect_machine_suggestions(message: &serde_json::Value) -> Vec<String> {
    let mut out = Vec::new();
    collect_suggestions_into(message, &mut out);
    out
}

fn collect_suggestions_into(node: &serde_json::Value, out: &mut Vec<String>) {
    let help = node.get("message").and_then(|m| m.as_str()).unwrap_or("");
    if let Some(spans) = node.get("spans").and_then(|s| s.as_array()) {
        for span in spans {
            if span
                .get("suggestion_applicability")
                .and_then(|v| v.as_str())
                != Some("MachineApplicable")
            {
                continue;
            }
            let Some(replacement) = span.get("suggested_replacement").and_then(|v| v.as_str())
            else {
                continue;
            };
            let line = span.get("line_start").and_then(|v| v.as_u64()).unwrap_or(0);
            let loc = match span.get("column_start").and_then(|v| v.as_u64()) {
                Some(col) => format!("line {line}:{col}"),
                None => format!("line {line}"),
            };
            let rationale = if help.is_empty() {
                String::new()
            } else {
                format!(" — {help}")
            };
            out.push(format!(
                "rustc suggests (machine-applicable): replace at {loc} \
                 with `{replacement}`{rationale}"
            ));
        }
    }
    if let Some(children) = node.get("children").and_then(|c| c.as_array()) {
        for child in children {
            collect_suggestions_into(child, out);
        }
    }
}

/// Parse a single line of `cargo check --message-format=json`
/// stdout. Returns Some(Diagnostic) for error-level
/// compiler-message lines that have a primary span;
/// None for everything else (warnings, build progress
/// envelopes, etc.).
fn parse_cargo_line(line: &str, crate_root: &Path) -> Option<Diagnostic> {
    let envelope: serde_json::Value = serde_json::from_str(line).ok()?;

    // Filter to compiler-message envelopes.
    if envelope.get("reason")?.as_str()? != "compiler-message" {
        return None;
    }

    let message = envelope.get("message")?;
    let level = message.get("level")?.as_str()?;

    // Errors only.
    if level != "error" {
        return None;
    }

    let mut text = message.get("message")?.as_str()?.to_string();
    let code = message
        .get("code")
        .and_then(|c| c.get("code"))
        .and_then(|s| s.as_str())
        .map(String::from);

    // Find the primary span. Cargo spans have an
    // `is_primary: true` flag on the one we want.
    let spans = message.get("spans")?.as_array()?;
    let primary = spans.iter().find(|s| {
        s.get("is_primary")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    })?;

    let file_name = primary.get("file_name")?.as_str()?;
    let line_start = primary.get("line_start")?.as_u64()? as u32;
    let column_start = primary
        .get("column_start")
        .and_then(|v| v.as_u64())
        .map(|c| c as u32);

    // Cargo's file_name is relative to the crate root.
    let path = crate_root.join(file_name);

    // Append machine-applicable rustc suggestions to the message.
    for suggestion in collect_machine_suggestions(message) {
        text.push_str("\n  ");
        text.push_str(&suggestion);
    }

    Some(Diagnostic {
        path,
        line: line_start,
        column: column_start,
        severity: Severity::Error,
        message: text,
        code,
    })
}

/// A resolved `tsc` invocation: the program to spawn plus any
/// prefix args that must precede tsc's own flags. `prefix_args`
/// is non-empty only for the `npx` form (`npx --no-install tsc`).
struct TscCommand {
    program: PathBuf,
    prefix_args: &'static [&'static str],
}

/// The PATH search directories, or empty if PATH is unset.
fn path_dirs() -> Vec<PathBuf> {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default()
}

/// Walk from `project_root` up to the filesystem root looking for
/// `node_modules/.bin/tsc`. Returns the first existing *file*
/// (a directory of that name is not a match). Walking up catches
/// monorepo dependency hoisting, where `node_modules` sits at the
/// workspace root above the package's `tsconfig.json`.
fn find_local_tsc(project_root: &Path) -> Option<PathBuf> {
    let mut current = Some(project_root);
    while let Some(dir) = current {
        let candidate = dir.join("node_modules").join(".bin").join("tsc");
        if candidate.is_file() {
            return Some(candidate);
        }
        current = dir.parent();
    }
    None
}

/// True if `name` resolves to an existing file in any of the
/// given search directories. Mirrors `doctor::resolve_binary`'s
/// bare-name branch; kept local because the `mcp` crate (where
/// that lives) depends on this one, not the reverse.
fn binary_in_dirs(name: &str, search_paths: &[PathBuf]) -> bool {
    search_paths.iter().any(|dir| dir.join(name).is_file())
}

/// Resolve which `tsc` invocation to spawn, in priority order:
/// local `node_modules/.bin/tsc` → `npx --no-install tsc` → bare
/// `tsc` on PATH. `npx_on_path` is threaded in (not read from the
/// environment here) so the resolution stays a pure, hermetically
/// testable function.
fn resolve_tsc_command(project_root: &Path, npx_on_path: bool) -> TscCommand {
    if let Some(local) = find_local_tsc(project_root) {
        return TscCommand {
            program: local,
            prefix_args: &[],
        };
    }
    if npx_on_path {
        return TscCommand {
            program: PathBuf::from("npx"),
            prefix_args: &["--no-install", "tsc"],
        };
    }
    TscCommand {
        program: PathBuf::from("tsc"),
        prefix_args: &[],
    }
}

async fn verify_typescript(path: &Path) -> VerifierResult {
    let project_root = match find_typescript_project_root(path) {
        Some(root) => root,
        None => {
            return VerifierResult::Failed(format!(
                "no tsconfig.json found at or above {}",
                path.display(),
            ));
        }
    };

    let cmd = resolve_tsc_command(&project_root, binary_in_dirs("npx", &path_dirs()));
    let output = match Command::new(&cmd.program)
        .args(cmd.prefix_args)
        .arg("--noEmit")
        .arg("--pretty=false")
        .current_dir(&project_root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
    {
        Ok(o) => o,
        Err(e) => {
            return spawn_failure(
                "tsc",
                "install TypeScript locally (npm install -D typescript) or globally \
                 (npm install -g typescript)",
                &e,
            );
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut diagnostics = Vec::new();
    for line in stdout.lines() {
        if let Some(diag) = parse_tsc_line(line, &project_root) {
            diagnostics.push(diag);
        }
    }

    VerifierResult::Checked { diagnostics }
}

fn find_typescript_project_root(start: &Path) -> Option<PathBuf> {
    let mut current = if start.is_dir() {
        start
    } else {
        start.parent()?
    };
    loop {
        if current.join("tsconfig.json").exists() {
            return Some(current.to_path_buf());
        }
        current = current.parent()?;
    }
}

fn parse_tsc_line(line: &str, project_root: &Path) -> Option<Diagnostic> {
    let paren_open = line.find('(')?;
    let paren_close = line.find("):")?;
    if paren_close <= paren_open {
        return None;
    }
    let file_part = &line[..paren_open];
    let location = &line[paren_open + 1..paren_close];
    let after_paren = &line[paren_close + 2..];

    let trimmed = after_paren.trim_start();
    if !trimmed.starts_with("error ") {
        return None;
    }
    let after_error = &trimmed[6..];

    let colon_idx = after_error.find(':')?;
    let code_str = &after_error[..colon_idx];
    let message = after_error[colon_idx + 1..].trim().to_string();

    let mut parts = location.split(',');
    let line_num: u32 = parts.next()?.parse().ok()?;
    let col_num: Option<u32> = parts.next().and_then(|s| s.parse().ok());

    let path = project_root.join(file_part.trim());

    Some(Diagnostic {
        path,
        line: line_num,
        column: col_num,
        severity: Severity::Error,
        message,
        code: Some(code_str.to_string()),
    })
}

async fn verify_python(path: &Path) -> VerifierResult {
    // Prefer the project root (so ruff lints all sibling files
    // in one invocation, matching cargo/tsc semantics). Falls
    // back to the file itself when no pyproject.toml/setup.py
    // ancestor exists.
    let target = find_ancestor_with(path, "pyproject.toml")
        .or_else(|| find_ancestor_with(path, "setup.py"))
        .unwrap_or_else(|| path.to_path_buf());
    let path_str = target.to_string_lossy().to_string();

    let output = match Command::new("ruff")
        .arg("check")
        .arg("--output-format=json")
        .arg(&path_str)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
    {
        Ok(o) => o,
        Err(e) => {
            return spawn_failure("ruff", "install ruff (pip install ruff)", &e);
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let diagnostics = parse_ruff_output(&stdout);

    VerifierResult::Checked { diagnostics }
}

fn parse_ruff_output(stdout: &str) -> Vec<Diagnostic> {
    let value: serde_json::Value = match serde_json::from_str(stdout) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let array = match value.as_array() {
        Some(a) => a,
        None => return Vec::new(),
    };

    let mut diagnostics = Vec::new();
    for entry in array {
        let code = entry.get("code").and_then(|v| v.as_str()).map(String::from);
        let message = entry
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let filename = entry.get("filename").and_then(|v| v.as_str()).unwrap_or("");
        let location = entry.get("location");
        let row = location
            .and_then(|l| l.get("row"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        let column = location
            .and_then(|l| l.get("column"))
            .and_then(|v| v.as_u64())
            .map(|c| c as u32);

        diagnostics.push(Diagnostic {
            path: PathBuf::from(filename),
            line: row,
            column,
            severity: Severity::Error,
            message,
            code,
        });
    }
    diagnostics
}

#[cfg(test)]
#[path = "verifier_tests.rs"]
mod tests;
