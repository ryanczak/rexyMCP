//! Architect loop-journal write-back — `rexymcp journal` subcommand.

use std::path::{Path, PathBuf};

use rexymcp_executor::config::Config;
use rexymcp_executor::store::telemetry::{
    self, ARCHITECT_ACTIVITY_RECORD_TAG, ArchitectActivity, is_known_activity,
};

/// Borrowed journal inputs from the CLI flags.
pub struct JournalArgs<'a> {
    pub phase_doc: Option<&'a Path>,
    pub phase_id: &'a str,
    pub project_id: Option<&'a str>,
    pub milestone_id: Option<&'a str>,
    pub activity: &'a str,
    pub outcome: Option<&'a str>,
    pub model: Option<&'a str>,
}

/// Result of recording an activity: the store path and the activity kind if
/// it was outside the canonical vocabulary (recorded anyway; caller warns).
pub struct JournalOutcome {
    pub path: PathBuf,
    pub unknown_activity: Option<String>,
}

/// Build an `ArchitectActivity` from `args` (stamped with `ts`) and append it
/// to the telemetry store. Validation of `activity` is advisory: an unknown
/// kind is returned, not rejected.
pub fn record_activity(
    config_path: &Path,
    telemetry_path: Option<&Path>,
    ts: u64,
    args: &JournalArgs,
) -> Result<JournalOutcome, String> {
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

    let unknown_activity = (!is_known_activity(args.activity)).then(|| args.activity.to_string());

    let activity = ArchitectActivity {
        record: ARCHITECT_ACTIVITY_RECORD_TAG.to_string(),
        ts,
        phase_doc_path: args.phase_doc.map(|p| p.to_string_lossy().into_owned()),
        phase_id: args.phase_id.to_string(),
        project_id,
        milestone_id: args.milestone_id.map(str::to_string),
        activity: args.activity.to_string(),
        outcome: args.outcome.map(str::to_string),
        model: args.model.map(str::to_string),
        architect_input_tokens: 0,
        architect_output_tokens: 0,
    };

    let path = telemetry::append_architect_activity(&telemetry_dir, &activity)
        .map_err(|e| format!("failed to append activity: {}", e))?;

    Ok(JournalOutcome {
        path,
        unknown_activity,
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

    #[test]
    fn records_and_reads_back_activity() {
        let dir = TempDir::new().unwrap();
        let config = make_config(&dir);
        let telemetry_file = dir.path().join("telemetry").join("phase_runs.jsonl");

        let args = JournalArgs {
            phase_doc: Some(Path::new("/abs/path/to/phase-02.md")),
            phase_id: "phase-02",
            project_id: None,
            milestone_id: Some("M27-autonomous-escalation-loop"),
            activity: "assist",
            outcome: Some("complete"),
            model: Some("claude-opus-4-8"),
        };

        let outcome = record_activity(&config, None, 1_717_000_000_000, &args).unwrap();
        assert_eq!(outcome.path, telemetry_file);
        assert!(outcome.unknown_activity.is_none());

        let activities = telemetry::read_architect_activities(&telemetry_file).unwrap();
        assert_eq!(activities.len(), 1);
        let a = &activities[0];
        assert_eq!(a.activity, "assist");
        assert_eq!(a.outcome, Some("complete".to_string()));
        assert_eq!(
            a.phase_doc_path,
            Some("/abs/path/to/phase-02.md".to_string())
        );
        assert_eq!(a.record, "architect_activity");
    }

    #[test]
    fn unknown_activity_is_recorded_not_rejected() {
        let dir = TempDir::new().unwrap();
        let config = make_config(&dir);
        let telemetry_file = dir.path().join("telemetry").join("phase_runs.jsonl");

        let args = JournalArgs {
            phase_doc: None,
            phase_id: "phase-02",
            project_id: None,
            milestone_id: None,
            activity: "frobnicate",
            outcome: None,
            model: None,
        };

        let outcome = record_activity(&config, None, 1_717_000_001_000, &args).unwrap();
        assert_eq!(outcome.unknown_activity, Some("frobnicate".to_string()));

        // The activity is still recorded despite the unknown kind
        let activities = telemetry::read_architect_activities(&telemetry_file).unwrap();
        assert_eq!(activities.len(), 1);
        assert_eq!(activities[0].activity, "frobnicate");
    }

    #[test]
    fn project_id_defaults_from_config() {
        let dir = TempDir::new().unwrap();
        let config = make_config(&dir);
        let telemetry_file = dir.path().join("telemetry").join("phase_runs.jsonl");

        // Omit project_id — should fall back to config's "test-project"
        let args = JournalArgs {
            phase_doc: None,
            phase_id: "phase-03",
            project_id: None,
            milestone_id: None,
            activity: "draft",
            outcome: None,
            model: None,
        };

        record_activity(&config, None, 1_717_000_002_000, &args).unwrap();
        let activities = telemetry::read_architect_activities(&telemetry_file).unwrap();
        assert_eq!(activities.len(), 1);
        assert_eq!(activities[0].project_id, Some("test-project".to_string()));

        // Pinned negative: when --project-id IS supplied, it wins
        let args2 = JournalArgs {
            phase_doc: None,
            phase_id: "phase-04",
            project_id: Some("override-project"),
            milestone_id: None,
            activity: "draft",
            outcome: None,
            model: None,
        };
        record_activity(&config, None, 1_717_000_003_000, &args2).unwrap();
        let activities = telemetry::read_architect_activities(&telemetry_file).unwrap();
        let override_activity = activities
            .iter()
            .find(|a| a.phase_id == "phase-04")
            .unwrap();
        assert_eq!(
            override_activity.project_id,
            Some("override-project".to_string())
        );
    }
}
