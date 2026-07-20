//! Shared derivations over telemetry records — the single home for every
//! *derived* metric number (reclaimed sums, tok/s, settings labels, cost).
//! Readers (`runs`/`scorecard`/`status`/dashboard) call these instead of
//! re-deriving; pinning the definition once.

use crate::ai::types::TokenBreakdown;
use crate::store::telemetry::{ContextEfficiency, GenerationParams, ModelRates, PhaseRun};

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
/// `"temp=T,seed=S"`. The exact strings `runs`/`scorecard` render.
pub fn settings_label(params: &GenerationParams) -> String {
    match (params.temperature, params.seed) {
        (None, None) => "default".to_string(),
        (Some(t), None) => format!("temp={t}"),
        (None, Some(s)) => format!("seed={s}"),
        (Some(t), Some(s)) => format!("temp={t},seed={s}"),
    }
}

/// USD cost of an executor `TokenBreakdown` at per-class rates. Mirrors
/// `ArchitectTokens::cost`; note `cache_write_tokens` is the cache-creation
/// class.
pub fn token_cost(tokens: &TokenBreakdown, rates: &ModelRates) -> f64 {
    let per_m = |toks: u32, rate: f64| (toks as f64 / 1_000_000.0) * rate;
    per_m(tokens.input_tokens, rates.input_per_mtok)
        + per_m(tokens.cache_write_tokens, rates.cache_creation_per_mtok)
        + per_m(tokens.cache_read_tokens, rates.cache_read_per_mtok)
        + per_m(tokens.output_tokens, rates.output_per_mtok)
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
        };
        assert_eq!(settings_label(&default_params), "default");

        let temp_only = GenerationParams {
            temperature: Some(0.2),
            seed: None,
        };
        assert_eq!(settings_label(&temp_only), "temp=0.2");

        let seed_only = GenerationParams {
            temperature: None,
            seed: Some(42),
        };
        assert_eq!(settings_label(&seed_only), "seed=42");

        let both = GenerationParams {
            temperature: Some(0.2),
            seed: Some(42),
        };
        assert_eq!(settings_label(&both), "temp=0.2,seed=42");
    }

    #[test]
    fn token_cost_prices_each_class() {
        let tokens = TokenBreakdown {
            input_tokens: 1_000_000,
            output_tokens: 1_000_000,
            cache_read_tokens: 1_000_000,
            cache_write_tokens: 1_000_000,
        };
        let rates = ModelRates {
            input_per_mtok: 1.0,
            output_per_mtok: 2.0,
            cache_read_per_mtok: 3.0,
            cache_creation_per_mtok: 4.0,
        };
        assert_eq!(token_cost(&tokens, &rates), 10.0);
    }

    #[test]
    fn token_cost_zero_when_unpriced() {
        let tokens = TokenBreakdown {
            input_tokens: 1_000_000,
            output_tokens: 500_000,
            cache_read_tokens: 100_000,
            cache_write_tokens: 200_000,
        };
        let rates = ModelRates::default();
        assert_eq!(token_cost(&tokens, &rates), 0.0);
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
}
