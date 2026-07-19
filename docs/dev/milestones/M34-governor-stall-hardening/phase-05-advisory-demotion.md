# Phase 05: Advisory-demotion of the novelty stall detector

**Milestone:** M34 — Governor Stall Hardening
**Status:** todo
**Depends on:** phase-03 (`check_low_novelty_stall`), phase-04 (the `NoveltySample`
stream that becomes the advisory record)
**Estimated diff:** ~140 lines
**Tags:** language=rust, kind=feature, size=m

## Goal

Stop the `LowNoveltyStall` detector from **terminating** runs by default. It
becomes **advisory**: the measurement is still recorded (phase-04's
`NoveltySample`), but a low-novelty window no longer hard-fails the run. A new
`[governor] novelty_action` switch (`advisory` default / `terminate`) makes the
behavior legible and reversible. The raw `NoProgressStall` (`read_only_stall_threshold`,
default 60) **stays terminating** as the conservative high backstop, and
`max_turns` / the optional `wall_clock_secs` remain the real ceilings.

## Why

rexyMCP must not introduce failure modes on arbitrary, un-backed thresholds. The
novelty defaults (`novelty_window = 24`, `novelty_distinct_floor = 6`) were chosen
by judgment, not data. phase-04's own dispatch (`fc515cd4`) proved the cost: the
detector hard-failed the run at **turn 104 of a 600-turn budget — 82% unused** —
by flagging bursts of 24–59 read-only calls over ~6 files as "churn," when the
executor was navigating a large test file. The aggressive early catch (24-window)
misfires; the conservative backstop (60 consecutive read-only) does not, and it
**stays** — so a genuine pure-read verify-loop (the DaemonEye 529-turn motivation)
is still caught at 60. Demoting novelty removes the false-positive kill while
keeping a real safety net. Thresholds get set from data in phase-06, not guessed;
until then, novelty observes.

## Architecture references

Read before starting:

- `docs/architecture.md` § Layer 1 (the executor turn cycle — the hard-fail
  detector list) and § Status #34.
- `docs/dev/STANDARDS.md` § 2.2 ("No fallbacks for 'if X is missing'" / prefer
  additive shapes) and § "Derive intentionally".

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture reference above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

The stall detectors feed a single `hard_fail_signal` in the turn cycle
(`executor/src/agent/mod.rs`, ~line 1307, after phase-04's emit block):

```rust
let hard_fail_signal = evaluate(
    &recent_tool_calls,
    &recent_verifier_error_counts,
    Some((&tool_call.name, content.len())),
    &deps.governor,
)
.or_else(|| {
    crate::governor::hard_fail::check_oscillation(
        &recent_tool_calls,
        deps.governor.oscillation_window,
        deps.governor.oscillation_distinct_max,
    )
})
.or_else(|| {
    crate::governor::hard_fail::check_windowed_output(
        &recent_output_bytes,
        deps.governor.output_window,
        deps.governor.output_window_bytes,
    )
})
.or_else(|| {
    crate::governor::hard_fail::check_low_novelty_stall(
        &recent_tool_calls,
        deps.governor.novelty_window,
        deps.governor.novelty_distinct_floor,
    )
})
.or_else(|| {
    crate::governor::hard_fail::check_read_only_stall(
        &recent_tool_calls,
        deps.governor.read_only_stall_threshold,
    )
});
```

`GovernorConfig` (`executor/src/config.rs:207`) is
`#[derive(Debug, Clone, Copy, Serialize, Deserialize)]` + `#[serde(default)]` — it
does **not** derive `JsonSchema`, so a new field/enum needs no schema derive
(derive intentionally). `ModelOverride` (`config.rs:283`) carries per-model
`Option<..>` overrides applied in `apply_model_override` (the `if let Some(v) =
over.<field>` block, ~line 599). The `rexymcp init` `[governor]` template is
`mcp/src/init.rs:37`.

The novelty **measurement + emit** (phase-04) is unconditional and stays — so in
advisory mode a `NoveltySample { window, distinct_targets }` with
`distinct_targets <= novelty_distinct_floor` **is** the advisory record. No new
event is needed (see Out of scope).

## Spec

Additive throughout — a new enum + one config field + one override field + one
match in the loop. No existing detector's behavior changes except novelty's
default.

### 1. Add the `NoveltyAction` enum + config field

In `executor/src/config.rs`, above `GovernorConfig`:

```rust
/// What the low-novelty (churn) detector does when a full window collapses to
/// `<= novelty_distinct_floor` distinct targets. `Advisory` (default): the
/// measurement is recorded (`NoveltySample`) but the run continues — the real
/// terminators are `max_turns`, the optional `wall_clock_secs`, and the high
/// `read_only_stall_threshold` volume backstop. `Terminate`: hard-fail with
/// `LowNoveltyStall` (pre-M34-phase-05 behavior). Advisory is the default because
/// the 24/6 thresholds are un-calibrated; a data-free early kill must not pre-empt
/// a run the turn budget would still fund.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum NoveltyAction {
    #[default]
    Advisory,
    Terminate,
}
```

Add the field to `GovernorConfig` (after `novelty_distinct_floor`):

```rust
    /// Whether a fired low-novelty window terminates the run or is advisory-only.
    /// Default `Advisory` — see `NoveltyAction`.
    pub novelty_action: NoveltyAction,
```

And in `impl Default for GovernorConfig` (after `novelty_distinct_floor: 6,`):

```rust
            novelty_action: NoveltyAction::Advisory,
```

`GovernorConfig` is `Copy`; `NoveltyAction` derives `Copy`, so this holds. Because
`GovernorConfig` is `#[serde(default)]`, an existing `rexymcp.toml` with no
`novelty_action` key loads as `Advisory` (the intended default flip).

### 2. Per-model override

In `ModelOverride` (`config.rs:283`), add:

```rust
    pub novelty_action: Option<NoveltyAction>,
```

and in `apply_model_override`, alongside the other `if let Some(v) = over.<field>`
lines:

```rust
        if let Some(v) = over.novelty_action {
            self.governor.novelty_action = v;
        }
```

### 3. Gate the terminating branch in the loop

In `executor/src/agent/mod.rs`, change the `check_low_novelty_stall` `.or_else`
branch so it contributes to `hard_fail_signal` **only** in `Terminate` mode:

```rust
.or_else(|| match deps.governor.novelty_action {
    crate::config::NoveltyAction::Terminate => {
        crate::governor::hard_fail::check_low_novelty_stall(
            &recent_tool_calls,
            deps.governor.novelty_window,
            deps.governor.novelty_distinct_floor,
        )
    }
    crate::config::NoveltyAction::Advisory => None,
})
```

Leave the `check_read_only_stall` branch (raw backstop) **unchanged** — it stays
terminating. Leave phase-04's `measure_novelty` emit block above **unchanged** —
the `NoveltySample` stream is the advisory record in both modes.

### 4. `rexymcp init` template

In `mcp/src/init.rs`, add to the `[governor]` block:

```
novelty_action = "advisory"       # "advisory" (default): log low-novelty churn but keep running; "terminate": hard-fail on it
```

(A commented per-model override line is optional — match the surrounding style if
you add one.)

## Acceptance criteria

- [ ] `GovernorConfig::default().novelty_action == NoveltyAction::Advisory`.
- [ ] A `[governor]` table with no `novelty_action` key parses to `Advisory`
      (back-compat).
- [ ] `novelty_action = "terminate"` parses to `Terminate`; a per-model
      `[models."…"] novelty_action = "terminate"` overrides the global default.
- [ ] With the default (`Advisory`), a read-only churn that fills the novelty
      window **does not** hard-fail — the run reaches the turn cap
      (`budget_exceeded`) — and a `NoveltySample` with `distinct_targets <=
      novelty_distinct_floor` is present in the log.
- [ ] With `novelty_action = Terminate`, the same churn **does** hard-fail
      `LowNoveltyStall` (old behavior preserved).
- [ ] The raw `NoProgressStall` still fires at `read_only_stall_threshold`
      regardless of `novelty_action` (unchanged).
- [ ] All four gates green.

## Test plan

- `governor_novelty_action_defaults_to_advisory` in `config.rs` — `Default`.
- `governor_config_without_novelty_action_key_loads_as_advisory` — parse a
  `[governor]` toml missing the field (back-compat pin).
- `novelty_action_terminate_parses_and_model_override_applies` — `"terminate"`
  round-trips; a per-model override flips the resolved governor value.
- Loop integration in `agent/tests.rs` (mirror phase-04's
  `novelty_samples_are_emitted_deduped_and_rearm_after_edit` harness):
  - `low_novelty_churn_is_advisory_by_default` — churn over a few files with
    `novelty_window` small and `novelty_distinct_floor` such that it *would* trip;
    default `novelty_action` → assert `status == BudgetExceeded` (not `HardFail`)
    **and** a `NoveltySample` with `distinct_targets <= floor` is logged.
  - `low_novelty_churn_hard_fails_when_action_is_terminate` — same setup with
    `novelty_action: NoveltyAction::Terminate` → assert `HardFail` with
    `Blocker::HardFail(HardFailSignal::LowNoveltyStall { .. })`. (Proves the switch
    and preserves the pre-demotion behavior — mutation-resistant: revert the gate
    and the advisory test hard-fails instead.)

## End-to-end verification

Ships a runtime-loadable config artifact. Verify the running binary honors the
default:

```
rexymcp init --config <tmp>/rexymcp.toml     # template contains novelty_action = "advisory"
```

and confirm a real config load resolves `novelty_action` (quote the value). If the
loop-level default is verified only via the hermetic harness (the integration
tests above), state that and quote the asserted status. A live serve dispatch is
**not** required and is discouraged here (see Notes — routing).

## Authorizations

- [ ] May touch `docs/architecture.md` (§ Layer 1 detector-list wording: note the
      low-novelty catch is advisory-by-default). Only if needed for accuracy;
      otherwise leave it — the architect will fold the detector-list wording at
      milestone close.

## Out of scope

- **Do not** add a new "advisory fired" `SessionEvent` — the `NoveltySample`
  stream already carries the signal (a live advisory marker is deferred to
  phase-06's metrics work).
- **Do not** demote or otherwise change the raw `NoProgressStall`,
  `oscillation`, `identical-repetition`, `runaway`, `cumulative-flood`,
  `verifier-persistence`, `empty-completion`, or `stuck-gate` detectors — this
  phase touches novelty only. (Whether the others should also become advisory is a
  phase-06 question.)
- **Do not** build the calibration back-test or any metrics aggregation — that is
  phase-06.
- **Do not** change threshold *values* (`novelty_window`/`novelty_distinct_floor`/
  `read_only_stall_threshold`) or `max_turns`/`wall_clock_secs` defaults.
- The `[governor]` init template is missing the `read_only_stall_threshold` /
  `novelty_window` / `novelty_distinct_floor` lines (a pre-existing gap from the
  FR-2/issue-#3 direct commits). **Do not** fix that here — add only
  `novelty_action`. Note it in "Notes for review".

## Notes

**Routing — direct execution strongly recommended.** This phase disarms the
novelty terminator, but the **supervising** governor (the running `serve` binary)
still has novelty in `Terminate` mode until a new binary is installed and `serve`
restarted. Dispatching this phase to the local executor therefore risks the *same*
`LowNoveltyStall` takedown that killed phase-04's dispatch at turn 104 — the thing
under repair would supervise its own repair. Finish it by **direct execution**
(Claude Code), as with phases 01–03. The phase doc records the actual executor at
review.

**Follow-on the user should weigh (not this phase):** with novelty advisory, the
effective terminators are the raw 60-consecutive-read-only backstop + `max_turns`
(600 here) + `wall_clock_secs` (currently `0`/off). Consider setting a
`wall_clock_secs` ceiling as a wall-clock safety net. Phase-06 informs the final
threshold/terminator policy from data.

## Update Log

(Filled in by the executor.)

<!-- entries appended below this line -->
