//! Architect usage harvester — `rexymcp harvest` subcommand.
//!
//! Reads Claude Code session transcripts, sums per-message token usage by class,
//! attributes each message to the `ArchitectActivity` whose journal time-window
//! contains it, and appends an enriched copy (tokens filled) that
//! `fold_activities` overlays at read time.

use std::path::{Path, PathBuf};

use rexymcp_executor::config::Config;
use rexymcp_executor::store::telemetry::{self, ArchitectActivity, ArchitectTokens};

/// Borrowed harvest inputs from the CLI flags.
pub struct HarvestArgs<'a> {
    /// Directory of Claude Code `*.jsonl` session transcripts.
    pub transcript_dir: &'a Path,
    /// Project ID override (defaults to `[project].id` from config).
    pub project_id: Option<&'a str>,
}

/// Result of a harvest run.
pub struct HarvestOutcome {
    pub path: PathBuf,
    /// Distinct messages counted (post-dedup).
    pub messages: usize,
    /// Activities enriched with non-zero tokens (enriched copies appended).
    pub enriched: usize,
    /// Messages that fell after the last activity boundary (unattributed).
    pub unattributed: usize,
}

/// Days from 1970-01-01 to civil date (y, m[1..12], d[1..31]). Howard Hinnant's
/// `days_from_civil`, exact for the proleptic Gregorian calendar. No date crate.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146097 + doe - 719468
}

/// Parse a fixed-format ISO-8601-Zulu timestamp (`2026-07-09T16:00:56.539Z`) to
/// epoch milliseconds. Tolerant of a missing/short/long fractional part. Returns
/// `None` on any structural malformation (caller skips the line).
fn parse_iso_to_epoch_ms(s: &str) -> Option<u64> {
    let s = s.strip_suffix('Z').unwrap_or(s);
    let (date, time) = s.split_once('T')?;
    let mut dp = date.split('-');
    let y: i64 = dp.next()?.parse().ok()?;
    let mo: i64 = dp.next()?.parse().ok()?;
    let d: i64 = dp.next()?.parse().ok()?;
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) {
        return None;
    }
    let (hms, frac) = match time.split_once('.') {
        Some((h, f)) => (h, f),
        None => (time, ""),
    };
    let mut tp = hms.split(':');
    let h: i64 = tp.next()?.parse().ok()?;
    let mi: i64 = tp.next()?.parse().ok()?;
    let sec: i64 = tp.next()?.parse().ok()?;
    // Normalise the fractional part to exactly 3 digits (milliseconds).
    let mut millis_str = String::new();
    for c in frac.chars().take(3) {
        if !c.is_ascii_digit() {
            return None;
        }
        millis_str.push(c);
    }
    while millis_str.len() < 3 {
        millis_str.push('0');
    }
    let millis: i64 = millis_str.parse().ok()?;
    let days = days_from_civil(y, mo, d);
    let total_ms = ((((days * 24 + h) * 60 + mi) * 60 + sec) * 1000) + millis;
    u64::try_from(total_ms).ok()
}

/// One deduped assistant message's usage.
struct Usage {
    ts_ms: u64,
    tokens: ArchitectTokens,
}

/// Read every `*.jsonl` under `dir`, extract assistant-line usage, dedup by
/// `message.id` (first occurrence wins — repeats are byte-identical). A missing
/// dir yields an empty vec (not an error). Lines that are not assistant-with-usage,
/// or whose timestamp/ids are missing/unparseable, are skipped.
fn read_transcript_usages(dir: &Path) -> Vec<Usage> {
    use std::collections::HashSet;
    let mut seen: HashSet<String> = HashSet::new();
    let mut out: Vec<Usage> = Vec::new();

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return out,
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
        .collect();
    files.sort(); // deterministic dedup order across files

    for file in files {
        let content = match std::fs::read_to_string(&file) {
            Ok(c) => c,
            Err(_) => continue,
        };
        for line in content.lines().filter(|l| !l.trim().is_empty()) {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            if v.get("type").and_then(|t| t.as_str()) != Some("assistant") {
                continue;
            }
            let Some(msg) = v.get("message") else {
                continue;
            };
            let Some(usage) = msg.get("usage") else {
                continue;
            };
            let Some(id) = msg.get("id").and_then(|i| i.as_str()) else {
                continue;
            };
            if !seen.insert(id.to_string()) {
                continue; // Gotcha 1: same message.id already counted
            }
            let Some(ts_ms) = v
                .get("timestamp")
                .and_then(|t| t.as_str())
                .and_then(parse_iso_to_epoch_ms)
            else {
                continue;
            };
            let u = |k: &str| usage.get(k).and_then(|n| n.as_u64()).unwrap_or(0);
            out.push(Usage {
                ts_ms,
                tokens: ArchitectTokens {
                    input: u("input_tokens"),
                    cache_creation: u("cache_creation_input_tokens"),
                    cache_read: u("cache_read_input_tokens"),
                    output: u("output_tokens"),
                },
            });
        }
    }
    out
}

/// Sum `b` into `a` per class (saturating).
fn add_tokens(a: &mut ArchitectTokens, b: &ArchitectTokens) {
    a.input = a.input.saturating_add(b.input);
    a.cache_creation = a.cache_creation.saturating_add(b.cache_creation);
    a.cache_read = a.cache_read.saturating_add(b.cache_read);
    a.output = a.output.saturating_add(b.output);
}

/// For each usage, find the activity with the smallest `ts >= usage.ts_ms` (the
/// next journaling boundary at or after the message) and add its tokens there.
/// Returns per-activity summed tokens (index-aligned to `sorted`) and the count of
/// usages that fell after the last boundary (unattributed).
fn attribute(sorted: &[ArchitectActivity], usages: &[Usage]) -> (Vec<ArchitectTokens>, usize) {
    let mut sums = vec![ArchitectTokens::default(); sorted.len()];
    let mut unattributed = 0usize;
    for u in usages {
        // sorted ascending by ts; first activity whose ts >= u.ts_ms wins.
        match sorted.iter().position(|a| a.ts >= u.ts_ms) {
            Some(idx) => add_tokens(&mut sums[idx], &u.tokens),
            None => unattributed += 1,
        }
    }
    (sums, unattributed)
}

/// Harvest transcript usage onto this project's journal activities.
pub fn harvest(
    config_path: &Path,
    telemetry_path: Option<&Path>,
    args: &HarvestArgs,
) -> Result<HarvestOutcome, String> {
    let cfg =
        Config::load_with_env(config_path).map_err(|e| format!("failed to load config: {}", e))?;

    let telemetry_dir: PathBuf = if let Some(p) = telemetry_path {
        p.parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| "invalid --telemetry-path: no parent directory".to_string())?
    } else if let Some(ref dir) = cfg.telemetry.dir {
        dir.clone()
    } else {
        return Err(
            "telemetry disabled: cfg.telemetry.dir not set and no --telemetry-path provided"
                .to_string(),
        );
    };

    let project_id = args
        .project_id
        .map(str::to_string)
        .or_else(|| cfg.project.id.clone());

    let store_path = telemetry_dir.join("phase_runs.jsonl");
    let all = telemetry::read_architect_activities(&store_path)
        .map_err(|e| format!("failed to read activities: {}", e))?;
    // Fold first so we enrich the current winners, then keep only this project's,
    // sorted ascending by ts for the window scan.
    let mut sorted: Vec<ArchitectActivity> = telemetry::fold_activities(all)
        .into_iter()
        .filter(|a| a.project_id == project_id)
        .collect();
    sorted.sort_by_key(|a| a.ts);

    let usages = read_transcript_usages(args.transcript_dir);
    let messages = usages.len();
    let (sums, unattributed) = attribute(&sorted, &usages);

    let mut enriched = 0usize;
    for (act, toks) in sorted.iter().zip(sums.iter()) {
        if *toks == ArchitectTokens::default() {
            continue; // nothing landed in this window
        }
        let mut copy = act.clone();
        copy.tokens = *toks;
        telemetry::append_architect_activity(&telemetry_dir, &copy)
            .map_err(|e| format!("failed to append enriched activity: {}", e))?;
        enriched += 1;
    }

    Ok(HarvestOutcome {
        path: store_path,
        messages,
        enriched,
        unattributed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn make_config(temp_dir: &TempDir) -> PathBuf {
        let telemetry_dir = temp_dir.path().join("telemetry");
        fs::create_dir_all(&telemetry_dir).unwrap();
        let config_path = temp_dir.path().join("rexymcp.toml");
        fs::write(
            &config_path,
            format!(
                r#"[project]
id = "test-project"

[executor]
provider = "openai"
base_url = "http://localhost:8000/v1"
model = "qwen"

[telemetry]
dir = "{}"
"#,
                telemetry_dir.display()
            ),
        )
        .unwrap();
        config_path
    }

    // ---- parse_iso_to_epoch_ms ----

    #[test]
    fn parse_iso_epoch_ms_matches_known_instant() {
        // 2026-07-09T16:00:56.539Z → 1783612856539
        let ms = parse_iso_to_epoch_ms("2026-07-09T16:00:56.539Z").unwrap();
        assert_eq!(ms, 1_783_612_856_539);
        // Assert millis are included (not truncated to seconds)
        assert_ne!(ms % 1000, 0);
    }

    #[test]
    fn parse_iso_handles_missing_and_extra_fraction() {
        // No fraction → 000 millis
        let ms = parse_iso_to_epoch_ms("2026-07-09T16:00:56Z").unwrap();
        assert_eq!(ms % 1000, 0);

        // Short fraction → padded to 3 digits
        let ms = parse_iso_to_epoch_ms("2026-01-01T00:00:00.5Z").unwrap();
        assert_eq!(ms % 1000, 500);
    }

    #[test]
    fn parse_iso_rejects_malformed() {
        assert!(parse_iso_to_epoch_ms("not-a-date").is_none());
        assert!(parse_iso_to_epoch_ms("2026-07-09").is_none());
        assert!(parse_iso_to_epoch_ms("2026-13-40T00:00:00Z").is_none());
    }

    #[test]
    fn parse_iso_epoch_at_unix_epoch() {
        let ms = parse_iso_to_epoch_ms("1970-01-01T00:00:00.000Z").unwrap();
        assert_eq!(ms, 0);
    }

    // ---- read_transcript_usages ----

    fn write_fixture(dir: &Path, name: &str, lines: &[&str]) {
        let path = dir.join(name);
        fs::write(&path, lines.join("\n")).unwrap();
    }

    #[test]
    fn read_transcript_usages_dedups_by_message_id() {
        let dir = TempDir::new().unwrap();
        // 5 lines with the same message.id (identical usage), plus 1 distinct
        let _dup_id = "msg_dup";
        let dup_line = r#"{"type":"assistant","timestamp":"2026-07-09T16:00:00.000Z","message":{"id":"msg_dup","role":"assistant","model":"claude-opus-4-8","usage":{"input_tokens":6369,"cache_creation_input_tokens":16136,"cache_read_input_tokens":18456,"output_tokens":304}}}"#;
        let distinct_line = r#"{"type":"assistant","timestamp":"2026-07-09T16:01:00.000Z","message":{"id":"msg_other","role":"assistant","model":"claude-opus-4-8","usage":{"input_tokens":100,"cache_creation_input_tokens":200,"cache_read_input_tokens":300,"output_tokens":50}}}"#;
        write_fixture(
            dir.path(),
            "session.jsonl",
            &[
                dup_line,
                dup_line,
                dup_line,
                dup_line,
                dup_line,
                distinct_line,
            ],
        );

        let usages = read_transcript_usages(dir.path());
        // Gotcha 1: 5 dups collapse to 1, plus 1 distinct = 2
        assert_eq!(usages.len(), 2);

        // Total input is 6369 + 100, NOT 6369*5 + 100
        let total_input: u64 = usages.iter().map(|u| u.tokens.input).sum();
        assert_eq!(total_input, 6369 + 100);
    }

    #[test]
    fn read_transcript_usages_skips_user_and_usageless_lines() {
        let dir = TempDir::new().unwrap();
        write_fixture(
            dir.path(),
            "session.jsonl",
            &[
                r#"{"type":"user","timestamp":"2026-07-09T16:00:00.000Z","message":{"id":"msg_user","role":"user","usage":{"input_tokens":9999}}}"#,
                r#"{"type":"assistant","timestamp":"2026-07-09T16:00:00.000Z","message":{"id":"msg_no_usage","role":"assistant"}}"#,
            ],
        );

        let usages = read_transcript_usages(dir.path());
        assert!(usages.is_empty());
    }

    #[test]
    fn read_transcript_usages_missing_dir_is_empty() {
        let usages = read_transcript_usages(Path::new("/nonexistent/dir/that/does/not/exist"));
        assert!(usages.is_empty());
    }

    #[test]
    fn read_transcript_usages_maps_all_four_classes() {
        let dir = TempDir::new().unwrap();
        write_fixture(
            dir.path(),
            "session.jsonl",
            &[
                r#"{"type":"assistant","timestamp":"2026-07-09T16:00:00.000Z","message":{"id":"msg_1","role":"assistant","model":"claude-opus-4-8","usage":{"input_tokens":1000,"cache_creation_input_tokens":2000,"cache_read_input_tokens":3000,"output_tokens":400}}}"#,
            ],
        );

        let usages = read_transcript_usages(dir.path());
        assert_eq!(usages.len(), 1);
        let u = &usages[0];
        assert_eq!(u.tokens.input, 1000);
        assert_eq!(u.tokens.cache_creation, 2000);
        assert_eq!(u.tokens.cache_read, 3000);
        assert_eq!(u.tokens.output, 400);
    }

    // ---- attribute ----

    #[test]
    fn attribute_sends_message_to_next_boundary() {
        let act_100 = ArchitectActivity {
            record: "architect_activity".to_string(),
            ts: 100,
            phase_doc_path: None,
            phase_id: "p1".to_string(),
            project_id: Some("proj".to_string()),
            milestone_id: None,
            activity: "review".to_string(),
            outcome: None,
            model: None,
            tokens: ArchitectTokens::default(),
        };
        let act_200 = ArchitectActivity {
            record: "architect_activity".to_string(),
            ts: 200,
            phase_doc_path: None,
            phase_id: "p1".to_string(),
            project_id: Some("proj".to_string()),
            milestone_id: None,
            activity: "draft".to_string(),
            outcome: None,
            model: None,
            tokens: ArchitectTokens::default(),
        };
        let sorted = vec![act_100, act_200];

        let usages = vec![
            // ts=150 → next boundary is act_200
            Usage {
                ts_ms: 150,
                tokens: ArchitectTokens {
                    input: 10,
                    cache_creation: 0,
                    cache_read: 0,
                    output: 0,
                },
            },
            // ts=100 → >= is inclusive, lands on act_100
            Usage {
                ts_ms: 100,
                tokens: ArchitectTokens {
                    input: 20,
                    cache_creation: 0,
                    cache_read: 0,
                    output: 0,
                },
            },
            // ts=250 → after last boundary, unattributed
            Usage {
                ts_ms: 250,
                tokens: ArchitectTokens {
                    input: 30,
                    cache_creation: 0,
                    cache_read: 0,
                    output: 0,
                },
            },
        ];

        let (sums, unattributed) = attribute(&sorted, &usages);

        // act_100 gets ts=100 message only
        assert_eq!(sums[0].input, 20);
        // act_200 gets ts=150 message only
        assert_eq!(sums[1].input, 10);
        // ts=250 is unattributed
        assert_eq!(unattributed, 1);
    }

    // ---- harvest end-to-end ----

    #[test]
    fn harvest_appends_enriched_copy_and_fold_overlays() {
        let dir = TempDir::new().unwrap();
        let config = make_config(&dir);
        let telemetry_dir = dir.path().join("telemetry");
        let store_path = telemetry_dir.join("phase_runs.jsonl");

        // Append a zero-token activity with a known ts
        let activity = ArchitectActivity {
            record: "architect_activity".to_string(),
            ts: 1_717_000_000_000,
            phase_doc_path: None,
            phase_id: "phase-05b".to_string(),
            project_id: Some("test-project".to_string()),
            milestone_id: Some("M27".to_string()),
            activity: "review".to_string(),
            outcome: Some("approved_first_try".to_string()),
            model: Some("claude-opus-4-8".to_string()),
            tokens: ArchitectTokens::default(),
        };
        telemetry::append_architect_activity(&telemetry_dir, &activity).unwrap();

        // Write a fixture transcript with one message dated FAR in the past
        // so it precedes the activity's ts and lands in its window
        let tx_dir = dir.path().join("tx");
        fs::create_dir_all(&tx_dir).unwrap();
        fs::write(
            tx_dir.join("session.jsonl"),
            r#"{"type":"assistant","timestamp":"2020-01-01T00:00:00.000Z","message":{"id":"msg_e2e","role":"assistant","model":"claude-opus-4-8","usage":{"input_tokens":6369,"cache_creation_input_tokens":16136,"cache_read_input_tokens":18456,"output_tokens":304}}}"#,
        ).unwrap();

        let args = HarvestArgs {
            transcript_dir: &tx_dir,
            project_id: None,
        };
        let outcome = harvest(&config, None, &args).unwrap();

        assert_eq!(outcome.messages, 1);
        assert_eq!(outcome.enriched, 1);
        assert_eq!(outcome.unattributed, 0);

        // Read back + fold: the activity should now have non-zero tokens
        let all = telemetry::read_architect_activities(&store_path).unwrap();
        let folded = telemetry::fold_activities(all);
        let enriched_act = folded.iter().find(|a| a.activity == "review").unwrap();
        assert_eq!(enriched_act.tokens.input, 6369);
        assert_eq!(enriched_act.tokens.cache_creation, 16136);
        assert_eq!(enriched_act.tokens.cache_read, 18456);
        assert_eq!(enriched_act.tokens.output, 304);
    }

    #[test]
    fn harvest_project_scoping() {
        let dir = TempDir::new().unwrap();
        let config = make_config(&dir);
        let telemetry_dir = dir.path().join("telemetry");

        // Append an activity with a DIFFERENT project_id
        let activity = ArchitectActivity {
            record: "architect_activity".to_string(),
            ts: 1_717_000_000_000,
            phase_doc_path: None,
            phase_id: "phase-05b".to_string(),
            project_id: Some("other-project".to_string()),
            milestone_id: None,
            activity: "review".to_string(),
            outcome: None,
            model: None,
            tokens: ArchitectTokens::default(),
        };
        telemetry::append_architect_activity(&telemetry_dir, &activity).unwrap();

        // Write a fixture transcript with one message dated in the past
        let tx_dir = dir.path().join("tx");
        fs::create_dir_all(&tx_dir).unwrap();
        fs::write(
            tx_dir.join("session.jsonl"),
            r#"{"type":"assistant","timestamp":"2020-01-01T00:00:00.000Z","message":{"id":"msg_x","role":"assistant","model":"claude-opus-4-8","usage":{"input_tokens":9999,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"output_tokens":0}}}"#,
        ).unwrap();

        let args = HarvestArgs {
            transcript_dir: &tx_dir,
            project_id: None, // defaults to "test-project" from config
        };
        let outcome = harvest(&config, None, &args).unwrap();

        // Cross-project isolation: the "other-project" activity receives no tokens
        assert_eq!(outcome.enriched, 0);
        assert_eq!(outcome.messages, 1);
        assert_eq!(outcome.unattributed, 1); // no matching project activity
    }
}
