# Phase 06c-iii-a: Rewire costs + dashboard architect cost onto the ledger (per-model)

**Milestone:** M35 — Metrics & Cost Accounting Overhaul
**Status:** done
**Depends on:** phase-06c-ii
**Estimated diff:** ~380 lines
**Tags:** language=rust, kind=refactor, size=m

## Goal

Make `rexymcp costs` and the dashboard Budget panel compute **architect** cost from
the **transcript-native ledger** (06c-i), priced **per-model** via
`ArchitectLedger::cost` + `ArchitectConfig::rates_for` (06c-ii) — replacing the
stale single-rate `sum_architect_tokens(ArchitectActivity)` path (which reads
`ArchitectActivity.tokens` that harvest no longer fills).

**Two consequences of the ledger's shape, resolved with the user (2026-07-21):**
- The ledger is keyed `(project × session × model × skill)` with **no milestone**,
  so **milestone-scope architect cost is not attributable** → the Milestone column's
  **Architect and Net render `—`** (executor stays milestone-scoped from `PhaseRun`).
- Each ledger record has its **own model/rate**, so a single `ArchitectRates` can't
  price the mix; architect cost is **pre-computed per-model in `scope_costs`** and
  carried as a dollar figure.

**This is 06c-iii-a — the core rewire only.** The **per-skill breakdown** and
**harvest-freshness** displays are **06c-iii-b**. `profile` stays executor-only (the
ledger has no phase dimension). `ArchitectActivity` itself is **not** removed here —
only its cost path is retired; its `tokens` field lingers unused until 06e decides
`ArchitectActivity`'s fate (the harvest/journal deprecation question).

## Architecture references

Read before starting:

- `mcp/src/costs.rs` — the whole file (the rewire target): `ScopeReport` (18–24),
  `scope_report` (40–72), `scope_costs` (75–108), `sum_architect_tokens` (111–129,
  **to delete**), `load_cost_report` (132–225), `format_costs` (228+).
- `mcp/src/dashboard/panels.rs` — `ScopeCosts` (the struct; add a field) and
  `savings_lines` (renders `ScopeReport`; architect becomes `Option`). Its tokens-mode
  (06b-ii) reads `ScopeCosts.architect` **token** fields — leave that path working.
- `mcp/src/dashboard/mod.rs` — `load_data` (43–113): the other `scope_costs` caller.
- `executor/src/store/telemetry.rs` — `ArchitectLedger` + `ArchitectLedger::cost`
  (06c-ii), `read_architect_ledger`, `fold_ledger` (06c-i).
- `executor/src/config.rs` — `ArchitectConfig::rates_for(model) -> Option<(f64, f64)>`
  (06c-ii); **and the damaged `review_model` doc comment at config.rs:95 to restore
  (Task 6).**

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.
5. **Test placement:** tests go in the existing `#[cfg(test)] mod tests` block of the
   file under test (`costs.rs`, `panels.rs`).
6. **Editing discipline (load-bearing — prior 06c phases hard-failed here):** edit
   with `patch`/`patch_lines`, **never a whole-file `write_file`** (a ~530-line
   rewrite budget-exhausted a prior run). View with `read_file` (`start_line`/
   `end_line`), **never `sed -n`/`cat`, and never read the same region twice** (the
   identical-repetition governor hard-failed two prior runs on repeated reads). Run
   `cargo check -p rexymcp` after each file so you stay green.

## Current state

**`ScopeReport`** (costs.rs:18) — `architect` is a bare `f64`; it becomes `Option<f64>`:

```rust
pub struct ScopeReport {
    pub baseline: Option<f64>,
    pub executor: f64,
    pub architect: f64,   // -> Option<f64>
    pub net: Option<f64>,
}
```

**`scope_report`** (costs.rs:40–72) — today prices architect from a single rate
(`costs.architect.cost(&baseline.architect)`) and always computes `net`:

```rust
    let architect = costs.architect.cost(&baseline.architect);
    // ...
    let net = baseline_cost.map(|b| b - executor - architect);
```

**`scope_costs`** (costs.rs:75) + **`sum_architect_tokens`** (costs.rs:111) — sum
architect **tokens** from `ArchitectActivity` (the stale path to replace):

```rust
    let arch = sum_architect_tokens(activities, Some(project_id), milestone_id);
```

**`load_cost_report`** (costs.rs:160–206) — Session scope already has **no architect
cost** (`..Default::default()`); Project passes `milestone=None`, Milestone passes
`Some(mid)`; **the assist count** reads `activity == "assist"` (costs.rs:203, keep it):

```rust
    let assists = activities.iter()
        .filter(|a| a.project_id.as_deref() == Some(pid) && a.activity == "assist")
        .count() as u32;
```

**`load_data`** (dashboard/mod.rs:60, 73) — calls `scope_costs(&phase_runs,
&folded_activities, pid, None|Some(mid))` and derives `project_escalation_count` from
`activity == "assist"` (keep that).

**`ArchitectLedger::cost`** (06c-ii): `l.cost(input_per_mtok, output_per_mtok) -> f64`.
**`ArchitectConfig::rates_for`** (06c-ii): `Option<(f64, f64)>` (override → known → None).

## Spec

The end state: architect cost is computed **once**, per-model, in `scope_costs` (which
gains ledger + `&ArchitectConfig` access) and stored as `ScopeCosts.architect_cost:
Option<f64>`; `scope_report` just passes it through; Milestone scope → `None` → `—`.

### Task 1 — `ScopeCosts` gains `architect_cost` (panels.rs)

Add to the `ScopeCosts` struct: `pub architect_cost: Option<f64>` (its `#[derive(Default)]`
gives `None`). Keep the existing `architect: ArchitectTokens` (aggregate tokens — the
tokens-mode display still uses it).

### Task 2 — `ScopeReport.architect` → `Option<f64>` + `scope_report` (costs.rs)

Change the field to `pub architect: Option<f64>`. Rewrite the architect/net lines:

```rust
    let architect = costs.architect_cost;
    // baseline_cost unchanged (executor input/output at baseline rate)
    let net = match (baseline_cost, architect) {
        (Some(b), Some(a)) => Some(b - executor - a),
        _ => None,
    };
```

(`executor` is unchanged; `baseline` unchanged.) The `baseline.architect` field is no
longer read by `scope_report` — leave `BudgetRates`/`ArchitectRates` as they are (still
used elsewhere).

### Task 3 — `scope_costs` prices architect from the ledger, per-model (costs.rs)

New signature (drop `activities`, add ledgers + architect config):

```rust
pub(crate) fn scope_costs(
    runs: &[PhaseRun],
    ledgers: &[telemetry::ArchitectLedger],
    architect: &rexymcp_executor::config::ArchitectConfig,
    project_id: &str,
    milestone_id: Option<&str>,
) -> ScopeCosts {
    // executor: unchanged — sum PhaseRun tokens by project (+ milestone filter).
    // ... existing executor fold ...

    // architect: attributable at PROJECT scope only (the ledger has no milestone).
    let (architect_tokens, architect_cost) = if milestone_id.is_some() {
        (ArchitectTokens::default(), None) // milestone → "—"
    } else {
        let mut toks = ArchitectTokens::default();
        let mut cost = 0.0_f64;
        for l in ledgers.iter().filter(|l| l.project_id.as_deref() == Some(project_id)) {
            toks.input = toks.input.saturating_add(l.tokens.input);
            toks.cache_creation = toks.cache_creation.saturating_add(l.tokens.cache_creation);
            toks.cache_read = toks.cache_read.saturating_add(l.tokens.cache_read);
            toks.output = toks.output.saturating_add(l.tokens.output);
            if let Some((inp, out)) = architect.rates_for(&l.model) {
                cost += l.cost(inp, out);
            }
        }
        (toks, Some(cost))
    };

    ScopeCosts {
        executor_in: exec.executor_in,
        executor_out: exec.executor_out,
        executor_cache_read: exec.executor_cache_read,
        executor_cache_write: exec.executor_cache_write,
        architect: architect_tokens,
        architect_cost,
    }
}
```

**Delete `sum_architect_tokens`** (costs.rs:111–129) — the ledger path replaces it.

### Task 4 — update the two callers (costs.rs `load_cost_report`, dashboard `load_data`)

Both must now: (a) read the **ledger** (`read_architect_ledger` + `fold_ledger`) in
addition to activities, and (b) pass `ledgers` + `&cfg.architect` to `scope_costs`.
**Keep the `activities` read solely for the assist count** (`activity == "assist"`).

- **`load_cost_report`** (costs.rs): after reading `runs`/`activities`, add
  `let ledgers = telemetry::fold_ledger(telemetry::read_architect_ledger(&telemetry_file).unwrap_or_default());`
  and change the two `scope_costs(&runs, &activities, pid, …)` calls to
  `scope_costs(&runs, &ledgers, &cfg.architect, pid, …)`.
- **`load_data`** (dashboard/mod.rs): add an `architect: &ArchitectConfig` **parameter**
  (thread it from `main.rs`'s Dashboard arm, which has `cfg`), read the ledger the same
  way, and update both `scope_costs(...)` calls. Keep `folded_activities` +
  `project_escalation_count` exactly as they are.

### Task 5 — render `—` for a `None` architect (costs.rs `format_costs`, panels.rs `savings_lines`)

`ScopeReport.architect` is now `Option<f64>`. Render it with the **same `fmt_opt`
`—`-when-`None`** treatment `baseline`/`net` already use.
- `format_costs` (costs.rs): the ARCHITECT column uses `fmt_opt` (not `fmt_dollars`).
- `savings_lines` (panels.rs): the Architect **dollars-mode** cell uses the `None → —`
  path (mirror how it renders `baseline`/`net`). **Tokens-mode is unchanged** — it
  still sums `ScopeCosts.architect.{input,cache_creation,cache_read,output}`.

### Task 6 — restore the `review_model` doc comment (config.rs)

The 06c-ii patch dropped the first line of `review_model`'s doc (config.rs:95). Restore
it to a two-line doc, e.g.:

```rust
    /// Model ID for review subagent delegation. `None` means inherit the
    /// session/architect model (not `[architect] model`).
    pub review_model: Option<String>,
```

## Acceptance criteria

- [ ] `rexymcp costs` and the dashboard Budget compute **Project** architect cost from
      the ledger, **per-model** (`ArchitectLedger::cost` × `rates_for(model)` summed),
      not from `ArchitectActivity.tokens`.
- [ ] **Milestone-scope Architect and Net render `—`** (not `$0.00`); executor cost is
      still milestone-scoped. Session architect stays `—` (unchanged).
- [ ] `sum_architect_tokens` is deleted; `scope_costs` takes `ledgers` + `&ArchitectConfig`.
- [ ] The **assist count** (`activity == "assist"`) is unchanged in both surfaces.
- [ ] `ScopeReport.architect` is `Option<f64>`; `format_costs`/`savings_lines` render
      `—` for `None`. Dashboard **tokens-mode** architect still shows summed tokens.
- [ ] The `review_model` doc comment is restored to two lines.
- [ ] `cargo fmt --all --check`, `cargo build`,
      `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test` all pass.
- [ ] `profile` and `executor/src/**` (except the config.rs doc restore) are untouched.

## Test plan

In `mcp/src/costs.rs` `mod tests`:

- **`scope_costs_prices_architect_per_model_from_ledger`** — build `runs` (one project
  PhaseRun) + two `ArchitectLedger` records for the project with **different models**
  (e.g. `claude-opus-4-8` and `claude-sonnet-5`), each with known tokens; call
  `scope_costs(&runs, &ledgers, &architect_cfg, pid, None)` with a default
  `ArchitectConfig` (built-in rates); assert `architect_cost == Some(x)` where `x` is
  the hand-sum of `opus_ledger.cost(5,25) + sonnet_ledger.cost(2,10)`.
  **Mutation-sensitive:** must fail if both are priced at one model's rate.
- **`scope_costs_milestone_architect_is_none`** — same inputs, call with
  `milestone_id = Some("M35-…")`; assert `architect_cost == None` (and executor cost is
  still summed for that milestone's runs).
- **`scope_report_none_architect_yields_dash_and_no_net`** — a `ScopeCosts` with
  `architect_cost = None` and a baseline set → `scope_report(...).architect == None`
  and `.net == None`.
- **`scope_report_some_architect_priced`** — `architect_cost = Some(3.0)`, baseline set,
  executor priced → `.architect == Some(3.0)` and `.net == Some(baseline - exec - 3.0)`.

In `mcp/src/dashboard/panels.rs` `mod tests`:

- **`savings_lines_architect_none_renders_dash`** — a `ScopeCosts` with `architect_cost
  = None` (e.g. the milestone scope) → the Architect cell in **dollars** mode renders
  `—`, not `$0.00`. (Reuse the existing `savings_lines` test scaffolding.)
- Update the existing `savings_lines`/`scope_report` tests that construct `ScopeCosts`
  or read `ScopeReport.architect` for the new `architect_cost` field / `Option` type
  (behaviour-preserving where they used a priced architect — set `architect_cost =
  Some(...)`).

## End-to-end verification

Run `rexymcp costs` against the **real** telemetry store (the harvested ledger from
06c-i is on disk) and quote the table in the completion Update Log: confirm the
**Project ARCHITECT** column shows a non-`—`, non-`$0.00` dollar figure (per-model
priced — opus + sonnet + fable mix), and the **Milestone ARCHITECT + NET** columns show
`—`. If the live store has no priced architect models, note it and fall back to the
`scope_costs_prices_architect_per_model_from_ledger` unit assertion as the artifact
check. Do **not** hand-edit the store.

## Authorizations

- Editing `mcp/src/costs.rs`, `mcp/src/dashboard/panels.rs`, `mcp/src/dashboard/mod.rs`,
  `mcp/src/dashboard/render.rs` (only if a signature it calls changed), `mcp/src/main.rs`
  (the Dashboard arm threading `&cfg.architect` into `load_data`), and the **doc-comment
  restore in `executor/src/config.rs`** is in scope.
- **No new dependencies.**
- No `docs/architecture.md`/`STANDARDS.md`/`WORKFLOW.md`/other-phase-doc edits.
- No `unsafe`, no `#[allow]`, no new `unwrap`/`expect`/`panic!` in production paths.

## Out of scope

- **Per-skill breakdown** and **harvest-freshness** display — **06c-iii-b**.
- **`profile`** — stays executor-only (the ledger has no phase key). Do not touch it.
- **Removing `ArchitectActivity` or its `tokens` field** — only its *cost path* is
  retired here (the `sum_architect_tokens` deletion). The struct + `journal` CLI stay;
  06e decides their fate. The **assist count still reads `ArchitectActivity`**.
- **Session-scope architect from the ledger** — the session-log session id ≠ the
  transcript session id, so Session architect stays `—` (as today). Do not attempt to
  map them.
- **Milestone timestamp-bracketing** — the user chose `—` for milestone architect; do
  not bracket ledgers into milestones.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
<!-- entries appended below this line -->
### Update — 2026-07-21 (escalation → session takeover)

**Chosen lever:** session takeover (to finish + repair).
**Rationale:** the executor (AEON-7, 268 turns) got the **core rewire correct**
(`scope_report`/`scope_costs` per-model, milestone→None) but **hard-failed on
`IdenticalToolCallRepetition` (bash `sed -n` ×6)** — the **third same-class hard_fail
of the 06c arc** (06c-i ×2), *despite the read-once/`sed`-avoidance gotcha being in
this phase doc*. It left the tree non-compiling with a production regression +
extensive test-fixup mangles. Re-dispatch would loop again on the same class; the work
was ~90% done. Architect took over.

### Review verdict — 2026-07-21

- **Verdict:** escalated (session takeover)
- **Bounces:** 1 hard_fail (`IdenticalToolCallRepetition`/bash, 268 turns) → takeover.
- **Executor:** AEON-7/Qwen3.6-27B-AEON (core rewire) + Claude Code (direct) for repair/finish.
- **Takeover work:** (1) **production regression** — the executor replaced
  `savings_lines`' session `ScopeCosts` with `::default()`, zeroing Session executor
  tokens (restored from `sess_in`/`sess_out`); (2) **dead field** — `BudgetRates.architect`
  became unread once `scope_report` stopped single-rate pricing → removed the field +
  all literals (clippy `-D warnings` blocker); (3) **debit_row** now hides `—`-everywhere
  rows (a `None` architect) as it already hid `$0.00`; (4) fixed ~15 mangled test literals
  (the executor repeatedly deleted `let lines = savings_lines(` and unindented while
  adding `architect_cost`), converted `load_data`'s test to seed an `ArchitectLedger`
  (was an `ArchitectActivity`), and updated `scope_report` tests for the `Option<f64>`
  architect; (5) **added the 2 missing mutation-sensitive tests** the executor skipped
  (`scope_costs_prices_architect_per_model_from_ledger`, `scope_costs_milestone_architect_is_none`).
- **Verification:** all four gates green (581 mcp + 1031 executor). Per-model pricing
  **mutation-verified** (pricing all at opus → $60 ≠ the $42 opus+sonnet-5 expected).
  **E2E** (`rexymcp costs` on the real store): Project ARCHITECT **$1432.00** (per-model),
  Session + Milestone ARCHITECT/NET **`—`** — the design decisions confirmed live.
  `costs.rs`/`profile` scope respected (profile untouched).
- **Calibration (STRONG — reinforced 3×):** `IdenticalToolCallRepetition` on read-only
  `sed -n` loops is now a **confirmed recurring executor failure the phase-doc gotcha does
  NOT prevent** (06c-i ×2 + 06c-iii-a, the gotcha present each time). This is past the
  three-strikes threshold: **fold candidate for M35 close** — make the identical-repetition
  detector **advisory (or higher-threshold) for read-only commands** (`sed -n`/`cat`/`grep`
  that don't mutate), and/or a harder executor-contract steer off `sed -n` for viewing.
  Also: the executor **skips architect-specified new tests** under stress (2nd occurrence,
  after 06c-i) and **mangles adjacent test literals** during multi-literal struct-field
  additions — a green-at-every-step / one-literal-per-patch nudge may help.
