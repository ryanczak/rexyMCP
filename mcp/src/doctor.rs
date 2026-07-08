use rexymcp_executor::config::CommandConfig;
use serde::Serialize;
use std::path::{Path, PathBuf};

/// The binary a configured command shells out to: its first
/// whitespace-delimited token. `None` for a blank/empty command.
pub fn command_binary(command: &str) -> Option<&str> {
    command.split_whitespace().next()
}

/// Resolve a binary against a list of search directories. A name
/// containing a path separator is treated as a path and checked
/// directly; a bare name is probed as `dir.join(name)` in each
/// search dir. Returns the first match that is an existing *file*.
pub fn resolve_binary(binary: &str, search_paths: &[PathBuf]) -> Option<PathBuf> {
    if binary.contains(std::path::MAIN_SEPARATOR) {
        // Treat as a literal path; check directly.
        if Path::new(binary).is_file() {
            return Some(PathBuf::from(binary));
        }
        return None;
    }

    for dir in search_paths {
        let candidate = dir.join(binary);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolStatus {
    pub binary: String,
    pub found: bool,
    pub note: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DoctorReport {
    pub tier0: Vec<ToolStatus>,
    pub tier1: Vec<ToolStatus>,
}

impl DoctorReport {
    /// True iff every required (Tier-0) tool was found. Tier-1
    /// status never affects this — enhancers fail open.
    pub fn tier0_ok(&self) -> bool {
        self.tier0.iter().all(|t| t.found)
    }
}

/// Build the toolchain report from the configured commands and
/// the known per-language verifier enhancers, resolving each
/// binary against `search_paths`.
pub fn build_report(commands: &CommandConfig, search_paths: &[PathBuf]) -> DoctorReport {
    let mut tier0: Vec<ToolStatus> = Vec::new();

    // Walk the six configured commands in fixed order, deduping by binary name.
    let tier0_commands = [
        ("format", commands.format.as_deref()),
        ("build", commands.build.as_deref()),
        ("lint", commands.lint.as_deref()),
        ("test", commands.test.as_deref()),
        ("lint_fix", commands.lint_fix.as_deref()),
        ("format_fix", commands.format_fix.as_deref()),
    ];

    for (role, cmd) in tier0_commands {
        let Some(cmd) = cmd else {
            continue;
        };
        let Some(binary) = command_binary(cmd) else {
            continue;
        };

        if let Some(existing) = tier0.iter_mut().find(|t| t.binary == binary) {
            // Dedup: append role to existing note.
            existing.note.push_str(&format!(", {role}"));
        } else {
            let found = resolve_binary(binary, search_paths).is_some();
            tier0.push(ToolStatus {
                binary: binary.to_string(),
                found,
                note: role.to_string(),
            });
        }
    }

    // Tier 1: always emit all three enhancer rows.
    let tier1_enhancers = [
        (
            "cargo",
            "Rust",
            "install the Rust toolchain via https://rustup.rs",
        ),
        ("tsc", "TypeScript", "npm install -g typescript"),
        ("ruff", "Python", "pip install ruff"),
    ];

    let tier1: Vec<ToolStatus> = tier1_enhancers
        .iter()
        .map(|(binary, lang, remedy)| {
            let found = resolve_binary(binary, search_paths).is_some();
            ToolStatus {
                binary: (*binary).to_string(),
                found,
                note: format!("{lang} ({remedy})"),
            }
        })
        .collect();

    DoctorReport { tier0, tier1 }
}

/// The PATH search directories, or empty if PATH is unset.
fn path_dirs() -> Vec<PathBuf> {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default()
}

/// Build the report against the real PATH, print it (human or JSON),
/// and return whether all required tools were found.
pub fn run(commands: &CommandConfig, json: bool) -> bool {
    let report = build_report(commands, &path_dirs());
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).unwrap_or_else(|e| {
                format!("{{\"error\": \"failed to serialize report: {}\"}}", e)
            })
        );
    } else {
        println!("{}", format_report(&report));
    }
    report.tier0_ok()
}

fn format_report(report: &DoctorReport) -> String {
    let mut out = String::new();

    out.push_str("=== Tier 0 (required) ===\n");
    for tool in &report.tier0 {
        let status = if tool.found { "ok" } else { "MISSING" };
        out.push_str(&format!(
            "  [{status:>7}] {} — {}\n",
            tool.binary, tool.note
        ));
    }

    out.push_str("\n=== Tier 1 (advisory) ===\n");
    for tool in &report.tier1 {
        let status = if tool.found { "ok" } else { "MISSING" };
        out.push_str(&format!(
            "  [{status:>7}] {} — {}\n",
            tool.binary, tool.note
        ));
    }

    if !report.tier0_ok() {
        out.push_str("\nA required tool is missing — fix the above before dispatching a phase.\n");
    } else {
        out.push_str("\nAll required tools are present.\n");
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn command_binary_returns_first_token() {
        assert_eq!(command_binary("cargo +nightly fmt --all"), Some("cargo"));
    }

    #[test]
    fn command_binary_none_for_blank() {
        assert_eq!(command_binary("   "), None);
        assert_eq!(command_binary(""), None);
    }

    #[test]
    fn resolve_binary_finds_file_in_search_dir() {
        let dir = TempDir::new().unwrap();
        std::fs::File::create(dir.path().join("cargo")).unwrap();
        let paths = vec![dir.path().to_path_buf()];

        let result = resolve_binary("cargo", &paths);
        assert!(result.is_some());
    }

    #[test]
    fn resolve_binary_rejects_directory_of_same_name() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir(dir.path().join("cargo")).unwrap();
        let paths = vec![dir.path().to_path_buf()];

        let result = resolve_binary("cargo", &paths);
        assert!(result.is_none());
    }

    #[test]
    fn resolve_binary_is_exact_not_substring() {
        let dir = TempDir::new().unwrap();
        std::fs::File::create(dir.path().join("cargo-clippy")).unwrap();
        let paths = vec![dir.path().to_path_buf()];

        let result = resolve_binary("cargo", &paths);
        assert!(result.is_none());
    }

    #[test]
    fn resolve_binary_absolute_path_checked_directly() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("mytool");
        std::fs::File::create(&file_path).unwrap();

        let result = resolve_binary(file_path.to_str().unwrap(), &[]);
        assert!(result.is_some());
    }

    #[test]
    fn build_report_dedupes_tier0_by_binary() {
        let dir = TempDir::new().unwrap();
        std::fs::File::create(dir.path().join("cargo")).unwrap();
        let paths = vec![dir.path().to_path_buf()];

        let commands = CommandConfig {
            format: Some("cargo fmt --all --check".into()),
            build: Some("cargo build".into()),
            lint: Some("cargo clippy".into()),
            test: Some("cargo test".into()),
            lint_fix: None,
            format_fix: None,
        };

        let report = build_report(&commands, &paths);
        assert_eq!(report.tier0.len(), 1);
        assert_eq!(report.tier0[0].binary, "cargo");
        assert!(report.tier0[0].found);
    }

    #[test]
    fn build_report_skips_unset_commands() {
        let dir = TempDir::new().unwrap();
        std::fs::File::create(dir.path().join("cargo")).unwrap();
        let paths = vec![dir.path().to_path_buf()];

        let commands = CommandConfig {
            format: None,
            build: Some("cargo build".into()),
            lint: None,
            test: None,
            lint_fix: None,
            format_fix: None,
        };

        let report = build_report(&commands, &paths);
        assert_eq!(report.tier0.len(), 1);
        assert_eq!(report.tier0[0].binary, "cargo");
    }

    #[test]
    fn build_report_emits_three_tier1_rows() {
        let commands = CommandConfig::default();
        let report = build_report(&commands, &[]);

        assert_eq!(report.tier1.len(), 3);
        assert_eq!(report.tier1[0].binary, "cargo");
        assert_eq!(report.tier1[1].binary, "tsc");
        assert_eq!(report.tier1[2].binary, "ruff");
    }

    #[test]
    fn tier0_ok_true_when_all_present_ignoring_tier1() {
        let dir = TempDir::new().unwrap();
        std::fs::File::create(dir.path().join("cargo")).unwrap();
        let paths = vec![dir.path().to_path_buf()];

        let commands = CommandConfig {
            format: Some("cargo fmt".into()),
            build: Some("cargo build".into()),
            lint: None,
            test: None,
            lint_fix: None,
            format_fix: None,
        };

        let report = build_report(&commands, &paths);
        // cargo is found (Tier 0), but tsc and ruff are not (Tier 1).
        // tier0_ok must still be true because Tier 1 fails open.
        assert!(report.tier0_ok());
    }

    #[test]
    fn tier0_ok_false_when_a_required_tool_missing() {
        let dir = TempDir::new().unwrap();
        // Don't create any files — nothing will be found.
        let paths = vec![dir.path().to_path_buf()];

        let commands = CommandConfig {
            format: None,
            build: Some("cargo build".into()),
            lint: None,
            test: None,
            lint_fix: None,
            format_fix: None,
        };

        let report = build_report(&commands, &paths);
        assert!(!report.tier0_ok());
    }

    #[test]
    fn format_report_contains_binary_names_and_markers() {
        let dir = TempDir::new().unwrap();
        std::fs::File::create(dir.path().join("cargo")).unwrap();
        let paths = vec![dir.path().to_path_buf()];

        let commands = CommandConfig {
            format: Some("cargo fmt".into()),
            build: Some("cargo build".into()),
            lint: None,
            test: None,
            lint_fix: None,
            format_fix: None,
        };

        let report = build_report(&commands, &paths);
        let rendered = format_report(&report);

        assert!(rendered.contains("cargo"));
        assert!(rendered.contains("tsc"));
        assert!(rendered.contains("ruff"));
        assert!(rendered.contains("ok"));
        assert!(rendered.contains("MISSING"));
        assert!(rendered.contains("Tier 0"));
        assert!(rendered.contains("Tier 1"));
    }

    #[test]
    fn format_report_missing_tier0_shows_warning() {
        let commands = CommandConfig {
            format: None,
            build: Some("nonexistent-build-tool".into()),
            lint: None,
            test: None,
            lint_fix: None,
            format_fix: None,
        };

        let report = build_report(&commands, &[]);
        let rendered = format_report(&report);
        assert!(rendered.contains("required tool is missing"));
    }

    #[test]
    fn format_report_all_present_shows_success() {
        let dir = TempDir::new().unwrap();
        std::fs::File::create(dir.path().join("cargo")).unwrap();
        let paths = vec![dir.path().to_path_buf()];

        let commands = CommandConfig {
            format: Some("cargo fmt".into()),
            build: Some("cargo build".into()),
            lint: None,
            test: None,
            lint_fix: None,
            format_fix: None,
        };

        let report = build_report(&commands, &paths);
        let rendered = format_report(&report);
        assert!(rendered.contains("All required tools are present"));
    }

    #[test]
    fn tier0_note_merges_roles_for_same_binary() {
        let dir = TempDir::new().unwrap();
        std::fs::File::create(dir.path().join("cargo")).unwrap();
        let paths = vec![dir.path().to_path_buf()];

        let commands = CommandConfig {
            format: Some("cargo fmt --all --check".into()),
            build: Some("cargo build".into()),
            lint: Some("cargo clippy".into()),
            test: Some("cargo test".into()),
            lint_fix: None,
            format_fix: None,
        };

        let report = build_report(&commands, &paths);
        assert_eq!(report.tier0.len(), 1);
        let note = &report.tier0[0].note;
        assert!(note.starts_with("format"));
        assert!(note.contains(", build"));
        assert!(note.contains(", lint"));
        assert!(note.contains(", test"));
    }

    #[test]
    fn tier1_note_contains_language_and_remedy() {
        let commands = CommandConfig::default();
        let report = build_report(&commands, &[]);

        // Rust row
        assert!(report.tier1[0].note.contains("Rust"));
        assert!(
            report.tier1[0]
                .note
                .contains("install the Rust toolchain via https://rustup.rs")
        );

        // TypeScript row
        assert!(report.tier1[1].note.contains("TypeScript"));
        assert!(report.tier1[1].note.contains("npm install -g typescript"));

        // Python row
        assert!(report.tier1[2].note.contains("Python"));
        assert!(report.tier1[2].note.contains("pip install ruff"));
    }

    #[test]
    fn resolve_binary_returns_first_matching_dir() {
        let dir1 = TempDir::new().unwrap();
        let dir2 = TempDir::new().unwrap();
        std::fs::File::create(dir1.path().join("tool")).unwrap();
        std::fs::File::create(dir2.path().join("tool")).unwrap();

        let paths = vec![dir1.path().to_path_buf(), dir2.path().to_path_buf()];
        let result = resolve_binary("tool", &paths);

        assert!(result.is_some());
        assert_eq!(result.unwrap(), dir1.path().join("tool"));
    }

    #[test]
    fn resolve_binary_nonexistent_returns_none() {
        let dir = TempDir::new().unwrap();
        let paths = vec![dir.path().to_path_buf()];
        let result = resolve_binary("does-not-exist", &paths);
        assert!(result.is_none());
    }
}
