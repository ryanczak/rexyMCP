use rexymcp_executor::config::Tier;
use std::path::Path;
use toml_edit::{DocumentMut, value};

pub struct CalibrateArgs<'a> {
    pub tier: Tier,
    pub config_path: &'a Path,
}

pub fn run(args: &CalibrateArgs<'_>) -> anyhow::Result<()> {
    let src = if args.config_path.exists() {
        std::fs::read_to_string(args.config_path)?
    } else {
        String::new()
    };

    let mut doc: DocumentMut = src
        .parse()
        .map_err(|e| anyhow::anyhow!("TOML parse error: {e}"))?;

    // [executor].tier
    doc["executor"]["tier"] = value(tier_str(args.tier));

    // [budget] tier-derived defaults — only write when the key is absent so an
    // explicit override survives a re-calibrate.
    let max_turns = args.tier.default_max_turns();
    if doc.get("budget").and_then(|b| b.get("max_turns")).is_none() {
        doc["budget"]["max_turns"] = value(max_turns as i64);
    }
    // gate_retries: write only when absent; skip for Large (u32::MAX is implicit).
    let gate_retries = args.tier.default_gate_retries();
    if gate_retries != u32::MAX
        && doc
            .get("budget")
            .and_then(|b| b.get("gate_retries"))
            .is_none()
    {
        doc["budget"]["gate_retries"] = value(gate_retries as i64);
    }

    // [escalation] — write only for Small; remove for Medium/Large (absent = ignored).
    match args.tier {
        Tier::Small => {
            if doc.get("escalation").is_none() {
                doc["escalation"]["max_assists"] = value(3i64);
            }
        }
        _ => {
            doc.remove("escalation");
        }
    }

    // [architect] — add skeleton when absent so the user sees the section.
    if doc.get("architect").is_none() {
        doc["architect"]["model"] = value("");
        doc["architect"]["input_per_mtok"] = value(0.0);
        doc["architect"]["output_per_mtok"] = value(0.0);
    }

    std::fs::write(args.config_path, doc.to_string())?;

    println!(
        "Calibrated to {tier} — updated executor.tier={tier_s}, budget.max_turns={max_turns}{}{}",
        if gate_retries != u32::MAX {
            format!(", budget.gate_retries={gate_retries}")
        } else {
            String::new()
        },
        match args.tier {
            Tier::Small => ", escalation.max_assists=3",
            _ => "",
        },
        tier = tier_label(args.tier),
        tier_s = tier_str(args.tier),
    );
    Ok(())
}

fn tier_str(tier: Tier) -> &'static str {
    match tier {
        Tier::Large => "LARGE",
        Tier::Medium => "MEDIUM",
        Tier::Small => "SMALL",
    }
}

fn tier_label(tier: Tier) -> &'static str {
    tier_str(tier)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rexymcp_executor::config::Tier;
    use tempfile::TempDir;

    fn run_calibrate(dir: &TempDir, tier: Tier, initial: &str) -> String {
        let path = dir.path().join("rexymcp.toml");
        std::fs::write(&path, initial).unwrap();
        run(&CalibrateArgs {
            tier,
            config_path: &path,
        })
        .unwrap();
        std::fs::read_to_string(&path).unwrap()
    }

    #[test]
    fn calibrate_medium_sets_tier_and_budget() {
        let dir = TempDir::new().unwrap();
        let result = run_calibrate(
            &dir,
            Tier::Medium,
            r#"
[executor]
provider = "openai"
model = "m"
base_url = "http://localhost:1234/v1"

[budget]
context_length = 131071
max_context_pct = 80
max_turns = 200
escalation_slots = 1
"#,
        );
        let doc: toml_edit::DocumentMut = result.parse().unwrap();
        assert_eq!(doc["executor"]["tier"].as_str(), Some("MEDIUM"));
        // max_turns already set — calibrate does NOT overwrite it
        assert_eq!(doc["budget"]["max_turns"].as_integer(), Some(200));
        // gate_retries written for Medium
        assert_eq!(doc["budget"]["gate_retries"].as_integer(), Some(2));
    }

    #[test]
    fn calibrate_small_adds_escalation_section() {
        let dir = TempDir::new().unwrap();
        let result = run_calibrate(
            &dir,
            Tier::Small,
            r#"
[executor]
provider = "openai"
model = "m"
base_url = "http://localhost:1234/v1"

[budget]
context_length = 131071
max_context_pct = 80
max_turns = 200
escalation_slots = 1
"#,
        );
        let doc: toml_edit::DocumentMut = result.parse().unwrap();
        assert_eq!(doc["executor"]["tier"].as_str(), Some("SMALL"));
        assert_eq!(doc["escalation"]["max_assists"].as_integer(), Some(3));
    }

    #[test]
    fn calibrate_medium_removes_escalation_section() {
        let dir = TempDir::new().unwrap();
        let result = run_calibrate(
            &dir,
            Tier::Medium,
            r#"
[executor]
provider = "openai"
model = "m"
base_url = "http://localhost:1234/v1"

[budget]
context_length = 32768
max_context_pct = 70
max_turns = 100
escalation_slots = 1

[escalation]
max_assists = 3
"#,
        );
        let doc: toml_edit::DocumentMut = result.parse().unwrap();
        assert!(
            doc.get("escalation").is_none(),
            "[escalation] must be removed for MEDIUM"
        );
    }

    #[test]
    fn calibrate_large_does_not_write_gate_retries() {
        let dir = TempDir::new().unwrap();
        let result = run_calibrate(
            &dir,
            Tier::Large,
            r#"
[executor]
provider = "openai"
model = "m"
base_url = "http://localhost:1234/v1"

[budget]
context_length = 131071
max_context_pct = 80
max_turns = 400
escalation_slots = 1
"#,
        );
        let doc: toml_edit::DocumentMut = result.parse().unwrap();
        assert_eq!(doc["executor"]["tier"].as_str(), Some("LARGE"));
        // gate_retries must not be written for Large (unlimited is the default)
        assert!(
            doc.get("budget")
                .and_then(|b| b.get("gate_retries"))
                .is_none()
        );
    }

    #[test]
    fn calibrate_adds_architect_skeleton_when_absent() {
        let dir = TempDir::new().unwrap();
        let result = run_calibrate(
            &dir,
            Tier::Medium,
            r#"
[executor]
provider = "openai"
model = "m"
base_url = "http://localhost:1234/v1"

[budget]
context_length = 32768
max_context_pct = 70
max_turns = 40
escalation_slots = 1
"#,
        );
        let doc: toml_edit::DocumentMut = result.parse().unwrap();
        assert!(
            doc.get("architect").is_some(),
            "[architect] skeleton must be added"
        );
    }

    #[test]
    fn calibrate_preserves_existing_architect_section() {
        let dir = TempDir::new().unwrap();
        let result = run_calibrate(
            &dir,
            Tier::Medium,
            r#"
[executor]
provider = "openai"
model = "m"
base_url = "http://localhost:1234/v1"

[budget]
context_length = 32768
max_context_pct = 70
max_turns = 40
escalation_slots = 1

[architect]
model = "claude-opus-4-8"
input_per_mtok = 5.0
output_per_mtok = 25.0
"#,
        );
        let doc: toml_edit::DocumentMut = result.parse().unwrap();
        assert_eq!(doc["architect"]["model"].as_str(), Some("claude-opus-4-8"));
    }
}
