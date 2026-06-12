# Phase 07: Savings scope — session + milestone + project

**Milestone:** M17 — Dashboard Polish (Round 3)
**Status:** review
**Depends on:** phase-06 (phase-06 renames `"$ saved"` → `"Savings:"` and the
`dollars_saved_line` function must exist before this phase replaces it)
**Estimated diff:** ~160 lines across 8 files
**Tags:** language=rust, kind=feature, size=m

## Goal

Expand the Budget panel's savings display from a single session line to three
scope levels: **session** (live token usage), **milestone** (cumulative
`PhaseRun` records belonging to the active milestone), **project** (all
`PhaseRun` records ever). This requires recording the phase-doc path in every
`PhaseRun` (so the dashboard can derive which milestone each run belongs to),
and threading the telemetry directory from config through the dashboard stack.

## Architecture references

Read before starting:

- `executor/src/agent/mod.rs:61–72` — `PhaseInput` struct (gains `phase_doc_path`).
- `executor/src/agent/tests.rs:28–38` — `fn input()` test helper (gains `phase_doc_path`).
- `executor/src/agent/metrics.rs:55–115` — `emit_phase_run` (sets `phase_doc_path`).
- `executor/src/store/telemetry.rs:102–142` — `PhaseRun` struct (gains `phase_doc_path`).
- `mcp/src/runner.rs:155–190` — `run_phase_with`, builds `PhaseInput` literal at line 179.
- `mcp/src/dashboard/mod.rs:22–61` — `DashboardData`, `load_data`, `run_dashboard`.
- `mcp/src/dashboard/mod.rs:68–114` — `resolve_milestone` (to be refactored into
  `resolve_milestone_dir` + thin `resolve_milestone` wrapper).
- `mcp/src/dashboard/event_loop.rs:8–33` — `run_loop` signature and `load_data` call.
- `mcp/src/dashboard/panels.rs:447–472` — `dollars_saved_line` (replaced by `savings_lines`).
- `mcp/src/dashboard/render.rs:157–162` — `dollars_saved_line` call site (becomes
  `savings_lines`).
- `mcp/src/main.rs:362–382` — `Dashboard` command handler (passes `telemetry_dir`).

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read all architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.
5. Confirm phase-06 is `done` (its `dollars_saved_line` rename to `"Savings:"` is a
   prerequisite — this phase replaces the function entirely).

## Current state

Relevant current shapes:

```rust
// executor/src/agent/mod.rs
pub struct PhaseInput {
    pub standards: String,
    pub phase_doc: String,
    pub goal: String,
    pub acceptance_criteria: String,
    pub phase: String,
    pub tags: Vec<String>,
}

// executor/src/store/telemetry.rs
pub struct PhaseRun {
    pub ts: u64,
    pub model: String,
    pub generation_params: GenerationParams,
    pub phase_id: String,
    // ... (no phase_doc_path field) ...
}

// mcp/src/dashboard/mod.rs
pub struct DashboardData {
    pub summary: StatusSummary,
    pub records: Vec<SessionRecord>,
    pub error: Option<String>,
    pub milestone: Option<String>,
}

pub fn load_data(repo: &Path, session: Option<&str>) -> DashboardData { ... }

pub fn run_dashboard(repo: &Path, session: Option<&str>, rates: BudgetRates) -> std::io::Result<()> {
    let result = event_loop::run_loop(&mut terminal, repo, session, rates);
    ...
}

// event_loop.rs
pub(crate) fn run_loop(terminal, repo, session, rates) {
    ...
    let data = load_data(repo, session);
    ...
}

// render.rs:157–162
if let Some(line) = dollars_saved_line(&data.summary, rates) {
    budget.push(line);
}

// panels.rs:461–472 (after phase-06)
pub(crate) fn dollars_saved_line(
    summary: &StatusSummary,
    rates: BudgetRates,
) -> Option<Line<'static>> {
    // returns "Savings: —" or "Savings: $X.XX"
}
```

## Spec

### §1 — Add `phase_doc_path` to `PhaseInput` (executor/src/agent/mod.rs)

Add the field after `tags`:

```rust
pub struct PhaseInput {
    pub standards: String,
    pub phase_doc: String,
    pub goal: String,
    pub acceptance_criteria: String,
    pub phase: String,
    pub tags: Vec<String>,
    /// Full absolute path to the phase doc, recorded in `PhaseRun` for
    /// milestone-aware savings queries.
    pub phase_doc_path: String,
}
```

### §2 — Add `phase_doc_path` to `PhaseRun` (executor/src/store/telemetry.rs)

Add after the `phase_id` field (line ~107) with `#[serde(default)]` for
backward compatibility with the ~120 existing records that lack this field:

```rust
    pub phase_id: String,
    /// Full path to the phase doc, for milestone-aware savings queries.
    /// `None` for legacy records that predate this field (M7 phase-08b and earlier).
    #[serde(default)]
    pub phase_doc_path: Option<String>,
```

### §3 — Set `phase_doc_path` in `emit_phase_run` (executor/src/agent/metrics.rs)

In the `PhaseRun { ... }` struct literal inside `emit_phase_run`, add:

```rust
phase_doc_path: Some(input.phase_doc_path.clone()),
```

alongside the existing `phase_id: input.phase.clone()` line.

### §4 — Thread `phase_doc_path` into `PhaseInput` (mcp/src/runner.rs)

In `run_phase_with`, the `PhaseInput` literal at line ~179 currently ends at
`tags: fields.tags`. Add the new field:

```rust
let input = PhaseInput {
    standards: inp.standards.to_string(),
    phase_doc,
    goal: fields.goal,
    acceptance_criteria: fields.acceptance_criteria,
    phase,
    tags: fields.tags,
    phase_doc_path: inp.phase_doc_path.to_string_lossy().into_owned(),
};
```

`inp.phase_doc_path` is `&Path` (from `AssemblyInput.phase_doc_path`, line 221),
so `.to_string_lossy().into_owned()` converts it to an owned `String`.

### §5 — Update `DashboardData` and `load_data` (mcp/src/dashboard/mod.rs)

**5a. Add savings fields to `DashboardData`:**

```rust
pub struct DashboardData {
    pub summary: StatusSummary,
    pub records: Vec<SessionRecord>,
    pub error: Option<String>,
    pub milestone: Option<String>,
    /// Cumulative (input_tokens, output_tokens) from `PhaseRun` records whose
    /// `phase_doc_path` belongs to the active milestone. `None` when telemetry
    /// is absent, no phase is active, or no matching records exist.
    pub milestone_savings: Option<(u32, u32)>,
    /// Cumulative (input_tokens, output_tokens) from ALL `PhaseRun` records in
    /// the telemetry file. `(0, 0)` when telemetry dir is not configured.
    pub project_savings: (u32, u32),
}
```

**5b. Refactor `resolve_milestone` into `resolve_milestone_dir` + wrapper.**

`resolve_milestone` currently finds the dir name and applies `format_milestone_name`.
Extract the directory-finding logic into a new private function:

```rust
/// Returns the milestone **directory name** (e.g. `"M17-dashboard-polish-3"`)
/// for the running phase, using the same candidate-selection rules as
/// `resolve_milestone`. `None` when no matching milestone directory is found.
fn resolve_milestone_dir(repo: &Path, phase: Option<&str>) -> Option<String> {
    let phase = phase?;
    let milestones = repo.join("docs/dev/milestones");
    let prefix = format!("{phase}-");
    let mut candidates: Vec<(u32, String, bool)> = Vec::new();
    let entries = std::fs::read_dir(&milestones).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() { continue; }
        let Some(dir_name) = path.file_name().and_then(|n| n.to_str()) else { continue; };
        let Some(num) = milestone_number(dir_name) else { continue; };
        let Ok(files) = std::fs::read_dir(&path) else { continue; };
        for f in files.flatten() {
            let fname = f.file_name();
            let Some(fname) = fname.to_str() else { continue; };
            if fname.starts_with(&prefix) && fname.ends_with(".md") {
                let active = match std::fs::read_to_string(f.path()) {
                    Ok(body) => !phase_doc_is_done(&body),
                    Err(_) => false,
                };
                candidates.push((num, dir_name.to_string(), active));
                break;
            }
        }
    }
    candidates
        .iter()
        .filter(|(_, _, active)| *active)
        .max_by_key(|(num, _, _)| *num)
        .or_else(|| candidates.iter().max_by_key(|(num, _, _)| *num))
        .map(|(_, dir, _)| dir.clone())
}

/// Thin wrapper — same contract as before, unchanged external behaviour.
fn resolve_milestone(repo: &Path, phase: Option<&str>) -> Option<String> {
    resolve_milestone_dir(repo, phase).map(|dir| format_milestone_name(&dir))
}
```

The original `resolve_milestone` body can be removed; its logic now lives in
`resolve_milestone_dir`.

**5c. Add a `read_phase_runs` helper:**

```rust
/// Parse `<telemetry_dir>/phase_runs.jsonl`, returning one `PhaseRun` per
/// valid line; silently skips empty lines and malformed JSON.
fn read_phase_runs(telemetry_dir: &Path) -> Vec<rexymcp_executor::store::telemetry::PhaseRun> {
    let path = telemetry_dir.join("phase_runs.jsonl");
    let Ok(content) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}
```

**5d. Change `load_data` signature to accept `telemetry_dir`:**

```rust
pub fn load_data(
    repo: &Path,
    session: Option<&str>,
    telemetry_dir: Option<&Path>,
) -> DashboardData {
```

Add the savings computation after the `milestone` resolution:

```rust
let (milestone_savings, project_savings) = match telemetry_dir {
    None => (None, (0, 0)),
    Some(dir) => {
        let runs = read_phase_runs(dir);
        let project_savings = runs.iter().fold((0u32, 0u32), |(i, o), r| {
            (
                i.saturating_add(r.tokens.input_tokens),
                o.saturating_add(r.tokens.output_tokens),
            )
        });
        let milestone_savings = resolve_milestone_dir(repo, summary.phase.as_deref())
            .map(|milestone_dir| {
                runs.iter()
                    .filter(|r| {
                        r.phase_doc_path
                            .as_deref()
                            .map(|p| p.contains(milestone_dir.as_str()))
                            .unwrap_or(false)
                    })
                    .fold((0u32, 0u32), |(i, o), r| {
                        (
                            i.saturating_add(r.tokens.input_tokens),
                            o.saturating_add(r.tokens.output_tokens),
                        )
                    })
            })
            .filter(|&(i, o)| i > 0 || o > 0);
        (milestone_savings, project_savings)
    }
};
```

Return the new fields:

```rust
DashboardData {
    summary,
    records,
    error: None,
    milestone,
    milestone_savings,
    project_savings,
}
```

The error-path `DashboardData` gets:

```rust
DashboardData {
    summary: StatusSummary::default(),
    records: Vec::new(),
    error: Some(e),
    milestone: None,
    milestone_savings: None,
    project_savings: (0, 0),
}
```

**Required import addition in `mod.rs`** — `PhaseRun` is in a different crate:

```rust
use rexymcp_executor::store::telemetry::PhaseRun;
```

Add this use statement in the imports at the top of the file.

### §6 — Thread `telemetry_dir` through the dashboard stack

**6a. `event_loop::run_loop` (mcp/src/dashboard/event_loop.rs):**

Add `telemetry_dir: Option<&Path>` parameter:

```rust
pub(crate) fn run_loop(
    terminal: &mut ratatui::DefaultTerminal,
    repo: &Path,
    session: Option<&str>,
    rates: BudgetRates,
    telemetry_dir: Option<&Path>,
) -> std::io::Result<()> {
```

Update the `load_data` call (line 33):

```rust
let data = load_data(repo, session, telemetry_dir);
```

**Do not add caching.** `run_loop` calls `load_data` once per ~500ms poll tick
(`event_loop.rs:58`), so `read_phase_runs` re-reads and re-parses
`phase_runs.jsonl` (≈120 lines today) every frame. This is intentional and
consistent with the existing per-tick `status::load_records` re-read — it keeps
the live dashboard simple and the figures fresh when a phase completes
mid-session. Introducing a cache / dirty-flag / mtime check here is **out of
scope** (note it under "Notes for review" if you think it's worth a future
phase; do not implement it).

**6b. `run_dashboard` (mcp/src/dashboard/mod.rs):**

Add `telemetry_dir: Option<&Path>` parameter:

```rust
pub fn run_dashboard(
    repo: &Path,
    session: Option<&str>,
    rates: BudgetRates,
    telemetry_dir: Option<&Path>,
) -> std::io::Result<()> {
    let mut terminal = ratatui::init();
    let result = event_loop::run_loop(&mut terminal, repo, session, rates, telemetry_dir);
    ratatui::restore();
    result
}
```

**6c. `main.rs` Dashboard handler (lines ~362–382):**

Extract the telemetry dir from config and pass it:

```rust
Commands::Dashboard { repo, session, config } => {
    let config_path = config.unwrap_or_else(|| PathBuf::from("rexymcp.toml"));
    let cfg = Config::load_with_env(&config_path)?;
    let d = &cfg.dashboard;
    let rates = d
        .saved_model
        .as_deref()
        .and_then(dashboard::model_rates)
        .unwrap_or(dashboard::BudgetRates {
            input_per_mtok: d.saved_input_per_mtok,
            output_per_mtok: d.saved_output_per_mtok,
        });
    let telemetry_dir = cfg.telemetry.dir.as_deref();
    dashboard::run_dashboard(&repo, session.as_deref(), rates, telemetry_dir)
        .unwrap_or_else(|e| {
            eprintln!("dashboard error: {e}");
            std::process::exit(1);
        });
    Ok(())
}
```

### §7 — Replace `dollars_saved_line` with `savings_lines` (mcp/src/dashboard/panels.rs)

**7a. Remove `dollars_saved_line`.** Delete the entire function and its doc comment
(lines ~458–472 after phase-06). Keep the private `dollars_saved(input, output, in_rate, out_rate)` helper — it's still used internally.

**7b. Add `savings_lines`.**

Layout decision (architect, 2026-06-11): the savings block is a **`Savings`
header** followed by indented, **value-aligned** scope rows — `Session`
(always, when session metrics exist), then `Milestone` and `Project` (each only
when its token data is available). This supersedes phase-06's single
`"Savings:"` session line: phase-06 emits that line as an interim state and this
phase replaces the whole function, so the session figure now lives under the
header as the `Session:` row. Rationale: a flat `"Savings:"` session line next
to scope-named `"Milestone:"`/`"Project:"` sub-lines read as if *Savings* were a
total and the others its breakdown — semantically inverted (session ⊂ milestone
⊂ project). A header + uniformly scope-named rows fixes the hierarchy, and
right-aligning the dollar values makes the decimals line up in a column.

Target rendering (the `$` decimals align because values are right-aligned in a
fixed-width field; the em-dash replaces the value when rates are unset):

```
Savings
  Session:      $10.50
  Milestone:     $3.20
  Project:     $120.00
```

Implementation — note the **explicit named width args** (`lw`/`vw`) in the
`format!`; do not rely on inline-captured width identifiers:

```rust
/// Budget-panel savings block. A `Savings` header followed by indented,
/// value-aligned rows: `Session` (always, when session metrics exist), then
/// `Milestone` and `Project` (each only when its token data is available).
/// Dollar values are right-aligned so their decimals line up in a column.
/// Returns empty when there are no session metrics yet — never a lone header.
pub(crate) fn savings_lines(
    summary: &StatusSummary,
    rates: BudgetRates,
    milestone_tok: Option<(u32, u32)>,
    project_tok: (u32, u32),
) -> Vec<Line<'static>> {
    let in_tok = match summary.last_input_tokens {
        Some(v) => v,
        None => return Vec::new(),
    };
    let out_tok = summary.last_output_tokens.unwrap_or(0);
    let no_rates = rates.input_per_mtok == 0.0 && rates.output_per_mtok == 0.0;

    // Dollar value for a scope, or an em-dash when no rates are configured.
    let value = |i: u32, o: u32| -> String {
        if no_rates {
            "—".to_string()
        } else {
            let saved = dollars_saved(i, o, rates.input_per_mtok, rates.output_per_mtok);
            format!("${saved:.2}")
        }
    };
    // Indented row: label padded left, value padded right (decimals align).
    // `lw` covers the longest label ("Milestone:"); `vw` holds "$XXXX.XX".
    let row = |label: &str, v: String| -> Line<'static> {
        Line::from(format!("  {:<lw$}{:>vw$}", label, v, lw = 11, vw = 9))
    };

    let mut lines = vec![Line::from("Savings")];
    lines.push(row("Session:", value(in_tok, out_tok)));
    if let Some((m_in, m_out)) = milestone_tok {
        lines.push(row("Milestone:", value(m_in, m_out)));
    }
    let (p_in, p_out) = project_tok;
    if p_in > 0 || p_out > 0 {
        lines.push(row("Project:", value(p_in, p_out)));
    }
    lines
}
```

The value field width `vw = 9` is a **minimum**, not a cap — an all-time
`Project` total exceeding `$9999.99` simply renders wider, shifting that one
row's alignment rather than truncating. That is acceptable; do not clamp it.

### §8 — Update the budget call site (mcp/src/dashboard/render.rs)

Replace the `dollars_saved_line` block (lines ~158–161):

```rust
// Old:
if let Some(line) = dollars_saved_line(&data.summary, rates) {
    budget.push(line);
}
// New:
budget.extend(savings_lines(
    &data.summary,
    rates,
    data.milestone_savings,
    data.project_savings,
));
```

**Update the import — this is required, not optional.** `render.rs:10–12` has a
`use super::panels::{ ... };` block that names `dollars_saved_line`. Remove
`dollars_saved_line` from that list and add `savings_lines` (alphabetical
position is fine):

```rust
// render.rs imports, e.g.:
use super::panels::{
    BudgetRates, budget_lines, files_lines, milestone_line, panel, savings_lines,
    /* ...the rest unchanged... */
};
```

`savings_lines` is exported from `panels.rs` with `pub(crate)` like its
predecessor, so no visibility change is needed.

## Acceptance criteria

- [ ] `cargo build` succeeds with zero warnings.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes.
- [ ] `cargo fmt --all --check` passes.
- [ ] `cargo test` passes (existing tests updated; new tests pass).
- [ ] `PhaseRun` in `telemetry.rs` has `pub phase_doc_path: Option<String>` with
      `#[serde(default)]`.
- [ ] `PhaseInput` in `agent/mod.rs` has `pub phase_doc_path: String`.
- [ ] `emit_phase_run` sets `phase_doc_path: Some(input.phase_doc_path.clone())`.
- [ ] `run_phase_with` in `runner.rs` sets `phase_doc_path: inp.phase_doc_path.to_string_lossy().into_owned()`.
- [ ] `load_data` signature is `load_data(repo, session, telemetry_dir: Option<&Path>)`.
- [ ] `run_dashboard` and `run_loop` accept `telemetry_dir: Option<&Path>`.
- [ ] `savings_lines` emits a `"Savings"` header followed by a `"  Session:"` row
      whenever session metrics exist, and adds `"  Milestone:"` / `"  Project:"`
      rows when their token data is present.
- [ ] Dollar values are right-aligned (decimals line up) and the em-dash replaces
      the value on every row when rates are unset.
- [ ] Legacy `PhaseRun` records without `phase_doc_path` deserialize without error
      (their `phase_doc_path` is `None`).

## Test plan

### Executor-side tests

**Update `fn input()` helper** in `executor/src/agent/tests.rs` (line 29–38).
Add `phase_doc_path: "docs/dev/milestones/M0-test/phase-01-test.md".to_string()`.
All test literals that use `..input()` inherit the new field for free — **no
other test file change is needed in the executor crate**.

**Add `emit_phase_run_records_phase_doc_path`** in `executor/src/agent/metrics.rs`
or in a new `#[cfg(test)]` block nearby. The existing `emit_phase_run` tests (if
any) should be extended or a new test added:

```rust
#[test]
fn emit_phase_run_records_phase_doc_path() {
    use super::*;
    use crate::store::telemetry::PhaseRun;
    use tempfile::TempDir;
    // Build a minimal PhaseInput with a known phase_doc_path
    let input = PhaseInput {
        standards: String::new(),
        phase_doc: String::new(),
        goal: String::new(),
        acceptance_criteria: String::new(),
        phase: "phase-07".to_string(),
        tags: vec![],
        phase_doc_path: "/home/user/repo/docs/dev/milestones/M17-foo/phase-07-bar.md"
            .to_string(),
    };
    let dir = TempDir::new().unwrap();
    let run = build_phase_run_for_test(&input, /* other params */);
    assert_eq!(
        run.phase_doc_path.as_deref(),
        Some("/home/user/repo/docs/dev/milestones/M17-foo/phase-07-bar.md")
    );
}
```

If `emit_phase_run` has no existing isolated test and constructing a real run is
complex, a lighter approach: add an assertion to the compile-time struct test
or add a new unit test in `telemetry.rs` that round-trips a `PhaseRun` with
`phase_doc_path: Some("...")`:

```rust
#[test]
fn phase_run_phase_doc_path_round_trips() {
    let json = r#"{"ts":1,"model":"t","generation_params":{},"phase_id":"p","phase_doc_path":"/a/b.md","tags":[],"status":"c","escalated":false,"gates":{},"parse_failure_rate":0.0,"repairs_per_call":0.0,"verifier_retries":0,"tool_success_rate":1.0,"turns":1,"wall_clock_s":1.0,"tokens":{}}"#;
    let run: PhaseRun = serde_json::from_str(json).unwrap();
    assert_eq!(run.phase_doc_path.as_deref(), Some("/a/b.md"));
}

#[test]
fn phase_run_phase_doc_path_defaults_none_on_legacy_record() {
    // A JSON record without phase_doc_path — as emitted before this phase.
    let json = r#"{"ts":1,"model":"t","generation_params":{},"phase_id":"p","tags":[],"status":"c","escalated":false,"gates":{},"parse_failure_rate":0.0,"repairs_per_call":0.0,"verifier_retries":0,"tool_success_rate":1.0,"turns":1,"wall_clock_s":1.0,"tokens":{}}"#;
    let run: PhaseRun = serde_json::from_str(json).unwrap();
    assert!(run.phase_doc_path.is_none(), "legacy record must not error");
}
```

### Dashboard tests

**Update all four existing `load_data_*` tests** in `mcp/src/dashboard/mod.rs`.
Each call `load_data(dir.path(), None)` becomes `load_data(dir.path(), None, None)`.

**Add `load_data_reads_project_savings_from_phase_runs`:**

```rust
#[test]
fn load_data_reads_project_savings_from_phase_runs() {
    let dir = TempDir::new().unwrap();
    // Needs sessions dir to avoid the error path in load_data.
    let sessions = sessions_dir(dir.path());
    std::fs::create_dir_all(&sessions).unwrap();
    // Write two minimal PhaseRun records as JSONL.
    let run1 = r#"{"ts":1,"model":"t","generation_params":{},"phase_id":"p1","tags":[],"status":"complete","escalated":false,"gates":{},"parse_failure_rate":0.0,"repairs_per_call":0.0,"verifier_retries":0,"tool_success_rate":1.0,"turns":1,"wall_clock_s":1.0,"tokens":{"input_tokens":1000,"output_tokens":500}}"#;
    let run2 = r#"{"ts":2,"model":"t","generation_params":{},"phase_id":"p2","tags":[],"status":"complete","escalated":false,"gates":{},"parse_failure_rate":0.0,"repairs_per_call":0.0,"verifier_retries":0,"tool_success_rate":1.0,"turns":1,"wall_clock_s":1.0,"tokens":{"input_tokens":2000,"output_tokens":800}}"#;
    let telemetry_dir = dir.path().join("telemetry");
    std::fs::create_dir_all(&telemetry_dir).unwrap();
    std::fs::write(
        telemetry_dir.join("phase_runs.jsonl"),
        format!("{run1}\n{run2}\n"),
    ).unwrap();

    let data = load_data(dir.path(), None, Some(&telemetry_dir));
    assert_eq!(
        data.project_savings,
        (3000, 1300),
        "project savings must sum all phase runs"
    );
    // No session phase id → no milestone match.
    assert!(data.milestone_savings.is_none());
}
```

### Panel tests

**Remove three `dollars_saved_line_*` tests** (lines ~1506–1543 in `panels.rs`):
- `dollars_saved_line_none_without_metrics`
- `dollars_saved_line_dash_when_rates_unset`
- `dollars_saved_line_shows_dollars`

**Add six `savings_lines_*` tests** (place them in the same location). These pin
the new header + value-aligned layout: `lines[0]` is the `"Savings"` header, the
scope rows follow, and every scope row renders to the **same character width**
(that is the alignment guarantee — values share one right-aligned column). Note
the `.chars().count()` for width (the em-dash is multi-byte; `.len()` would
count bytes):

```rust
#[test]
fn savings_lines_empty_without_session_metrics() {
    let rates = BudgetRates { input_per_mtok: 3.0, output_per_mtok: 15.0 };
    let result = savings_lines(&StatusSummary::default(), rates, None, (0, 0));
    assert!(result.is_empty(), "no session tokens → no header, no lines");
}

#[test]
fn savings_lines_starts_with_header() {
    let summary = StatusSummary {
        last_input_tokens: Some(500),
        last_output_tokens: Some(100),
        ..StatusSummary::default()
    };
    let rates = BudgetRates { input_per_mtok: 3.0, output_per_mtok: 15.0 };
    let lines = savings_lines(&summary, rates, None, (0, 0));
    assert_eq!(format!("{}", lines[0]), "Savings", "first line is the header");
}

#[test]
fn savings_lines_session_dash_when_rates_unset() {
    let summary = StatusSummary {
        last_input_tokens: Some(500),
        last_output_tokens: Some(100),
        ..StatusSummary::default()
    };
    let rates = BudgetRates::default();
    let lines = savings_lines(&summary, rates, None, (0, 0));
    let row = format!("{}", lines[1]);
    assert!(row.starts_with("  Session:"), "session row: {row}");
    assert!(row.ends_with('—'), "value is the em-dash when rates unset: {row}");
}

#[test]
fn savings_lines_session_shows_dollars() {
    let summary = StatusSummary {
        last_input_tokens: Some(1_000_000),
        last_output_tokens: Some(500_000),
        ..StatusSummary::default()
    };
    let rates = BudgetRates { input_per_mtok: 3.0, output_per_mtok: 15.0 };
    let lines = savings_lines(&summary, rates, None, (0, 0));
    // 1.0*3 + 0.5*15 = $10.50; right-aligned under the header.
    assert_eq!(format!("{}", lines[1]), "  Session:      $10.50");
    assert_eq!(lines.len(), 2, "header + session only");
}

#[test]
fn savings_lines_shows_milestone_when_provided() {
    let summary = StatusSummary {
        last_input_tokens: Some(1_000_000),
        last_output_tokens: Some(500_000),
        ..StatusSummary::default()
    };
    let rates = BudgetRates { input_per_mtok: 3.0, output_per_mtok: 15.0 };
    let lines = savings_lines(&summary, rates, Some((1_000_000, 500_000)), (0, 0));
    assert_eq!(lines.len(), 3, "header + session + milestone, no project (0,0)");
    assert!(format!("{}", lines[2]).contains("Milestone:"), "{:?}", lines);
}

#[test]
fn savings_lines_shows_all_three_scopes_value_aligned() {
    let summary = StatusSummary {
        last_input_tokens: Some(500_000),
        last_output_tokens: Some(200_000),
        ..StatusSummary::default()
    };
    let rates = BudgetRates { input_per_mtok: 3.0, output_per_mtok: 15.0 };
    let lines = savings_lines(&summary, rates, Some((2_000_000, 800_000)), (10_000_000, 4_000_000));
    let text: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();
    assert_eq!(lines.len(), 4, "header + session + milestone + project: {text:?}");
    assert_eq!(text[0], "Savings");
    assert!(text[1].contains("Session:"),   "{}", text[1]);
    assert!(text[2].contains("Milestone:"), "{}", text[2]);
    assert!(text[3].contains("Project:"),   "{}", text[3]);
    // Alignment guarantee: all three scope rows share one width, so their
    // right-aligned values land in the same column.
    let widths: Vec<usize> = text[1..].iter().map(|s| s.chars().count()).collect();
    assert!(widths.iter().all(|&w| w == widths[0]),
        "scope rows must be equal width for value alignment: {widths:?}");
}
```

## End-to-end verification

After all gates pass, observe the live dashboard (if a session is available):

1. The Budget panel shows a `"Savings"` header with a `"  Session:   $X.XX"` row
   beneath it (value `"—"` when rates unset), dollar value right-aligned.
2. When `rexymcp.toml` has a `[telemetry] dir` set and `phase_runs.jsonl` exists
   there with entries, two additional rows appear: `"  Milestone:  $X.XX"` and
   `"  Project:   $X.XX"`, with their decimals aligned under the session row's.
3. When no telemetry dir is configured, only the header + `Session:` row appear —
   no panic or error.
4. Confirm existing `phase_runs.jsonl` records (written before this phase) still
   parse cleanly (the `serde(default)` field defaults to `None`).

If no live session is available, the gate suite is sufficient.

## Authorizations

- Edit `executor/src/agent/mod.rs`.
- Edit `executor/src/agent/tests.rs` (the `fn input()` helper only).
- Edit `executor/src/agent/metrics.rs`.
- Edit `executor/src/store/telemetry.rs`.
- Edit `mcp/src/runner.rs`.
- Edit `mcp/src/dashboard/mod.rs`.
- Edit `mcp/src/dashboard/event_loop.rs`.
- Edit `mcp/src/dashboard/panels.rs`.
- Edit `mcp/src/dashboard/render.rs`.
- Edit `mcp/src/main.rs`.

## Out of scope

- Chart / sparkline visualization of savings history.
- Any savings data for intermediate (non-final) turns within a session.
- Persisting session-level savings separately from `PhaseRun` data.
- Any new Cargo dependency.
- Any change to `SessionEvent` variants.

## Update Log

<!-- entries appended below this line -->
