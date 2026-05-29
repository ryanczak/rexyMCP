# Phase 07d: read-before-edit invariant

**Milestone:** M4 — Headless agent loop + governor/verifier
**Status:** review
**Depends on:** phase-07a (loop + dispatch), 07c (the edit-class dispatch site +
`edit_target`). All done.
**Estimated diff:** ~220 lines (working-set + gate + tests)
**Tags:** language=rust, kind=feature, size=m

## Goal

Stop the executor from patching blind. The loop refuses a `patch` on a file the
model **hasn't read this session**, or one that **changed on disk underneath it**
since the read — the read-before-edit invariant the architecture lists as M4 loop
work. This protects against a weak model editing a file it never inspected (or one
a concurrent process/tool mutated), which is exactly how a blind `patch` corrupts
code. A refusal is a **model-visible** outcome (fed back so the model reads first),
never an `Err`.

## Architecture references

Read before starting:

- `docs/architecture.md` § Status, M4: "The **read-before-edit invariant** is
  enforced by the loop … `patch` refuses a file the executor hasn't read this
  session or that changed on disk underneath it."
- `docs/dev/STANDARDS.md` § "Pin negative cases" (and the M2 bug-04-1 / bug-05-1
  calibration): a confinement/refusal invariant lives or dies by its negatives —
  this phase pins the *must-refuse* cases, not just the happy path.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture reference and the M4 README.
3. Read this entire phase doc before touching any code.
4. Read:
   - `executor/src/agent/mod.rs` — the dispatch site, `edit_target` (07c, resolves
     a `write_file`/`patch` `"path"` against `project_root`), and how a failed
     tool call is already fed back model-visibly (`(succeeded, content)`).
   - `executor/src/tools/read_file.rs` and `tools/patch.rs` — tool names
     (`"read_file"`, `"patch"`) and that both take a `"path"` arg (absolute or
     project-root-relative).
5. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

The loop dispatches `patch` unconditionally. `read_file` dispatch records nothing.
There is no working-set. `edit_target` (07c) already resolves an edit-class call's
path against `project_root`; reuse its resolution so the gate and the recorder
agree on the key.

## Spec

All edits are in `executor/src/agent/**`. The gate is **`patch`-only** — `write_file`
creates/overwrites whole files (often new ones the model could not have read), so
it is **not** gated (pin this as a negative).

### 1. The gate — a pure helper (the testable seam)

```rust
fn read_before_edit_refusal(
    tool_call: &ToolCall,
    working_set: &HashMap<PathBuf, SystemTime>,
    project_root: &Path,
) -> Option<String>;
```

- Returns `None` for any call that is not `patch` (no gate).
- For `patch`: resolve the `"path"` against `project_root` (reuse 07c's resolution
  — factor it if helpful). Then:
  - **Not in `working_set`** (never read this session) → `Some(refusal)` naming the
    path and instructing the model to `read_file` it first.
  - In `working_set` but the file's **current mtime** (`fs::metadata(path)?
    .modified()`) **differs** from the recorded `SystemTime` (or the file can no
    longer be stat'd) → `Some(refusal)` saying it changed on disk since the read
    and must be re-read.
  - Recorded mtime **equals** current → `None` (allowed).

Keeping the gate a pure function over an explicit `working_set` makes the
"changed-underneath" case unit-testable without mid-session filesystem hooks.

### 2. Working-set bookkeeping (in the loop)

Maintain `working_set: HashMap<PathBuf, SystemTime>` keyed by the **resolved
absolute** path:

- After a **successful `read_file`** dispatch: resolve its `"path"`, stat the
  mtime, and insert/update the entry. Best-effort — if the stat fails, skip
  (don't error).
- After a **successful `patch`** dispatch: update the entry to the **post-patch**
  mtime (the loop just wrote the file, so a follow-up `patch` is allowed without a
  re-read).

### 3. Wiring the gate into dispatch

At the dispatch site, **before** the 07c pre-dispatch baseline capture and the
`dispatch` call, evaluate `read_before_edit_refusal`. If it returns `Some(msg)`:

- Treat it exactly like a failed tool call: `(succeeded, content) = (false, msg)` —
  do **not** execute the `patch`, do **not** capture a baseline, do **not** run the
  verifier (no edit happened).
- Still: log the `ToolResult` (07b), `scorer.record(name, false)`, push the
  `ToolCallSnapshot`, `append_tool_exchange` (so the refusal is fed back), and run
  the **hard-fail check + turn cap** (a model that repeats a refused `patch` should
  trip `IdenticalToolCallRepetition`).

If it returns `None`, proceed exactly as 07c does (baseline → dispatch → verify).

The cleanest shape: compute `refusal` once, then branch the `(succeeded, content)`
binding (refusal vs. real dispatch) and gate the baseline/verify block on "an edit
actually ran." Structure is your call; pin only the behavior.

### 4. Error model

- A refusal is **model-visible** (`content` fed back), never `Err`.
- mtime stat failures inside the gate → treat as "changed/unavailable" → refuse
  (the model re-reads); do not `unwrap`/panic.
- No `.unwrap()` / `.expect()` / `panic!()` in the loop or gate.

## Acceptance criteria

- [ ] `read_before_edit_refusal` returns `None` for non-`patch` calls (incl.
      `write_file`), `None` for a `patch` whose path was read and is unchanged,
      and `Some(_)` for an unread path **and** for a path whose recorded mtime ≠
      current mtime.
- [ ] A `patch` on a file not read this session is **refused** (fed back, file
      unchanged on disk, the `patch` tool never runs).
- [ ] A `patch` after a successful `read_file` of the same file is **allowed**
      (executes).
- [ ] `write_file` on a never-read path is **allowed** (not gated) (**negative**).
- [ ] A refused `patch` still records a snapshot / scores / feeds back and is
      subject to hard-fail + turn-cap (no early `continue` that bypasses them).
- [ ] No new dependency; no `tracing`; `governor`/`phase`/`tools` unmodified;
      completion artifacts (07e) not added.
- [ ] All four required commands pass with zero new warnings.

## Test plan

Hermetic (`TempDir`), deterministic. **Unit-test the pure gate** (this is where the
mtime-mismatch negative lives, via a hand-built `working_set`); **integration-test**
the wired read→allow / no-read→refuse paths through `execute_phase` (inject the
`NoopVerifier`/`MockFileVerifier` from 07c; use `.txt` targets so the verifier
stays out of the way).

**Gate unit tests:**
- `gate_allows_non_patch_calls` — a `write_file` (and a `read_file`) → `None`.
- `gate_refuses_patch_of_unread_file` (**negative**).
- `gate_allows_patch_of_read_unchanged_file` — `working_set` holds the file's
  actual current mtime → `None`.
- `gate_refuses_patch_when_mtime_changed` (**negative**) — `working_set` holds a
  stale `SystemTime` (e.g. `UNIX_EPOCH`) for an existing file → `Some(_)`.

**Loop integration tests:**
- `patch_without_prior_read_is_refused` (**negative**) — model patches an existing
  file it never read; assert the file's on-disk content is unchanged and a refusal
  was fed back.
- `patch_after_reading_is_allowed` — `read_file` then `patch` the same file; assert
  the edit landed (content changed).
- `write_file_without_read_is_allowed` (**negative for the gate**) — a `write_file`
  to a never-read path executes.
- `repeated_refused_patch_trips_hard_fail` — three identical refused `patch` calls
  → `IdenticalToolCallRepetition` (proves the refusal path still feeds the
  hard-fail detector).

## End-to-end verification

> Not applicable — phase ships no runtime-loadable artifact. The gate is exercised
> by unit tests (pure function) and `MockAiClient*` loop integration tests over a
> `TempDir`. The first live run is M5.

## Authorizations

- [x] **May modify** `executor/src/agent/**` (loop + tests).
- [ ] **No new dependencies**; no `tracing`. (`std::fs`/`SystemTime` only;
      `File::set_modified` is available if a test needs to age a file, though the
      gate unit test can just hand-build a stale `working_set` entry.)
- [ ] May **NOT** modify `executor/src/tools/**`, `governor/**`, `phase/**`,
      `Cargo.toml`, `docs/architecture.md`, `STANDARDS.md`, `WORKFLOW.md`, or
      another phase doc.

## Out of scope

- **Completion artifacts** — final command set, diff, `files_changed` /
  `command_outputs`, log-path surfacing — **07e** (the last 07 sub-phase).
- **Gating `write_file`** — out by design (whole-file create/overwrite).
- **Content-hash change detection** — mtime is the specified signal; do not add
  hashing.
- **The `scorer` consumer** — phase-08.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-05-29 (started)

**Executor:** Claude Code (direct) — pre-routed off opencode per NEXT.md.

Adding a `working_set: HashMap<PathBuf, SystemTime>` to the loop, a pure
`read_before_edit_refusal` gate (patch-only; refuse unread / mtime-mismatch), and
wiring: record mtime after a successful `read_file`/`patch`, evaluate the gate
before the 07c baseline/dispatch (a refusal → `(false, msg)`, no dispatch, no
verify, but still snapshot/score/feedback/hard-fail). Factoring a `resolve_path`
helper shared by `edit_target`, the recorder, and the gate. Unit tests on the gate
(incl. the mtime-mismatch negative via a stale working-set) + loop integration
tests.

### Update — 2026-05-29 (complete)

**Summary:** Added the read-before-edit invariant. A `working_set:
HashMap<PathBuf, SystemTime>` is keyed by resolved path; a successful `read_file`
or `patch` records the file's mtime. A pure `read_before_edit_refusal(tool_call,
working_set, project_root) -> Option<String>` gate (patch-only) refuses an unread
path or one whose current mtime differs from what was recorded. Wired at the
dispatch site **before** the 07c baseline/dispatch: a refusal yields `(false,
refusal)` — no baseline, no dispatch, no verify — but still logs the `ToolResult`,
scores, snapshots, feeds back, and runs the hard-fail + turn-cap checks (so a
repeated refused `patch` trips `IdenticalToolCallRepetition`). Factored a shared
`resolve_path` helper (used by `edit_target`, the recorder, and the gate). The
existing `succeeded` guard on the 07c verify block already excludes refusals (a
refusal sets `succeeded = false`), so no extra flag was needed. `write_file` is
not gated. Also registered the `patch` tool in the test registry (it was absent,
which the first on-disk patch assertion surfaced). No deviations from the spec.

**Acceptance criteria:** all met.

**Commands:**

```
cargo fmt --all --check
(no output — clean)

cargo build 2>&1 | tail -1
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.66s

cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail -1
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.13s

cargo test 2>&1 | grep "test result:" (lib line)
test result: ok. 469 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out
```

(469 = 461 prior + 8 read-before-edit tests. `agent::` alone: 44 passed.)

**End-to-end verification:**

Not applicable — phase ships no runtime-loadable artifact. The gate is unit-tested
as a pure function (incl. the mtime-mismatch negative via a hand-built stale
working-set) and the wired refuse/allow paths are exercised through `execute_phase`
over a `TempDir`. First live run is M5.

**Files changed:**
- `executor/src/agent/mod.rs` — `working_set` state; `resolve_path` /
  `read_before_edit_refusal` / `record_mtime` helpers; gate wired into dispatch +
  working-set bookkeeping; `patch` registered in the test registry; 8 new tests.

**New tests:** `gate_allows_non_patch_calls`, `gate_refuses_patch_of_unread_file`
(neg), `gate_allows_patch_of_read_unchanged_file`,
`gate_refuses_patch_when_mtime_changed` (neg), `patch_without_prior_read_is_refused`
(neg), `patch_after_reading_is_allowed`, `write_file_without_read_is_allowed`
(neg-for-gate), `repeated_refused_patch_trips_hard_fail`.

**Commits:** (pending — committed below)

**Notes for review:**
- The gate is a pure fn over an explicit `working_set`, so the changed-underneath
  case is unit-tested deterministically (stale `SystemTime::UNIX_EPOCH` entry) — no
  mid-session filesystem hook needed.
- A refusal flows through the same model-visible failure path as any failed tool
  (snapshot/score/feedback/hard-fail), so it is **not** an `Err` and still bounds
  loops.
- Registering `patch` in `registry_over` was needed for the on-disk patch
  assertion (07a/b/c never dispatched a real `patch`); harmless to other tests.
- `scorer.record` consumer still pending → phase-08.

verification: fmt OK · clippy OK · tests 469 passed · build OK
