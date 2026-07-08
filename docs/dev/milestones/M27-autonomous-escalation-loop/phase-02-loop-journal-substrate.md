# Phase 02: Loop-journal telemetry substrate (`ArchitectActivity` + `rexymcp journal` CLI)

**Milestone:** M27 — Autonomous Escalation Loop
**Status:** todo
**Depends on:** phase-01 (done)
**Estimated diff:** ~430 lines
**Tags:** language=rust, kind=feature, size=l

## Goal

The `/rexymcp:auto` loop (phase-06) needs a portable, structured record of every
architect activity — `draft`, `dispatch`, `review`, `assist`, `takeover`,
`boundary` — so a run's work is auditable and, later, cost-attributable
(phase-05 joins token usage onto these). This phase lands that substrate: a new
`ArchitectActivity` append-only telemetry record, its store API, and a
`rexymcp journal` CLI subcommand the loop skill invokes to write one (mirroring
how `rexymcp review` writes a `PhaseReview`). It **retires** the dead M20
`EscalationEvent` — a record type with zero producers and zero readers whose
`assist` concern is now just one `ArchitectActivity` kind (the same
consolidation phase-01 applied to `escalation_slots`).

No consumer of `ArchitectActivity` ships here: the dashboard Assists-counter
rewiring and the retirement of the orphaned `tier_telemetry.escalation_count`
field are **phase-02b** (a small follow-on). This phase is the write-side
substrate only.

## Architecture references

Read before starting:

- `docs/dev/milestones/M27-autonomous-escalation-loop/README.md` § "Token/cost
  accounting" — the loop-journal design (the six activity kinds; portable
  counts-and-durations now, harvested tokens in phase-05).
- `docs/architecture.md` § Status #27 — milestone context.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

**`executor/src/store/telemetry.rs`** — the single append-only JSONL store
(`phase_runs.jsonl`) holds three discriminated record types today: `PhaseRun`
(no `record` field), `PhaseReview` (`record = "review"`), and `EscalationEvent`
(`record = "escalation"`). Each non-`PhaseRun` type has a `record` discriminator
that its reader filters on, so the readers never confuse line types.

`EscalationEvent` (lines 401–425), its tag `ESCALATION_RECORD_TAG` (429), and
its store fns `append_escalation` (434–450) / `read_escalations` (455–467) are
**dead** — `grep -rn "EscalationEvent\|read_escalations\|append_escalation" mcp/
executor/src` finds only the definitions and their own tests; no production code
writes or reads one. It was built in M20 for a SMALL-tier executor-side
escalation loop that M27 replaced with the architect-side `/rexymcp:auto` loop.

The **exact shape** to mirror for the new record's store API — quote
`EscalationEvent`, `append_escalation`, `read_escalations` verbatim as the
template (they are what `ArchitectActivity` replaces):

```rust
pub const ESCALATION_RECORD_TAG: &str = "escalation";

pub fn append_escalation(
    telemetry_dir: &Path,
    event: &EscalationEvent,
) -> std::io::Result<PathBuf> {
    use std::io::Write;

    std::fs::create_dir_all(telemetry_dir)?;
    let path = telemetry_dir.join("phase_runs.jsonl");
    let line = serde_json::to_string(event).map_err(std::io::Error::other)?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    file.write_all(line.as_bytes())?;
    file.write_all(b"\n")?;
    Ok(path)
}

pub fn read_escalations(path: &Path) -> std::io::Result<Vec<EscalationEvent>> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    Ok(content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<EscalationEvent>(l).ok())
        .filter(|e| e.record == ESCALATION_RECORD_TAG)
        .collect())
}
```

The **advisory-vocabulary** pattern to mirror for activity-kind validation is
`FAILURE_CLASSES` + `is_known_failure_class` (lines 306–321):

```rust
pub const FAILURE_CLASSES: &[&str] = &[
    "none",
    // ...
];

pub fn is_known_failure_class(class: &str) -> bool {
    FAILURE_CLASSES.contains(&class)
}
```

**`mcp/src/review.rs`** — the entire file is the producer-CLI template for the
new `journal.rs`. `record_review` (lines 33–89) resolves the telemetry
directory from config or a `--telemetry-path` file override, defaults
`project_id` from `[project].id`, validates `failure_class` advisorily (unknown
classes returned, not rejected), builds the record, and appends it. Copy its
config/telemetry-dir resolution and `project_id` defaulting **exactly**.

**`mcp/src/main.rs`** — `mod review;` (line 14); the `Review` clap variant
(lines 229–270); its dispatch arm (lines 519–566). The new `Journal` subcommand
mirrors all three. The dispatch arm's timestamp idiom (lines 531–534):

```rust
let now_ms = std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .map(|d| d.as_millis() as u64)
    .unwrap_or(0);
```

## Spec

Numbered tasks in execution order.

1. **Retire `EscalationEvent`** — in `executor/src/store/telemetry.rs`, delete
   the `EscalationEvent` struct, `ESCALATION_RECORD_TAG`, `append_escalation`,
   and `read_escalations` (with their doc comments). In `mod tests`, delete the
   escalation-only tests `escalation_event_round_trips` and
   `read_escalations_excludes_run_lines` outright. The two **cross-discriminator
   exclusion** tests are not deleted but **converted** to `ArchitectActivity` in
   Task 4 (`read_skips_escalation_lines` → `read_skips_architect_activity_lines`;
   `read_reviews_skips_escalation_lines` →
   `read_reviews_skips_architect_activity_lines`) and the
   `read_escalations_excludes_review_by_discriminator` test becomes
   `read_architect_activities_excludes_review_by_discriminator`. Do the deletion
   first; the compiler (E0425/E0412 in the converted tests) then guides Task 4.

2. **Add the `ArchitectActivity` record + tag** — in
   `executor/src/store/telemetry.rs`, add verbatim:

   ```rust
   /// An append-only record of one architect activity in a `/rexymcp:auto`
   /// loop run — the portable loop journal. Appended to `phase_runs.jsonl`
   /// alongside `PhaseRun` and `PhaseReview`; the `record` discriminator
   /// (`"architect_activity"`) keeps the readers from confusing the line types.
   /// Written by the `rexymcp journal` CLI (the loop skill invokes it); the
   /// executor never writes one. The two `architect_*_tokens` fields default to
   /// `0` and are filled by the phase-05 usage harvester on Claude Code; on
   /// other clients they stay `0` (counts-and-durations, never fabricated).
   #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
   pub struct ArchitectActivity {
       /// Literal discriminator. Always `"architect_activity"`.
       /// `#[serde(default)]` so a `PhaseRun` line (no `record` field)
       /// deserializes to `""` here and is filtered out by
       /// `read_architect_activities`.
       #[serde(default)]
       pub record: String,
       pub ts: u64,
       /// Identity of the phase this activity concerns. Prefer `phase_doc_path`;
       /// `phase_id` + `project_id` are the fallback key (mirrors `PhaseReview`).
       #[serde(default)]
       pub phase_doc_path: Option<String>,
       pub phase_id: String,
       #[serde(default)]
       pub project_id: Option<String>,
       /// Milestone directory slug (e.g. `"M27-autonomous-escalation-loop"`) for
       /// milestone-scoped queries. `None` when the loop did not supply one.
       #[serde(default)]
       pub milestone_id: Option<String>,
       /// The activity kind — one of `ARCHITECT_ACTIVITIES`.
       pub activity: String,
       /// Free-text outcome of the activity (e.g. `"complete"`, `"hard_fail"`,
       /// `"approved_first_try"`, `"bounced"`). `None` when not applicable.
       #[serde(default)]
       pub outcome: Option<String>,
       /// Architect model that performed the activity (e.g. `"claude-opus-4-8"`).
       #[serde(default)]
       pub model: Option<String>,
       /// Architect input tokens for this activity. `0` until the phase-05
       /// harvester fills it.
       #[serde(default)]
       pub architect_input_tokens: u64,
       /// Architect output tokens for this activity. `0` until the phase-05
       /// harvester fills it.
       #[serde(default)]
       pub architect_output_tokens: u64,
   }

   /// The literal value of `ArchitectActivity.record`. Use everywhere instead of
   /// a bare string so the discriminator is single-sourced.
   pub const ARCHITECT_ACTIVITY_RECORD_TAG: &str = "architect_activity";
   ```

3. **Add the activity vocabulary + store API** — in
   `executor/src/store/telemetry.rs`, add the advisory vocabulary (mirroring
   `FAILURE_CLASSES` / `is_known_failure_class`) and the append/read fns
   (mirroring the quoted `append_escalation` / `read_escalations` shape,
   substituting the type and tag):

   ```rust
   /// Canonical architect-activity vocabulary for `ArchitectActivity.activity`.
   /// Intentionally open (new kinds fold in as the loop grows) — a *documented*
   /// vocabulary, not a closed enum, matching `FAILURE_CLASSES`.
   pub const ARCHITECT_ACTIVITIES: &[&str] = &[
       "draft",     // authored or refined a phase doc
       "dispatch",  // dispatched a phase to the executor
       "review",    // reviewed a completed phase against the DoD
       "assist",    // refined + re-dispatched after hard_fail/budget_exceeded
       "takeover",  // took the phase over directly (session takeover)
       "boundary",  // reached a milestone boundary or a loop stop condition
   ];

   /// True if `activity` is in the canonical `ARCHITECT_ACTIVITIES` vocabulary.
   pub fn is_known_activity(activity: &str) -> bool {
       ARCHITECT_ACTIVITIES.contains(&activity)
   }

   /// Append one `ArchitectActivity` as a JSON line to
   /// `<telemetry_dir>/phase_runs.jsonl`. Returns the file path.
   pub fn append_architect_activity(
       telemetry_dir: &Path,
       activity: &ArchitectActivity,
   ) -> std::io::Result<PathBuf> {
       use std::io::Write;

       std::fs::create_dir_all(telemetry_dir)?;
       let path = telemetry_dir.join("phase_runs.jsonl");
       let line = serde_json::to_string(activity).map_err(std::io::Error::other)?;
       let mut file = std::fs::OpenOptions::new()
           .create(true)
           .append(true)
           .open(&path)?;
       file.write_all(line.as_bytes())?;
       file.write_all(b"\n")?;
       Ok(path)
   }

   /// Read all `ArchitectActivity` records from a store file. Lines that are
   /// `PhaseRun` or `PhaseReview` records (or anything without
   /// `record == "architect_activity"`) are skipped.
   pub fn read_architect_activities(path: &Path) -> std::io::Result<Vec<ArchitectActivity>> {
       let content = match std::fs::read_to_string(path) {
           Ok(c) => c,
           Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
           Err(e) => return Err(e),
       };
       Ok(content
           .lines()
           .filter(|l| !l.trim().is_empty())
           .filter_map(|l| serde_json::from_str::<ArchitectActivity>(l).ok())
           .filter(|a| a.record == ARCHITECT_ACTIVITY_RECORD_TAG)
           .collect())
   }
   ```

4. **Add/convert the `telemetry.rs` tests** — in `mod tests`:
   - `architect_activity_round_trips` — build an `ArchitectActivity`
     (`record = ARCHITECT_ACTIVITY_RECORD_TAG`, `activity = "assist"`, non-null
     `outcome`/`model`/token fields), `append_architect_activity` to a `TempDir`,
     `read_architect_activities` back, assert `len() == 1` and equality.
   - `read_architect_activities_excludes_run_lines` — `append` a `sample()`
     `PhaseRun`, assert `read_architect_activities` returns empty.
   - `read_architect_activities_excludes_review_by_discriminator` — the
     **load-bearing discriminator pin** (M18 bug-01-1 lesson). Build an
     activity-SHAPED record whose `record` is `REVIEW_RECORD_TAG` (wrong tag,
     right shape), append it, assert `read_architect_activities` returns empty —
     so **deleting the `.filter(|a| a.record == ARCHITECT_ACTIVITY_RECORD_TAG)`
     line fails this test**, not just a structural mismatch. Keep this comment in
     the test.
   - `read_skips_architect_activity_lines` (converted from
     `read_skips_escalation_lines`) — `append` a `PhaseRun` and
     `append_architect_activity` an activity; assert `read` returns exactly the
     one `PhaseRun`.
   - `read_reviews_skips_architect_activity_lines` (converted from
     `read_reviews_skips_escalation_lines`) — `append_architect_activity` an
     activity; assert `read_reviews` returns empty.
   - `is_known_activity_validates_vocabulary` — assert `is_known_activity`
     is true for `"draft"`, `"assist"`, `"boundary"` and false for `"made_up"`.

5. **Add the `journal.rs` producer** — create `mcp/src/journal.rs`, modeled on
   `mcp/src/review.rs`. Copy `record_review`'s telemetry-dir resolution
   (config `cfg.telemetry.dir` or `--telemetry-path` parent) and `project_id`
   defaulting from `[project].id` **exactly**. Shape:

   ```rust
   //! Architect loop-journal write-back — `rexymcp journal` subcommand.

   use std::path::{Path, PathBuf};

   use rexymcp_executor::config::Config;
   use rexymcp_executor::store::telemetry::{
       self, ARCHITECT_ACTIVITY_RECORD_TAG, ArchitectActivity, is_known_activity,
   };

   /// Borrowed journal inputs from the CLI flags.
   pub struct JournalArgs<'a> {
       pub phase_doc: Option<&'a Path>,
       pub phase_id: &'a str,
       pub project_id: Option<&'a str>,
       pub milestone_id: Option<&'a str>,
       pub activity: &'a str,
       pub outcome: Option<&'a str>,
       pub model: Option<&'a str>,
   }

   /// Result of recording an activity: the store path and the activity kind if
   /// it was outside the canonical vocabulary (recorded anyway; caller warns).
   pub struct JournalOutcome {
       pub path: PathBuf,
       pub unknown_activity: Option<String>,
   }

   /// Build an `ArchitectActivity` from `args` (stamped with `ts`) and append it
   /// to the telemetry store. Validation of `activity` is advisory: an unknown
   /// kind is returned, not rejected.
   pub fn record_activity(
       config_path: &Path,
       telemetry_path: Option<&Path>,
       ts: u64,
       args: &JournalArgs,
   ) -> Result<JournalOutcome, String> {
       let cfg = Config::load_with_env(config_path)
           .map_err(|e| format!("failed to load config: {}", e))?;

       let telemetry_dir: PathBuf = if let Some(p) = telemetry_path {
           p.parent()
               .map(Path::to_path_buf)
               .ok_or_else(|| "invalid --telemetry-path: no parent directory".to_string())?
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

       let unknown_activity = (!is_known_activity(args.activity))
           .then(|| args.activity.to_string());

       let activity = ArchitectActivity {
           record: ARCHITECT_ACTIVITY_RECORD_TAG.to_string(),
           ts,
           phase_doc_path: args.phase_doc.map(|p| p.to_string_lossy().into_owned()),
           phase_id: args.phase_id.to_string(),
           project_id,
           milestone_id: args.milestone_id.map(str::to_string),
           activity: args.activity.to_string(),
           outcome: args.outcome.map(str::to_string),
           model: args.model.map(str::to_string),
           architect_input_tokens: 0,
           architect_output_tokens: 0,
       };

       let path = telemetry::append_architect_activity(&telemetry_dir, &activity)
           .map_err(|e| format!("failed to append activity: {}", e))?;

       Ok(JournalOutcome {
           path,
           unknown_activity,
       })
   }
   ```

   Add `mod tests` mirroring `review.rs`'s tests (reuse its `make_config`
   helper shape — a `[project] id = "test-project"` + `[telemetry] dir` config):
   - `records_and_reads_back_activity` — round-trip an `"assist"` activity via
     `record_activity`, read via `read_architect_activities`, assert fields.
   - `unknown_activity_is_recorded_not_rejected` — pass `activity = "frobnicate"`;
     assert `unknown_activity == Some("frobnicate")` **and** the record is still
     written and reads back.
   - `project_id_defaults_from_config` — omit `project_id`, assert it falls back
     to `"test-project"`; then pass an explicit one and assert it wins (the
     pinned negative, mirroring `review.rs`).

6. **Wire the `Journal` CLI subcommand** — in `mcp/src/main.rs`:
   - Add `mod journal;` next to `mod review;` (line 14).
   - Add a `Journal` clap variant after `Review` (mirror its flags):
     `config: PathBuf`, `phase_doc: Option<PathBuf>`, `phase_id: String`,
     `project_id: Option<String>`, `milestone_id: Option<String>`,
     `activity: String`, `outcome: Option<String>`, `model: Option<String>`,
     `telemetry_path: Option<PathBuf>`. Doc-comment the variant
     `/// Record an architect loop activity as an ArchitectActivity journal record`.
   - Add the dispatch arm after the `Review` arm, mirroring it: compute `now_ms`
     with the quoted `SystemTime` idiom, build `journal::JournalArgs`, call
     `journal::record_activity`, on `Ok` emit an `eprintln!` warning when
     `outcome.unknown_activity` is `Some` (naming the known vocabulary
     `telemetry::ARCHITECT_ACTIVITIES`) then
     `println!("recorded {activity} activity for {phase_id} -> {path}")`, on
     `Err` `eprintln!` + `std::process::exit(1)`.

## Acceptance criteria

- [ ] `grep -rn "EscalationEvent\|ESCALATION_RECORD_TAG\|append_escalation\|read_escalations" executor/src mcp/src`
      returns nothing (the type is fully retired).
- [ ] `ArchitectActivity`, `ARCHITECT_ACTIVITY_RECORD_TAG`,
      `append_architect_activity`, `read_architect_activities`,
      `ARCHITECT_ACTIVITIES`, and `is_known_activity` are defined in
      `executor/src/store/telemetry.rs`.
- [ ] A legacy `phase_runs.jsonl` line for a `PhaseRun` or `PhaseReview` is
      **not** read as an `ArchitectActivity` (discriminator pin), and an
      `ArchitectActivity` line is not read by `read` or `read_reviews`.
- [ ] `rexymcp journal --config <cfg> --phase-id phase-02 --activity assist
      --outcome complete` appends one `architect_activity` record to
      `phase_runs.jsonl` and reading it back yields `activity == "assist"`,
      `outcome == Some("complete")`, `project_id` defaulted from `[project].id`.
- [ ] An unknown `--activity` value is recorded (not rejected) and the CLI
      prints a warning naming the known vocabulary.
- [ ] `cargo build` succeeds with zero new warnings.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes.
- [ ] `cargo fmt --all --check` passes.
- [ ] `cargo test` passes (existing + new tests).

## Test plan

New/changed tests (hermetic, `TempDir`):

- `executor/src/store/telemetry.rs`: `architect_activity_round_trips`,
  `read_architect_activities_excludes_run_lines`,
  `read_architect_activities_excludes_review_by_discriminator` (load-bearing
  discriminator pin), `read_skips_architect_activity_lines`,
  `read_reviews_skips_architect_activity_lines`,
  `is_known_activity_validates_vocabulary`.
- `mcp/src/journal.rs`: `records_and_reads_back_activity`,
  `unknown_activity_is_recorded_not_rejected`, `project_id_defaults_from_config`
  (with the explicit-wins pinned negative).

Unchanged behavior pins: all existing `PhaseReview` and `PhaseRun` tests pass
unmodified (only the escalation-specific tests are removed/converted).

## End-to-end verification

The real artifact is the `rexymcp journal` CLI writing a store record. Run
against a scratch config in a temp dir and paste the outputs in the completion
Update Log:

1. `rexymcp init` into a temp dir (gives a `[project] id` + `[telemetry] dir`;
   if the init template's telemetry dir is a real path, pass
   `--telemetry-path <tmp>/phase_runs.jsonl` to keep it hermetic).
2. `rexymcp journal --config <tmp>/rexymcp.toml --phase-id phase-02
   --activity assist --outcome complete --telemetry-path <tmp>/phase_runs.jsonl`
   → prints `recorded assist activity for phase-02 -> <tmp>/phase_runs.jsonl`.
3. Show the appended line: `grep architect_activity <tmp>/phase_runs.jsonl` →
   one JSON object with `"record":"architect_activity"`, `"activity":"assist"`,
   `"outcome":"complete"`.
4. Run once with `--activity frobnicate` and show the stderr warning naming the
   known vocabulary (record still written).

## Authorizations

- Creating `mcp/src/journal.rs` (a new file) is in scope — it is the CLI
  producer this phase ships.
- No new dependency. No `Cargo.toml`, `architecture.md`, `STANDARDS.md`, or
  `WORKFLOW.md` edit.

## Out of scope

- **The dashboard Assists counter and `tier_telemetry.escalation_count`.**
  Rewiring the counter to derive from `assist` journal records and retiring the
  orphaned `escalation_count` field is **phase-02b**. Do not touch
  `mcp/src/dashboard/` or the `TierTelemetry` struct.
- **Filling `architect_*_tokens`.** They stay `0` here; the phase-05 usage
  harvester joins real token counts onto these records. Do not estimate or
  fabricate token counts.
- **Any `/rexymcp:auto` loop skill logic.** The consumer of `rexymcp journal`
  is the phase-06 skill; this phase only ships the command.
- **`assist_index` / per-assist ordinal.** The loop derives ordinals from the
  journal sequence; no such field is added.

## Gotchas (pre-injected)

- **serde ignores unknown keys** — retiring `EscalationEvent` cannot break
  parsing of any old line, and no production code ever wrote an `escalation`
  line, so there is no data to migrate. If a test fails after Task 1, you broke
  a converted test's reference, not the store format.
- **The discriminator filter is load-bearing, not decorative.** `read`,
  `read_reviews`, and `read_architect_activities` all share one JSONL file;
  each relies on `record`-tag filtering to reject the other two types. The
  `read_architect_activities_excludes_review_by_discriminator` test exists
  specifically so that deleting the `.filter(...)` line fails a test (M18
  bug-01-1). Do not "simplify" it away.
- **Mirror `review.rs` exactly for config/telemetry-dir resolution.** The
  `--telemetry-path` override names the *file*; `append_*` needs its *parent*
  dir. `project_id` falls back to `cfg.project.id` only when the flag is
  absent. Copy the shape; do not re-derive it.
- **`ArchitectActivity` derives `PartialEq`** (like `PhaseReview` /
  `EscalationEvent`) so round-trip tests can assert equality — it holds no
  non-`PartialEq` field (no `TokenBreakdown`), so this is safe.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
