# Phase 06e: Auto-telemetry — periodic background harvest sweep inside `serve` + liveness

**Status:** in-progress
**Status:** todo
**Depends on:** phase-06c-iii-b, phase-06d
**Estimated diff:** ~520 lines
**Tags:** language=rust, kind=feature, size=m

## Goal

Make the architect ledger stay current **without any manual step**. Today the
transcript→ledger harvest only runs when a human types `rexymcp harvest` or when
the `/rexymcp:auto` loop calls it; the interactive workflow never harvests, and
Claude Code prunes transcripts after ~30 days, so unharvested history evaporates.
This phase adds a **periodic background sweep inside `rexymcp serve`**: on an
interval (default **60 s**) it re-runs `harvest()` (deriving the transcript dir
from the serve process's cwd and the project id from config), keeping the ledger
continuously fresh, and writes a small **liveness marker** so `rexymcp costs` can
show when the sweep last ran.

**A skip-guard makes the short interval safe.** `harvest()` is append-only —
it re-appends the whole ledger to the shared `phase_runs.jsonl` on every call
(readers fold last-wins at read time), so a naive 60 s sweep would bloat the store
(~10⁵–10⁶ records/day) and slow *every* telemetry reader. The sweep therefore
tracks a **watermark** (the max transcript-file mtime seen at the last harvest) and
**skips the harvest append when nothing changed** — the common case at 60 s. A tick
harvests only when a transcript actually changed since the last watermark;
otherwise it just refreshes the liveness marker. Near-real-time freshness, no
unbounded growth.

**Scope is deliberately narrow (this executor bounces on TUI churn and large
multi-site edits — see gotchas):** the sweep re-runs **harvest only**, is a **new
additive module**, and surfaces liveness as **one plain-text line in `costs`** (no
dashboard/ratatui change). Two things the milestone README floated are explicitly
**out of scope** and deferred — see § Out of scope:

- **Journal reconciliation / assist-count derivation.** Investigation (this
  phase's design pass) found `serve` **cannot** reconcile assist counts: its run
  registry (`mcp/src/jobs.rs`) is in-memory, transient (lost on restart), and
  never observes the architect-side escalation round-trips that define an
  "assist" (those happen in Claude Code, which then calls `execute_phase` again as
  an unlinked fresh run). So the journal path stays as-is and the sweep does not
  touch it.
- **Harvest/journal CLI deprecation.** Deferred to keep `/rexymcp:auto` working
  mid-milestone and because the assist finding above means the `journal` CLI must
  stay regardless.

## Architecture references

Read before starting:

- `mcp/src/harvest.rs` — `harvest()` (171–175, the function the sweep re-runs) and
  its `HarvestArgs` (17–22) / `HarvestOutcome` (25–31). The sweep calls this; do
  **not** reimplement harvesting.
- `mcp/src/stop_watcher.rs` — `watch_stop_sentinel` (20–37): the **worked example**
  for a spawned interval loop with an injectable poll `Duration` and a hermetic
  test harness (its `mod tests`, 55+). Mirror this shape.
- `mcp/src/main.rs` — the `Serve` handler (528–548): where the sweep task is
  spawned; the `Costs` handler (853–879): where the liveness line is printed. The
  `mod` declarations block (6–30): add `mod sweep;` there.
- `executor/src/config.rs` — `TelemetryConfig` (389–403): the additive-field
  target; `default_telemetry_dir` / load resolution (579–611) for how the
  telemetry dir is resolved.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.
5. **Verify the Claude Code transcript-dir convention on this host** before
   coding (the architect can only confirm the `/`→`-` rule; Claude Code owns the
   exact munging and may transform other characters). Run `ls ~/.claude/projects`
   and confirm an entry equals the cwd with every `/` replaced by `-`, case
   preserved — e.g. `/home/matt/src/rexyMCP` → `-home-matt-src-rexyMCP`. **Trust
   the live listing over this doc's sketch;** if the real munging differs from
   `/`→`-` for this repo's path, note it in "Notes for review" and implement what
   the listing shows. (The sweep's existence-check no-op below is the safety net
   if the derivation is imperfect.)
6. **Editing discipline (load-bearing — three prior M35 runs hard-failed here):**
   edit with `patch`/`patch_lines`, **never a whole-file `write_file`** on an
   existing file (a new file is created once with `write_file`, that's fine). View
   with `read_file` (`start_line`/`end_line`), **never `sed -n`/`cat`, and NEVER
   run the same command twice** — the governor hard-fails on identical repeated
   calls. Read once, act. `cargo check -p rexymcp` after each file.

## Current state

**`harvest()`** (`mcp/src/harvest.rs:171–175`) — the function the sweep re-runs.
It is **synchronous** and takes the transcript dir explicitly (it does **not**
derive it):

```rust
pub fn harvest(
    config_path: &Path,
    telemetry_path: Option<&Path>,
    args: &HarvestArgs,
) -> Result<HarvestOutcome, String>
```

with (`harvest.rs:17–31`):

```rust
pub struct HarvestArgs<'a> { pub transcript_dir: &'a Path, pub project_id: Option<&'a str> }
pub struct HarvestOutcome { pub path: PathBuf, pub messages: usize, pub duplicates: usize, pub sessions: usize, pub records: usize }
```

When `telemetry_path` is `None`, `harvest` resolves the telemetry dir from
`cfg.telemetry.dir` and **returns `Err("telemetry disabled: …")` if that is unset**
(`harvest.rs:183–190`). With `project_id: None` it falls back to `cfg.project.id`
(`harvest.rs:192–195`). Re-running `harvest` is idempotent for **readers** (records
fold last-wins per `(session,model,skill)` key), but it **appends** the full ledger
each call — see the interval-default rationale in Task 1.

**`TelemetryConfig`** (`executor/src/config.rs:389–403`) — telemetry is default-on:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TelemetryConfig {
    pub dir: Option<PathBuf>,
    pub enabled: bool,
}
impl Default for TelemetryConfig {
    fn default() -> Self { Self { dir: None, enabled: true } }
}
```

**The interval-loop worked example** — `mcp/src/stop_watcher.rs:20–37` (mirror this
exact shape: a `loop { sleep(poll).await; … }` with an injectable `poll: Duration`,
and a hermetic `#[tokio::test]` using a 1 ms poll):

```rust
pub async fn watch_stop_sentinel(
    repo_path: PathBuf,
    registry: Arc<JobRegistry>,
    run_id: String,
    poll: Duration,
) {
    loop {
        tokio::time::sleep(poll).await;
        if !registry.is_running(&run_id) { return; }
        if stop::sentinel_present(&repo_path) {
            registry.request_stop_all(CancelReason::UserStop);
            let _ = stop::clear_sentinel(&repo_path);
            return;
        }
    }
}
```

**The `Serve` handler** (`mcp/src/main.rs:528–548`) already computes and logs the
cwd; note that `config` (a `PathBuf`) is **moved** into `RexyMcpServer::new(config)`
at line 539, so clone it for the sweep **before** that line.

**The `Costs` handler** (`mcp/src/main.rs:853–879`) prints
`costs::format_costs(&report)` in the non-`json` branch — the liveness line is
appended right after, in that same branch only.

## Spec

### Task 1 — add `sweep_interval_secs` to `TelemetryConfig` (config.rs)

In `executor/src/config.rs`, add one **additive** field to `TelemetryConfig`
(`389–403`): `pub sweep_interval_secs: Option<u64>`, defaulting to `None` in the
`Default` impl. Add a `pub const DEFAULT_SWEEP_INTERVAL_SECS: u64 = 60;` near the
struct, and a small accessor `pub fn sweep_interval(&self) -> Duration` that returns
`Duration::from_secs(self.sweep_interval_secs.unwrap_or(DEFAULT_SWEEP_INTERVAL_SECS))`.

**Why 60 s is safe:** the skip-guard in Task 2 makes idle ticks cheap — a tick only
re-runs `harvest()` (and its append) when a transcript changed since the last
watermark, so the common 60 s tick just refreshes the liveness marker. Without the
guard a 60 s cadence would bloat the append-only store; with it, freshness is
near-real-time and growth is bounded to actual activity. The field still lets an
operator tune the cadence.

Because `#[serde(default)]` is on the struct and the new field is `Option`, existing
configs deserialize unchanged (no migration).

### Task 2 — new module `mcp/src/sweep.rs`

Add `mod sweep;` to the `mod` block in `mcp/src/main.rs` (6–30, alphabetical
neighborhood: after `mod status;`/`mod stop;`/`mod stop_watcher;`). The module
contains:

**(a) Transcript-dir derivation.** A pure fn that maps an absolute cwd to its
Claude Code transcript dir under `$HOME/.claude/projects/`:

```rust
/// `/home/matt/src/rexyMCP` → `<home>/.claude/projects/-home-matt-src-rexyMCP`.
/// Every `/` in the absolute cwd becomes `-`; case is preserved.
pub fn transcript_dir_for(home: &Path, cwd: &Path) -> PathBuf {
    let slug = cwd.to_string_lossy().replace('/', "-");
    home.join(".claude").join("projects").join(slug)
}
```

Pin the behavior with tests (see Test plan): `/home/matt/src/rexyMCP` →
slug `-home-matt-src-rexyMCP` (leading `/` → leading `-`); **case preserved**
(`rexyMCP`, not `rexymcp`) — a must-NOT: the slug must **not** be lowercased.

**(b) The liveness marker.** A small serde struct persisted in the telemetry dir as
`sweep_state.json`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SweepState {
    pub last_swept_ms: u64,        // when the last tick ran (harvest OR skip)
    pub outcome: String,           // e.g. "12 records / 480 msgs", "no change", "skipped: no transcript dir"
    pub last_seen_mtime_ms: u64,   // watermark: max transcript mtime at the last *harvest* (0 if never harvested)
}
```

- `pub fn write_liveness(telemetry_dir: &Path, state: &SweepState) -> Result<(), String>`
  — serialize to `<telemetry_dir>/sweep_state.json` (overwrite; single writer).
- `pub fn read_liveness(telemetry_dir: &Path) -> Option<SweepState>` — read + parse;
  `None` if the file is absent or unparseable (never an error to the caller).

**(b′) The skip-guard helpers** (two small, independently testable pieces):

```rust
/// Max mtime (millis since epoch) across `*.jsonl` files in `dir`. `None` if the dir
/// is unreadable or has no matching files. Stats only — no file contents read.
pub fn max_transcript_mtime_ms(dir: &Path) -> Option<u64>

/// Pure decision: harvest when we've never harvested (`prev` None) or a transcript
## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-07-22 15:39 (started)

**Executor:** phase-06e executor

**Tasks started:** Tasks 1–4 (config field, sweep module, serve handler, costs handler)
    match (current_mtime_ms, prev_watermark_ms) {
        (Some(cur), Some(prev)) => cur > prev,
        (Some(_), None) => true,   // never harvested → harvest
        (None, _) => false,        // nothing to harvest (no transcript files)
    }
}
```

Keep `should_harvest` a **pure** fn (inject both values) so the change/skip decision
is tested without depending on filesystem-mtime timing.

**(c) One sweep tick.** A **synchronous** fn the loop calls and tests exercise
directly (the real behavioral unit — keep the async loop thin):

```rust
/// Run one sweep tick + write the liveness marker. `now_ms` is injected (no wall
/// clock in the unit under test). Never returns Err — every outcome is folded into
/// the marker string and logged; a broken sweep must not crash serve.
pub fn sweep_once(config_path: &Path, transcript_dir: &Path, telemetry_dir: &Path, now_ms: u64)
```

Behavior, in order:
1. Read the prior marker: `let prev = read_liveness(telemetry_dir);` and its
   watermark `let prev_watermark = prev.as_ref().map(|s| s.last_seen_mtime_ms)`
   (treat `Some(0)` as "never harvested" for the skip decision — i.e. map `0` → the
   `prev = None` branch, or equivalently a watermark of `0` always harvests on a
   real transcript).
2. **If `transcript_dir` does not exist** (`!transcript_dir.exists()`) → write a
   marker `{ last_swept_ms: now_ms, outcome: "skipped: no transcript dir",
   last_seen_mtime_ms: prev_watermark.unwrap_or(0) }` and return. Safety net for an
   imperfect munging or a repo Claude Code never opened — a **no-op, not an error**.
3. Compute `let current = max_transcript_mtime_ms(transcript_dir);`.
4. **Skip path** — if `!should_harvest(current, prev_watermark_as_option)`: write a
   marker `{ last_swept_ms: now_ms, outcome: "no change",
   last_seen_mtime_ms: prev_watermark.unwrap_or(0) }` and return. **Do NOT call
   `harvest()`** — this is the whole point (no append).
5. **Harvest path** — otherwise call
   `harvest(config_path, None, &HarvestArgs { transcript_dir, project_id: None })`:
   - `Ok(o)` → `outcome = format!("{} records / {} msgs", o.records, o.messages)`;
     advance the watermark to `current.unwrap_or(0)`.
   - `Err(e)` → `outcome = format!("error: {e}")`; **do not advance** the watermark
     (carry `prev_watermark.unwrap_or(0)`), so the next tick retries the harvest.
   Write `{ last_swept_ms: now_ms, outcome, last_seen_mtime_ms: <per above> }`.
6. Log one line to stderr (e.g. `eprintln!("rexymcp sweep: {}", outcome)`).

(The exact handling of the `Some(0)` watermark sentinel is the executor's call —
what's pinned: a first-ever sweep on a real transcript dir **harvests**; a
subsequent sweep with no transcript change **skips without appending**; a sweep
after a transcript changes **harvests again**.)

**(d) The interval loop.** Async, mirrors `watch_stop_sentinel`:

```rust
pub async fn run_sweep_loop(
    config_path: PathBuf,
    transcript_dir: PathBuf,
    telemetry_dir: PathBuf,
    interval: Duration,
) {
    loop {
        tokio::time::sleep(interval).await;
        let now_ms = /* real wall-clock millis; see note */;
        sweep_once(&config_path, &transcript_dir, &telemetry_dir, now_ms);
    }
}
```

For `now_ms` in the loop (production only, not under unit test), read real time
with `std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)` mapped to
millis, `unwrap_or(0)` on the (impossible pre-epoch) error — this is production
wall-clock, and the determinism rule applies to the **unit under test**
(`sweep_once`, which takes `now_ms`), which is what the tests drive. Do **not** put
`SystemTime::now()` inside `sweep_once`.

**(e) Liveness line formatter.** `pub fn liveness_line(state: &SweepState, now_ms: u64) -> String`
returning a single human line, e.g. `"Last swept: <n>m ago (12 records / 480 msgs)"`
computed from `now_ms - state.last_swept_ms`. Pin the *content* (that it names the
elapsed time and the outcome), not exact rendering.

### Task 3 — spawn the sweep in the `Serve` handler (main.rs)

In the `Commands::Serve` arm (`main.rs:528–548`), after the existing cwd/eprintln
block and **before** `RexyMcpServer::new(config)` moves `config`:

1. Load config (`Config::load_with_env(&config)`) to read `telemetry`.
2. **Only if `cfg.telemetry.enabled` and `cfg.telemetry.dir` is `Some`**, derive
   `transcript_dir = sweep::transcript_dir_for(&home, &cwd_path)` (home from
   `std::env::var_os("HOME")`; if `HOME` is unset, skip spawning and log it) and
   `tokio::spawn(sweep::run_sweep_loop(config.clone(), transcript_dir, dir.clone(), cfg.telemetry.sweep_interval()))`.
   Log one line that the sweep started with its interval.
3. If telemetry is disabled or `HOME`/dir is unavailable, **do not spawn** (log one
   line and continue) — serve still runs, just without auto-sweep.

Keep `config` usable for line 539 (clone for the sweep, or reorder so `new` gets the
original after the clone). The `ctrl_c().await` at 544 keeps the process (and the
spawned loop) alive until shutdown, exactly as today.

### Task 4 — liveness line in `rexymcp costs` (main.rs)

In the `Commands::Costs` arm (`main.rs:853–879`), in the **non-`json`** branch only,
after `println!("{}", costs::format_costs(&report));`:

- Resolve the telemetry dir from config (`Config::load_with_env(&config)` →
  `cfg.telemetry.dir`), and if `Some(dir)`, `sweep::read_liveness(&dir)`:
  - `Some(state)` → `println!("{}", sweep::liveness_line(&state, now_ms))` with
    `now_ms` = real wall-clock millis (this is a CLI print path, real time is fine
    here).
  - `None` → `println!("Last swept: never")`.
- Leave the `json` branch untouched (the report shape does not change; the marker is
  a serve-side artifact, not part of `CostReport`).

## Acceptance criteria

- [ ] `sweep::transcript_dir_for` maps `/home/matt/src/rexyMCP` (home `/home/matt`)
      to `…/.claude/projects/-home-matt-src-rexyMCP`, **case preserved** (a test
      asserts the slug is not lowercased).
- [ ] `sweep_once` against a temp transcript dir with a fixture `.jsonl` writes
      `sweep_state.json` with a non-empty `outcome` and the injected `last_swept_ms`;
      against a **missing** transcript dir it writes a `"skipped: no transcript dir"`
      marker and does **not** error.
- [ ] **Skip-guard:** a second `sweep_once` with no transcript change (files
      untouched since the prior harvest) writes a `"no change"` marker and does
      **not** append to `phase_runs.jsonl` (the store's line count is unchanged from
      after the first sweep). `should_harvest` returns `false` for `current <= prev`
      and `true` for `current > prev` / `prev = None`.
- [ ] `read_liveness` round-trips a written marker (including `last_seen_mtime_ms`)
      and returns `None` for an absent/garbage file.
- [ ] `run_sweep_loop` with a 1 ms interval writes/refreshes the marker within a
      bounded `tokio::time::timeout` (mirrors the `stop_watcher` test).
- [ ] `[telemetry] sweep_interval_secs = 120` in a config parses and
      `sweep_interval()` returns 120 s; unset → 60 s.
- [ ] `rexymcp costs` prints a `Last swept: …` line when a marker exists and
      `Last swept: never` when it does not (non-json path); `--json` output is
      unchanged.
- [ ] Telemetry **disabled** (or `HOME` unset) → serve starts and does **not** spawn
      the sweep (no panic, one explanatory log line).
- [ ] `cargo fmt --all --check`, `cargo build`,
      `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test` all
      pass.
- [ ] No change to `harvest()`, `journal`/`record_activity`, the dashboard render,
      `CostReport`'s shape, or the executor loop.

## Test plan

All in `mcp/src/sweep.rs` `#[cfg(test)] mod tests` unless noted. Hermetic
(`tempfile::TempDir`), deterministic (inject `now_ms`; inject the loop interval;
no real `Utc::now`/`SystemTime` in the unit under test). Reuse `harvest.rs`'s
fixture style (`write_fixture` writes a mock `*.jsonl`; a `rexymcp.toml` with
`[telemetry] dir = <temp>`).

- `transcript_dir_for_munges_slashes_to_dashes` — asserts the exact slug
  `-home-matt-src-rexyMCP` for cwd `/home/matt/src/rexyMCP`, home `/home/matt`.
- `transcript_dir_for_preserves_case` — cwd `/x/rexyMCP` slug ends `-rexyMCP`, **not**
  `-rexymcp` (the pinned must-NOT).
- `sweep_once_harvests_and_writes_marker` — temp transcript dir + one assistant-usage
  fixture line + temp telemetry dir/config; assert `sweep_state.json` exists,
  `last_swept_ms` equals the injected value, `outcome` is non-empty, and
  `phase_runs.jsonl` gained ledger records.
- `sweep_once_skips_append_when_unchanged` — run `sweep_once` twice with the fixture
  **untouched** between calls; record `phase_runs.jsonl`'s line count after the first
  sweep and assert it is **unchanged** after the second, and the second marker
  `outcome` is `"no change"`. (Deterministic: no write between sweeps → equal max
  mtime → skip.)
- `should_harvest_decides_on_watermark` — pure-fn unit test:
  `should_harvest(Some(5), None) == true`, `should_harvest(Some(5), Some(5)) == false`,
  `should_harvest(Some(6), Some(5)) == true`, `should_harvest(None, Some(5)) == false`.
- `sweep_once_missing_transcript_dir_is_noop` — transcript dir does not exist; assert
  the marker `outcome` is the `"skipped: no transcript dir"` sentinel and the call
  did not panic/return Err.
- `read_liveness_roundtrips` / `read_liveness_none_for_absent_or_garbage` — write via
  `write_liveness` then read back equal (incl. `last_seen_mtime_ms`); absent file and
  a non-JSON file both → `None`.
- `sweep_interval_defaults_to_60_when_unset` / `sweep_interval_honors_config` — in
  `config.rs` `mod tests` (or sweep tests via a written config): unset → 60 s;
  `sweep_interval_secs = 120` → 120 s.
- `run_sweep_loop_ticks_and_writes_marker` — `#[tokio::test]`; spawn the loop with a
  1 ms interval against a temp transcript+telemetry dir, `tokio::time::timeout(…)`
  until `sweep_state.json` appears, then assert it was written (mirror
  `stop_watcher.rs` `mod tests`). Keep the timeout small and bounded.
- `liveness_line_names_elapsed_and_outcome` — a `SweepState` + a `now_ms` 5 min later
  produces a line mentioning the elapsed minutes and the outcome text (assert by
  `contains` on the content, not exact rendering).

## End-to-end verification

The sweep only runs inside a live `rexymcp serve`, which this phase cannot spin up
hermetically. For the artifact check:

1. Quote the `cargo test` results for `sweep_once_harvests_and_writes_marker`,
   `sweep_once_skips_append_when_unchanged`, `sweep_once_missing_transcript_dir_is_noop`,
   and `run_sweep_loop_ticks_and_writes_marker` in the completion Update Log.
2. **Real-artifact E2E for the `costs` liveness line** (it ships a user-visible CLI
   change): with a `sweep_state.json` present in the telemetry dir, run
   `cargo run -p rexymcp -- costs --repo .` and **quote the `Last swept: …` line**;
   then temporarily point at an empty telemetry dir (or remove the marker) and quote
   the `Last swept: never` line. If you cannot place a marker by hand, run
   `cargo run -p rexymcp -- costs --repo .` and quote whichever of the two lines
   appears, and say which. The serve-side loop itself is exercised on the next real
   `rexymcp serve` (out of this phase's control) — note that in the log.

## Authorizations

- Editing `executor/src/config.rs` (`TelemetryConfig`), adding `mcp/src/sweep.rs`,
  and editing `mcp/src/main.rs` (`mod` block + `Serve`/`Costs` arms) is in scope.
- **No new dependencies** (`tokio`, `serde`, `serde_json`, `tempfile` are already
  in use).
- No `unsafe`, no `#[allow]`, no new `unwrap`/`expect`/`panic!` in production paths
  (test code exempt). `SystemTime::now()` in `run_sweep_loop` is production
  wall-clock, allowed; it must **not** appear in `sweep_once` or any unit-tested fn.
- No `docs/architecture.md`/`STANDARDS.md`/`WORKFLOW.md`/other-phase-doc edits.

## Out of scope

- **Journal reconciliation / assist-count derivation by the sweep.** `serve` cannot
  reconcile assists (transient in-memory registry; blind to architect-side
  escalation round-trips). The journal path and its consumers
  (`costs.rs:259–263`, `dashboard/mod.rs:72–75`) are untouched.
- **Harvest/journal CLI deprecation** and any `/rexymcp:auto` skill edit. Deferred —
  pulling the CLIs before/around this sweep would risk `/auto`, and the `journal`
  CLI must stay regardless (per the finding above).
- **Dashboard liveness display.** No `mcp/src/dashboard/**` change — liveness surfaces
  only via the `costs` plain-text line here (the ratatui panels are the executor's
  known TUI-hard-fail zone; keep this phase off them).
- **`phase_runs.jsonl` compaction.** The skip-guard bounds append-growth to *actual*
  transcript activity (idle ticks do not append), which is enough for this phase. A
  general store-compaction pass (rewriting the file folded, dropping superseded
  records) is still a separate concern — do **not** add one here.
- **Changing `harvest()`'s signature or behavior.** The sweep is a *caller*; do not
  refactor harvest.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
