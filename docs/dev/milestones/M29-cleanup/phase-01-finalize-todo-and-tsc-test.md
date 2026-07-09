# Phase 1: Finalize tolerates a `todo` start + hermetic tsc-resolver test

**Milestone:** M29 — Cleanup
**Status:** todo
**Depends on:** none
**Estimated diff:** ~120 lines
**Tags:** language=rust, kind=bugfix, size=m

## Goal

Two independent cleanup fixes, both found during the M28 dispatch/review:

1. Make `finalize_complete` complete the bookkeeping for a phase left at
   `**Status:** todo` (an executor that skipped the `todo→in-progress` start-flip),
   not only `in-progress`.
2. Replace the ETXTBSY-flaky write-then-exec tsc test with a deterministic test of
   the pure `resolve_tsc_command` resolver.

## Architecture references

Read before starting:

- `docs/architecture.md` § Status #27 — the server-authored finalize (M27 03a/04b)
  this extends; the thesis is that the *server* owns completion bookkeeping so a
  weak executor's bookkeeping misses don't strand a phase.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture reference above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

### Task 1 — `mcp/src/finalize.rs`

`finalize_complete` (line 22) guards on `status_is_in_progress(&doc)` and returns
early otherwise, so a `todo` doc is never finalized. The predicates and the flip:

```rust
    let doc = std::fs::read_to_string(inp.phase_doc_path)?;
    if !status_is_in_progress(&doc) {
        return Ok(false);
    }
// ...
fn is_in_progress_status(trimmed: &str) -> bool {
    trimmed == "**Status:** in-progress" || trimmed.starts_with("**Status:** in-progress ")
}
fn status_is_in_progress(doc: &str) -> bool {
    doc.lines().any(|line| is_in_progress_status(line.trim()))
}
// flip_status_to_review (line 69) replaces the first `is_in_progress_status` line
// with `**Status:** review`.
```

The README-row flip (`flip_readme_row`, line 176) only matches a last cell that
`starts_with("in-progress")` (line 187):

```rust
                        if last_cell.starts_with("in-progress") {
                            found = true;
                            format!("{}| review |{}", &line[..last_pipe], &line[last_pipe + 1..])
```

Existing tests to update: `status_is_in_progress_rejects_todo` (line ~291) asserts
`todo` is rejected — that expectation **inverts** under this phase.

### Task 2 — `executor/src/governor/verifier_tests.rs`

`verify_typescript_spawns_resolved_local_binary` (line 888) writes an executable
`node_modules/.bin/tsc` shell script and calls `verify_typescript`, which
**spawns** it — the write-then-exec ETXTBSY race. The pure resolver it exercises,
`resolve_tsc_command(project_root, npx_on_path) -> TscCommand`
(`executor/src/governor/verifier.rs:465`), is already hermetically testable:
`find_local_tsc` returns the local `node_modules/.bin/tsc` path if it exists as a
file, and `resolve_tsc_command` returns it as `program` with empty `prefix_args`.

## Spec

### 1. `finalize_complete` accepts `todo` as a pre-review status

In `mcp/src/finalize.rs`, broaden the "may be finalized to review" status from
`in-progress`-only to **`todo` OR `in-progress`** (each with an optional trailing
space-delimited note), in all three places that currently hard-code `in-progress`:

- the base predicate (`is_in_progress_status`) — match `**Status:** todo` and
  `**Status:** todo ` + note **in addition to** the existing in-progress forms;
- the flip (`flip_status_to_review`) — it replaces the first pre-review line, so it
  inherits the broadened predicate;
- the README-row match (`flip_readme_row`) — accept a last cell starting with
  `todo` **or** `in-progress`.

Renaming the predicate to a pre-review concept (e.g. `is_pre_review_status` /
`status_is_pre_review`) is encouraged for clarity but optional — pin the
**behavior**, not the names.

**Pin these negatives (must still NOT match / NOT flip):**
- `**Status:** review` and `**Status:** done` — finalize must stay dormant (the
  idempotency 03a relies on).
- `**Status:** todoish` and `**Status:** in-progressish` — the trailing space is
  the note delimiter; a bare-suffix look-alike must not match.

The flip still targets **only the first** matching line and leaves everything else
byte-identical (whitespace-preserving, drops a trailing `(bounced — …)` note as
today).

### 2. Replace the flaky tsc test with a pure resolver test

In `executor/src/governor/verifier_tests.rs`, **remove**
`verify_typescript_spawns_resolved_local_binary` (the write-then-exec test) and
replace it with a deterministic test of `resolve_tsc_command` that does **not**
spawn anything:

- Create a `TempDir`, `tsconfig.json`, and a `node_modules/.bin/tsc` **file**
  (contents irrelevant; it does **not** need to be executable — it is never
  exec'd).
- Call `resolve_tsc_command(project_root, /* npx_on_path */ false)` and assert the
  returned `TscCommand.program` equals the planted `node_modules/.bin/tsc` path and
  `prefix_args` is empty — i.e. the local binary is the resolved program (which is
  what `verify_typescript` then spawns).
- Keep it `#[cfg(unix)]` only if the path handling requires it; no async, no
  subprocess.

Optionally also pin the two fallbacks in the same or sibling tests (no local tsc +
`npx_on_path == true` → `npx --no-install tsc`; no local tsc + `false` → bare
`tsc`) — these are pure and cheap, and pin the resolution order the removed test
implicitly covered.

Do **not** change `resolve_tsc_command`, `find_local_tsc`, or `verify_typescript`
production code — only the test.

## Acceptance criteria

- [ ] `cargo build` (zero new warnings), `clippy`, `fmt --check`, and `cargo test`
      all pass — and a **second** back-to-back `cargo test` also passes (the flake
      must not recur).
- [ ] A finalize unit test proves a `**Status:** todo` doc + a `Complete` result is
      flipped to `**Status:** review` with the baseline entry appended.
- [ ] A finalize unit test proves the README row `| … | todo |` for the phase is
      flipped to `| … | review |`.
- [ ] Finalize still no-ops on `**Status:** review` and `**Status:** done`, and
      still does not match `todoish` / `in-progressish`.
- [ ] `resolve_tsc_command` test asserts the local `node_modules/.bin/tsc` path is
      the resolved `program` **without spawning** it; the old write-then-exec test
      is gone (`grep -n "spawns_resolved_local_binary" ` returns nothing).

## Test plan

- `finalizes_a_todo_doc_to_review` (finalize.rs tests) — a `todo` doc + a
  `Complete` `PhaseResult` → `flip` returns `true`, the on-disk doc reads
  `**Status:** review`, and a completion entry is appended. Mirror the existing
  `run_phase_with_finalizes_an_in_progress_doc_to_review` / the finalize tests'
  fixture style.
- Update `status_is_in_progress_rejects_todo` → the `todo` case is now **accepted**
  by the pre-review predicate; re-point the negative to assert `review`/`done` are
  rejected (and add `todoish`/`in-progressish` negatives if not already present).
- `flip_readme_row_flips_todo_cell` — a README table row ending `| todo |` for the
  phase file → `| review |`; a `| review |` row → `None` (unchanged).
- `resolve_tsc_command_prefers_local_binary` (verifier_tests.rs) — planted local
  `node_modules/.bin/tsc` file → `program` is that path, `prefix_args` empty, **no
  subprocess**.
- (optional) `resolve_tsc_command_falls_back_to_npx_then_bare` — no local tsc,
  `npx_on_path` true → `npx --no-install tsc`; false → bare `tsc`.

## End-to-end verification

> Not applicable as a standalone CLI E2E — both changes are internal (a server
> bookkeeping predicate and a test). The finalize behavior is exercised by the
> unit tests above calling the real `finalize_complete` / `flip_readme_row` against
> a `TempDir` doc (the shipped path), and the tsc change is test-only. State this
> in the completion entry. (A live confirmation — a CLI `run-phase` dispatch of a
> `todo` phase now landing at `review` with a bookkeeping commit — is the ultimate
> check but is non-hermetic and out of the gate.)

## Authorizations

None. (No new dependency; no `docs/architecture.md` edit — the milestone is
recorded in § Status.)

## Out of scope

- Changing the executor's start-flip behavior or the executor contract — the fix
  is server-side robustness, not making the model flip reliably.
- Wiring finalize into any *new* entry point — `run_phase_with` already calls it
  (`mcp/src/runner.rs:318`); this phase only broadens *which status* it acts on.
- Any change to `resolve_tsc_command` / `find_local_tsc` / `verify_typescript`
  production logic — Task 2 is test-only.
- The other finalize concerns (commit message, git staging) — unchanged.

## Update Log

(Filled in by the executor.)

<!-- entries appended below this line -->
