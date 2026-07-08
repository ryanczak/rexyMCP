# Phase 01: Consolidate escalation budget knobs (retire `escalation_slots`)

**Milestone:** M27 — Autonomous Escalation Loop
**Status:** todo
**Depends on:** none
**Estimated diff:** ~150 lines (mostly mechanical fixture-line deletions)
**Tags:** language=rust, kind=refactor, size=m

## Goal

The M27 kickoff consolidated the escalation budget on a single knob:
`[escalation] max_assists` is the per-phase autonomous assist budget consumed by
the architect-side `/rexymcp:auto` loop (a later phase). The overlapping
`[budget] escalation_slots` never gained distinct semantics and is **retired**:
removed from `BudgetConfig`, stripped from existing config files by
`rexymcp calibrate`, and deleted from the `rexymcp init` template. `max_assists`
also stops being tier-derived (it was SMALL-only): it becomes a flat,
tier-independent knob with default 3, so `calibrate` stops managing the
`[escalation]` section entirely — a user's explicit setting now survives any
re-calibrate.

No runtime behavior changes: neither knob has a production consumer today. This
is config-surface cleanup so later M27 phases build on one clearly-defined knob.

## Architecture references

Read before starting:

- `docs/architecture.md` § Status #27 — milestone context (the consolidation
  decision).
- `docs/dev/milestones/M27-autonomous-escalation-loop/README.md` § Design,
  fork 4 — the decision this phase implements.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

**`executor/src/config.rs`** — `BudgetConfig` (lines 371–404) carries the field:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetConfig {
    /// Model's context-window size in tokens.
    pub context_length: usize,
    /// % of the model's context window the loop may fill before compacting.
    pub max_context_pct: u8,
    /// Hard cap on executor turns in one phase before budget_exceeded.
    pub max_turns: u32,
    /// Escalation slots (briefings returned to the architect) per phase.
    pub escalation_slots: u32,
    ...
}
```

with `escalation_slots: 1,` in its `Default` impl (line 399). **33 test-fixture
TOML lines** in the same file's `mod tests` contain `escalation_slots = ...`;
exactly one test *asserts* on the field
(`load_parses_toml_executor_block`, line 592:
`assert_eq!(cfg.budget.escalation_slots, 2);`).

`EscalationConfig` (lines 52–67) has the stale SMALL-only doc comment:

```rust
/// SMALL-tier escalation settings. When `tier = "SMALL"`, the executor fires
/// up to `max_assists` autonomous Architect assists before hard-failing. Absent
/// or ignored for MEDIUM and LARGE tiers; consumed by the architect-side `/loop`
/// (M27), not the executor loop.
```

and the `Tier` doc comment (lines 20–22) says tier controls "whether mid-phase
Architect escalation is enabled (deferred to M27)" — stale after this phase.

**`mcp/src/calibrate.rs`** — a tier match (lines 45–57) writes `[escalation]`
for SMALL and **removes it** for MEDIUM/LARGE, plus a matching println arm
(lines 77–80). Under the new flat semantics both are wrong (a re-calibrate to
MEDIUM would delete a user's explicit `max_assists`). The `gate_retries`
stale-key strip just above it (lines 41–43) is the shape the new
`escalation_slots` strip mirrors:

```rust
    } else if let Some(budget) = doc.get_mut("budget").and_then(|b| b.as_table_mut()) {
        budget.remove("gate_retries");
    }
```

Seven calibrate test fixtures contain `escalation_slots = 1` lines.

**`mcp/src/init.rs`** — the template (line 32) emits:

```
escalation_slots = 1              # turns reserved for the final command set retry
```

(the trailing comment was wrong all along), and `init_writes_parseable_config`
(line 132) asserts `cfg.budget.escalation_slots == 1`.

**Nothing else references either knob** — verified at draft time:
`grep -rn "escalation_slots" executor/src mcp/src` hits only the three files
above; `cfg.escalation` / `max_assists` are read nowhere in production code.

## Spec

Numbered tasks in execution order.

1. **Remove the field, compiler-guided** — in `executor/src/config.rs`, delete
   from `BudgetConfig` the `escalation_slots: u32` field **and its
   `/// Escalation slots ...` doc comment**, and delete `escalation_slots: 1,`
   from `impl Default for BudgetConfig`. Then run `cargo build` /
   `cargo test --no-run`: the compiler (E0609/E0560) flags every remaining use.
   There are exactly two, both test assertions — delete each assertion line
   (keep the rest of both tests):
   - `executor/src/config.rs:592` — `assert_eq!(cfg.budget.escalation_slots, 2);`
   - `mcp/src/init.rs:132` — replace
     `assert_eq!(cfg.budget.escalation_slots, 1);` with
     `assert_eq!(cfg.escalation.max_assists, 3);` (pins that the init-written
     config, which has **no** `[escalation]` section, resolves `max_assists`
     to the default 3).

2. **Delete the fixture lines mechanically** — every test-fixture TOML line
   matching `escalation_slots` in `executor/src/config.rs` (33 lines) and
   `mcp/src/calibrate.rs` (7 lines), **except** one deliberately kept for the
   new strip test in Task 5. The sanctioned approach is one `bash` call per
   file, not 40 individual `patch` calls:

   ```bash
   sed -i '/escalation_slots = /d' executor/src/config.rs
   sed -i '/escalation_slots = /d' mcp/src/calibrate.rs
   ```

   (Task 5 re-adds the one fixture line its strip test needs.) After the sed,
   run `cargo test -p rexymcp-executor config` to confirm the config tests
   still pass — serde ignores unknown TOML keys by default, so this ordering
   (field first, fixtures second) never breaks parsing, but the fixtures must
   not keep asserting a retired reality.

3. **Rewrite the two stale doc comments** in `executor/src/config.rs`,
   verbatim:

   - `EscalationConfig` (replaces the SMALL-tier comment quoted above):

     ```rust
     /// Escalation budget for the architect-side autonomous loop
     /// (`/rexymcp:auto`, M27). `max_assists` is the number of autonomous
     /// assist round-trips (refine + re-dispatch, or resume) the loop may
     /// spend on one phase before stopping for the human. Tier-independent;
     /// consumed by the plugin skill layer, never by the executor loop.
     ```

   - `Tier` (replaces lines 20–22; the escalation clause is retired):

     ```rust
     /// Executor capability tier. Set via `rexymcp calibrate` and recorded in
     /// `[executor].tier`. Controls default `max_turns` and `gate_retries`
     /// (wired M26).
     ```

4. **`calibrate` stops managing `[escalation]`** — in `mcp/src/calibrate.rs`,
   delete the whole `// [escalation] — write only for Small ...` match block
   (lines 45–57) and the `match args.tier { Tier::Small => ", escalation.max_assists=3", _ => "" }`
   arm from the `println!` (replace that format argument with nothing — drop
   the `{}` placeholder too).

5. **`calibrate` strips the retired key on every run** — in
   `mcp/src/calibrate.rs`, after the existing `gate_retries` block, add
   (mirroring the quoted strip shape):

   ```rust
   // escalation_slots was retired in favor of [escalation] max_assists —
   // strip the stale key from configs written before the consolidation.
   if let Some(budget) = doc.get_mut("budget").and_then(|b| b.as_table_mut()) {
       budget.remove("escalation_slots");
   }
   ```

6. **Update the calibrate tests** — in `mcp/src/calibrate.rs` `mod tests`:

   - `calibrate_small_adds_escalation_section` → rename to
     `calibrate_small_does_not_write_escalation_section`; same fixture (minus
     the sed-deleted line), assertion becomes
     `assert!(doc.get("escalation").is_none());`.
   - `calibrate_medium_removes_escalation_section` → rename to
     `calibrate_preserves_user_escalation_section`; same fixture (it already
     carries an explicit `[escalation]\nmax_assists = 3`), assertion becomes
     `assert_eq!(doc["escalation"]["max_assists"].as_integer(), Some(3));`
     — **the load-bearing negative: calibrate must NOT delete a user's
     explicit section on any tier.**
   - New `calibrate_strips_retired_escalation_slots` — fixture with
     `escalation_slots = 1` inside `[budget]` (any tier); assert
     `doc.get("budget").and_then(|b| b.get("escalation_slots")).is_none()`
     after the run.
   - All other calibrate tests: unchanged apart from the sed-deleted fixture
     lines.

7. **Update the `init` template** — in `mcp/src/init.rs` `generate_config`,
   delete the `escalation_slots = 1 ...` template line, and add a commented
   `[escalation]` block between the `[governor]` section and the
   `# [models."<model-id>"]` block:

   ```
   # [escalation]
   # max_assists = 3                 # autonomous architect assists per phase (/rexymcp:auto loop)
   ```

8. **Add the back-compat unit test** — in `executor/src/config.rs`
   `mod tests`, new test `load_ignores_retired_escalation_slots_key`: write a
   TOML whose `[budget]` still contains `escalation_slots = 1` (an old config
   in the wild), `Config::load` it, assert it parses `Ok` and
   `cfg.budget.max_turns` carries the fixture value — retired keys must be
   ignored, never a parse error.

## Acceptance criteria

- [ ] `BudgetConfig` has no `escalation_slots` field;
      `grep -rn "escalation_slots" executor/src mcp/src` hits only (a) the
      calibrate strip code + its test fixture, and (b) the new back-compat
      test in `config.rs`.
- [ ] `Config::load` succeeds on a TOML whose `[budget]` still contains
      `escalation_slots = 1` (back-compat pin, Task 8).
- [ ] `rexymcp calibrate` never writes or removes an `[escalation]` section on
      any tier; a pre-existing user `[escalation] max_assists` survives
      re-calibrate to every tier.
- [ ] `rexymcp calibrate` removes a stale `escalation_slots` key from
      `[budget]` on any run.
- [ ] The `rexymcp init` template contains no `escalation_slots` and documents
      `# [escalation]` / `# max_assists = 3` as a commented block; the
      init-written config resolves `cfg.escalation.max_assists` to 3 via the
      serde default.
- [ ] The `EscalationConfig` and `Tier` doc comments match the Task-3 text.
- [ ] `cargo build` succeeds with zero new warnings.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes.
- [ ] `cargo fmt --all --check` passes.
- [ ] `cargo test` passes (existing + new tests).

## Test plan

New/changed tests (hermetic, `TempDir`):

- `load_ignores_retired_escalation_slots_key` (config.rs) — back-compat pin.
- `calibrate_small_does_not_write_escalation_section` — flat knob, no
  tier-derivation.
- `calibrate_preserves_user_escalation_section` — the must-NOT-delete negative.
- `calibrate_strips_retired_escalation_slots` — stale-key cleanup.
- `init_writes_parseable_config` — `escalation_slots` assertion replaced by the
  `max_assists` default-resolution assertion.
- `load_parses_toml_executor_block` — `escalation_slots` assertion dropped;
  test otherwise unchanged.

Unchanged behavior pins: all other `config.rs` / `calibrate.rs` / `init.rs`
tests pass with only fixture-line deletions.

## End-to-end verification

The real artifacts are the CLI behaviors. Run both against a scratch config in
a temp directory and paste the outputs in the completion Update Log:

1. `rexymcp init` into a temp dir, then grep the written file:
   `grep -c "escalation_slots" <tmp>/rexymcp.toml` → `0`, and
   `grep -c "max_assists" <tmp>/rexymcp.toml` → `1` (the commented doc line).
2. Write a minimal config containing `escalation_slots = 1` under `[budget]`
   plus an explicit `[escalation]` `max_assists = 5`; run
   `rexymcp calibrate MEDIUM --config <tmp>/rexymcp.toml`; show the resulting
   file: `escalation_slots` gone, `max_assists = 5` intact.

## Authorizations

- Editing `mcp/src/init.rs`'s template string and `mcp/src/calibrate.rs`'s
  write/strip logic is in scope (they are the knob's producers).
- Using `bash` + `sed` for the mechanical fixture-line deletions (Task 2) is
  explicitly sanctioned — preferred over per-line `patch` calls.

## Out of scope

- **Any consumer of `max_assists`.** The `/rexymcp:auto` loop that reads it is
  a later M27 phase; this phase only fixes the knob's definition.
- **This repo's own `rexymcp.toml`** (it carries `escalation_slots = 1` at the
  root). It parses fine after this phase (unknown key, ignored); the architect
  strips it at approval. Do not edit it.
- **`docs/architecture.md` § Configuration wording** — architect-maintained;
  already amended at draft time. Do not edit `architecture.md`.
- **`ArchitectConfig` / `[architect]`** — untouched; `calibrate`'s skeleton
  write for it stays exactly as is.
- **Tier semantics for `max_turns`/`gate_retries`** — untouched.

## Gotchas (pre-injected)

- **This phase is mostly mechanical multi-site churn — the known stall class.**
  Past sessions stalled re-editing dozens of similar sites one call at a time.
  Do NOT hand-edit 40 fixture lines: use the two `sed` commands in Task 2, then
  let the compiler find the assertion sites (Task 1 ordering: field first,
  build, fix the two flagged assertions, then sed).
- **serde ignores unknown TOML keys by default** — removing the struct field
  cannot make old configs fail to parse. If you see a parse failure after
  Task 1, you broke something else; do not add `#[serde(default)]` shims or a
  dummy field to "fix" it.
- **`toml_edit` vs `toml`**: `calibrate.rs` manipulates the file via
  `toml_edit::DocumentMut` (preserves comments/layout) — the strip in Task 5
  must use the quoted `get_mut`/`as_table_mut`/`remove` shape, not a
  deserialize-reserialize round-trip.
- **The `println!` edit in Task 4**: removing a format argument without
  removing its `{}` placeholder is a compile error the other direction —
  adjust the format string and the argument list together.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
