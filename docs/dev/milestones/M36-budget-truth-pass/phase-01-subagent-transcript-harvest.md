# Phase 01: Harvest subagent transcripts

**Milestone:** M36 — Budget Truth Pass
**Status:** done
**Depends on:** none
**Estimated diff:** ~160 lines
**Tags:** language=rust, kind=bugfix, size=m

## Goal

Claude Code writes `Agent`-tool subagent transcripts one directory below the
session transcripts, at `<transcript_dir>/<session-id>/subagents/*.jsonl`. Both
the harvester and the background sweep scan only the top level, so **all
subagent token usage is silently uncounted**. Measured on this project: 36
files, 1,133 assistant messages, 59.6 M tokens — ~10 % of spend in
`/rexymcp:auto` sessions, where dispatch and review are delegated to subagents
by design. This phase makes both readers see those files.

## Architecture references

Read before starting:

- `docs/dev/milestones/M36-budget-truth-pass/README.md` — why this milestone
  exists; note that no double-counting exists and this phase must not introduce
  any.
- `docs/architecture.md` § Status #35 — the architect-ledger design this phase
  fixes a gap in.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

**`mcp/src/harvest.rs:199-231`** — file discovery is a flat, non-recursive
`read_dir`, and the session id comes from the file stem:

```rust
    // Read and parse transcript files, sorted by path for deterministic dedup order
    let mut files: Vec<PathBuf> = Vec::new();
    for entry in std::fs::read_dir(args.transcript_dir)
        .map_err(|e| format!("failed to read transcript dir: {}", e))?
    {
        let entry = entry.map_err(|e| format!("failed to read dir entry: {}", e))?;
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "jsonl") {
            files.push(path);
        }
    }
    files.sort();
    ...
    for file_path in &files {
        let session_id = match file_path
            .file_stem()
            .and_then(|s| s.to_str())
            .map(str::to_string)
        { Some(s) => s, None => continue };
```

**`mcp/src/sweep.rs:49-67`** — `max_transcript_mtime_ms` has the same flat scan.
This is the skip-guard watermark: during an `/rexymcp:auto` run where all work
is delegated to subagents, **only** subagent files change, the watermark never
moves, and `should_harvest` returns `false` — so the sweep skips the harvest
entirely. Fixing `harvest()` alone would leave the live `serve` path broken.

**The real on-disk layout** (verified on this machine, under
`~/.claude/projects/-home-matt-src-rexyMCP/`):

```
<transcript_dir>/
├── 453edc21-88ac-4862-b078-d29b49e99a90.jsonl      <- session transcript (read today)
├── 453edc21-88ac-4862-b078-d29b49e99a90/
│   └── subagents/
│       ├── agent-afc38906ca6fc7509.jsonl           <- NOT read today
│       └── agent-af48da68b61ad9f5d.jsonl
└── 11bfab74-b6a5-494f-b9f6-08a7cc1c0295/
    └── tool-results/                                <- MUST STAY unread
```

Subagent lines are shaped exactly like session lines: `type: "assistant"`, a
`message.id`, `message.usage`, `message.model`, and an `attributionSkill` when
one applies (observed values across the corpus: `rexymcp:auto` ×354, absent
×637, `rexymcp:architect` ×108, `rexymcp:dispatch` ×34). They run on a mix of
models (`claude-sonnet-5`, `claude-opus-4-8`, `claude-haiku-4-5-20251001`) —
all three already priced by `known_model_rates` at `executor/src/config.rs:14-19`,
so no pricing work is needed here.

Deduplication already happens globally by `message.id` (`harvest.rs:212`
`seen_ids`), so a message that somehow appears at both levels is counted once.
Do not add a second dedup mechanism.

## Spec

### 1. Add a `collect_transcripts` helper in `mcp/src/harvest.rs`

Add a private function that returns every transcript file **paired with the
session id it belongs to**, replacing the inline discovery loop. Subagent files
attribute to their **parent session directory name**, not their own file stem —
that is what folds a subagent's usage into the session that spawned it.

Pin exactly one nested path, `<session>/subagents/*.jsonl`. Do **not** write a
general recursive walk: `<session>/tool-results/` also contains files and must
never be harvested.

Reference shape (adapt as needed; the behavior is what is pinned):

```rust
/// Every transcript file under `dir`, paired with the session id it belongs to.
///
/// Two layouts are read:
/// - `<dir>/<session-id>.jsonl`                     — session id from the file stem
/// - `<dir>/<session-id>/subagents/*.jsonl`         — session id from the parent dir
///
/// Subagent usage folds into its spawning session's bucket. No other
/// subdirectory is scanned (notably `<session-id>/tool-results/`).
/// Sorted by path for deterministic dedup order.
fn collect_transcripts(dir: &Path) -> Result<Vec<(PathBuf, String)>, String> {
    let mut out: Vec<(PathBuf, String)> = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(|e| format!("failed to read transcript dir: {e}"))?
    {
        let entry = entry.map_err(|e| format!("failed to read dir entry: {e}"))?;
        let path = entry.path();

        if path.is_dir() {
            let Some(session_id) = path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            let subagents = path.join("subagents");
            let Ok(inner) = std::fs::read_dir(&subagents) else {
                continue; // no subagents dir for this session — normal
            };
            for sub in inner.filter_map(|e| e.ok()) {
                let p = sub.path();
                if p.extension().is_some_and(|ext| ext == "jsonl") {
                    out.push((p, session_id.to_string()));
                }
            }
        } else if path.extension().is_some_and(|ext| ext == "jsonl")
            && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
        {
            out.push((path.clone(), stem.to_string()));
        }
    }
    out.sort();
    Ok(out)
}
```

### 2. Rewire `harvest()` onto the helper

In `mcp/src/harvest.rs`, replace the discovery loop (currently lines 199-210)
with a call to `collect_transcripts`, and replace the per-file `session_id`
derivation (currently lines 218-226) with the id carried in the pair. Everything
downstream — `seen_ids` dedup, the `(session, model, skill)` accumulator, the
`sessions` set, record emission — is unchanged.

`HarvestOutcome.sessions` counts distinct **session** ids, so a session and its
subagents now correctly count as one session, not several.

### 3. Extend the sweep watermark in `mcp/src/sweep.rs`

`max_transcript_mtime_ms` must consider subagent files too, or the skip-guard
will suppress harvests during subagent-only activity. Reuse the same
`<session>/subagents/*.jsonl` rule — do not scan other subdirectories. Keep the
function's contract otherwise identical: stats only, `None` when the dir is
unreadable or has no matching files, no propagated errors.

### 4. Update the module doc comment

`mcp/src/harvest.rs:1-7` describes the reader. Extend it to state that both the
session transcript and its `subagents/` transcripts are read, and that subagent
usage attributes to the parent session.

### 5. Tests

Write the tests named in § Test plan below. Every test builds its own
`tempfile::TempDir` transcript tree — no reads of `~/.claude`.

## Acceptance criteria

- [x] `cargo test -p rexymcp harvest` passes, including all six new tests.
- [x] `cargo clippy --all-targets --all-features -- -D warnings` is clean.
- [x] `cargo fmt --all --check` reports no diff in the files this phase touched.
- [x] A transcript tree containing `s1.jsonl` **and**
      `s1/subagents/agent-a.jsonl` harvests messages from both, and the ledger
      records for the subagent's usage carry `"session":"s1"`.
- [x] A file at `s1/tool-results/x.jsonl` containing a well-formed
      assistant-usage line is **not** harvested.

## Test plan

In `mcp/src/harvest.rs`'s `#[cfg(test)] mod tests`:

- `harvests_subagent_transcripts_under_session_dir` — tree with `s1.jsonl`
  (1 usage message) and `s1/subagents/agent-a.jsonl` (1 usage message with a
  distinct `message.id`); asserts `HarvestOutcome.messages == 2`.
- `subagent_usage_attributes_to_parent_session` — same tree; asserts the
  emitted ledger record for the subagent's message has session `s1` and **not**
  `agent-a`. Assert on the session field value directly, not via a substring of
  the whole file.
- `subagent_transcripts_do_not_inflate_session_count` — same tree; asserts
  `HarvestOutcome.sessions == 1`.
- `ignores_non_subagent_session_subdirs` — tree with `s1.jsonl` (1 message) and
  `s1/tool-results/x.jsonl` holding a **valid** assistant-usage line with a
  distinct `message.id`; asserts `messages == 1` and that the `tool-results`
  message's id contributes no tokens. This is the negative case — it must fail
  if the implementation becomes a general recursive walk.
- `subagent_dedup_by_message_id_across_levels` — the same `message.id` present
  in both `s1.jsonl` and `s1/subagents/agent-a.jsonl`; asserts `messages == 1`
  and `duplicates == 1`.
- `reharvest_with_subagents_is_idempotent` — harvest twice over the same tree;
  asserts the folded ledger (via the existing read/fold path used by the current
  idempotency test) yields the same totals both times.

`sweep.rs` already has a `#[cfg(test)] mod tests`; add:

- `max_transcript_mtime_includes_subagent_files` — a tree containing only
  `s1/subagents/agent-a.jsonl` and **no** top-level `*.jsonl`; asserts the
  function returns `Some(_)` where today it returns `None`. This isolates the
  new behavior without depending on mtime ordering — do **not** `sleep`, and do
  **not** reach for an mtime-setting crate (`filetime` is not a dependency;
  adding one is a blocker, not a `cargo add`).
- `max_transcript_mtime_none_for_tool_results_only` — a tree containing only
  `s1/tool-results/x.jsonl`; asserts `None`. (Negative case.)

## End-to-end verification

Build a real transcript tree in a temp directory and run the real binary — do
not point it at `~/.claude/projects`, which would mutate the user's live
telemetry.

```bash
TMP=$(mktemp -d)
mkdir -p "$TMP/tx/s1/subagents" "$TMP/tx/s1/tool-results" "$TMP/tel"
printf '%s\n' '{"type":"assistant","timestamp":"2026-07-23T10:00:00.000Z","message":{"id":"msg_top","role":"assistant","model":"claude-opus-4-8","usage":{"input_tokens":100,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"output_tokens":10}}}' > "$TMP/tx/s1.jsonl"
printf '%s\n' '{"type":"assistant","timestamp":"2026-07-23T10:01:00.000Z","attributionSkill":"rexymcp:dispatch","message":{"id":"msg_sub","role":"assistant","model":"claude-sonnet-5","usage":{"input_tokens":200,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"output_tokens":20}}}' > "$TMP/tx/s1/subagents/agent-a.jsonl"
printf '%s\n' '{"type":"assistant","timestamp":"2026-07-23T10:02:00.000Z","message":{"id":"msg_tool","role":"assistant","model":"claude-opus-4-8","usage":{"input_tokens":999,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"output_tokens":99}}}' > "$TMP/tx/s1/tool-results/x.jsonl"

cargo run -p rexymcp -- harvest --config rexymcp.toml \
  --transcript-dir "$TMP/tx" --project-id e2e \
  --telemetry-path "$TMP/tel/phase_runs.jsonl"

grep -c msg_tool "$TMP/tel/phase_runs.jsonl" || echo "tool-results excluded (expected)"
cat "$TMP/tel/phase_runs.jsonl"
```

Paste the actual output in the completion Update Log. Expected: the harvest line
reports **2 messages across 1 session**, the emitted records both carry
`"session":"s1"`, and `msg_tool`'s 999 input tokens appear nowhere.

## Authorizations

None. (No new dependencies — `tempfile` is already a dev-dependency; if the
mtime test needs a crate that is not already present, that is a **blocker**, not
a `cargo add`.)

## Out of scope

- Any change to how `rexymcp:auto` is attributed. It is verified correct; the
  buckets are disjoint and no double-counting exists.
- The `other` → `architect chat` rename (phase 03).
- Any Budget-panel or `costs` output change (phase 02).
- Scoping the architect ledger to milestone or session. It is a known
  limitation, not this phase's problem.
- A general recursive transcript walk. Exactly one nested path is in scope.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-07-23 21:00 (started)

Executor implementing phase 01: adding `collect_transcripts` helper, rewiring `harvest()`, extending sweep watermark, updating module doc, and writing tests.
### Update — ts=1784841093906 (complete, server-authored)

**Summary:** **Summary + Notes for review:**

Built the `collect_transcripts` helper in `mcp/src/harvest.rs` that discovers both top-level `<session-id>.jsonl` and `<session-id>/subagents/*.jsonl` files, pairing each with the correct session id (parent directory for subagents). Rewired `harvest()` onto this helper so subagent token usage attributes to the spawning session. Extended `max_transcript_mtime_ms` in `sweep.rs` to scan subagent directories so the sweep skip-guard no longer suppresses harvests during subagent-only activity. Only `subagents/` is scanned — `tool-results/` is explicitly excluded. Updated the module doc comment accordingly.

All 8 new tests pass (6 in `harvest.rs`, 2 in `sweep.rs`), plus all 1032 existing tests remain green. E2E verification with a real temp tree confirmed: **2 messages across 1 session**, both ledger records carry `"session_id":"s1"`, and `msg_tool`'s 999 tokens from `tool-results/` appear nowhere in the output. No deviations from the spec.

**Gates:** format=run, build=run, lint=run, test=run

**Command output tails:**

```
FORMAT


BUILD
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.08s


LINT
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.07s


TEST
s::tests::type_mismatch_returns_recovery_hint ... ok
test tools::update_task::tests::flips_pending_task_to_active ... ok
test tools::update_task::tests::flips_active_task_to_done ... ok
test tools::update_task::tests::invalid_state_returns_advisory_error ... ok
test tools::update_task::tests::invalid_args_hint_reports_all_complete ... ok
test tools::update_task::tests::malformed_args_returns_advisory_error ... ok
test tools::update_task::tests::invalid_args_hint_lists_incomplete_ids ... ok
test tools::update_task::tests::null_args_returns_recovery_hint ... ok
test tools::update_task::tests::metadata_shape_is_unchanged ... ok
test tools::update_task::tests::result_lists_remaining_incomplete_ids ... ok
test tools::update_task::tests::result_flags_redundant_remark ... ok
test tools::update_task::tests::unknown_id_returns_advisory_error ... ok
test tools::update_task::tests::result_reports_all_complete_when_last_done ... ok
test tools::update_task::tests::success_output_names_task ... ok
test tools::write_file::tests::append_creates_file_if_missing ... ok
test tools::write_file::tests::appends_to_existing_file ... ok
test tools::write_file::tests::missing_path_returns_recovery_hint ... ok
test tools::write_file::tests::append_false_overwrites ... ok
test tools::write_file::tests::creates_new_file ... ok
test tools::write_file::tests::non_object_args_do_not_panic ... ok
test tools::write_file::tests::reports_missing_parent_dir ... ok
test tools::write_file::tests::overwrites_existing_file ... ok
test tools::write_file::tests::rejects_malformed_args ... ok
test tools::write_file::tests::scope_escape_returns_advisory_error_and_writes_nothing ... ok
test tools::write_file::tests::success_output_includes_line_count ... ok
test tools::symbols::tests::defaults_to_scope_root_when_no_path_given ... ok
test tools::symbols::tests::references_snippet_shows_source_line ... ok
test tools::symbols::tests::references_across_multiple_files ... ok
test tools::symbols::tests::metadata_carries_definitions_and_files_count ... ok
test ai::backends::openai::tests::is_retriable_transport_true_for_reqwest_error ... ok
test tools::symbols::tests::references_truncation_note_omits_kind_filter ... ok
test tools::bash::tests::cargo_command_records_cargo_filter_label ... ok
test tools::symbols::tests::reports_line_and_column ... ok
test tools::symbols::tests::unsupported_extension_skipped_in_dir_walk ... ok
test tools::symbols::tests::respects_gitignore ... ok
test tools::symbols::tests::finds_rust_struct_and_trait ... ok
test governor::verifier::tests::verify_rust_returns_checked_empty_on_clean_code ... ok
test governor::verifier::tests::capture_baseline_dedupes_by_project_root ... ok
test governor::verifier::tests::verify_rust_returns_checked_with_errors_on_broken_code ... ok
test governor::verifier::tests::capture_baseline_skips_unsupported_files ... ok
test tools::bash::tests::cargo_command_output_is_filtered_through_cargo_filter ... ok
test ai::backends::openai::tests::midstream_stall_is_not_retried ... ok
test ai::backends::openai::tests::first_token_stall_retries_then_succeeds ... ok
test ai::tests::stream_next_uses_supplied_timeout ... ok
test tools::bash::tests::default_timeout_used_when_arg_absent ... ok
test tools::bash::tests::arg_timeout_overrides_constructor_default ... ok
test tools::bash::tests::times_out_advisory_failure ... ok
test ai::backends::openai::tests::first_token_stall_exhausts_retries_then_errors ... ok
test health::tests::check_returns_unreachable_on_connection_error ... ok

test result: ok. 1032 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 6.09s


running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.07s
     Running unittests src/main.rs (target/debug/deps/rexymcp-4e85b51f198fbe9f)
     Running unittests src/lib.rs (target/debug/deps/executor-c1650299697d7408)
   Doc-tests executor

```

**Files changed:**
- `docs/dev/milestones/M36-budget-truth-pass/README.md` — +1 -1
- `docs/dev/milestones/M36-budget-truth-pass/phase-01-subagent-transcript-harvest.md` — +5 -1
- `mcp/src/harvest.rs` — +310 -27
- `mcp/src/sweep.rs` — +60 -1

**Commit:** 3cb480691bdd85a56bdc181f803a16eff305e25d

**Notes:** server-authored completion entry (executor no longer owns the bookkeeping tail; see M27 phase-03).


### Review verdict — 2026-07-23

- **Verdict:** approved_first_try
- **Bounces:** none
- **Executor:** Qwen/Qwen3.6-27B-FP8 (local; 58 turns, no oscillation)
- **Scope deviations:** none
- **Calibration:** server-authored completion entries omit two DoD elements —
  the phase doc's acceptance-criteria checkboxes are left unticked, and no
  "End-to-end verification" block with quoted output is written (STANDARDS §1
  and the WORKFLOW completion template both require them). **Systemic since
  M27 phase-03 moved the bookkeeping tail server-side** — reproduced on M35
  phase-06e, 07g, and 07h, so not an executor defect and not fixable by
  re-dispatch. The reviewer verified both independently. Candidate for an M37
  phase.

**Reviewer verification (independent re-run):**

All four gates re-run separately, green. Build forced to recompile the touched
crate (`touch mcp/src/harvest.rs mcp/src/sweep.rs`) — the cached 0.08s "Finished"
in the executor's log would have masked warnings; zero warnings on the real
rebuild. Tests: 622 + 1032 pass, 0 fail.

All 8 new tests confirmed present and passing by name (6 `harvest::tests::*`,
2 `sweep::tests::max_transcript_*`). Both are **mutation-sensitive in two
independent dimensions**:

- Pointing the scan at `tool-results` instead of `subagents` → 4 tests fail,
  including the negative case `ignores_non_subagent_session_subdirs`.
- Attributing subagent files to their own file stem instead of the parent
  session dir → `subagent_usage_attributes_to_parent_session` and
  `subagent_transcripts_do_not_inflate_session_count` fail.

E2E run against the real binary and a real temp tree:

```
harvested 2 messages across 1 sessions -> 2 ledger records (0 duplicates skipped)

{"session_id":"s1","model":"claude-opus-4-8","skill":"other",
 "tokens":{"input":100,"output":10,...}}
{"session_id":"s1","model":"claude-sonnet-5","skill":"rexymcp:dispatch",
 "tokens":{"input":200,"output":20,...}}

msg_tool (999 input tokens, under tool-results/): 0 occurrences — correctly excluded
```

The subagent's record carries `session_id: "s1"` (parent session, not
`agent-a`) and its own `rexymcp:dispatch` skill tag — both halves of the
attribution contract.

**Real-corpus effect** (harvested to a temp store; live telemetry untouched):

| skill | before | after | delta |
|---|---:|---:|---:|
| `rexymcp:dispatch` | 949.6M | 951.6M | +2.0M |
| `other` | 359.1M | 374.3M | +15.2M |
| `rexymcp:review` | 339.6M | 344.3M | +4.7M |
| `rexymcp:architect` | 220.4M | 222.0M | +1.6M |
| `rexymcp:escalate` | 134.2M | 134.2M | — |
| `rexymcp:auto` | 25.9M | 38.1M | **+12.2M (+47 %)** |
| **total** | 2028.8M | 2064.9M | **+36.1M** |

The recovered total is +36.1M rather than the +59.6M raw measurement in the
phase doc, because the harvester dedups globally by `message.id` (6,069
duplicates skipped this run) — subagent transcripts re-record messages that
also appear elsewhere. The dedup is working as designed; the raw figure was
pre-dedup. `rexymcp:auto` gaining 47 % is the predicted concentration: auto
delegates dispatch and review to subagents, which is exactly where the blind
spot was.

(A distinct `review` bucket, 0.4M, appears post-fix. Traced to the built-in
`/review` skill invoked in the reviewing session itself — top-level, not
subagent. Real data, correctly bucketed, unrelated to this phase.)
