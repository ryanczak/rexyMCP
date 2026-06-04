# Phase 11a: Budget panel — Tokens/Sec throughput

**Milestone:** M8 — Live session dashboard
**Status:** review
**Depends on:** phase-06a/06b (done — `SessionEvent::Metrics` and the Budget panel
this extends).
**Estimated diff:** ~110 lines (`mcp/src/status.rs` summarize + `mcp/src/dashboard.rs`
Budget line + tests).
**Tags:** language=rust, kind=feature, size=s

## Goal

Add a live **Tokens/Sec** generation-throughput line to the Budget panel (a wireframe
item). Each per-turn `SessionEvent::Metrics` record carries the session's **cumulative**
`output_tokens` plus the record's timestamp (`SessionRecord.ts`). The most recent
interval's throughput is therefore `Δoutput_tokens / Δseconds` between the last two
`Metrics` records — no new executor data, **no config**.

This is **11a** of a two-phase split of the original phase-11 (Budget metrics):

- **11a (this phase):** Tokens/Sec — pure JSONL-derived, mcp-crate only.
- **11b (next):** "$ saved" — needs a configurable `$/Mtok` rate from `rexymcp.toml`,
  which requires config-schema + plumbing the dashboard doesn't have today. Separate
  phase.

## Architecture references

Read before starting:

- M8 README § "Measurement roadmap" → Gap C names "tokens-per-second … derivable from
  record `ts` deltas but never computed." This phase computes it.
- `executor/src/store/sessions/event.rs` — `SessionEvent::Metrics { input_tokens,
  output_tokens, context_pct }`; `SessionRecord { ts, turn, event }`. The token counts
  are **cumulative** across the session (see `StatusSummary`'s existing doc comments and
  the executor's `tokens_accumulate_across_done_events` test).

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read `mcp/src/status.rs` end to end (small) — you add three `StatusSummary` fields
   and extend the `Metrics` fold arm.
3. Read `mcp/src/dashboard.rs`'s `budget_lines` (small) — you add one line + a pure
   helper.
4. Read this entire phase doc before touching code.
5. Confirm `cargo clippy --all-targets --all-features -- -D warnings` and
   `cargo test` are green before changing anything.

## Current state

### The `Metrics` fold today (`mcp/src/status.rs`)

The fold loop binds `rec` (used at the top for `summary.last_ts = … rec.ts`), so
`rec.ts` is in scope in the `Metrics` arm:

```rust
            SessionEvent::Metrics {
                input_tokens,
                output_tokens,
                context_pct,
            } => {
                summary.last_input_tokens = Some(*input_tokens);
                summary.last_output_tokens = Some(*output_tokens);
                summary.last_context_pct = Some(*context_pct);
            }
```

`StatusSummary` is a flat struct of `Option` fields; the budget fields (06b) show the
convention:

```rust
    /// Cumulative input tokens from the most recent `Metrics` record.
    pub last_input_tokens: Option<u32>,
    /// Cumulative output tokens from the most recent `Metrics` record.
    pub last_output_tokens: Option<u32>,
    /// Context-window fraction … from the most recent `Metrics`; `None` = no metrics yet.
    pub last_context_pct: Option<f64>,
```

### `budget_lines` today (`mcp/src/dashboard.rs`)

```rust
    if summary.last_input_tokens.is_none() {
        return vec![Line::from("(no metrics yet)")];
    }
    let in_toks = summary.last_input_tokens.unwrap_or(0);
    let out_toks = summary.last_output_tokens.unwrap_or(0);
    let mut lines = vec![
        Line::from(format!("tokens in:  {in_toks}")),
        Line::from(format!("tokens out: {out_toks}")),
    ];
    if let Some(pct) = summary.last_context_pct {
        … context line …
    }
    lines
```

The existing pure-scalar helpers `clamp_scroll` / `visible_offset` in `dashboard.rs`
are the model for a small, unit-testable computation taking scalar args (not the whole
summary), which the panel calls.

## Spec

Numbered tasks. **mcp-crate only**, no executor change, no config, no new deps.

1. **Track the previous + latest `Metrics` snapshot in `summarize`** — in
   `mcp/src/status.rs`, add three `StatusSummary` fields (after the budget fields):
   ```rust
       /// Unix-millis timestamp of the most recent `Metrics` record.
       pub last_metrics_ts: Option<u64>,
       /// Timestamp of the *second*-most-recent `Metrics` record.
       pub prev_metrics_ts: Option<u64>,
       /// Cumulative output tokens at the second-most-recent `Metrics` record.
       pub prev_output_tokens: Option<u32>,
   ```
   In the `Metrics` arm, **shift the current latest into "prev" before overwriting**
   (order matters — read the old `last_*` values first):
   ```rust
           SessionEvent::Metrics {
               input_tokens,
               output_tokens,
               context_pct,
           } => {
               // Shift the prior latest snapshot into "prev" for throughput.
               summary.prev_metrics_ts = summary.last_metrics_ts;
               summary.prev_output_tokens = summary.last_output_tokens;
               summary.last_metrics_ts = Some(rec.ts);
               summary.last_input_tokens = Some(*input_tokens);
               summary.last_output_tokens = Some(*output_tokens);
               summary.last_context_pct = Some(*context_pct);
           }
   ```
   (First `Metrics`: `prev_*` stay `None`. Second onward: `prev_*` = the one before.)

2. **Add a pure `tokens_per_sec` helper** — in `mcp/src/dashboard.rs`, taking scalar
   args (mirrors `visible_offset`'s scalar shape for testability):
   ```rust
   /// Recent generation throughput: cumulative output tokens gained over the most
   /// recent `Metrics` interval, divided by that interval's wall-clock seconds.
   /// `None` until two `Metrics` records exist, or if the interval is zero-length.
   fn tokens_per_sec(
       prev_ts: Option<u64>,
       prev_out: Option<u32>,
       last_ts: Option<u64>,
       last_out: Option<u32>,
   ) -> Option<f64> {
       let dt_ms = last_ts?.checked_sub(prev_ts?)?;
       if dt_ms == 0 {
           return None;
       }
       let d_out = last_out?.saturating_sub(prev_out?);
       Some(d_out as f64 / (dt_ms as f64 / 1000.0))
   }
   ```
   **Negative/edge cases to honor** (pin these): `None` when either snapshot is missing
   (`prev_*` is `None` on the first metric); `None` (not a divide-by-zero / NaN) when
   `last_ts == prev_ts`; `0.0` is a valid result when `Δoutput == 0` over a real interval.

3. **Render the `tok/s` line in `budget_lines`** — insert **after** the `tokens out:`
   line and **before** the context line:
   ```rust
   match tokens_per_sec(
       summary.prev_metrics_ts,
       summary.prev_output_tokens,
       summary.last_metrics_ts,
       summary.last_output_tokens,
   ) {
       Some(rate) => lines.push(Line::from(format!("tok/s: {rate:.1}"))),
       None => lines.push(Line::from("tok/s: —")),
   }
   ```
   (The `—` placeholder shows once metrics exist but before a second sample arrives,
   matching the existing `context: — (unmeasured)` style. The early
   `(no metrics yet)` return is unchanged, so `tok/s` only appears once there is ≥1
   metric.) No color requirement for this phase.

## Acceptance criteria

- [ ] `StatusSummary` has `last_metrics_ts`, `prev_metrics_ts`, `prev_output_tokens`;
      the `Metrics` fold shifts latest→prev correctly.
- [ ] `tokens_per_sec` returns the interval rate, `None` for <2 samples or a
      zero-length interval, and `0.0` for a real interval with no new output.
- [ ] `budget_lines` shows `tok/s: {rate:.1}` with two samples and `tok/s: —` with one.
- [ ] `cargo build` clean; clippy `-D warnings` clean; `cargo fmt --all --check` clean
      (use `rustfmt` only on touched files); `cargo test -p rexymcp` passes.

## Test plan

Add to the `#[cfg(test)] mod tests` blocks (`status.rs` for the fold, `dashboard.rs`
for the helper + panel). Reuse the existing `metrics(input, output, context_pct)` test
helper in `status.rs` and the `rec(ts, turn, event)` helper.

- `summarize_tracks_prev_and_last_metrics` (status.rs) — two `Metrics` records at
  `ts=1000` (out 100) and `ts=3000` (out 300) → `prev_metrics_ts == Some(1000)`,
  `prev_output_tokens == Some(100)`, `last_metrics_ts == Some(3000)`,
  `last_output_tokens == Some(300)`.
- `summarize_one_metric_has_no_prev` (status.rs) — a single `Metrics` →
  `prev_metrics_ts == None`, `prev_output_tokens == None`.
- `tokens_per_sec_computes_recent_rate` (dashboard.rs) —
  `tokens_per_sec(Some(1000), Some(100), Some(3000), Some(300))` → `Some(100.0)`
  (200 tokens over 2.0 s).
- `tokens_per_sec_none_without_two_samples` (dashboard.rs) —
  `tokens_per_sec(None, None, Some(3000), Some(300)) == None`.
- `tokens_per_sec_none_on_zero_interval` (dashboard.rs) —
  `tokens_per_sec(Some(1000), Some(100), Some(1000), Some(300)) == None` (the
  must-NOT-divide-by-zero negative case).
- `tokens_per_sec_zero_when_no_new_output` (dashboard.rs) —
  `tokens_per_sec(Some(1000), Some(300), Some(3000), Some(300)) == Some(0.0)`.
- `budget_lines_shows_tokens_per_sec` (dashboard.rs) — a summary with both metric
  snapshots (rate 100) → lines contain `tok/s: 100.0`.
- `budget_lines_tokens_per_sec_dash_with_one_sample` (dashboard.rs) — a summary with
  `last_*` set but `prev_*` `None` → lines contain `tok/s: —` and **not** a numeric rate.

## End-to-end verification

The throughput math and the panel line are proven by the unit tests (they exercise the
real `summarize` fold and `budget_lines`). The on-screen render needs a TTY. Verify:

1. Paste `cargo test -p rexymcp` output covering the new `tokens_per_sec_*`,
   `summarize_*_metrics`, and `budget_lines_*` tests.
2. Build and launch against a session log with ≥2 `Metrics` records:
   `cargo run -p rexymcp -- dashboard --repo .` — confirm the Budget panel shows a
   `tok/s:` line with a number (and quote it). If the newest log has <2 metrics, note
   that the `—` placeholder shows instead.

## Authorizations

None. No new dependency. No `docs/architecture.md` / `STANDARDS.md` / `WORKFLOW.md`
edit. No executor-crate or config change (that's 11b).

## Out of scope

- **"$ saved"** — needs a configurable `$/Mtok` rate from `rexymcp.toml` + dashboard
  config plumbing; that is **phase-11b**. Do not add config here.
- **Smoothing / averaging** the rate (EMA, windowed) — the recent-interval value is the
  first cut; revisit only if it reads too jumpy live.
- **Per-turn decode-only throughput** (excluding tool/verify wall-time) — not derivable
  from the current data; the interval rate includes turn overhead by design.
- **Any change** to the Activity / Session / Compactions / Files panels or `summarize`'s
  other arms.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2025-07-18 (started)

**Executor:** rexyMCP agent

Implementing Tokens/Sec throughput in the Budget panel: three new `StatusSummary` fields, `tokens_per_sec` helper, `budget_lines` integration, and 8 tests.

### Update — 2025-07-18 (complete)

**Executor:** rexyMCP agent

**Summary:** Added `tok/s` throughput line to the Budget panel.

**Files changed:**
- `mcp/src/status.rs` — added `last_metrics_ts`, `prev_metrics_ts`, `prev_output_tokens` to `StatusSummary`; extended `Metrics` fold arm to shift latest→prev; added 2 tests (`summarize_tracks_prev_and_last_metrics`, `summarize_one_metric_has_no_prev`).
- `mcp/src/dashboard.rs` — added `tokens_per_sec` pure helper; inserted `tok/s` line in `budget_lines` after `tokens out:` and before context; added 6 tests (`tokens_per_sec_computes_recent_rate`, `tokens_per_sec_none_without_two_samples`, `tokens_per_sec_none_on_zero_interval`, `tokens_per_sec_zero_when_no_new_output`, `budget_lines_shows_tokens_per_sec`, `budget_lines_tokens_per_sec_dash_with_one_sample`).

**Verification commands:**
```
cargo fmt --all --check → clean
cargo build → clean
cargo clippy --all-targets --all-features -- -D warnings → clean
cargo test -p rexymcp → 207 passed; 0 failed; 0 ignored
```

**Verification summary:** fmt clean, build clean, clippy clean, all 207 tests pass (including 8 new).

**Grep for spec-pinned literal:** `grep -rn 'tok/s:' mcp/src/dashboard.rs` → 5 matches (2 in `budget_lines`, 3 in tests) — literal landed correctly.

**End-to-end verification:** Unit tests exercise the real `summarize` fold and `budget_lines` with both one-sample and two-sample summaries. The `tok/s: —` placeholder and `tok/s: {rate:.1}` numeric output are both verified. Live TTY render not tested in CI (no TTY available).

**Notes for review:** None. Implementation matches spec exactly.
