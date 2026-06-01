# Phase 06a: dogfood preparation

**Milestone:** M6 — Plugin + architect/review skills
**Status:** todo
**Depends on:** M6 phases 01–05 (all done) — the plugin scaffold, templates, executor wiring, and three skills are all in place. M5 (done) — the `rexymcp serve` binary is what the plugin launches.
**Estimated diff:** ~700 lines (procedure doc + log template + phase doc spec + pre-flight verification output; no Rust code)
**Tags:** language=markdown, kind=feature, size=l

## Goal

Prepare everything the user needs to actually run the M6 end-to-end
dogfood. **This phase is the opencode-executable half** of M6's closer;
the dogfood run itself (`06b`) requires the user driving Claude Code +
the rexyMCP plugin + a local LLM endpoint against a real third-party
repo, which opencode in dev mode cannot do.

06a's deliverables:

1. **`docs/dev/milestones/M6-plugin/dogfood-procedure.md`** — a
   step-by-step checklist the user follows during the dogfood run. Each
   step names what to do, what to observe, and what to record.
2. **`docs/dev/milestones/M6-plugin/dogfood-log.md`** — a template the
   user fills in during the run, with named slots for every observation
   the retrospective will need.
3. **Pre-flight verification output in the Update Log** — proof that
   `rexymcp serve` builds + runs, the plugin layout is valid against
   Claude Code's plugin-load expectations (as far as we can verify
   without an actual Claude Code session), and the embedded templates +
   the executor contract are non-empty + render-safe.
4. **Target-repo selection criteria** documented in
   `dogfood-procedure.md` (criteria, not a specific repo — the user
   picks per their constraints).

After 06a lands, the user can run the dogfood on their schedule.
**06b** drafts after 06a is done; 06b's spec is "the user runs through
06a's procedure, fills in the log template, and the architect (me)
synthesizes the observations into the M6 retrospective + the
compaction-monitoring decision (per the M6 README phase-06 description)
+ any calibration folds the dogfood surfaces."

## Architecture references

- `docs/architecture.md` — Layer 3 "End-to-end flow" (the architect →
  `/rexymcp:dispatch` → executor → `/rexymcp:review` cycle that the
  dogfood exercises).
- M6 README phase-06 description — the compaction-monitoring concern
  (heuristic eviction vs summarization-based compaction; data-driven
  decision). The procedure doc must surface the data the architect
  needs to make that call.
- M6 phase-04 (architect skill) — defines what `/rexymcp:architect`
  does on first invocation (bootstrap + explore + design + draft).
- M6 phase-05 (dispatch + review + escalate skills) — defines the rest
  of the cycle. The dogfood exercises all three.
- M5 phase-02 / 05b — `execute_phase` returns `PhaseResult` (capped) +
  emits progress notifications + writes a session log + a `PhaseRun`
  telemetry record. The dogfood produces the **first non-self
  `PhaseRun`** records.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` and the M6 README (esp. the phase-06
   description with the compaction-monitoring concern).
2. Read this entire phase doc.
3. **Verify Claude Code's plugin-install convention** before writing
   the procedure. Where does Claude Code expect the plugin directory
   to live (a known global location? a per-project `.claude/plugins/`?
   a user-data-dir?)? How does the user enable a plugin after
   installing? Verify against Claude Code docs / `claude-code-guide`
   Agent / working examples. **Trust docs over the architect's
   sketch** (per WORKFLOW.md "Verify external APIs against live
   docs"); flag any divergence in "Notes for review".
4. Build the binary and confirm `rexymcp serve --help` works
   (smoke check — see § 3 below).
5. Confirm the plugin directory shape from phase-01 matches the
   verified Claude Code expectations (re-check `plugin/.mcp.json`,
   `plugin/.claude-plugin/plugin.json`, `plugin/skills/<name>/SKILL.md`
   for the four skills).

## Spec

### 1. Target-repo selection criteria

Document criteria (not a specific recommendation) for the user picking
a dogfood target. Place in `dogfood-procedure.md` § "Picking a target
repo". Required criteria:

- **Small** — single-digit thousands of LOC, ideally under 5K.
- **Open-source** — legally clone-able / fork-able.
- **Active or recently-active** — last commit in the past year.
- **Language ≠ Rust** — the dogfood's main job is exercising
  `{...}_COMMAND` placeholder substitution beyond `cargo`. Python,
  Go, JavaScript, TypeScript all good choices.
- **Has a working test suite** — the review skill re-runs the project's
  command set; without tests there's nothing to verify.
- **Has detectable build/test commands** — the bootstrap routine
  inspects `Cargo.toml` / `package.json` / `pyproject.toml` / `go.mod`
  / etc.; pick a repo whose conventions match those detection rules.

Recommended attributes (not required):
- A small open issue or feature request that could be a real first
  phase (gives the architect-skill explore-then-design a concrete
  target).
- An MIT/Apache/BSD-licensed project (forking is unambiguously
  permitted).
- Fewer than ~20 transitive dependencies (faster `{BUILD_COMMAND}`
  re-runs during review).

Do **not** recommend a specific repo — repos change, get archived,
acquire maintainers/policies. Criteria are stable; recommendations rot.

### 2. The dogfood procedure — `docs/dev/milestones/M6-plugin/dogfood-procedure.md`

A step-by-step checklist the user runs through, organized into phases
that mirror the architect → dispatch → review → (escalate) cycle.
Each step has three parts: **Do**, **Observe**, **Record**.

Structure:

1. **Pre-dogfood setup** (one-time)
   - Install rexyMCP binary (`cargo install --path mcp` from the
     rexyMCP repo).
   - Verify `rexymcp serve --help` works.
   - Install the Claude Code plugin (per the verified install
     convention from Pre-flight 3).
   - Configure your local LLM endpoint (which OpenAI-compatible
     server, which model, what context size).
   - Clone the target repo to a fresh directory.

2. **Bootstrap phase**
   - Open a fresh Claude Code session in the target repo's directory.
   - Run `/rexymcp:architect` (no args).
   - **Observe:** does the bootstrap routine detect the command set
     correctly? Does it prompt for confirmation? Does it write
     `rexymcp.toml` + `docs/dev/STANDARDS.md` + `docs/dev/WORKFLOW.md`
     + `CLAUDE.md` + register `rexymcp` in `.mcp.json`?
   - **Record:** in `dogfood-log.md` § "Bootstrap": what was detected
     vs what required user override; any files that didn't get
     written; any unexpected prompts or errors.

3. **Design phase** (architect skill continues)
   - Engage Claude on the project's design — what's it for, what
     should the first milestone unlock.
   - **Observe:** does the architect write `docs/architecture.md` +
     a milestone README + draft `phase-01-<slug>.md`?
   - **Record:** § "Design": quality of the architecture doc, quality
     of the milestone decomposition, quality of the phase-01 draft
     (especially: how good is the pre-injection?).

4. **Dispatch phase**
   - Run `/rexymcp:dispatch phase-01`.
   - **Observe:** does `executor_health` confirm reachability? Does
     `execute_phase` invoke? Do MCP progress notifications appear?
     What's the executor's behavior — does it follow the spec?
   - **Record:** § "Dispatch": health-check result; how often progress
     notifications fired; how many turns the executor ran; how long
     wall-clock; **how many compaction events** (`CompactionReport`
     totals from the returned `PhaseResult` or
     `executor_log_search { event_type: "compaction" }` if the loop
     logs them) and **how early in the phase they fired**; whether
     the executor lost track of state visibly (re-asking for context
     it already had, repeating tool calls).

5. **Review phase**
   - On `PhaseResult.status == complete`: run `/rexymcp:review
     phase-01`.
   - **Observe:** does the review skill re-run the command set
     successfully? Does it walk the DoD? Does it write a Review
     verdict + flip status to `done` + commit?
   - On `hard_fail` / `budget_exceeded`: run `/rexymcp:escalate
     phase-01` and exercise the decision tree.
   - **Record:** § "Review" or § "Escalate": result, command-set
     differences (executor's vs reviewer's), any DoD boxes the
     executor missed, the verdict.

6. **Telemetry inspection**
   - After at least one `complete` result, query the
     `phase_runs.jsonl` from the configured telemetry dir.
   - **Observe:** is the `PhaseRun` record well-formed? Are the
     `tool_success_rate` / `parse_failure_rate` / etc. metrics
     plausible?
   - **Record:** § "Telemetry": the full `PhaseRun` record contents
     (or its key fields), the session log path, any anomalies.

7. **Optional: deliberate hard_fail** (exercises escalate)
   - Draft a phase deliberately likely to hard_fail (e.g. asks the
     executor to integrate with an external API whose docs aren't
     pre-injected).
   - Run dispatch → escalate cycle.
   - **Observe:** does the escalate decision tree steer correctly?
     Does refined re-dispatch actually fix the issue?
   - **Record:** § "Escalate path": the briefing's structure, which
     lever was chosen and why, whether the refinement worked.

Each step has explicit "Stop and tell the architect (file an issue or
note in dogfood-log.md) if X" guidance — e.g. "stop if bootstrap
writes `AGENTS.md` (architecture violation)" or "stop if the executor
silently improvises around an unclear spec (Confirmation gate
violation)."

### 3. The dogfood log template — `docs/dev/milestones/M6-plugin/dogfood-log.md`

A skeleton file with named sections matching the procedure's "Record"
prompts. The user fills it in during the run. Sections:

- `## Setup` — versions, target repo, model, context size
- `## Bootstrap` — detected commands, files written, deviations
- `## Design` — quality assessment of arch doc / milestone / phase-01
- `## Dispatch` — health, progress, compaction data, turns, wall-clock
- `## Review` — command rerun, DoD walk, verdict
- `## Escalate path` (optional) — briefing, lever, refinement result
- `## Telemetry` — PhaseRun fields, log path
- `## Surprises and breakages` — anything that didn't go as expected
- `## Compaction observations` (called out separately because it's a
  primary objective): how often, how early, did it bite, would
  summarization help

Empty fields with one-line prompts for what to fill in. The user
follows the procedure top-to-bottom and the log is the artifact.

### 4. Pre-flight verification

Before the dogfood, opencode runs these checks and pastes the output
in the Update Log:

1. `cargo build` — succeeds (no new warnings).
2. `cargo install --path mcp --locked --offline --debug` — succeeds
   (verifies the binary actually installs from this tree; `--offline`
   confirms no missing deps; `--debug` keeps build time short).
   *If `--locked` errors against the workspace `Cargo.lock`,* fall back
   to `cargo install --path mcp --debug`; document which form worked
   in Notes for review.
3. `<install-dir>/rexymcp --version` — prints the version.
4. `<install-dir>/rexymcp serve --help` — prints the subcommand help
   (smoke check that `serve` is wired and parses cleanly; do not
   actually start the server).
5. `jq . plugin/.mcp.json` — valid JSON.
6. `jq . plugin/.claude-plugin/plugin.json` — valid JSON; `name` field
   is `"rexymcp"`.
7. `wc -l plugin/skills/*/SKILL.md executor/templates/executor_contract.md
   plugin/templates/STANDARDS.md plugin/templates/WORKFLOW.md` — each
   non-empty.
8. Re-run the validation greps from phases 02 / 04 / 05 to confirm
   nothing has regressed: only the four authorized
   `{FORMAT|BUILD|LINT|TEST}_COMMAND` placeholders in templates; only
   legitimate rexyMCP refs in skills; zero opencode/non-MCP-Rexy/cargo
   leaks in `plugin/templates/`.

Paste each command's output in the Update Log under "Pre-flight
verification."

### 5. What 06a does NOT do

- Does **not** run the dogfood. That's 06b (user-driven).
- Does **not** write the M6 retrospective. That's also 06b.
- Does **not** make the compaction-monitoring decision (heuristic vs
  summarization). That decision depends on dogfood data.
- Does **not** decide whether to draft a follow-up milestone for
  summarization-based compaction. Also 06b.
- Does **not** install the plugin into a Claude Code config (that's
  the user's machine, not opencode's; the procedure tells the user
  how).
- Does **not** modify the M6 README's phase-06 description (the
  compaction-monitoring concern stays as-pinned; 06b may amend it
  based on data).

## Adaptations / decisions

1. **06a / 06b split** — same pattern as M4 phase-07a–e, M5 phase-05a/b.
   The opencode-executable prep and the user-driven dogfood are
   different roles and probably different days; clean to separate.
2. **Criteria, not a recommended repo** — repos rot, criteria don't.
3. **Procedure structure: Do / Observe / Record per step** — keeps the
   user's experience structured and the log artifact disciplined.
4. **Compaction observations are a primary objective** — called out
   separately in the log template because the M6 README pinned this
   as a decision-driving concern.
5. **The optional escalate-path exercise** is real but optional — if
   the natural first phase completes cleanly, the user can stop
   without forcing a hard_fail. The log notes whether it was
   exercised.
6. **No specific install location for the plugin in the procedure** —
   defer to whatever Claude Code's plugin-install convention turns
   out to be (Pre-flight 3 verifies; the procedure references that
   convention by name, not by hardcoded path).
7. **The procedure tells the user when to stop and escalate to the
   architect** — explicit "stop if X" lines at the architecturally-
   critical points (bootstrap writing `AGENTS.md`, executor silent
   improvisation, contract drift).

## Acceptance criteria

- [ ] `docs/dev/milestones/M6-plugin/dogfood-procedure.md` exists.
      Covers all 7 procedure phases from § 2 (pre-dogfood setup,
      bootstrap, design, dispatch, review, telemetry, optional
      escalate-path).
- [ ] **Each procedure step has explicit Do / Observe / Record
      parts.** Not "run /rexymcp:architect" — instead "**Do:** run
      /rexymcp:architect; **Observe:** Claude prompts you to confirm
      detected commands; **Record:** in § Bootstrap, what was detected
      vs what you overrode."
- [ ] **Target-repo selection criteria** documented in
      dogfood-procedure.md § "Picking a target repo." Both required
      and recommended attributes named. No specific repo named.
- [ ] **Architecture-critical stop conditions** explicit in the
      procedure: stop and escalate if bootstrap writes `AGENTS.md`,
      stop if executor silently improvises around an unclear spec,
      stop if the contract drifts from the embedded template.
- [ ] `docs/dev/milestones/M6-plugin/dogfood-log.md` exists as a
      template with named sections matching the procedure's "Record"
      prompts. Includes the dedicated `## Compaction observations`
      section per § 3.
- [ ] **Pre-flight verification output** pasted in the Update Log:
      `cargo build` + `cargo install` + `rexymcp --version` +
      `rexymcp serve --help` + `jq .` for both JSON files + `wc -l`
      for the four content artifacts + re-running the phases
      02/04/05 validation greps. **All must pass.**
- [ ] **Validation greps still pass** (no regression in phases 02 /
      04 / 05 content). Pin output in the Update Log.
- [ ] **No Rust code changes.** `cargo fmt --check`, `cargo build`,
      `cargo clippy --all-targets --all-features -- -D warnings`,
      `cargo test` all pass unchanged.
- [ ] **No new dependencies.**
- [ ] **Calibration carry-forward (mandatory):** declare every scope
      deviation in "Notes for review", even defensible ones.
      Pre-flight 3 carry-over: if Claude Code's plugin-install
      convention differs from the spec's sketch, follow the
      convention and flag.

## Test plan

This phase ships no Rust code; the four `cargo` gates are unchanged.
Content verification is via:

1. The pre-flight verification commands from § 4 (their output is the
   test).
2. **Read-pass gut-check** (per M6 bug-02-1's lesson): pretend to be a
   user (not a Claude session — a *user*) opening the
   dogfood-procedure.md for the first time, with no prior context
   about rexyMCP's internals. Confirm:
   - You know what to install, in what order.
   - You know what `/rexymcp:architect` will do before running it.
   - You know what to record at each step.
   - You know when to stop and ask the architect.
   - The procedure doesn't assume rexyMCP-internal knowledge (e.g.
     doesn't say "as you know, the executor uses M3's forgiving
     parser to handle…").

No `#[cfg(test)]` blocks (no Rust). The actual dogfood is exercised
in 06b.

## End-to-end verification

> Not applicable in the traditional sense — 06a ships preparation
> artifacts for the dogfood, not a runtime-loadable artifact. The
> pre-flight verification commands in § 4 *are* the closest thing
> (they exercise the live `rexymcp` binary against `--help` and
> confirm the plugin JSON files parse). The full end-to-end dogfood
> run lands in 06b.

## Authorizations

- [x] **May create** `docs/dev/milestones/M6-plugin/dogfood-procedure.md`
      and `docs/dev/milestones/M6-plugin/dogfood-log.md`.
- [x] **May run** `cargo install --path mcp --debug` to verify the
      binary installs (this is part of pre-flight verification, not a
      lasting change to the workspace).
- [ ] **No Rust code changes** in `executor/src/` or `mcp/src/`.
- [ ] **No new dependencies.** No `Cargo.toml` edits.
- [ ] May **NOT** run the dogfood itself (that's 06b).
- [ ] May **NOT** write the M6 retrospective (that's 06b).
- [ ] May **NOT** make the compaction-monitoring decision (that's 06b
      based on dogfood data).
- [ ] May **NOT** modify any other phase doc, `docs/architecture.md`,
      `STANDARDS.md`, `WORKFLOW.md`, `AGENTS.md`, any `plugin/` file
      (the plugin's content is settled across phases 01–05), or the
      embedded templates.

## Out of scope

- **The dogfood run itself** — 06b.
- **The M6 retrospective + calibration folds + compaction decision**
  — 06b.
- **A follow-up milestone for summarization-based compaction** —
  decided in 06b based on data; not pre-committed.
- **A user-facing plugin QUICKSTART.md** — the dogfood-procedure.md
  is for our internal test; a polished user quickstart can come
  later (probably after M6 close, once we know the workflow holds).
- **Recommending a specific dogfood target repo** — criteria only
  (Adaptation 2).
- **CI integration** — the dogfood is a one-shot manual run; not
  worth automating yet.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
