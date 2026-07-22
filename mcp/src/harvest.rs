//! Architect usage harvester — `rexymcp harvest` subcommand.
//!
//! Reads Claude Code session transcripts, dedups assistant-usage lines by
//! `message.id`, and sums per-message token usage into `ArchitectLedger` records
//! keyed by `(project_id, session, model, skill)` (messages with no
//! `attributionSkill` bucket under `"other"`). Emits one ledger record per key;
//! `fold_ledger` keeps the latest per key at read time, so re-harvest is idempotent.

use std::path::{Path, PathBuf};

use rexymcp_executor::config::Config;
use rexymcp_executor::store::telemetry::{
    ARCHITECT_LEDGER_RECORD_TAG, ArchitectLedger, ArchitectTokens, append_architect_ledger,
};

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
    pub messages: usize,
    pub duplicates: usize,
    pub sessions: usize,
    pub records: usize,
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

/// Accumulator for a single (session, model, skill) bucket.
struct Accum {
    input: u64,
    cache_creation: u64,
    cache_read: u64,
    output: u64,
    cache_creation_5m: u64,
    cache_creation_1h: u64,
    messages: u64,
    last_ts: u64,
}

/// Token extraction result from a single assistant-usage line.
/// Fields: `(message_id, model, skill, input, cache_creation, cache_read, output, cc_5m, cc_1h, ts)`
type ExtractedTokens = (String, String, String, u64, u64, u64, u64, u64, u64, u64);

fn extract_usage(v: &serde_json::Value) -> Option<ExtractedTokens> {
    let usage = v.get("message").and_then(|m| m.get("usage"))?;
    let message_id = v
        .get("message")
        .and_then(|m| m.get("id"))?
        .as_str()?
        .to_string();
    let model = v
        .get("message")
        .and_then(|m| m.get("model"))
        .and_then(|m| m.as_str())
        .unwrap_or("unknown")
        .to_string();
    let skill = match v.get("attributionSkill").and_then(|s| s.as_str()) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => "other".to_string(),
    };
    let input = usage
        .get("input_tokens")
        .and_then(|t| t.as_u64())
        .unwrap_or(0);
    let cache_read = usage
        .get("cache_read_input_tokens")
        .and_then(|t| t.as_u64())
        .unwrap_or(0);
    let output = usage
        .get("output_tokens")
        .and_then(|t| t.as_u64())
        .unwrap_or(0);
    let ts = parse_iso_to_epoch_ms(v.get("timestamp").and_then(|t| t.as_str()).unwrap_or(""))
        .unwrap_or(0);

    let (cache_creation, cc_5m, cc_1h) = if let Some(cc) = usage.get("cache_creation") {
        if cc.is_object() {
            let c5m = cc
                .get("ephemeral_5m_input_tokens")
                .and_then(|t| t.as_u64())
                .unwrap_or(0);
            let c1h = cc
                .get("ephemeral_1h_input_tokens")
                .and_then(|t| t.as_u64())
                .unwrap_or(0);
            (c5m + c1h, c5m, c1h)
        } else {
            let cc = usage
                .get("cache_creation_input_tokens")
                .and_then(|t| t.as_u64())
                .unwrap_or(0);
            (cc, cc, 0)
        }
    } else {
        let cc = usage
            .get("cache_creation_input_tokens")
            .and_then(|t| t.as_u64())
            .unwrap_or(0);
        (cc, cc, 0)
    };

    Some((
        message_id,
        model,
        skill,
        input,
        cache_creation,
        cache_read,
        output,
        cc_5m,
        cc_1h,
        ts,
    ))
}

/// Harvest transcript usage into ledger records.
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

    let _project_id = args
        .project_id
        .map(str::to_string)
        .or_else(|| cfg.project.id.clone());

    let store_path = telemetry_dir.join("phase_runs.jsonl");

    // Read and parse transcript files, sorted by path for deterministic dedup order
    let mut files: Vec<PathBuf> = Vec::new();
    for entry in std::fs::read_dir(args.transcript_dir)
        .map_err(|e| format!("failed to read transcript dir: {}", e))?
    {
        let entry = entry.map_err(|e| format!("failed to read dir entry: {}", e))?;
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "jsonl") {
            files.push(path);
        }
    }
    files.sort();

    let mut seen_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut accum: std::collections::HashMap<(String, String, String), Accum> =
        std::collections::HashMap::new();
    let mut duplicates: usize = 0;
    let mut sessions: std::collections::HashSet<String> = std::collections::HashSet::new();

    for file_path in &files {
        let session_id = match file_path
            .file_stem()
            .and_then(|s| s.to_str())
            .map(str::to_string)
        {
            Some(s) => s,
            None => continue,
        };

        let content = match std::fs::read_to_string(file_path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        for line in content.lines().filter(|l| !l.trim().is_empty()) {
            let v = match serde_json::from_str::<serde_json::Value>(line) {
                Ok(v) => v,
                Err(_) => continue,
            };

            // Skip non-assistant lines
            if v.get("type").and_then(|t| t.as_str()) != Some("assistant") {
                continue;
            }

            // Extract usage; None means skip (no message.id, no usage, etc.)
            let (msg_id, model, skill, input, cache_creation, cache_read, output, cc_5m, cc_1h, ts) =
                match extract_usage(&v) {
                    Some(t) => t,
                    None => continue,
                };

            // Dedup by message.id
            if !seen_ids.insert(msg_id) {
                duplicates += 1;
                continue;
            }

            sessions.insert(session_id.clone());

            let key = (session_id.clone(), model, skill);
            let acc = accum.entry(key).or_insert(Accum {
                input: 0,
                cache_creation: 0,
                cache_read: 0,
                output: 0,
                cache_creation_5m: 0,
                cache_creation_1h: 0,
                messages: 0,
                last_ts: 0,
            });
            acc.input += input;
            acc.cache_creation += cache_creation;
            acc.cache_read += cache_read;
            acc.output += output;
            acc.cache_creation_5m += cc_5m;
            acc.cache_creation_1h += cc_1h;
            acc.messages += 1;
            if ts > acc.last_ts {
                acc.last_ts = ts;
            }
        }
    }

    // Build ledger records from accumulators, sorted for deterministic output
    let mut total_messages = 0usize;
    let mut total_records = 0usize;
    for (key, acc) in accum {
        let ledger = ArchitectLedger {
            record: ARCHITECT_LEDGER_RECORD_TAG.to_string(),
            project_id: _project_id.clone(),
            session_id: key.0,
            model: key.1,
            skill: key.2,
            tokens: ArchitectTokens {
                input: acc.input,
                cache_creation: acc.cache_creation,
                cache_read: acc.cache_read,
                output: acc.output,
            },
            cache_creation_5m: acc.cache_creation_5m,
            cache_creation_1h: acc.cache_creation_1h,
            messages: acc.messages,
            last_ts: acc.last_ts,
        };
        if let Err(e) = append_architect_ledger(&telemetry_dir, &ledger) {
            eprintln!("warning: failed to append ledger record: {}", e);
        }
        total_messages += acc.messages as usize;
        total_records += 1;
    }

    Ok(HarvestOutcome {
        path: store_path,
        messages: total_messages,
        duplicates,
        sessions: sessions.len(),
        records: total_records,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rexymcp_executor::store::telemetry::{fold_ledger, read_architect_ledger};
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

    // ---- write_fixture helper ----

    fn write_fixture(dir: &Path, name: &str, lines: &[&str]) {
        let path = dir.join(name);
        fs::write(&path, lines.join("\n")).unwrap();
    }

    // ---- harvest tests ----

    #[test]
    fn harvest_dedups_by_message_id() {
        let dir = TempDir::new().unwrap();
        let config = make_config(&dir);
        let tx_dir = dir.path().join("tx");
        fs::create_dir_all(&tx_dir).unwrap();

        // 5 lines with the same message.id, plus 1 distinct
        let dup_line = r#"{"type":"assistant","timestamp":"2026-07-09T16:00:00.000Z","message":{"id":"msg_dup","role":"assistant","model":"claude-opus-4-8","usage":{"input_tokens":6369,"cache_creation_input_tokens":16136,"cache_read_input_tokens":18456,"output_tokens":304}}}"#;
        let distinct_line = r#"{"type":"assistant","timestamp":"2026-07-09T16:01:00.000Z","message":{"id":"msg_other","role":"assistant","model":"claude-opus-4-8","usage":{"input_tokens":100,"cache_creation_input_tokens":200,"cache_read_input_tokens":300,"output_tokens":50}}}"#;
        write_fixture(
            &tx_dir,
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

        let args = HarvestArgs {
            transcript_dir: &tx_dir,
            project_id: None,
        };
        let outcome = harvest(&config, None, &args).unwrap();

        // 5 dups collapse to 1, plus 1 distinct = 2 messages
        assert_eq!(outcome.messages, 2);
        // 4 duplicates skipped
        assert_eq!(outcome.duplicates, 4);
        assert_eq!(outcome.records, 1);
    }

    #[test]
    fn harvest_buckets_by_session_model_skill() {
        let dir = TempDir::new().unwrap();
        let config = make_config(&dir);
        let tx_dir = dir.path().join("tx");
        fs::create_dir_all(&tx_dir).unwrap();

        // Session 1: two models, two skills (one with, one without attributionSkill)
        write_fixture(
            &tx_dir,
            "session_a.jsonl",
            &[
                r#"{"type":"assistant","timestamp":"2026-07-09T16:00:00.000Z","attributionSkill":"rexymcp:dispatch","message":{"id":"msg_1","role":"assistant","model":"claude-opus-4-8","usage":{"input_tokens":1000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"output_tokens":100}}}"#,
                r#"{"type":"assistant","timestamp":"2026-07-09T16:01:00.000Z","message":{"id":"msg_2","role":"assistant","model":"claude-opus-4-8","usage":{"input_tokens":2000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"output_tokens":200}}}"#,
                r#"{"type":"assistant","timestamp":"2026-07-09T16:02:00.000Z","attributionSkill":"rexymcp:dispatch","message":{"id":"msg_3","role":"assistant","model":"claude-sonnet-4-8","usage":{"input_tokens":3000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"output_tokens":300}}}"#,
            ],
        );

        // Session 2: one model, one skill
        write_fixture(
            &tx_dir,
            "session_b.jsonl",
            &[
                r#"{"type":"assistant","timestamp":"2026-07-09T17:00:00.000Z","attributionSkill":"rexymcp:dispatch","message":{"id":"msg_4","role":"assistant","model":"claude-opus-4-8","usage":{"input_tokens":4000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"output_tokens":400}}}"#,
            ],
        );

        let args = HarvestArgs {
            transcript_dir: &tx_dir,
            project_id: None,
        };
        let outcome = harvest(&config, None, &args).unwrap();

        // 4 distinct messages, 0 dups
        assert_eq!(outcome.messages, 4);
        assert_eq!(outcome.duplicates, 0);
        // 4 distinct (session, model, skill) keys
        assert_eq!(outcome.records, 4);

        // Verify the store contains ledger records with "other" skill
        let store_path = dir.path().join("telemetry/phase_runs.jsonl");
        let content = fs::read_to_string(&store_path).unwrap();
        assert!(content.contains(r#""skill":"other""#));
    }

    #[test]
    fn harvest_last_ts_is_max_message_timestamp() {
        let dir = TempDir::new().unwrap();
        let config = make_config(&dir);
        let tx_dir = dir.path().join("tx");
        fs::create_dir_all(&tx_dir).unwrap();

        // Two messages in one (session, model, skill) bucket, different timestamps,
        // zero cache tokens. last_ts must be the LATER message's epoch-ms — not a
        // cache value (guards the cc_5m-for-last_ts regression).
        write_fixture(
            &tx_dir,
            "session.jsonl",
            &[
                r#"{"type":"assistant","timestamp":"2026-07-09T16:00:00.000Z","attributionSkill":"rexymcp:dispatch","message":{"id":"a","role":"assistant","model":"claude-opus-4-8","usage":{"input_tokens":10,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"output_tokens":1}}}"#,
                r#"{"type":"assistant","timestamp":"2026-07-09T16:01:00.000Z","attributionSkill":"rexymcp:dispatch","message":{"id":"b","role":"assistant","model":"claude-opus-4-8","usage":{"input_tokens":10,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"output_tokens":1}}}"#,
            ],
        );

        let args = HarvestArgs {
            transcript_dir: &tx_dir,
            project_id: None,
        };
        let outcome = harvest(&config, None, &args).unwrap();
        assert_eq!(outcome.records, 1);

        let store_path = dir.path().join("telemetry/phase_runs.jsonl");
        let content = fs::read_to_string(&store_path).unwrap();
        // 2026-07-09T16:01:00.000Z == 1_783_612_860_000 ms
        assert!(
            content.contains(r#""last_ts":1783612860000"#),
            "last_ts must be the later message's epoch-ms, got: {content}"
        );
    }

    #[test]
    fn harvest_splits_cache_creation_5m_1h() {
        let dir = TempDir::new().unwrap();
        let config = make_config(&dir);
        let tx_dir = dir.path().join("tx");
        fs::create_dir_all(&tx_dir).unwrap();

        // Line with nested 5m/1h split
        let line_with_split = r#"{"type":"assistant","timestamp":"2026-07-09T16:00:00.000Z","message":{"id":"msg_1","role":"assistant","model":"claude-opus-4-8","usage":{"input_tokens":1000,"cache_read_input_tokens":3000,"output_tokens":400,"cache_creation":{"ephemeral_5m_input_tokens":500,"ephemeral_1h_input_tokens":1500}}}}"#;

        // Line without nested split (fallback: cache_creation_input_tokens)
        let line_fallback = r#"{"type":"assistant","timestamp":"2026-07-09T16:01:00.000Z","message":{"id":"msg_2","role":"assistant","model":"claude-opus-4-8","usage":{"input_tokens":100,"cache_creation_input_tokens":900,"cache_read_input_tokens":200,"output_tokens":50}}}"#;

        write_fixture(&tx_dir, "session.jsonl", &[line_with_split, line_fallback]);

        let args = HarvestArgs {
            transcript_dir: &tx_dir,
            project_id: None,
        };
        let outcome = harvest(&config, None, &args).unwrap();

        assert_eq!(outcome.messages, 2);
        assert_eq!(outcome.duplicates, 0);

        // Read back from store and verify the invariant
        let store_path = dir.path().join("telemetry/phase_runs.jsonl");
        let ledgers = read_architect_ledger(&store_path).unwrap();
        assert_eq!(ledgers.len(), 1);
        let ledger = &ledgers[0];
        // msg_1: cc_5m=500, cc_1h=1500, cache_creation=2000
        // msg_2: cc_5m=900, cc_1h=0, cache_creation=900
        // totals: cc_5m=1400, cc_1h=1500, cache_creation=2900
        assert_eq!(ledger.cache_creation_5m, 1400);
        assert_eq!(ledger.cache_creation_1h, 1500);
        assert_eq!(ledger.tokens.cache_creation, 2900);
        // Invariant: cc_5m + cc_1h == cache_creation
        assert_eq!(
            ledger.cache_creation_5m + ledger.cache_creation_1h,
            ledger.tokens.cache_creation
        );
    }

    #[test]
    fn harvest_is_idempotent() {
        let dir = TempDir::new().unwrap();
        let config = make_config(&dir);
        let tx_dir = dir.path().join("tx");
        fs::create_dir_all(&tx_dir).unwrap();

        write_fixture(
            &tx_dir,
            "session.jsonl",
            &[
                r#"{"type":"assistant","timestamp":"2026-07-09T16:00:00.000Z","attributionSkill":"rexymcp:dispatch","message":{"id":"msg_1","role":"assistant","model":"claude-opus-4-8","usage":{"input_tokens":1000,"cache_creation_input_tokens":2000,"cache_read_input_tokens":3000,"output_tokens":400}}}"#,
            ],
        );

        let args = HarvestArgs {
            transcript_dir: &tx_dir,
            project_id: None,
        };

        // First harvest
        let outcome1 = harvest(&config, None, &args).unwrap();
        assert_eq!(outcome1.messages, 1);

        // Second harvest (appends duplicate records to the same store)
        let outcome2 = harvest(&config, None, &args).unwrap();
        assert_eq!(outcome2.messages, 1);

        // Read all + fold: should yield the same per-key totals as a single run
        let store_path = dir.path().join("telemetry/phase_runs.jsonl");
        let all_ledgers = read_architect_ledger(&store_path).unwrap();
        let folded = fold_ledger(all_ledgers);

        // After fold, only 1 record (the second harvest's record replaces the first)
        assert_eq!(folded.len(), 1);
        let ledger = &folded[0];
        assert_eq!(ledger.tokens.input, 1000);
        assert_eq!(ledger.tokens.cache_creation, 2000);
        assert_eq!(ledger.messages, 1);
    }
}
