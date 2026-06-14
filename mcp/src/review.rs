//! Architect review write-back — `rexymcp review` subcommand.

use std::path::{Path, PathBuf};

use rexymcp_executor::config::Config;
use rexymcp_executor::store::telemetry::{
    self, PhaseReview, REVIEW_RECORD_TAG, is_known_failure_class,
};

/// Borrowed review inputs from the CLI flags.
pub struct ReviewArgs<'a> {
    pub phase_doc: Option<&'a Path>,
    pub phase_id: &'a str,
    pub project_id: Option<&'a str>,
    pub verdict: &'a str,
    pub failure_class: &'a [String],
    pub bounces: Option<u32>,
    pub bugs_filed: Option<u32>,
    pub warnings: Option<u32>,
}

/// Result of recording a review: the store path and any failure classes that
/// were outside the canonical vocabulary (recorded anyway; the caller warns).
pub struct ReviewOutcome {
    pub path: PathBuf,
    pub unknown_classes: Vec<String>,
}

/// Build a `PhaseReview` from `args` (stamped with `ts`) and append it to the
/// telemetry store. Resolves the telemetry **directory** from config or the
/// `--telemetry-path` file override (its parent dir). Validation of
/// `failure_class` is advisory: unknown classes are returned, not rejected.
pub fn record_review(
    config_path: &Path,
    telemetry_path: Option<&Path>,
    ts: u64,
    args: &ReviewArgs,
) -> Result<ReviewOutcome, String> {
    let cfg =
        Config::load_with_env(config_path).map_err(|e| format!("failed to load config: {}", e))?;

    // Resolve the telemetry DIRECTORY (append_review joins phase_runs.jsonl).
    let telemetry_dir: PathBuf = if let Some(p) = telemetry_path {
        // The override names the file; append_review needs its parent dir.
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

    let unknown_classes: Vec<String> = args
        .failure_class
        .iter()
        .filter(|c| !is_known_failure_class(c))
        .cloned()
        .collect();

    let review = PhaseReview {
        record: REVIEW_RECORD_TAG.to_string(),
        ts,
        phase_doc_path: args.phase_doc.map(|p| p.to_string_lossy().into_owned()),
        phase_id: args.phase_id.to_string(),
        project_id,
        architect_verdict: args.verdict.to_string(),
        bounces_to_approval: args.bounces,
        bugs_filed: args.bugs_filed,
        warnings: args.warnings,
        failure_class: args.failure_class.to_vec(),
    };

    let path = telemetry::append_review(&telemetry_dir, &review)
        .map_err(|e| format!("failed to append review: {}", e))?;

    Ok(ReviewOutcome {
        path,
        unknown_classes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rexymcp_executor::store::telemetry;
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
    fn records_and_reads_back_review() {
        let dir = TempDir::new().unwrap();
        let config = make_config(&dir);
        let telemetry_file = dir.path().join("telemetry").join("phase_runs.jsonl");

        let args = ReviewArgs {
            phase_doc: Some(Path::new("/abs/path/to/phase-01.md")),
            phase_id: "phase-01",
            project_id: None,
            verdict: "approved_first_try",
            failure_class: &["none".to_string()],
            bounces: Some(0),
            bugs_filed: Some(0),
            warnings: Some(0),
        };

        let outcome = record_review(&config, None, 1_717_000_000_000, &args).unwrap();
        assert_eq!(outcome.path, telemetry_file);
        assert!(outcome.unknown_classes.is_empty());

        let reviews = telemetry::read_reviews(&telemetry_file).unwrap();
        assert_eq!(reviews.len(), 1);
        let r = &reviews[0];
        assert_eq!(r.architect_verdict, "approved_first_try");
        assert_eq!(
            r.phase_doc_path,
            Some("/abs/path/to/phase-01.md".to_string())
        );
        assert_eq!(r.failure_class, ["none"]);
        assert_eq!(r.record, "review");
    }

    #[test]
    fn unknown_failure_class_is_recorded_not_rejected() {
        let dir = TempDir::new().unwrap();
        let config = make_config(&dir);
        let telemetry_file = dir.path().join("telemetry").join("phase_runs.jsonl");

        let args = ReviewArgs {
            phase_doc: None,
            phase_id: "phase-02",
            project_id: None,
            verdict: "bounced",
            failure_class: &["made_up_class".to_string()],
            bounces: None,
            bugs_filed: None,
            warnings: None,
        };

        let outcome = record_review(&config, None, 1_717_000_001_000, &args).unwrap();
        assert_eq!(outcome.unknown_classes, vec!["made_up_class".to_string()]);

        // The review is still recorded despite the unknown class
        let reviews = telemetry::read_reviews(&telemetry_file).unwrap();
        assert_eq!(reviews.len(), 1);
        assert_eq!(reviews[0].failure_class, vec!["made_up_class".to_string()]);
        assert_eq!(reviews[0].architect_verdict, "bounced");
    }

    #[test]
    fn project_id_defaults_from_config() {
        let dir = TempDir::new().unwrap();
        let config = make_config(&dir);
        let telemetry_file = dir.path().join("telemetry").join("phase_runs.jsonl");

        // Omit project_id — should fall back to config's "test-project"
        let args = ReviewArgs {
            phase_doc: None,
            phase_id: "phase-03",
            project_id: None,
            verdict: "approved_first_try",
            failure_class: &[],
            bounces: None,
            bugs_filed: None,
            warnings: None,
        };

        record_review(&config, None, 1_717_000_002_000, &args).unwrap();
        let reviews = telemetry::read_reviews(&telemetry_file).unwrap();
        assert_eq!(reviews.len(), 1);
        assert_eq!(reviews[0].project_id, Some("test-project".to_string()));

        // Pinned negative: when --project-id IS supplied, it wins
        let args2 = ReviewArgs {
            phase_doc: None,
            phase_id: "phase-04",
            project_id: Some("override-project"),
            verdict: "approved_first_try",
            failure_class: &[],
            bounces: None,
            bugs_filed: None,
            warnings: None,
        };
        record_review(&config, None, 1_717_000_003_000, &args2).unwrap();
        let reviews = telemetry::read_reviews(&telemetry_file).unwrap();
        let override_review = reviews.iter().find(|r| r.phase_id == "phase-04").unwrap();
        assert_eq!(
            override_review.project_id,
            Some("override-project".to_string())
        );
    }
}
