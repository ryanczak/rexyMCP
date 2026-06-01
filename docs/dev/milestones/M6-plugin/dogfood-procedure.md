# Dogfood procedure — M6 end-to-end

**Purpose:** Run the first real architect → dispatch → review cycle through the
rexyMCP plugin, without opencode in the picture. This exercises the full stack:
Claude Code (architect), the rmcp stdio server, the local LLM (executor), and
the review/escalate skills.

**Who runs this:** The project owner (human), with Claude Code as the architect.
The architect (rexyMCP's principal engineer) reads the filled-in log afterward
and synthesizes the M6 retrospective.

**Prerequisites:**
- Claude Code CLI installed and authenticated.
- A local LLM endpoint reachable via an OpenAI-compatible API (the executor's
  target model). Note the base URL, model name, and context window size.
- `rexymcp` binary installable from this repo (`cargo install --path mcp`).
- Git access to clone a target repo.

---

## Picking a target repo

**Required criteria:**
- **Small** — single-digit thousands of LOC, ideally under 5K.
- **Open-source** — legally clone-able / fork-able.
- **Active or recently-active** — last commit in the past year.
- **Language ≠ Rust** — the dogfood's main job is exercising `{…_COMMAND}`
  placeholder substitution beyond `cargo`. Python, Go, JavaScript, TypeScript
  all good choices.
- **Has a working test suite** — the review skill re-runs the project's command
  set; without tests there's nothing to verify.
- **Has detectable build/test commands** — the bootstrap routine inspects
  `Cargo.toml` / `package.json` / `pyproject.toml` / `go.mod` / etc.; pick a
  repo whose conventions match those detection rules.

**Recommended (not required):**
- A small open issue or feature request that could be a real first phase.
- An MIT/Apache/BSD-licensed project.
- Fewer than ~20 transitive dependencies (faster build/test re-runs).

**Do not** pick a specific repo based on a recommendation here — repos change,
get archived, acquire new policies. Criteria are stable; recommendations rot.

---

## Step 1 — Pre-dogfood setup (one-time)

**Do:**
1. From the rexyMCP repo, run `cargo install --path mcp --debug` (or
   `cargo install --path mcp` for a release binary).
2. Verify the binary installed: `rexymcp --version` and `rexymcp serve --help`.
3. Install the rexyMCP plugin into Claude Code. The plugin lives in the
   `plugin/` directory of this repo. Two options:
   - **Test mode (recommended for first run):** Start Claude Code with
     `claude --plugin-dir /path/to/rexyMCP/plugin`. This loads the plugin
     for that session only, no installation needed.
   - **Persistent install:** Copy the `plugin/` directory to
     `~/.claude/plugins/rexymcp/` so it's available in every session.
4. Configure your local LLM endpoint — note the base URL, model name, and
   context window size. The executor skill will use this when dispatching.
5. Clone the target repo to a fresh directory (not inside the rexyMCP repo).

**Observe:**
- Does `rexymcp serve --help` print cleanly?
- Does Claude Code start with the plugin loaded (test mode) or show the plugin
  in `/plugins` (persistent install)?

**Record:** In `dogfood-log.md` § Setup — versions of rexyMCP, Claude Code, the
model name, context size, target repo URL and commit hash, plugin install method.

**Stop and tell the architect if:**
- `rexymcp serve --help` fails (binary is broken).
- Claude Code doesn't recognize the plugin at all (plugin layout is wrong).

---

## Step 2 — Bootstrap phase

**Do:**
1. Open a fresh Claude Code session in the target repo's directory.
   - If using test mode: `cd /path/to/target-repo && claude --plugin-dir /path/to/rexyMCP/plugin`
   - If using persistent install: `cd /path/to/target-repo && claude`
2. Run `/rexymcp:architect` with no arguments.

**Observe:**
- Does the bootstrap routine detect the command set correctly? (e.g. for a
  Node.js project, does it find `package.json` and infer `npm test` / `npm run
  build`?)
- Does it prompt for confirmation before writing files?
- Does it write these files: `rexymcp.toml`, `docs/dev/STANDARDS.md`,
  `docs/dev/WORKFLOW.md`, `CLAUDE.md`?
- Does it register rexyMCP in `.mcp.json` (project scope)?
- **Critically:** does it **NOT** write `AGENTS.md` or an executor-contract
  file? (Architecture violation — rexyMCP projects carry no root executor
  contract file.)

**Record:** In `dogfood-log.md` § Bootstrap — what commands were detected vs
what required user override; which files got written; any unexpected prompts or
errors; whether `AGENTS.md` was incorrectly written.

**Stop and tell the architect if:**
- Bootstrap writes `AGENTS.md` or any executor-contract file to the target repo
  root (architecture violation).
- Bootstrap fails to write `rexymcp.toml` or `CLAUDE.md`.
- Bootstrap doesn't detect any commands from a standard project layout.

---

## Step 3 — Design phase (architect skill continues)

**Do:**
1. After bootstrap completes, engage Claude on the project's design. Tell it
   what the project is for and what the first milestone should unlock.
2. Let the architect skill work — it should explore the repo, write a design
   doc, create a milestone README, and draft `phase-01-<slug>.md`.

**Observe:**
- Does the architect write `docs/architecture.md` (or update an existing one)?
- Does it create a milestone README under `docs/dev/milestones/`?
- Does it draft a phase-01 doc with the standard sections (Goal, Spec,
  Acceptance criteria, etc.)?
- **Quality check:** how good is the pre-injection in the phase doc? Does it
  include worked examples, idioms, or reference docs the executor will need?

**Record:** In `dogfood-log.md` § Design — quality assessment of the
architecture doc (1–5 scale, with one-line rationale), quality of the milestone
decomposition, quality of the phase-01 draft (especially pre-injection).

**Stop and tell the architect if:**
- The architect writes a phase doc with no spec or no acceptance criteria.
- The phase doc's acceptance criteria reference artifacts the project doesn't
  have (e.g. a Rust test suite for a Python project).

---

## Step 4 — Dispatch phase

**Do:**
1. Run `/rexymcp:dispatch phase-01` (or whatever the phase doc is named).
2. Watch the session output.

**Observe:**
- Does `executor_health` confirm the local LLM is reachable?
- Does `execute_phase` invoke? What does the confirmation prompt look like?
- Do MCP progress notifications appear during execution? How often?
- What's the executor's behavior — does it follow the phase spec? Does it hit
  the Confirmation gate before coding?
- How many turns does the executor run?
- How long does it take wall-clock?
- **Compaction data:** how many compaction events fired? (Check the returned
  `PhaseResult` for `CompactionReport` totals, or run
  `executor_log_search { event_type: "compaction" }` if the loop logs them.)
  How early in the phase did they fire?
- Did the executor visibly lose track of state? (Re-asking for context it
  already had, repeating tool calls, editing the wrong file.)

**Record:** In `dogfood-log.md` § Dispatch — health-check result; progress
notification frequency; turn count; wall-clock time; compaction event count and
timing; whether the executor stayed on track or lost state.

**Stop and tell the architect if:**
- `executor_health` fails (local LLM endpoint unreachable or misconfigured).
- `execute_phase` never invokes (MCP server not connected or tool not
  registered).
- The executor starts coding without going through the Confirmation gate
  (AGENTS.md violation — silent improvisation).

---

## Step 5 — Review phase

**Do:**
1. **On `PhaseResult.status == complete`:** run `/rexymcp:review phase-01`.
2. **On `hard_fail` / `budget_exceeded`:** run `/rexymcp:escalate phase-01`
   and exercise the decision tree (refine spec, session takeover, or resume).

**Observe (review path):**
- Does the review skill re-run the project's command set successfully?
  (`cargo build`, `cargo test`, `npm test`, etc. — whatever `rexymcp.toml`
  says.)
- Does it walk the Definition of Done from `STANDARDS.md`?
- Does it write a Review verdict in the phase doc's Update Log?
- Does it flip the phase status to `done` and commit?
- Are there any differences between the executor's command-set results and the
  reviewer's? (e.g. executor's `cargo clippy` passed but reviewer's failed?)

**Observe (escalate path):**
- Does the escalate skill present the decision tree correctly? (Refine spec,
  session takeover, resume.)
- Which lever was chosen and why?
- If refined re-dispatch was chosen, did the refinement actually fix the issue?

**Record:** In `dogfood-log.md` § Review (or § Escalate path) — command rerun
result, DoD walk result, any DoD boxes the executor missed, the verdict,
command-set differences between executor and reviewer. For escalate: the
briefing's structure, which lever was chosen, whether the refinement worked.

**Stop and tell the architect if:**
- The review skill doesn't re-run the command set at all (it's just rubber-
  stamping).
- The review skill approves a phase that clearly fails a command (e.g. tests
  are red).
- The escalate skill's decision tree is missing a lever or steers to the wrong
  one for the briefing type.

---

## Step 6 — Telemetry inspection

**Do:**
1. After at least one `complete` result, find the `phase_runs.jsonl` file in
   the configured telemetry directory (check `rexymcp.toml` for the path, or
   look in the default location).
2. Read the `PhaseRun` record(s).

**Observe:**
- Is the `PhaseRun` record well-formed? (Has all expected fields: `phase_id`,
  `status`, `turns`, `wall_clock_secs`, `tool_success_rate`,
  `parse_failure_rate`, etc.)
- Are the metrics plausible? (e.g. `tool_success_rate` should be between 0 and
  1; `turns` should match what you observed.)
- Is there a session log path recorded?

**Record:** In `dogfood-log.md` § Telemetry — the full `PhaseRun` record
contents (or its key fields if it's very long), the session log path, any
anomalies.

**Stop and tell the architect if:**
- The `PhaseRun` record is missing critical fields (no `status`, no `turns`,
  no timing data).
- Metrics are implausible (negative turns, `tool_success_rate` > 1, etc.).

---

## Step 7 — Optional: deliberate hard_fail (exercises escalate)

Only do this if the natural first phase completed cleanly and you want to
exercise the escalate path.

**Do:**
1. Draft a phase deliberately likely to hard_fail. Good candidates:
   - A phase that asks the executor to integrate with an external API whose
     docs aren't pre-injected.
   - A phase with impossible acceptance criteria (e.g. "add a feature that
     requires a new dependency" when the phase doesn't authorize one).
   - A phase with an ambiguous spec between two valid implementations.
2. Place it in the target repo's milestone as `phase-02-<slug>.md` with status
   `todo`.
3. Run `/rexymcp:dispatch phase-02`.
4. When it hard_fails, run `/rexymcp:escalate phase-02`.

**Observe:**
- Does the executor hit the Confirmation gate and flag the ambiguity / missing
   authorization? (It should — if it silently improvises, that's a bug.)
- Does the escalate skill's decision tree steer correctly for a `hard_fail`
   briefing?
- Does refined re-dispatch actually fix the issue (if that lever was chosen)?

**Record:** In `dogfood-log.md` § Escalate path — the briefing's structure,
which lever was chosen and why, whether the refinement worked, whether the
executor correctly flagged the issue vs silently improvising.

**Stop and tell the architect if:**
- The executor silently improvises around the unclear spec instead of filing a
  blocker (Confirmation gate violation).
- The escalate skill doesn't recognize the `hard_fail` briefing type.

---

## When to stop

Stop the dogfood and tell the architect (file an issue or note in
`dogfood-log.md`) if any of these happen:

- **Bootstrap writes `AGENTS.md`** — architecture violation; rexyMCP projects
  carry no root executor contract file.
- **Executor silently improvises** around an unclear spec instead of going
  through the Confirmation gate and filing a blocker.
- **Contract drift** — the embedded executor contract template doesn't match
  what the executor actually receives (visible as the executor following
  different rules than the phase doc names).
- **Plugin doesn't load** — Claude Code can't find or parse the plugin at all.
- **MCP server doesn't connect** — `execute_phase` and `executor_health` never
  respond.

For anything else, fill in the log and keep going — the retrospective will
surface patterns.
