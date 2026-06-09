# Phase 06: Inject the current date into the executor system prompt

**Milestone:** M11 — Polish
**Status:** review
**Depends on:** phase-05b (ordering only — no code dependency)
**Estimated diff:** ~55 lines
**Tags:** language=rust, kind=feature, size=s

## Goal

The local executor has **no temporal grounding**: it cannot read a clock, so
every Update Log it writes is stamped with a hallucinated date (recent runs
stamped `2025-07-09`, `2025-07-15`, etc., while the real date is months later).
All *machine* records already use the real injected `deps.clock` (epoch-millis),
so this is a polish fix for *model-authored prose*, not a correctness fix.

This phase prepends a single grounding line — `Today's date is YYYY-MM-DD
(UTC).` — to the top of the assembled system prompt, formatted from the same
injected clock the rest of the loop uses. This is the **final phase of M11**.

## Architecture references

Read before starting:

- `docs/architecture.md#status` — M11 sub-goal "Executor temporal grounding"
  (added at this phase's kickoff) names this work: inject the real date so the
  model stops hallucinating Update Log timestamps.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read this entire phase doc before touching any code.
3. Run `cargo test -p rexymcp-executor 2>&1 | tail -3` and record the result line
   (expected: **665 passed; 0 failed; 2 ignored**). After this phase the *passed*
   count rises by the number of new tests you add (6 below); the `2 ignored`
   count must stay **2**.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

`executor/src/agent/prompt.rs` (production lines 1–24) assembles the system
prompt from three string inputs. **You will not change this function** — you add
two new functions beside it and compose them at the one call site.

```rust
// executor/src/agent/prompt.rs:9
pub fn assemble_system_prompt(
    commands: &CommandConfig,
    standards: &str,
    phase_doc: &str,
) -> String {
    let contract_body = contract::assemble_executor_contract(commands);
    let mut out = String::new();
    out.push_str("# Executor contract\n\n");
    // … pushes contract, standards, phase_doc …
    out
}
```

The injected clock is epoch-**milliseconds**, a `u64`, reached as `(deps.clock)()`
(`executor/src/agent/mod.rs:88`):

```rust
// executor/src/agent/mod.rs:87-88
/// Epoch-millis clock for session-log record timestamps.
pub clock: &'a (dyn Fn() -> u64 + Send + Sync),
```

The **single production call site** of `assemble_system_prompt` is
`executor/src/agent/mod.rs:115`, inside `execute_phase`, where `deps.clock` is in
scope:

```rust
// executor/src/agent/mod.rs:115
let system = prompt::assemble_system_prompt(deps.commands, &input.standards, &input.phase_doc);
```

The three existing tests in `prompt.rs` (`assembles_system_prompt_in_contract_standards_phase_order`,
`system_prompt_includes_substituted_contract`,
`system_prompt_order_is_contract_then_standards_then_phase_doc`) call
`assemble_system_prompt` with **three** arguments. **They must stay untouched** —
the additive design below does not change that signature.

**There is no `chrono` (or any date) crate in this workspace** — the project
deliberately carries the clock as a raw `u64` epoch-millis and formats nothing.
Do **not** add `chrono` or any dependency. Format the date with the pure
integer-arithmetic routine pre-injected below.

## Spec

Two new functions in `executor/src/agent/prompt.rs`, plus a one-line compose at
the call site. Additive shape: the existing `assemble_system_prompt` and its
three tests are not touched.

### Task 1 — add the pure date formatter `format_utc_date`

Add this **private** function to `prompt.rs` (below `assemble_system_prompt`,
above the `#[cfg(test)]` module). It converts epoch-millis to a `YYYY-MM-DD`
UTC date using the standard civil-from-days integer algorithm. **Use this body
verbatim — it is verified against the fixtures in the Test plan** (including the
leap-day, epoch-zero, and year-boundary cases):

```rust
/// Format epoch-millis (UTC) as `YYYY-MM-DD` using civil-from-days integer
/// arithmetic — no date dependency, deterministic, hermetic. Input is the
/// injected `clock` value (always ≥ 0), so no negative-era branch is needed.
fn format_utc_date(now_ms: u64) -> String {
    let days = (now_ms / 1_000) / 86_400; // whole days since 1970-01-01 (UTC)
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097; // day-of-era, [0, 146096]
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // day-of-year, [0, 365]
    let mp = (5 * doy + 2) / 153; // month-prime, [0, 11]
    let day = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let month = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let year = if month <= 2 { year + 1 } else { year };
    format!("{year:04}-{month:02}-{day:02}")
}
```

Do **not** "simplify", reorder, or rename the intermediate steps — the magic
constants (`719_468`, `146_097`, `36_524`, `1_460`, `153`) are the algorithm.
All arithmetic is `u64`; no casts are required.

### Task 2 — add the public header builder `datetime_header`

Add this **public** function beside `format_utc_date`:

```rust
/// The one-line temporal-grounding header prepended to the system prompt. The
/// local model has no clock of its own; without this it stamps hallucinated
/// dates in its Update Log. Built from the injected `clock`, never real
/// wall-clock time, so it stays deterministic under test.
pub fn datetime_header(now_ms: u64) -> String {
    format!("Today's date is {} (UTC).\n\n", format_utc_date(now_ms))
}
```

### Task 3 — compose the header at the call site

In `executor/src/agent/mod.rs`, replace the single line at **`mod.rs:115`**:

```rust
let system = prompt::assemble_system_prompt(deps.commands, &input.standards, &input.phase_doc);
```

with the header prepended (read the clock once, prepend, then the existing
assembly):

```rust
let system = format!(
    "{}{}",
    prompt::datetime_header((deps.clock)()),
    prompt::assemble_system_prompt(deps.commands, &input.standards, &input.phase_doc),
);
```

This is the **only** edit to `mod.rs`. The header lands before `# Executor
contract`, so it is the first thing the model reads.

### Task 4 — format the two touched files

Format only the touched files — **never** run the writing form `cargo fmt --all`:

```bash
rustfmt executor/src/agent/prompt.rs executor/src/agent/mod.rs
```

### Step — verify

```bash
cargo build -p rexymcp-executor 2>&1 | tail -5
cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail -5
cargo test -p rexymcp-executor 2>&1 | tail -3
cargo fmt --all --check
```

## Acceptance criteria

- [ ] `executor/src/agent/prompt.rs` has a private `format_utc_date(now_ms: u64)
  -> String` and a public `datetime_header(now_ms: u64) -> String`, with the
  bodies as specified.
- [ ] `assemble_system_prompt`'s signature is **unchanged** (still three params)
  and its three existing tests are untouched.
- [ ] `executor/src/agent/mod.rs:115` prepends `prompt::datetime_header((deps.clock)())`
  to the assembled prompt via `format!`; it is the only `mod.rs` change.
- [ ] `cargo build -p rexymcp-executor` succeeds with zero warnings.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes.
- [ ] `cargo test -p rexymcp-executor` passes; the *passed* count is the
  pre-flight count + the new tests, and the **`2 ignored`** count is unchanged.
- [ ] `cargo fmt --all --check` passes.
- [ ] No new dependency added (`git diff Cargo.toml` is empty for both crates).

## Test plan

All tests go in the existing `#[cfg(test)] mod tests` block in `prompt.rs`.
These fixtures were computed from real UTC dates and the formatter was verified
against every one of them — they are the regression guard, especially the
negative/boundary cases (a naive or off-by-one implementation fails the last
three):

- `format_utc_date_formats_midnight_epoch_millis` — `format_utc_date(1_780_963_200_000)`
  equals `"2026-06-09"`.
- `format_utc_date_truncates_time_of_day` — `format_utc_date(1_781_012_730_000)`
  (2026-06-09 **13:45:30** UTC) equals `"2026-06-09"`; asserts the time-of-day is
  dropped, not rolled forward.
- `format_utc_date_handles_leap_day` — `format_utc_date(1_709_208_000_000)`
  equals `"2024-02-29"` (leap-day boundary).
- `format_utc_date_handles_epoch_zero` — `format_utc_date(0)` equals
  `"1970-01-01"`.
- `format_utc_date_does_not_roll_over_at_year_boundary` —
  `format_utc_date(1_767_225_599_000)` (2025-12-31 **23:59:59** UTC) equals
  `"2025-12-31"`, **not** `"2026-01-01"`; the strongest mutation guard.
- `datetime_header_contains_grounding_line` —
  `datetime_header(1_780_963_200_000)` contains the exact substring
  `"Today's date is 2026-06-09 (UTC)."`.

`assemble_system_prompt` itself is unchanged, so no new test is needed for it.
The call-site `format!` composition in `mod.rs` is pure plumbing (STANDARDS §3.2)
— the header content and formatting are fully covered by the `datetime_header`
test above.

## End-to-end verification

This phase ships a runtime-loadable behavior change: the system prompt sent to
the live model now opens with the date line. The hermetic path is covered by the
unit tests; the live path is not headlessly reproducible (it requires dispatching
a real phase against the local LLM and inspecting the system prompt the model
received). Verify by inspection at review instead:

> Confirm at review by reading the assembled `system` string composition at
> `executor/src/agent/mod.rs:115` and the `datetime_header` body — the date line
> precedes `# Executor contract`. No CLI/binary entrypoint exposes the system
> prompt directly, so quoting live output is not applicable; the deterministic
> `datetime_header_contains_grounding_line` test stands in for the wire content.

## Authorizations

None. (No new dependency — the date is formatted with pure integer arithmetic;
no `unsafe`; no edit to `Cargo.toml`, the architecture doc, or any other phase
doc. The architecture-doc M11 amendment and the README phase-table row for this
phase are made by the **architect at kickoff**, not by the executor — do not
touch `docs/architecture.md` or other phases.)

## Out of scope

- Do **not** add `chrono`, `time`, or any date/time dependency. The pre-injected
  integer formatter is the prescribed method.
- Do **not** change `assemble_system_prompt`'s signature or its three existing
  tests. The header is composed additively at the call site.
- Do **not** add a timezone option, a configurable format, or a time-of-day
  component. `YYYY-MM-DD` UTC is the whole feature.
- Do **not** touch `contract.rs`, the executor contract template, or any file
  other than `prompt.rs` and `mod.rs`.
- Do **not** edit `docs/architecture.md` or any milestone/phase doc (the
  architect handles the M11 amendment at kickoff).

## Notes for executor

- **Why pure integer math, not `chrono`:** the workspace has no date crate and
  adding one for a single line is an unauthorized design decision. The
  civil-from-days routine above is the standard, verified conversion — paste it
  verbatim.
- **Additive shape:** by adding `datetime_header` and composing at the call site
  rather than widening `assemble_system_prompt`, the three existing prompt tests
  need **no** edits. Do not "tidy" them to take a date argument — that would be
  scope creep and break the additive design.
- The clock is **milliseconds**, so the formatter divides by `1_000` first, then
  by `86_400`. Do not divide by `86_400` alone (that would treat ms as seconds).
- If `cargo test` shows the `2 ignored` count changed, you touched something you
  should not have — the two ignored tests live in `governor/verifier_tests.rs`
  and are unrelated to this phase.
- Commit as a single `feat:` commit; the body explains *why* (the model has no
  clock and stamps hallucinated Update Log dates), not *what*.

## Update Log

<!-- entries appended below this line -->

### Update — 2025-07-10 00:00 (started)

**Executor:** Claude (direct)

Implementing `format_utc_date` and `datetime_header` in `prompt.rs`, composing at the call site in `mod.rs`.

### Update — 2025-07-10 00:00 (complete)

**Summary:** Added `format_utc_date(now_ms: u64) -> String` (private) and `datetime_header(now_ms: u64) -> String` (public) to `executor/src/agent/prompt.rs`. The date formatter uses the prescribed civil-from-days integer algorithm verbatim. The header is composed additively at the single call site in `executor/src/agent/mod.rs` via `format!`, prepending the date line before `# Executor contract`. No changes to `assemble_system_prompt`'s signature or its three existing tests.

**Acceptance criteria:** all ticked above.

**Commands:**

```
cargo fmt --all --check
(exit 0, clean)

cargo build -p rexymcp-executor 2>&1 | tail -5
(exit 0, zero warnings)

cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail -5
(exit 0, clean)

cargo test -p rexymcp-executor 2>&1 | grep "test result"
test result: ok. 671 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 6.07s
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

Verification summary: build clean (zero warnings), clippy clean, tests 671 passed / 2 ignored (up from 665, +6 new tests), fmt clean.

**End-to-end verification:**

Per phase doc: "Confirm at review by reading the assembled `system` string composition at `executor/src/agent/mod.rs:115` and the `datetime_header` body — the date line precedes `# Executor contract`. No CLI/binary entrypoint exposes the system prompt directly, so quoting live output is not applicable; the deterministic `datetime_header_contains_grounding_line` test stands in for the wire content."

Grep proof for spec-pinned literal:
```
$ grep -n "Today's date is" executor/src/agent/prompt.rs
49:    format!("Today's date is {} (UTC).\n\n", format_utc_date(now_ms))
86:        assert!(header.contains("Today's date is 2026-06-09 (UTC)."));
```

**Files changed:**
- `executor/src/agent/prompt.rs` — added `format_utc_date`, `datetime_header`, and 6 unit tests
- `executor/src/agent/mod.rs` — composed `datetime_header` at the `execute_phase` call site

**New tests:**
- `format_utc_date_formats_midnight_epoch_millis` in `prompt.rs`
- `format_utc_date_truncates_time_of_day` in `prompt.rs`
- `format_utc_date_handles_leap_day` in `prompt.rs`
- `format_utc_date_handles_epoch_zero` in `prompt.rs`
- `format_utc_date_does_not_roll_over_at_year_boundary` in `prompt.rs`
- `datetime_header_contains_grounding_line` in `prompt.rs`

**Commits:**
- `f1c30dd` — feat: inject current date into executor system prompt

**Notes for review:** None — implementation matches spec verbatim.
