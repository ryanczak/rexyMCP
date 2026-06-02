//! Cross-project `PhaseRun` telemetry — one summary record per `execute_phase`,
//! appended as JSONL to a single global store (not per-repo). The durable
//! substrate for the M7 model scorecard (`model × tag`) and project review
//! (`milestone × phase`). The executor fills the objective fields at phase end;
//! the architect's review fills the supervision fields (`bugs_filed`,
//! `bounces_to_approval`, `architect_verdict`, `warnings`) later.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::ai::types::TokenBreakdown;

/// Generation knobs for the run — "how" the model was asked. The executor layer
/// often does not know these (M5 populates from the request); `None` until then.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GenerationParams {
    pub temperature: Option<f64>,
    pub seed: Option<u64>,
}

/// Pass/fail of the final command set, captured on clean completion. `None` for a
/// command that was not configured, or any field when the phase did not complete
/// (the command set runs only on a clean finish).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Gates {
    pub fmt: Option<bool>,
    pub build: Option<bool>,
    pub lint: Option<bool>,
    pub test: Option<bool>,
}

/// One per-phase metrics row. Objective fields are filled by the executor; the
/// supervision fields are filled by the architect at review (M7).
/// (No `PartialEq` — `TokenBreakdown` doesn't implement it; compare via JSON.)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhaseRun {
    pub ts: u64,
    // identity
    pub model: String,
    pub generation_params: GenerationParams,
    pub phase_id: String,
    pub tags: Vec<String>,
    // outcome
    pub status: String,
    pub escalated: bool,
    // quality (objective)
    pub gates: Gates,
    // reliability (objective)
    pub parse_failure_rate: f64,
    pub repairs_per_call: f64,
    pub verifier_retries: usize,
    pub tool_success_rate: f64,
    // efficiency (objective)
    pub turns: usize,
    pub wall_clock_s: f64,
    pub tokens: TokenBreakdown,
    // supervision (architect-filled at review — M7)
    pub warnings: Option<u32>,
    pub bugs_filed: Option<u32>,
    pub bounces_to_approval: Option<u32>,
    pub architect_verdict: Option<String>,
}

/// Append one `PhaseRun` as a JSON line to `<telemetry_dir>/phase_runs.jsonl`,
/// creating the directory if needed. Returns the file path.
pub fn append(telemetry_dir: &Path, run: &PhaseRun) -> std::io::Result<PathBuf> {
    use std::io::Write;

    std::fs::create_dir_all(telemetry_dir)?;
    let path = telemetry_dir.join("phase_runs.jsonl");
    let line = serde_json::to_string(run).map_err(std::io::Error::other)?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    file.write_all(line.as_bytes())?;
    file.write_all(b"\n")?;
    Ok(path)
}

/// Read all `PhaseRun` records from a store file (skips blank/corrupt lines).
pub fn read(path: &Path) -> std::io::Result<Vec<PhaseRun>> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    Ok(content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<PhaseRun>(l).ok())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn sample() -> PhaseRun {
        PhaseRun {
            ts: 1_717_000_000_000,
            model: "qwen2.5-coder".to_string(),
            generation_params: GenerationParams::default(),
            phase_id: "phase-08".to_string(),
            tags: vec!["rust".to_string(), "feature".to_string()],
            status: "complete".to_string(),
            escalated: false,
            gates: Gates {
                fmt: Some(true),
                build: Some(true),
                lint: Some(true),
                test: Some(false),
            },
            parse_failure_rate: 0.1,
            repairs_per_call: 0.5,
            verifier_retries: 2,
            tool_success_rate: 0.9,
            turns: 7,
            wall_clock_s: 12.5,
            tokens: TokenBreakdown::default(),
            warnings: None,
            bugs_filed: None,
            bounces_to_approval: None,
            architect_verdict: None,
        }
    }

    #[test]
    fn phase_run_round_trips_through_json() {
        let run = sample();
        let json = serde_json::to_string(&run).unwrap();
        let back: PhaseRun = serde_json::from_str(&json).unwrap();
        // TokenBreakdown isn't PartialEq; compare via re-serialization.
        assert_eq!(json, serde_json::to_string(&back).unwrap());
    }

    #[test]
    fn append_writes_one_line_per_run() {
        let dir = TempDir::new().unwrap();
        let path = append(dir.path(), &sample()).unwrap();
        append(dir.path(), &sample()).unwrap();
        let records = read(&path).unwrap();
        assert_eq!(records.len(), 2);
    }

    #[test]
    fn read_missing_file_is_empty() {
        let records = read(Path::new("/nonexistent/phase_runs.jsonl")).unwrap();
        assert!(records.is_empty());
    }
}
