# Phase 01: Harvest subagent transcripts

**Milestone:** M36 — Budget Truth Pass
**Status:** in-progress
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

- [ ] `cargo test -p rexymcp harvest` passes, including all six new tests.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` is clean.
- [ ] `cargo fmt --all --check` reports no diff in the files this phase touched.
- [ ] A transcript tree containing `s1.jsonl` **and**
      `s1/subagents/agent-a.jsonl` harvests messages from both, and the ledger
      records for the subagent's usage carry `"session":"s1"`.
- [ ] A file at `s1/tool-results/x.jsonl` containing a well-formed
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
