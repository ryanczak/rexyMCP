---
name: review
description: >
  Review a completed phase against STANDARDS.md DoD, rerun the project's
  commands, and either approve (flip to done) or file a bug.
model: opus
argument-hint: "<phase>"
allowed-tools: Read, Write, Edit, Glob, Grep, Bash(*)
---

# Review Skill

This skill is the **substantive review gate**. It re-runs the project's
commands, walks the Definition of Done from `STANDARDS.md`, spot-checks
tests, and either writes the Review verdict (flip to `done`) or files a bug
(flip back to `in-progress`). It does not execute the phase itself — that is
the executor's job. On `hard_fail` or `budget_exceeded` results, it delegates
to `/rexymcp:escalate` rather than reviewing.

## Read these first

Before any action:

1. Read the phase doc (resolve `<phase>` from the argument). Its `Status:`
   line **must** be `review`. If it is `todo`, `in-progress`, or `done`, stop
   and tell the user the phase is not in a reviewable state.
2. Read `<repo>/docs/dev/STANDARDS.md` — the Definition of Done checklist
   (§1) is what you verify against.
3. Read the milestone README (`docs/dev/milestones/M<n>-<slug>/README.md`)
   for the phase table — status must mirror the phase doc.
4. Read the returned `PhaseResult`. If the dispatch skill passed it in
   context, use that. If not available, query via `executor_log_tail` +
   `get_turn` MCP tools using the phase's `log_path` from `rexymcp.toml`.

The repo root is `<repo>` — resolve it from `CLAUDE_PROJECT_DIR`, `ANTIGRAVITY_PROJECT_DIR`, or the
nearest directory containing the milestone layout.

## 1. Refuse non-review-status phases

If the phase doc's `Status:` line is not `review`, stop. Tell the user:
"This phase is not in `review` status (current status: `<actual>`). It must
be reviewed by the executor first — re-dispatch via `/rexymcp:dispatch
<phase>` if it is still `todo`."

## 2. Refuse hard_fail / budget_exceeded results

Check `PhaseResult.status`:

- If `"hard_fail"` or `"budget_exceeded"`: this is an escalation, not a
  review. Point the user at `/rexymcp:escalate <phase>` and stop. Do not
  attempt to review partial work.
- If `"complete"`: proceed to §3.

## 3. Re-run the command set

Read `<repo>/rexymcp.toml`'s `[commands]` section. Run each command in
sequence as **separate invocations** (not chained with `&&` — chaining
against the same build cache can race and produce spurious failures):

1. `format` — e.g. `<format command from rexymcp.toml>`
2. `build` — e.g. `<build command from rexymcp.toml>`
3. `lint` — e.g. `<lint command from rexymcp.toml>`
4. `test` — e.g. `<test command from rexymcp.toml>`

Run each in the repo root. Capture the output. If any command fails, note
it — the executor's `PhaseResult.command_outputs` is the executor's run;
this is your independent re-run. If the executor passed a command but you
fail, surface it as a possible environment-vs-spec mismatch.

## 4. Walk the DoD checklist

For each box in `STANDARDS.md` §1 (Definition of Done), verify it is met by
inspecting the diff, the phase doc, and the test output:

- [ ] All tasks in the phase doc's Spec section are implemented.
- [ ] Every acceptance criterion in the phase doc is verifiably met.
- [ ] Acceptance criteria referencing real artifacts are verified end-to-end
      against those artifacts (not just unit-test fakes).
- [ ] `{BUILD_COMMAND}` succeeds with zero new warnings.
- [ ] `{LINT_COMMAND}` passes.
- [ ] `{FORMAT_COMMAND}` passes.
- [ ] `{TEST_COMMAND}` passes (existing + new tests).
- [ ] New code is covered by tests per `STANDARDS.md` §3.
- [ ] No `TODO` / `FIXME` / `XXX` in code (unless the phase doc explicitly
      authorizes one).
- [ ] No `dbg!`, `println!` debug calls, or commented-out code.
- [ ] No new `unwrap()` / `expect()` / `panic!()` in production paths.
      Grep for them: `grep -rnE '\.(unwrap|expect)\(|panic!\(' <new-files>`.
- [ ] No new `unsafe` blocks.
- [ ] No `#[allow(...)]`, `#[ignore]`, or lint-silencing shims to mask
      diagnostics.
- [ ] Architecture doc updated only if the phase required it.
- [ ] Phase doc's Update Log is filled in.
- [ ] One conventional commit per logical change.

Pay extra attention to the `unwrap()` / `expect()` / `panic!()` grep, the
`#[allow]` check, and test coverage. These are the most common ways to paper
over a failing diagnostic.

## 5. Spot-check tests are real

Pick one or two new tests from the phase. Confirm they would actually fail
if the code under test were broken. A test that passes after mentally
deleting its assertion is a fake test — flag it.

Look for:
- Tests that assert nothing (or only trivial properties).
- Tests that use `unwrap_or_default()` on the `Result` they care about.
- Tests that mock everything so thoroughly the code path is never exercised.

## 6. Walk the phase doc's Acceptance criteria

Every checkbox in the phase doc's "Acceptance criteria" section should be
verifiable. Verify each one. If an acceptance criterion references a command
output, run the command. If it references a file, read the file. If it
references a test, run the test.

## 7. On pass

When all DoD boxes are checked and all acceptance criteria are met:

a. **Write the Review verdict** block at the bottom of the phase doc's
   Update Log (append after the `<!-- entries appended below this line -->`
   comment). Use the template from `WORKFLOW.md` § "Review verdict":

   ```markdown
   ### Review verdict — YYYY-MM-DD

   - **Verdict:** approved_first_try
   - **Bounces:** none
   - **Executor:** <executor model name | Claude Code (direct)>
   - **Scope deviations:** none
   - **Calibration:** none
   ```

   Adjust `Verdict` to `approved_after_N` if there were prior bounces, or
   `escalated` if this was a takeover.

a-bis. **Record the verdict in the telemetry store** so the scorecard sees
   it. Run (absolute `--phase-doc` so it matches the stored run identity):

   `rexymcp review --config <repo>/rexymcp.toml --phase-doc <abs phase-doc path>
   --phase-id <phase short id> --verdict approved_first_try --failure-class none`

   Use `approved_after_N` and the real bounce/bug counts (`--bounces N
   --bugs-filed N`) and the matching `--failure-class` from the taxonomy when
   there were prior bounces.

b. **Flip the phase doc's `Status:` line** from `review` to `done`.

c. **Update the milestone README's phase-table row** to `done`.

d. **Update `<repo>/docs/dev/NEXT.md`**: if this was the milestone's last
   in-scope phase, set the active phase to "none". Otherwise, leave it for
   `/rexymcp:architect next`.

e. **Commit** with a conventional commit message:
   `docs: approve <milestone> <phase> (done, <verdict>)`.

f. **Stop.** Do not draft the next phase. The user advances explicitly via
   `/rexymcp:architect next`. (Per the architect skill's §6 prohibition #2:
   no auto-advance.)

## 8. On fail

When one or more DoD boxes or acceptance criteria are not met:

a. **Write a bug report** at
   `<repo>/docs/dev/milestones/M<n>-<slug>/bugs/bug-<phase>-<n>.md`. Use the
   template from `WORKFLOW.md` § "Bug report template":

   ```markdown
   # Bug <n> on phase-<phase>: <One-line title>

   **Severity:** blocker | major | minor | nit
   **Status:** open
   **Filed:** YYYY-MM-DD

   ## What's wrong
   <Concrete. Quote the offending code with file:line. State observed behavior.>

   ## What should happen
   <Concrete. Reference the architecture doc section or phase spec requirement.>

   ## How to fix
   <Specific instruction: file path, what to change, expected result.>

   ## Verification
   - [ ] <command produces expected output>
   - [ ] <test_name passes>
   ```

   Choose severity per `WORKFLOW.md` § "Severity meanings":
   - **blocker** — phase cannot be merged in this state.
   - **major** — must fix before done; correctness or contract violation.
   - **minor** — should fix; style, naming, a missing-but-not-critical test.
   - **nit** — optional preference; executor may decline with reasoning.

b. **Flip the phase doc's `Status:` line** from `review` back to
   `in-progress` (with a note referencing the bug).

c. **Update the milestone README's phase-table row** to `in-progress`.

c-bis. **Record the bounce in the telemetry store:** `rexymcp review --config
   <repo>/rexymcp.toml --phase-doc <abs phase-doc path> --phase-id <phase short
   id> --verdict bounced --bugs-filed 1 --failure-class <class from taxonomy>`

d. **Commit** with a conventional commit message:
   `docs: bounce <milestone> <phase> — bug-<n>-<n> (<short summary>)`.

e. **Tell the user:** "Bounced. Re-dispatch via `/rexymcp:dispatch <phase>`
   once the bug is fixed."

f. **Stop.** Do not fix the bug yourself — that is the executor's job.

## 9. On milestone close

If this phase was the milestone's last in-scope phase and all sibling phases
are `done`:

- Write the **milestone retrospective** in the README's Notes section — what
  worked, what broke, calibration data.
- Fold any calibration lessons into `WORKFLOW.md` (with user sign-off) per
  `WORKFLOW.md` § "Calibration — fold lessons in": one occurrence is data,
  two is a trend, three is a fix.
- Update `<repo>/docs/dev/NEXT.md` to "none".
- Tell the user the milestone is complete and ask whether to proceed to the
  next milestone. The user kicks off the next milestone explicitly.

## 10. What you do not do

- You do **not** execute the phase. If the phase needs fixing, bounce it
  back to the executor — do not implement it yourself.
- You do **not** auto-advance to the next phase. The user gates each step.
- You do **not** review `hard_fail` or `budget_exceeded` results — those go
  to `/rexymcp:escalate`.
- You do **not** modify `STANDARDS.md` or `WORKFLOW.md` without explicit
  user approval and a recurring-pattern fold.
