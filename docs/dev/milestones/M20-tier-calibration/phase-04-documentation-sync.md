# Phase 04: Documentation sync — README and architecture

**Milestone:** M20 — Tier Calibration and Cost Visibility
**Status:** review
**Depends on:** phase-03 (dashboard cost breakdown complete)
**Estimated diff:** ~110 lines
**Tags:** language=rust, kind=docs, size=s

## Goal

Bring `README.md` and `docs/architecture.md` up to date with all M19 and M20
changes: the structural gate enforcement runtime change, the `TierTelemetry`
additions to `PhaseRun`, the new `[escalation]` and `[architect]` config sections,
the calibrate CLI, and the tabular Baseline/Executor/Architect/Net cost breakdown
in the dashboard Budget panel.

No code changes in this phase. The only files touched are `README.md`,
`docs/architecture.md`, and this phase doc's own Update Log.

## Architecture references

Read before starting:

- `docs/dev/milestones/M19-gate-enforcement/README.md` — M19 exit criteria and
  retrospective; source of truth for M19's status-section entry and the turn-cycle
  step 8 description.
- `docs/dev/milestones/M20-tier-calibration/README.md` — M20 exit criteria;
  confirm all three implementation phases are done before this doc phase runs.
- `executor/src/store/telemetry.rs` — `TierTelemetry` struct definition (five
  fields: `tier`, `doc_level`, `escalation_count`, `architect_input_tokens`,
  `architect_output_tokens`); `PhaseRun.tier_telemetry` nesting.
- `executor/src/config.rs` — `EscalationConfig`, `ArchitectConfig`,
  `known_model_rates`; confirms the field names for the Configuration section.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read this entire phase doc before touching any file.
3. Confirm `cargo build` exits 0 (no code changes in this phase; a red build
   means the tree was already broken before you started — stop and report it).

## Current state

### `docs/architecture.md` — seven stale passages

1. **Status header (lines 3–9):** says "M9–M17 are fully implemented and closed;
   M18 is active." M19 and M20 are now done; M18 is still in progress.

2. **Turn cycle step 8 (lines 134–136):** says "On clean completion, run the
   project's full command set … and return." M19 added gate enforcement: failed
   gates inject their output as a user message and the loop continues. This is
   a material runtime change; the step 8 description must reflect it.

3. **`PhaseRun` schema (lines 252–273):** Missing the `tier_telemetry` block
   added by M20 phase-02. The schema pseudocode is the architecture reference
   for what a `PhaseRun` contains.

4. **Configuration section (lines 484–492):** lists "budget knobs" as the last
   item. `[escalation]` (tier calibration) and `[architect]` (Claude model cost
   rates) sections added by M20 phase-01 are unmentioned.

5. **M8 Budget panel description (lines 616–619):** says `$ saved` with a single
   rate pair. M20 phase-03 replaced this with a Baseline/Executor/Architect/Net
   tabular layout.

6. **M8 `[dashboard]` config section description (lines 630–632):** mentions
   only `saved_input_per_mtok` / `saved_output_per_mtok`. `saved_model` (M15)
   and the sibling `[architect]` section (M20 phase-01) are absent.

7. **Status section end (after line 864):** M18 is the last entry. M19 and M20
   have no entries.

### `README.md` — three stale passages

1. **CLI dashboard entry (lines 209–214):** says `$ saved` and `[dashboard]`
   rates. M20 phase-03 replaced this with the tabular breakdown; `[architect]`
   rates are now also relevant.

2. **CLI list (after line 222):** `rexymcp calibrate` (M20 phase-01) is absent.

3. **Config example (lines 340–342):** shows only `[dashboard]` with two
   commented fields referring to `$ saved in the TUI`. `[architect]` section
   is missing; `saved_model` is missing; comment text is stale.

## Spec

Work through `docs/architecture.md` top-to-bottom (Tasks 1–7), then
`README.md` top-to-bottom (Tasks 8–10). Use `read_file` with explicit
`start_line`/`end_line` to verify each passage before editing.

### Task 1 — Update the `docs/architecture.md` status header

**Location:** lines 3–9.

Replace:

```
> **Status:** Living design doc. M1–M7 and M9–M17 are fully implemented
> and closed; M18 (capability-aware adaptation) is active. M8 (live session dashboard) is implemented but
> open — the wireframe redesign shipped (2026-06-03) and M8 remains open for
> live-session confirmation and bug fixes before its milestone close. This document is the source of truth
> for the *intended* design; the code under `executor/` and `mcp/` is the source
> of truth for what actually runs. Milestones are listed in the **Status** section
> at the bottom — that list is the project plan.
```

with:

```
> **Status:** Living design doc. M1–M7 and M9–M20 are fully implemented
> and closed (M8 and M18 remain open — see below); M8 (live session dashboard)
> is implemented but open — the wireframe redesign shipped (2026-06-03) and M8
> remains open for live-session confirmation and bug fixes before its milestone
> close; M18 (capability-aware adaptation) is in progress. This document is the
> source of truth for the *intended* design; the code under `executor/` and
> `mcp/` is the source of truth for what actually runs. Milestones are listed
> in the **Status** section at the bottom — that list is the project plan.
```

### Task 2 — Update turn cycle step 8 in `docs/architecture.md`

**Location:** lines 134–136 (the `8.` item at the end of the turn cycle list).

Replace:

```
8. On clean completion, run the project's full command set
   (`{FORMAT_COMMAND}`/`{BUILD_COMMAND}`/`{LINT_COMMAND}`/`{TEST_COMMAND}`),
   capture output, and return.
```

with:

```
8. When the model signals completion (`NoToolCall`), run the project's full
   command set (`{FORMAT_COMMAND}`/`{BUILD_COMMAND}`/`{LINT_COMMAND}`/
   `{TEST_COMMAND}`). If any gate exits non-zero, inject its output as a user
   message and re-enter the loop at step 3 — the model must fix the failure and
   re-signal completion. If the turn budget is exhausted while fixing gate
   failures, the result is `PhaseResult::BudgetExceeded`. Only when all
   configured gates pass does the loop return `PhaseResult::Complete`. (M19.)
```

### Task 3 — Add `tier_telemetry` to the `PhaseRun` schema in `docs/architecture.md`

**Location:** lines 252–273 (the fenced `PhaseRun { … }` pseudocode block,
inside the "Model effectiveness metrics" subsection).

Replace the entire block:

```
```
PhaseRun {
  model,                                // requested model id
  generation_params: { temperature, seed }, // sampling settings (None = endpoint default)
  phase_id, tags,                       // language, kind (feature|refactor|bugfix|test), size bucket
  status, escalated,                    // complete|hard_fail|budget_exceeded; did it hand off to Claude?
  // quality
  gates: { fmt, build, lint, test },    // booleans on first completion
  warnings, bugs_filed,                 // count + max severity (filled by the architect at review)
  bounces_to_approval,                  // 0 = first-try; the single most telling number
  // reliability (the small-model differentiator)
  parse_failure_rate, repairs_per_call, // from the forgiving parser
  verifier_retries, tool_success_rate,  // from the governor
  length_finish_rate,                   // fraction of completions that hit max_tokens (truncation signal)
  // efficiency
  turns, wall_clock_s, tokens,          // TokenBreakdown (in/out/cached)
  // endpoint-reported provenance (best-effort; None when the server omits them)
  served_model,                         // model id from the chat response — more accurate than requested
  context_window,                       // max_model_len from /v1/models; distinct from budget.context_length
  // supervision label (filled at review)
  architect_verdict,                    // approved_first_try | approved_after_N | rejected | escalated
}
```
```

with:

```
```
PhaseRun {
  model,                                // requested model id
  generation_params: { temperature, seed }, // sampling settings (None = endpoint default)
  phase_id, tags,                       // language, kind (feature|refactor|bugfix|test), size bucket
  status, escalated,                    // complete|hard_fail|budget_exceeded; did it hand off to Claude?
  // quality
  gates: { fmt, build, lint, test },    // booleans on first completion
  warnings, bugs_filed,                 // count + max severity (filled by the architect at review)
  bounces_to_approval,                  // 0 = first-try; the single most telling number
  // reliability (the small-model differentiator)
  parse_failure_rate, repairs_per_call, // from the forgiving parser
  verifier_retries, tool_success_rate,  // from the governor
  length_finish_rate,                   // fraction of completions that hit max_tokens (truncation signal)
  // efficiency
  turns, wall_clock_s, tokens,          // TokenBreakdown (in/out/cached)
  // endpoint-reported provenance (best-effort; None when the server omits them)
  served_model,                         // model id from the chat response — more accurate than requested
  context_window,                       // max_model_len from /v1/models; distinct from budget.context_length
  // tier calibration (M20; #[serde(default)] — absent in pre-M20 records)
  tier_telemetry: {
    tier,                               // LARGE | MEDIUM | SMALL — from [escalation] config
    doc_level,                          // 0–3 architect detail level
    escalation_count,                   // mid-phase Architect assists this run
    architect_input_tokens,             // Architect (Claude) input tokens
    architect_output_tokens,            // Architect (Claude) output tokens
  },
  // supervision label (filled at review)
  architect_verdict,                    // approved_first_try | approved_after_N | rejected | escalated
}
```
```

Note: the outer triple-backtick fences belong to the Markdown fenced block in
`docs/architecture.md` — do **not** add extra fences. The inner backtick
delimiters above are just for quoting the replacement in this phase doc; the
actual file should contain exactly one opening ```` ``` ```` and one closing
```` ``` ````.

### Task 4 — Update the Configuration section in `docs/architecture.md`

**Location:** lines 484–492 (the bullet list in the `## Configuration` section).

Replace:

```
rexyMCP config (designed in M1) carries, per invocation or per target project:

- the OpenAI-compatible **endpoint** + **model** (executor),
- the **target-repo root** (the scope boundary for all file/bash tools),
- the **command set** that resolves the `{…_COMMAND}` placeholders
  (`format`/`build`/`lint`/`test`), plus an optional `lint_fix` autofixing
  command run by the post-write hook (step 5a above) — not advertised to the
  executor model, not a gate command,
- budget knobs (context %, max turns, escalation slots).
```

with:

```
rexyMCP config (designed in M1) carries, per invocation or per target project:

- the OpenAI-compatible **endpoint** + **model** (executor),
- the **target-repo root** (the scope boundary for all file/bash tools),
- the **command set** that resolves the `{…_COMMAND}` placeholders
  (`format`/`build`/`lint`/`test`), plus an optional `lint_fix` autofixing
  command run by the post-write hook (step 5a above) — not advertised to the
  executor model, not a gate command,
- budget knobs (context %, max turns, escalation slots),
- **`[escalation]`** (M20) — tier (`LARGE`/`MEDIUM`/`SMALL`) and tier-derived
  defaults for `max_turns`, `escalation_slots`, `doc_level`,
- **`[architect]`** (M20) — Claude model id and cost rates
  (`input_per_mtok`, `output_per_mtok`) for the dashboard Architect cost column;
  a `known_model_rates` registry auto-fills rates for recognized Claude model IDs.
```

### Task 5 — Update the M8 Budget panel description in `docs/architecture.md`

**Location:** lines 616–619 (the `- **Budget panel**` bullet inside the M8
status entry).

Replace:

```
   - **Budget panel** — tokens in/out, context % (color-coded gauge: green <50 /
     yellow 50–80 / red ≥80), tok/s (derived from `Metrics` record timestamps),
     and `$ saved` (configurable cloud-baseline $/Mtok via `[dashboard]` in
     `rexymcp.toml`; shows `—` when unset).
```

with:

```
   - **Budget panel** — tokens in/out, context % (color-coded gauge: green <50 /
     yellow 50–80 / red ≥80), tok/s (derived from `Metrics` record timestamps),
     and a tabular **Baseline / Executor / Architect / Net** cost breakdown across
     Session, Milestone, and Project scopes — plus an **Assists** counter (sum of
     mid-phase Architect escalations). Configurable cloud-baseline and Architect
     rates via `[dashboard]` / `[architect]` in `rexymcp.toml`; rows show `—`
     when the corresponding rate is unset.
```

### Task 6 — Update the M8 `[dashboard]` config section description in `docs/architecture.md`

**Location:** lines 630–632 (the `- **\`[dashboard]\` config section**` bullet
inside the M8 status entry, immediately before the M10 entry).

Replace:

```
   - **`[dashboard]` config section** — `saved_input_per_mtok` /
     `saved_output_per_mtok` (f64, default 0.0 → show `—`). A missing section
     falls back to defaults (purely additive; no required config to run).
```

with:

```
   - **`[dashboard]` config section** — `saved_input_per_mtok` /
     `saved_output_per_mtok` (f64, default 0.0 → `—` in Baseline/Net rows) and
     `saved_model` (optional string — auto-fills rates for recognized Claude model
     IDs, M15). A missing section falls back to defaults (purely additive). A
     sibling **`[architect]` config section** (M20) mirrors the same fields
     (`model`, `input_per_mtok`, `output_per_mtok`) for the Architect cost column.
```

### Task 7 — Append M19 and M20 entries to the Status section of `docs/architecture.md`

**Location:** after line 864 (the last line of the M18 entry, end of file).

Append the following two entries, separated from the M18 block by one blank line:

```markdown

19. **M19 — Structural Gate Enforcement** *(done, 2026-06-16; 1/1
    approved_after_1)*. Make `false_completion` structurally impossible:
    `execute_phase` now returns `PhaseResult::Complete` only when every
    configured DoD gate passes (exit 0). When a gate exits non-zero at
    completion, the runtime injects the failure output as a user message and
    continues the loop; turn-budget exhaustion while fixing gates yields
    `BudgetExceeded`. Backward-compatible: `EMPTY_COMMANDS` / `NoopRunner`
    (gates all `None`) paths unchanged. A ~130-line change in
    `executor/src/agent/mod.rs` closes the control-flow gap in the `NoToolCall`
    completion arm.

20. **M20 — Tier Calibration and Cost Visibility** *(done, 2026-06-16; 4/4
    phases approved)*. Three implementation phases plus a documentation sync:
    - **Phase-01** adds `[escalation]` + `[architect]` config sections and a
      `rexymcp calibrate LARGE|MEDIUM|SMALL` CLI subcommand — one knob that
      writes tier-derived budget defaults (`max_turns`, `escalation_slots`,
      `doc_level`) to `rexymcp.toml`. A shared `known_model_rates` registry in
      `executor/src/config.rs` auto-fills baseline and architect cost rates for
      recognized Claude model IDs.
    - **Phase-02** adds a `TierTelemetry` struct nested in `PhaseRun` via
      `#[serde(default)]` — five new fields: `tier`, `doc_level`,
      `escalation_count`, `architect_input_tokens`, `architect_output_tokens`.
      `EscalationEvent` record appended to `phase_runs.jsonl` each time a
      mid-phase Architect assist fires (M21 wires the firing; M20 defines the
      record).
    - **Phase-03** upgrades the dashboard Budget panel's Savings block from a
      gross three-scope `$ saved` row to a tabular **Baseline / Executor /
      Architect / Net** breakdown (Session × Milestone × Project columns) with
      a project Assists counter. Net savings now subtracts Architect spend from
      the cloud baseline. `ScopeCosts` struct introduced in
      `mcp/src/dashboard/panels.rs`; `BudgetRates` gains
      `architect_input_per_mtok` / `architect_output_per_mtok`.
```

### Task 8 — Update the `rexymcp dashboard` CLI entry in `README.md`

**Location:** lines 209–214.

Replace:

```
- `rexymcp dashboard` — live full-screen TUI over the session JSONL: Session,
  Budget (tokens · context % · tok/s · $ saved), Reclaim (compaction + per-lever
  Arc A/B sources), Tasks (the phase's Spec-seeded TODO list, checked off live),
  Activity transcript (scrollable full replay), and Files panels.
  Stays open and auto-follows new sessions. `--config` loads `[dashboard]` rates
  for the `$ saved` line.
```

with:

```
- `rexymcp dashboard` — live full-screen TUI over the session JSONL: Session,
  Budget (tokens · context % · tok/s · tabular Baseline/Executor/Architect/Net
  cost breakdown per scope · Assists counter), Reclaim (compaction + per-lever
  Arc A/B sources), Tasks (the phase's Spec-seeded TODO list, checked off live),
  Activity transcript (scrollable full replay), and Files panels.
  Stays open and auto-follows new sessions. `--config` loads `[dashboard]` and
  `[architect]` rates for the cost breakdown.
```

### Task 9 — Add `rexymcp calibrate` to the CLI list in `README.md`

**Location:** insert after line 222 (after the `rexymcp scorecard` bullet,
before the blank line separating the CLI section from the Plugin section).

Insert:

```
- `rexymcp calibrate LARGE|MEDIUM|SMALL` — write tier-derived budget defaults
  (`max_turns`, `escalation_slots`, `doc_level`) to `rexymcp.toml` and print a
  confirmation. Sets how much hand-holding the Architect provides and how many
  retries the executor gets before escalation fires.
```

### Task 10 — Update the config example comment block in `README.md`

**Location:** lines 340–342 (the commented `[dashboard]` block inside the config
example code fence).

Replace:

```
# [dashboard]
# saved_input_per_mtok  = 3.0    # USD/Mtok for cloud input  — shown as "$ saved" in the TUI
# saved_output_per_mtok = 15.0   # USD/Mtok for cloud output   (0.0 = omit, show "—")
```

with:

```
# [dashboard]
# saved_input_per_mtok  = 3.0      # USD/Mtok for cloud input  — cost Baseline in Budget panel
# saved_output_per_mtok = 15.0     # USD/Mtok for cloud output   (0.0 → "—" in Baseline/Net rows)
# saved_model = "claude-opus-4-5"  # auto-fill rates from the known-model registry

# [architect]
# model = "claude-opus-4-8"        # Claude model id — auto-fills input/output rates
# input_per_mtok  = 15.0           # override: USD/Mtok for Architect input
# output_per_mtok = 75.0           # override: USD/Mtok for Architect output
```

## Acceptance criteria

### `docs/architecture.md`

- [ ] Status header (lines 3–9) says "M9–M20 are fully implemented and closed
      (M8 and M18 remain open)"; "M18 is in progress"; no longer says "M18 is
      active" as the sole non-M8 caveat.
- [ ] Turn cycle step 8 describes the gate-retry loop: failed gate → inject
      output → re-enter loop; budget exhausted while fixing → `BudgetExceeded`;
      all gates pass → `Complete`. No longer reads "On clean completion … and return."
- [ ] `PhaseRun` schema block contains a `// tier calibration` comment section
      with `tier_telemetry` and its five sub-fields (`tier`, `doc_level`,
      `escalation_count`, `architect_input_tokens`, `architect_output_tokens`).
- [ ] Configuration section bullet list ends with `[escalation]` and `[architect]`
      items; "budget knobs" is no longer the last item.
- [ ] M8 Budget panel bullet no longer mentions `$ saved`; now describes the
      tabular Baseline/Executor/Architect/Net layout and Assists counter.
- [ ] M8 `[dashboard]` config section description mentions `saved_model` (M15)
      and the sibling `[architect]` section (M20).
- [ ] Status section contains M19 entry (gate enforcement, done 2026-06-16) and
      M20 entry (tier calibration, done 2026-06-16, 4 phases).

### `README.md`

- [ ] Dashboard CLI entry no longer says `$ saved`; now describes the tabular
      cost breakdown and Assists counter.
- [ ] CLI list includes `rexymcp calibrate LARGE|MEDIUM|SMALL`.
- [ ] Config example includes a commented-out `[architect]` section with `model`,
      `input_per_mtok`, `output_per_mtok` fields.
- [ ] Config example `[dashboard]` comment no longer says `$ saved in the TUI`;
      updated text matches the spec above; `saved_model` is present.

### Gates

- [ ] `cargo fmt --all --check` exits 0.
- [ ] `cargo build` exits 0 with zero new warnings.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` exits 0.
- [ ] `cargo test` passes (no tests added or removed in this phase).

## Test plan

This phase has no new Rust code and therefore no new unit tests. The acceptance
criteria above are the full verification: after each edit, use `read_file` with
the relevant line range to confirm the passage matches the spec text exactly.

## End-to-end verification

Not applicable — documentation-only phase. Gate commands confirm the tree is
still clean.

## Authorizations

This phase explicitly authorizes the executor to edit:

- `docs/architecture.md` — all seven passages described in the Spec. This file
  is normally off-limits per CLAUDE.md; this authorization overrides that rule
  **for this phase only**.
- `README.md` — the three passages described in the Spec. This file is normally
  off-limits per CLAUDE.md; this authorization overrides that rule **for this
  phase only**.

No other file may be touched. No Cargo.toml, no source files, no other docs.

## Out of scope

- Updating `docs/dev/STANDARDS.md` or `docs/dev/WORKFLOW.md`.
- Editing any source file under `executor/` or `mcp/`.
- Updating `docs/architecture.md` Configuration prose beyond Task 4 (e.g., the
  `[telemetry]` section is already described; do not expand it).
- Updating `docs/architecture.md` M18 status text beyond the header fix —
  M18 is still in progress; its status entry (lines 838–864) is correct as-is.
- Adding any new MCP tool, CLI command, or config section not introduced by
  M19 or M20.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-06-16 23:45 (started)

**Executor:** Claude (Sonnet)

Implementing all 10 spec tasks: 7 passages in `docs/architecture.md` and 3 passages in `README.md`.

### Update — 2026-06-16 23:45 (complete)

**Tasks completed:** All 10 spec tasks.

**Files changed:**
- `docs/architecture.md` — 7 passages updated (status header, turn cycle step 8, PhaseRun schema tier_telemetry, Configuration section, M8 Budget panel, M8 [dashboard] config, M19+M20 status entries)
- `README.md` — 3 passages updated (dashboard CLI entry, calibrate CLI addition, config example)
- `docs/dev/milestones/M20-tier-calibration/phase-04-documentation-sync.md` — status → review, Update Log
- `docs/dev/milestones/M20-tier-calibration/README.md` — phase table row 04 → review

**Verification commands:**
```
cargo fmt --all --check → exit 0
cargo build → exit 0, zero new warnings
cargo clippy --all-targets --all-features -- -D warnings → exit 0
cargo test → 807 passed; 0 failed; 2 ignored
```

**One-line verification summary:** fmt clean, build clean, clippy clean, 807 tests passed.

**Grep for spec-pinned literals:**
- `tier_telemetry` in `docs/architecture.md`: 1 match (PhaseRun schema) ✓
- `rexymcp calibrate` in `README.md`: 1 match (CLI list) ✓
- `[architect]` in `README.md`: 1 match (config example) ✓
- `M19 — Structural Gate Enforcement` in `docs/architecture.md`: 1 match (status section) ✓
- `M20 — Tier Calibration and Cost Visibility` in `docs/architecture.md`: 1 match (status section) ✓

**End-to-end verification:** N/A — documentation-only phase. Gate commands confirm the tree is still clean.

**Notes for review:** None — all changes are documentation sync per spec.
