# Bug 1 on phase-05b: hard-rule `#[allow]` + missing wrapper-level handler tests

**Severity:** minor (both items have trivial fixes; no functional defect)
**Status:** fixed
**Filed:** 2026-05-31

## What's wrong

Two distinct issues. Both are bounce-class because they violate explicit rules
the spec / repo standards pin.

### Issue 1 — Hard-rule violation: `#[allow(clippy::too_many_arguments)]`

`mcp/src/runner.rs:201`:

```rust
#[allow(clippy::too_many_arguments)]
pub async fn run_phase(
    cfg: &Config,
    phase_doc_path: &Path,
    repo_path: &Path,
    executor_contract: &str,
    standards: &str,
    model_override: Option<&str>,
    telemetry_dir: Option<&Path>,
    progress: Option<&dyn ProgressCallback>,
) -> Result<PhaseResult>;
```

**This violates a CLAUDE.md hard rule** (excerpted):

> Do not, without explicit phase-doc authorization: add a dependency …; write
> `unsafe`; widen scope …; **add `#[allow]`/`#[ignore]` to mask a diagnostic**;
> leave `TODO`/`FIXME`/`dbg!`/`println!`/commented-out code; …

Phase-05b's authorizations did **not** authorize `#[allow]`. The phase-01 spec
explicitly walked through this exact decision for `run_phase_with` and chose
the struct-grouping path (which gave us `Seams` and `AssemblyInput`). Phase-05a
chose the same path (giving us `EmitCtx` so `emit_progress` stayed under
clippy's limit). Phase-05b took the lazy `#[allow]` shortcut.

The clean fix is identical to the patterns already in this codebase: group the
non-seam args into a small struct.

## How to fix Issue 1

Introduce a `RunPhaseConfig<'a>` (name negotiable) that groups the
configuration-shaped args, leaving the leaner public signature:

```rust
pub struct RunPhaseConfig<'a> {
    pub cfg: &'a Config,
    pub phase_doc_path: &'a Path,
    pub repo_path: &'a Path,
    pub executor_contract: &'a str,
    pub standards: &'a str,
    pub model_override: Option<&'a str>,
    pub telemetry_dir: Option<&'a Path>,
    pub progress: Option<&'a dyn ProgressCallback>,
}

pub async fn run_phase(input: RunPhaseConfig<'_>) -> Result<PhaseResult>;
```

Update both call sites accordingly:

- `mcp/src/main.rs` (`RunPhase` clap handler) — build a `RunPhaseConfig`,
  pass it.
- `mcp/src/server.rs` (`execute_phase_inner`) — build a `RunPhaseConfig` from
  the params it already has, pass it.

Remove the `#[allow(clippy::too_many_arguments)]` attribute. Clippy must be
clean without it.

Alternative shapes are fine (e.g. a builder; or splitting into
`RunPhaseArgs` + `RunPhaseSeams`) — the **only** requirement is that the
`#[allow]` disappears and clippy stays green.

### Issue 2 — Missing wrapper-level handler tests (acceptance criteria)

Phase-05b's Acceptance criteria had two explicit checkboxes:

- [ ] **Wrapper-level test:** `execute_phase_inner` with `Some(&capture)`
      drives the M4 loop through a small `MockAiClient` script and the
      `CaptureCallback` receives the expected sequence (regression check
      that 05b's threading doesn't drop the events 05a tested).
- [ ] **Wrapper-level test:** `execute_phase_inner` with `None` captures
      nothing (the `None` path still works).

Both were skipped. opencode's declared rationale: "would duplicate 05a's
integration tests."

**The rationale is wrong.** 05a's integration tests verify the *executor
loop's* emission behavior, called directly against `agent::execute_phase`.
Phase-05b's wrapper-level tests would verify the **mcp threading layer** —
that a `Some(&cb)` passed into `execute_phase_inner` actually reaches
`LoopDeps.progress` after traversing `runner::run_phase_with` →
`AssemblyInput.progress` → `LoopDeps`. That is the *specific* bug class —
silently dropping `progress` somewhere in the manual `ServerHandler` /
`call_tool` rewrite — that this acceptance criterion exists to catch.

The notifier-level tests opencode did add (`progress_notifier_maps_fields_correctly`,
`progress_notifier_fire_and_forget_does_not_panic`) test the `McpProgressNotifier`
in isolation, not the *threading*. Different concern.

## How to fix Issue 2

Add two tests in `mcp/src/server.rs` `#[cfg(test)] mod tests`:

1. **`execute_phase_inner_forwards_progress_to_loop`** — build a `TempDir`
   repo + a tiny phase-doc fixture + a `Config` (use phase-04's pattern from
   `make_test_config` / `make_phase_doc` already in the test module), inject
   a `MockAiClient` scripted to drive the loop through at least one tool call
   (e.g. `read_file` → completion), pass a `CaptureCallback` (or a free
   `Arc<Mutex<Vec<ProgressEvent>>>`-capturing closure via the closure
   blanket impl 05a added) as `progress`, call `execute_phase_inner`, and
   assert the captured events include at least one `turn_start` and one
   `tool:<name>` matching what 05a's emission sites produce.

2. **`execute_phase_inner_with_none_captures_nothing`** — same setup with
   `progress: None`; assert nothing was captured (use an outside `AtomicBool`
   or a captured `Mutex<bool>` that the would-be-callback flips, but only
   if `Some` — proves the absence by the absence of mutation).

The first one needs to thread a real `MockAiClient` through the existing
`runner::run_phase_with` path, which `execute_phase_inner` calls through
`runner::run_phase`. **Note:** `execute_phase_inner` takes a `config_path`,
which means it constructs the production `OpenAiClient` — it can't be swapped
for a mock without further factoring. **There are two ways to make this
testable:**

- **Option A — refactor the seam:** Extract another `pub(crate)` layer between
  `execute_phase_inner` and `runner::run_phase` that takes the client (and
  other seams) explicitly, the way `run_phase_with` already does. Test that
  layer. The current `execute_phase_inner` becomes the production wrapper.
- **Option B — accept a smaller-scope test:** Don't test the full execution;
  instead test that `execute_phase_inner` *would* call `runner::run_phase`
  with the right `progress` value, by stubbing `runner::run_phase` behind an
  injectable function pointer (overkill).

**Recommended: Option A.** It's the same `run_phase_with` /
`run_phase` split the phase-01 spec mandated for runner.rs. Apply the same
pattern at the server layer.

If Option A's refactor feels too large, **file a sub-blocker** noting that
the wrapper-level test requires a server-layer seam split, and propose a
smaller assertion that *does* validate threading without driving the full
loop (e.g. a test-only `runner::run_phase_capturing` fn that records its
`progress` argument and returns a fixed `PhaseResult`). That's a real spec
gap on my part — phase-05b's spec named the test but didn't pin the
testability mechanism. I'll accept either resolution: do the refactor, or
file the sub-blocker so we can spec it.

## Why this matters

- **Issue 1** is a hard-rule violation; the repo's discipline depends on
  these rules being enforced consistently. Phase-05a chose the right path
  (`EmitCtx`); phase-05b chose the wrong path (`#[allow]`). The fix is the
  same pattern, applied at the next layer up.
- **Issue 2** is a missed acceptance criterion. The threading layer added in
  phase-05b is exactly the kind of glue that silently breaks when something
  upstream changes (e.g. a future phase refactors `runner::run_phase`'s
  signature). A wrapper-level test is the regression net.

The phase-04 / phase-05a streak of "approved_first_try, zero deviations"
established a high bar; the calibration carry-forward in phase-05b's spec
explicitly invoked that streak. Bouncing here keeps the bar high — *with the
calibration intact*, since opencode **correctly declared** Issue 2 upfront
in Notes for review. Self-review accuracy is working; the gap is in how I
characterized "declared deviation" vs "missed acceptance criterion." Going
forward, a declared deviation that's against an explicit acceptance criterion
checkbox is still a bounce; a declared deviation against a *test plan*
suggestion is a discussion.

## Tracking

Phase-05b status flips back to `in-progress` until both items are resolved
+ re-reviewed.

## Notes

- The big rmcp-API divergence (manual `ServerHandler` impl replacing the
  macro tool router for `execute_phase`) is **not** in this bug. That was
  the right architectural call given rmcp 1.7's macro limitation, was
  correctly declared, and the hybrid pattern (manual `call_tool` for
  `execute_phase` + `Self::tool_router()` fallback for the other four
  tools) is clean. The 98 mcp tests still passing confirms the routing
  works.
- Notifier-level tests (`progress_notifier_maps_fields_correctly`,
  `progress_notifier_fire_and_forget_does_not_panic`) are fine — keep them
  as-is.
- The `Acceptance criteria: all ticked above` line in the Update Log should
  also be corrected when fixing — phase-05b's self-review claimed all
  ticked, but the wrapper-level criteria were skipped. The calibration
  about self-review accuracy (phase-01 origin, phase-02/03/04/05a held)
  needs to hold here too. **Replace the "all ticked above" claim with an
  honest acknowledgment** in the next Update entry.
