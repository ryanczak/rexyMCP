//! Boundary output filtering (M10, Arc A). Normalizes a command's raw output
//! (strip ANSI, collapse consecutive duplicate lines) and, when it is still too
//! long, truncates to head+tail while writing the full normalized output to a
//! rotated recovery file the model can re-read with `read_file`.
//!
//! Diagnostic-preserving by construction: the tail (where errors and panics
//! live) is always kept verbatim, and nothing is discarded without a recovery
//! file to recover it from.

use std::path::Path;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

use regex::Regex;

/// Output longer than this (in lines, post-normalization) is truncated. Matches
/// the bash tool's prior threshold so behavior is familiar.
const LINE_CAP: usize = 100;
const HEAD_LINES: usize = 20;
const TAIL_LINES: usize = 80;
/// Recovery files retained per directory before the oldest are pruned.
const MAX_RECOVERY_FILES: usize = 20;
/// Subdirectory under the repo root where recovery files are written.
const RECOVERY_SUBDIR: &str = ".rexymcp/output";

/// Monotonic recovery-file sequence (process-global). Filenames sort by age.
static RECOVERY_SEQ: AtomicU64 = AtomicU64::new(0);

fn ansi_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\x1b\[[0-9;]*[A-Za-z]").expect("valid ANSI regex"))
}

/// Strip ANSI escape sequences, then collapse runs of >= 2 identical consecutive
/// lines into one line plus an ` (xN)` count. Lossless for diagnostics: only
/// color codes and exact consecutive repetition are removed. Pure.
pub fn normalize(raw: &str) -> String {
    let stripped = ansi_re().replace_all(raw, "");
    let mut out = String::new();
    let mut prev: Option<&str> = None;
    let mut run = 0usize;
    let flush = |out: &mut String, line: &str, run: usize| {
        if run >= 2 {
            out.push_str(&format!("{line} (x{run})\n"));
        } else {
            out.push_str(line);
            out.push('\n');
        }
    };
    for line in stripped.lines() {
        match prev {
            Some(p) if p == line => run += 1,
            Some(p) => {
                flush(&mut out, p, run);
                run = 1;
            }
            None => run = 1,
        }
        prev = Some(line);
    }
    if let Some(p) = prev {
        flush(&mut out, p, run);
    }
    out
}

/// Normalize `raw`; if the normalized result still exceeds `LINE_CAP` lines,
/// write the full normalized text to a rotated recovery file under
/// `<project_root>/.rexymcp/output/` and return a head+tail view whose elision
/// marker points at that file (a root-relative path the model can `read_file`).
/// Returns `(body, truncated)`.
///
/// Best-effort recovery: if the recovery write fails, the marker falls back to
/// "full output not retained" and the tool still returns truncated output — a
/// failed write is never an error to the caller.
pub fn compact_with_recovery(raw: &str, project_root: &Path) -> (String, bool) {
    let normalized = normalize(raw);
    let lines: Vec<&str> = normalized.lines().collect();
    if lines.len() <= LINE_CAP {
        return (normalized, false);
    }
    let total = lines.len();
    let omitted = total - HEAD_LINES - TAIL_LINES;

    let marker = match write_recovery(&normalized, project_root) {
        Some(rel) => format!("[… {omitted} lines omitted — full output: {rel} …]"),
        None => format!("[… {omitted} lines omitted; full output not retained …]"),
    };

    let mut result = String::new();
    for line in &lines[..HEAD_LINES] {
        result.push_str(line);
        result.push('\n');
    }
    result.push_str(&marker);
    result.push('\n');
    for line in &lines[total - TAIL_LINES..] {
        result.push_str(line);
        result.push('\n');
    }
    (result, true)
}

/// Write `content` to a fresh recovery file under `<root>/.rexymcp/output/`,
/// prune to `MAX_RECOVERY_FILES`, and return the root-relative path (e.g.
/// `.rexymcp/output/cmd-output-7.log`). `None` on any I/O failure.
fn write_recovery(content: &str, project_root: &Path) -> Option<String> {
    let dir = project_root.join(RECOVERY_SUBDIR);
    std::fs::create_dir_all(&dir).ok()?;
    let seq = RECOVERY_SEQ.fetch_add(1, Ordering::Relaxed);
    let name = format!("cmd-output-{seq}.log");
    std::fs::write(dir.join(&name), content).ok()?;
    prune_recovery(&dir);
    Some(format!("{RECOVERY_SUBDIR}/{name}"))
}

/// Keep at most `MAX_RECOVERY_FILES` `cmd-output-*.log` files, deleting the
/// oldest (lowest sequence). Best-effort.
fn prune_recovery(dir: &Path) {
    let mut files: Vec<_> = match std::fs::read_dir(dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("cmd-output-") && n.ends_with(".log"))
            })
            .collect(),
        Err(_) => return,
    };
    if files.len() <= MAX_RECOVERY_FILES {
        return;
    }
    // Sort by the numeric sequence so the oldest prune first (lexical sort would
    // put cmd-output-10 before cmd-output-2).
    files.sort_by_key(|p| {
        p.file_stem()
            .and_then(|s| s.to_str())
            .and_then(|s| s.rsplit('-').next())
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0)
    });
    let excess = files.len() - MAX_RECOVERY_FILES;
    for p in files.into_iter().take(excess) {
        let _ = std::fs::remove_file(p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_strips_ansi_escape_sequences() {
        let input = "\x1b[31mred text\x1b[0m normal";
        let out = normalize(input);
        assert!(!out.contains('\x1b'), "no escape bytes should remain");
        assert!(out.contains("red text"), "text content should be intact");
        assert!(out.contains("normal"), "non-colored text should be intact");
    }

    #[test]
    fn normalize_collapses_consecutive_duplicate_lines() {
        let input = "same\nsame\nsame\nother";
        let out = normalize(input);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 2, "should be 2 lines after collapse");
        assert!(
            lines[0].ends_with("(x3)"),
            "first line should have (x3) count: {}",
            lines[0]
        );
        assert_eq!(lines[1], "other");
    }

    #[test]
    fn normalize_does_not_collapse_nonconsecutive_duplicates() {
        let input = "a\nb\na";
        let out = normalize(input);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 3, "non-consecutive duplicates stay separate");
        assert!(!out.contains("(x"), "no (xN) count should appear");
    }

    #[test]
    fn normalize_leaves_distinct_lines_verbatim() {
        let input = "one\ntwo\nthree";
        let out = normalize(input);
        assert_eq!(out, "one\ntwo\nthree\n");
    }

    #[test]
    fn compact_returns_short_output_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let mut lines = String::new();
        for i in 0..50 {
            lines.push_str(&format!("line {i}\n"));
        }
        let (body, truncated) = compact_with_recovery(&lines, dir.path());
        assert!(!truncated, "short output should not be truncated");
        // normalized output should equal input (no ANSI, no duplicates)
        assert_eq!(body, lines);
        // no .rexymcp directory created
        assert!(
            !dir.path().join(".rexymcp").exists(),
            "no recovery dir for short output"
        );
    }

    #[test]
    fn compact_truncates_long_output_keeping_head_and_tail() {
        let dir = tempfile::tempdir().unwrap();
        let mut lines = String::new();
        for i in 0..200 {
            lines.push_str(&format!("line {i}\n"));
        }
        let (body, truncated) = compact_with_recovery(&lines, dir.path());
        assert!(truncated, "long output should be truncated");
        assert!(body.contains("line 0"), "head should be present");
        assert!(body.contains("line 199"), "tail should be present");
        assert!(body.contains("omitted"), "elision marker should be present");
    }

    #[test]
    fn compact_writes_recovery_file_referenced_by_marker() {
        let dir = tempfile::tempdir().unwrap();
        let mut lines = String::new();
        for i in 0..200 {
            lines.push_str(&format!("line {i}\n"));
        }
        let (body, _truncated) = compact_with_recovery(&lines, dir.path());

        // Marker should reference a recovery file
        assert!(
            body.contains("full output: .rexymcp/output/cmd-output-"),
            "marker should reference recovery file: {}",
            body
        );

        // Recovery file should exist
        let recovery_dir = dir.path().join(".rexymcp/output");
        assert!(recovery_dir.exists(), "recovery dir should exist");
        let files: Vec<_> = std::fs::read_dir(&recovery_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert!(!files.is_empty(), "at least one recovery file should exist");

        // Recovery file contents should equal the full normalized output
        let recovery_path = files[0].path();
        let recovery_content = std::fs::read_to_string(&recovery_path).unwrap();
        assert_eq!(
            recovery_content, lines,
            "recovery file should contain full normalized output"
        );
    }

    #[test]
    fn compact_preserves_trailing_diagnostic_line() {
        let dir = tempfile::tempdir().unwrap();
        let mut input = String::new();
        for i in 0..199 {
            input.push_str(&format!("line {i}\n"));
        }
        input.push_str("error[E0425]: cannot find value `x` in this scope\n");
        let (body, _truncated) = compact_with_recovery(&input, dir.path());
        assert!(
            body.contains("error[E0425]: cannot find value `x` in this scope"),
            "diagnostic line should survive verbatim in tail"
        );
    }

    #[test]
    fn dedupe_can_drop_long_output_below_truncation_threshold() {
        let dir = tempfile::tempdir().unwrap();
        // ~150 lines that are mostly consecutive duplicates collapsing to < 100
        let mut input = String::new();
        for i in 0..50 {
            // Each distinct line repeated 3 times = 150 lines → 50 after dedup
            let label = format!("line {i}");
            for _ in 0..3 {
                input.push_str(&label);
                input.push('\n');
            }
        }
        let (_body, truncated) = compact_with_recovery(&input, dir.path());
        assert!(
            !truncated,
            "after dedup, output should be below threshold and not truncated"
        );
    }

    #[test]
    fn recovery_rotation_keeps_at_most_max_files() {
        let dir = tempfile::tempdir().unwrap();
        let recovery_dir = dir.path().join(".rexymcp/output");
        std::fs::create_dir_all(&recovery_dir).unwrap();

        // Reset the global sequence counter for this test
        // We can't reset AtomicU64, so we just write enough to exceed MAX_RECOVERY_FILES
        let long_input: String = (0..200).map(|i| format!("line {i}\n")).collect();

        for _ in 0..MAX_RECOVERY_FILES + 5 {
            let _ = compact_with_recovery(&long_input, dir.path());
        }

        let files: Vec<_> = std::fs::read_dir(&recovery_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("cmd-output-") && n.ends_with(".log"))
            })
            .collect();

        assert!(
            files.len() <= MAX_RECOVERY_FILES,
            "should keep at most {} files, found {}",
            MAX_RECOVERY_FILES,
            files.len()
        );

        // The highest-sequence file should be among survivors
        let seqs: Vec<u64> = files
            .iter()
            .filter_map(|p| {
                p.file_stem()
                    .and_then(|s| s.to_str())
                    .and_then(|s| s.rsplit('-').next())
                    .and_then(|s| s.parse::<u64>().ok())
            })
            .collect();
        let max_seq = seqs.iter().max().copied().unwrap_or(0);
        let min_seq = seqs.iter().min().copied().unwrap_or(0);
        // The newest (highest seq) should survive
        assert!(max_seq > min_seq, "newest files should survive rotation");
    }
}
