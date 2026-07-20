# Phase 05a-ii: Migrate MCP `model_scorecard` onto the core (retire the Tag wrapper)

**Milestone:** M35 — Metrics & Cost Accounting Overhaul
**Status:** in-progress
**Depends on:** phase-05a-i
**Estimated diff:** ~170 lines (mostly deletions + mechanical test edits)
**Tags:** language=rust, kind=refactor, size=m

## Goal

Point the MCP `model_scorecard` tool at the unified core `aggregate_scorecard`
(from 05a-i) and **retire the now-orphaned Tag wrapper** — delete `aggregate`
and `ScorecardRow`, migrate their test consumers, and change the tool's output
rows to `ScorecardBucket`. After this phase the Tag aggregation has exactly one
implementation (`aggregate_scorecard(_, Tag, _)`) with no wrapper and no dead
code; the only remaining wrapper is `aggregate_by_settings` (retired in 05a-iii
alongside the CLI).

**This re-slices the split slightly vs. the original NEXT.md plan.** NEXT.md put
"delete the wrappers/old structs" entirely in 05a-iii. But the Tag wrapper's
**only** production caller is `model_scorecard_inner`; the moment this phase
migrates it, `aggregate` has zero production callers, and in the `rexymcp`
**binary** crate a `pub fn` used only by tests is `dead_code` → `cargo clippy -D
warnings` fails. Test-only use does not save it (this is the exact mechanism
behind 05a-i's forced `#[allow(dead_code)]`). So "migrate the last consumer" and
"delete the wrapper + migrate its tests" are **one inseparable unit** — doing
them together is what keeps the phase dead-code-free without an `#[allow]`.
Retiring the Settings wrapper stays in 05a-iii (its consumer is the CLI, which
05a-iii migrates).

## Architecture references

Read before starting:

- `docs/dev/milestones/M35-metrics-cost-accounting/README.md` — exit-criterion
  bullet 5 (`scorecard --by …` unifies the CLI and MCP aggregations) and the
  milestone theme "every recorded number is either displayed somewhere or
  deleted" (this phase surfaces the previously-dropped `length_finish_rate_mean`
  on the MCP tool).
- `mcp/src/scorecard.rs` — the 05a-i core `aggregate_scorecard`, the
  `ScorecardBucket` superset row, the `aggregate`/`ScorecardRow` Tag wrapper
  being retired.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.
5. **Test placement rule:** every new/edited `#[test]` fn stays **INSIDE** the
   existing `#[cfg(test)] mod tests` block of `mcp/src/scorecard.rs`
   (`scorecard_tests.rs` is `include!`d into that module) and `server_tests.rs`.
6. **Editing discipline:** edit with `write_file`/`patch`/`patch_lines`. To view
   a file or a line range, use `read_file` (`start_line`/`end_line`), not
   `sed -n`/`cat` — `bash` refuses `sed -i` and repeated identical reads trip the
   governor. On a `patch` "0 matches"/"changed on disk", `read_file` again then
   re-patch.

## Current state

- **The core (keep, make public):** `aggregate_scorecard(runs, dimension,
  filter) -> Vec<ScorecardBucket>` at `mcp/src/scorecard.rs:105`. It is currently
  **private** (`fn`, no `pub`). `ScorecardBucket` (scorecard.rs:54) and
  `ScorecardDimension` (scorecard.rs:44) are already `pub` and already derive
  `Serialize, JsonSchema` (added in 05a-i precisely for this MCP migration).
- **The Tag wrapper (delete):** `pub fn aggregate(runs, filter) ->
  Vec<ScorecardRow>` at scorecard.rs:322 — a thin map over
  `aggregate_scorecard(_, ScorecardDimension::Tag, _)`. Its **only** production
  caller is `model_scorecard_inner` (server.rs:487). `pub struct ScorecardRow`
  (scorecard.rs:270) — identical to `ScorecardBucket` except its key field is
  named `tag` (not `key`) and it has **no** `length_finish_rate_mean`.
- **The MCP tool (migrate):** `ModelScorecardOutput.rows: Vec<scorecard::ScorecardRow>`
  (server.rs:429) and `model_scorecard_inner` (server.rs:455) which calls
  `scorecard::aggregate(&runs, &filter)` (server.rs:487) then truncates to
  `MAX_ROWS`. `ModelScorecardOutput` derives `Serialize, JsonSchema`.
- **Test consumers of the Tag wrapper:** `mcp/src/scorecard_tests.rs` calls
  `aggregate(&runs, …)` at ~24 sites (imported via `use super::{… aggregate …}`
  at scorecard_tests.rs:2) and reads `.tag` on the returned rows at ~14 sites
  (e.g. scorecard_tests.rs:534 `r.tag == "rust"`). `server_tests.rs`'s
  `model_scorecard_*` tests read only `.model` / `.approved_first_try_rate` /
  `.rows.len()` / `.total_runs_considered` — **no `.tag`** — so they compile and
  pass unchanged against `ScorecardBucket`.

**Not touched by this phase:** the CLI `scorecard` subcommand (main.rs:611)
routes through `scorecard_cli::load_settings_scorecard` /
`aggregate_by_settings` — the **Settings** path, retired in 05a-iii. `profile`
and `calibrate` are unrelated.

## Spec

### Task 1 — Publish the core

In `mcp/src/scorecard.rs`, change the core's signature from `fn
aggregate_scorecard(` to `pub fn aggregate_scorecard(` (scorecard.rs:105). No
other change to its body.

### Task 2 — Migrate the MCP tool to `ScorecardBucket` via the core

In `mcp/src/server.rs`:

- Change the output row type (server.rs:429):

  ```rust
  pub struct ModelScorecardOutput {
      pub rows: Vec<scorecard::ScorecardBucket>,   // was Vec<scorecard::ScorecardRow>
      pub total_runs_considered: usize,
      pub truncated: bool,
  }
  ```

- Change the aggregation call in `model_scorecard_inner` (server.rs:487) from

  ```rust
  let mut rows = scorecard::aggregate(&runs, &filter);
  ```

  to

  ```rust
  let mut rows =
      scorecard::aggregate_scorecard(&runs, scorecard::ScorecardDimension::Tag, &filter);
  ```

  Everything else in `model_scorecard_inner` (the `MAX_ROWS` truncation, the
  `truncated` flag, `total_runs_considered`) is **unchanged** — `Vec::len` /
  `Vec::truncate` work identically on `ScorecardBucket`.

The tool keeps its **model × tag** behavior (same buckets, same rows, same sort
order — `aggregate_scorecard(_, Tag, _)` is exactly what `aggregate` delegated
to). Two output-JSON changes, both **intended** (pin them in Task 4):

1. The per-row key field is now serialized as `"key"` (was `"tag"`).
2. Each row now carries `"length_finish_rate_mean"` — the length-truncation rate
   the old `ScorecardRow` computed-then-dropped (milestone theme: no dropped
   numbers). It is `Some(_)` for buckets whose runs carry `length_finish_rate`,
   `null` otherwise.

Do **not** add a dimension/`by` parameter to `model_scorecard` — it stays
Tag-only. (The `--by` selector is a CLI surface in 05a-iii; whether the MCP tool
later gains one is a separate future decision, out of scope here.)

### Task 3 — Delete the Tag wrapper + struct; migrate the tests

Order matters only in that the file must **compile before the final gates** — a
`dead_code` warning is build-tolerable (it is a warning, not a check error), so
the brief window after Task 2 where `aggregate` is uncalled is fine; the final
`cargo clippy -D warnings` is what enforces the deletion. Recommended order:

1. **`scorecard_tests.rs` — swap the import for a local shim.** In the top
   `use super::{ … };` (scorecard_tests.rs:1–4), **remove `aggregate`** from the
   import list and **add `ScorecardBucket`**. Then add a small private helper
   inside the test module so the ~24 existing `aggregate(&runs, …)` call sites
   need **no** change:

   ```rust
   /// Test convenience: the model×tag aggregation is now `aggregate_scorecard`
   /// with the `Tag` dimension (the old `aggregate` wrapper was retired in
   /// phase-05a-ii). Returns `ScorecardBucket` (key field is `key`, not `tag`).
   fn aggregate(runs: &[PhaseRun], filter: &ScorecardFilter) -> Vec<ScorecardBucket> {
       aggregate_scorecard(runs, ScorecardDimension::Tag, filter)
   }
   ```

   This is a **test-only** helper (private, inside `#[cfg(test)]`), so it is not
   production dead code. It keeps the 24 call sites byte-identical.

2. **`scorecard_tests.rs` — `.tag` → `.key`.** The returned rows are now
   `ScorecardBucket`, whose key field is `key`. Change every `.tag` **read on an
   aggregation-result row** to `.key` (~14 sites, e.g. `r.tag == "rust"` →
   `r.key == "rust"`). **Do NOT** touch `run.tags`, `filter.tags`,
   `make_run(…, &["rust"], …)` tag *arguments*, or the settings-path tests — only
   the `.tag` **field access on a row returned by `aggregate(...)`**.

3. **`scorecard.rs` — delete the wrapper and struct.** Delete `pub fn aggregate`
   (scorecard.rs:322 through its closing `}`) and `pub struct ScorecardRow`
   (the `/// One row of the model × tag matrix.` doc comment at scorecard.rs:268
   through the struct's closing `}` at :301). Leave `gates_all_pass`,
   `ScorecardFilter`, `MAX_ROWS`, `aggregate_by_settings`, `SettingsScorecardRow`,
   and `aggregate_scorecard` in place.

4. **`scorecard.rs` — fix the stale doc reference.** `ScorecardBucket`'s doc
   comment (scorecard.rs:49–52) says it is "the dimension-neutral superset of
   `ScorecardRow` and `SettingsScorecardRow`." `ScorecardRow` no longer exists —
   reword to drop the dead reference, e.g. "the dimension-neutral superset row
   returned by [`aggregate_scorecard`] (and mapped by the `aggregate_by_settings`
   wrapper into `SettingsScorecardRow`)." Do not leave a rustdoc intra-doc link
   to a deleted item (it breaks `cargo doc`/clippy `broken_intra_doc_links`).

### Task 4 — Pin the migrated MCP output

In `mcp/src/server_tests.rs`, inside its `#[cfg(test)] mod tests`, add one test
that pins the migration (reuse the existing `make_config_with_telemetry` +
`write_telemetry_fixture` helpers the other `model_scorecard_*` tests use):

- `model_scorecard_rows_are_buckets_keyed_by_tag` — run `model_scorecard_inner`
  over the standard fixture and assert the returned rows are `ScorecardBucket`s
  keyed by tag: at least one row has a non-empty `key` equal to a tag present in
  the fixture (e.g. assert a row exists with `key == "rust"` if the fixture's m1
  run is tagged `rust`; otherwise assert `rows.iter().all(|r| !r.key.is_empty())`
  since every Tag bucket has a non-empty key). This pins that the tool now
  returns the unified bucket type keyed by `key` (a mutation swapping the
  dimension to `Model` — empty keys — would fail it).

  Confirm against the fixture what tag m1 actually carries (read
  `write_telemetry_fixture` in `server_tests.rs`) and assert the real tag value;
  do not invent one.

(The existing `model_scorecard_*` tests already exercise `.model`,
`.approved_first_try_rate`, truncation, folding, and the empty/malformed cases —
they continue to pass unchanged and are the regression net for behavior
preservation.)

## Acceptance criteria

- [ ] `model_scorecard_inner` calls `aggregate_scorecard(_, ScorecardDimension::Tag,
      _)`; `ModelScorecardOutput.rows` is `Vec<ScorecardBucket>`.
- [ ] `pub fn aggregate` and `pub struct ScorecardRow` are **deleted**;
      `grep -nE "fn aggregate\b|struct ScorecardRow" mcp/src/scorecard.rs`
      returns nothing (only `aggregate_by_settings` / `aggregate_scorecard` /
      `SettingsScorecardRow` remain).
- [ ] **No `#[allow(dead_code)]`** anywhere in `mcp/src/scorecard.rs` or
      `mcp/src/server.rs` (the deletion, not an allow, is what removes the dead
      code). `grep -rn "allow(dead_code)" mcp/src/scorecard.rs mcp/src/server.rs`
      returns nothing.
- [ ] Every pre-existing `model_scorecard_*` test in `server_tests.rs` passes
      **unchanged**; the new `model_scorecard_rows_are_buckets_keyed_by_tag`
      passes.
- [ ] The scorecard math tests in `scorecard_tests.rs` pass via the test-local
      `aggregate` shim (behavior identical — the shim delegates to the same core
      the old wrapper did).
- [ ] `cargo fmt --all --check`, `cargo build`,
      `cargo clippy --all-targets --all-features -- -D warnings`, and
      `cargo test` all pass.

## Test plan

- **Existing (unchanged):** all `model_scorecard_*` tests in `server_tests.rs`
  (success via config/override, precedence, disabled-error, missing-file-empty,
  malformed-survivors, truncation, `folds_review`) — these are the behavior-
  preservation net; they must stay green with **no edits**.
- **Existing (via shim):** the ~24 `aggregate(...)`-based tests in
  `scorecard_tests.rs` — untouched call sites; only `.tag`→`.key` reads change.
- **New:** `model_scorecard_rows_are_buckets_keyed_by_tag` (Task 4) — pins the
  migrated output is `ScorecardBucket` keyed by `key`.

## End-to-end verification

Live MCP-tool check against the real telemetry store (read-only), to confirm the
output schema change is real and well-formed — the tool now emits `key` +
`length_finish_rate_mean` instead of `tag`:

```
cargo run -p rexymcp -- scorecard --config rexymcp.toml --format json
```

Wait — the CLI `scorecard` subcommand is the **Settings** path (unchanged here),
so it does not exercise this tool. Instead verify the migrated tool via its unit
surface: `cargo test -p rexymcp model_scorecard` (all `model_scorecard_*` green,
including the new bucket-key pin). The MCP `model_scorecard` tool has no CLI
entry point of its own (it is an `#[rmcp::tool]` reached over stdio); the unit
tests over `model_scorecard_inner` **are** its behavioral surface. State this in
the completion Update Log (do not claim a CLI E2E that does not exist for this
tool).

## Authorizations

None. No new dependencies (`serde`/`schemars` already used). No
`docs/architecture.md` edit — the § "Model effectiveness metrics" prose
describes the model×tag scorecard conceptually and is unaffected by the internal
row-type rename. No `STANDARDS.md`/`WORKFLOW.md` edit.

## Out of scope

- **A `by`/dimension parameter on the MCP `model_scorecard` tool** — it stays
  Tag-only. The `--by model|tag|settings` selector is a CLI feature (05a-iii).
- **Retiring `aggregate_by_settings` / `SettingsScorecardRow`** — their consumer
  is the CLI, migrated in 05a-iii.
- **The `scorecard` CLI subcommand, `scorecard_cli.rs`, dropped columns
  (wall-clock/verifier-retries/repairs)** — all 05a-iii.
- **`profile` tokens/cost** — phase-05b.
- **`docs/architecture.md`, `STANDARDS.md`, `WORKFLOW.md`, other phase docs.**

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
