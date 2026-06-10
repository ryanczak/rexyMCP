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

/// One parsed test failure from a `---- <name> stdout ----` block.
#[derive(Debug, PartialEq)]
struct TestFailure {
    /// Test path from the header, e.g. `tests::it_adds`.
    name: String,
    /// Source location from `panicked at <loc>:`, e.g. `src/lib.rs:6:20`.
    location: Option<String>,
    /// Distilled detail: assertion line plus `left`/`right` values, or the
    /// custom message / panic string.
    detail: String,
}

/// Parse libtest failure blocks from normalized `cargo test` output.
/// Returns one `TestFailure` per `---- <name> stdout ----` block.
fn parse_test_failures(normalized: &str) -> Vec<TestFailure> {
    let mut failures = Vec::new();
    let mut current: Option<TestFailure> = None;
    let mut past_panic_line = false;

    for line in normalized.lines() {
        let trimmed = line.trim_start();

        // Detect a new test-failure block header.
        if trimmed.starts_with("---- ") && trimmed.ends_with(" stdout ----") {
            // Flush any previous block.
            if let Some(prev) = current.take() {
                failures.push(prev);
            }
            let name = trimmed
                .strip_prefix("---- ")
                .unwrap() // starts_with guaranteed the prefix
                .strip_suffix(" stdout ----")
                .unwrap() // ends_with guaranteed the suffix
                .to_string();
            current = Some(TestFailure {
                name,
                location: None,
                detail: String::new(),
            });
            past_panic_line = false;
            continue;
        }

        // If we are not inside a block, skip.
        let Some(ref mut failure) = current else {
            continue;
        };

        // Block terminators: next header (handled above), `failures:`, or
        // `test result:`.
        if trimmed == "failures:" || trimmed.starts_with("test result:") {
            failures.push(current.take().unwrap());
            past_panic_line = false;
            continue;
        }

        // Capture location from the `panicked at` line.
        if !past_panic_line && trimmed.contains("panicked at ") {
            let after = trimmed.split("panicked at ").nth(1).unwrap(); // contains guaranteed the substring
            let loc = after.strip_suffix(':').unwrap_or(after).to_string();
            failure.location = Some(loc);
            past_panic_line = true;
            continue;
        }

        // After the panic line, collect detail lines.
        if past_panic_line {
            // Skip `note:` lines and blank / collapsed-blank lines.
            if trimmed.starts_with("note:") {
                continue;
            }
            if trimmed.is_empty() || trimmed.starts_with(" (x") {
                continue;
            }
            // Append with a separator if detail is non-empty.
            if !failure.detail.is_empty() {
                failure.detail.push_str(", ");
            }
            // Strip leading whitespace from detail lines (libtest indents
            // `left:`/`right:`).
            failure.detail.push_str(trimmed);
        }
    }

    // Flush the last block if any.
    if let Some(last) = current {
        failures.push(last);
    }

    failures
}

/// Format a compact digest of test failures. Returns `""` when `failures` is
/// empty.
fn format_failure_digest(failures: &[TestFailure]) -> String {
    if failures.is_empty() {
        return String::new();
    }

    let mut out = String::new();
    out.push_str(&format!("=== Test failures ({}) ===\n", failures.len()));
    for f in failures {
        out.push_str("test ");
        out.push_str(&f.name);
        out.push_str(" failed");
        if let Some(ref loc) = f.location {
            out.push_str(" at ");
            out.push_str(loc);
            out.push_str(" \u{2014} ");
        } else {
            out.push_str(" \u{2014} ");
        }
        out.push_str(&f.detail);
        out.push('\n');
    }
    out.push('\n');
    out
}

/// Returns `true` when `command` is a `cargo` invocation. Matches `cargo`
/// standing alone or followed by a space (i.e. `cargo <subcommand>`). Leading
/// whitespace is stripped. Does not match `echo cargo` or `CARGO_HOME=…`.
pub fn is_cargo_command(command: &str) -> bool {
    let t = command.trim_start();
    t == "cargo" || t.starts_with("cargo ")
}

/// Filter cargo subcommand output, keeping only diagnostic content: error and
/// warning blocks (with their multi-line spans), test-failure blocks (panic,
/// assertion, stdout headers), and the final summary line. Everything else —
/// passing-test lines, progress messages (`Compiling`, `Checking`, `Finished`,
/// etc.) — is dropped.
///
/// Unknown lines (not matching any keep or drop pattern) are kept by default —
/// the keep-by-default rule ensures no diagnostic content is silently lost.
///
/// After filtering, if the result still exceeds `LINE_CAP` lines, the full
/// filtered output is written to a recovery file via `compact_with_recovery`.
/// Returns `(body, truncated)`.
pub fn cargo_filter(raw: &str, project_root: &Path) -> (String, bool) {
    let normalized = normalize(raw);
    let digest = format_failure_digest(&parse_test_failures(&normalized));
    let mut kept = String::new();
    let mut last_was_blank = true; // suppress leading blank lines

    for line in normalized.lines() {
        let trimmed = line.trim_start();

        if is_cargo_noise(trimmed) {
            continue;
        }

        // Collapse runs of blank lines to at most one.
        if trimmed.is_empty() {
            if !last_was_blank {
                kept.push('\n');
                last_was_blank = true;
            }
            continue;
        }

        last_was_blank = false;
        kept.push_str(line);
        kept.push('\n');
    }

    // Strip trailing blank line left by the collapse above.
    let kept = kept.trim_end_matches('\n');
    let kept = if kept.is_empty() {
        String::new()
    } else {
        format!("{kept}\n")
    };

    let (body, truncated) = if kept.lines().count() <= LINE_CAP {
        (kept, false)
    } else {
        compact_with_recovery(&kept, project_root)
    };

    (format!("{digest}{body}"), truncated)
}

/// Returns `true` for lines that are pure cargo progress noise: passing tests,
/// compilation progress, and other lines that carry no diagnostic information.
fn is_cargo_noise(trimmed: &str) -> bool {
    // Passing test line: "test foo::bar ... ok"
    if trimmed.starts_with("test ") && trimmed.ends_with(" ... ok") {
        return true;
    }
    // Cargo progress tokens (leading whitespace already stripped).
    for prefix in &[
        "Compiling ",
        "Checking ",
        "Finished ",
        "Running ",
        "Downloaded ",
        "Downloading ",
        "Blocking ",
        "Updating ",
        "Locking ",
        "Fresh ",
        "Dirty ",
        "Replaced ",
        "Unpacking ",
    ] {
        if trimmed.starts_with(prefix) {
            return true;
        }
    }
    // "running N test(s)" header line from libtest.
    if trimmed.starts_with("running ") && trimmed.contains(" test") {
        return true;
    }
    false
}

/// Dispatch to the appropriate output filter based on the command string.
/// Cargo commands are routed to the structured `cargo_filter`; everything else
/// falls back to the generic `compact_with_recovery`.
pub fn filter_for_command(command: &str, raw: &str, project_root: &Path) -> (String, bool) {
    if is_cargo_command(command) {
        cargo_filter(raw, project_root)
    } else {
        compact_with_recovery(raw, project_root)
    }
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

    #[test]
    fn is_cargo_command_matches_cargo_subcommands() {
        assert!(is_cargo_command("cargo test"));
        assert!(is_cargo_command("cargo build"));
        assert!(is_cargo_command("  cargo clippy --all-targets"));
        assert!(is_cargo_command("cargo"));
        assert!(!is_cargo_command("echo cargo"));
        assert!(!is_cargo_command("rustc main.rs"));
        assert!(!is_cargo_command(""));
        assert!(!is_cargo_command("CARGO_HOME=/foo cargo test"));
    }

    #[test]
    fn cargo_filter_drops_passing_test_lines() {
        let dir = tempfile::tempdir().unwrap();
        let input = "test foo::bar ... ok\ntest baz ... FAILED\n";
        let (body, _truncated) = cargo_filter(input, dir.path());
        assert!(
            !body.contains(" ... ok"),
            "passing test line should be dropped: {body}"
        );
        assert!(
            body.contains("FAILED"),
            "failing test line should be kept: {body}"
        );
    }

    #[test]
    fn cargo_filter_drops_compiling_noise() {
        let dir = tempfile::tempdir().unwrap();
        let input = "   Compiling foo v1.0\n    Finished dev [unoptimized] target(s) in 1.19s\n     Running unittests (target/debug/deps/foo-abc)\n";
        let (body, _truncated) = cargo_filter(input, dir.path());
        assert!(
            !body.contains("Compiling"),
            "Compiling line should be dropped: {body}"
        );
        assert!(
            !body.contains("Finished"),
            "Finished line should be dropped: {body}"
        );
        assert!(
            !body.contains("Running"),
            "Running line should be dropped: {body}"
        );
    }

    #[test]
    fn cargo_filter_keeps_error_diagnostic_block() {
        let dir = tempfile::tempdir().unwrap();
        let input = "error[E0425]: cannot find value `x`\n  --> src/main.rs:10:5\n   |\n10 |     x\n   |     ^ not found\n";
        let (body, _truncated) = cargo_filter(input, dir.path());
        assert!(
            body.contains("error[E0425]"),
            "error line should survive: {body}"
        );
        assert!(
            body.contains("--> src/main.rs:10:5"),
            "span line should survive: {body}"
        );
        assert!(body.contains("|"), "span detail should survive: {body}");
        assert!(
            body.contains("not found"),
            "diagnostic message should survive: {body}"
        );
    }

    #[test]
    fn cargo_filter_keeps_test_failure_block() {
        let dir = tempfile::tempdir().unwrap();
        let input = "\
test my_test ... FAILED

---- my_test stdout ----

thread 'my_test' panicked at 'assertion failed', src/lib.rs:5

failures:

    my_test

test result: FAILED. 0 passed; 1 failed
";
        let (body, _truncated) = cargo_filter(input, dir.path());
        assert!(
            body.contains("test my_test ... FAILED"),
            "failure line should survive: {body}"
        );
        assert!(
            body.contains("---- my_test stdout ----"),
            "stdout header should survive: {body}"
        );
        assert!(
            body.contains("panicked at"),
            "panic line should survive: {body}"
        );
        assert!(
            body.contains("failures:"),
            "failures header should survive: {body}"
        );
        assert!(
            body.contains("test result: FAILED"),
            "summary should survive: {body}"
        );
    }

    #[test]
    fn cargo_filter_keeps_summary_line() {
        let dir = tempfile::tempdir().unwrap();
        let input = "test result: ok. 42 passed; 0 failed; 0 ignored\n";
        let (body, _truncated) = cargo_filter(input, dir.path());
        assert!(
            body.contains("test result: ok. 42 passed; 0 failed; 0 ignored"),
            "summary line should be present: {body}"
        );
    }

    #[test]
    fn cargo_filter_uses_compact_when_filtered_output_still_long() {
        let dir = tempfile::tempdir().unwrap();
        // Build input with >100 distinct error blocks (each 2 lines, not noise)
        let mut input = String::new();
        for i in 0..60 {
            input.push_str(&format!(
                "error[E{:03}]: error number {}\n  --> src/lib.rs:{}:1\n",
                i, i, i
            ));
        }
        let (body, truncated) = cargo_filter(&input, dir.path());
        assert!(
            truncated,
            "filtered output exceeding LINE_CAP should trigger compact"
        );
        // Recovery file should exist
        let recovery_dir = dir.path().join(".rexymcp/output");
        assert!(recovery_dir.exists(), "recovery dir should exist");
        assert!(
            body.contains("omitted"),
            "elision marker should be present: {body}"
        );
    }

    #[test]
    fn filter_for_command_routes_cargo_to_structured_filter() {
        let dir = tempfile::tempdir().unwrap();
        let input = "test foo::bar ... ok\nCompiling foo v1.0\ntest baz ... FAILED\ntest result: FAILED. 0 passed; 1 failed\n";
        let (body, _truncated) = filter_for_command("cargo test", input, dir.path());
        assert!(
            !body.contains(" ... ok"),
            "passing test noise should be dropped by cargo filter: {body}"
        );
        assert!(
            !body.contains("Compiling"),
            "compiling noise should be dropped by cargo filter: {body}"
        );
        assert!(body.contains("FAILED"), "failure should be kept: {body}");
    }

    #[test]
    fn filter_for_command_routes_non_cargo_to_generic() {
        let dir = tempfile::tempdir().unwrap();
        let mut input = String::new();
        for i in 0..200 {
            input.push_str(&format!("line {i}\n"));
        }
        let (body, truncated) = filter_for_command("make build", &input, dir.path());
        assert!(
            truncated,
            "non-cargo long output should be truncated by generic filter"
        );
        assert!(
            body.contains("omitted"),
            "elision marker should be present: {body}"
        );
        // Generic filter does NOT drop "test ... ok" lines — they survive
        // because the generic filter doesn't know about cargo patterns.
    }

    // --- Test failure digest tests ---

    const FIXTURE_FAIL: &str = r#"running 4 tests
test tests::it_adds ... FAILED
test tests::it_bools ... FAILED
test tests::it_panics ... FAILED
test tests::it_passes ... ok

failures:

---- tests::it_adds stdout ----

thread 'tests::it_adds' (3787800) panicked at src/lib.rs:6:20:
assertion `left == right` failed
  left: 4
 right: 5
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace

---- tests::it_bools stdout ----

thread 'tests::it_bools' (3787801) panicked at src/lib.rs:8:21:
sum too small

---- tests::it_panics stdout ----

thread 'tests::it_panics' (3787802) panicked at src/lib.rs:10:22:
boom 7


failures:
    tests::it_adds
    tests::it_bools
    tests::it_panics

test result: FAILED. 1 passed; 3 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s"#;

    const FIXTURE_PASS: &str = r#"running 2 tests
test tests::a ... ok
test tests::b ... ok

test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s"#;

    #[test]
    fn parse_test_failures_extracts_all_failed_tests() {
        let failures = parse_test_failures(FIXTURE_FAIL);
        assert_eq!(failures.len(), 3);

        let adds = &failures[0];
        assert_eq!(adds.name, "tests::it_adds");
        assert_eq!(adds.location, Some("src/lib.rs:6:20".into()));
        assert!(
            adds.detail.contains("left: 4") && adds.detail.contains("right: 5"),
            "it_adds detail should contain left/right: {}",
            adds.detail
        );

        let bools = &failures[1];
        assert_eq!(bools.name, "tests::it_bools");
        assert_eq!(bools.location, Some("src/lib.rs:8:21".into()));
        assert!(
            bools.detail.contains("sum too small"),
            "it_bools detail: {}",
            bools.detail
        );

        let panics = &failures[2];
        assert_eq!(panics.name, "tests::it_panics");
        assert_eq!(panics.location, Some("src/lib.rs:10:22".into()));
        assert!(
            panics.detail.contains("boom 7"),
            "it_panics detail: {}",
            panics.detail
        );
    }

    #[test]
    fn parse_test_failures_empty_on_passing_output() {
        let failures = parse_test_failures(FIXTURE_PASS);
        assert!(
            failures.is_empty(),
            "passing output should yield no failures"
        );
    }

    #[test]
    fn parse_test_failures_preserves_left_right_labels() {
        let failures = parse_test_failures(FIXTURE_FAIL);
        let adds = &failures[0];
        assert!(
            adds.detail.contains("left"),
            "detail should contain 'left': {}",
            adds.detail
        );
        assert!(
            adds.detail.contains("right"),
            "detail should contain 'right': {}",
            adds.detail
        );
        assert!(
            !adds.detail.contains("expected"),
            "detail must not relabel to 'expected': {}",
            adds.detail
        );
        assert!(
            !adds.detail.contains("actual"),
            "detail must not relabel to 'actual': {}",
            adds.detail
        );
    }

    #[test]
    fn format_failure_digest_empty_for_no_failures() {
        let digest = format_failure_digest(&[]);
        assert_eq!(digest, "");
    }

    #[test]
    fn cargo_filter_prepends_failure_digest() {
        let dir = tempfile::tempdir().unwrap();
        let (body, _truncated) = cargo_filter(FIXTURE_FAIL, dir.path());
        assert!(
            body.starts_with("=== Test failures (3) ==="),
            "body should start with digest header: {}",
            body
        );
        assert!(
            body.contains("tests::it_adds"),
            "digest should contain test name"
        );
        assert!(
            body.contains("panicked at"),
            "verbose failure blocks should still be present below the digest: {}",
            body
        );
    }

    #[test]
    fn cargo_filter_no_digest_on_passing_output() {
        let dir = tempfile::tempdir().unwrap();
        let (body, _truncated) = cargo_filter(FIXTURE_PASS, dir.path());
        assert!(
            !body.contains("=== Test failures"),
            "passing output must not contain digest header: {}",
            body
        );
        assert!(
            body.contains("test result: ok."),
            "passing test summary should be preserved: {}",
            body
        );
    }

    #[test]
    fn parse_test_failures_handles_bare_panic_without_left_right() {
        let fixture = r#"running 1 test
test tests::bare_panic ... FAILED

failures:

---- tests::bare_panic stdout ----

thread 'tests::bare_panic' (12345) panicked at src/lib.rs:42:5:
something went wrong


failures:
    tests::bare_panic

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s"#;

        let failures = parse_test_failures(fixture);
        assert_eq!(failures.len(), 1);
        let f = &failures[0];
        assert_eq!(f.name, "tests::bare_panic");
        assert_eq!(f.location, Some("src/lib.rs:42:5".into()));
        assert!(
            f.detail.contains("something went wrong"),
            "detail should contain panic message: {}",
            f.detail
        );
        assert!(
            !f.detail.contains("left") && !f.detail.contains("right"),
            "no fabricated left/right: {}",
            f.detail
        );
    }
}
