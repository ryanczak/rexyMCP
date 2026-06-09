# Phase 01: Verifier missing-binary → `Skipped` advisory

**Milestone:** M12 — Executor Tooling
**Status:** review
**Depends on:** none (first M12 phase)
**Estimated diff:** ~70 lines
**Tags:** language=rust, kind=bugfix, size=s

## Goal

When a verifier toolchain binary is **absent** (no `cargo`/`tsc`/`ruff` on PATH),
the verifier today returns `VerifierResult::Failed("cargo check spawn failed:
No such file or directory (os error 2)")`. The agent loop appends that raw,
opaque string to the conversation **on every edit turn** (`mod.rs:804`) — it
names no remedy, and it repeats. This phase distinguishes "the tool isn't
installed" from a genuine infrastructure failure: a missing binary becomes a new
`VerifierResult::Skipped` advisory that **names the binary and how to install
it**, surfaced once per edit turn as a *skipped* (not *failed*) notice.

This is **Arc 0** of M12 — toolchain robustness (fail-open at runtime: degrade to
a clear advisory, keep working). It is the foundation the Arc B code-intelligence
phases build on, since those also shell out to `cargo`.

## Architecture references

Read before starting:

- `docs/architecture.md#status` — M12 Arc 0 ("toolchain robustness"): missing
  validation tools fail open at runtime with a remedy-naming advisory.
- `docs/dev/WORKFLOW.md` § "Validation features depend on the target toolchain" —
  the rule this phase implements: a missing binary is a model-visible advisory
  naming the binary + remedy, never a panic, never an opaque "spawn failed", and
  not a verifier-failure outcome the governor counts as a strike.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom (note §2.6 on runtime binaries vs.
   crate deps).
2. Read this entire phase doc before touching any code.
3. Run `cargo test -p rexymcp-executor 2>&1 | tail -3` and record the result line
   (expected: **671 passed; 0 failed; 2 ignored**). After this phase the *passed*
   count rises by the new tests you add; `2 ignored` stays **2**.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

### The `VerifierResult` enum — `executor/src/governor/verifier.rs:66`

```rust
/// Outcome of a single `verify(...)` call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifierResult {
    /// The verifier ran successfully. `diagnostics` may be empty …
    Checked { diagnostics: Vec<Diagnostic> },
    /// The file's extension isn't supported by any available checker …
    Unsupported,
    /// Infrastructure failure — process spawn failed, the output didn't parse … 
    Failed(String),
}
```

### The three spawn sites are identical in shape

`verify_rust` (`verifier.rs:239`), `verify_typescript` (`:347`), `verify_python`
(`:433`) each do:

```rust
let output = match Command::new("cargo")          // or "tsc" / "ruff"
    .arg("check").arg("--message-format=json")
    .current_dir(&crate_root)
    .stdout(Stdio::piped()).stderr(Stdio::piped())
    .output()
    .await
{
    Ok(o) => o,
    Err(e) => {
        return VerifierResult::Failed(format!("cargo check spawn failed: {e}"));
    }
};
```

A **missing binary** surfaces here as an `io::Error` with
`e.kind() == std::io::ErrorKind::NotFound`. Other spawn errors (e.g.
`PermissionDenied`) are genuine failures and must stay `Failed`.

### How results are consumed — the "no strike" fact

The **only** place a verifier outcome feeds the hard-fail governor is
`mod.rs:794`, inside the `Checked` arm:

```rust
VerifierResult::Checked { diagnostics } => {
    let (author, _ambient) = baseline.partition(&diagnostics);
    // …
    recent_verifier_error_counts.push(author.len());   // <-- only Checked pushes
    // …
}
VerifierResult::Unsupported => {}
VerifierResult::Failed(msg) => {
    messages.push(user_text(&format!("verifier failed: {msg}"), turns));
}
```

`check_verifier_persistence` (`governor/hard_fail.rs:100`) reads **only**
`recent_verifier_error_counts`. So `Failed`/`Unsupported` already never accrue a
strike — and `Skipped` must not either. **The "no strike" property comes for free
by not pushing to that vector** (same as `Failed`/`Unsupported`); the value of
this phase is the *clear, actionable advisory*, plus making the distinction
explicit and future-proof in the type.

`capture_baseline` (`verifier.rs:164`) matches the enum too:

```rust
VerifierResult::Unsupported | VerifierResult::Failed(_) => {
    // No baseline diagnostics for files the verifier can't check. Skip.
}
```

## Spec

Additive: a new enum variant + a pure classifier helper + three one-line spawn-arm
swaps + two match-arm additions. No existing behavior changes for the `Checked`
path or for non-NotFound spawn errors.

### Task 1 — add the `Skipped` variant — `verifier.rs:66`

Add a fourth variant to `VerifierResult`, documented like its siblings:

```rust
    /// A required checker binary isn't installed (not on PATH).
    /// Distinct from `Failed` (a genuine infra error) and from
    /// `Unsupported` (the file type has no checker at all). The
    /// agent loop surfaces this as a one-line advisory naming the
    /// binary and how to install it; it is NOT the model's fault
    /// and never counts toward verifier-persistence hard-fail.
    Skipped(String),
```

### Task 2 — add the pure classifier `spawn_failure` — `verifier.rs`

Add this private helper near the per-language checkers (above `verify_rust` is
fine). It maps a spawn `io::Error` to the right variant:

```rust
/// Map a checker-spawn `io::Error` to a VerifierResult. A
/// `NotFound` error means the toolchain binary isn't installed —
/// a `Skipped` advisory that names the remedy. Any other spawn
/// error is a genuine infrastructure `Failed`.
fn spawn_failure(tool: &str, install_hint: &str, err: &std::io::Error) -> VerifierResult {
    if err.kind() == std::io::ErrorKind::NotFound {
        VerifierResult::Skipped(format!(
            "{tool} not found on PATH — {install_hint}; \
             incremental verification is disabled this run"
        ))
    } else {
        VerifierResult::Failed(format!("{tool} spawn failed: {err}"))
    }
}
```

### Task 3 — route the three spawn arms through it

Replace each `Err(e) => { return VerifierResult::Failed(format!("… spawn failed: {e}")); }`
with a `spawn_failure` call carrying the binary name and an install hint:

- `verify_rust` (`:248`): `Err(e) => return spawn_failure("cargo", "install the Rust toolchain via https://rustup.rs", &e),`
- `verify_typescript` (`:356`): `Err(e) => return spawn_failure("tsc", "install TypeScript (npm install -g typescript)", &e),`
- `verify_python` (`:442`): `Err(e) => return spawn_failure("ruff", "install ruff (pip install ruff)", &e),`

Note this changes the non-NotFound `Failed` text slightly (e.g. `cargo check
spawn failed` → `cargo spawn failed`) — that string is an opaque infra notice,
asserted by no test; the change is intentional and fine.

### Task 4 — handle `Skipped` in `capture_baseline` — `verifier.rs:171`

Add the new variant to the skip arm (no baseline diagnostics when the tool can't
run):

```rust
VerifierResult::Unsupported | VerifierResult::Failed(_) | VerifierResult::Skipped(_) => {
    // No baseline diagnostics for files the verifier can't check. Skip.
}
```

### Task 5 — handle `Skipped` in the agent loop — `mod.rs` (the `match` at ~:781)

Add a `Skipped` arm beside the existing `Failed` arm. It appends a *skipped*
advisory (distinct wording from "verifier failed") and does **nothing** with
`recent_verifier_error_counts` — that is what guarantees no strike:

```rust
VerifierResult::Skipped(msg) => {
    messages.push(user_text(&format!("verifier skipped: {msg}"), turns));
}
```

### Step — verify

```bash
cargo build -p rexymcp-executor 2>&1 | tail -5
cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail -5
cargo test -p rexymcp-executor 2>&1 | tail -3
cargo fmt --all --check
```

If the build reports a non-exhaustive `match` on `VerifierResult` anywhere not
listed above, that is a site this spec missed — add the `Skipped` arm there
(treat it like `Failed`/`Unsupported`: skip / advisory, never a strike) and note
it in "Notes for review".

## Acceptance criteria

- [ ] `VerifierResult` has a `Skipped(String)` variant documented as a
  missing-binary advisory.
- [ ] A private `spawn_failure(tool, install_hint, &io::Error)` returns `Skipped`
  for `ErrorKind::NotFound` and `Failed` for any other error.
- [ ] All three spawn arms (`verify_rust`/`verify_typescript`/`verify_python`)
  route through `spawn_failure`.
- [ ] `capture_baseline` and the agent-loop `verify` match both handle `Skipped`
  (skip / advisory respectively); the loop's `Skipped` arm does **not** touch
  `recent_verifier_error_counts`.
- [ ] `cargo build -p rexymcp-executor` succeeds with zero warnings.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes.
- [ ] `cargo test -p rexymcp-executor` passes; `2 ignored` unchanged.
- [ ] `cargo fmt --all --check` passes.

## Test plan

New tests in `executor/src/governor/verifier_tests.rs` (the sibling test module;
`use super::*` already gives access to private items like `spawn_failure`):

- `spawn_failure_not_found_is_skipped_naming_remedy` — build
  `std::io::Error::new(std::io::ErrorKind::NotFound, "x")`, call
  `spawn_failure("cargo", "install the Rust toolchain", &err)`, assert the result
  is `VerifierResult::Skipped(msg)` **and** `msg` contains `"cargo"` and
  `"install the Rust toolchain"`. (Per-assertion messages on the multi-assert.)
- `spawn_failure_other_error_stays_failed` — same but with
  `ErrorKind::PermissionDenied`; assert `VerifierResult::Failed(msg)` and `msg`
  contains `"spawn failed"`. **This is the pinned negative case**: a present-but-
  unspawnable tool must not be mislabeled as "not installed".

New test in `executor/src/agent/tests.rs` (use the existing `MockFileVerifier`
that pops `results` — `tests.rs:914` — and the `run_with_verifier` helper at
`tests.rs:943`):

- `loop_surfaces_skipped_verifier_as_advisory` — drive one edit turn with the
  mock verifier returning `VerifierResult::Skipped("cargo not found on PATH — …".into())`;
  assert the conversation gains a message containing `"verifier skipped"` and the
  binary/remedy text. (Follow the existing `Failed`/`Checked` loop tests' shape.)

No test needs to remove `cargo` from PATH — the `NotFound` path is exercised
directly through the pure `spawn_failure` classifier, which is the exact code the
spawn arms call.

## End-to-end verification

> Not applicable as a quoted-output check — the verifier is an internal loop
> component with no CLI/binary entrypoint that surfaces its result directly, and a
> genuinely-missing-binary repro can't be made hermetic (it would require
> mutating the test host's PATH). The missing-binary code path is covered by the
> `spawn_failure_not_found_is_skipped_naming_remedy` unit test (the exact classifier
> the three spawn arms invoke) and the loop advisory by
> `loop_surfaces_skipped_verifier_as_advisory`. Optional manual check at review:
> run the executor binary with `PATH=` against a `.rs` edit and observe the
> `verifier skipped: cargo not found …` advisory instead of a raw spawn error.

## Authorizations

None. (No new dependency — `std::io::ErrorKind` is std; no `unsafe`; no edit to
`Cargo.toml`, the architecture doc, or any other phase doc. The `Skipped` variant
and `spawn_failure` are additions the spec requires, not new files.)

## Out of scope

- Do **not** add a `SessionEvent` for `Skipped` or wire it to the dashboard — the
  loop advisory (mirroring how `Failed` is handled) is the whole scope. Dashboard
  surfacing, if ever wanted, is a separate phase (and would hit the known
  `SessionEvent` match-arm wall — keep it out of here).
- Do **not** change the `Checked` path, diagnostic parsing, baseline logic, or any
  governor threshold.
- Do **not** stop re-spawning the missing tool on later turns (an
  once-per-session-memo optimization) — out of scope; one advisory per edit turn
  is acceptable and matches today's `Failed` cadence.
- Do **not** touch `tsc`/`ruff` parsing or the `rexymcp doctor` command (phase-02).

## Notes for executor

- **Why a new variant rather than just a better `Failed` message:** "the tool
  isn't installed" is a genuinely different outcome from "the tool ran/errored",
  and a later phase (`rexymcp doctor`) and the dashboard will want to treat it
  distinctly. The variant makes the distinction explicit in the type.
- **The "no strike" guarantee is structural, not something you add:** only the
  `Checked` arm pushes to `recent_verifier_error_counts` (`mod.rs:794`). As long
  as your `Skipped` arm doesn't push to it (it shouldn't — just append the
  advisory), the governor can't count it. Do not add any counter bump.
- **`spawn_failure` is pure and private** — test it directly from
  `verifier_tests.rs` via `use super::*`; no subprocess, no PATH manipulation.
- Adding the variant makes every exhaustive `match VerifierResult` non-exhaustive
  until you add the arm. The spec lists the two production sites
  (`capture_baseline`, the `mod.rs` loop match); `cargo build` will name any
  other (e.g. a test helper) — add `Skipped` there too and note it.
- Commit as a single `fix:` commit; the body explains *why* (a missing toolchain
  binary produced an opaque, remedy-less notice repeated each edit turn), not
  *what*.

## Update Log

### Update — 2026-06-09 (started)

**Executor:** rexyMCP executor LLM

**Summary:** Implementing `VerifierResult::Skipped` variant + `spawn_failure` classifier to turn missing-binary spawn errors into clear, remedy-naming advisories.

### Update — 2026-06-09 (complete)

**Executor:** rexyMCP executor LLM

**Summary:** Added `Skipped(String)` variant to `VerifierResult`, a pure `spawn_failure` classifier, routed all three spawn arms through it, and handled `Skipped` in `capture_baseline`, the agent loop, and one test match. Added 3 new tests.

**Verification commands:**
```
cargo build -p rexymcp-executor: success (0 warnings)
cargo clippy --all-targets --all-features -- -D warnings: success
cargo test -p rexymcp-executor: 674 passed; 0 failed; 2 ignored
cargo fmt --all --check: success
```

**One-line verification summary:** build clean, clippy clean, 674 tests pass (2 ignored unchanged), fmt clean.

**Grep proof for pinned literals:**
```
grep -n "Skipped(String)" executor/src/governor/verifier.rs → 91:Skipped(String)
grep -n "spawn_failure" executor/src/governor/verifier.rs → 241,274,386,472
grep -n "verifier skipped" executor/src/agent/mod.rs → 808
```

**Files changed:**
- `executor/src/governor/verifier.rs` — added `Skipped` variant, `spawn_failure` helper, routed 3 spawn arms, updated `capture_baseline` match
- `executor/src/governor/verifier_tests.rs` — added `spawn_failure_not_found_is_skipped_naming_remedy`, `spawn_failure_other_error_stays_failed`, updated `python_verifier_handles_missing_ruff` match
- `executor/src/agent/mod.rs` — added `Skipped` arm to verifier result match
- `executor/src/agent/tests.rs` — added `loop_surfaces_skipped_verifier_as_advisory`
- `docs/dev/milestones/M12-executor-tooling/phase-01-verifier-degrade.md` — status flip + Update Log
- `docs/dev/milestones/M12-executor-tooling/README.md` — phase table status flip

**Commit:** fix: verifier missing-binary → Skipped advisory naming the binary and remedy

**Notes for review:**
- One additional match arm was needed beyond the spec's two listed sites: `verifier_tests.rs`'s `python_verifier_handles_missing_ruff` test had an exhaustive match on `VerifierResult` that now includes `Skipped(_)`. Handled by treating it as acceptable (ruff may not be installed on CI).
- The `Skipped` arm in `mod.rs` does NOT touch `recent_verifier_error_counts` — the no-strike guarantee is structural (only `Checked` pushes to that vector).

**End-to-end verification:** N/A — the verifier is an internal loop component with no CLI entrypoint. The missing-binary path is covered by the pure `spawn_failure_not_found_is_skipped_naming_remedy` unit test and the loop advisory by `loop_surfaces_skipped_verifier_as_advisory`. Optional manual check: run the executor with `PATH=` against a `.rs` edit and observe `verifier skipped: cargo not found …`.
