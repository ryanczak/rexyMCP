# Phase 03: Surface silent input degradations as architect-visible warnings

**Milestone:** M26 — Polish & Hardening
**Status:** review
**Depends on:** none
**Estimated diff:** ~140 lines
**Tags:** language=rust, kind=feature, size=m

## Goal

Two ways a phase can run *silently degraded* today, where the executor gets
worse inputs than the architect intended and nothing surfaces it:

1. **Empty or missing `STANDARDS.md`.** Both callers read it with
   `std::fs::read_to_string(&standards_path).unwrap_or_default()`
   (`mcp/src/server.rs:98`, `mcp/src/main.rs:319`), so a missing file becomes an
   empty string and the executor runs with **no Definition of Done** — no error,
   no warning.
2. **Phase-doc heading drift.** `parse_phase_doc` (`mcp/src/runner.rs`) extracts
   the `## Goal` / `## Acceptance criteria` sections by exact heading match; a
   doc that deviates yields an empty goal / empty criteria with no signal, so the
   executor implements against a blank goal.

This phase adds an additive `warnings: Vec<String>` field to `PhaseResult` and
populates it from `runner::run_phase_with` — the single function **both** the MCP
`execute_phase` path and the CLI `run-phase` path route through — so a degraded
run is visible to the architect in the returned structured result instead of
passing silently.

**Deferred to a later phase (do NOT implement here):** the third silent
degradation from the review — an unwritable session-log directory
(`executor/src/agent/mod.rs:190`, `open_session_log(...).ok()`). That warning is
generated deep inside the loop and needs different plumbing (a mutable warnings
channel through `execute_phase`); folding it in here would force the wide
multi-site change this phase is specifically designed to avoid. See § Out of
scope.

## Architecture references

Read before starting:

- `docs/dev/codebase-review-2026-07-07.md` §1 item 6 — the silent-degradation
  finding this phase fixes.
- `docs/dev/WORKFLOW.md` § "Prefer additive change shapes; avoid
  wide-blast-radius breaking changes" — why this phase stamps the result
  post-hoc in one place rather than threading a field through `PhaseInput`'s 11
  constructors.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

### The `PhaseResult` type — `executor/src/phase/result.rs:48-88`

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PhaseResult {
    pub status: PhaseStatus,
    pub files_changed: Vec<FileChange>,
    pub diff: String,
    pub command_outputs: CommandOutputs,
    pub update_log: String,
    pub briefing: Option<Briefing>,
    /// Path to the on-disk JSONL session log Claude can query; `None` when the log
    /// failed to open.
    pub log_path: Option<PathBuf>,
}

impl PhaseResult {
    // ... complete() / hard_fail() / budget_exceeded() all delegate to assemble()

    fn assemble(status: PhaseStatus, briefing: Option<Briefing>, artifacts: Artifacts) -> Self {
        Self {
            status,
            files_changed: artifacts.files_changed,
            diff: artifacts.diff,
            command_outputs: artifacts.command_outputs,
            update_log: artifacts.update_log,
            briefing,
            log_path: artifacts.log_path,
        }
    }
}
```

**Every** terminal return flows through `assemble`, so setting `warnings` to
empty there covers all statuses. Do **not** add `warnings` to `Artifacts` or to
`build_artifacts` — that would be a 14-call-site change. The field is populated
later, in the runner (see the Spec).

The `#[serde(default)]` idiom is already used throughout the codebase for
additive fields — see `executor/src/store/telemetry.rs:134` and neighbors. Use
it plus `skip_serializing_if` so the common (empty) case does not change the
wire format at all.

### The only two field-by-field `PhaseResult` literals — `mcp/src/cap.rs`

Adding a field to `PhaseResult` breaks exactly two struct literals (verified by
grep across `executor/src` + `mcp/src`). Both are in `cap.rs`. The production one
at `cap.rs:55`:

```rust
    PhaseResult {
        status: result.status,
        files_changed: result.files_changed,
        diff,
        command_outputs,
        update_log,
        briefing,
        log_path: result.log_path,
    }
```

and the test helper `base_result()` at `cap.rs:124` (same shape). Both need the
new field added (see Spec task 4). No other struct literal exists — the
`outcome.rs` and `tests.rs` sites the grep shows are `-> PhaseResult {` function
bodies that construct via the `complete`/`hard_fail`/`budget_exceeded`
constructors, not field-by-field, so they are unaffected.

### `run_phase_with` — `mcp/src/runner.rs:189-244`

The choke point. Abridged, showing the load-bearing lines:

```rust
async fn run_phase_with(
    inp: &AssemblyInput<'_>,
    seams: &Seams<'_>,
) -> rexymcp_executor::error::Result<PhaseResult> {
    let phase_doc = std::fs::read_to_string(inp.phase_doc_path)?;
    let fields = parse_phase_doc(&phase_doc);
    // ... scope, cfg, tasks, registry, budget ...
    let input = PhaseInput {
        standards: inp.standards.to_string(),
        phase_doc,
        goal: fields.goal,                        // <-- fields.goal MOVED here
        acceptance_criteria: fields.acceptance_criteria,   // <-- MOVED here
        // ... phase, tags, phase_doc_path, project_id, milestone_id, tier ...
    };
    // ... session_id, deps ...
    agent::execute_phase(&input, deps).await     // <-- current final expression
}
```

**Gotcha:** `fields.goal` and `fields.acceptance_criteria` are *moved* into
`input`. Compute the warnings **before** building `input`, borrowing
`&fields.goal` / `&fields.acceptance_criteria`, or you will hit a
use-after-move. `inp.standards` is a `&str` and is not moved.

### Sibling pure helper for the pattern — `mcp/src/runner.rs`

Phase-02 added `resolve_telemetry_dir` (a small pure helper near
`milestone_id_from_path`, unit-tested in this file's own `#[cfg(test)] mod
tests`). Add the new helper in the same place and the same style:

```rust
/// Resolve the telemetry directory for a CLI-driven `run-phase` invocation:
/// ...
pub fn resolve_telemetry_dir(cfg: &Config, no_telemetry: bool) -> Option<&Path> {
    if no_telemetry {
        None
    } else {
        cfg.telemetry.dir.as_deref()
    }
}
```

### The integration-test pattern — `mcp/src/runner.rs:499-552`

`run_phase_with_assembles_and_returns_result` builds a real repo dir + phase
doc, a `MockAiClient::new(vec!["Done.".to_string()])`, `NoopVerifier`,
`NoopRunner`, an `AssemblyInput`, calls `run_phase_with`, and asserts the
returned `phase_result.status == PhaseStatus::Complete`. Mirror this exact shape
for the end-to-end warnings test (Spec task 6), varying only the `standards`
value and the phase-doc headings, and asserting on `phase_result.warnings`.

## Spec

1. **Add the `warnings` field to `PhaseResult`** — in
   `executor/src/phase/result.rs`, add to the struct (after `log_path`):

   ```rust
       /// Non-fatal warnings about the run's *inputs* — e.g. an empty/missing
       /// STANDARDS.md, or a phase doc whose Goal / Acceptance-criteria sections
       /// did not parse. Empty in the common case; surfaced to the architect so a
       /// silently-degraded run is visible in the structured result.
       #[serde(default, skip_serializing_if = "Vec::is_empty")]
       pub warnings: Vec<String>,
   ```

   and set it in `assemble` (add one line to the `Self { ... }` literal):

   ```rust
           warnings: Vec::new(),
   ```

   Leave `Artifacts`, `build_artifacts`, and the three constructors' signatures
   unchanged — `warnings` starts empty and is filled by the runner.

2. **Add a pure `collect_input_warnings` helper to `mcp/src/runner.rs`** — place
   it next to `resolve_telemetry_dir`. It takes the three input strings and
   returns the warnings (empty when all are non-blank):

   ```rust
   /// Collect non-fatal warnings about a phase run's inputs, for surfacing in
   /// `PhaseResult.warnings`. A blank (whitespace-only or absent) STANDARDS
   /// string or an unparsed Goal / Acceptance-criteria section each means the
   /// executor is running degraded, and today that is silent.
   pub fn collect_input_warnings(
       standards: &str,
       goal: &str,
       acceptance_criteria: &str,
   ) -> Vec<String> {
       let mut warnings = Vec::new();
       if standards.trim().is_empty() {
           warnings.push(
               "STANDARDS.md is empty or missing at <repo>/docs/dev/STANDARDS.md — \
                the executor ran without a Definition of Done. Confirm the file \
                exists and is readable."
                   .to_string(),
           );
       }
       if goal.trim().is_empty() {
           warnings.push(
               "Phase doc has no parseable '## Goal' section — the executor ran \
                without a stated goal. Confirm the heading is exactly '## Goal'."
                   .to_string(),
           );
       }
       if acceptance_criteria.trim().is_empty() {
           warnings.push(
               "Phase doc has no parseable '## Acceptance criteria' section. \
                Confirm the heading is exactly '## Acceptance criteria'."
                   .to_string(),
           );
       }
       warnings
   }
   ```

   The exact wording is yours to refine, but each message MUST name the relevant
   artifact so the test substrings in the Test plan hold.

3. **Wire it into `run_phase_with`** — in `mcp/src/runner.rs`:
   - Immediately after `let fields = parse_phase_doc(&phase_doc);`, compute the
     warnings while `fields` is still borrowable:

     ```rust
     let input_warnings =
         collect_input_warnings(inp.standards, &fields.goal, &fields.acceptance_criteria);
     ```

   - Replace the final expression `agent::execute_phase(&input, deps).await` with:

     ```rust
     let mut result = agent::execute_phase(&input, deps).await?;
     result.warnings.extend(input_warnings);
     Ok(result)
     ```

     Use `extend` (not assignment) so a future phase that adds executor-internal
     warnings inside `execute_phase` is not clobbered. Today `execute_phase`
     returns an empty `warnings`, so `extend` fills it with exactly the input
     warnings.

4. **Fix the two `cap.rs` struct literals** — `cap_phase_result` must preserve
   the field (it round-trips the whole result): in the `PhaseResult { ... }` at
   `cap.rs:55`, add `warnings: result.warnings,`. In the `base_result()` test
   helper at `cap.rs:124`, add `warnings: Vec::new(),`. Warnings are short
   architect-authored strings; do **not** cap them.

5. **Serde tests for the field** — in `executor/src/phase/result.rs`'s
   `#[cfg(test)] mod tests`:
   - a test that a `PhaseResult` with non-empty `warnings` round-trips through
     JSON (serialize → deserialize → equal);
   - a test that an empty `warnings` is **omitted** from the serialized JSON (the
     `skip_serializing_if` — the serialized object must NOT contain a
     `"warnings"` key when the vec is empty);
   - a test that JSON with **no** `warnings` key deserializes to an empty vec
     (the `serde(default)` back-compat — construct the JSON string without the
     key and assert it parses and `warnings.is_empty()`).

6. **Pure-helper + end-to-end tests** — in `mcp/src/runner.rs`'s `mod tests`:
   - pure `collect_input_warnings` tests: all-non-blank → empty vec; empty
     `standards` → exactly one warning containing `"STANDARDS"`; empty `goal` →
     a warning containing `"Goal"`; empty `acceptance_criteria` → a warning
     containing `"Acceptance criteria"`; a **whitespace-only** input (`"   \n"`)
     is treated as blank (boundary pin); a non-blank input produces **no**
     warning for that field (negative pin — e.g. `collect_input_warnings("x",
     "y", "z")` is empty).
   - an end-to-end test mirroring `run_phase_with_assembles_and_returns_result`
     (`runner.rs:499`) but with `standards: ""` in the `AssemblyInput` and a
     phase doc that still has a `## Goal`; assert the returned
     `phase_result.warnings` is non-empty and that one entry contains
     `"STANDARDS"`. This proves the stamping reaches the real returned result,
     not just the pure helper.

## Acceptance criteria

- [ ] `PhaseResult` has a `warnings: Vec<String>` field with `#[serde(default,
      skip_serializing_if = "Vec::is_empty")]`.
- [ ] `cargo build` succeeds (both `cap.rs` literals updated).
- [ ] A `PhaseResult` with empty `warnings` serializes to JSON with **no**
      `"warnings"` key (test in task 5).
- [ ] JSON lacking a `warnings` key deserializes to an empty vec (task 5).
- [ ] `collect_input_warnings("x", "y", "z")` returns an empty vec; each blank
      input adds exactly one artifact-naming warning (task 6 pure tests).
- [ ] The end-to-end test shows `run_phase_with` stamps input warnings onto the
      returned `PhaseResult.warnings` (task 6 integration test).
- [ ] All four gates green.

## Test plan

Unit + integration tests, all hermetic (`TempDir`, `MockAiClient`, no network):

- `collect_input_warnings_empty_when_all_present` in `mcp/src/runner.rs` —
  `collect_input_warnings("s", "g", "a")` is empty.
- `collect_input_warnings_flags_blank_standards` — empty and whitespace-only
  `standards` each yield a warning containing `"STANDARDS"`; the goal/criteria
  warnings are absent when those are non-blank.
- `collect_input_warnings_flags_blank_goal` / `..._blank_criteria` — analogous,
  substrings `"Goal"` / `"Acceptance criteria"`.
- `phase_result_warnings_round_trip` in `executor/src/phase/result.rs` —
  non-empty warnings survive serialize→deserialize.
- `phase_result_empty_warnings_omitted_from_json` — empty vec ⇒ no `"warnings"`
  key in the serialized object.
- `phase_result_missing_warnings_key_defaults_empty` — JSON without the key
  parses to an empty vec.
- `run_phase_with_stamps_input_warnings` in `mcp/src/runner.rs` — mirrors the
  existing completing integration test with `standards: ""`; asserts
  `phase_result.warnings` is non-empty and names STANDARDS.

## End-to-end verification

The real artifact is the `PhaseResult` JSON the executor returns across the MCP
boundary. The `run_phase_with_stamps_input_warnings` integration test drives the
real `run_phase_with` to a completing `PhaseResult` and asserts the warning is
present in the returned value — this is the end-to-end check that the field is
populated on a real run, not just by the pure helper. Paste that test's output
(the relevant `cargo test` lines) into the completion Update Log.

## Authorizations

None from STANDARDS.md §5. This phase authorizes edits to exactly
`executor/src/phase/result.rs`, `mcp/src/runner.rs`, and `mcp/src/cap.rs`.

## Out of scope

- **The unwritable-session-log warning** (`executor/src/agent/mod.rs:190`).
  Deferred — it is generated inside `execute_phase` and needs a mutable warnings
  channel through the loop, which is a materially different (and wider) change.
  Do not touch `agent/mod.rs` or `PhaseInput`.
- **Adding `warnings` to `PhaseInput`, `Artifacts`, or `build_artifacts`.** The
  whole point of stamping in `run_phase_with` is to avoid those multi-site
  changes. Do not thread the field through them.
- **Capping / truncating warning strings** in `cap.rs`. They are short and
  architect-authored.
- **Surfacing warnings in the dispatch skill or the dashboard.** The field is in
  the structured `PhaseResult` (already architect-visible); presenting it in the
  `/rexymcp:dispatch` summary is a separate plugin-doc change, not this phase.
- **Changing `parse_phase_doc` or the `unwrap_or_default()` reads** in
  `server.rs` / `main.rs`. Detecting the empty result downstream (in
  `collect_input_warnings`) covers both the missing-file and empty-file cases in
  one place; do not also rework the reads.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-07-07 22:05 (started)

**Who:** executor (claude-code)

**Notes:** Implementing PhaseResult.warnings field, collect_input_warnings helper, wiring into run_phase_with, cap.rs fixes, and tests.

### Update — 2026-07-07 22:05 (complete)

**Who:** executor (claude-code)

**Summary:** Added `warnings: Vec<String>` to `PhaseResult` with `#[serde(default, skip_serializing_if = "Vec::is_empty")]`, populated in `run_phase_with` via a pure `collect_input_warnings` helper that checks for blank STANDARDS, Goal, and Acceptance criteria. Stamped onto the result with `extend` so future executor-internal warnings aren't clobbered.

**Files changed:**
- `executor/src/phase/result.rs` — added `warnings` field + `assemble` initialization; 3 serde tests
- `mcp/src/runner.rs` — added `collect_input_warnings` helper; wired into `run_phase_with`; 6 pure-helper tests + 1 end-to-end test
- `mcp/src/cap.rs` — added `warnings` to both `PhaseResult` struct literals

**Verification summary:**
- `cargo fmt --all --check`: clean
- `cargo build`: clean, zero warnings
- `cargo clippy --all-targets --all-features -- -D warnings`: clean
- `cargo test`: 874 passed, 0 failed, 2 ignored

**End-to-end verification:** `run_phase_with_stamps_input_warnings` drives the real `run_phase_with` with `standards: ""` and asserts `phase_result.warnings` contains a `"STANDARDS"` warning — confirming the stamping reaches the real returned result. Test output: `test result: ok. 1 passed`.

**Grep proof for pinned literals:**
```
$ grep -rn 'warnings' executor/src/phase/result.rs
59:    /// Non-fatal warnings about the run's *inputs*
64:    pub warnings: Vec<String>,
92:            warnings: Vec::new(),
```

**Notes for review:** None — implementation follows the spec exactly.
