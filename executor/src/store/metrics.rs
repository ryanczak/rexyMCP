//! Shared derivations over telemetry records — the single home for every
//! *derived* metric number (reclaimed sums, tok/s, settings labels, cost).
//! Readers (`runs`/`scorecard`/`status`/dashboard) call these instead of
//! re-deriving; pinning the definition once.

use crate::store::telemetry::{ContextEfficiency, GenerationParams, PhaseRun};

/// Total tokens reclaimed in a run: boundary-filter + evicted + deduped +
/// compaction. The one definition of "reclaimed."
pub fn reclaimed_total(eff: &ContextEfficiency) -> usize {
    eff.output_filtered_tokens
        + eff.read_evicted_tokens
        + eff.read_deduped_tokens
        + eff.compaction_tokens_reclaimed
}

/// Generation throughput in output tokens per second. `None` when `gen_time_s`
/// is non-positive (no timed generation recorded) — callers render `—`.
pub fn tokens_per_sec(output_tokens: u32, gen_time_s: f64) -> Option<f64> {
    if gen_time_s > 0.0 {
        Some(output_tokens as f64 / gen_time_s)
    } else {
        None
    }
}

/// Sampling-settings label: `"default"` / `"temp=T"` / `"seed=S"` /
/// `"temp=T,seed=S"`, with `think=on` or `think=<effort>` appended when the
/// run had thinking enabled (`think=on` replaces `default`). Thinking-off
/// runs and records written before thinking was captured carry no suffix,
/// so their labels are unchanged. The exact strings `runs`/`scorecard` render.
pub fn settings_label(params: &GenerationParams) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(t) = params.temperature {
        parts.push(format!("temp={t}"));
    }
    if let Some(s) = params.seed {
        parts.push(format!("seed={s}"));
    }
    if params.enable_thinking == Some(true) {
        match params.reasoning_effort {
            Some(effort) => parts.push(format!("think={}", effort.as_str())),
            None => parts.push("think=on".to_string()),
        }
    }
    if parts.is_empty() {
        "default".to_string()
    } else {
        parts.join(",")
    }
}

/// Stable git-sha-style 8-hex-char handle for a run, derived from its identity
/// (`ts`, `model`, `phase_id`). Deterministic (FNV-1a/32, no dependency, stable
/// across platforms) so `rexymcp runs` and `runs show <id>` agree. Not
/// cryptographic — just a compact, copy-pasteable address.
pub fn run_id(run: &PhaseRun) -> String {
    let seed = format!("{}|{}|{}", run.ts, run.model, run.phase_id);
    let mut h: u32 = 0x811c_9dc5;
    for b in seed.as_bytes() {
        h ^= *b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    format!("{h:08x}")
}

/// Compact human token/count rendering: decimal SI with thousands and millions
/// tiers, one decimal place. Zero renders as the `—` sentinel.
///
/// `0 → "—"`, `1..=999 → "123"`, `1_000.. → "12.3k"`, `1_000_000.. → "2.1M"`.
/// Decimal (1000), not binary (1024): tokens are a decimal quantity. This is the
/// single formatter for token/reclaimed cells across `costs`, `runs`,
/// `scorecard`, and `profile`.
pub fn fmt_tokens(count: u64) -> String {
    if count == 0 {
        "—".to_string()
    } else if count >= 1_000_000 {
        format!("{:.1}M", count as f64 / 1_000_000.0)
    } else if count >= 1_000 {
        format!("{:.1}k", count as f64 / 1_000.0)
    } else {
        count.to_string()
    }
}

/// Nearest-rank percentile of a **sorted** slice. `p` in `0.0..=1.0`. Empty → 0.
/// The one definition of percentile, shared by calibrate-governor's stall-signal report.
pub fn percentile(sorted: &[usize], p: f64) -> usize {
    if sorted.is_empty() {
        return 0;
    }
    let rank = (p * (sorted.len() as f64 - 1.0)).round() as usize;
    sorted[rank.min(sorted.len() - 1)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reclaimed_total_sums_all_four_reclaim_fields() {
        let eff = ContextEfficiency {
            output_filtered_tokens: 100,
            read_evicted_tokens: 50,
            read_deduped_tokens: 30,
            compaction_tokens_reclaimed: 20,
            ..Default::default()
        };
        assert_eq!(reclaimed_total(&eff), 200);

        // Must-NOT pin: vary a single field and confirm it's reflected
        let eff2 = ContextEfficiency {
            output_filtered_tokens: 101,
            read_evicted_tokens: 50,
            read_deduped_tokens: 30,
            compaction_tokens_reclaimed: 20,
            ..Default::default()
        };
        assert_eq!(reclaimed_total(&eff2), 201);
    }

    #[test]
    fn tokens_per_sec_divides_output_by_time() {
        assert_eq!(tokens_per_sec(1000, 2.0), Some(500.0));
    }

    #[test]
    fn tokens_per_sec_none_when_time_zero() {
        assert_eq!(tokens_per_sec(1000, 0.0), None);
    }

    #[test]
    fn settings_label_covers_all_four_shapes() {
        let default_params = GenerationParams {
            temperature: None,
            seed: None,
            ..Default::default()
        };
        assert_eq!(settings_label(&default_params), "default");

        let temp_only = GenerationParams {
            temperature: Some(0.2),
            seed: None,
            ..Default::default()
        };
        assert_eq!(settings_label(&temp_only), "temp=0.2");

        let seed_only = GenerationParams {
            temperature: None,
            seed: Some(42),
            ..Default::default()
        };
        assert_eq!(settings_label(&seed_only), "seed=42");

        let both = GenerationParams {
            temperature: Some(0.2),
            seed: Some(42),
            ..Default::default()
        };
        assert_eq!(settings_label(&both), "temp=0.2,seed=42");
    }

    #[test]
    fn settings_label_appends_thinking_effort_when_thinking_on() {
        let low = GenerationParams {
            temperature: Some(0.2),
            seed: None,
            enable_thinking: Some(true),
            reasoning_effort: Some(crate::config::ReasoningEffort::Low),
        };
        assert_eq!(settings_label(&low), "temp=0.2,think=low");

        let on_no_effort = GenerationParams {
            temperature: None,
            seed: None,
            enable_thinking: Some(true),
            reasoning_effort: None,
        };
        assert_eq!(settings_label(&on_no_effort), "think=on");

        let xhigh_full = GenerationParams {
            temperature: Some(0.2),
            seed: Some(42),
            enable_thinking: Some(true),
            reasoning_effort: Some(crate::config::ReasoningEffort::Xhigh),
        };
        assert_eq!(settings_label(&xhigh_full), "temp=0.2,seed=42,think=xhigh");
    }

    #[test]
    fn settings_label_unchanged_when_thinking_off_or_unrecorded() {
        let off = GenerationParams {
            temperature: Some(0.2),
            seed: None,
            enable_thinking: Some(false),
            reasoning_effort: None,
        };
        assert_eq!(settings_label(&off), "temp=0.2");

        let legacy = GenerationParams {
            temperature: None,
            seed: None,
            enable_thinking: None,
            reasoning_effort: None,
        };
        assert_eq!(settings_label(&legacy), "default");
    }

    #[test]
    fn run_id_is_eight_hex_chars() {
        let run = PhaseRun {
            ts: 1_000,
            model: "qwen".to_string(),
            phase_id: "phase-01".to_string(),
            ..Default::default()
        };
        let id = run_id(&run);
        assert_eq!(id.len(), 8);
        // Lowercase hex: every char is a hex digit and none is an uppercase
        // letter (numeric digits are not `is_lowercase`, so check that way).
        assert!(
            id.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
            "expected all lowercase hex digits, got: {id}"
        );
    }

    #[test]
    fn run_id_is_deterministic() {
        let run = PhaseRun {
            ts: 1_000,
            model: "qwen".to_string(),
            phase_id: "phase-01".to_string(),
            ..Default::default()
        };
        let id1 = run_id(&run);
        let id2 = run_id(&run);
        assert_eq!(id1, id2);
    }

    #[test]
    fn run_id_differs_on_ts_model_or_phase() {
        let base = PhaseRun {
            ts: 1_000,
            model: "qwen".to_string(),
            phase_id: "phase-01".to_string(),
            ..Default::default()
        };
        let base_id = run_id(&base);

        let mut ts_diff = base.clone();
        ts_diff.ts = 2_000;
        assert_ne!(run_id(&ts_diff), base_id, "changing ts should change id");

        let mut model_diff = base.clone();
        model_diff.model = "gemma".to_string();
        assert_ne!(
            run_id(&model_diff),
            base_id,
            "changing model should change id"
        );

        let mut phase_diff = base.clone();
        phase_diff.phase_id = "phase-02".to_string();
        assert_ne!(
            run_id(&phase_diff),
            base_id,
            "changing phase_id should change id"
        );
    }

    #[test]
    fn percentile_nearest_rank() {
        assert_eq!(percentile(&[], 0.5), 0);
        assert_eq!(percentile(&[], 0.9), 0);
        let single = vec![42];
        assert_eq!(percentile(&single, 0.5), 42);
        assert_eq!(percentile(&single, 0.9), 42);
        assert_eq!(percentile(&single, 0.99), 42);
        let eight = vec![1, 2, 3, 4, 5, 6, 7, 8];
        assert_eq!(percentile(&eight, 0.01), 1);
        assert_eq!(percentile(&eight, 0.1), 2);
        assert_eq!(percentile(&eight, 0.5), 5);
        assert_eq!(percentile(&eight, 0.9), 7);
        assert_eq!(percentile(&eight, 0.99), 8);
    }

    #[test]
    fn fmt_tokens_zero_is_dash() {
        assert_eq!(fmt_tokens(0), "—");
    }

    #[test]
    fn fmt_tokens_raw_below_thousand() {
        assert_eq!(fmt_tokens(999), "999");
    }

    #[test]
    fn fmt_tokens_thousands_one_decimal() {
        assert_eq!(fmt_tokens(12_288), "12.3k");
    }

    #[test]
    fn fmt_tokens_millions_tier() {
        assert_eq!(fmt_tokens(2_100_000), "2.1M");
    }

    #[test]
    fn fmt_tokens_boundary_at_thousand() {
        assert_eq!(fmt_tokens(1_000), "1.0k");
        assert!(fmt_tokens(999_999).ends_with('k'));
        assert_eq!(fmt_tokens(1_000_000), "1.0M");
    }
}
