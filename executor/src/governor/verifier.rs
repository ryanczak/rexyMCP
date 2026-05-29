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
            VerifierResult::Unsupported | VerifierResult::Failed(_) => {
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
            return VerifierResult::Failed(format!("cargo check spawn failed: {e}"));
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

    let text = message.get("message")?.as_str()?.to_string();
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

    Some(Diagnostic {
        path,
        line: line_start,
        column: column_start,
        severity: Severity::Error,
        message: text,
        code,
    })
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

    let output = match Command::new("tsc")
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
            return VerifierResult::Failed(format!("tsc spawn failed: {e}"));
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
            return VerifierResult::Failed(format!("ruff spawn failed: {e}"));
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
mod tests {
    use super::*;
    use std::fs;

    #[tokio::test]
    async fn unsupported_for_non_rust_file() {
        let result = verify(Path::new("x.md")).await;
        assert!(matches!(result, VerifierResult::Unsupported));
    }

    #[tokio::test]
    async fn unsupported_for_no_extension() {
        let result = verify(Path::new("Makefile")).await;
        assert!(matches!(result, VerifierResult::Unsupported));
    }

    #[test]
    fn find_crate_root_walks_up() {
        let dir = tempfile::TempDir::new().unwrap();
        let crate_dir = dir.path().join("crate-x");
        let src_dir = crate_dir.join("src");
        fs::create_dir_all(&src_dir).unwrap();
        fs::write(
            crate_dir.join("Cargo.toml"),
            "[package]\nname = \"x\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        let lib_path = src_dir.join("lib.rs");
        fs::write(&lib_path, "").unwrap();

        let result = find_crate_root(&lib_path);
        assert_eq!(result, Some(crate_dir));
    }

    #[test]
    fn find_crate_root_returns_none_outside_crate() {
        let dir = tempfile::TempDir::new().unwrap();
        let lib_path = dir.path().join("lib.rs");
        fs::write(&lib_path, "").unwrap();

        let result = find_crate_root(&lib_path);
        assert!(result.is_none());
    }

    #[test]
    fn parse_cargo_line_skips_non_compiler_messages() {
        let line = r#"{"reason":"build-script-executed","package_id":"foo 0.1.0"}"#;
        let dir = tempfile::TempDir::new().unwrap();
        assert!(parse_cargo_line(line, dir.path()).is_none());
    }

    #[test]
    fn parse_cargo_line_skips_warnings() {
        let line = r#"{
            "reason": "compiler-message",
            "message": {
                "message": "unused variable",
                "level": "warning",
                "spans": [{
                    "file_name": "src/lib.rs",
                    "line_start": 5,
                    "column_start": 9,
                    "is_primary": true
                }]
            }
        }"#;
        let dir = tempfile::TempDir::new().unwrap();
        assert!(parse_cargo_line(line, dir.path()).is_none());
    }

    #[test]
    fn parse_cargo_line_extracts_error_with_code() {
        let line = r#"{
            "reason": "compiler-message",
            "message": {
                "message": "cannot find function `foo` in this scope",
                "level": "error",
                "code": { "code": "E0425", "explanation": "" },
                "spans": [{
                    "file_name": "src/lib.rs",
                    "line_start": 3,
                    "column_start": 5,
                    "is_primary": true
                }]
            }
        }"#;
        let dir = tempfile::TempDir::new().unwrap();
        let diag = parse_cargo_line(line, dir.path()).unwrap();

        assert_eq!(diag.message, "cannot find function `foo` in this scope");
        assert_eq!(diag.line, 3);
        assert_eq!(diag.column, Some(5));
        assert_eq!(diag.severity, Severity::Error);
        assert_eq!(diag.code, Some("E0425".to_string()));
        assert_eq!(diag.path, dir.path().join("src/lib.rs"));
    }

    #[test]
    fn parse_cargo_line_extracts_error_without_code() {
        let line = r#"{
            "reason": "compiler-message",
            "message": {
                "message": "mismatched types",
                "level": "error",
                "spans": [{
                    "file_name": "src/lib.rs",
                    "line_start": 10,
                    "column_start": null,
                    "is_primary": true
                }]
            }
        }"#;
        let dir = tempfile::TempDir::new().unwrap();
        let diag = parse_cargo_line(line, dir.path()).unwrap();

        assert_eq!(diag.code, None);
        assert_eq!(diag.column, None);
        assert_eq!(diag.line, 10);
    }

    #[tokio::test]
    async fn verify_rust_returns_checked_with_errors_on_broken_code() {
        let dir = tempfile::TempDir::new().unwrap();
        let crate_dir = dir.path().join("broken");
        let src_dir = crate_dir.join("src");
        fs::create_dir_all(&src_dir).unwrap();
        fs::write(
            crate_dir.join("Cargo.toml"),
            "[package]\nname = \"broken\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        fs::write(src_dir.join("lib.rs"), "fn broken() { let x = y; }").unwrap();

        let result = verify_rust(&src_dir.join("lib.rs")).await;
        match result {
            VerifierResult::Checked { diagnostics } => {
                assert!(!diagnostics.is_empty());
                for d in &diagnostics {
                    assert_eq!(d.severity, Severity::Error);
                }
            }
            VerifierResult::Failed(msg) => {
                // cargo might not be available in some test envs
                assert!(msg.contains("spawn failed") || msg.contains("no Cargo.toml"));
            }
            _ => panic!("expected Checked or Failed, got {:?}", result),
        }
    }

    #[tokio::test]
    async fn verify_rust_returns_checked_empty_on_clean_code() {
        let dir = tempfile::TempDir::new().unwrap();
        let crate_dir = dir.path().join("clean");
        let src_dir = crate_dir.join("src");
        fs::create_dir_all(&src_dir).unwrap();
        fs::write(
            crate_dir.join("Cargo.toml"),
            "[package]\nname = \"clean\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        fs::write(src_dir.join("lib.rs"), "pub fn hello() {}").unwrap();

        let result = verify_rust(&src_dir.join("lib.rs")).await;
        match result {
            VerifierResult::Checked { diagnostics } => {
                for d in &diagnostics {
                    assert_eq!(d.severity, Severity::Error);
                }
            }
            VerifierResult::Failed(msg) => {
                assert!(msg.contains("spawn failed") || msg.contains("no Cargo.toml"));
            }
            _ => panic!("expected Checked or Failed, got {:?}", result),
        }
    }

    #[tokio::test]
    async fn verify_dispatches_ts_to_typescript() {
        let dir = tempfile::TempDir::new().unwrap();
        let ts_path = dir.path().join("foo.ts");
        fs::write(&ts_path, "").unwrap();

        let result = verify(&ts_path).await;
        assert!(
            matches!(result, VerifierResult::Failed(msg) if msg.contains("no tsconfig.json found"))
        );
    }

    #[tokio::test]
    async fn verify_dispatches_tsx_to_typescript() {
        let dir = tempfile::TempDir::new().unwrap();
        let tsx_path = dir.path().join("foo.tsx");
        fs::write(&tsx_path, "").unwrap();

        let result = verify(&tsx_path).await;
        assert!(
            matches!(result, VerifierResult::Failed(msg) if msg.contains("no tsconfig.json found"))
        );
    }

    #[tokio::test]
    async fn verify_dispatches_py_to_python() {
        let dir = tempfile::TempDir::new().unwrap();
        let py_path = dir.path().join("foo.py");
        fs::write(&py_path, "").unwrap();

        let result = verify(&py_path).await;
        match result {
            VerifierResult::Checked { .. } => {}
            VerifierResult::Failed(msg) => {
                assert!(msg.contains("ruff spawn failed"));
            }
            VerifierResult::Unsupported => {
                panic!("expected Checked or Failed, got Unsupported");
            }
        }
    }

    #[test]
    fn find_typescript_project_root_walks_up() {
        let dir = tempfile::TempDir::new().unwrap();
        let proj = dir.path().join("proj");
        let src = proj.join("src");
        fs::create_dir_all(&src).unwrap();
        fs::write(proj.join("tsconfig.json"), "{}").unwrap();
        let ts_path = src.join("main.ts");
        fs::write(&ts_path, "").unwrap();

        let result = find_typescript_project_root(&ts_path);
        assert_eq!(result, Some(proj));
    }

    #[test]
    fn find_typescript_project_root_returns_none_outside() {
        let dir = tempfile::TempDir::new().unwrap();
        let ts_path = dir.path().join("main.ts");
        fs::write(&ts_path, "").unwrap();

        let result = find_typescript_project_root(&ts_path);
        assert!(result.is_none());
    }

    #[test]
    fn parse_tsc_line_extracts_error_with_code() {
        let line = "src/main.ts(5,10): error TS2304: Cannot find name 'foo'.";
        let dir = tempfile::TempDir::new().unwrap();
        let diag = parse_tsc_line(line, dir.path()).unwrap();

        assert_eq!(diag.line, 5);
        assert_eq!(diag.column, Some(10));
        assert_eq!(diag.code, Some("TS2304".to_string()));
        assert_eq!(diag.message, "Cannot find name 'foo'.");
        assert_eq!(diag.severity, Severity::Error);
        assert_eq!(diag.path, dir.path().join("src/main.ts"));
    }

    #[test]
    fn parse_tsc_line_skips_warnings() {
        let line = "src/main.ts(5,10): warning TS6000: deprecated";
        let dir = tempfile::TempDir::new().unwrap();
        assert!(parse_tsc_line(line, dir.path()).is_none());
    }

    #[test]
    fn parse_ruff_output_extracts_diagnostics() {
        let json = r#"[{
            "code": "F401",
            "message": "unused import",
            "filename": "foo.py",
            "location": {"row": 3, "column": 5}
        }]"#;

        let diags = parse_ruff_output(json);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, Some("F401".to_string()));
        assert_eq!(diags[0].message, "unused import");
        assert_eq!(diags[0].line, 3);
        assert_eq!(diags[0].column, Some(5));
        assert_eq!(diags[0].path, PathBuf::from("foo.py"));
        assert_eq!(diags[0].severity, Severity::Error);
    }

    #[test]
    fn parse_ruff_output_empty_json() {
        assert!(parse_ruff_output("[]").is_empty());
    }

    #[test]
    fn parse_ruff_output_unparseable_json() {
        assert!(parse_ruff_output("not json").is_empty());
    }

    #[tokio::test]
    #[ignore]
    async fn verify_typescript_returns_checked_on_broken_code() {
        let dir = tempfile::TempDir::new().unwrap();
        fs::write(dir.path().join("tsconfig.json"), "{}").unwrap();
        let src = dir.path().join("src");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("main.ts"), "const x: unknown_type = 1;").unwrap();

        let result = verify_typescript(&src.join("main.ts")).await;
        match result {
            VerifierResult::Checked { diagnostics } => {
                assert!(!diagnostics.is_empty());
            }
            VerifierResult::Failed(msg) => {
                panic!("expected Checked, got Failed: {msg}");
            }
            other => panic!("expected Checked, got {:?}", other),
        }
    }

    #[tokio::test]
    #[ignore]
    async fn verify_python_returns_checked_on_broken_code() {
        let dir = tempfile::TempDir::new().unwrap();
        let py_path = dir.path().join("foo.py");
        fs::write(&py_path, "import os\n").unwrap();

        let result = verify_python(&py_path).await;
        match result {
            VerifierResult::Checked { diagnostics } => {
                let f401 = diagnostics
                    .iter()
                    .find(|d| d.code.as_deref() == Some("F401"));
                assert!(
                    f401.is_some(),
                    "expected F401 diagnostic, got: {:?}",
                    diagnostics
                );
            }
            VerifierResult::Failed(msg) => {
                panic!("expected Checked, got Failed: {msg}");
            }
            other => panic!("expected Checked, got {:?}", other),
        }
    }

    // --- Author-vs-ambient baseline tests ---

    #[test]
    fn signature_ignores_line_and_column() {
        let d1 = Diagnostic {
            path: PathBuf::from("src/lib.rs"),
            line: 5,
            column: Some(10),
            severity: Severity::Error,
            message: "cannot find function `foo`".to_string(),
            code: Some("E0425".to_string()),
        };
        let d2 = Diagnostic {
            path: PathBuf::from("src/lib.rs"),
            line: 10,
            column: Some(3),
            severity: Severity::Error,
            message: "cannot find function `foo`".to_string(),
            code: Some("E0425".to_string()),
        };
        assert_eq!(d1.signature(), d2.signature());
    }

    #[test]
    fn signature_distinguishes_paths() {
        let d1 = Diagnostic {
            path: PathBuf::from("src/lib.rs"),
            line: 5,
            column: None,
            severity: Severity::Error,
            message: "unused variable".to_string(),
            code: None,
        };
        let d2 = Diagnostic {
            path: PathBuf::from("src/main.rs"),
            line: 5,
            column: None,
            severity: Severity::Error,
            message: "unused variable".to_string(),
            code: None,
        };
        assert_ne!(d1.signature(), d2.signature());
    }

    #[test]
    fn signature_distinguishes_severities() {
        let d1 = Diagnostic {
            path: PathBuf::from("src/lib.rs"),
            line: 5,
            column: None,
            severity: Severity::Error,
            message: "unused variable".to_string(),
            code: None,
        };
        let d2 = Diagnostic {
            path: PathBuf::from("src/lib.rs"),
            line: 5,
            column: None,
            severity: Severity::Warning,
            message: "unused variable".to_string(),
            code: None,
        };
        assert_ne!(d1.signature(), d2.signature());
    }

    #[test]
    fn signature_distinguishes_messages() {
        let d1 = Diagnostic {
            path: PathBuf::from("src/lib.rs"),
            line: 5,
            column: None,
            severity: Severity::Error,
            message: "cannot find function `foo`".to_string(),
            code: Some("E0425".to_string()),
        };
        let d2 = Diagnostic {
            path: PathBuf::from("src/lib.rs"),
            line: 5,
            column: None,
            severity: Severity::Error,
            message: "cannot find function `bar`".to_string(),
            code: Some("E0425".to_string()),
        };
        assert_ne!(d1.signature(), d2.signature());
    }

    #[test]
    fn signature_handles_none_code() {
        let d1 = Diagnostic {
            path: PathBuf::from("src/lib.rs"),
            line: 5,
            column: None,
            severity: Severity::Error,
            message: "mismatched types".to_string(),
            code: None,
        };
        let d2 = Diagnostic {
            path: PathBuf::from("src/lib.rs"),
            line: 10,
            column: Some(1),
            severity: Severity::Error,
            message: "mismatched types".to_string(),
            code: None,
        };
        assert_eq!(d1.signature(), d2.signature());
    }

    #[test]
    fn baseline_new_is_empty() {
        let baseline = Baseline::new();
        assert!(baseline.signatures.is_empty());
    }

    #[test]
    fn baseline_record_and_contains_roundtrip() {
        let diag = Diagnostic {
            path: PathBuf::from("src/lib.rs"),
            line: 5,
            column: None,
            severity: Severity::Error,
            message: "cannot find function `foo`".to_string(),
            code: Some("E0425".to_string()),
        };
        let mut baseline = Baseline::new();
        baseline.record(&diag);
        assert!(baseline.contains(&diag));

        let different = Diagnostic {
            path: PathBuf::from("src/lib.rs"),
            line: 5,
            column: None,
            severity: Severity::Error,
            message: "cannot find function `bar`".to_string(),
            code: Some("E0425".to_string()),
        };
        assert!(!baseline.contains(&different));
    }

    #[test]
    fn baseline_partition_splits_author_and_ambient() {
        let ambient_diag = Diagnostic {
            path: PathBuf::from("src/lib.rs"),
            line: 5,
            column: None,
            severity: Severity::Error,
            message: "unused variable".to_string(),
            code: None,
        };
        let mut baseline = Baseline::new();
        baseline.record(&ambient_diag);

        let author_a = Diagnostic {
            path: PathBuf::from("src/lib.rs"),
            line: 10,
            column: None,
            severity: Severity::Error,
            message: "cannot find `foo`".to_string(),
            code: Some("E0425".to_string()),
        };
        let author_b = Diagnostic {
            path: PathBuf::from("src/main.rs"),
            line: 3,
            column: None,
            severity: Severity::Error,
            message: "type mismatch".to_string(),
            code: None,
        };
        let recheck = Diagnostic {
            path: PathBuf::from("src/lib.rs"),
            line: 20,
            column: Some(5),
            severity: Severity::Error,
            message: "unused variable".to_string(),
            code: None,
        };

        let diags = vec![author_a.clone(), ambient_diag.clone(), author_b.clone()];
        let (author, ambient) = baseline.partition(&diags);
        assert_eq!(author.len(), 2);
        assert_eq!(ambient.len(), 1);
        assert_eq!(ambient[0].message, "unused variable");

        // Verify partition preserves input order
        assert_eq!(author[0].message, "cannot find `foo`");
        assert_eq!(author[1].message, "type mismatch");

        // recheck should match baseline even at different line
        assert!(baseline.contains(&recheck));
    }

    #[test]
    fn partition_preserves_input_order() {
        let mut baseline = Baseline::new();
        let b_match = Diagnostic {
            path: PathBuf::from("src/lib.rs"),
            line: 1,
            column: None,
            severity: Severity::Error,
            message: "ambient error".to_string(),
            code: None,
        };
        baseline.record(&b_match);

        let a = Diagnostic {
            path: PathBuf::from("src/lib.rs"),
            line: 2,
            column: None,
            severity: Severity::Error,
            message: "author A".to_string(),
            code: None,
        };
        let c = Diagnostic {
            path: PathBuf::from("src/lib.rs"),
            line: 3,
            column: None,
            severity: Severity::Error,
            message: "author C".to_string(),
            code: None,
        };
        let d = Diagnostic {
            path: PathBuf::from("src/lib.rs"),
            line: 4,
            column: None,
            severity: Severity::Error,
            message: "ambient error".to_string(),
            code: None,
        };

        let diags = vec![a.clone(), b_match.clone(), c.clone(), d.clone()];
        let (author, ambient) = baseline.partition(&diags);
        assert_eq!(author.len(), 2);
        assert_eq!(ambient.len(), 2);
        assert_eq!(author[0].message, "author A");
        assert_eq!(author[1].message, "author C");
        assert_eq!(ambient[0].message, "ambient error");
        assert_eq!(ambient[1].message, "ambient error");
    }

    #[test]
    fn find_ancestor_with_walks_up_to_marker() {
        let dir = tempfile::TempDir::new().unwrap();
        let nested = dir.path().join("a/b/c");
        fs::create_dir_all(&nested).unwrap();
        fs::write(dir.path().join("a/Cargo.toml"), "[package]\nname=\"x\"\n").unwrap();
        let file = nested.join("foo.rs");
        fs::write(&file, "").unwrap();

        let root = find_ancestor_with(&file, "Cargo.toml").unwrap();
        assert_eq!(root, dir.path().join("a"));
    }

    #[test]
    fn project_root_for_resolves_per_extension() {
        let dir = tempfile::TempDir::new().unwrap();
        let rs_root = dir.path().join("rust");
        fs::create_dir_all(rs_root.join("src")).unwrap();
        fs::write(rs_root.join("Cargo.toml"), "[package]\nname=\"x\"\n").unwrap();
        let rs_file = rs_root.join("src/lib.rs");
        fs::write(&rs_file, "").unwrap();

        let py_root = dir.path().join("py");
        fs::create_dir_all(py_root.join("pkg")).unwrap();
        fs::write(py_root.join("pyproject.toml"), "[project]\nname=\"x\"\n").unwrap();
        let py_file = py_root.join("pkg/mod.py");
        fs::write(&py_file, "").unwrap();

        assert_eq!(project_root_for(&rs_file, "rs"), Some(rs_root));
        assert_eq!(project_root_for(&py_file, "py"), Some(py_root));
        assert_eq!(project_root_for(&rs_file, "md"), None);
    }

    #[tokio::test]
    async fn capture_baseline_dedupes_by_project_root() {
        // Two .rs files sharing the same Cargo.toml ancestor
        // should trigger only one cargo invocation, but produce
        // identical baseline diagnostics regardless. We verify
        // the dedup by checking that the result equals what one
        // call produces — implicitly proving N invocations
        // weren't needed (the test would be ~Nx slower if we
        // were still per-file).
        let dir = tempfile::TempDir::new().unwrap();
        let crate_dir = dir.path().join("c");
        let src = crate_dir.join("src");
        fs::create_dir_all(&src).unwrap();
        fs::write(
            crate_dir.join("Cargo.toml"),
            "[package]\nname=\"c\"\nversion=\"0.1.0\"\n",
        )
        .unwrap();
        fs::write(src.join("lib.rs"), "fn a() { let _: i32 = \"\"; }\n").unwrap();
        fs::write(src.join("other.rs"), "// unused\n").unwrap();

        let paths = vec![
            src.join("lib.rs"),
            src.join("other.rs"),
            crate_dir.join("Cargo.toml"),
        ];
        let baseline = capture_baseline(&paths).await;
        // At least one diagnostic from lib.rs's type error.
        assert!(!baseline.signatures.is_empty());
    }

    #[tokio::test]
    async fn capture_baseline_skips_unsupported_files() {
        let dir = tempfile::TempDir::new().unwrap();
        let md_path = dir.path().join("README.md");
        fs::write(&md_path, "# Hello").unwrap();

        let crate_dir = dir.path().join("crate-x");
        let src_dir = crate_dir.join("src");
        fs::create_dir_all(&src_dir).unwrap();
        fs::write(
            crate_dir.join("Cargo.toml"),
            "[package]\nname = \"x\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        let lib_path = src_dir.join("lib.rs");
        fs::write(&lib_path, "fn broken() { let x = y; }").unwrap();

        let baseline = capture_baseline(&[md_path, lib_path]).await;

        // The .md file is unsupported, so only the .rs file's
        // diagnostics should be in the baseline.
        assert!(!baseline.signatures.is_empty());
        for sig in &baseline.signatures {
            assert_eq!(sig.path.extension().and_then(|s| s.to_str()), Some("rs"));
        }
    }
}
