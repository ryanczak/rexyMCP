# Phase 06c-ii: Per-model architect pricing — built-in Claude table + 5m/1h cache-write split

**Milestone:** M35 — Metrics & Cost Accounting Overhaul
**Status:** review
**Depends on:** phase-06c-i
**Estimated diff:** ~230 lines
**Tags:** language=rust, kind=feature, size=m

## Goal

Price an `ArchitectLedger` record (06c-i) at the correct USD cost for **its own
model**, honouring the **5-minute vs 1-hour cache-write split** the ledger now
carries. Today architect pricing is single-model (one `[architect] model`) and
collapses cache-writes to a single 1.25× rate; the ledger is multi-model with a
`cache_creation_5m` / `cache_creation_1h` split, and 1-hour cache writes cost
**2×** input, not 1.25×.

This phase adds: (1) the **1-hour cache-write multiplier**, (2) an
`ArchitectLedger::cost` method that prices the 5m/1h split per the ledger's model,
(3) the missing **`claude-sonnet-5`** entry in the built-in Claude price table, and
(4) a **`[architect.rates]`** per-model config override so a user can correct or add
a model's base rate.

**This is 06c-ii — the pricing core only.** It is additive: nothing consumes
`ArchitectLedger::cost` yet. **06c-iii** rewires the surfaces (`costs`, dashboard
Budget, `profile`) onto the ledger + this cost fn. Do **not** touch `costs.rs`, the
dashboard, or `profile` here (see Out of scope).

## Architecture references

Read before starting:

- `docs/dev/milestones/M35-metrics-cost-accounting/README.md` — the "Architect-ledger
  design pass" note (built-in price table + config override + 5m/1h split) and the
  06c phase rows.
- `executor/src/store/telemetry.rs` — the cache multipliers (~417–422), `ArchitectRates`
  + `ArchitectTokens::cost` (~441–459), and the **`ArchitectLedger`** struct + its
  `cache_creation_5m`/`cache_creation_1h` fields (added in 06c-i, ~466+). Quoted below.
- `executor/src/config.rs` — `known_model_rates` (10–18), `ArchitectConfig` (73–146)
  incl. `effective_architect_rates` (the derive-cache-from-input pattern to mirror).
- `mcp/src/init.rs` — the `[architect]` template block (86–95) to extend.

**Authoritative Anthropic pricing (fetched 2026-07-21 from
`platform.claude.com/docs/en/docs/about-claude/pricing`):**

- **Cache-write/read multipliers relative to base input** (confirmed): 5-minute
  cache write = **1.25×**, 1-hour cache write = **2×**, cache read (hit) = **0.1×**.
- **Base (input, output) $/Mtok:** Fable 5 = (10, 50); Opus 4.8/4.7/4.6/4.5 = (5, 25);
  **Sonnet 5 = (2, 10)** introductory through 2026-08-31, then (3, 15) from
  2026-09-01; Sonnet 4.6/4.5 = (3, 15); Haiku 4.5 = (1, 5). The existing
  `known_model_rates` entries are all correct; **only `claude-sonnet-5` is missing.**
- **Cross-check columns (use in tests):** Opus 4.8 5m-write = $6.25/Mtok, 1h-write =
  **$10/Mtok**, cache-read = $0.50/Mtok. (These are `input × {1.25, 2.0, 0.1}` = `5 ×
  …`.)

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.
5. **Test placement rule:** unit tests go **inside the existing `#[cfg(test)] mod
   tests { … }` block at the bottom of the same file** — `telemetry.rs` tests in
   `telemetry.rs`, `config.rs` tests in `config.rs`. Never at file scope beside
   `mod tests`.
6. **Editing discipline:** edit with `write_file`/`patch`/`patch_lines`. To view a
   file/line range use `read_file` (`start_line`/`end_line`), **not** `sed -n`/`cat`.
   **Never run the same read command twice** — read a region once, then act; repeated
   identical calls trip the governor's identical-repetition detector.

## Current state

**Cache multipliers** (telemetry.rs ~417–422) — the 1-hour rate is missing; its comment
even flags the gap:

```rust
/// Anthropic prompt-cache rate multipliers relative to the base input rate:
/// a 5-minute cache **write** costs 1.25× input; a cache **read** costs 0.1×
/// input. (1-hour cache writes cost 2× input — approximated here as the 1.25×
/// standard; a dedicated 1h rate can be added additively later.)
pub const CACHE_CREATION_RATE_MULTIPLIER: f64 = 1.25;
pub const CACHE_READ_RATE_MULTIPLIER: f64 = 0.1;
```

**`ArchitectTokens::cost`** (telemetry.rs ~450–459) — the per-Mtok math shape to mirror
(note `per_m`):

```rust
impl ArchitectTokens {
    pub fn cost(&self, rates: &ArchitectRates) -> f64 {
        let per_m = |toks: u64, rate: f64| (toks as f64 / 1_000_000.0) * rate;
        per_m(self.input, rates.input_per_mtok)
            + per_m(self.cache_creation, rates.cache_creation_per_mtok)
            + per_m(self.cache_read, rates.cache_read_per_mtok)
            + per_m(self.output, rates.output_per_mtok)
    }
}
```

**`ArchitectLedger`** (telemetry.rs, from 06c-i) — the type you add `cost` to. Relevant
fields: `model: String`, `tokens: ArchitectTokens` (with `input`/`output`/`cache_read`;
its `cache_creation` is the **total** = 5m+1h — do **not** use it for cost),
`cache_creation_5m: u64`, `cache_creation_1h: u64`.

**Built-in price table** (config.rs 10–18) — missing `claude-sonnet-5`:

```rust
pub fn known_model_rates(model: &str) -> Option<(f64, f64)> {
    match model {
        "claude-fable-5" | "claude-mythos-5" => Some((10.0, 50.0)),
        "claude-opus-4-8" | "claude-opus-4-7" | "claude-opus-4-6" => Some((5.0, 25.0)),
        "claude-sonnet-4-6" => Some((3.0, 15.0)),
        "claude-haiku-4-5" | "claude-haiku-4-5-20251001" => Some((1.0, 5.0)),
        _ => None,
    }
}
```

**`ArchitectConfig`** (config.rs 73–146) — `#[serde(default)]` struct with a manual
`impl Default` (99–108) you must extend when adding a field, plus the
`effective_architect_rates` derive-cache-from-input pattern (125–145) to mirror.

## Spec

### Task 1 — 1-hour cache-write multiplier (telemetry.rs)

Add next to the existing multipliers, and **correct the stale 5m comment** (drop the
"approximated as 1.25" note now that a real 1h rate exists):

```rust
/// Anthropic prompt-cache rate multipliers relative to the base input rate:
/// a **5-minute** cache write costs 1.25× input, a **1-hour** cache write costs 2×
/// input, and a cache **read** (hit) costs 0.1× input.
pub const CACHE_CREATION_RATE_MULTIPLIER: f64 = 1.25;
pub const CACHE_CREATION_1H_RATE_MULTIPLIER: f64 = 2.0;
pub const CACHE_READ_RATE_MULTIPLIER: f64 = 0.1;
```

### Task 2 — `ArchitectLedger::cost` (telemetry.rs)

Add a method that prices the record from its base `(input, output)` $/Mtok rates,
deriving the three cache rates from the input rate via the multipliers. **Price the
5m/1h split** (`cache_creation_5m` / `cache_creation_1h`), **not** `tokens.cache_creation`:

```rust
impl ArchitectLedger {
    /// USD cost of this ledger slice at the given base `(input, output)` $/Mtok
    /// rates. Cache rates derive from the input rate via the standard Anthropic
    /// multipliers (read 0.1×, 5m-write 1.25×, 1h-write 2×), pricing the 5m and 1h
    /// cache-write buckets separately.
    pub fn cost(&self, input_per_mtok: f64, output_per_mtok: f64) -> f64 {
        let per_m = |toks: u64, rate: f64| (toks as f64 / 1_000_000.0) * rate;
        per_m(self.tokens.input, input_per_mtok)
            + per_m(self.tokens.output, output_per_mtok)
            + per_m(self.tokens.cache_read, input_per_mtok * CACHE_READ_RATE_MULTIPLIER)
            + per_m(self.cache_creation_5m, input_per_mtok * CACHE_CREATION_RATE_MULTIPLIER)
            + per_m(self.cache_creation_1h, input_per_mtok * CACHE_CREATION_1H_RATE_MULTIPLIER)
    }
}
```

### Task 3 — add `claude-sonnet-5` to the built-in table (config.rs)

Add one arm to `known_model_rates` with the introductory rate + a comment flagging the
scheduled change (the `[architect.rates]` override handles the post-switch value):

```rust
        // Introductory pricing through 2026-08-31; standard (3.0, 15.0) from
        // 2026-09-01 — override via [architect.rates] after the switch.
        "claude-sonnet-5" => Some((2.0, 10.0)),
```

### Task 4 — `[architect.rates]` per-model override (config.rs + init.rs)

**`ArchitectModelRate`** — a new small struct near `ArchitectConfig`:

```rust
/// A per-model architect price override (a `[architect.rates."<model>"]` entry):
/// base `(input, output)` $/Mtok that overrides `known_model_rates` for that model.
/// Cache rates always derive from `input` via the standard multipliers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ArchitectModelRate {
    pub input_per_mtok: f64,
    pub output_per_mtok: f64,
}
```

**On `ArchitectConfig`:** add `pub rates: std::collections::HashMap<String,
ArchitectModelRate>` (add `rates: HashMap::new()` to the manual `impl Default`), and a
resolver:

```rust
    /// Resolved base `(input, output)` $/Mtok for **any** architect model by ID: a
    /// `[architect.rates]` override wins, else the built-in `known_model_rates`.
    /// `None` when neither knows the model (the cost is then $0; surfaces render "—").
    pub fn rates_for(&self, model: &str) -> Option<(f64, f64)> {
        self.rates
            .get(model)
            .map(|r| (r.input_per_mtok, r.output_per_mtok))
            .or_else(|| known_model_rates(model))
    }
```

**`mcp/src/init.rs`** — after the `[architect]` block (after line ~95, before
`[telemetry]`), add a commented example so users discover the override:

```
# [architect.rates."claude-sonnet-5"]   # override/add a per-model architect rate
# input_per_mtok = 3.0                   #   (cache rates derive: read 0.1x, 5m 1.25x, 1h 2x)
# output_per_mtok = 15.0
```

## Acceptance criteria

- [ ] `CACHE_CREATION_1H_RATE_MULTIPLIER == 2.0` exists; the 5m-multiplier comment no
      longer claims the 1h rate is approximated.
- [ ] `ArchitectLedger::cost(input, output)` prices input + output + cache_read (0.1×)
      + `cache_creation_5m` (1.25×) + `cache_creation_1h` (2×), using the **split**
      fields (not `tokens.cache_creation`). Concretely (opus base input 5.0): 1_000_000
      `cache_creation_1h` tokens ⇒ **$10.00**; 1_000_000 `cache_creation_5m` ⇒ **$6.25**;
      1_000_000 `cache_read` ⇒ **$0.50**.
- [ ] `known_model_rates("claude-sonnet-5") == Some((2.0, 10.0))`; the other entries
      are unchanged.
- [ ] `ArchitectConfig::rates_for` returns the `[architect.rates]` override when present,
      else `known_model_rates`, else `None` for an unrecognised model with no override.
- [ ] `cargo fmt --all --check`, `cargo build`,
      `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test` all pass.
- [ ] `mcp/src/costs.rs`, the dashboard, and `profile` are **untouched** (grep the diff).

## Test plan

In `executor/src/store/telemetry.rs` `mod tests`:

- **`architect_ledger_cost_prices_5m_and_1h_split`** — build an `ArchitectLedger` (opus,
  base input 5.0 / output 25.0) with `cache_creation_5m = 1_000_000`,
  `cache_creation_1h = 1_000_000`, `tokens.cache_read = 1_000_000`, `tokens.input =
  1_000_000`, `tokens.output = 1_000_000`; assert `l.cost(5.0, 25.0)` equals the
  hand-sum `5.00 + 25.00 + 0.50 + 6.25 + 10.00 = $46.75` (use an epsilon compare on
  the f64). **Mutation-sensitive:** it must fail if the 1h term uses the 1.25×
  multiplier (would give $6.25 not $10.00) or prices `tokens.cache_creation` instead
  of the split.
- **`architect_ledger_cost_ignores_total_cache_creation`** — a ledger where
  `cache_creation_5m + cache_creation_1h` is set but `tokens.cache_creation` is left 0
  (an inconsistent record); assert the cost still reflects the 5m/1h split (proving
  `cost` reads the split, not the total).

In `executor/src/config.rs` `mod tests`:

- **`known_model_rates_prices_sonnet_5`** — `known_model_rates("claude-sonnet-5") ==
  Some((2.0, 10.0))`, and a spot-check that an existing entry (e.g. opus-4-8) is still
  `Some((5.0, 25.0))`.
- **`architect_rates_for_override_wins_then_known_then_none`** — an `ArchitectConfig`
  with a `[architect.rates]` entry for `"claude-opus-4-8"` at `(9.9, 9.9)`: assert
  `rates_for("claude-opus-4-8") == Some((9.9, 9.9))` (override beats the built-in
  5/25), `rates_for("claude-sonnet-5") == Some((2.0, 10.0))` (falls through to the
  table), and `rates_for("some-local-model") == None`.

All tests hermetic; no TempDir needed (pure functions). f64 asserts use an epsilon
(e.g. `(a - b).abs() < 1e-9`).

## End-to-end verification

No CLI/dashboard surface ships this phase (06c-iii adds the consumer), so the
real-artifact check is that `ArchitectLedger::cost` matches **Anthropic's published
per-Mtok rate columns**: the `architect_ledger_cost_prices_5m_and_1h_split` test pins
opus 1h-write = $10.00 / 5m-write = $6.25 / read = $0.50 per Mtok, exactly the values
in the fetched pricing table (§Architecture references). Quote the passing test output
in the completion Update Log as the verification, and note explicitly that the cost fn
has no runtime consumer until 06c-iii.

## Authorizations

- Editing `executor/src/store/telemetry.rs`, `executor/src/config.rs`, and
  `mcp/src/init.rs` is in scope.
- **No new dependencies** (`std::collections::HashMap` is already used in config.rs).
- No `docs/architecture.md` / `STANDARDS.md` / `WORKFLOW.md` / other-phase-doc edits.
- No `unsafe`, no `#[allow]`, no new `unwrap`/`expect`/`panic!` in production paths.

## Out of scope

- **Surfaces.** Do **not** touch `mcp/src/costs.rs` (`scope_report`/`scope_costs`), the
  dashboard Budget panel, or `profile`. **06c-iii** rewires them onto the ledger +
  `ArchitectLedger::cost`. This phase ships the cost fn with no runtime consumer — that
  is expected.
- **Re-pricing existing known models.** Fable/opus/sonnet-4-6/haiku entries are correct
  per the fetched table; only **add** sonnet-5. Do not change existing rates.
- **The single-model `[architect]` path.** `effective_architect_rates` /
  `effective_rates` and the `ArchitectTokens::cost` method stay as they are (still used
  by the pre-ledger path until 06c-iii retires it). Do not refactor them.
- **The introductory→standard sonnet-5 switch automation.** Ship the introductory
  `(2.0, 10.0)` with the comment; a date-aware switch is not in scope (the override is
  the mechanism).

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
### Update — ts=1784688834134 (complete, server-authored)

**Summary:** Summary + Notes for review:

This phase implemented the architect pricing core: (1) added `CACHE_CREATION_1H_RATE_MULTIPLIER = 2.0` and corrected the stale comment; (2) added `ArchitectLedger::cost(input_per_mtok, output_per_mtok)` that prices the 5m/1h cache-write split separately using the standard Anthropic multipliers; (3) added `claude-sonnet-5` to the built-in price table at `(2.0, 10.0)`; (4) added `ArchitectModelRate` struct and `ArchitectConfig.rates_for()` for per-model `[architect.rates]` config overrides, with a commented example in the init template. All four tests pass: `architect_ledger_cost_prices_5m_and_1h_split` verifies the $46.75 hand-sum (including $10.00 for 1h-write, $6.25 for 5m-write, $0.50 for cache-read), `architect_ledger_cost_ignores_total_cache_creation` confirms the split fields are used over `tokens.cache_creation`, and the config tests verify sonnet-5 pricing and the override fallback chain. `cargo fmt`, `cargo build`, `cargo clippy`, and `cargo test` all pass clean. No surfaces (`costs.rs`, dashboard, `profile`) were touched — this phase ships the cost function with no runtime consumer until 06c-iii.

**Gates:** format=run, build=run, lint=run, test=run

**Command output tails:**

```
FORMAT


BUILD
   Compiling rexymcp-executor v0.9.1 (/home/matt/src/rexyMCP/executor)
   Compiling rexymcp v0.9.1 (/home/matt/src/rexyMCP/mcp)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.65s


LINT
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.08s


TEST
ips_pending_task_to_active ... ok
test tools::update_task::tests::invalid_state_returns_advisory_error ... ok
test tools::update_task::tests::malformed_args_returns_advisory_error ... ok
test tools::update_task::tests::metadata_shape_is_unchanged ... ok
test tools::update_task::tests::result_flags_redundant_remark ... ok
test tools::update_task::tests::null_args_returns_recovery_hint ... ok
test tools::update_task::tests::result_lists_remaining_incomplete_ids ... ok
test tools::update_task::tests::result_reports_all_complete_when_last_done ... ok
test tools::update_task::tests::success_output_names_task ... ok
test tools::update_task::tests::unknown_id_returns_advisory_error ... ok
test tools::write_file::tests::append_creates_file_if_missing ... ok
test tools::write_file::tests::append_false_overwrites ... ok
test tools::symbols::tests::exact_match_no_substring ... ok
test tools::write_file::tests::appends_to_existing_file ... ok
test tools::write_file::tests::creates_new_file ... ok
test tools::write_file::tests::missing_path_returns_recovery_hint ... ok
test tools::write_file::tests::non_object_args_do_not_panic ... ok
test tools::symbols::tests::caps_at_max_results ... ok
test tools::symbols::tests::references_single_file_path ... ok
test tools::symbols::tests::references_no_matches_advisory ... ok
test tools::symbols::tests::references_respects_max_results ... ok
test tools::write_file::tests::rejects_malformed_args ... ok
test tools::write_file::tests::reports_missing_parent_dir ... ok
test tools::write_file::tests::overwrites_existing_file ... ok
test tools::write_file::tests::scope_escape_returns_advisory_error_and_writes_nothing ... ok
test tools::write_file::tests::success_output_includes_line_count ... ok
test tools::symbols::tests::kind_filter_returns_only_matching_kind ... ok
test tools::bash::tests::cargo_command_records_cargo_filter_label ... ok
test tools::symbols::tests::references_truncation_note_omits_kind_filter ... ok
test tools::symbols::tests::references_snippet_shows_source_line ... ok
test tools::symbols::tests::metadata_carries_definitions_and_files_count ... ok
test tools::symbols::tests::reports_line_and_column ... ok
test ai::backends::openai::tests::is_retriable_transport_true_for_reqwest_error ... ok
test tools::symbols::tests::references_across_multiple_files ... ok
test tools::symbols::tests::respects_gitignore ... ok
test tools::symbols::tests::unsupported_extension_skipped_in_dir_walk ... ok
test tools::symbols::tests::finds_rust_struct_and_trait ... ok
test governor::verifier::tests::verify_rust_returns_checked_empty_on_clean_code ... ok
test governor::verifier::tests::capture_baseline_dedupes_by_project_root ... ok
test governor::verifier::tests::capture_baseline_skips_unsupported_files ... ok
test governor::verifier::tests::verify_rust_returns_checked_with_errors_on_broken_code ... ok
test tools::bash::tests::cargo_command_output_is_filtered_through_cargo_filter ... ok
test ai::backends::openai::tests::midstream_stall_is_not_retried ... ok
test ai::tests::stream_next_uses_supplied_timeout ... ok
test ai::backends::openai::tests::first_token_stall_retries_then_succeeds ... ok
test tools::bash::tests::arg_timeout_overrides_constructor_default ... ok
test tools::bash::tests::default_timeout_used_when_arg_absent ... ok
test tools::bash::tests::times_out_advisory_failure ... ok
test ai::backends::openai::tests::first_token_stall_exhausts_retries_then_errors ... ok
test health::tests::check_returns_unreachable_on_connection_error ... ok

test result: ok. 1031 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 6.09s


running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.08s
     Running unittests src/main.rs (target/debug/deps/rexymcp-4e85b51f198fbe9f)
     Running unittests src/lib.rs (target/debug/deps/executor-c1650299697d7408)
   Doc-tests executor

```

**Files changed:**
- `executor/src/config.rs` — +65 -1
- `executor/src/store/telemetry.rs` — +92 -3
- `mcp/src/init.rs` — +4 -0

**Commit:** 28e0fad440868e807621e90566902e0e78305587

**Notes:** server-authored completion entry (executor no longer owns the bookkeeping tail; see M27 phase-03).

