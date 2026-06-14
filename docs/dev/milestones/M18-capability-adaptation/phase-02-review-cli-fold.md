# Phase 02: `rexymcp review` CLI + fold into read paths + skill wiring

**Milestone:** M18 — Capability-Aware Adaptation
**Status:** review
**Depends on:** phase-01 (the `PhaseReview` substrate — done)
**Estimated diff:** ~340 lines
**Tags:** language=rust, kind=feature, size=m

## Goal

Phase-01 built the store-layer substrate (`PhaseReview`, `append_review`,
`read_reviews`, `fold_reviews`, `FAILURE_CLASSES`) but **nothing produces or
consumes** review annotations yet. This phase closes the loop:

1. A new **`rexymcp review` CLI subcommand** that builds a `PhaseReview` from
   architect-supplied flags and persists it via `append_review` — the *producer*.
2. **Folding `read_reviews` + `fold_reviews` into the three existing read
   paths** (`runs`, `scorecard`, `model_scorecard`) so the supervision columns
   that are always-empty today (`architect_verdict`, `bounces_to_approval`,
   `bugs_filed`, `warnings`) finally carry real data — the *consumers*.
3. **Wiring the `/rexymcp:review` skill** to invoke the new CLI after it writes
   its verdict block, so every review write-back happens automatically.

After this phase the eval loop is live end-to-end: the architect reviews → the
verdict lands in the store → the scorecard reflects it.

## Architecture references

Read before starting:

- `docs/architecture.md#model-effectiveness-metrics--the-scorecard` — the
  supervision label these read paths surface.
- `docs/architecture.md#layer-2--mcp-crate-binary` — `model_scorecard`, the
  `rexymcp runs` / `rexymcp scorecard` CLIs (the surfaces this phase touches).
- `docs/dev/milestones/M18-capability-adaptation/README.md` § "The failure-class
  taxonomy" — the vocabulary the CLI validates against.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the phase-01 doc (`phase-01-review-substrate.md`) — it defines every
   substrate function this phase calls.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

The phase-01 substrate is in `executor/src/store/telemetry.rs` and is `pub`:

```rust
pub const REVIEW_RECORD_TAG: &str = "review";          // PhaseReview.record value
pub fn is_known_failure_class(class: &str) -> bool;    // advisory validator
pub struct PhaseReview { record, ts, phase_doc_path, phase_id, project_id,
                         architect_verdict, bounces_to_approval, bugs_filed,
                         warnings, failure_class }     // see phase-01 §2 for fields
pub fn append_review(telemetry_dir: &Path, review: &PhaseReview) -> std::io::Result<PathBuf>;
pub fn read_reviews(path: &Path) -> std::io::Result<Vec<PhaseReview>>;
pub fn fold_reviews(runs: Vec<PhaseRun>, reviews: &[PhaseReview]) -> Vec<PhaseRun>;
```

**Note the path-argument asymmetry** (copied from the phase-01 `append`/`read`
pair): `append_review` takes the telemetry **directory** (it joins
`phase_runs.jsonl` itself); `read_reviews` and `read` take the **file** path.
The CLI resolves both — see the worked example below.

### Worked example A — the clap subcommand pattern (`mcp/src/main.rs`)

Add a `Review` variant to the `Commands` enum, mirroring `Scorecard`
(`main.rs:103-128`). Required `--config`, plus the review fields. Optional
fields are `Option<…>`; `--failure-class` is repeatable (`Vec<String>`, same
shape as `--tag` at `main.rs:113-115`):

```rust
    /// Record an architect review verdict as a PhaseReview annotation
    Review {
        /// Path to the config file
        #[arg(long)]
        config: PathBuf,

        /// Absolute path to the phase doc under review (primary fold key)
        #[arg(long)]
        phase_doc: Option<PathBuf>,

        /// Phase id label (e.g. phase-01); also the fallback fold key
        #[arg(long)]
        phase_id: String,

        /// Project id; defaults to [project].id from config when omitted
        #[arg(long)]
        project_id: Option<String>,

        /// The verdict string (e.g. approved_first_try, approved_after_1, escalated)
        #[arg(long)]
        verdict: String,

        /// Failure class from the canonical vocabulary; repeat for several
        #[arg(long = "failure-class")]
        failure_class: Vec<String>,

        /// Bounces to approval
        #[arg(long)]
        bounces: Option<u32>,

        /// Bugs filed during review
        #[arg(long)]
        bugs_filed: Option<u32>,

        /// Warnings noted during review
        #[arg(long)]
        warnings: Option<u32>,

        /// Override the telemetry phase_runs.jsonl path
        #[arg(long)]
        telemetry_path: Option<PathBuf>,
    },
```

The dispatch arm mirrors `Commands::Scorecard` (`main.rs:329-366`) — thin glue
that delegates to a module function, prints, and exits nonzero on `Err`. The
**clock is injected at the call site** (STANDARDS testing rule — no real
`Utc::now()` inside the testable function); use the exact millis idiom already
in `main.rs` (`runs` arm, `main.rs:321-324`):

```rust
        Commands::Review {
            config, phase_doc, phase_id, project_id, verdict,
            failure_class, bounces, bugs_filed, warnings, telemetry_path,
        } => {
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            let args = review::ReviewArgs {
                phase_doc: phase_doc.as_deref(),
                phase_id: &phase_id,
                project_id: project_id.as_deref(),
                verdict: &verdict,
                failure_class: &failure_class,
                bounces, bugs_filed, warnings,
            };
            match review::record_review(&config, telemetry_path.as_deref(), now_ms, &args) {
                Ok(outcome) => {
                    for unknown in &outcome.unknown_classes {
                        eprintln!("warning: unknown failure class {:?} (recorded anyway); known classes: {:?}",
                            unknown, rexymcp_executor::store::telemetry::FAILURE_CLASSES);
                    }
                    println!("recorded review for {} -> {}", phase_id, outcome.path.display());
                    Ok(())
                }
                Err(e) => { eprintln!("{e}"); std::process::exit(1); }
            }
        }
```

Add `mod review;` to the module list (`main.rs:6-17`).

### Worked example B — the testable module function (`mcp/src/runs.rs` shape)

Put the logic in a new `mcp/src/review.rs`, mirroring `runs::load_runs`
(`runs.rs:152-176`): resolve the telemetry path from config-or-override, build
the record, persist it. **`ts` is a parameter** (injected), never read from a
real clock inside this function.

```rust
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
    let cfg = Config::load_with_env(config_path)
        .map_err(|e| format!("failed to load config: {}", e))?;

    // Resolve the telemetry DIRECTORY (append_review joins phase_runs.jsonl).
    let telemetry_dir: PathBuf = if let Some(p) = telemetry_path {
        // The override names the file; append_review needs its parent dir.
        p.parent().map(Path::to_path_buf).ok_or_else(|| {
            "invalid --telemetry-path: no parent directory".to_string()
        })?
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

    Ok(ReviewOutcome { path, unknown_classes })
}
```

**Identity gotcha — pin the path form.** `fold_reviews` matches on
`phase_doc_path` *string equality* with the run's stored `phase_doc_path`, which
the executor wrote as `Some(input.phase_doc_path.clone())` — the **absolute**
path the dispatch skill passed to `execute_phase`
(`executor/src/agent/metrics.rs:109`). So the `--phase-doc` the review CLI
receives must be the **same absolute string**, or the fold silently misses and
falls back to `(phase_id, project_id)`. Do **not** canonicalize, relativize, or
re-resolve the path inside `record_review` — store it verbatim as received. The
skill (worked example D) passes the absolute path.

### Worked example C — folding into the three read paths

Three call sites read `PhaseRun`s today and must now fold reviews in. The change
is identical at each: after the existing `read(...)`, also `read_reviews(...)`
the same file and `fold_reviews`. Use the same fully-qualified call style each
site already uses for `read`.

**Site 1 — `mcp/src/runs.rs:173-175`** (inside `load_runs`):

```rust
    let runs =
        rexymcp_executor::store::telemetry::read(&telemetry_file).map_err(|e| e.to_string())?;
    let reviews = rexymcp_executor::store::telemetry::read_reviews(&telemetry_file)
        .map_err(|e| e.to_string())?;
    let runs = rexymcp_executor::store::telemetry::fold_reviews(runs, &reviews);
    Ok(select(runs, filter))
```

**Site 2 — `mcp/src/scorecard_cli.rs:31-33`** (inside `load_settings_scorecard`):

```rust
    let runs: Vec<PhaseRun> =
        rexymcp_executor::store::telemetry::read(&telemetry_file).map_err(|e| e.to_string())?;
    let reviews = rexymcp_executor::store::telemetry::read_reviews(&telemetry_file)
        .map_err(|e| e.to_string())?;
    let runs = rexymcp_executor::store::telemetry::fold_reviews(runs, &reviews);
    Ok(aggregate_by_settings(&runs, filter))
```

**Site 3 — `mcp/src/server.rs:292-293`** (inside `model_scorecard_inner`):

```rust
    let runs =
        rexymcp_executor::store::telemetry::read(&telemetry_file).map_err(|e| e.to_string())?;
    let reviews = rexymcp_executor::store::telemetry::read_reviews(&telemetry_file)
        .map_err(|e| e.to_string())?;
    let runs = rexymcp_executor::store::telemetry::fold_reviews(runs, &reviews);
```

`total_runs_considered = runs.len()` (server.rs:295) must stay correct after the
fold — `fold_reviews` returns one `PhaseRun` per input run (it overlays, never
adds or drops runs), so the count is unchanged. Do not move that line above the
fold in a way that changes its value; leaving it where it is is fine.

### Worked example D — wiring the `/rexymcp:review` skill

`plugin/skills/review/SKILL.md` §7 ("On pass") and §8 ("On fail") currently
write the verdict block to the phase doc and commit, but never persist to the
store. Add a step — between writing the verdict block and committing — that runs
the CLI. Insert into §7 (after step **a**, before the existing commit step):

```markdown
   a-bis. **Record the verdict in the telemetry store** so the scorecard sees
   it. Run (absolute `--phase-doc` so it matches the stored run identity):

   `rexymcp review --config <repo>/rexymcp.toml --phase-doc <abs phase-doc path>
   --phase-id <phase short id> --verdict approved_first_try --failure-class none`

   Use `approved_after_N` and the real bounce/bug counts (`--bounces N
   --bugs-filed N`) and the matching `--failure-class` from the taxonomy when
   there were prior bounces.
```

Add the symmetric step to §8 ("On fail") — record the bounce with the bug count
and the failure class that fired:

```markdown
   c-bis. **Record the bounce in the telemetry store:** `rexymcp review --config
   <repo>/rexymcp.toml --phase-doc <abs phase-doc path> --phase-id <phase short
   id> --verdict bounced --bugs-filed 1 --failure-class <class from taxonomy>`
```

Keep edits minimal and confined to those two sections — do not restructure the
skill. (`SKILL.md` is plugin documentation, explicitly authorized below; it is
NOT one of the forbidden contract docs.)

## Spec

1. **`Commands::Review`** clap variant in `mcp/src/main.rs` (worked example A),
   plus its thin dispatch arm and `mod review;`.
2. **`mcp/src/review.rs`** with `ReviewArgs`, `ReviewOutcome`, and
   `record_review` (worked example B). Clock injected as a `ts` parameter.
   `project_id` falls back to `cfg.project.id`. Unknown failure classes are
   collected and returned (advisory), never rejected.
3. **Fold wire-in** at the three read paths (worked example C): `runs.rs`,
   `scorecard_cli.rs`, `server.rs`.
4. **Skill wiring** in `plugin/skills/review/SKILL.md` §7 and §8 (worked
   example D).

All Rust changes are additive except the three two-line read-path insertions.
No `PhaseRun`/`PhaseReview` field changes (phase-01 froze those).

## Acceptance criteria

- [ ] `cargo build` and `cargo clippy --all-targets --all-features -- -D warnings`
      are clean.
- [ ] `cargo fmt --all --check` is clean.
- [ ] `cargo test` passes (existing + new).
- [ ] `record_review` writes a `PhaseReview` line that `read_reviews` recovers
      with the supplied `verdict`, `phase_doc_path`, and `failure_class`.
- [ ] `record_review` returns an unknown class in `unknown_classes` for a class
      outside `FAILURE_CLASSES`, **and still records it** (the review is written;
      `read_reviews` recovers the unknown class in `failure_class`).
- [ ] `record_review` fills `project_id` from config when `--project-id` is
      omitted.
- [ ] `load_runs` over a store with a run **and** a matching review returns the
      run with `architect_verdict` populated from the review.
- [ ] `load_settings_scorecard` and `model_scorecard_inner` reflect the folded
      verdict (a run that was `architect_verdict: None` in the raw store shows
      the review's verdict after fold).
- [ ] `rexymcp review --help` lists the subcommand (smoke check the clap wiring).

## Test plan

New tests are hermetic (`TempDir`, injected `ts`, no real clock/network):

- In **`mcp/src/review.rs`** `#[cfg(test)] mod tests`:
  - `records_and_reads_back_review` — `record_review` with a known class, then
    `telemetry::read_reviews` on the returned path recovers a `PhaseReview` with
    the same `verdict`/`phase_doc_path`/`failure_class`. Mutation-resistant:
    asserts the recovered `architect_verdict` equals the input, so dropping the
    field assignment fails the test.
  - `unknown_failure_class_is_recorded_not_rejected` — a made-up class is
    returned in `unknown_classes` **and** present in the recovered review's
    `failure_class`. (Pins the advisory-not-rejecting contract.)
  - `project_id_defaults_from_config` — omit `--project-id`; the recovered
    review's `project_id` equals the config's `[project].id`. Pinned negative:
    when `--project-id` IS supplied, it wins over the config value.
- In **`mcp/src/runs.rs`** tests: `load_runs_folds_review_verdict` — write a
  `PhaseRun` (verdict `None`) and a matching `PhaseReview` to one store; assert
  the returned run carries the review's `architect_verdict`. (You can build the
  fixture with `telemetry::append` + `telemetry::append_review`, then call
  `load_runs` with a written config — mirror `server_tests.rs` fixture style,
  `server_tests.rs:340-377`.)
- In **`mcp/src/server_tests.rs`**: `model_scorecard_folds_review` — extend the
  `write_telemetry_fixture` pattern (`server_tests.rs:367-377`) to append a
  review JSONL line keyed to one of the fixture runs; assert the scorecard row
  for that model reflects the folded verdict (e.g. a non-empty
  `approved_first_try_rate`). Pinned negative: a review whose `phase_doc_path`
  matches **no** run leaves all rows' supervision aggregates unchanged.

Pin **behavior and test names**, not exact assertion wording or file placement.

## End-to-end verification

This phase ships a runtime CLI surface — verify it end-to-end, not only via unit
tests:

1. Build: `cargo build`.
2. Create a scratch telemetry dir and a minimal `rexymcp.toml` pointing
   `[telemetry] dir` at it (or use `--telemetry-path`).
3. Append a real `PhaseRun` (any existing store, or run a trivial phase) so there
   is a run to fold onto.
4. `rexymcp review --config <toml> --phase-doc <abs path> --phase-id phase-XX
   --verdict approved_first_try --failure-class none` → confirm it prints the
   store path and exits 0.
5. `rexymcp runs --config <toml>` → confirm the run's verdict column now shows
   `approved_first_try` (was empty before step 4).
6. `rexymcp review … --failure-class made_up_class` → confirm the unknown-class
   warning prints to stderr **and** the review is still recorded.

Record the exact commands and their output tails in the Update Log.

## Authorizations

- **Edit `plugin/skills/review/SKILL.md`** — §7 and §8 only, to add the
  `rexymcp review` invocation steps (worked example D). This is the "skill
  wiring" deliverable; it is plugin documentation, not a forbidden contract doc.
- New module `mcp/src/review.rs` and a new `Commands` variant.
- No new dependency (`clap`, `serde`, config already in use). No
  `Cargo.toml`/`docs/architecture.md`/`STANDARDS.md`/`WORKFLOW.md` edit.

## Out of scope

- The `model_profile` aggregation (strengths + ranked failure classes) — phase-03.
- Surfacing the profile to the architect (MCP tool / `rexymcp profile` CLI) —
  phase-04.
- Adding a `failure_class` field to `PhaseRun` or surfacing failure-class data in
  `runs`/`scorecard` output columns — phase-03 reads it from `read_reviews`
  directly; this phase only wires the verdict overlay.
- Any per-model runtime-knob resolution (thread 3, phases 05–06).
- Changing how the executor writes `PhaseRun` (`metrics.rs` stays as-is).
- Refining the "review applies to latest run only" fold limitation (phase-01's
  documented known limitation) — out of scope here.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-06-14 16:12 (started)

**Executor:** rexyMCP executor LLM

**Summary:** Implementing the `rexymcp review` CLI subcommand, `mcp/src/review.rs` module, folding `read_reviews` + `fold_reviews` into the three read paths (`runs`, `scorecard`, `model_scorecard`), and wiring the `/rexymcp:review` skill.

### Update — 2026-06-14 16:15 (complete)

**Summary:** All 4 spec tasks implemented. New `Commands::Review` clap variant with dispatch arm, `mcp/src/review.rs` with `ReviewArgs`, `ReviewOutcome`, and `record_review` (clock injected as `ts` parameter). Three read paths now fold reviews. Skill wiring added to `plugin/skills/review/SKILL.md` §7 (a-bis) and §8 (c-bis). 7 new tests added (3 in `review.rs`, 1 in `runs.rs`, 1 in `server_tests.rs`, plus 1 CLI parse test via the existing pattern).

**Files changed:**
- `mcp/src/main.rs` — added `mod review;`, `Commands::Review` variant, dispatch arm.
- `mcp/src/review.rs` — new module with `ReviewArgs`, `ReviewOutcome`, `record_review`, and 3 unit tests.
- `mcp/src/runs.rs` — fold wire-in at `load_runs`, plus `load_runs_folds_review_verdict` test.
- `mcp/src/scorecard_cli.rs` — fold wire-in at `load_settings_scorecard`.
- `mcp/src/server.rs` — fold wire-in at `model_scorecard_inner`.
- `mcp/src/server_tests.rs` — `model_scorecard_folds_review` test.
- `plugin/skills/review/SKILL.md` — §7 a-bis and §8 c-bis skill wiring steps.
- `docs/dev/milestones/M18-capability-adaptation/phase-02-review-cli-fold.md` — status flip to `review`, update log entries.
- `docs/dev/milestones/M18-capability-adaptation/README.md` — phase table row updated to `review`.

**Verification commands:**
```
cargo fmt --all --check: clean
cargo build: clean (zero warnings)
cargo clippy --all-targets --all-features -- -D warnings: clean
cargo test: 749 passed, 0 failed, 2 ignored
```

**Verification summary:** build clean, clippy clean, format clean, all 749 tests pass (7 new + existing).

**Grep for pinned literals:**
```
grep REVIEW_RECORD_TAG mcp/src/review.rs mcp/src/runs.rs mcp/src/server_tests.rs → 7 matches total (2+2+3)
grep FAILURE_CLASSES mcp/src/main.rs → 1 match (dispatch arm warning)
```

**End-to-end verification:**
```
$ cargo run -- review --config /tmp/rexymcp-e2e/rexymcp.toml --phase-doc /abs/path/to/phase-XX.md --phase-id phase-XX --verdict approved_first_try --failure-class none
recorded review for phase-XX -> /tmp/rexymcp-e2e/telemetry/phase_runs.jsonl

$ cargo run -- review --config /tmp/rexymcp-e2e/rexymcp.toml --phase-doc /abs/path/to/phase-YY.md --phase-id phase-YY --verdict bounced --failure-class made_up_class --bugs-filed 1
warning: unknown failure class "made_up_class" (recorded anyway); known classes: ["none", "false_completion", "prod_unwrap", "multi_site_break", "parse_format", "masked_diagnostic", "scope_deviation", "spec_bug", "infra_blip"]
recorded review for phase-YY -> /tmp/rexymcp-e2e/telemetry/phase_runs.jsonl

$ cargo run -- review --help
Record an architect review verdict as a PhaseReview annotation
Usage: rexymcp review [OPTIONS] --config <CONFIG> --phase-id <PHASE_ID> --verdict <VERDICT>
```

**Notes for review:** The end-to-end `rexymcp runs` command showed "(no runs)" because the e2e store only contained a review line (no `PhaseRun` line) — this is correct behavior since `read` filters out review lines. The fold wire-in was verified via the `load_runs_folds_review_verdict` unit test which writes both a `PhaseRun` and a matching `PhaseReview` to a `TempDir` store and asserts the verdict is populated after the fold.

**Commit:** `feat: add rexymcp review CLI, fold reviews into read paths, wire skill`
