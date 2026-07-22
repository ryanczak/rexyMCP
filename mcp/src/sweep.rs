//! Auto-telemetry sweep — periodic background harvest inside `rexymcp serve`.
//!
//! On an interval (default 60 s) it re-runs `harvest()` keeping the ledger
//! continuously fresh. A skip-guard tracks a watermark (max transcript mtime)
//! and skips the harvest append when nothing changed.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::harvest::{self, HarvestArgs};

/// Map an absolute cwd to its Claude Code transcript dir under
/// `$HOME/.claude/projects/`. Every `/` in the absolute cwd becomes `-`; case
/// is preserved.
pub fn transcript_dir_for(home: &Path, cwd: &Path) -> PathBuf {
    let slug = cwd.to_string_lossy().replace('/', "-");
    home.join(".claude").join("projects").join(slug)
}

/// Liveness marker persisted in the telemetry dir as `sweep_state.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SweepState {
    /// When the last tick ran (harvest OR skip), millis since epoch.
    pub last_swept_ms: u64,
    /// Human-readable outcome of the last tick.
    pub outcome: String,
    /// Watermark: max transcript mtime at the last *harvest* (0 if never harvested).
    pub last_seen_mtime_ms: u64,
}

/// Write the liveness marker to `<telemetry_dir>/sweep_state.json`.
pub fn write_liveness(telemetry_dir: &Path, state: &SweepState) -> Result<(), String> {
    let path = telemetry_dir.join("sweep_state.json");
    let json = serde_json::to_string(state).map_err(|e| format!("serialize sweep_state: {e}"))?;
    fs::write(&path, json).map_err(|e| format!("write {}: {e}", path.display()))
}

/// Read the liveness marker. `None` if absent or unparseable.
pub fn read_liveness(telemetry_dir: &Path) -> Option<SweepState> {
    let path = telemetry_dir.join("sweep_state.json");
    let json = fs::read_to_string(&path).ok()?;
    serde_json::from_str(&json).ok()
}

/// Max mtime (millis since epoch) across `*.jsonl` files in `dir`.
/// `None` if the dir is unreadable or has no matching files. Stats only.
pub fn max_transcript_mtime_ms(dir: &Path) -> Option<u64> {
    let mut max_ms: Option<u64> = None;
    let entries = fs::read_dir(dir).ok()?;
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("jsonl")
            && let Ok(meta) = path.metadata()
            && let Ok(modified) = meta.modified()
            && let Ok(dur) = modified.duration_since(std::time::UNIX_EPOCH)
        {
            let ms = dur.as_millis() as u64;
            max_ms = Some(match max_ms {
                Some(prev) => prev.max(ms),
                None => ms,
            });
        }
    }
    max_ms
}

/// Pure decision: harvest when we've never harvested or a transcript changed
/// since the watermark.
pub fn should_harvest(current_mtime_ms: Option<u64>, prev_watermark_ms: Option<u64>) -> bool {
    match (current_mtime_ms, prev_watermark_ms) {
        (Some(cur), Some(prev)) => cur > prev,
        (Some(_), None) => true,
        (None, _) => false,
    }
}

/// Run one sweep tick + write the liveness marker. `now_ms` is injected (no wall
/// clock in the unit under test). Never returns Err — every outcome is folded
/// into the marker string and logged.
pub fn sweep_once(config_path: &Path, transcript_dir: &Path, telemetry_dir: &Path, now_ms: u64) {
    let prev = read_liveness(telemetry_dir);
    let prev_watermark = prev.as_ref().map(|s| s.last_seen_mtime_ms);
    // Treat 0 as "never harvested" → maps to None for skip decision
    let prev_for_skip = prev_watermark.and_then(|w| if w == 0 { None } else { Some(w) });

    // Safety net: transcript dir might not exist (imperfect munging or fresh repo)
    if !transcript_dir.exists() {
        let watermark = prev_watermark.unwrap_or(0);
        let state = SweepState {
            last_swept_ms: now_ms,
            outcome: "skipped: no transcript dir".to_string(),
            last_seen_mtime_ms: watermark,
        };
        let _ = write_liveness(telemetry_dir, &state);
        eprintln!("rexymcp sweep: {}", state.outcome);
        return;
    }

    let current = max_transcript_mtime_ms(transcript_dir);

    // Skip path: no transcript change since last harvest
    if !should_harvest(current, prev_for_skip) {
        let watermark = prev_watermark.unwrap_or(0);
        let state = SweepState {
            last_swept_ms: now_ms,
            outcome: "no change".to_string(),
            last_seen_mtime_ms: watermark,
        };
        let _ = write_liveness(telemetry_dir, &state);
        eprintln!("rexymcp sweep: {}", state.outcome);
        return;
    }

    // Harvest path
    let args = HarvestArgs {
        transcript_dir,
        project_id: None,
    };
    match harvest::harvest(config_path, None, &args) {
        Ok(o) => {
            let outcome = format!("{} records / {} msgs", o.records, o.messages);
            let watermark = current.unwrap_or(0);
            let state = SweepState {
                last_swept_ms: now_ms,
                outcome: outcome.clone(),
                last_seen_mtime_ms: watermark,
            };
            let _ = write_liveness(telemetry_dir, &state);
            eprintln!("rexymcp sweep: {}", outcome);
        }
        Err(e) => {
            let watermark = prev_watermark.unwrap_or(0);
            let outcome = format!("error: {e}");
            let state = SweepState {
                last_swept_ms: now_ms,
                outcome: outcome.clone(),
                last_seen_mtime_ms: watermark,
            };
            let _ = write_liveness(telemetry_dir, &state);
            eprintln!("rexymcp sweep: {}", outcome);
        }
    }
}

/// Format a liveness line for `rexymcp costs`.
pub fn liveness_line(state: &SweepState, now_ms: u64) -> String {
    let elapsed_ms = now_ms.saturating_sub(state.last_swept_ms);
    let (unit, val) = if elapsed_ms < 60_000 {
        ("s", elapsed_ms / 1_000)
    } else if elapsed_ms < 3_600_000 {
        ("m", elapsed_ms / 60_000)
    } else {
        ("h", elapsed_ms / 3_600_000)
    };
    format!("Last swept: {val}{unit} ago ({})", state.outcome)
}

/// Run the interval loop. Production wall-clock for `now_ms`.
pub async fn run_sweep_loop(
    config_path: PathBuf,
    transcript_dir: PathBuf,
    telemetry_dir: PathBuf,
    interval: Duration,
) {
    loop {
        tokio::time::sleep(interval).await;
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        sweep_once(&config_path, &transcript_dir, &telemetry_dir, now_ms);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn make_config(temp: &TempDir) -> PathBuf {
        let config_path = temp.path().join("rexymcp.toml");
        let telemetry_dir = temp.path().join("telemetry");
        fs::create_dir_all(&telemetry_dir).unwrap();
        let content = format!(
            r#"[project]
id = "test-proj"

[telemetry]
dir = "{}"
"#,
            telemetry_dir.display()
        );
        fs::write(&config_path, content).unwrap();
        config_path
    }

    // ---- transcript_dir_for tests ----

    #[test]
    fn transcript_dir_for_munges_slashes_to_dashes() {
        let home = PathBuf::from("/home/matt");
        let cwd = PathBuf::from("/home/matt/src/rexyMCP");
        let result = transcript_dir_for(&home, &cwd);
        assert_eq!(
            result,
            PathBuf::from("/home/matt/.claude/projects/-home-matt-src-rexyMCP")
        );
    }

    #[test]
    fn transcript_dir_for_preserves_case() {
        let home = PathBuf::from("/x");
        let cwd = PathBuf::from("/x/rexyMCP");
        let result = transcript_dir_for(&home, &cwd);
        assert_eq!(result, PathBuf::from("/x/.claude/projects/-x-rexyMCP"));
        // Must NOT be lowercased
        assert!(!result.to_string_lossy().ends_with("-rexymcp"));
    }

    // ---- should_harvest tests ----

    #[test]
    fn should_harvest_decides_on_watermark() {
        // Never harvested → harvest
        assert!(should_harvest(Some(5), None));
        // Equal → skip
        assert!(!should_harvest(Some(5), Some(5)));
        // Greater → harvest
        assert!(should_harvest(Some(6), Some(5)));
        // No current files → skip
        assert!(!should_harvest(None, Some(5)));
    }

    // ---- read/write liveness tests ----

    #[test]
    fn read_liveness_roundtrips() {
        let dir = TempDir::new().unwrap();
        let state = SweepState {
            last_swept_ms: 1234,
            outcome: "test".to_string(),
            last_seen_mtime_ms: 5678,
        };
        write_liveness(dir.path(), &state).unwrap();
        let read = read_liveness(dir.path()).unwrap();
        assert_eq!(read.last_swept_ms, 1234);
        assert_eq!(read.outcome, "test");
        assert_eq!(read.last_seen_mtime_ms, 5678);
    }

    #[test]
    fn read_liveness_none_for_absent_or_garbage() {
        let dir = TempDir::new().unwrap();
        // Absent
        assert!(read_liveness(dir.path()).is_none());
        // Garbage
        fs::write(dir.path().join("sweep_state.json"), "not-json").unwrap();
        assert!(read_liveness(dir.path()).is_none());
    }

    // ---- sweep_once tests ----

    #[test]
    fn sweep_once_harvests_and_writes_marker() {
        let temp = TempDir::new().unwrap();
        let config_path = make_config(&temp);
        let tx_dir = temp.path().join("tx");
        fs::create_dir_all(&tx_dir).unwrap();
        // Write a minimal assistant-usage fixture line
        let line = r#"{"type":"assistant","timestamp":"2026-07-09T16:00:00.000Z","message":{"id":"msg1","role":"assistant","model":"claude-opus-4-8","usage":{"input_tokens":100,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"output_tokens":50}}}"#;
        fs::write(tx_dir.join("session.jsonl"), line).unwrap();

        let tel_dir = temp.path().join("telemetry");
        let now_ms = 1000;
        sweep_once(&config_path, &tx_dir, &tel_dir, now_ms);

        let state = read_liveness(&tel_dir).unwrap();
        assert_eq!(state.last_swept_ms, now_ms);
        assert!(!state.outcome.is_empty());
        // Verify phase_runs.jsonl was created with records
        let store = tel_dir.join("phase_runs.jsonl");
        assert!(store.exists());
        let content = fs::read_to_string(&store).unwrap();
        assert!(!content.is_empty());
    }

    #[test]
    fn sweep_once_skips_append_when_unchanged() {
        let temp = TempDir::new().unwrap();
        let config_path = make_config(&temp);
        let tx_dir = temp.path().join("tx");
        fs::create_dir_all(&tx_dir).unwrap();
        let line = r#"{"type":"assistant","timestamp":"2026-07-09T16:00:00.000Z","message":{"id":"msg1","role":"assistant","model":"claude-opus-4-8","usage":{"input_tokens":100,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"output_tokens":50}}}"#;
        fs::write(tx_dir.join("session.jsonl"), line).unwrap();

        let tel_dir = temp.path().join("telemetry");
        // First sweep: harvests
        sweep_once(&config_path, &tx_dir, &tel_dir, 1000);
        let store = tel_dir.join("phase_runs.jsonl");
        let lines_after_first = fs::read_to_string(&store).unwrap().lines().count();

        // Second sweep: should skip (no transcript change)
        sweep_once(&config_path, &tx_dir, &tel_dir, 2000);

        let lines_after_second = fs::read_to_string(&store).unwrap().lines().count();
        // Store should not have grown
        assert_eq!(
            lines_after_first, lines_after_second,
            "phase_runs.jsonl should not grow on skip"
        );

        // Marker should say "no change"
        let state = read_liveness(&tel_dir).unwrap();
        assert_eq!(state.outcome, "no change");
    }

    #[test]
    fn sweep_once_missing_transcript_dir_is_noop() {
        let temp = TempDir::new().unwrap();
        let config_path = make_config(&temp);
        let tx_dir = temp.path().join("nonexistent");
        let tel_dir = temp.path().join("telemetry");
        let now_ms = 1000;
        sweep_once(&config_path, &tx_dir, &tel_dir, now_ms);

        let state = read_liveness(&tel_dir).unwrap();
        assert_eq!(state.outcome, "skipped: no transcript dir");
        assert_eq!(state.last_swept_ms, now_ms);
    }

    // ---- run_sweep_loop test ----

    #[tokio::test]
    async fn run_sweep_loop_ticks_and_writes_marker() {
        let temp = TempDir::new().unwrap();
        let config_path = make_config(&temp);
        let tx_dir = temp.path().join("tx");
        fs::create_dir_all(&tx_dir).unwrap();
        let line = r#"{"type":"assistant","timestamp":"2026-07-09T16:00:00.000Z","message":{"id":"msg1","role":"assistant","model":"claude-opus-4-8","usage":{"input_tokens":100,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"output_tokens":50}}}"#;
        fs::write(tx_dir.join("session.jsonl"), line).unwrap();

        let tel_dir = temp.path().join("telemetry");
        let loop_handle = tokio::spawn(run_sweep_loop(
            config_path.clone(),
            tx_dir.clone(),
            tel_dir.clone(),
            Duration::from_millis(1),
        ));

        // Wait for the marker to appear (bounded timeout)
        let found = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if read_liveness(&tel_dir).is_some() {
                    return true;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;

        assert!(
            found.is_ok_and(|v| v),
            "sweep_state.json should appear within timeout"
        );
        loop_handle.abort();
    }

    // ---- liveness_line tests ----

    #[test]
    fn liveness_line_names_elapsed_and_outcome() {
        let state = SweepState {
            last_swept_ms: 100_000,
            outcome: "12 records / 480 msgs".to_string(),
            last_seen_mtime_ms: 50_000,
        };
        // 5 minutes later
        let now_ms = 100_000 + 5 * 60_000;
        let line = liveness_line(&state, now_ms);
        assert!(
            line.contains("5m"),
            "should mention elapsed minutes: {}",
            line
        );
        assert!(
            line.contains("12 records / 480 msgs"),
            "should include outcome: {}",
            line
        );
    }

    // ---- sweep_interval tests ----

    #[test]
    fn sweep_interval_defaults_to_60_when_unset() {
        let tel = rexymcp_executor::config::TelemetryConfig::default();
        assert_eq!(
            tel.sweep_interval(),
            Duration::from_secs(rexymcp_executor::config::DEFAULT_SWEEP_INTERVAL_SECS)
        );
    }

    #[test]
    fn sweep_interval_honors_config() {
        let temp = TempDir::new().unwrap();
        let config_path = temp.path().join("rexymcp.toml");
        let content = r#"[project]
id = "test"

[telemetry]
sweep_interval_secs = 120
"#;
        fs::write(&config_path, content).unwrap();
        let cfg = rexymcp_executor::config::Config::load(&config_path).unwrap();
        assert_eq!(cfg.telemetry.sweep_interval(), Duration::from_secs(120));
    }
}
