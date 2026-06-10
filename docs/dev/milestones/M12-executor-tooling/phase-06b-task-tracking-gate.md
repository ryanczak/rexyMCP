# Phase 06b: Task-tracking gate — `[executor] task_tracking` + `LoopDeps` field

**Milestone:** M12 — Executor Tooling
**Status:** review
**Depends on:** phase-06a (done)
**Estimated diff:** ~120 lines (≈45 prod + ≈75 test)
**Tags:** language=rust, kind=feature, size=s

## Goal

Put 06a's task-tracking substrate behind a **config kill-switch** so it becomes a
clean A/B intervention. Add `[executor] task_tracking` (a bool, **default on**),
thread it to the loop as a new `LoopDeps` field, and gate 06a's turn-0 seeding
emit behind it. With the switch **off**, the loop emits **zero** `TaskUpdate`
events and the session is byte-identical to its pre-06a behavior — which is what
makes on-vs-off runs directly comparable on the scorecard.

This phase is **plumbing only**. It deliberately isolates the
**`LoopDeps`-struct-literal churn** (the phase-08a/08d stall class — a new
`LoopDeps` field touches every construction site) into a phase with *no other*
concern, mirroring how 06a isolated the new-`SessionEvent`-variant match-arm
wall. It does **not** add the model-facing flip tool, the router arm, or
prompt injection — those are **phase-06c**, where the `LoopDeps` field already
exists so 06c carries no literal churn.

## Architecture references

Read before starting:

- `docs/architecture.md` § Status #12 (M12) — Arc A: "Config-gated (a `[tasks]` /
  `[executor] task_tracking` toggle, default on) so it is a clean A/B
  intervention … no measurement without an off-switch."
- `docs/dev/milestones/M12-executor-tooling/README.md` § "Pre-injection
  watch-items" — "the off-switch byte-identity requirement is a pinned negative
  case: a test must assert that with `task_tracking = false` no `TaskUpdate`
  event is emitted … This lives in phase-06b."

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

06a added the substrate, all **unconditional**:

- `executor/src/store/sessions/event.rs` — the `TaskState` enum +
  `SessionEvent::TaskUpdate { id, title, state }` variant (do **not** touch).
- `executor/src/agent/tasks.rs` — the pure `seed_from_spec(phase_doc)` parser
  (do **not** touch).
- `executor/src/agent/mod.rs:181-195` — the seed-and-emit block runs on **every**
  phase, with no gate:

  ```rust
  // Task-tracking substrate (M12 Arc A / phase-06a): seed the TODO list from
  // the phase doc's Spec and broadcast it as one `pending` TaskUpdate each.
  for task in tasks::seed_from_spec(&input.phase_doc) {
      log_event(
          &log_handle,
          &redactor,
          deps.clock,
          0,
          SessionEvent::TaskUpdate {
              id: task.id,
              title: task.title,
              state: task.state,
          },
      );
  }
  ```

This phase wraps that block in `if deps.task_tracking { … }`.

### Config: how a gate field is shaped (worked example — `ContextConfig`)

`[context] output_filter` (M10) is the exact pattern to mirror for a
default-on bool toggle. From `executor/src/config.rs:26-41`:

```rust
/// Context-optimization settings (M10). `output_filter` is the kill-switch for
/// boundary output filtering — default on; set false to restore raw head+tail
/// truncation with no recovery file.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct ContextConfig {
    pub output_filter: bool,
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            output_filter: true,
        }
    }
}
```

**But the toggle lives in `[executor]`, not a new section** (per the locked
scope). `ExecutorConfig` (`config.rs:87-110`) does **not** carry `#[serde(default)]`
on the struct — it has required fields (`provider`/`model`/`base_url`) plus
per-field `#[serde(default = "…")]` on its optional ones (e.g.
`first_token_timeout_secs`). So the new field needs its **own**
`#[serde(default = "…")]` attribute to stay optional in TOML, plus a line in the
hand-written `Default for ExecutorConfig` impl (`config.rs:120-…`). See the
`default_first_token_timeout_secs` fn at `config.rs:112-114` for the helper-fn
shape.

### The `LoopDeps` literal blast radius — the known stall class

A new `LoopDeps` field is a hard compile error at **every** construction site
until each sets it (this is the phase-08a/08d stall class). The complete site
list (grep-verified):

| # | File:line | Site |
|---|---|---|
| 1 | `executor/src/agent/mod.rs:76` | the `LoopDeps<'a>` struct definition (add the field here) |
| 2 | `mcp/src/runner.rs:179` | the **production** literal (`let deps = LoopDeps {`) |
| 3 | `executor/src/agent/tests.rs:83` | the `deps()` **test helper** (most loop tests route through it) |
| 4 | `executor/src/agent/tests.rs:833` | standalone test literal |
| 5 | `executor/src/agent/tests.rs:953` | standalone test literal |
| 6 | `executor/src/agent/tests.rs:1619` | standalone test literal |
| 7 | `executor/src/agent/tests.rs:2370` | standalone test literal |
| 8 | `executor/src/agent/tests.rs:2443` | standalone test literal |
| 9 | `executor/src/agent/tests.rs:2628` | standalone test literal |
| 10 | `executor/src/agent/tests.rs:3390` | standalone test literal |
| 11 | `executor/src/agent/tests.rs:3586` | standalone test literal |
| 12 | `executor/src/agent/tests.rs:3649` | standalone test literal |

**Every one of these literals ends with the same last field** —
`governor: GovernorConfig::default(),` (the field M11 phase-01 added last). That
is your insertion anchor: add the new field **immediately after the `governor:`
line** in each literal.

**The stall-proof recipe (do this exactly — it converts "find all the sites"
into a compiler-driven checklist):**

1. Add the field to the struct (site 1) and to the **production** literal
   (site 2) and the **`deps()` helper** (site 3).
2. Run `cargo build`. Rustc emits one
   `error[E0063]: missing field `task_tracking` in initializer of `LoopDeps`` per
   remaining literal, **each with its exact line number** — sites 4–12.
3. Add `task_tracking: true,` after the `governor:` line at each line rustc
   names. Re-run `cargo build` until clean.

Do **not** try to find the 9 standalone literals by reading the 3700-line test
file top to bottom — let the compiler enumerate them. The line numbers above may
drift as you edit; the compiler's are authoritative.

## Spec

Numbered tasks in execution order.

1. **Add the config field.** In `executor/src/config.rs`:
   - Add to `ExecutorConfig` (after the `seed` field, `config.rs:109`):
     ```rust
     /// Whether the loop seeds a per-session task list from the phase doc's
     /// `## Spec` and emits `TaskUpdate` events as the executor flips state
     /// (M12 Arc A). Default on; set false for a control run with no task
     /// tracking (the seeding emit is byte-for-byte suppressed).
     #[serde(default = "default_task_tracking")]
     pub task_tracking: bool,
     ```
   - Add the default helper near `default_stream_idle_timeout_secs`
     (`config.rs:116-118`):
     ```rust
     fn default_task_tracking() -> bool {
         true
     }
     ```
   - Add to the `Default for ExecutorConfig` impl (after the `seed: None,` line):
     ```rust
     task_tracking: default_task_tracking(),
     ```

2. **Add the `LoopDeps` field.** In `executor/src/agent/mod.rs`, in the
   `LoopDeps<'a>` struct (`mod.rs:76`), append after the `governor` field
   (`mod.rs:108`):
   ```rust
   /// Whether to seed + emit the M12 Arc A task list. Read from
   /// `[executor] task_tracking` (default true). Off → zero `TaskUpdate`
   /// events, byte-identical to pre-06a behavior.
   pub task_tracking: bool,
   ```

3. **Wire the production literal.** In `mcp/src/runner.rs`, in the `LoopDeps`
   literal (`runner.rs:179`), after the `governor: inp.cfg.governor,` line, add:
   ```rust
   task_tracking: inp.cfg.executor.task_tracking,
   ```

4. **Fix the test literals (compiler-guided — see "stall-proof recipe" above).**
   In `executor/src/agent/tests.rs`, add `task_tracking: true,` after the
   `governor: GovernorConfig::default(),` line in the `deps()` helper (`tests.rs:83`)
   **and** in all 9 standalone literals. Use `cargo build` to enumerate the 9
   standalone sites by line number — do not hand-search. (All existing tests want
   `true`; the off-switch test in task 7 overrides to `false`.)

5. **Gate the seeding emit.** In `executor/src/agent/mod.rs`, wrap the existing
   06a seed-and-emit block (`mod.rs:181-195`, shown in Current state) in a guard:
   ```rust
   // Task-tracking substrate (M12 Arc A). Gated by [executor] task_tracking
   // (06b): off → no seeding, byte-identical to pre-06a.
   if deps.task_tracking {
       for task in tasks::seed_from_spec(&input.phase_doc) {
           log_event(
               &log_handle,
               &redactor,
               deps.clock,
               0,
               SessionEvent::TaskUpdate {
                   id: task.id,
                   title: task.title,
                   state: task.state,
               },
           );
       }
   }
   ```
   (Only the `if deps.task_tracking {` wrapper + closing brace are new — the body
   is 06a's verbatim.)

6. **Document the field in `rexymcp init`.** In `mcp/src/init.rs`, in the
   `[executor]` block of the `TEMPLATE` raw string (after the commented
   `# temperature = …` line, near `init.rs:14`), add one commented line:
   ```
   # task_tracking = true            # seed + track a per-session task list from the phase Spec (M12)
   ```
   (Commented, so the default-on behavior is unchanged and the field is
   discoverable. Match the column alignment of the surrounding comments.)

## Acceptance criteria

- [ ] `cargo build` succeeds with zero new warnings; `cargo clippy
      --all-targets --all-features -- -D warnings` passes; `cargo fmt --all
      --check` passes; `cargo test` passes (existing + new).
- [ ] `[executor] task_tracking` deserializes from TOML, defaults to `true` when
      absent, and can be set `false`.
- [ ] With `deps.task_tracking == true`, a loop run over a phase doc with an
      N-item `## Spec` writes exactly N `task_update` records at turn 0 (06a's
      behavior, unchanged).
- [ ] With `deps.task_tracking == false`, the **same** run writes **zero**
      `task_update` records, and the session log is otherwise unchanged (the
      `SessionStart`/`Prompt` records are byte-identical to the on-run's).
- [ ] No change to `event.rs`, `tasks.rs`, `status.rs`, the dashboard, the tool
      registry, the router, or the system prompt in this phase.

## Test plan

In `executor/src/config.rs` (`#[cfg(test)] mod tests`, mirror
`context_output_filter_can_be_disabled` at `config.rs:603`):

- `executor_task_tracking_defaults_on` — a config with an `[executor]` section
  that omits `task_tracking` → `cfg.executor.task_tracking == true`.
- `executor_task_tracking_can_be_disabled` — `task_tracking = false` in
  `[executor]` → `cfg.executor.task_tracking == false`.

In `executor/src/agent/tests.rs` (loop integration, mirror 06a's
`loop_seeds_task_updates_from_spec` / `loop_emits_no_task_updates_when_spec_absent`):

- `loop_emits_no_task_updates_when_tracking_off` — build deps for a phase doc
  with a 3-item `## Spec`, then **override the flag off**:
  ```rust
  let mut d = deps(&client, &registry, &budget, max_turns, root);
  d.task_tracking = false;
  ```
  Run the loop; assert the session log has **zero** `event_kind == "task_update"`
  records. (Mutation-resistant: a naive "always seed" impl yields 3 and fails.)
- `loop_still_seeds_task_updates_when_tracking_on` — the same 3-item-Spec doc
  with the default-on `deps()` (no override) → exactly 3 `task_update` records,
  all `Pending`. (Pins that the gate's *on* path is 06a's behavior. If
  `loop_seeds_task_updates_from_spec` from 06a already asserts exactly this via
  the `deps()` helper, you may rely on it instead of duplicating — but the
  paired off/on tests read best together; the executor's call.)

The 06a tests that route through `deps()` (now `task_tracking: true`) must stay
green at their existing counts.

## End-to-end verification

The gate has no new user-visible CLI surface (the model-facing behavior is 06c),
so the **off-switch loop test is the behavioral end-to-end proof** — it exercises
the full `LoopDeps.task_tracking → seeding-emit gate → session log` path with a
`MockAiClient`. In addition, verify the config field round-trips through the real
binary's scaffolder:

```
cargo run -p rexymcp -- init --dir <tmpdir>
grep task_tracking <tmpdir>/rexymcp.toml
```

Confirm the generated `rexymcp.toml` contains the documented `task_tracking`
line. Quote the grep output in the completion Update Log.

## Authorizations

None. (No new dependencies; no `Cargo.toml`/architecture/STANDARDS/WORKFLOW
edits. `rexymcp.toml` is **not** edited — only the `init` template string and the
`config.rs` schema.)

## Out of scope

Do **not**, in this phase:

- Add the model-facing flip tool (`update_task`), its `router::categorize` arm,
  or its registration in `build_registry` — that is **phase-06c**.
- Inject the task list or any task instructions into the system prompt — that is
  **phase-06c**. (06b adds **no** prompt change; that is what keeps the off-run's
  `Prompt` record byte-identical to the on-run's, and is testable as such.)
- Touch the dashboard `Tasks` panel / `panels.rs` / `render.rs` — **phase-07**.
- Touch `event.rs`, `tasks.rs`, `status.rs`, or any `SessionEvent` match arm
  (06a already landed the full variant blast radius; no variant work here).
- Edit the user's live `rexymcp.toml`.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-06-10 (progress)

**Executor:** Claude (direct)

Added `[executor] task_tracking` config field (default `true`), threaded it through `LoopDeps`, gated the 06a seed-and-emit block, and wired all 12 construction sites (1 struct, 1 production, 11 test). Added 4 tests: 2 config round-trip + 2 loop integration (off/on). Updated `rexymcp init` template with the commented config line.

### Update — 2026-06-10 (complete)

**Summary:** Implemented the `[executor] task_tracking` kill-switch for M12 Arc A's task-tracking substrate. Added a default-on bool to `ExecutorConfig`, a corresponding `LoopDeps` field, and gated the 06a seeding emit behind `if deps.task_tracking`. All 12 `LoopDeps` construction sites updated (compiler-guided for the 9 standalone test literals). Config field round-trips through TOML deserialization and the `rexymcp init` scaffolder.

**Acceptance criteria:** all ticked above.

**Commands:**

```
cargo fmt --all --check
(clean, exit 0)

cargo build 2>&1 | tail -20
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.1s

cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail -20
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.5s

cargo test 2>&1 | tail -30
test result: ok. 710 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 6.04s
```

**End-to-end verification:**

```
$ cargo run -p rexymcp -- init --dir <tmpdir>
wrote rexymcp.toml
$ grep task_tracking <tmpdir>/rexymcp.toml
# task_tracking = true            # seed + track a per-session task list from the phase Spec (M12)
```

The generated `rexymcp.toml` contains the documented `task_tracking` line as a commented default.

**Grep for spec-pinned literal:**
```
$ grep -rn 'task_tracking' executor/src/config.rs mcp/src/init.rs executor/src/agent/mod.rs mcp/src/runner.rs
executor/src/config.rs:113:    #[serde(default = "default_task_tracking")]
executor/src/config.rs:114:    pub task_tracking: bool,
executor/src/config.rs:127:fn default_task_tracking() -> bool {
executor/src/config.rs:142:            task_tracking: default_task_tracking(),
executor/src/agent/mod.rs:111:    pub task_tracking: bool,
executor/src/agent/mod.rs:185:    if deps.task_tracking {
mcp/src/runner.rs:198:        task_tracking: inp.cfg.executor.task_tracking,
mcp/src/init.rs:15:# task_tracking = true            # seed + track a per-session task list from the phase Spec (M12)
```

**Files changed:**
- `executor/src/config.rs` — added `task_tracking` field to `ExecutorConfig`, default helper, `Default` impl, 2 config tests
- `executor/src/agent/mod.rs` — added `task_tracking` field to `LoopDeps`, gated the seed-and-emit block
- `executor/src/agent/tests.rs` — added `task_tracking: true` to `deps()` helper + 9 standalone literals, 2 new integration tests
- `executor/src/ai/mod.rs` — added `task_tracking: true` to 3 test `ExecutorConfig` literals
- `executor/src/health.rs` — added `task_tracking: true` to 1 test `ExecutorConfig` literal
- `mcp/src/runner.rs` — wired `task_tracking` from config to `LoopDeps` in production
- `mcp/src/init.rs` — added commented `task_tracking` line to template
- `docs/dev/milestones/M12-executor-tooling/phase-06b-task-tracking-gate.md` — status flip + Update Log
- `docs/dev/milestones/M12-executor-tooling/README.md` — phase table status flip

**New tests:**
- `executor_task_tracking_defaults_on` in `executor/src/config.rs`
- `executor_task_tracking_can_be_disabled` in `executor/src/config.rs`
- `loop_emits_no_task_updates_when_tracking_off` in `executor/src/agent/tests.rs`
- `loop_still_seeds_task_updates_when_tracking_on` in `executor/src/agent/tests.rs`

**Commits:**
- `5ce7730` — `feat: add [executor] task_tracking config gate for task-tracking substrate`

**Notes for review:** None — clean implementation, no deviations from spec.
