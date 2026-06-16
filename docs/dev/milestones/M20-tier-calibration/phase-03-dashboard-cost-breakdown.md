# Phase 03: Dashboard cost breakdown (Executor/Architect/Net per scope)

**Milestone:** M20 — Tier Calibration and Cost Visibility
**Status:** done
**Depends on:** phase-02 (tier_telemetry and EscalationEvent fields in PhaseRun)
**Estimated diff:** ~130 lines
**Tags:** language=rust, kind=feature, size=m

## Goal

Upgrade the Budget panel's Savings block from a three-scope gross row layout to a
five-row tabular breakdown — Baseline / Executor / Architect / Net — aligned in
Session / Milestone / Project columns, with a project Assists count below. This
gives honest accounting: the cloud baseline is gross savings before real costs;
Net subtracts Architect spend; Executor is always $0.00 today (no local rate
configured) but the row is always shown so the structure is ready for paid
providers (OpenRouter, etc.).

## Architecture references

Read before starting:

- `mcp/src/dashboard/panels.rs` — `BudgetRates`, `savings_lines`, and their
  tests. The primary files this phase rewrites.
- `mcp/src/dashboard/mod.rs` — `DashboardData`, `load_data`, and their tests.
  Introduces `ScopeCosts` and replaces the `(u32, u32)` savings tuples.
- `mcp/src/dashboard/render.rs` — `render_dashboard`: the production call site
  for `savings_lines` and the header-band height constant.
- `mcp/src/main.rs` — the one production `BudgetRates {…}` literal; gains
  architect rates.
- `executor/src/config.rs:93` — `ArchitectConfig::effective_rates()`, which
  supplies `(architect_input_per_mtok, architect_output_per_mtok)` to wire into
  `BudgetRates`.
- `executor/src/store/telemetry.rs` — `PhaseRun.tier_telemetry` (`TierTelemetry`
  struct from phase-02): `architect_input_tokens: u64`,
  `architect_output_tokens: u64`, `escalation_count: u32`. These are the source
  of the new cost columns.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read this entire phase doc before touching any code.
3. Run `cargo build` to confirm the tree is clean.

## Current state

### `mcp/src/dashboard/panels.rs` — `BudgetRates` and `savings_lines`

`BudgetRates` (line 17–22) is a two-field struct with `#[derive(Default)]`:

```rust
#[derive(Debug, Clone, Copy, Default)]
pub struct BudgetRates {
    pub input_per_mtok: f64,
    pub output_per_mtok: f64,
}
```

`savings_lines` (line 457–495) produces a header row plus up to three scope
rows (Session / Milestone / Project), each with a single dollar value.

### `mcp/src/dashboard/mod.rs` — `DashboardData` savings fields

```rust
pub struct DashboardData {
    pub summary: StatusSummary,
    pub records: Vec<SessionRecord>,
    pub error: Option<String>,
    pub milestone: Option<String>,
    pub milestone_savings: Option<(u32, u32)>,   // ← renamed + new type
    pub project_savings: (u32, u32),             // ← renamed + new type
}
```

`load_data` computes these by folding `PhaseRun.tokens.input_tokens` /
`output_tokens` (both `u32`) per project-id scope. The fold is at lines 49–60
(project) and 68–84 (milestone).

### `mcp/src/dashboard/render.rs` — `savings_lines` call (lines 157–165)

```rust
let mut budget = Vec::new();
budget.extend(budget_lines(&data.summary));
budget.extend(savings_lines(
    &data.summary,
    rates,
    data.milestone_savings,
    data.project_savings,
));
frame.render_widget(panel(" Budget ", budget), budget_area);
```

Header-band height: `Constraint::Length(11)` at render.rs:132.

### `mcp/src/main.rs` — single `BudgetRates` production literal (lines 568–572)

```rust
let (i, o) = cfg.dashboard.effective_rates();
let rates = dashboard::BudgetRates {
    input_per_mtok: i,
    output_per_mtok: o,
};
```

## Spec

### 1. `ScopeCosts` struct in `mcp/src/dashboard/panels.rs`

Add directly **above** `BudgetRates`:

```rust
/// Token costs for one budget scope (Session / Milestone / Project).
/// `executor_*` are local-model tokens (cost = $0.00 until a local rate is
/// configured; future: paid OpenRouter/provider rates). `architect_*` are summed
/// from `PhaseRun.tier_telemetry.architect_*_tokens`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ScopeCosts {
    pub executor_in: u64,
    pub executor_out: u64,
    pub architect_in: u64,
    pub architect_out: u64,
}
```

### 2. Extend `BudgetRates` in `mcp/src/dashboard/panels.rs`

Add two new fields (already derives `Default`, so 0.0 is the zero value):

```rust
#[derive(Debug, Clone, Copy, Default)]
pub struct BudgetRates {
    pub input_per_mtok: f64,              // cloud-baseline rate
    pub output_per_mtok: f64,             // cloud-baseline rate
    pub architect_input_per_mtok: f64,    // Architect model cost
    pub architect_output_per_mtok: f64,   // Architect model cost
}
```

`BudgetRates` already derives `Default`; adding two `f64 = 0.0` fields needs
no change to `Default`. All existing struct-literal sites that do not name the
new fields will get `E0063` — traverse them compiler-guided (Task 6).

### 3. Update `DashboardData` in `mcp/src/dashboard/mod.rs`

Replace the two `(u32, u32)` fields and add the escalation count:

```rust
    /// Cumulative executor + architect token costs from `PhaseRun` records whose
    /// `phase_doc_path` belongs to the active milestone. `None` when telemetry is
    /// absent, no phase is active, or no matching records exist.
    pub milestone_costs: Option<ScopeCosts>,
    /// Cumulative executor + architect token costs from ALL project `PhaseRun`
    /// records. `ScopeCosts::default()` when telemetry is not configured.
    pub project_costs: ScopeCosts,
    /// Sum of `PhaseRun.tier_telemetry.escalation_count` across all project runs.
    pub project_escalation_count: u32,
```

Add `pub use panels::{BudgetRates, ScopeCosts};` to the existing re-export line
(currently `pub use panels::BudgetRates;`).

### 4. Update `load_data` in `mcp/src/dashboard/mod.rs`

Replace the `project_savings` fold (lines ~49–60) with one that accumulates
all four `ScopeCosts` fields plus the escalation count. Executor tokens are
`u32` in `PhaseRun.tokens`; cast via `as u64`. Architect tokens and escalation
count are already in `PhaseRun.tier_telemetry` (added by phase-02).

```rust
// project_costs: executor tokens + architect tokens + escalation count
let (project_costs, project_escalation_count) = match project_id {
    Some(pid) => phase_runs
        .iter()
        .filter(|r| r.project_id.as_deref() == Some(pid))
        .fold(
            (ScopeCosts::default(), 0u32),
            |(mut costs, mut assists), r| {
                costs.executor_in = costs
                    .executor_in
                    .saturating_add(r.tokens.input_tokens as u64);
                costs.executor_out = costs
                    .executor_out
                    .saturating_add(r.tokens.output_tokens as u64);
                costs.architect_in = costs
                    .architect_in
                    .saturating_add(r.tier_telemetry.architect_input_tokens);
                costs.architect_out = costs
                    .architect_out
                    .saturating_add(r.tier_telemetry.architect_output_tokens);
                assists = assists
                    .saturating_add(r.tier_telemetry.escalation_count);
                (costs, assists)
            },
        ),
    None => (ScopeCosts::default(), 0u32),
};
```

Replace the `milestone_savings` fold (lines ~68–84) similarly, replacing
`(0u32, 0u32)` with `ScopeCosts::default()` and accumulating all four fields:

```rust
let milestone_costs = resolve_milestone_dir(repo, summary.phase.as_deref())
    .zip(project_id)
    .map(|(milestone_dir, pid)| {
        phase_runs
            .iter()
            .filter(|r| {
                r.project_id.as_deref() == Some(pid)
                    && r.milestone_id.as_deref() == Some(milestone_dir.as_str())
            })
            .fold(ScopeCosts::default(), |mut costs, r| {
                costs.executor_in = costs
                    .executor_in
                    .saturating_add(r.tokens.input_tokens as u64);
                costs.executor_out = costs
                    .executor_out
                    .saturating_add(r.tokens.output_tokens as u64);
                costs.architect_in = costs
                    .architect_in
                    .saturating_add(r.tier_telemetry.architect_input_tokens);
                costs.architect_out = costs
                    .architect_out
                    .saturating_add(r.tier_telemetry.architect_output_tokens);
                costs
            })
    })
    .filter(|c| c.executor_in > 0 || c.executor_out > 0
                || c.architect_in > 0 || c.architect_out > 0);
```

Update both `DashboardData {…}` struct literals — the `Ok` branch (lines ~85–92)
and the `Err` branch (lines ~94–101) — to use the new field names:

```rust
// Ok branch:
DashboardData {
    summary,
    records,
    error: None,
    milestone,
    milestone_costs,
    project_costs,
    project_escalation_count,
}
// Err branch:
DashboardData {
    summary: StatusSummary::default(),
    records: Vec::new(),
    error: Some(e),
    milestone: None,
    milestone_costs: None,
    project_costs,
    project_escalation_count,
}
```

### 5. Redesign `savings_lines` in `mcp/src/dashboard/panels.rs`

**New signature:**

```rust
pub(crate) fn savings_lines(
    summary: &StatusSummary,
    rates: BudgetRates,
    milestone_costs: Option<ScopeCosts>,
    project_costs: ScopeCosts,
    project_escalation_count: u32,
) -> Vec<Line<'static>>
```

**Format constants and helpers (inside the function body or as module-level consts):**

```rust
const LW: usize = 10; // label field width: "Architect:" is 10 chars (longest label)
const VW: usize = 9;  // value column width: right-align into 9 chars ("$XXXX.XX" = 8)
```

**Dollar-value helpers (closures or inline):**

```rust
// Inline dollar computation for u64 token counts:
let cost = |in_toks: u64, out_toks: u64, in_rate: f64, out_rate: f64| -> f64 {
    (in_toks as f64 / 1_000_000.0) * in_rate
        + (out_toks as f64 / 1_000_000.0) * out_rate
};
let fmt_dollars = |v: f64| format!("${v:.2}");
let no_baseline = rates.input_per_mtok == 0.0 && rates.output_per_mtok == 0.0;
// Baseline: "—" when unset; dollar amount otherwise.
let baseline_val = |in_toks: u64, out_toks: u64| -> String {
    if no_baseline { "—".to_string() }
    else { fmt_dollars(cost(in_toks, out_toks, rates.input_per_mtok, rates.output_per_mtok)) }
};
// Executor: always "$0.00" (no executor rate today; u64 tokens × 0.0 = 0).
let executor_val = |_in: u64, _out: u64| -> String { "$0.00".to_string() };
// Architect: dollar amount (0.00 when rates or tokens are zero).
let architect_val = |in_toks: u64, out_toks: u64| -> String {
    fmt_dollars(cost(in_toks, out_toks, rates.architect_input_per_mtok, rates.architect_output_per_mtok))
};
// Net: "—" when no baseline; Baseline − Executor − Architect otherwise.
let net_val = |b_in: u64, b_out: u64, a_in: u64, a_out: u64| -> String {
    if no_baseline { return "—".to_string(); }
    let baseline = cost(b_in, b_out, rates.input_per_mtok, rates.output_per_mtok);
    let architect = cost(a_in, a_out, rates.architect_input_per_mtok, rates.architect_output_per_mtok);
    // Executor = 0.0 today; subtract explicitly for future proofing.
    fmt_dollars(baseline - architect)
};
```

**Row and header builders:**

When `milestone_costs.is_some()` → 3-scope layout (Session + Milestone + Project):

```rust
// Header: format!("{:<12}{:>9}{:>9}{:>9}", "Savings", "Session", "Milestone", "Project")
// Row:    format!("  {:<10}{:>9}{:>9}{:>9}", label, v_sess, v_mile, v_proj)
```

When `milestone_costs.is_none()` → 2-scope layout (Session + Project):

```rust
// Header: format!("{:<12}{:>9}{:>9}", "Savings", "Session", "Project")
// Row:    format!("  {:<10}{:>9}{:>9}", label, v_sess, v_proj)
```

**Complete output structure (3-scope case):**

```
Savings       Session Milestone   Project    ← format!("{:<12}{:>9}{:>9}{:>9}", ...)
  Baseline:    $2.10     $8.30    $12.40    ← format!("  {:<10}{:>9}{:>9}{:>9}", ...)
  Executor:    $0.00     $0.00     $0.00
  Architect:   $0.00     $0.00     $0.00
  Net:         $2.10     $8.30    $12.40
  Assists: 0                                ← format!("  Assists: {}", count)
```

**Session scope tokens:** executor tokens from `summary.last_input_tokens` /
`last_output_tokens` (both `Option<u32>`; default to 0 when `None`). Architect
tokens for the live session are 0 (no completed `PhaseRun` yet). Return empty
when `summary.last_input_tokens.is_none()` (preserves existing behavior).

**Complete function skeleton:**

```rust
pub(crate) fn savings_lines(
    summary: &StatusSummary,
    rates: BudgetRates,
    milestone_costs: Option<ScopeCosts>,
    project_costs: ScopeCosts,
    project_escalation_count: u32,
) -> Vec<Line<'static>> {
    let sess_in = match summary.last_input_tokens {
        Some(v) => v as u64,
        None => return Vec::new(),
    };
    let sess_out = summary.last_output_tokens.unwrap_or(0) as u64;

    // … define cost/fmt_dollars/no_baseline/baseline_val/executor_val/
    //   architect_val/net_val closures as above …

    let has_milestone = milestone_costs.is_some();
    let mile = milestone_costs.unwrap_or_default();

    let header: Line<'static> = if has_milestone {
        Line::from(format!("{:<12}{:>9}{:>9}{:>9}", "Savings", "Session", "Milestone", "Project"))
    } else {
        Line::from(format!("{:<12}{:>9}{:>9}", "Savings", "Session", "Project"))
    };

    let make_row = |label: &str,
                    v_sess: String,
                    v_mile: String,
                    v_proj: String|
     -> Line<'static> {
        if has_milestone {
            Line::from(format!("  {:<10}{:>9}{:>9}{:>9}", label, v_sess, v_mile, v_proj))
        } else {
            Line::from(format!("  {:<10}{:>9}{:>9}", label, v_sess, v_proj))
        }
    };

    vec![
        header,
        make_row(
            "Baseline:",
            baseline_val(sess_in, sess_out),
            baseline_val(mile.executor_in, mile.executor_out),
            baseline_val(project_costs.executor_in, project_costs.executor_out),
        ),
        make_row(
            "Executor:",
            executor_val(sess_in, sess_out),
            executor_val(mile.executor_in, mile.executor_out),
            executor_val(project_costs.executor_in, project_costs.executor_out),
        ),
        make_row(
            "Architect:",
            architect_val(0, 0),         // live session: no PhaseRun yet
            architect_val(mile.architect_in, mile.architect_out),
            architect_val(project_costs.architect_in, project_costs.architect_out),
        ),
        make_row(
            "Net:",
            net_val(sess_in, sess_out, 0, 0),
            net_val(mile.executor_in, mile.executor_out, mile.architect_in, mile.architect_out),
            net_val(project_costs.executor_in, project_costs.executor_out,
                    project_costs.architect_in, project_costs.architect_out),
        ),
        Line::from(format!("  Assists: {project_escalation_count}")),
    ]
}
```

### 6. Multi-site struct-literal traversal (compiler-guided)

**`BudgetRates {…}` literals** — two new fields default to 0.0 (`Default`):
Add `..BudgetRates::default()` to each literal that does not name all six fields,
OR add the two explicit new fields:

- `mcp/src/main.rs:569` — **production site**, Task 7 below: add architect rates.
- `mcp/src/dashboard/panels.rs` test sites: all 5 existing `savings_lines` test
  literals. **These tests are being replaced** (Task 8); update the literal type
  as part of the rewrite rather than patching the old tests.

**`savings_lines(…)` call sites** — new signature:

- `mcp/src/dashboard/render.rs:159` — **production site**, Task 7 below.
- `mcp/src/dashboard/panels.rs` test sites: all 6 existing `savings_lines` tests
  **being replaced** (Task 8).

**`DashboardData.milestone_savings` / `.project_savings` field accesses:**

- `mcp/src/dashboard/render.rs:162–163` — **production site**, Task 7 below.
- `mcp/src/dashboard/mod.rs` — two `DashboardData {…}` literals (Task 4 above)
  and three test functions that read `data.project_savings` / `data.milestone_savings`
  (Task 9 below: test updates).

Use `cargo build` E0063 / E0609 output to drive traversal; do **not** hand-search.

### 7. Update `render.rs` and `main.rs`

**`mcp/src/dashboard/render.rs`:**

Update the `savings_lines` call to pass the new args (replacing lines 159–164):

```rust
budget.extend(savings_lines(
    &data.summary,
    rates,
    data.milestone_costs,
    data.project_costs,
    data.project_escalation_count,
));
```

Increase the header-band height to accommodate the new savings block (max 6 lines
+ budget_lines max 4 lines = 10 content rows + 2 border = 12). Change line 132:

```rust
// Before:
let [header, body] =
    Layout::vertical([Constraint::Length(11), Constraint::Min(0)]).areas::<2>(area);
// After:
let [header, body] =
    Layout::vertical([Constraint::Length(13), Constraint::Min(0)]).areas::<2>(area);
```

Update the comment on the line above to reflect the new height:

```rust
// Height 13 = 2 border rows + up to 11 Budget content rows (savings block now
// 6 lines: header + Baseline/Executor/Architect/Net + Assists).
```

**`mcp/src/main.rs`:**

Extract architect rates and extend the `BudgetRates` literal (replacing lines 568–572):

```rust
let (i, o) = cfg.dashboard.effective_rates();
let (arch_in, arch_out) = cfg.architect.effective_rates();
let rates = dashboard::BudgetRates {
    input_per_mtok: i,
    output_per_mtok: o,
    architect_input_per_mtok: arch_in,
    architect_output_per_mtok: arch_out,
};
```

`cfg.architect` is `rexymcp_executor::config::ArchitectConfig`; its
`effective_rates()` method is at `executor/src/config.rs:96`. The `Config` type
already has an `architect: ArchitectConfig` field (added by phase-01).

### 8. Replace `savings_lines` tests in `mcp/src/dashboard/panels.rs`

Delete all six existing `savings_lines_*` tests (they test the old format and
will fail). Add these tests:

```rust
#[test]
fn savings_lines_empty_without_session_metrics() {
    let result = savings_lines(
        &StatusSummary::default(),
        BudgetRates::default(),
        None,
        ScopeCosts::default(),
        0,
    );
    assert!(result.is_empty(), "no session tokens → empty");
}

#[test]
fn savings_lines_header_contains_scope_names() {
    let summary = StatusSummary {
        last_input_tokens: Some(1_000_000),
        last_output_tokens: Some(500_000),
        ..StatusSummary::default()
    };
    // No milestone → 2-scope header
    let lines = savings_lines(&summary, BudgetRates::default(), None, ScopeCosts::default(), 0);
    let header = format!("{}", lines[0]);
    assert!(header.contains("Savings"), "header must start with Savings");
    assert!(header.contains("Session"), "header must name Session");
    assert!(header.contains("Project"), "header must name Project");
    assert!(!header.contains("Milestone"), "no Milestone column when None");
}

#[test]
fn savings_lines_three_scope_header_contains_milestone() {
    let summary = StatusSummary {
        last_input_tokens: Some(1_000_000),
        last_output_tokens: Some(500_000),
        ..StatusSummary::default()
    };
    let mile = Some(ScopeCosts { executor_in: 500_000, executor_out: 200_000, ..ScopeCosts::default() });
    let lines = savings_lines(&summary, BudgetRates::default(), mile, ScopeCosts::default(), 0);
    let header = format!("{}", lines[0]);
    assert!(header.contains("Milestone"), "3-scope header must name Milestone");
}

#[test]
fn savings_lines_produces_six_lines_with_session_metrics() {
    // header + Baseline + Executor + Architect + Net + Assists = 6 lines
    let summary = StatusSummary {
        last_input_tokens: Some(1_000_000),
        last_output_tokens: Some(500_000),
        ..StatusSummary::default()
    };
    let lines = savings_lines(&summary, BudgetRates::default(), None, ScopeCosts::default(), 0);
    assert_eq!(lines.len(), 6, "exactly 6 lines: {lines:?}");
    let texts: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();
    assert!(texts[1].contains("Baseline:"), "row 1 is Baseline");
    assert!(texts[2].contains("Executor:"), "row 2 is Executor");
    assert!(texts[3].contains("Architect:"), "row 3 is Architect");
    assert!(texts[4].contains("Net:"), "row 4 is Net");
    assert!(texts[5].contains("Assists:"), "row 5 is Assists");
}

#[test]
fn savings_lines_baseline_dash_when_rates_unset() {
    let summary = StatusSummary {
        last_input_tokens: Some(1_000_000),
        last_output_tokens: Some(500_000),
        ..StatusSummary::default()
    };
    let lines = savings_lines(&summary, BudgetRates::default(), None, ScopeCosts::default(), 0);
    let texts: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();
    assert!(texts[1].contains('—'), "Baseline shows — when no rates: {}", texts[1]);
    assert!(texts[4].contains('—'), "Net shows — when no rates: {}", texts[4]);
}

#[test]
fn savings_lines_executor_always_shows_zero_dollars() {
    let summary = StatusSummary {
        last_input_tokens: Some(1_000_000),
        last_output_tokens: Some(500_000),
        ..StatusSummary::default()
    };
    let rates = BudgetRates { input_per_mtok: 5.0, output_per_mtok: 25.0, ..BudgetRates::default() };
    let lines = savings_lines(&summary, rates, None, ScopeCosts::default(), 0);
    let exec_row = format!("{}", lines[2]);
    assert!(exec_row.contains("$0.00"), "Executor always $0.00: {exec_row}");
}

#[test]
fn savings_lines_architect_cost_shown_from_project_costs() {
    // architect_*_tokens > 0 with configured architect rates → non-zero Architect value
    let summary = StatusSummary {
        last_input_tokens: Some(1_000_000),
        last_output_tokens: Some(0),
        ..StatusSummary::default()
    };
    let rates = BudgetRates {
        input_per_mtok: 5.0,
        output_per_mtok: 25.0,
        architect_input_per_mtok: 5.0,
        architect_output_per_mtok: 25.0,
    };
    let project_costs = ScopeCosts {
        executor_in: 1_000_000,
        executor_out: 0,
        architect_in: 1_000_000,  // 1M architect input tokens
        architect_out: 0,
    };
    let lines = savings_lines(&summary, rates, None, project_costs, 0);
    let texts: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();
    // Project Architect column: 1M tokens × $5/MTok = $5.00
    assert!(texts[3].contains("$5.00"), "Architect project column shows $5.00: {}", texts[3]);
}

#[test]
fn savings_lines_net_subtracts_architect_from_baseline() {
    // Baseline $5.00, Architect $1.00, Net $4.00
    let summary = StatusSummary {
        last_input_tokens: Some(1_000_000),
        last_output_tokens: Some(0),
        ..StatusSummary::default()
    };
    let rates = BudgetRates {
        input_per_mtok: 5.0,
        output_per_mtok: 25.0,
        architect_input_per_mtok: 1.0,
        architect_output_per_mtok: 5.0,
    };
    let project_costs = ScopeCosts {
        executor_in: 1_000_000,
        executor_out: 0,
        architect_in: 1_000_000,
        architect_out: 0,
    };
    let lines = savings_lines(&summary, rates, None, project_costs, 0);
    let texts: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();
    // Baseline project: 1M × $5 = $5.00; Architect project: 1M × $1 = $1.00; Net = $4.00
    assert!(texts[1].contains("$5.00"), "Baseline project $5.00: {}", texts[1]);
    assert!(texts[4].contains("$4.00"), "Net project $4.00: {}", texts[4]);
}

#[test]
fn savings_lines_assists_shows_project_escalation_count() {
    let summary = StatusSummary {
        last_input_tokens: Some(1_000_000),
        last_output_tokens: Some(0),
        ..StatusSummary::default()
    };
    let lines = savings_lines(&summary, BudgetRates::default(), None, ScopeCosts::default(), 3);
    let assists_row = format!("{}", lines[5]);
    assert!(assists_row.contains("Assists: 3"), "Assists row: {assists_row}");
}

#[test]
fn savings_lines_data_rows_equal_width_for_alignment() {
    // All four data rows (Baseline/Executor/Architect/Net) must be equal width
    // so values land in the same columns.
    let summary = StatusSummary {
        last_input_tokens: Some(1_000_000),
        last_output_tokens: Some(500_000),
        ..StatusSummary::default()
    };
    let rates = BudgetRates { input_per_mtok: 3.0, output_per_mtok: 15.0, ..BudgetRates::default() };
    let project_costs = ScopeCosts {
        executor_in: 5_000_000, executor_out: 2_000_000,
        architect_in: 0, architect_out: 0,
    };
    let lines = savings_lines(&summary, rates, None, project_costs, 0);
    let texts: Vec<String> = lines[1..5].iter().map(|l| format!("{l}")).collect();
    let widths: Vec<usize> = texts.iter().map(|s| s.chars().count()).collect();
    assert!(
        widths.iter().all(|&w| w == widths[0]),
        "all data rows must be equal width for column alignment: {widths:?}",
    );
}
```

### 9. Update `load_data` tests in `mcp/src/dashboard/mod.rs`

The three existing project-savings tests reference the renamed fields and old type. Update:

- `load_data_reads_project_savings_from_phase_runs`:
  - Change `data.project_savings` to `data.project_costs`
  - Change assertion `== (3000, 1300)` to `== ScopeCosts { executor_in: 3000, executor_out: 1300, architect_in: 0, architect_out: 0 }`
  - Change `data.milestone_savings.is_none()` to `data.milestone_costs.is_none()`

- `load_data_project_savings_excludes_other_projects`:
  - Change `data.project_savings == (1000, 500)` to `data.project_costs == ScopeCosts { executor_in: 1000, executor_out: 500, architect_in: 0, architect_out: 0 }`

- `load_data_project_savings_zero_when_no_project_id`:
  - Change `data.project_savings == (0, 0)` to `data.project_costs == ScopeCosts::default()`

Add two new tests immediately after the existing three:

```rust
#[test]
fn load_data_reads_project_architect_costs_from_phase_runs() {
    // PhaseRun lines with non-zero tier_telemetry.architect_*_tokens are summed.
    let dir = TempDir::new().unwrap();
    let sessions = sessions_dir(dir.path());
    std::fs::create_dir_all(&sessions).unwrap();
    let pid = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
    // A run with architect_input_tokens=1500, architect_output_tokens=300 in tier_telemetry.
    let run = format!(
        r#"{{"ts":1,"model":"t","generation_params":{{}},"phase_id":"p1","project_id":"{pid}","tags":[],"status":"complete","escalated":false,"gates":{{}},"parse_failure_rate":0.0,"repairs_per_call":0.0,"verifier_retries":0,"tool_success_rate":1.0,"turns":1,"wall_clock_s":1.0,"tokens":{{"input_tokens":1000,"output_tokens":500}},"tier_telemetry":{{"tier":null,"doc_level":null,"escalation_count":0,"architect_input_tokens":1500,"architect_output_tokens":300}}}}"#
    );
    let telemetry_dir = dir.path().join("telemetry");
    std::fs::create_dir_all(&telemetry_dir).unwrap();
    std::fs::write(telemetry_dir.join("phase_runs.jsonl"), format!("{run}\n")).unwrap();

    let data = load_data(dir.path(), None, Some(&telemetry_dir), Some(pid));
    assert_eq!(
        data.project_costs.architect_in, 1500,
        "architect_in must be summed from tier_telemetry"
    );
    assert_eq!(
        data.project_costs.architect_out, 300,
        "architect_out must be summed from tier_telemetry"
    );
}

#[test]
fn load_data_reads_project_escalation_count() {
    let dir = TempDir::new().unwrap();
    let sessions = sessions_dir(dir.path());
    std::fs::create_dir_all(&sessions).unwrap();
    let pid = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
    let run1 = format!(
        r#"{{"ts":1,"model":"t","generation_params":{{}},"phase_id":"p1","project_id":"{pid}","tags":[],"status":"complete","escalated":false,"gates":{{}},"parse_failure_rate":0.0,"repairs_per_call":0.0,"verifier_retries":0,"tool_success_rate":1.0,"turns":1,"wall_clock_s":1.0,"tokens":{{"input_tokens":0,"output_tokens":0}},"tier_telemetry":{{"tier":null,"doc_level":null,"escalation_count":2,"architect_input_tokens":0,"architect_output_tokens":0}}}}"#
    );
    let run2 = format!(
        r#"{{"ts":2,"model":"t","generation_params":{{}},"phase_id":"p2","project_id":"{pid}","tags":[],"status":"complete","escalated":false,"gates":{{}},"parse_failure_rate":0.0,"repairs_per_call":0.0,"verifier_retries":0,"tool_success_rate":1.0,"turns":1,"wall_clock_s":1.0,"tokens":{{"input_tokens":0,"output_tokens":0}},"tier_telemetry":{{"tier":null,"doc_level":null,"escalation_count":1,"architect_input_tokens":0,"architect_output_tokens":0}}}}"#
    );
    let telemetry_dir = dir.path().join("telemetry");
    std::fs::create_dir_all(&telemetry_dir).unwrap();
    std::fs::write(
        telemetry_dir.join("phase_runs.jsonl"),
        format!("{run1}\n{run2}\n"),
    )
    .unwrap();

    let data = load_data(dir.path(), None, Some(&telemetry_dir), Some(pid));
    assert_eq!(
        data.project_escalation_count, 3,
        "escalation_count must sum across all project runs"
    );
}
```

## Acceptance criteria

- [ ] `ScopeCosts` exists in `mcp/src/dashboard/panels.rs` with four `u64` fields,
      deriving `Default + PartialEq`.
- [ ] `BudgetRates` has `architect_input_per_mtok: f64` and
      `architect_output_per_mtok: f64`; both default to 0.0.
- [ ] `DashboardData` has `milestone_costs: Option<ScopeCosts>`,
      `project_costs: ScopeCosts`, `project_escalation_count: u32` (old
      `milestone_savings`/`project_savings` fields gone).
- [ ] `load_data` sums `tier_telemetry.architect_input_tokens` /
      `architect_output_tokens` / `escalation_count` per project scope.
- [ ] `savings_lines` with `last_input_tokens = None` returns empty.
- [ ] `savings_lines` with session metrics returns exactly 6 lines: header +
      Baseline + Executor + Architect + Net + Assists.
- [ ] Header row contains "Session" and "Project"; adds "Milestone" only when
      `milestone_costs.is_some()`.
- [ ] Executor row always contains "$0.00".
- [ ] Baseline and Net rows contain "—" when `BudgetRates` baseline rates are 0.0.
- [ ] Net = Baseline − Architect (test `savings_lines_net_subtracts_architect_from_baseline`).
- [ ] Assists row shows `project_escalation_count` as "Assists: N".
- [ ] All four data rows are equal-width (alignment test).
- [ ] The three updated `load_data` tests pass with renamed fields.
- [ ] `load_data_reads_project_architect_costs_from_phase_runs` passes.
- [ ] `load_data_reads_project_escalation_count` passes.
- [ ] `cargo fmt --all --check` exits 0.
- [ ] `cargo build` exits 0 with zero new warnings.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` exits 0.
- [ ] `cargo test` passes; count rises by at least 5 (10 new savings tests − 6
      removed + 2 new load_data tests + 3 updated load_data tests net zero).

## Test plan

**`mcp/src/dashboard/panels.rs`** — replacing 6 old savings tests, adding 10 new:
- `savings_lines_empty_without_session_metrics`
- `savings_lines_header_contains_scope_names`
- `savings_lines_three_scope_header_contains_milestone`
- `savings_lines_produces_six_lines_with_session_metrics`
- `savings_lines_baseline_dash_when_rates_unset`
- `savings_lines_executor_always_shows_zero_dollars`
- `savings_lines_architect_cost_shown_from_project_costs`
- `savings_lines_net_subtracts_architect_from_baseline`
- `savings_lines_assists_shows_project_escalation_count`
- `savings_lines_data_rows_equal_width_for_alignment`

**`mcp/src/dashboard/mod.rs`** — updating 3 existing tests, adding 2:
- `load_data_reads_project_savings_from_phase_runs` (updated)
- `load_data_project_savings_excludes_other_projects` (updated)
- `load_data_project_savings_zero_when_no_project_id` (updated)
- `load_data_reads_project_architect_costs_from_phase_runs` (new)
- `load_data_reads_project_escalation_count` (new)

## End-to-end verification

The real artifact this phase ships is the dashboard's Budget panel. The panel
renders in a TUI (no headless harness) — per the M13/M17 dashboard-phase
precedent, E2E is declared N/A and the load-bearing acceptance criteria are
verified by the unit tests above.

> Not applicable — phase ships a TUI display-only change. No runtime-loadable
> artifact other than the dashboard panel itself; unit tests render real
> `Line`/`Span` output.

## Authorizations

None. No new dependency, no `Cargo.toml` edit, no `docs/architecture.md` edit.

## Out of scope

- Executor rate config field (`executor_input_per_mtok` / `executor_output_per_mtok`
  in `BudgetRates` or config) — future milestone when paid local providers arrive.
  The Executor row always shows "$0.00" in this phase.
- Milestone-scope Architect costs for the escalation counter — phased: escalation
  events aren't produced until M21; the project escalation count comes from
  `tier_telemetry.escalation_count` already summed into `PhaseRun`.
- EscalationEvent-based per-session escalation tracking — M21 (the producer fires
  then). Session Architect cost is always $0.00 in this phase.
- Phase-outcome feed panel (a separate visual not in the current dashboard).
  The Assists counter satisfies "phase outcome feeds gains escalation column."
- Touching `STANDARDS.md`, `WORKFLOW.md`, or any other phase doc.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-06-16 22:51 (started)

**Executor:** rexyMCP executor LLM
**Tasks started:** 1–9 (ScopeCosts struct, BudgetRates extension, DashboardData update, load_data rewrite, savings_lines redesign, multi-site traversal, render.rs/main.rs updates, test replacements)

### Update — 2026-06-16 22:51 (complete)

**Tasks completed:** 1–9

**Files changed:**
- `mcp/src/dashboard/panels.rs` — added `ScopeCosts` struct, extended `BudgetRates`, redesigned `savings_lines`, replaced 6 old tests with 10 new ones
- `mcp/src/dashboard/mod.rs` — updated `DashboardData` fields, rewrote `load_data` folds for `ScopeCosts` + escalation count, updated 3 existing tests, added 2 new tests
- `mcp/src/dashboard/render.rs` — updated `savings_lines` call site, increased header-band height from 11 to 13
- `mcp/src/main.rs` — wired architect rates into `BudgetRates`
- `docs/dev/milestones/M20-tier-calibration/phase-03-dashboard-cost-breakdown.md` — status → review
- `docs/dev/milestones/M20-tier-calibration/README.md` — phase-03 row → review

**Verification commands:**
- `cargo fmt --all --check`: exit 0 (after `cargo fmt --all`)
- `cargo build`: exit 0, zero new warnings
- `cargo clippy --all-targets --all-features -- -D warnings`: exit 0
- `cargo test`: 807 passed, 0 failed, 2 ignored

**Verification summary:** fmt clean, build clean, clippy clean, all 807 tests pass (10 new savings_lines tests + 2 new load_data tests, 6 old savings tests removed, 3 load_data tests updated).

**Grep for pinned literals:**
```
$ grep -rn 'ScopeCosts' mcp/src/dashboard/panels.rs | head -3
→ ScopeCosts struct at line 17, used in savings_lines signature
$ grep -rn 'architect_input_per_mtok' mcp/src/dashboard/panels.rs | head -3
→ BudgetRates field at line 30, used in architect_val/net_val closures
$ grep -rn 'project_escalation_count' mcp/src/dashboard/mod.rs | head -3
→ DashboardData field, load_data fold, struct literals
$ grep -rn 'Assists:' mcp/src/dashboard/panels.rs | head -3
→ Line::from(format!("  Assists: {project_escalation_count}")) in savings_lines
```

**End-to-end verification:** N/A — phase ships a TUI display-only change. No runtime-loadable artifact other than the dashboard panel itself; unit tests render real `Line`/`Span` output.

**Notes for review:** None — implementation follows the spec exactly.

### Review verdict — 2026-06-16

- **Verdict:** approved_first_try
- **Bounces:** none
- **Executor:** Qwen/Qwen3.6-27B-FP8
- **Scope deviations:** none
- **Calibration:** none. Clean 73-turn first-try. All four gates green on
  independent re-run (fmt/build/clippy clean; 807 passed / 2 ignored).
  Compiler-guided multi-site `BudgetRates`/`savings_lines`/`DashboardData`
  traversal landed without churn stall. The Net-subtracts-architect test is
  mutation-resistant (Baseline $5 − Architect $1 = $4; a non-subtracting impl
  yields $5 and fails). E2E declared N/A per the established M13/M17 TUI
  dashboard-panel precedent — authorized in the phase doc. The cosmetic
  Update-Log identity self-stamp ("rexyMCP executor LLM") persists; date
  correct (2026-06-16).
