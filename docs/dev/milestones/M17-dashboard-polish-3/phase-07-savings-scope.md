# Phase 07: Savings scope — session + milestone + project

**Milestone:** M17 — Dashboard Polish (Round 3)
**Status:** todo
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

**7b. Add `savings_lines`:**

```rust
/// Budget-panel savings block: session savings (always, when metrics exist),
/// plus milestone and project savings when token data is available.
/// Returns empty when there are no session metrics yet.
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
    let mut lines = Vec::new();

    // Session line — same label as phase-06 established
    lines.push(if no_rates {
        Line::from("Savings: —")
    } else {
        let saved = dollars_saved(in_tok, out_tok, rates.input_per_mtok, rates.output_per_mtok);
        Line::from(format!("Savings: ${saved:.2}"))
    });

    // Milestone line (only when telemetry data available)
    if let Some((m_in, m_out)) = milestone_tok {
        lines.push(if no_rates {
            Line::from("  Milestone: —")
        } else {
            let saved = dollars_saved(m_in, m_out, rates.input_per_mtok, rates.output_per_mtok);
            Line::from(format!("  Milestone: ${saved:.2}"))
        });
    }

    // Project line (only when at least one telemetry record exists)
    let (p_in, p_out) = project_tok;
    if p_in > 0 || p_out > 0 {
        lines.push(if no_rates {
            Line::from("  Project: —")
        } else {
            let saved = dollars_saved(p_in, p_out, rates.input_per_mtok, rates.output_per_mtok);
            Line::from(format!("  Project: ${saved:.2}"))
        });
    }

    lines
}
```

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

Update the import in `render.rs` if needed — `savings_lines` is exported from
`panels.rs` with `pub(crate)` like its predecessor.

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
- [ ] Budget panel shows `"  Milestone: $X.XX"` and `"  Project: $X.XX"` lines when
      telemetry data is present.
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

**Add five `savings_lines_*` tests** (place them in the same location):

```rust
#[test]
fn savings_lines_empty_without_session_metrics() {
    let rates = BudgetRates { input_per_mtok: 3.0, output_per_mtok: 15.0 };
    let result = savings_lines(&StatusSummary::default(), rates, None, (0, 0));
    assert!(result.is_empty(), "no session tokens → no lines");
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
    assert_eq!(format!("{}", lines[0]), "Savings: —");
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
    assert_eq!(format!("{}", lines[0]), "Savings: $10.50");
}

#[test]
fn savings_lines_shows_milestone_when_provided() {
    let summary = StatusSummary {
        last_input_tokens: Some(1_000_000),
        last_output_tokens: Some(500_000),
        ..StatusSummary::default()
    };
    let rates = BudgetRates { input_per_mtok: 3.0, output_per_mtok: 15.0 };
    // Milestone: same token counts → same savings
    let lines = savings_lines(&summary, rates, Some((1_000_000, 500_000)), (0, 0));
    assert!(lines.iter().any(|l| format!("{l}").contains("Milestone:")),
        "Milestone line must appear: {lines:?}");
    assert_eq!(lines.len(), 2, "session + milestone, no project (0,0)");
}

#[test]
fn savings_lines_shows_all_three_scopes() {
    let summary = StatusSummary {
        last_input_tokens: Some(500_000),
        last_output_tokens: Some(200_000),
        ..StatusSummary::default()
    };
    let rates = BudgetRates { input_per_mtok: 3.0, output_per_mtok: 15.0 };
    let milestone_tok = Some((2_000_000u32, 800_000u32));
    let project_tok = (10_000_000u32, 4_000_000u32);
    let lines = savings_lines(&summary, rates, milestone_tok, project_tok);
    let text: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();
    assert_eq!(lines.len(), 3, "session + milestone + project: {text:?}");
    assert!(text[0].contains("Savings:"),   "session line: {}", text[0]);
    assert!(text[1].contains("Milestone:"), "milestone line: {}", text[1]);
    assert!(text[2].contains("Project:"),   "project line: {}", text[2]);
}
```

## End-to-end verification

After all gates pass, observe the live dashboard (if a session is available):

1. The Budget panel still shows `"Savings: $X.XX"` (or `"—"` when rates unset).
2. When `rexymcp.toml` has a `[telemetry] dir` set and `phase_runs.jsonl` exists
   there with entries, two additional lines appear: `"  Milestone: $X.XX"` and
   `"  Project: $X.XX"`.
3. When no telemetry dir is configured, only the session line appears — no panic
   or error.
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
