# Engineering Standards

**Read this at the start of every phase, before reading the phase doc.**

This document is the contract between executor LLMs and the principal engineer.
If you (the executor) follow these standards, your work will pass review. If you
skip them, your work will bounce back with a bug report and you'll do it twice.

The goal is not bureaucracy — it's so the principal engineer can review your
output by checking a finite list, not by re-reading the whole architecture doc.

> **Command resolution.** This document uses placeholders for the project's
> commands. For rexyMCP (a Rust workspace) they resolve to:
>
> | Placeholder | Command |
> |---|---|
> | `{FORMAT_COMMAND}` | `cargo fmt --all` (CI runs `--check`) |
> | `{BUILD_COMMAND}` | `cargo build` |
> | `{LINT_COMMAND}` | `cargo clippy --all-targets --all-features -- -D warnings` |
> | `{TEST_COMMAND}` | `cargo test` |

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
      run that exercises a `TempDir`-scoped fake is **not** by itself sufficient.
- [ ] `{BUILD_COMMAND}` succeeds with **zero new warnings**.
- [ ] `{LINT_COMMAND}` passes.
- [ ] `{FORMAT_COMMAND}` passes.
- [ ] `{TEST_COMMAND}` passes (existing + new tests).
- [ ] New code is covered by tests per the rules in §3.
- [ ] No `TODO` / `FIXME` / `XXX` left in code, unless the phase doc explicitly
      authorizes one (with a follow-up phase referenced).
- [ ] No `dbg!`, `println!` debug calls, or commented-out code.
- [ ] No new `unwrap()` / `expect()` / `panic!()` in production paths (test code
      is exempt). See §2.
- [ ] No new `unsafe` blocks. (If you think you need one, stop and report a
      blocker — `unsafe` requires principal-engineer review.)
- [ ] Architecture doc (`docs/architecture.md`) updated **only if** the phase
      explicitly requires it. Otherwise leave it alone.
- [ ] Phase doc's Update Log filled in (see WORKFLOW.md).
- [ ] One conventional commit per logical change (see §6).

---

## 2. Code Quality

### 2.1 Error handling

rexyMCP splits errors by audience:

- **Programmer / infrastructure failures** → a `thiserror` error enum
  (`Result<_, _>`). Add a new variant only if no existing one fits.
- **Things the model is supposed to see and adapt to** (failed tool calls, parse
  failures, verifier disagreements) → a `ToolResult`-style value, **not**
  `Result::Err`. This is a normal outcome, not a programmer mistake.
- **A generic error-wrapping type** (`anyhow`) is acceptable at binary entry
  points (`main`) where errors propagate to user-visible output. Library code
  uses specific error types.
- **`?` is the propagation operator.** Never `.unwrap()` / `.expect()` in
  production paths. `.expect("…")` is acceptable when the value is set at
  compile-time (const / `OnceLock`) or the invariant was already proven upstream
  (give the `expect` message a sentence explaining the invariant).
- **Never** `unwrap_or_default()` on a `Result` you don't want to silently
  swallow — that's how bugs hide.

### 2.2 What to write (and not write)

- **No new files unless the phase requires them.** Prefer editing.
- **No premature abstraction.** Three similar lines are better than a generic
  helper. Abstract when the *fourth* caller appears.
- **No error handling for cases that can't happen.** Trust framework guarantees
  and internal invariants. Validate at system boundaries (user input, external
  APIs, the local-LLM response), not internally.
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
comments above every function. Doc comments (`///`) on public APIs are fine.

### 2.4 Naming

- `snake_case` for functions, variables, modules.
- `UpperCamelCase` for types, traits, enum variants.
- `SCREAMING_SNAKE_CASE` for consts and statics.
- Test functions describe behavior in present tense
  (`loads_default_when_no_config_present`), not `test1` / `it_works`.

### 2.5 Module layout

- Public API at a single declared entry point per module (`mod.rs`).
- Internal helpers below `pub` items.
- Unit tests in a `#[cfg(test)] mod tests { … }` block at the bottom of the same
  file; integration tests in `tests/`.
- Imports grouped `StdExternalCrate`, granularity `Module` (see `rustfmt.toml`).

### 2.6 Dependencies

- **Do not add new dependencies** unless the phase doc authorizes it. Adding a
  crate is a design decision, not an implementation choice. If you need one,
  **stop, report a blocker**, and wait for principal-engineer authorization.
- When lifting a module from Rexy, lift the code — do **not** add Rexy as a
  dependency. rexyMCP does not link against Rexy.

---

## 3. Test Coverage

### 3.1 What requires a test

- **Every new pure function** (no side effects, no async): unit test.
- **Every new tool**: happy-path test + at least one failure-mode test.
  Hermetic via `tempfile::TempDir` if it touches the filesystem.
- **Every new agent-loop integration point**: a `MockAiClient` integration test
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
  locations. Use `MockAiClient` for any `AiClient` interaction.
- Tests are **deterministic**: no `sleep`, no real `Utc::now()` (inject a clock),
  no unseeded RNG. If a test can't be made deterministic, mark it `#[ignore]`
  and explain why in a comment on the test.

### 3.4 Live-LLM tests

Don't write them unless the phase doc explicitly asks. They are opt-in,
`#[ignore]`-gated, and never run on CI — they need a live local endpoint and
carry setup the CI environment can't reliably provide. When the phase doc calls
for one, gate it `#[ignore]` and document how to run it.

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
chained with `&&` — chaining them against the same target directory can race the
build cache and produce spurious failures.

If any command fails, the phase is **not** done. Fix the issue or file a blocker;
do not paper over (no `#[allow]` to silence the linter, no `#[ignore]` to skip a
failing test).

---

## 5. Files You Must Not Touch

Without explicit authorization in the phase doc:

- `rustfmt.toml`, `clippy.toml`, `.github/workflows/*` (CI is principal-engineer
  territory).
- `docs/architecture.md` (architecture changes go through the principal engineer).
- `docs/dev/STANDARDS.md`, `docs/dev/WORKFLOW.md` (these documents).
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
2. Run all four commands in §4 and capture their output.
3. Fill in the phase's Update Log with the "complete" template (WORKFLOW.md).
4. Commit and stop. **Do not start the next phase.** The principal engineer
   reviews, then marks the phase **done** or files a bug report.

---

## 9. Source of Truth

When this document and the architecture doc disagree, the **architecture doc
wins**. When the phase doc and the architecture doc disagree, **stop and report a
blocker** — the phase doc has drifted from the design.
