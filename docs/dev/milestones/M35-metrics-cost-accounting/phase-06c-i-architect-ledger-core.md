# Phase 06c-i: Architect ledger core — transcript-native harvest rewrite

**Milestone:** M35 — Metrics & Cost Accounting Overhaul
**Status:** in-progress
**Depends on:** phase-06b-ii
**Estimated diff:** ~320 lines
**Tags:** language=rust, kind=feature, size=m

## Goal

Replace the architect side of cost accounting with a **transcript-native
ledger**. Today `rexymcp harvest` (`mcp/src/harvest.rs`) reads Claude Code
transcripts, then *attributes* each message's tokens to the nearest
`ArchitectActivity` journal time-window — a scheme that **drops** every message
after the last journal boundary, **discards `message.model`** (the whole
multi-model corpus is later priced at one rate), and **destroys per-skill and
per-session detail** at ingest.

This phase introduces a new telemetry record, **`ArchitectLedger`**, keyed
`(project_id × session_id × model × skill)`, and **rewrites `harvest` to emit
it**: dedup by `message.id`, count **all** project usage (messages with no
`attributionSkill` go to an `other` bucket, never dropped), carry the real
`message.model`, and split `cache_creation` into its 5-minute / 1-hour buckets
(the ingredient 06c-ii prices). The old time-window attribution machinery is
deleted.

**This is 06c-i — the ledger core only.** It is (nearly) additive: the new
record + reader/fold/append mirror the existing `ArchitectActivity` trio, and
the `harvest` rewrite swaps the internal algorithm. **Pricing** (a built-in
Claude price table + config override over the 5m/1h split) is **06c-ii**.
**Rewiring the surfaces** (`costs`, dashboard Budget, `profile`) onto the ledger
and retiring `ArchitectActivity.tokens` is **06c-iii**. Do **not** touch
`mcp/src/costs.rs` or any surface here (see Out of scope).

## Architecture references

Read before starting:

- `docs/dev/milestones/M35-metrics-cost-accounting/README.md` — the exit
  criterion "Architect usage is a **transcript-native ledger**" bullet and the
  "Architect-ledger design pass" note (the four resolved forks).
- `executor/src/store/telemetry.rs` — the telemetry store: `ArchitectActivity`
  + `ArchitectTokens` (lines ~428–495), the `append_architect_activity` /
  `read_architect_activities` / `fold_activities` trio (lines ~520–579), and
  `TELEMETRY_SCHEMA_VERSION` (line 190). **These three functions are the exact
  worked example for the ledger trio you will add** — quoted inline below.
- `mcp/src/harvest.rs` — the current harvester (the file you rewrite). Keep its
  timestamp parser + file-reading scaffold; replace its attribution core.
- `mcp/src/main.rs` — the `Harvest` clap arm (~line 937) whose output line you
  update. The clap **arguments** do not change.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.
5. **Test placement rule (the wall that hard-failed M35 phase-01 twice):** new
   unit tests go **inside the existing `#[cfg(test)] mod tests { … }` block at
   the bottom of the same file** — `telemetry.rs` tests in `telemetry.rs`'s
   block, `harvest.rs` tests in `harvest.rs`'s block. Never add a `#[test]` fn
   at file scope beside `mod tests`, and never relocate tests across files.
6. **Editing discipline:** edit with `write_file`/`patch`/`patch_lines`. To view
   a file/line range use `read_file` (`start_line`/`end_line`), **not**
   `sed -n`/`cat` — `bash` refuses `sed -i`/`perl -i` outright, and repeated
   identical reads trip the governor. On a `patch` "0 matches" / "changed on
   disk", `read_file` the region again, then re-patch against the fresh text.
   **For the harvest.rs rewrite (Task 2) do NOT use a whole-file `write_file`** —
   make small, targeted `patch` edits, one region at a time. See the 🛑 Notes-for-
   executor block at the top of the Update Log: the prior dispatch burned its entire
   600-turn budget doing exactly the whole-file write this rule forbids.

## Current state

**The store is one append-only file, `phase_runs.jsonl`, holding three line
types** discriminated by a `record` field: `PhaseRun` (no `record` field),
`PhaseReview` (`record:"review"`), `ArchitectActivity`
(`record:"architect_activity"`). Each reader version-gates on
`schema_version == TELEMETRY_SCHEMA_VERSION` (currently `1`) **and** filters to
its own record tag, so the line types coexist safely and a new fourth type is
simply ignored by the existing three readers.

**`ArchitectTokens`** (telemetry.rs ~428) is the shared four-class token type
(`input`, `cache_creation`, `cache_read`, `output`, all `u64`). Leave it
exactly as is — the ledger reuses it unchanged.

**The `ArchitectActivity` store trio you will mirror** (telemetry.rs ~520–579):

```rust
pub fn fold_activities(activities: Vec<ArchitectActivity>) -> Vec<ArchitectActivity> {
    use std::collections::HashMap;
    let mut latest: HashMap<(String, String, u64), usize> = HashMap::new();
    let mut out: Vec<ArchitectActivity> = Vec::new();
    for act in activities {
        let key = (act.phase_id.clone(), act.activity.clone(), act.ts);
        if let Some(&idx) = latest.get(&key) {
            out[idx] = act;                 // last-wins
        } else {
            latest.insert(key, out.len());
            out.push(act);
        }
    }
    out
}

pub fn append_architect_activity(
    telemetry_dir: &Path,
    activity: &ArchitectActivity,
) -> std::io::Result<PathBuf> {
    use std::io::Write;
    std::fs::create_dir_all(telemetry_dir)?;
    let path = telemetry_dir.join("phase_runs.jsonl");
    let mut value = serde_json::to_value(activity).map_err(std::io::Error::other)?;
    value["schema_version"] = TELEMETRY_SCHEMA_VERSION.into();   // stamped at the write boundary
    let line = serde_json::to_string(&value).map_err(std::io::Error::other)?;
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(&path)?;
    file.write_all(line.as_bytes())?;
    file.write_all(b"\n")?;
    Ok(path)
}

pub fn read_architect_activities(path: &Path) -> std::io::Result<Vec<ArchitectActivity>> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    Ok(content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|v| {
            v.get("schema_version").and_then(serde_json::Value::as_u64)
                == Some(TELEMETRY_SCHEMA_VERSION as u64)
        })
        .filter_map(|v| serde_json::from_value::<ArchitectActivity>(v).ok())
        .filter(|a| a.record == ARCHITECT_ACTIVITY_RECORD_TAG)
        .collect())
}
```

**The current `harvest.rs`** reads transcripts into a flat `struct Usage {
ts_ms, tokens }` (dedup by `message.id`, first-occurrence wins), then
`attribute()` sends each `Usage` to the nearest journal-activity time-window and
appends enriched activity copies. You keep the **file-reading + dedup +
timestamp** scaffold and **delete** the `Usage`→activity attribution
(`attribute`, the enrich loop, the `sorted`/`fold_activities` plumbing).

**Verified facts about the transcript corpus** (measured on this project's 59
`~/.claude/projects/-home-matt-src-rexyMCP/*.jsonl`, 9,406 assistant-usage
lines):

- **Every duplicate `message.id` is within a single file** (0 cross-file). So
  dedup by `message.id` (first-occurrence wins) attributes each surviving
  message to exactly one file → one session, unambiguously. 4,408 of the 9,406
  lines are within-file repeats (compaction rewrites re-append the transcript).
- **Every line carries `message.model`** (0 absent).
- **Every line carries the nested `usage.cache_creation.ephemeral_{5m,1h}_input_tokens`
  split.**
- **2,503 lines have no `attributionSkill`** (interactive design talks / direct
  edits) — these MUST be counted under an `"other"` skill bucket, not dropped.

## Spec

### Task 1 — `ArchitectLedger` record + store trio (telemetry.rs, additive)

Add, near the `ArchitectActivity` definitions:

```rust
/// The literal value of `ArchitectLedger.record`. Single-sources the discriminator.
pub const ARCHITECT_LEDGER_RECORD_TAG: &str = "architect_ledger";

/// One harvested architect-usage bucket: the token totals for a single
/// `(project_id, session_id, model, skill)` slice of a project's Claude Code
/// transcripts. Written by `rexymcp harvest`; the executor never writes one.
/// Coexists with `PhaseRun` / `PhaseReview` / `ArchitectActivity` in
/// `phase_runs.jsonl`, discriminated by `record`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArchitectLedger {
    /// Literal discriminator. Always `"architect_ledger"`. `#[serde(default)]`
    /// so a line of another record type deserializes with `record == ""` here
    /// and is filtered out by `read_architect_ledger`.
    #[serde(default)]
    pub record: String,
    /// Project identity (from `[project].id` or `--project-id`). `None` if unset.
    #[serde(default)]
    pub project_id: Option<String>,
    /// Claude Code session identity — the transcript file stem (== the file's
    /// `sessionId`). Since all `message.id` duplicates are within one file,
    /// dedup makes this unambiguous.
    pub session_id: String,
    /// The architect model that produced these tokens (`message.model`).
    pub model: String,
    /// The skill/slash-command the tokens were attributed to
    /// (`attributionSkill`), or `"other"` when the message carried none.
    pub skill: String,
    /// Summed four-class token usage over the deduped messages in this slice.
    pub tokens: ArchitectTokens,
    /// 5-minute-TTL share of `tokens.cache_creation`
    /// (`usage.cache_creation.ephemeral_5m_input_tokens`, summed).
    #[serde(default)]
    pub cache_creation_5m: u64,
    /// 1-hour-TTL share of `tokens.cache_creation`
    /// (`usage.cache_creation.ephemeral_1h_input_tokens`, summed).
    #[serde(default)]
    pub cache_creation_1h: u64,
    /// Count of deduped messages folded into this slice.
    pub messages: u64,
    /// Epoch-ms of the latest message in this slice (harvest freshness signal).
    pub last_ts: u64,
}
```

**Invariant to uphold when building a ledger (Task 2), pinned by a test:**
`cache_creation_5m + cache_creation_1h == tokens.cache_creation`.

Add the store trio, each **mirroring the `ArchitectActivity` example above**:

- **`append_architect_ledger(telemetry_dir: &Path, ledger: &ArchitectLedger) ->
  std::io::Result<PathBuf>`** — identical body shape to
  `append_architect_activity` (create dir, stamp `schema_version` at the write
  boundary via the `serde_json::Value` overlay, append the line).
- **`read_architect_ledger(path: &Path) -> std::io::Result<Vec<ArchitectLedger>>`**
  — identical to `read_architect_activities` but deserializing `ArchitectLedger`
  and filtering `r.record == ARCHITECT_LEDGER_RECORD_TAG`.
- **`fold_ledger(ledgers: Vec<ArchitectLedger>) -> Vec<ArchitectLedger>`** —
  identical to `fold_activities` but with the **key `(project_id.clone(),
  session_id.clone(), model.clone(), skill.clone())`** (a 4-tuple of
  `Option<String>` + three `String`s), last-wins, preserved input order. This is
  what makes re-harvest idempotent: a second harvest appends fresh full-sum
  records that replace the prior ones per key.

### Task 2 — rewrite `harvest.rs` to build ledger records

**Keep unchanged:** `days_from_civil`, `parse_iso_to_epoch_ms` (now feeding
`last_ts`), and the directory walk (`read_dir`, filter `*.jsonl`, **sort by path
for deterministic dedup order**).

**Delete:** `struct Usage`, `attribute`, `add_tokens`'s activity use (you may
keep a token-summing helper), and the `harvest()` body's
`fold_activities`/`sorted`/enrich-append loop.

**New extraction — per assistant line, in file order over sorted files:**

Skip the line (do **not** count it) when any of these holds:
- `v["type"] != "assistant"`.
- `v["message"]["usage"]` is absent.
- `v["message"]["id"]` is absent (cannot dedup).
- its `message.id` was already seen (a `HashSet<String>`, first-occurrence wins)
  — increment a `duplicates` counter.

Otherwise extract:
- **session_id** = the file's stem (`path.file_stem().and_then(OsStr::to_str)`;
  skip the file if it has no valid stem).
- **model** = `message["model"]` as str, else `"unknown"`.
- **skill** = `v["attributionSkill"]` as a non-empty str, else `"other"`
  (a `null`, absent, or empty-string value all map to `"other"`).
- **input / cache_read / output** = `usage.input_tokens` /
  `usage.cache_read_input_tokens` / `usage.output_tokens` (`as_u64`, 0 if
  absent).
- **cache-creation split** — the 5m/1h buckets are authoritative when the nested
  object is present:
  - `cc_5m = usage["cache_creation"]["ephemeral_5m_input_tokens"]` (0 if absent).
  - `cc_1h = usage["cache_creation"]["ephemeral_1h_input_tokens"]` (0 if absent).
  - **If `usage["cache_creation"]` is a present object:** `cache_creation =
    cc_5m + cc_1h`.
  - **Else** (no nested split — legacy/other clients): `cache_creation =
    usage["cache_creation_input_tokens"]` (0 if absent); `cc_5m = cache_creation`;
    `cc_1h = 0`. This keeps the invariant `cc_5m + cc_1h == cache_creation` in
    both branches.
- **ts** = `parse_iso_to_epoch_ms(v["timestamp"])`, else `0`.

Accumulate into a `HashMap<(String /*session*/, String /*model*/, String
/*skill*/), Accum>` where `Accum` sums the four token classes + `cc_5m` + `cc_1h`
+ a message count, and tracks `max(last_ts)`.

**Emit:** for each `(session, model, skill)` key, one `ArchitectLedger` with the
run's `project_id` (from `--project-id` else `cfg.project.id`), the summed
tokens, the 5m/1h totals, `messages`, and `last_ts`. Append each via
`append_architect_ledger`. Iterate keys in a **sorted order** (collect keys,
`sort()`) so the appended lines are deterministic.

**`HarvestOutcome`** becomes:

```rust
pub struct HarvestOutcome {
    pub path: PathBuf,      // the store path written
    pub messages: usize,    // deduped messages counted
    pub duplicates: usize,  // message.id repeats skipped
    pub sessions: usize,    // distinct session files that contributed >=1 message
    pub records: usize,     // ledger records written (== distinct keys)
}
```

`harvest()` keeps its signature (`config_path`, `telemetry_path`, `&HarvestArgs`)
and its telemetry-dir / project-id resolution (the top ~20 lines that load
config, resolve `telemetry_dir`, and compute `project_id`). It no longer reads
existing activities (drop the `read_architect_activities` / `fold_activities`
call).

### Task 3 — update the CLI output (main.rs)

The `Commands::Harvest` arm's success `println!` currently prints
`"harvested {} messages, enriched {} activities ({} unattributed) -> {}"`.
Replace it to report the new outcome, e.g.:

```
harvested {messages} messages across {sessions} sessions -> {records} ledger records ({duplicates} duplicates skipped) -> {path}
```

The clap `Harvest { config, transcript_dir, project_id, telemetry_path }`
arguments are unchanged.

## Acceptance criteria

- [ ] `ArchitectLedger` + `ARCHITECT_LEDGER_RECORD_TAG` exist;
      `append_architect_ledger` / `read_architect_ledger` / `fold_ledger` round-trip
      a ledger record through `phase_runs.jsonl` (write → read returns it), and
      `read_architect_ledger` **ignores** `PhaseRun` and `ArchitectActivity` lines
      in the same file (record-tag + schema-version gated).
- [ ] `fold_ledger` keeps the **last** record per `(project_id, session_id,
      model, skill)` key (idempotent re-harvest).
- [ ] `harvest` dedups by `message.id` (repeats counted once; `duplicates`
      reported), counts messages with **no `attributionSkill` under an `"other"`
      skill bucket** (not dropped), keys buckets by `(session, model, skill)`,
      and sets `cache_creation_5m + cache_creation_1h == tokens.cache_creation`
      for every emitted record.
- [ ] Running `harvest` **twice** over the same transcripts yields, after
      `read_architect_ledger` + `fold_ledger`, the **same** per-key totals as one
      run (idempotency).
- [ ] `cargo fmt --all --check`, `cargo build`,
      `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo test`
      all pass.
- [ ] `mcp/src/costs.rs` and every dashboard/`profile` surface are **untouched**
      (grep the diff: no `costs.rs` hunk).

## Test plan

In `executor/src/store/telemetry.rs` `mod tests`:

- **`architect_ledger_roundtrips_through_store`** — build an `ArchitectLedger`,
  `append_architect_ledger`, `read_architect_ledger`, assert the vec is `[the
  ledger]` (all fields equal). Mutation-sensitive: it must fail if the write
  drops a field or the read mis-filters.
- **`read_ledger_ignores_other_record_types`** — write a store file containing a
  `PhaseRun` line (use the existing `sample()` via `append`), an
  `ArchitectActivity` (via `append_architect_activity`), and an
  `ArchitectLedger` (via `append_architect_ledger`); assert
  `read_architect_ledger` returns exactly the one ledger, and — for symmetry —
  `read_architect_activities` still returns exactly the one activity (the ledger
  line does not leak into the activity reader).
- **`fold_ledger_keeps_last_per_key`** — two ledgers with the **same**
  `(project, session, model, skill)` but different `tokens`; assert `fold_ledger`
  returns one, with the second's tokens. A third with a different `skill` stays
  separate (len 2). Must fail if the key omits any of the four components (e.g.
  drop `skill` from the key → the different-skill record wrongly collapses).

In `mcp/src/harvest.rs` `mod tests` (extend/replace the existing harvest tests;
keep the `parse_iso_*` tests and the `write_fixture` helper — fixture line shape
example below):

```rust
// A realistic assistant-usage line (fields the harvester reads):
r#"{"type":"assistant","timestamp":"2026-07-09T16:00:00.000Z","attributionSkill":"rexymcp:dispatch","message":{"id":"msg_1","role":"assistant","model":"claude-opus-4-8","usage":{"input_tokens":1000,"cache_read_input_tokens":3000,"output_tokens":400,"cache_creation_input_tokens":2000,"cache_creation":{"ephemeral_5m_input_tokens":500,"ephemeral_1h_input_tokens":1500}}}}"#
```

- **`harvest_dedups_by_message_id`** — a session file with the same usage line
  repeated 5× plus one distinct line; assert the emitted ledger's `messages == 2`
  and `tokens.input` is the single-count sum (e.g. `6369 + 100`, **not** ×5), and
  `HarvestOutcome.duplicates == 4`.
- **`harvest_buckets_by_session_model_skill`** — fixtures across **two** files
  (two sessions) with two models and two skills; assert one ledger per distinct
  `(session, model, skill)` and that a line **without** `attributionSkill` lands
  in a `skill == "other"` record (counted, not dropped). Pin the negative: assert
  **no** message was dropped — total `messages` across records == total distinct
  `message.id`s in the fixtures.
- **`harvest_splits_cache_creation_5m_1h`** — a line with the nested split
  (`5m=500`, `1h=1500`); assert the ledger has `cache_creation_5m == 500`,
  `cache_creation_1h == 1500`, and `tokens.cache_creation == 2000`. Add a second
  line with **no nested object** but `cache_creation_input_tokens == 900`; assert
  its record has `cc_5m == 900`, `cc_1h == 0`, `cache_creation == 900` (the
  fallback branch), and that `cc_5m + cc_1h == cache_creation` holds. Must fail
  if the split is read from the wrong field or the invariant is broken.
- **`harvest_is_idempotent`** — run `harvest()` twice against the same TempDir
  fixtures + store; `read_architect_ledger` + `fold_ledger`; assert the folded
  per-key totals equal a single run's (the second run's records replace, not
  double).

All tests hermetic (`tempfile::TempDir` for both the transcript dir and the
telemetry store; a minimal config written to the TempDir — reuse the harvest
tests' existing `make_config` helper).

## End-to-end verification

The real artifact is the `rexymcp harvest` CLI writing ledger records to the
store. Verify both the logic (hermetic tests above) **and** one real CLI run:

1. Under a repo-relative scratch dir (e.g. `./.harvest-e2e/` — scope-confined;
   delete it when done), write a small 2-file transcript fixture (2 sessions,
   ≥2 skills incl. one line with no `attributionSkill`, the 5m/1h split), a
   minimal `rexymcp.toml` with a `[project] id`, and an empty store path.
2. Run `cargo run -p rexymcp -- harvest --config ./.harvest-e2e/rexymcp.toml
   --transcript-dir ./.harvest-e2e/transcripts --telemetry-path
   ./.harvest-e2e/phase_runs.jsonl` and **quote the stdout line** in the
   completion Update Log.
3. `read_file ./.harvest-e2e/phase_runs.jsonl` and confirm one
   `"record":"architect_ledger"` line per `(session, model, skill)` key, the
   `"other"` skill bucket present, and `cache_creation_5m + cache_creation_1h`
   equal to `tokens.cache_creation` on each line. Quote a representative line.
4. `rm -rf ./.harvest-e2e/` so no scratch artifact is committed.

Do **not** run harvest against the real `~/.claude/projects/…` in the E2E (host
state, non-hermetic).

## Authorizations

- Editing `executor/src/store/telemetry.rs`, `mcp/src/harvest.rs`, and the
  `Commands::Harvest` output line in `mcp/src/main.rs` is **in scope**.
- **No new dependencies.** `serde_json`, `tempfile`, `std::collections` are
  already available.
- No `docs/architecture.md` / `STANDARDS.md` / `WORKFLOW.md` / other-phase-doc
  edits.
- No `unsafe`, no `#[allow]`, no new `unwrap`/`expect`/`panic!` in production
  paths (tests exempt).

## Out of scope

- **Pricing.** No price table, no per-model architect rates, no `cost()` over the
  5m/1h split — that is **06c-ii**. This phase only *records* the split.
- **Surfaces.** Do **not** touch `mcp/src/costs.rs` (`scope_costs` /
  `sum_architect_tokens`), the dashboard Budget panel, or `profile`. They keep
  reading `ArchitectActivity.tokens` for now; **06c-iii** rewires them onto the
  ledger and retires `ArchitectActivity.tokens`. Between this phase and 06c-iii
  the ledger sits in the store unused by the surfaces — that is expected and
  acceptable (the surfaces are no more broken than before this phase).
- **Deleting `ArchitectActivity` or its `tokens` field.** It stays: the journal
  still writes activities for the assist/draft/dispatch/review vocabulary the
  dashboard's assist count uses. Only its harvest **enrichment** is removed.
- **Milestone attribution of architect tokens.** Transcripts carry no milestone
  id, so `ArchitectLedger` has none. How milestone-scope architect cost is
  presented is a **06c-iii** decision; do not invent a milestone field here.
- **A milestone/`cwd` filter on transcripts.** The transcript dir is already one
  project's history; count all of it.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
### 🛑 Notes for executor — 2026-07-21 (READ FIRST — refined re-dispatch after budget_exceeded)

**The previous dispatch burned its ENTIRE 600-turn budget and finished NOTHING on
`harvest.rs`.** Root cause: it tried to replace the whole `mcp/src/harvest.rs` file
in a single `write_file` call, repeatedly — each attempt corrupted the file, got
reverted, and retried until the budget ran out. **Do not repeat this.**

**Already DONE — do NOT redo:**
- **Task 1 is complete and committed** (commit `7299fa6`): `ArchitectLedger` +
  `ARCHITECT_LEDGER_RECORD_TAG` + `fold_ledger` / `append_architect_ledger` /
  `read_architect_ledger` + 3 unit tests, all in `executor/src/store/telemetry.rs`.
  It compiles (`cargo check -p rexymcp-executor` is green). **Do NOT touch
  `telemetry.rs`.** Mark Task 1 done and go straight to Task 2.

**HARD RULE for Task 2 (`mcp/src/harvest.rs`): never rewrite the whole file in one
`write_file`.** `harvest.rs` is ~530 lines; a single write that large is exactly
what killed the last run. Make **small, targeted `patch` edits, one region at a
time**, and run `cargo check -p rexymcp` after each so you stay green. Suggested
order (one `patch` per step):

1. **Imports** — `patch` the `use rexymcp_executor::store::telemetry::...` line to
   add `ArchitectLedger`, `ARCHITECT_LEDGER_RECORD_TAG`, `append_architect_ledger`
   (drop `ArchitectActivity`/`fold_activities` if they become unused).
2. **Delete the old attribution core, in separate `patch` hunks** — one `patch`
   each to remove `struct Usage`, remove `fn attribute`, and remove the
   `read_architect_activities` + `fold_activities` + enrich-append loop inside
   `harvest()`. Do not delete them all in one edit.
3. **`HarvestOutcome`** — `patch` the struct to the new fields (`duplicates`,
   `sessions`, `records`) per the Spec.
4. **Add the ledger-building helpers** via `patch` inserts after an existing fn:
   the per-message extraction (session = file stem, model, skill-or-`"other"`, the
   5m/1h `cache_creation` split with its fallback branch) and the
   `(session, model, skill)` accumulation.
5. **`harvest()` body** — `patch` the old attribution block (only) to emit
   `ArchitectLedger` records via `append_architect_ledger`, keeping the config /
   telemetry-dir / project-id resolution at the top of the fn.
6. **Task 3** — one small `patch` to the `Commands::Harvest` output line in
   `mcp/src/main.rs`.
7. **Then the tests** (`harvest.rs` `mod tests`) and the E2E, per the Test plan.

On a `patch` "0 matches" / "changed on disk", `read_file` that region again and
re-patch against the fresh text — **never** fall back to a whole-file write. The
Spec (Tasks, Test plan, E2E) is otherwise unchanged; this block only constrains
*how* you edit `harvest.rs`.

### Update — 2026-07-21 (escalation)

**Chosen lever:** refined re-dispatch
**Rationale:** the `budget_exceeded` traced entirely to a spec-shape gap — the
executor chose a whole-file `write_file` for the ~530-line harvest.rs rewrite and
looped on corruption/revert; pinning targeted `patch` edits (and flagging that Task 1
is already committed) removes the failure without a costlier takeover.
