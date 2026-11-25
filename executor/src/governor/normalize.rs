use std::sync::LazyLock;

use regex::Regex;

use crate::governor::hard_fail::normalize_arguments;

/// Normalize a bash command for repeat/plumbing comparison: collapse whitespace
/// runs, iteratively strip trailing output-plumbing segments (temp-file
/// capture-and-cat, `tail`/`head`/`wc`/`cat` pipes, stream redirects) from the
/// right until stable, and normalize single-vs-double quoting on simple literal
/// tokens.
///
/// Preserved (never stripped): substantive pipeline stages (`| grep ...`,
/// `| awk ...`, `| sort ...`, `| xargs ...`), redirects to non-temp paths
/// (`> src/foo.rs`), and distinct base-command arguments.
pub fn normalize_bash_command(cmd: &str) -> String {
    let mut normalized = collapse_whitespace(cmd);
    loop {
        let next = strip_trailing_plumbing(&normalized);
        if next == normalized {
            break;
        }
        normalized = next;
    }
    normalize_simple_quotes(&normalized)
}

/// Normalize tool-call arguments for repeat comparison. `bash` commands get the
/// full plumbing normalization; every other tool (and every other bash string
/// leaf) gets whitespace-only normalization so distinct file slices and search
/// patterns stay distinct.
pub fn normalize_tool_args_for_repeat(tool: &str, args: &serde_json::Value) -> serde_json::Value {
    if tool != "bash" {
        return normalize_arguments(args);
    }
    match args {
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.iter()
                .map(|(k, v)| match (k.as_str(), v) {
                    ("command", serde_json::Value::String(s)) => (
                        k.clone(),
                        serde_json::Value::String(normalize_bash_command(s)),
                    ),
                    (_, other) => (k.clone(), normalize_arguments(other)),
                })
                .collect(),
        ),
        other => normalize_arguments(other),
    }
}

/// Strip volatile wall-clock timing fields from tool output so identical
/// outcomes with different durations compare equal: the bash status line's
/// elapsed duration (`✓ exit 0 (0.3s)`), and cargo-style timing clauses
/// (`finished in 0.42s`, `Finished ... in 0.12s`). Exit codes, test
/// pass/fail counts, error messages and all other output are preserved.
pub fn normalize_output_for_repeat(output: &str) -> String {
    static STATUS_DURATION: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(✓|✗) (exit \d+|exit signal) \(\d+\.\d+s\)").unwrap());
    static CARGO_DURATION: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\bin \d+\.\d+s\b").unwrap());
    let no_status = STATUS_DURATION.replace_all(output, "$1 $2");
    CARGO_DURATION
        .replace_all(&no_status, "in <t>")
        .into_owned()
}

fn collapse_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Strip one trailing plumbing segment from the right, if present.
pub(crate) fn strip_trailing_plumbing(cmd: &str) -> String {
    static TAIL_PIPE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\s*\|\s*(tail|head|wc|cat)\b.*$").unwrap());
    static TEMP_CAT: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r#"(?:\s*(?:2>&1\s*)?>\s*(?:"([^"]+)"|'([^']+)'|([^\s;&|]+))(?:\s*2>&1)?\s*[;&]\s*cat\s+(?:"([^"]+)"|'([^']+)'|([^\s;&|]+))|\s*\|\s*tee\s+(?:"([^"]+)"|'([^']+)'|([^\s;&|]+)))$"#,
        )
        .unwrap()
    });
    static STDERR_REDIRECT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s*2>&1$").unwrap());
    static DEVNULL_REDIRECT: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\s*(2?>\s*/dev/null|2>/dev/null)$").unwrap());

    if let Some(caps) = TEMP_CAT.captures(cmd) {
        let capture_path = [1usize, 2, 3].iter().filter_map(|g| caps.get(*g)).next();
        let cat_path = [4usize, 5, 6].iter().filter_map(|g| caps.get(*g)).next();
        let tee_path = [7usize, 8, 9].iter().filter_map(|g| caps.get(*g)).next();
        if let (Some(capture), Some(cat)) = (capture_path, cat_path)
            && capture.as_str().trim_matches(['"', '\'']) == cat.as_str().trim_matches(['"', '\''])
        {
            let head = &cmd[..caps.get(0).unwrap().start()];
            return collapse_whitespace(head);
        }
        if let Some(_tee) = tee_path {
            let head = &cmd[..caps.get(0).unwrap().start()];
            let head = head.trim_end_matches('|');
            return collapse_whitespace(head);
        }
    }
    if let Some(m) = TAIL_PIPE.find(cmd) {
        // Only strip when the pipe stage is genuinely trailing (no later
        // redirect handling needed — the redirect strippers run first via the
        // fixed-point loop).
        let head = &cmd[..m.start()];
        if !head.trim_end().ends_with('|') {
            return collapse_whitespace(head);
        }
    }
    if let Some(m) = STDERR_REDIRECT.find(cmd) {
        return collapse_whitespace(&cmd[..m.start()]);
    }
    if let Some(m) = DEVNULL_REDIRECT.find(cmd) {
        return collapse_whitespace(&cmd[..m.start()]);
    }
    cmd.to_string()
}

/// Normalize single vs double quoting on simple literal tokens: replace
/// `'token'` and `"token"` with `token` when the quoted content contains no
/// spaces or quote/pipe/semicolon characters, so quoting-only differences read
/// equal.
fn normalize_simple_quotes(cmd: &str) -> String {
    static SINGLE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"'([^'\s";&|]*)'"#).unwrap());
    static DOUBLE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#""([^"\s';&|]*)""#).unwrap());
    let single = SINGLE.replace_all(cmd, "$1");
    DOUBLE.replace_all(&single, "$1").into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_trailing_tail_head_wc_and_stderr_plumbing() {
        let base = "grep -rn refused src/a.rs";
        for variant in [
            "grep -rn refused src/a.rs | tail -4",
            "grep -rn refused src/a.rs | tail -4 > /tmp/x && cat /tmp/x",
            "grep -rn refused src/a.rs 2>&1 | tail -6",
            "grep -rn refused src/a.rs | head -2",
            "grep -rn refused src/a.rs | wc -l",
            "grep -rn refused src/a.rs > /tmp/x; cat /tmp/x",
        ] {
            assert_eq!(
                normalize_bash_command(variant),
                normalize_bash_command(base),
                "variant: {variant}"
            );
        }
    }

    #[test]
    fn normalizes_simple_quote_differences() {
        assert_eq!(
            normalize_bash_command(r#"grep -rn "refused" src/a.rs | wc -l"#),
            normalize_bash_command("grep -rn 'refused' src/a.rs | head -2")
        );
    }

    #[test]
    fn preserves_substantive_pipes_and_non_temp_redirects() {
        assert_ne!(
            normalize_bash_command("cargo test | grep foo"),
            normalize_bash_command("cargo test | grep bar")
        );
        assert_ne!(
            normalize_bash_command("echo a > src/x.rs"),
            normalize_bash_command("echo a > src/y.rs")
        );
        assert_ne!(
            normalize_bash_command("cargo test foo | tail -4"),
            normalize_bash_command("cargo test bar | tail -4")
        );
    }

    #[test]
    fn tool_args_normalizer_only_strips_plumbing_for_bash() {
        let bash_a = serde_json::json!({"command": "grep -rn refused src/a.rs | wc -l"});
        let bash_b = serde_json::json!({"command": "grep -rn refused src/a.rs | head -2"});
        assert_eq!(
            normalize_tool_args_for_repeat("bash", &bash_a),
            normalize_tool_args_for_repeat("bash", &bash_b)
        );

        let read_a = serde_json::json!({"path": "src/a.rs", "start_line": 1, "end_line": 20});
        let read_b = serde_json::json!({"path": "src/a.rs", "start_line": 21, "end_line": 40});
        assert_ne!(
            normalize_tool_args_for_repeat("read_file", &read_a),
            normalize_tool_args_for_repeat("read_file", &read_b)
        );

        let search_a = serde_json::json!({"pattern": "refused", "path": "src/"});
        let search_b = serde_json::json!({"pattern": "denied", "path": "src/"});
        assert_ne!(
            normalize_tool_args_for_repeat("search", &search_a),
            normalize_tool_args_for_repeat("search", &search_b)
        );
    }

    #[test]
    fn output_normalizer_ignores_bash_status_and_cargo_timings() {
        let a = "✓ exit 0 (0.3s)\n\nall good\n";
        let b = "✓ exit 0 (2.1s)\n\nall good\n";
        assert_eq!(
            normalize_output_for_repeat(a),
            normalize_output_for_repeat(b)
        );

        let c =
            "    Finished `test` profile [unoptimized] target(s) in 0.00s\n1 passed; 0 failed\n";
        let d =
            "    Finished `test` profile [unoptimized] target(s) in 0.45s\n1 passed; 0 failed\n";
        assert_eq!(
            normalize_output_for_repeat(c),
            normalize_output_for_repeat(d)
        );

        let e = "test result: ok. 1 passed; 0 failed; 0 ignored; finished in 0.00s";
        let f = "test result: ok. 1 passed; 0 failed; 0 ignored; finished in 1.42s";
        assert_eq!(
            normalize_output_for_repeat(e),
            normalize_output_for_repeat(f)
        );
    }

    #[test]
    fn output_normalizer_preserves_exit_code_and_test_counts() {
        assert_ne!(
            normalize_output_for_repeat("✓ exit 0 (0.3s)\n\nout\n"),
            normalize_output_for_repeat("✗ exit 1 (0.3s)\n\nout\n")
        );
        assert_ne!(
            normalize_output_for_repeat("1 passed; 0 failed"),
            normalize_output_for_repeat("0 passed; 1 failed")
        );
    }
}
