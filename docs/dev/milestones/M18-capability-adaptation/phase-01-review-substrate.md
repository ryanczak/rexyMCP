# Phase 01: Review write-back substrate

**Milestone:** M18 — Capability-Aware Adaptation
**Status:** review
**Depends on:** none
**Estimated diff:** ~320 lines
**Tags:** language=rust, kind=feature, size=m

## Goal

Give the architect's review verdict a durable home in the telemetry store. Today
the supervision fields (`architect_verdict`, `bounces_to_approval`, `bugs_filed`,
`warnings`) are written `None` by the executor and **never updated** — the
telemetry store is append-only and no write-back path exists. This phase adds the
**store-layer substrate**: a `PhaseReview` annotation record (carrying a new
structured **failure-class**), an append function, a reader, and a fold that
overlays the latest review onto its matching `PhaseRun`. The `rexymcp review` CLI
that *produces* these records and the aggregation rewiring that *consumes* the
folded data are phase-02 — this phase is the pure store layer they both build on.

## Architecture references

Read before starting:

- `docs/architecture.md#model-effectiveness-metrics--the-scorecard` — the
  `PhaseRun` record and the supervision label this phase finally makes writable.
- `docs/dev/milestones/M18-capability-adaptation/README.md` § "The failure-class
  taxonomy" — the canonical vocabulary this phase encodes.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

Everything in this phase lives in **`executor/src/store/telemetry.rs`** — the
same file as `PhaseRun`. The relevant existing code:

The append/read pair you will mirror (`telemetry.rs:159-188`):

```rust
/// Append one `PhaseRun` as a JSON line to `<telemetry_dir>/phase_runs.jsonl`,
/// creating the directory if needed. Returns the file path.
pub fn append(telemetry_dir: &Path, run: &PhaseRun) -> std::io::Result<PathBuf> {
    use std::io::Write;

    std::fs::create_dir_all(telemetry_dir)?;
    let path = telemetry_dir.join("phase_runs.jsonl");
    let line = serde_json::to_string(run).map_err(std::io::Error::other)?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    file.write_all(line.as_bytes())?;
    file.write_all(b"\n")?;
    Ok(path)
}

/// Read all `PhaseRun` records from a store file (skips blank/corrupt lines).
pub fn read(path: &Path) -> std::io::Result<Vec<PhaseRun>> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    Ok(content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<PhaseRun>(l).ok())
        .collect())
}
```

The legacy-tolerant optional-field idiom `PhaseRun` already uses
(`telemetry.rs:108-156`) — copy this shape for every new optional field:

```rust
    /// Full path to the phase doc, for milestone-aware savings queries.
    /// `None` for legacy records that predate this field (M7 phase-08b and earlier).
    #[serde(default)]
    pub phase_doc_path: Option<String>,
```

`PhaseRun` carries the fields a review must key against:
`phase_doc_path: Option<String>` (`:111`), `project_id: Option<String>`
(`:150`), `phase_id: String` (`:107`), and `ts: u64` (`:103`).

## Spec

All tasks edit `executor/src/store/telemetry.rs`. All additive — no existing
`PhaseRun` field, function, or test changes.

### 1. The canonical failure-class vocabulary

Add a module-level const slice and a validator:

```rust
/// Canonical failure-class vocabulary for `PhaseReview.failure_class`. The list
/// is intentionally open — new classes fold in as they recur (WORKFLOW
/// § Calibration) — so this is a *documented* vocabulary, not a closed enum.
/// `spec_bug` and `infra_blip` exist so a bounce caused by the architect's spec
/// or by transient infrastructure is NOT charged against the model's competency.
pub const FAILURE_CLASSES: &[&str] = &[
    "none",             // clean approval
    "false_completion", // self-reported complete on a red gate
    "prod_unwrap",      // unwrap/expect in a production path (STANDARDS §2.1)
    "multi_site_break", // breaking multi-site type change ran out of verifier runway
    "parse_format",     // tool-call format / forgiving-parser repair churn
    "masked_diagnostic",// #[allow]/#[ignore] used to hide a warning/error
    "scope_deviation",  // touched out-of-scope files or widened scope
    "spec_bug",         // the bounce was the architect's spec fault, not the model's
    "infra_blip",       // transient backend/decode error, not a work defect
];

/// True if `class` is in the canonical `FAILURE_CLASSES` vocabulary.
pub fn is_known_failure_class(class: &str) -> bool {
    FAILURE_CLASSES.contains(&class)
}
```

Validation is **advisory** (callers — the phase-02 CLI — warn on an unknown
class but still record it); the store layer never rejects. Do not make
`append_review` fail on an unknown class.

### 2. The `PhaseReview` annotation record

Add a struct that annotates one phase's review outcome. The `record` field is a
**discriminator** that lets review lines and `PhaseRun` lines coexist in the
same `phase_runs.jsonl` without either reader mistaking one for the other (see
the Pre-injection gotcha below — this field is load-bearing, not decoration):

```rust
/// An append-only architect-review annotation, folded onto its matching
/// `PhaseRun` at read time. Written by the `rexymcp review` CLI (phase-02);
/// the executor never writes one. Coexists with `PhaseRun` in
/// `phase_runs.jsonl`; the `record` discriminator keeps the two readers from
/// confusing the line types.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PhaseReview {
    /// Literal discriminator. Always `"review"`. `#[serde(default)]` so a
    /// `PhaseRun` line (which has no `record` field) deserializes to `""` here
    /// and is filtered out by `read_reviews`.
    #[serde(default)]
    pub record: String,
    pub ts: u64,
    /// Identity of the phase being reviewed. Prefer `phase_doc_path` (unique per
    /// phase doc); `phase_id` + `project_id` are the fallback key for runs that
    /// predate `phase_doc_path`.
    #[serde(default)]
    pub phase_doc_path: Option<String>,
    pub phase_id: String,
    #[serde(default)]
    pub project_id: Option<String>,
    // the supervision label
    pub architect_verdict: String,
    #[serde(default)]
    pub bounces_to_approval: Option<u32>,
    #[serde(default)]
    pub bugs_filed: Option<u32>,
    #[serde(default)]
    pub warnings: Option<u32>,
    /// Structured failure classes from `FAILURE_CLASSES`. Empty or `["none"]`
    /// for a clean approval. May carry several (a phase can fail two ways).
    #[serde(default)]
    pub failure_class: Vec<String>,
}

/// The literal value of `PhaseReview.record`. Use everywhere instead of a bare
/// string so the discriminator is single-sourced.
pub const REVIEW_RECORD_TAG: &str = "review";
```

### 3. `append_review`

Mirror `append` exactly — same file (`phase_runs.jsonl`), same create/append
flags — but for a `PhaseReview`:

```rust
/// Append one `PhaseReview` as a JSON line to `<telemetry_dir>/phase_runs.jsonl`
/// (the same store as `PhaseRun`). Returns the file path.
pub fn append_review(telemetry_dir: &Path, review: &PhaseReview) -> std::io::Result<PathBuf> {
    // same body shape as `append`, serializing `review`
}
```

### 4. `read_reviews`

Read review lines back from the store, filtering on the discriminator so
`PhaseRun` lines are skipped:

```rust
/// Read all `PhaseReview` records from a store file. Lines that are `PhaseRun`
/// records (or anything without `record == "review"`) are skipped.
pub fn read_reviews(path: &Path) -> std::io::Result<Vec<PhaseReview>> {
    // NotFound -> Ok(vec![]) like `read`; for each non-blank line,
    // serde_json::from_str::<PhaseReview>(l).ok(), then KEEP only those whose
    // `.record == REVIEW_RECORD_TAG`.
}
```

The `.record == REVIEW_RECORD_TAG` filter is **required**, not optional: a
`PhaseRun` line *will* deserialize into a `PhaseReview` (it has `phase_id`/`ts`,
and every other `PhaseReview` field defaults), producing a phantom review with
`record == ""`. The filter drops it.

### 5. `fold_reviews`

Overlay the latest review onto its matching run:

```rust
/// Overlay each `PhaseReview` onto its matching `PhaseRun`, returning runs with
/// the supervision fields populated. For each run, the matching review is the
/// **latest** (max `ts`) review whose phase identity equals the run's:
/// `phase_doc_path` when both have it, else (`project_id`, `phase_id`). A review
/// applies only to the **latest** run sharing that identity (the approved run);
/// earlier bounce runs are left unannotated. Runs with no matching review are
/// returned unchanged.
pub fn fold_reviews(runs: Vec<PhaseRun>, reviews: &[PhaseReview]) -> Vec<PhaseRun> {
    // ...
}
```

Matching rules, precisely:

- **Identity key.** If a run has `phase_doc_path = Some(p)`, its key is
  `Key::Path(p)`. Otherwise its key is `Key::IdProject(phase_id, project_id)`. A
  review computes its key the same way. Two records match iff their keys are
  equal. (Never match on `phase_id` alone — `"phase-01"` recurs in every
  milestone and project.)
- **Latest review wins.** If several reviews share a key, the one with the
  greatest `ts` is applied (a re-review supersedes).
- **Applies to the latest run for the key.** If several runs share a key (a
  phase that bounced and was re-dispatched), apply the review only to the run
  with the greatest `ts`. Earlier runs stay `None`. (A known limitation, fine
  for the substrate; the thread-2 profile can refine attribution later.)
- **Overlay maps:** `architect_verdict = Some(review.architect_verdict)`,
  `bounces_to_approval`/`bugs_filed`/`warnings` copied through. `failure_class`
  is **not** stored on `PhaseRun` in this phase (no `PhaseRun` field is added —
  keep the fold's output a plain `PhaseRun`); the failure-class data reaches
  consumers through `read_reviews` directly in phase-03. Document this in the
  `fold_reviews` doc comment so phase-03 knows where to read it.

## Acceptance criteria

- [ ] `cargo build` and `cargo clippy --all-targets --all-features -- -D warnings`
      are clean.
- [ ] `cargo test -p rexymcp-executor` passes, including the new tests.
- [ ] A `PhaseReview` round-trips through `append_review` → `read_reviews`.
- [ ] `read` (the existing `PhaseRun` reader) over a store containing review
      lines returns **only** the `PhaseRun` records — no phantom run from a
      review line.
- [ ] `read_reviews` over a store containing `PhaseRun` lines returns **only**
      the review records — no phantom review from a run line.
- [ ] `fold_reviews` populates `architect_verdict` on the matching run and
      leaves a non-matching run's supervision fields `None`.

## Test plan

In the `#[cfg(test)] mod tests` block at the bottom of `telemetry.rs`:

- `phase_review_round_trips` — `append_review` then `read_reviews` recovers an
  equal `PhaseReview` (set `record = REVIEW_RECORD_TAG`).
- `read_skips_review_lines` — write one `PhaseRun` (via `append`) and one
  `PhaseReview` (via `append_review`) to the same dir; `read` returns exactly one
  record, the run. **Mutation-resistant:** asserts the returned `phase_id`/model
  is the run's, so a regression that let the review through would fail.
- `read_reviews_skips_run_lines` — same mixed store; `read_reviews` returns
  exactly one record, the review. Asserts `record == "review"`. **This is the
  phantom-review guard** — without the `.record` filter the run line yields a
  second, bogus review and the count assertion fails.
- `fold_reviews_overlays_by_doc_path` — a run and a review sharing
  `phase_doc_path`; after fold the run's `architect_verdict` is the review's.
- `fold_reviews_falls_back_to_id_project` — run/review with `phase_doc_path =
  None` but equal `(phase_id, project_id)` match; differing `project_id` does
  **not** match (pinned negative — two projects' `phase-01` must not cross-fold).
- `fold_reviews_latest_review_wins` — two reviews, same key, different `ts`; the
  higher-`ts` verdict is applied.
- `fold_reviews_applies_to_latest_run` — two runs, same key, different `ts`; only
  the higher-`ts` run is annotated, the earlier stays `None`.
- `fold_reviews_leaves_unmatched_none` — a run with no matching review keeps all
  supervision fields `None`.
- `is_known_failure_class_validates_vocabulary` — `"false_completion"` is known,
  `"made_up"` is not. (Pins the vocabulary without pinning its exact length.)

## End-to-end verification

Not applicable — phase ships no runtime-loadable artifact. This phase adds store
functions and a record type with no CLI/binary surface; the `rexymcp review` CLI
that exercises them end-to-end is phase-02. The hermetic tests over a real
`TempDir` store file are the verification here.

## Authorizations

None. (No new dependency — `serde`/`serde_json` already in use. No
`docs/architecture.md` edit — the M18 §Status entry was added at milestone
kickoff; this phase does not touch it.)

## Out of scope

- The `rexymcp review` CLI subcommand — phase-02.
- Rewiring `runs` / `scorecard` / `model_scorecard` reads to call `fold_reviews`
  — phase-02. This phase only provides the fold; it changes no consumer.
- Adding a `failure_class` field to `PhaseRun` — deliberately not done; the
  failure-class lives on `PhaseReview` and is read directly in phase-03.
- Any change to how the executor writes `PhaseRun` (`metrics.rs` stays as-is).
- The `model_profile` aggregation (thread 2) and any runtime-knob wiring
  (thread 3).

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-06-14 04:58 (started)

**Executor:** rexyMCP executor LLM

**Summary:** Implementing the store-layer substrate for architect review write-back: `PhaseReview` record type, `FAILURE_CLASSES` vocabulary, `append_review`, `read_reviews`, `fold_reviews`, and 9 tests.

### Update — 2026-06-14 04:58 (complete)

**Summary:** All spec tasks implemented in `executor/src/store/telemetry.rs`. Bug-01-1 fix: `read_reviews_skips_run_lines` test now sets `architect_verdict: Some("approved")` on the `PhaseRun` so the serialized line actually deserializes into a phantom `PhaseReview`, making the test mutation-resistant.

**Files changed:**
- `executor/src/store/telemetry.rs` — added `PhaseReview` struct, `REVIEW_RECORD_TAG` const, `FAILURE_CLASSES` const, `is_known_failure_class`, `append_review`, `read_reviews`, `fold_reviews`, and 9 tests. Bug-01-1 fix applied to `read_reviews_skips_run_lines`.
- `docs/dev/milestones/M18-capability-adaptation/phase-01-review-substrate.md` — status flip to `review`, update log entries.
- `docs/dev/milestones/M18-capability-adaptation/README.md` — phase table row updated to `review`.
- `docs/dev/milestones/M18-capability-adaptation/bugs/bug-01-1.md` — status closed.

**Verification commands:**
```
cargo fmt --all --check: clean
cargo build: clean (zero warnings)
cargo clippy --all-targets --all-features -- -D warnings: clean
cargo test -p rexymcp-executor: 749 passed, 0 failed, 2 ignored
```

**Verification summary:** build clean, clippy clean, format clean, all 749 tests pass (9 new + existing).

**Grep for pinned literals:**
```
grep REVIEW_RECORD_TAG executor/src/store/telemetry.rs → 11 matches (const + usage in read_reviews + 9 test sites)
grep FAILURE_CLASSES executor/src/store/telemetry.rs → 4 matches (const + is_known_failure_class + 2 doc refs)
```

**End-to-end verification:** N/A — phase ships no runtime-loadable artifact. Hermetic `TempDir` tests are the verification.

**Notes for review:** Bug-01-1 fix: the phantom-review guard test (`read_reviews_skips_run_lines`) was not mutation-resistant — a `PhaseRun` with `architect_verdict: None` serializes to JSON with `"architect_verdict":null` which fails to deserialize into `PhaseReview` (where `architect_verdict` is a required `String`). The fix sets `architect_verdict: Some("approved")` on the test's `PhaseRun` so the line *does* deserialize, and only the `.record` filter excludes it. Verified by commenting out the filter and confirming the test fails (2 reviews instead of 1).

**Commit:** `feat: add PhaseReview annotation substrate to telemetry store`
