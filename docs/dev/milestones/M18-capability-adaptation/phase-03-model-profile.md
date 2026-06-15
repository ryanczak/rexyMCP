# Phase 03: `model_profile` aggregation (strengths + ranked failure classes)

**Milestone:** M18 — Capability-Aware Adaptation
**Status:** done
**Depends on:** phase-01 (substrate), phase-02 (write-back loop is live — reviews
now exist in the store)
**Estimated diff:** ~360 lines
**Tags:** language=rust, kind=feature, size=m

## Goal

Now that reviews land in the store (phase-02), build the **per-`(model, tag)`
capability profile** thread 2 needs: each bucket's **strengths** (first-pass
rate, gate-pass rate, reliability means) **and** its **ranked failure classes
with counts**. This phase is the pure **aggregation layer** — a `ModelProfile`
struct and an `aggregate_profiles` function in a new `mcp/src/profile.rs`,
mirroring `mcp/src/scorecard.rs`. The `rexymcp profile` CLI and the MCP tool that
*surface* this to the architect are **phase-04**; this phase ships no binary
surface, exactly as phase-01 shipped no CLI.

The load-bearing wrinkle: **`failure_class` lives on `PhaseReview`, not
`PhaseRun`** (phase-01 deliberately kept it off the run — see its "Out of
scope"). The strengths come from the *folded* runs; the failure-class ranking
comes from the *reviews*, joined back to their matching run to discover the
`(model, tags)` to bucket under.

## Architecture references

Read before starting:

- `docs/architecture.md#model-effectiveness-metrics--the-scorecard` — the
  `model × tag` matrix this profile parallels.
- `docs/dev/milestones/M18-capability-adaptation/README.md` § "The failure-class
  taxonomy" — the vocabulary, and why `spec_bug`/`infra_blip` must NOT be charged
  against a model's competency.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the phase-01 doc (it defines `PhaseReview`, `fold_reviews`,
   `FAILURE_CLASSES`, `is_known_failure_class`).
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

### The aggregation idiom to mirror (`mcp/src/scorecard.rs`)

`aggregate` builds a per-`(model, tag)` matrix by exploding each run across its
tags into a `BTreeMap` of accumulators, then emitting sorted rows. Mirror this
shape exactly. The bucket loop (`scorecard.rs:281-333`):

```rust
    for run in runs {
        if let Some(model) = filter.model
            && run.model != model
        {
            continue;
        }
        if !filter.tags.is_empty() && !filter.tags.iter().all(|t| run.tags.contains(t)) {
            continue;
        }
        for tag in &run.tags {
            let key = (run.model.clone(), tag.clone());
            let acc = buckets.entry(key).or_default();
            acc.n += 1;
            if gates_all_pass(&run.gates) { acc.gates_all_pass += 1; }
            // ... reliability sums ...
            if run.architect_verdict.is_some() {
                acc.n_with_verdict += 1;
                if run.architect_verdict.as_deref() == Some("approved_first_try") {
                    acc.approved_first_try_count += 1;
                }
            }
            if let Some(b) = run.bounces_to_approval { acc.bounces_sum += b as f64; acc.bounces_n += 1; }
        }
    }
```

The emit/mean/sort tail (`scorecard.rs:336-390`): `filter_map` drops buckets with
`acc.n < filter.min_runs`, computes means as `sum / n`, emits `Option<f64>`
means as `Some(sum/n)` only when the relevant `n > 0` else `None`, then sorts:

```rust
    rows.sort_by(|a, b| {
        a.tag.cmp(&b.tag)
            .then(b.n_runs.cmp(&a.n_runs))   // higher n first
            .then(a.model.cmp(&b.model))
    });
```

`gates_all_pass` (`scorecard.rs:271-276`) — a gate that is `None` counts as a
non-pass:

```rust
fn gates_all_pass(gates: &Gates) -> bool {
    gates.fmt == Some(true) && gates.build == Some(true)
        && gates.lint == Some(true) && gates.test == Some(true)
}
```

Reuse the existing **`ScorecardFilter`** (`scorecard.rs:237-245`, `pub`) — do not
define a new filter type:

```rust
pub struct ScorecardFilter<'a> {
    pub tags: &'a [String],
    pub model: Option<&'a str>,
    pub min_runs: usize,
}
```

### The review→run join to reproduce (`executor/src/store/telemetry.rs:201-270`)

To attribute a `PhaseReview`'s `failure_class` to a `(model, tag)` bucket you
must find the review's matching run (which carries `model`/`tags`). Use the
**exact identity-key matching `fold_reviews` uses** — copy this `Key` shape into
`profile.rs` (it is a private local enum there; re-declare your own):

```rust
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
enum Key {
    Path(String),
    IdProject(String, String),
}

fn key_for_review(r: &PhaseReview) -> Key {
    if let Some(ref p) = r.phase_doc_path {
        Key::Path(p.clone())
    } else {
        Key::IdProject(r.phase_id.clone(), r.project_id.clone().unwrap_or_default())
    }
}

fn key_for_run(r: &PhaseRun) -> Key {
    if let Some(ref p) = r.phase_doc_path {
        Key::Path(p.clone())
    } else {
        Key::IdProject(r.phase_id.clone(), r.project_id.clone().unwrap_or_default())
    }
}
```

`fold_reviews` applies a review to the **latest run** (max `ts`) sharing a key;
**failure-class attribution must do the same** so strengths and weaknesses are
counted against the identical run. `fold_reviews` already populated the verdict
on that latest run, so a clean way to stay consistent is: **fold internally,
then drive failure-class attribution off the same latest-run selection** (see
worked example).

### The vocabulary (`executor/src/store/telemetry.rs:277-292`)

`FAILURE_CLASSES` (9 entries incl. `none`, `spec_bug`, `infra_blip`) and
`is_known_failure_class(&str) -> bool` are `pub`. `none` means *no failure*;
`spec_bug`/`infra_blip` are **not the model's fault** (README taxonomy).

## Spec

All code lands in a new **`mcp/src/profile.rs`**, plus a one-line `mod profile;`
in `mcp/src/main.rs` (so it compiles and tests run). Additive — touches no
existing function.

### 1. The output types

```rust
use schemars::JsonSchema;
use serde::Serialize;

/// One failure class and how many reviews in the bucket carried it.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct FailureClassCount {
    pub class: String,
    pub count: usize,
}

/// Per-(model, tag) capability profile: strengths from folded runs, weaknesses
/// from the matched reviews' failure classes.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ModelProfile {
    pub model: String,
    pub tag: String,
    pub n_runs: usize,
    // --- strengths (folded runs) ---
    pub gates_pass_rate: f64,
    pub tool_success_rate_mean: f64,
    pub parse_failure_rate_mean: f64,
    pub escalation_rate: f64,
    /// Number of runs in the bucket carrying an `architect_verdict`.
    pub n_with_verdict: usize,
    /// Fraction of verdict-present runs that were `approved_first_try`.
    /// `None` when `n_with_verdict == 0`.
    pub approved_first_try_rate: Option<f64>,
    /// Mean `bounces_to_approval` over runs where it is `Some`. `None` if none.
    pub bounces_to_approval_mean: Option<f64>,
    // --- weaknesses (matched reviews) ---
    /// Number of reviews attributed to this bucket (its matched run's tags).
    pub n_reviews: usize,
    /// Failure classes seen in this bucket, **excluding `none`**, ranked by
    /// `count` descending then `class` ascending. `spec_bug`/`infra_blip` ARE
    /// included here (they are real observations); use [`is_model_attributable`]
    /// to separate honest model weaknesses from spec/infra noise — the surfacing
    /// layer (phase-04) does that, this layer stays neutral.
    pub ranked_failure_classes: Vec<FailureClassCount>,
}
```

### 2. `is_model_attributable`

```rust
/// False for failure classes that must NOT be charged against a model's
/// competency: `none` (no failure), `spec_bug` (architect's fault),
/// `infra_blip` (transient backend). True for everything else. Single-sources
/// the README taxonomy's "judged on what *it* got wrong" rule.
pub fn is_model_attributable(class: &str) -> bool {
    !matches!(class, "none" | "spec_bug" | "infra_blip")
}
```

### 3. `aggregate_profiles`

```rust
/// Aggregate runs + reviews into per-(model, tag) profiles. `runs` and
/// `reviews` are the raw store reads (`telemetry::read` / `read_reviews`); this
/// function folds internally so callers pass both unmodified. Strengths come
/// from the folded runs; failure-class counts come from each review joined to
/// its matching **latest** run (the same run `fold_reviews` annotates), bucketed
/// under that run's `(model, tag)` pairs.
pub fn aggregate_profiles(
    runs: &[PhaseRun],
    reviews: &[PhaseReview],
    filter: &ScorecardFilter,
) -> Vec<ModelProfile> {
    // ...
}
```

Worked algorithm (pin this behavior):

1. `let folded = telemetry::fold_reviews(runs.to_vec(), reviews);` — strengths
   read `architect_verdict`/`bounces_to_approval` from `folded`.
2. Build `latest_review: HashMap<Key, &PhaseReview>` (max `ts` wins) and
   `latest_run_ts: HashMap<Key, u64>` over `folded` — **exactly** as
   `fold_reviews` does (quoted above).
3. **Strengths pass.** Iterate `folded`, apply the `filter.model`/`filter.tags`
   guards (quoted from `scorecard.rs`), and for each `tag` in the run accumulate
   `n`, `gates_all_pass`, `tool_success_rate_sum`, `parse_failure_rate_sum`,
   `escalated_count`, `n_with_verdict`, `approved_first_try_count`,
   `bounces_sum`/`bounces_n` into the `(model, tag)` bucket.
4. **Weakness pass.** Still inside the same per-run, per-tag loop: if this run is
   the latest run for its key (`run.ts == latest_run_ts[key]`) **and**
   `latest_review` has an entry for the key, then for each `class` in that
   review's `failure_class` where `class != "none"`, increment a
   `BTreeMap<String, usize>` of class→count on the bucket, and increment the
   bucket's `n_reviews` **once per matched review per tag** (i.e. once for this
   run+tag, not once per class).
5. **Emit.** `filter_map` dropping `n < filter.min_runs`. Means are `sum / n`;
   `Option` means follow the `n_with_verdict > 0` / `bounces_n > 0` guards.
   `ranked_failure_classes`: turn the class→count map into
   `Vec<FailureClassCount>` sorted by `count` **descending**, ties broken by
   `class` **ascending** (deterministic — hermetic tests depend on it).
6. **Sort rows** identically to `aggregate`: `tag` asc, then `n_runs` desc, then
   `model` asc.

### Gotchas (pin these — they are the easy mistakes here)

- **`none` is excluded from `ranked_failure_classes`** but the matched review
  still counts toward `n_reviews`. A bucket whose only review is a clean
  `["none"]` approval has `n_reviews >= 1` and an **empty** `ranked_failure_classes`.
- **Attribute to the latest run only.** A key with two runs (a bounce then the
  approved re-dispatch) must count its review's classes once, under the
  **latest** run's tags — not both. Mirrors `fold_reviews`.
- **A review matching no run contributes nothing** — no bucket, no count. (It
  has no `(model, tag)` to attribute to.) Pin this negative.
- **Do not add `failure_class` to `PhaseRun`** or otherwise touch
  `telemetry.rs` — read classes from `reviews` only.

## Acceptance criteria

- [ ] `cargo build`, `cargo clippy --all-targets --all-features -- -D warnings`,
      and `cargo fmt --all --check` are clean.
- [ ] `cargo test` passes (existing + new).
- [ ] `aggregate_profiles` populates strengths (`gates_pass_rate`,
      `approved_first_try_rate`, `bounces_to_approval_mean`) from the folded runs.
- [ ] `ranked_failure_classes` is ordered by count desc then class asc, and
      **excludes `none`**.
- [ ] A failure class is attributed to the `(model, tag)` buckets of its matched
      run; a review matching no run is ignored.
- [ ] When a key has multiple runs, the failure class is counted once, under the
      latest run's tags.
- [ ] `is_model_attributable` returns `false` for `none`/`spec_bug`/`infra_blip`
      and `true` for other classes.
- [ ] `min_runs` drops buckets with fewer runs.

## Test plan

Hermetic tests in `mcp/src/profile.rs` `#[cfg(test)] mod tests`, over hand-built
`Vec<PhaseRun>`/`Vec<PhaseReview>` (build runs with the full struct literal —
copy the field set from `mcp/src/runs.rs`'s `make_run` helper, `runs.rs:184+`,
which lists every `PhaseRun` field; build reviews with the `PhaseReview` literal
from phase-01 §2). Pin **behavior and names**, not exact assertion wording:

- `strengths_come_from_folded_runs` — one run (verdict `None` in the store) + a
  matching `approved_first_try` review; the bucket's `approved_first_try_rate ==
  Some(1.0)` and `n_with_verdict == 1`. Mutation-resistant: fails if the fold is
  skipped (verdict would stay `None`, rate `None`).
- `ranks_failure_classes_by_count_then_name` — reviews yielding e.g.
  `parse_format×2, prod_unwrap×2, scope_deviation×1`; assert order is
  `[parse_format(2), prod_unwrap(2), scope_deviation(1)]` (tie broken by name asc).
- `excludes_none_from_failure_ranking` — a bucket whose only review is `["none"]`:
  `n_reviews == 1` and `ranked_failure_classes` is empty. (Pinned negative.)
- `attributes_failure_to_matched_run_tags` — a `prod_unwrap` review matched to a
  run tagged `["language=rust","kind=feature"]` lands in **both** tag buckets;
  a second review whose `phase_doc_path` matches **no** run adds nothing.
  (Pinned negative.)
- `attributes_to_latest_run_only` — two runs share a key (`phase_doc_path`),
  different `ts`, different tag sets; the review's class is counted under the
  **latest** run's tags, not the earlier run's. (Pinned negative on the earlier
  run's tag bucket.)
- `multi_class_review_counts_each_class` — a review with
  `["parse_format","prod_unwrap"]` increments both; `n_reviews == 1`.
- `is_model_attributable_separates_spec_and_infra` — `none`/`spec_bug`/`infra_blip`
  → false; `prod_unwrap`/`false_completion` → true.
- `min_runs_filters_small_buckets` — `min_runs = 2` drops a 1-run bucket.

## End-to-end verification

Not applicable — this phase ships **no** runtime-loadable artifact (no CLI, no
MCP tool; those are phase-04). Like phase-01, the hermetic tests over hand-built
records are the verification. State this in the completion Update Log; do not
fabricate a CLI transcript.

## Authorizations

- New module `mcp/src/profile.rs` and a one-line `mod profile;` in
  `mcp/src/main.rs`.
- Reuse `crate::scorecard::ScorecardFilter` (no new filter type).
- No new dependency (`schemars`/`serde` already used in `scorecard.rs`). No edits
  to `telemetry.rs`, `scorecard.rs`, `architecture.md`, `Cargo.toml`,
  `STANDARDS.md`, or `WORKFLOW.md`.
- **`#![allow(dead_code)]` as the first line of `mcp/src/profile.rs`** —
  authorized because `mcp` is a binary crate and `aggregate_profiles` has no
  caller until phase-04 wires the CLI/MCP surface. This attribute MUST be
  removed in phase-04 when the surface lands. It must be the only
  `#[allow]`/`#![allow]` in the file.

## Out of scope

- The `rexymcp profile` CLI subcommand and the MCP tool — **phase-04**. This
  phase adds no `Commands` variant, no dispatch arm, no human-readable formatter.
- Wiring the profile into any read path or the dashboard.
- Per-model config-override resolution and runtime-knob wiring — thread 3
  (phases 05–06).
- Adding `failure_class` to `PhaseRun` (deliberately not done since phase-01).
- Any change to how the executor writes `PhaseRun`/how the CLI writes
  `PhaseReview`.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Notes for executor — 2026-06-14

The first dispatch hard-failed (`IdenticalToolCallRepetition`, `write_file` ×6
with `arguments: null`). You read every reference and produced a **correct** plan
— the failure was purely mechanical: you tried to emit the entire ~360-line
module as a **single** `write_file` call, and the native tool-call arguments blob
serialized as `null` six times. Context was fine (28%). The spec is unchanged and
correct. **The only thing to change is HOW you write the file: build it in small
steps so no single tool-call payload is large.** Do exactly this:

1. **`write_file mcp/src/profile.rs`** with ONLY the production code: the `use`
   imports, the `FailureClassCount` and `ModelProfile` structs,
   `is_model_attributable`, and `aggregate_profiles` (the full algorithm from
   §Spec). End the file with this exact sentinel line as its last line:

   ```
   // <tests appended below>
   ```

2. **`patch mcp/src/main.rs`** to add `mod profile;` next to the other `mod`
   declarations. The `patch` tool is search-and-replace — `{path, old_str,
   new_str}`, `old_str` must appear exactly once. Example shape:

   ```json
   {"path": "mcp/src/main.rs", "old_str": "mod review;", "new_str": "mod profile;\nmod review;"}
   ```

   (Anchor on whatever `mod` line actually exists; read the file first to pick a
   unique `old_str`.)

3. Run **`cargo build`** and confirm the production half compiles before writing
   any tests.

4. **`patch mcp/src/profile.rs`** replacing the sentinel
   `// <tests appended below>` with the `#[cfg(test)] mod tests { use super::*; …
   }` block. **If that whole block is large, do it in two steps:** first replace
   the sentinel with an empty `#[cfg(test)]\nmod tests {\n    use super::*;\n}`
   module, then `patch` in the test functions a few at a time (anchor each new
   `patch` on the module's closing `}` or on the previous test's closing `}` — a
   unique substring). Keep every `write_file`/`patch` payload small.

5. Run the full gate set (`cargo fmt --all --check`, `cargo build`, `cargo clippy
   --all-targets --all-features -- -D warnings`, `cargo test`) as separate
   invocations.

The sentinel comment must be **gone** from the final file (step 4 replaces it) —
do not leave it behind, and do not leave any other placeholder/TODO comment.

### Bounce — 2026-06-14

**Bugs filed:** bug-03-1 (major — false_completion: gates not run before claiming
done), bug-03-2 (major — clippy dead-code wall in binary crate, spec gap).

**Doc amendment:** Authorizations section updated to explicitly authorize
`#![allow(dead_code)]` at the top of `profile.rs` (with mandatory removal in
phase-04). This was impossible-as-written without the amendment; the executor
should have blocked.

**What was built:** `mcp/src/profile.rs` (565 lines) and `mod profile;` in
`main.rs` are present and logically correct — all 8 tests pass. Only the gate
compliance and the dead-code attribute are missing.

**To fix:** (1) Add `#![allow(dead_code)]` as the first line of `profile.rs`
with the removal-note comment (per bug-03-2). (2) Run `rustfmt mcp/src/profile.rs`
to fix the two formatting divergences (per bug-03-1). (3) Re-run all four gates
as separate invocations and confirm each exits 0.

### Update — 2026-06-14 (escalation)

**Chosen lever:** refined re-dispatch
**Rationale:** first failure, spec is sound and fully comprehended; the blocker
was an oversized single `write_file` payload (native args → null ×6), fixed by a
staged-write instruction, not a spec change.

### Update — 2026-06-15 01:01 (complete)

**Executor:** rexyMCP executor (re-dispatch)

**What changed:**
- Added `#![allow(dead_code)]` as the first line of `mcp/src/profile.rs` with the mandatory removal-note comment (per bug-03-2 authorization). This suppresses the dead-code wall in the binary crate since `aggregate_profiles` has no caller until phase-04.
- Ran `cargo fmt --all` to fix two formatting divergences (per bug-03-1).

**Verification commands (all exited 0):**
- `cargo fmt --all --check`: clean
- `cargo build`: clean
- `cargo clippy --all-targets --all-features -- -D warnings`: clean
- `cargo test --package rexymcp -- profile`: 8 passed; 0 failed
- `cargo test` (full suite): 749 passed; 0 failed; 2 ignored

**Files changed:**
- `mcp/src/profile.rs` — added `#![allow(dead_code)]` attribute + removal-note comment at line 1; formatting fixed by `rustfmt`
- `docs/dev/milestones/M18-capability-adaptation/phase-03-model-profile.md` — status flipped to `review`, completion Update Log appended
- `docs/dev/milestones/M18-capability-adaptation/README.md` — phase table row updated to `review`

**End-to-end verification:** N/A — this phase ships no runtime-loadable artifact (no CLI, no MCP tool; those are phase-04). Hermetic tests over hand-built records are the verification, as stated in the spec.

**One-line verification summary:** fmt clean, build clean, clippy clean, 8 profile tests + 749 total tests all pass.

**Notes for review:**
- The `#![allow(dead_code)]` attribute is the only `#[allow]`/`#![allow]` in `profile.rs`, as authorized. It must be removed in phase-04.
- All 8 spec-pinned tests pass: `strengths_come_from_folded_runs`, `ranks_failure_classes_by_count_then_name`, `excludes_none_from_failure_ranking`, `attributes_failure_to_matched_run_tags`, `attributes_to_latest_run_only`, `multi_class_review_counts_each_class`, `is_model_attributable_separates_spec_and_infra`, `min_runs_filters_small_buckets`.

### Review verdict — 2026-06-15

- **Verdict:** approved_after_1
- **Bounces:** 1 (bugs: bug-03-1 `false_completion` + bug-03-2 `spec_bug` — both major). The earlier `IdenticalToolCallRepetition` was a hard_fail handled by refined re-dispatch (oversized `write_file` payload), not a review bounce.
- **Executor:** Qwen/Qwen3.6-27B-FP8
- **Scope deviations:** the fix commit `0967f92` swept pre-existing dirty-tree files unrelated to phase-03 (`docs/dev/NEXT.md`, `phase-07-tooling-improvements.md`) into the same commit. Root cause is dispatching with an unclean tree (Pre-flight step 4), an architect-side miss — not an executor defect. The phase-03 deliverable (`profile.rs`, `main.rs`) is correct.
- **Calibration:** (1) The `spec_bug` half of the bounce — phase-01's "no surface" pattern doesn't transfer to the `mcp` **binary** crate, where unused `pub` items are dead code under `-D warnings`. Lesson: when a phase ships purely-additive code with no caller until a later phase, the architect must check lib-vs-bin and pre-authorize `#![allow(dead_code)]` (with a removal-phase reference) at draft time. This is the 1st occurrence of the lib/bin dead-code distinction — calibration data, no fold yet. (2) Recurring dirty-tree-at-dispatch sweep (seen M13 phase-06, M13 phase-08-era) — commit ambient/architect edits **before** dispatch. Independently re-verified at review: fmt/build/clippy clean, 402 mcp + 749 total tests pass.
