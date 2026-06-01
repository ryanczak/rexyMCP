# Engineering Standards

**Read this at the start of every phase, before reading the phase doc.**

This document is the contract between executor LLMs and the principal engineer.
If you (the executor) follow these standards, your work will pass review. If you
skip them, your work will bounce back with a bug report and you'll do it twice.

The goal is not bureaucracy — it's so the principal engineer can review your
output by checking a finite list, not by re-reading the whole architecture doc.

> **Command resolution.** This document uses placeholders for the project's
> commands, resolved per target project from configuration:
>
> | Placeholder | Meaning |
> |---|---|
> | `{FORMAT_COMMAND}` | Format-check command (verify only, no write) |
> | `{BUILD_COMMAND}` | Build command |
> | `{LINT_COMMAND}` | Lint/static-analysis command |
> | `{TEST_COMMAND}` | Test command |

---

## 1. Definition of Done

A phase is **not** done until every box below is checked. If you cannot tick a
box, the phase is **in-progress** or **blocked**, not done. Report blockers in
the phase's Update Log — never silently mark a phase done.

- [ ] All tasks in the phase doc's Spec section are implemented.
- [ ] Every acceptance criterion in the phase doc is verifiably met.
- [ ] Every acceptance criterion that references a real artifact the phase ships
      (a checked-in file, a CLI behavior, a config the running binary loads) has
      been **verified end-to-end against that real artifact** — not just against
      a unit-test fake — and the actual output is quoted in the completion
      Update Log entry under "End-to-end verification." A green `{TEST_COMMAND}`
      run that exercises a temp-directory-scoped fake is **not** by itself sufficient.
- [ ] `{BUILD_COMMAND}` succeeds with **zero new warnings**.
- [ ] `{LINT_COMMAND}` passes.
- [ ] `{FORMAT_COMMAND}` passes.
- [ ] `{TEST_COMMAND}` passes (existing + new tests).
- [ ] New code is covered by tests per the rules in §3.
- [ ] No `TODO` / `FIXME` / `XXX` left in code, unless the phase doc explicitly
      authorizes one (with a follow-up phase referenced).
- [ ] No debug calls or commented-out code.
- [ ] No error-suppressing idioms (`.unwrap()`, `.expect()`, `panic!()`, or
      language equivalents) in production paths. Test code is exempt. See §2.
- [ ] No `unsafe` blocks. (If you think you need one, stop and report a
      blocker — `unsafe` requires principal-engineer review.)
- [ ] Architecture doc updated **only if** the phase explicitly requires it.
      Otherwise leave it alone.
- [ ] Phase doc's Update Log filled in (see WORKFLOW.md).
- [ ] One conventional commit per logical change (see §6).

---

## 2. Code Quality

### 2.1 Error handling

Errors are split by audience:

- **Programmer / infrastructure failures** → a typed error enum
  (`Result<_, _>` or equivalent). Add a new variant only if no existing one fits.
- **Things the model is supposed to see and adapt to** (failed tool calls, parse
  failures, verifier disagreements) → a structured result value, **not**
  an exception or error. This is a normal outcome, not a programmer mistake.
- **A generic error-wrapping type** is acceptable at binary entry
  points (`main`) where errors propagate to user-visible output. Library code
  uses specific error types.
- **The language's propagation operator** is the default. Never suppress errors
  in production paths. A contextual message is acceptable when the value is set
  at compile-time or the invariant was already proven upstream.
- **Never** silently swallow a `Result` you don't want to ignore — that's how
  bugs hide.

### 2.2 What to write (and not write)

- **No new files unless the phase requires them.** Prefer editing.
- **No premature abstraction.** Three similar lines are better than a generic
  helper. Abstract when the *fourth* caller appears.
- **No error handling for cases that can't happen.** Trust framework guarantees
  and internal invariants. Validate at system boundaries (user input, external
  APIs, the model response), not internally.
- **No feature flags or back-compat shims** unless the phase calls for them.
- **No backwards-compatibility renames.** If a symbol is unused, delete it.
- **No fallbacks for "if X is missing."** Either X is required (fail loud) or it
  has a default (use it). No silent degradation.

### 2.3 Comments

Default: **write no comments.** Add one only when *why* is non-obvious — a hidden
constraint, a subtle invariant, a workaround for a known bug. If removing the
comment wouldn't confuse a future reader, don't write it.

Specifically forbidden: restating what the code does; "used by X" / "added for
Y" references (they rot); TODO/NOTE with no actionable instruction; block
comments above every function. Doc comments on public APIs are fine.

### 2.4 Naming

- Use the language's conventional naming style consistently.
- Test functions describe behavior in present tense
  (`loads_default_when_no_config_present`), not `test1` / `it_works`.

### 2.5 Module layout

- Public API at a single declared entry point per module.
- Internal helpers below public items.
- Unit tests co-located with source; integration tests in a dedicated test directory.
- Grouped imports following the language's conventions.

### 2.6 Dependencies

- **Do not add new dependencies** unless the phase doc authorizes it. Adding a
  dependency is a design decision, not an implementation choice. If you need one,
  **stop, report a blocker**, and wait for principal-engineer authorization.
- When lifting code from a donor project, lift the code — do **not** add the
  donor as a dependency.

---

## 3. Test Coverage

### 3.1 What requires a test

- **Every new pure function** (no side effects, no async): unit test.
- **Every new tool**: happy-path test + at least one failure-mode test.
  Hermetic via the language's temp-directory abstraction if it touches the filesystem.
- **Every new agent-loop integration point**: a mocked AI client integration test
  asserting both *what the loop did* and *what it asked the model*.
- **Every parser stage / repair rule**: a unit test with a real example of the
  malformed input it fixes, plus an example it correctly leaves alone.

### 3.2 What does not require a test

- Pure plumbing: a function that only constructs a struct from its fields or
  forwards args.
- Vendored backend wire-protocol code (covered upstream).
- Code paths the phase doc explicitly marks "stub, no behavior yet."

### 3.3 How tests are written

- One assertion per test where possible; multi-assertion tests need
  per-assertion messages.
- Tests are **hermetic**: no real network, no writes to the host home or shared
  locations. Use a mocked AI client for any AI-backend interaction.
- Tests are **deterministic**: no `sleep`, no real wall-clock time (inject a clock),
  no unseeded RNG. If a test can't be made deterministic, mark it as ignored
  and explain why in a comment on the test.
- **Inject subprocess / external-IO dependencies behind a trait seam.** Anything
  that would shell out or touch a real external process (a compiler/verifier, the
  final command set, the AI backend) goes behind an interface with a production impl and
  a test mock, so loop/integration tests stay hermetic and fast.

### 3.4 Live-model tests

Don't write them unless the phase doc explicitly asks. They are opt-in,
ignored by default, and never run on CI — they need a live endpoint and
carry setup the CI environment can't reliably provide. When the phase doc calls
for one, gate it as ignored and document how to run it.

---

## 4. Required Commands

Run these locally before reporting a phase done. Output of the full sequence
goes into the phase's Update Log.

```bash
{FORMAT_COMMAND}
{BUILD_COMMAND} 2>&1 | tail -20
{LINT_COMMAND} 2>&1 | tail -20
{TEST_COMMAND} 2>&1 | tail -30
```

Run `{LINT_COMMAND}` and `{TEST_COMMAND}` as **separate** invocations, not
chained — chaining them against the same target directory can race the build cache and produce spurious failures.

If any command fails, the phase is **not** done. Fix the issue or file a blocker;
do not paper over.

---

## 5. Files You Must Not Touch

Without explicit authorization in the phase doc:

- Build and configuration files (package manifests, linter config, CI workflows).
- The architecture doc (architecture changes go through the principal engineer).
- `STANDARDS.md`, `WORKFLOW.md` (these documents).
- Any milestone or phase doc other than the one you're executing.

If you think one of these needs to change to complete the phase, **stop and
report a blocker**.

---

## 6. Commits

- One conventional commit per logical change: `feat:`, `fix:`, `refactor:`,
  `test:`, `docs:`, `chore:`.
- The commit body explains *why*, not *what* (the diff shows what).
- One phase usually = one commit. Multi-commit phases are fine when the changes
  are genuinely independent.

---

## 7. When You Are Stuck

Stop. Do not improvise around an unclear spec or a missing dependency. File a
blocker in the phase doc's Update Log (template in WORKFLOW.md), then stop. The
principal engineer resolves it on the next review pass.

Always-blockers, never improvise:

- Need to add a dependency.
- Need to write `unsafe`.
- Spec is ambiguous between two valid implementations.
- An acceptance criterion is impossible as written.
- A required file referenced by the spec doesn't exist.
- A test reveals the design itself is wrong, not just your implementation.

---

## 8. Reporting Completion

1. Re-read this document, top to bottom.
2. Run all required commands in §4 and capture their output.
3. Fill in the phase's Update Log with the "complete" template (WORKFLOW.md).
4. Commit and stop. **Do not start the next phase.** The principal engineer
   reviews, then marks the phase **done** or files a bug report.

---

## 9. Source of Truth

When this document and the architecture doc disagree, the **architecture doc
wins**. When the phase doc and the architecture doc disagree, **stop and report a
blocker** — the phase doc has drifted from the design.
