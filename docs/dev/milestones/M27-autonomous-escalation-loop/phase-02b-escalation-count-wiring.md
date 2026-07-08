# Phase 02b: `escalation_count` wiring — retire the orphaned field, count assists from journal records

**Milestone:** M27 — Autonomous Escalation Loop
**Status:** todo
**Depends on:** phase-02 (done)
**Estimated diff:** ~120 lines
**Tags:** language=rust, kind=refactor, size=s

## Goal

The dashboard's "Assists" counter sums `PhaseRun.tier_telemetry.escalation_count`,
but **nothing ever writes a non-zero value** — the executor only sets `tier`, and
escalation is now architect-side (post-run), so a `PhaseRun` can't know how many
assists a phase took. Phase-02 landed the real source: `assist` `ArchitectActivity`
journal records. This phase retires the orphaned `escalation_count` field and
rewires the counter to count `assist` journal records for the project (the same
consolidation phase-01 applied to `escalation_slots`). The counter shows real data
as soon as the `/rexymcp:auto` loop journals its assists.

## Architecture references

Read before starting:

- `docs/dev/milestones/M27-autonomous-escalation-loop/README.md` § "Token/cost
  accounting" — assists are journaled as `ArchitectActivity` records (phase-02);
  `PhaseRun.escalation_count` becomes real by *deriving* from them.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture reference above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

**`executor/src/store/telemetry.rs`** — `TierTelemetry` (lines ~99–120) carries
the orphaned field. The struct doc comment and the `escalation_count` field both
reference an M21 producer that never shipped (escalation moved to M27,
architect-side). The read path for `assist` records already exists from phase-02:
`read_architect_activities(path) -> io::Result<Vec<ArchitectActivity>>`, filtering
on `record == "architect_activity"`; each record has `project_id: Option<String>`
and `activity: String`.

```rust
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TierTelemetry {
    pub tier: Option<Tier>,
    pub doc_level: Option<u8>,
    /// Number of mid-phase Architect assists that fired this run. `0` until M21.
    pub escalation_count: u32,
    /// Architect input tokens spent on assists this run. `0` until M21.
    pub architect_input_tokens: u64,
    /// Architect output tokens spent on assists this run. `0` until M21.
    pub architect_output_tokens: u64,
}
```

`phase_run_tier_telemetry_round_trips` (in `mod tests`) constructs a
`TierTelemetry` literal that sets `escalation_count: 1,`.

**`mcp/src/dashboard/mod.rs`** — `load_data` reads `PhaseRun`s once via
`read_phase_runs`, then folds project-scoped costs **and** the assist count in one
pass (lines ~50–76). The `escalation_count` accumulation (`assists =
assists.saturating_add(r.tier_telemetry.escalation_count)`) is the only reader of
the retired field:

```rust
    let phase_runs: Vec<PhaseRun> = telemetry_dir.map(read_phase_runs).unwrap_or_default();
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
                    assists = assists.saturating_add(r.tier_telemetry.escalation_count);
                    (costs, assists)
                },
            ),
        None => (ScopeCosts::default(), 0u32),
    };
```

`project_escalation_count` flows unchanged into `DashboardData` (lines ~122, 132)
and is rendered by `panels::savings_lines` as the "Assists:" row. **`panels.rs` and
`render.rs` are untouched** — they take the count as a plain `u32` argument; only
*how* `load_data` computes it changes.

The import at the top is `use rexymcp_executor::store::telemetry::PhaseRun;`.

## Spec

Numbered tasks in execution order.

1. **Retire the `escalation_count` field** — in `executor/src/store/telemetry.rs`,
   delete the `escalation_count: u32` field (and its `/// Number of mid-phase …`
   doc comment) from `TierTelemetry`, and rewrite the struct doc comment so it no
   longer references `escalation_count` or the never-shipped M21 producer. Replace
   the struct doc comment + field docs verbatim:

   ```rust
   /// Per-run M20 tier/cost instrumentation. Nested in `PhaseRun` as a single
   /// `#[serde(default)]` field so legacy records and every struct literal need
   /// only `Default` (the `ContextEfficiency` precedent). Only `tier` is
   /// populated by the executor — the configured executor tier from
   /// `[executor] tier`. `doc_level` and the two `architect_*_tokens` default to
   /// `None`/`0`; `architect_*_tokens` are filled by the M27 phase-05 usage
   /// harvester on Claude Code (assist cost is journaled architect-side, not
   /// executor-side). Assist *counts* are derived from `assist`
   /// `ArchitectActivity` journal records, not stored here.
   #[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
   pub struct TierTelemetry {
       /// Configured executor capability tier (`[executor] tier`); `None` when
       /// the project has not run `rexymcp calibrate`.
       pub tier: Option<Tier>,
       /// Phase-doc detail level (1/2/3). `None` until doc levels are wired.
       pub doc_level: Option<u8>,
       /// Architect input tokens attributed to this run's assists. `0` until the
       /// M27 phase-05 harvester fills it.
       pub architect_input_tokens: u64,
       /// Architect output tokens attributed to this run's assists. `0` until the
       /// M27 phase-05 harvester fills it.
       pub architect_output_tokens: u64,
   }
   ```

2. **Fix the round-trip test literal** — in
   `executor/src/store/telemetry.rs` `mod tests`,
   `phase_run_tier_telemetry_round_trips` sets `escalation_count: 1,` in its
   `TierTelemetry { … }` literal. Delete that one line; the rest of the test is
   unchanged (it still pins `tier`, `doc_level`, and the two token fields through
   a JSON round-trip).

3. **Add the back-compat unit test** — in
   `executor/src/store/telemetry.rs` `mod tests`, new test
   `phase_run_ignores_retired_escalation_count_key`: deserialize a `PhaseRun`
   JSON whose `tier_telemetry` object still carries `"escalation_count":3` (an
   old record), assert it parses `Ok` and the surviving fields are intact
   (`architect_input_tokens`, `tier`). Retired keys must be ignored, never a
   parse error (serde ignores unknown keys — no `deny_unknown_fields` on
   `TierTelemetry`). Fixture:

   ```rust
   let json = r#"{"ts":1,"model":"t","generation_params":{},"phase_id":"p","tags":[],"status":"c","escalated":false,"gates":{},"parse_failure_rate":0.0,"repairs_per_call":0.0,"verifier_retries":0,"tool_success_rate":1.0,"turns":1,"wall_clock_s":1.0,"tokens":{},"tier_telemetry":{"tier":"MEDIUM","doc_level":2,"escalation_count":3,"architect_input_tokens":1000,"architect_output_tokens":200}}"#;
   let run: PhaseRun = serde_json::from_str(json).unwrap();
   assert_eq!(run.tier_telemetry.architect_input_tokens, 1000);
   assert_eq!(run.tier_telemetry.tier, Some(Tier::Medium));
   ```

4. **Rewire the dashboard count** — in `mcp/src/dashboard/mod.rs`:

   - Change the import to bring in the module for the read fn:
     `use rexymcp_executor::store::telemetry::{self, PhaseRun};`.
   - Replace the quoted tuple-fold block (Tasks reference "Current state") so the
     cost fold no longer accumulates assists, and `project_escalation_count` is
     computed by counting `assist` `ArchitectActivity` records for the project:

     ```rust
         let phase_runs: Vec<PhaseRun> = telemetry_dir.map(read_phase_runs).unwrap_or_default();
         // project_costs: executor tokens + architect tokens across all project runs.
         let project_costs = match project_id {
             Some(pid) => phase_runs
                 .iter()
                 .filter(|r| r.project_id.as_deref() == Some(pid))
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
                 }),
             None => ScopeCosts::default(),
         };
         // Assists: count `assist` architect-activity journal records for this
         // project (retired tier_telemetry.escalation_count — the executor never
         // wrote it; assists are journaled architect-side by `rexymcp journal`).
         let project_escalation_count = match (project_id, telemetry_dir) {
             (Some(pid), Some(dir)) => {
                 telemetry::read_architect_activities(&dir.join("phase_runs.jsonl"))
                     .unwrap_or_default()
                     .iter()
                     .filter(|a| a.project_id.as_deref() == Some(pid) && a.activity == "assist")
                     .count() as u32
             }
             _ => 0,
         };
     ```

   - Update the `DashboardData.project_escalation_count` doc comment (currently
     `/// Sum of PhaseRun.tier_telemetry.escalation_count across all project
     runs.`) to: `/// Count of assist ArchitectActivity journal records for the
     project.`
   - The two `DashboardData { … project_escalation_count, … }` construction sites
     are unchanged (the binding keeps the same name).

5. **Rewrite the dashboard count test** — in `mcp/src/dashboard/mod.rs`
   `mod tests`, replace `load_data_reads_project_escalation_count` with
   `load_data_counts_assist_journal_records_as_escalations`. Write a real
   `phase_runs.jsonl` holding four `architect_activity` lines — two `assist` for
   this project, one non-assist (`draft`) for this project, one `assist` for a
   **different** project — and assert `data.project_escalation_count == 2`. The
   two negatives (non-assist kind, different project) are load-bearing:

   ```rust
   #[test]
   fn load_data_counts_assist_journal_records_as_escalations() {
       let dir = TempDir::new().unwrap();
       let sessions = sessions_dir(dir.path());
       std::fs::create_dir_all(&sessions).unwrap();
       let pid = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
       let telemetry_dir = dir.path().join("telemetry");
       std::fs::create_dir_all(&telemetry_dir).unwrap();
       // 2 matching assists + 1 non-assist (same project) + 1 assist (other
       // project). Only the 2 matching assists count.
       let lines = concat!(
           r#"{"record":"architect_activity","ts":1,"phase_id":"p1","project_id":"aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee","activity":"assist"}"#, "\n",
           r#"{"record":"architect_activity","ts":2,"phase_id":"p1","project_id":"aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee","activity":"assist"}"#, "\n",
           r#"{"record":"architect_activity","ts":3,"phase_id":"p2","project_id":"aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee","activity":"draft"}"#, "\n",
           r#"{"record":"architect_activity","ts":4,"phase_id":"p1","project_id":"ffffffff-0000-0000-0000-000000000000","activity":"assist"}"#, "\n",
       );
       std::fs::write(telemetry_dir.join("phase_runs.jsonl"), lines).unwrap();

       let data = load_data(dir.path(), None, Some(&telemetry_dir), Some(pid));
       assert_eq!(
           data.project_escalation_count, 2,
           "only assist activities for this project count"
       );
   }
   ```

6. **Clean the retired key from the sibling fixture** — in
   `mcp/src/dashboard/mod.rs` `mod tests`,
   `load_data_reads_project_architect_costs_from_phase_runs` has a `run` fixture
   whose `tier_telemetry` JSON contains `"escalation_count":0,`. Delete that
   `"escalation_count":0,` substring (the field is retired; serde would ignore it,
   but keep fixtures honest). The test's assertions are unchanged.

## Acceptance criteria

- [ ] `grep -rn "escalation_count" executor/src mcp/src` hits only (a) the new
      back-compat test fixture in `telemetry.rs` and (b) any prose in this phase
      doc — **no struct field, no dashboard read**.
- [ ] `TierTelemetry` has no `escalation_count` field; `cargo build` compiles
      (the only reader, `dashboard/mod.rs`, no longer references it).
- [ ] `load_data`'s `project_escalation_count` equals the number of `assist`
      `ArchitectActivity` records whose `project_id` matches — verified by
      `load_data_counts_assist_journal_records_as_escalations` (2 of 4 lines).
- [ ] A `PhaseRun` JSON whose `tier_telemetry` still carries `escalation_count`
      deserializes `Ok` (back-compat pin, Task 3).
- [ ] `mcp/src/dashboard/panels.rs` and `render.rs` are unchanged.
- [ ] `cargo build` succeeds with zero new warnings.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes.
- [ ] `cargo fmt --all --check` passes.
- [ ] `cargo test` passes (existing + new tests).

## Test plan

New/changed tests (hermetic, `TempDir`):

- `phase_run_ignores_retired_escalation_count_key` (telemetry.rs) — back-compat
  pin.
- `phase_run_tier_telemetry_round_trips` (telemetry.rs) — `escalation_count: 1`
  literal line dropped; otherwise unchanged.
- `load_data_counts_assist_journal_records_as_escalations` (dashboard/mod.rs) —
  replaces `load_data_reads_project_escalation_count`; counts assist journal
  records with two pinned negatives (non-assist kind, other project).

Unchanged behavior pins: `savings_lines_assists_shows_project_escalation_count`
(panels.rs) passes unmodified — it feeds the count as a direct `u32` arg, proving
the render surface is untouched.

## End-to-end verification

The counting logic is exercised end-to-end by
`load_data_counts_assist_journal_records_as_escalations`, which writes a **real**
on-disk `phase_runs.jsonl` and calls the **real** `load_data` (no mock) — the same
read-and-count path the live dashboard runs. The only rendering surface is the
interactive `rexymcp dashboard` TUI, which is not hermetically scriptable, so this
is the end-to-end check for the behavior.

Additionally, exercise the real producer→store path with the phase-02 CLI and
paste the output in the completion Update Log:

1. `rexymcp init --dir <tmp>` (or reuse a scratch config), then run
   `rexymcp journal --config <tmp>/rexymcp.toml --phase-id p1 --activity assist
   --telemetry-path <tmp>/phase_runs.jsonl` twice and once with
   `--activity draft`.
2. `grep -c '"activity":"assist"' <tmp>/phase_runs.jsonl` → `2` — the exact
   records `project_escalation_count` counts (the `draft` line is excluded).

## Authorizations

None. No new dependency, no `Cargo.toml`/`architecture.md`/`STANDARDS.md`/
`WORKFLOW.md` edit.

## Out of scope

- **`mcp/src/dashboard/panels.rs` and `render.rs`.** The "Assists:" row and the
  `project_escalation_count` argument are unchanged — do not touch them.
- **`architect_*_tokens` on `TierTelemetry`.** They stay; the M27 phase-05
  harvester fills them. Only `escalation_count` is retired.
- **Milestone-scoped assist counts.** The Assists counter is project-wide only
  (the milestone cost fold never counted assists). Do not add a milestone-scoped
  assist count.
- **This repo's own `~/.rexymcp/telemetry/phase_runs.jsonl`.** Any real records
  carrying `escalation_count` parse fine after this phase (unknown key, ignored).
  Do not edit the live store.

## Gotchas (pre-injected)

- **serde ignores unknown keys** — removing the `escalation_count` field cannot
  make an old `phase_runs.jsonl` line fail to parse (`TierTelemetry` has no
  `deny_unknown_fields`). The Task-3 test pins exactly this. If a parse fails
  after Task 1, you broke a struct-literal or a fixture, not the store format.
- **`"assist"` is the string literal to match** — the vocabulary lives in
  `ARCHITECT_ACTIVITIES` as string literals (like `FAILURE_CLASSES`); matching
  `a.activity == "assist"` is consistent with how `"none"` etc. are compared
  elsewhere. No new constant is needed.
- **Keep the binding name `project_escalation_count`** — the two `DashboardData`
  construction sites and the render path reference it by that name. Renaming it
  would churn `render.rs`/`panels.rs`, which are out of scope.
- **The count is `usize`; cast with `as u32`** — matches the existing `as u64`
  token casts in the same function (clippy `all` allows it; a saturating cast is
  unnecessary for a per-project assist count).

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
