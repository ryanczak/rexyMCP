# Phase 01: plugin scaffold + .mcp.json + slash-command stubs

**Milestone:** M6 — Plugin + architect/review skills
**Status:** done
**Depends on:** M5 (done) — `rexymcp serve --config <path>` is the binary the plugin wraps.
**Estimated diff:** ~200 lines (mostly JSON + Markdown stubs + a layout README)
**Tags:** language=markdown, kind=feature, size=s

## Goal

Stand up the **Claude Code plugin's filesystem layout** under a new top-level
`plugin/` directory and ship three pieces:

1. **`.mcp.json`** registering the `rexymcp serve --config <path>` MCP server,
   with a **raised per-tool timeout** on `execute_phase` (toward the 10-minute
   ceiling per architecture).
2. **Three slash-command stubs**: `/architect`, `/dispatch`, `/review`. Each is
   a placeholder Markdown file with a clear "filled in by M6 phase-04/05" note;
   no behavior yet.
3. **A `plugin/README.md`** explaining the layout and what's still to come
   (skills land in phase-04/05, templates in phase-02, dogfood in phase-06).

This is the leaf the rest of M6 builds on. **No skills, no templates, no
bootstrap, no executor edits.** All three of those are later M6 phases.

## Architecture references

- `docs/architecture.md` — Layer 3 "Plugin package": `.mcp.json`, skills,
  commands, embedded templates. Phase-01 ships the manifest + command stubs
  only.
- M5 phase-02 § 6 "Per-tool timeout — architecture note, not server work": the
  per-tool timeout is enforced **client-side** via `.mcp.json` per-server
  config. Phase-01 is where that lives.
- M5 README § "What carries forward to M6": per-tool MCP timeout is M6's
  `.mcp.json` work.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` and the M6 README.
2. Read this entire phase doc.
3. **Verify the current Claude Code plugin filesystem layout and `.mcp.json`
   schema.** The architect can't reliably enumerate the exact convention.
   Sources to consult, in priority order:
   1. The Claude Code documentation site (search "Claude Code plugins",
      "plugin layout", ".mcp.json schema", "slash commands").
   2. The `claude-code-guide` Agent if available in the executor's tool set
      (it's specifically scoped to questions about Claude Code).
   3. Working examples from other Claude Code plugins (look up published
      examples; do not invent).
   The architect-supplied sketch in § 1 below may be wrong on layout
   specifics (file names, directory names, `.mcp.json` field names).
   **Trust the docs over the sketch**; flag any divergence in "Notes for
   review". Pin behavior the spec names; let the executor adapt the structure
   to the real conventions.
4. Confirm `rexymcp serve --config <path>` is the binary the plugin launches
   — phase-02 of M5 added the subcommand; phase-01 here simply references it.

## Spec

### 1. Top-level `plugin/` directory

Create a new top-level directory `plugin/` (alongside `executor/`, `mcp/`,
`docs/`). Inside, the **architect's reference sketch updated with Pre-flight
3 findings** (2026-05-31, opencode):

```
plugin/
├── README.md                      # this layout, what's stubbed vs. to-come
├── .mcp.json                      # MCP server registration (no timeout field — see § 2)
├── .claude-plugin/
│   └── plugin.json                # required plugin manifest: { "name": "rexymcp", … }
├── skills/                        # modern Claude Code layout (verified)
│   ├── architect/SKILL.md         # stub for the architect skill (phase-04 fills it)
│   ├── dispatch/SKILL.md          # stub for the dispatch skill (phase-05 fills it)
│   └── review/SKILL.md            # stub for the review skill (phase-05 fills it)
└── (templates/ added in phase-02 — not this one)
```

**`.claude-plugin/plugin.json` is required** per Claude Code's plugin
contract (Pre-flight 3 finding). Minimum content: `{ "name": "rexymcp" }`.
Other fields (version, description, author) optional; add whatever Claude
Code's schema names as recommended. **Authorized** as part of this phase
(see Authorizations).

If Claude Code's plugin convention is still evolving and any of these paths
differ from what opencode finds in the live docs, use the live convention
and document the chosen layout in `plugin/README.md` (this is exactly the
trust-docs-over-sketch path Pre-flight 3 authorizes).

### 2. `.mcp.json` content

Register the rexyMCP MCP server. The architect's reference sketch:

```json
{
  "mcpServers": {
    "rexymcp": {
      "command": "rexymcp",
      "args": ["serve", "--config", "${REXYMCP_CONFIG:-./rexymcp.toml}"],
      "timeouts": {
        "execute_phase": 600000
      }
    }
  }
}
```

- **Server name `rexymcp`.** Stable identifier; M6 references this name
  elsewhere (slash commands, skill prompts).
- **Command `rexymcp`.** Assumes the binary is on `$PATH` after the plugin
  installs / the user runs `cargo install --path mcp`. **Do not** hardcode
  an absolute path.
- **Args** invoke `serve --config <path>`. The `<path>` should default to
  `./rexymcp.toml` in the target repo (bootstrap writes it there) but should
  also be overridable via an environment variable (the syntax depends on
  Claude Code's `.mcp.json` expansion rules — verify in pre-flight; if the
  client doesn't support env-var expansion, hardcode `./rexymcp.toml` and
  flag in Notes).
- **Per-tool timeout on `execute_phase`: 600000 ms (10 minutes).** This is
  the architecture's "toward the 10-minute ceiling." The exact field name
  and value-shape may differ (Claude Code may want `timeout` as a top-level
  integer applied to all tools, or `timeouts.<tool_name>` as a map, or
  something else entirely). Verify in pre-flight; adapt the JSON to whatever
  Claude Code actually accepts. **Pin the behavior** (long timeout on
  `execute_phase`, default on the others) not the exact JSON shape.

If Claude Code doesn't support per-tool timeouts (only per-server), set the
server-wide timeout to 10 minutes — `execute_phase` is the longest call; the
other five (`executor_health`, the three log-query tools, `model_scorecard`,
roots check) all return quickly and a server-wide cap doesn't hurt them.

**Pre-flight 3 finding (2026-05-31, opencode):** Claude Code's `.mcp.json`
schema documents `command`, `args`, `env`, `cwd`, `transport` — and
**no timeout field at all** (not per-tool, not per-server). **Resolution:
omit timeout config from `.mcp.json`** and document the limitation
prominently in `plugin/README.md`:

> Claude Code's `.mcp.json` schema does not currently expose a tool-timeout
> setting. The architecture's "raise toward 10 minutes" target depends on
> the client honoring long-running tool calls without enforcing a default
> interrupt. **Open question for the M6 phase-06 dogfood:** does Claude
> Code's MCP client interrupt long `execute_phase` calls? If yes, file a
> follow-up (either upstream to Claude Code or to add a heartbeat-driven
> keepalive path here). The M5 phase-05b progress notifications are the
> primary liveness signal the client sees during long calls.

This is exactly the pre-flight-3 win pattern — the architect's sketch named
a config field that the real schema doesn't have. Document the gap, don't
invent a field that won't be honored.

### 3. Skill stubs (and slash-command stubs if Claude Code keeps them separate)

**Pre-flight 3 finding (2026-05-31, opencode):** Claude Code's modern plugin
layout prefers `skills/<name>/SKILL.md` (directories with `SKILL.md` files)
over the legacy flat `commands/` layout. **Resolution: use `skills/`** —
phases 04/05 will fill these with real skill bodies anyway, so going
straight to `skills/` avoids a layout migration mid-milestone.

Three skill directories under `plugin/skills/` (or wherever Claude Code
expects them):

- `plugin/skills/architect/SKILL.md` (stub)
- `plugin/skills/dispatch/SKILL.md` (stub)
- `plugin/skills/review/SKILL.md` (stub)

If Claude Code *also* expects slash-command-trigger files separate from
skills (i.e. `/architect` is a command-trigger that invokes the `architect`
skill, and both files exist), create those too — verify the convention. If
the skill-and-slash-command are unified (one file does both), `skills/`
alone is enough. **Pin the behavior** (three stubs named `architect` /
`dispatch` / `review`, each a placeholder for phase-04/05 to fill); let the
exact file layout follow Claude Code's actual convention.

Each stub is a short Markdown file with:

- A one-line title and description (suitable for a slash-command listing).
- A "TODO — filled in by M6 phase-04/05" note pointing at the relevant
  phase doc.
- The eventual command's intended arg shape, as a placeholder:
  - `/architect` — args: `[next | next-phase]` (no args = explore + design;
    `next` / `next-phase` = draft the next phase doc).
  - `/dispatch <phase>` — args: `<phase>` (the phase doc path or a short id
    like `phase-01`).
  - `/review <phase>` — args: `<phase>` (same shape as dispatch).

The stub content is intentionally minimal — phase-04 will rewrite the
`architect` skill with the full bootstrap + pre-injection prompt; phase-05
will rewrite `dispatch` and `review`. Phase-01 just ships the file
scaffolding so the plugin registers cleanly (Claude Code can enable it and
see the three skills listed, even if invoking them is a no-op).

### 4. `plugin/README.md`

A short orientation doc:

- What this directory is (the Claude Code plugin package for rexyMCP).
- Layout map (matches § 1 — keep in sync if the layout adapts per pre-flight).
- Install/enable instructions (Claude Code's plugin install flow; verify in
  pre-flight — likely `claude code plugins install <path>` or similar).
- Status: what's present in M6 phase-01 (manifest + stubs), what's coming
  (skills in phase-04/05, templates in phase-02, contract embedding in
  phase-03, dogfood in phase-06).
- Pointer to `docs/dev/milestones/M6-plugin/README.md` for the full plan.

### 5. CI / gates

The plugin directory does not participate in `cargo` gates. No new
`Cargo.toml` entries. The existing four gates (`cargo fmt --check`, `cargo
build`, `cargo clippy`, `cargo test`) should still pass unchanged — phase-01
adds **no Rust code**.

`.gitignore` does not need changes — plugin files are committed.

## Adaptations / decisions

1. **`plugin/` is a top-level directory.** Alternatives considered: under
   `docs/` (no — these are runtime artifacts, not docs) or under `mcp/` (no —
   the plugin wraps the mcp server but is conceptually one level up). Top
   level matches the layered structure (executor → mcp → plugin).
2. **JSON, not TOML, for `.mcp.json`.** It's Claude Code's convention; we
   follow it.
3. **Stub commands have no behavior.** Phase-01 ships scaffolding only.
   Behavior comes from phases 04/05 when the skills exist.
4. **Per-tool timeout on `execute_phase` only.** If Claude Code only supports
   per-server, raise server-wide. Document in Notes.
5. **`rexymcp` binary assumed to be on `$PATH`.** Installation flow is the
   user's responsibility (or `cargo install --path mcp` — `plugin/README.md`
   mentions it). Not hardcoding an absolute path keeps the plugin portable.
6. **Bootstrap (writing `.mcp.json` into a target repo) is NOT this phase.**
   Phase-04 (architect skill) owns the bootstrap routine. Phase-01 only
   ships the *plugin-internal* `.mcp.json` template that gets copied into
   target repos by bootstrap (or registered globally — depends on Claude
   Code's model).

## Acceptance criteria

- [ ] Top-level `plugin/` directory exists with the verified Claude Code
      layout: `README.md`, `.mcp.json`, `.claude-plugin/plugin.json`, and
      three skill stubs under `skills/<name>/SKILL.md` for `architect`,
      `dispatch`, `review`.
- [ ] `plugin/.claude-plugin/plugin.json` is valid JSON, sets `"name":
      "rexymcp"`, and includes whatever other fields Claude Code's plugin
      schema names as required or recommended.
- [ ] `.mcp.json` is **valid JSON** (`jq . plugin/.mcp.json` is a clean
      acceptance check; or use `python -m json.tool`).
- [ ] `.mcp.json` registers a server named `rexymcp` whose command launches
      `rexymcp serve --config <path>` (with a sensible default for `<path>`).
- [ ] **No timeout field in `.mcp.json`** (Pre-flight 3 finding: Claude
      Code's schema doesn't expose one). The limitation is documented in
      `plugin/README.md` with the architecture-target note + the dogfood
      open question.
- [ ] Each of the three skill stub files (`skills/architect/SKILL.md`,
      `skills/dispatch/SKILL.md`, `skills/review/SKILL.md`) exists, contains
      a clear "filled in by M6 phase-04/05" note pointing at the right
      phase, and describes the eventual arg shape as a placeholder.
- [ ] `plugin/README.md` documents the layout, install path, and status
      (what's present in phase-01, what each later phase will add).
- [ ] **No Rust code changes.** `cargo fmt --check`, `cargo build`,
      `cargo clippy --all-targets --all-features -- -D warnings`, and
      `cargo test` all pass unchanged (no new test count, no warnings).
- [ ] **No new dependencies.** No `Cargo.toml` changes.
- [ ] No Rexy references in the new files (`grep -rn 'Rexy\|rexy/src' plugin/`
      → 0 — the plugin is rexyMCP's public-facing surface, not the
      internal lift apparatus).
- [ ] **Calibration carry-forward (mandatory):** declare every scope
      deviation in "Notes for review", even defensible ones. M6 is the first
      milestone since the M5 close folded "Wrap-vs-derive" and "Anticipate
      cross-boundary trait bounds" into WORKFLOW.md; the *declare-deviations*
      muscle still applies even though M6 is mostly Markdown.

## Test plan

This phase ships no Rust code, so the four `cargo` gates are the only
automated checks. Hermetic verification of the plugin files:

- **`jq . plugin/.mcp.json`** returns valid JSON (no parse error).
- **`grep`** confirms `rexymcp` server name, `serve --config` args, and the
  raised timeout in the `.mcp.json`.
- **`ls plugin/commands/`** shows exactly three files (`architect.md`,
  `dispatch.md`, `review.md` — or the verified-equivalent extensions).
- **`grep -rn 'TODO\|filled in by'`** in `plugin/commands/` shows each stub
  has its placeholder note.
- **`grep -rn 'Rexy\|rexy/src' plugin/`** returns nothing.

No `#[cfg(test)]` blocks (no Rust). No live plugin install or end-to-end
verification at this phase — the plugin doesn't *do* anything yet (no skills
behind the stubs). End-to-end exercise lands in phase-06 (dogfood).

## End-to-end verification

> Not applicable. The plugin has no behavior in phase-01 — slash commands are
> stubs, no skills exist. Verifying the plugin loads correctly in Claude Code
> is possible (install the plugin, list slash commands, see the three names)
> but not required by this phase; if attempted, document the result in the
> Update Log as a manual smoke test.

## Authorizations

- [x] **May create** `plugin/`, `plugin/README.md`, `plugin/.mcp.json`,
      **`plugin/.claude-plugin/plugin.json`** (required per Claude Code
      contract — Pre-flight 3 finding), and three skill stubs under
      `plugin/skills/<name>/SKILL.md` for `architect`, `dispatch`, `review`.
      Verified-equivalent paths (per Pre-flight 3) are fine — flag the
      chosen layout in Notes for review.
- [ ] **No Rust code changes.** No `executor/` or `mcp/` edits.
- [ ] **No new dependencies.** No `Cargo.toml` edits.
- [ ] May **NOT** add embedded templates (phase-02), wire the contract
      (phase-03), write any skill body (phase-04/05), or perform the dogfood
      (phase-06).
- [ ] May **NOT** modify `docs/architecture.md`, `STANDARDS.md`,
      `WORKFLOW.md`, `AGENTS.md`, or any other phase doc.
- [ ] **Calibration carry-forward (mandatory):** declare every scope
      deviation in "Notes for review".

## Out of scope

- **Skills bodies** (architect, review-phase, escalate) — phases 04/05.
- **Embedded templates** (`STANDARDS`, `WORKFLOW`, `executor_contract`) —
  phase-02.
- **Bootstrap routine** — phase-04 (part of the architect skill).
- **Dogfood** — phase-06.
- **Plugin install / publish flow** — out of M6 entirely; user-facing
  packaging.
- **Per-skill UI tweaks** (icons, autocomplete metadata) — defer; ship
  minimal stubs.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-05-31 15:00 (started)

**Executor:** opencode (Qwen/Qwen3.6-27B-FP8)

Pre-flight 3 verified Claude Code plugin layout against docs.anthropic.com. Three findings applied: (1) `.claude-plugin/plugin.json` manifest is required, (2) `skills/<name>/SKILL.md` is the modern layout (not legacy `commands/`), (3) `.mcp.json` has no timeout field — omitted per architect resolution. Creating 6 files across `plugin/`.

### Update — 2026-05-31 15:05 (complete)

**Summary:** Created the Claude Code plugin scaffold under `plugin/` with 6 new files: manifest (`.claude-plugin/plugin.json`), MCP server registration (`.mcp.json`), three skill stubs (`skills/{architect,dispatch,review}/SKILL.md`), and `README.md`. Pre-flight 3 verified the layout against Claude Code docs; all three findings (manifest required, `skills/` over `commands/`, no timeout field) were applied. No Rust code changes, no new dependencies.

**Acceptance criteria:** all ticked.

**Commands:**

```
cargo fmt --all --check
(no output — clean)

cargo build 2>&1 | tail -20
Compiling rexymcp v0.1.0 (/home/matt/src/rexyMCP/mcp)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.47s

cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail -20
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.06s

cargo test 2>&1 | tail -30
test result: ok. 512 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out
```

**End-to-end verification:**

Not applicable — phase ships no runtime-loadable artifact. The plugin has no behavior in phase-01; skill stubs are placeholders. Verification against real Claude Code install is deferred to phase-06 dogfood. File-level verification:
- `jq . plugin/.mcp.json` — valid JSON, server `rexymcp` with `serve --config ./rexymcp.toml`
- `jq . plugin/.claude-plugin/plugin.json` — valid JSON, `name: rexymcp`
- `ls plugin/skills/` — `architect`, `dispatch`, `review`
- `grep -rn 'filled in by' plugin/skills/` — 3 hits, one per stub
- `grep -rn 'Rexy\|rexy/src' plugin/` — 1 hit on `displayName: "RexyMCP"` in `plugin.json`, which is the product name, not a Rexy-repo reference

**Files changed:**
- `plugin/.claude-plugin/plugin.json` — plugin manifest (name: rexymcp)
- `plugin/.mcp.json` — MCP server registration (no timeout field per pre-flight finding)
- `plugin/skills/architect/SKILL.md` — stub for architect skill (phase-04)
- `plugin/skills/dispatch/SKILL.md` — stub for dispatch skill (phase-05)
- `plugin/skills/review/SKILL.md` — stub for review skill (phase-05)
- `plugin/README.md` — layout, install, status, timeout limitation note

**New tests:** none (no Rust code).

**Commits:** (pending)

**Notes for review:**
- Scope deviation: used `skills/<name>/SKILL.md` instead of `commands/<name>.md` per Claude Code's modern convention. This avoids a mid-milestone layout migration when phases 04/05 fill the skills.
- Scope deviation: added `.claude-plugin/plugin.json` manifest — required by Claude Code's plugin contract, not in the original spec.
- Omitted timeout field from `.mcp.json` — Claude Code's schema has no timeout field. Documented in README with dogfood open question.
- `displayName: "RexyMCP"` in `plugin.json` causes a grep hit on the acceptance criterion pattern `Rexy\|rexy/src`. This is the product name, not a Rexy-repo reference.

verification: fmt OK · clippy OK · tests 512 passed · build OK

### Update — 2026-05-31 (approved — architect)

**Verdict:** approved_first_try. Clean scaffold landing — both JSON files
parse, the `.mcp.json` correctly omits the timeout field per the Pre-flight
3 resolution, `plugin.json` carries `name: "rexymcp"` plus sensible
optional metadata, all three skill stubs have phase-04/05 placeholders,
and `plugin/README.md` documents the timeout limitation with the dogfood
open question forwarded to phase-06.

**Gates:** fmt ✓ · clippy ✓ · tests **629** unchanged (117 mcp + 512
executor — zero Rust changes per spec).

**Three deviations all declared in Notes for review** (the M5-era calibration
holding into M6):
1. **Added `.claude-plugin/plugin.json`** — required by Claude Code's plugin
   contract; not in the original spec. Authorized inline in the architect's
   pre-flight-3 resolution; opencode correctly declared it.
2. **Omitted timeout field from `.mcp.json`** — Pre-flight 3 finding;
   documented in README. Correctly declared.
3. **`displayName: "RexyMCP"` matches the acceptance criterion's "no Rexy
   refs" grep pattern.** Self-flagged. This is a *spec calibration issue,
   not an executor miss* — the architect's `grep 'Rexy\|rexy/src'` pattern
   was too broad; the intent was to catch references to the *donor* Rexy
   crate (`~/src/rexy`), not the product's own brand name. "RexyMCP" is
   correct as the display name; excluding it would break Claude Code's UI
   labeling. **Calibration note for the architect (me):** use a more
   precise pattern in future specs — e.g. `grep -P '\\brexy\\b(?!MCP)'`
   or `grep 'rexy/src\\b'` — to catch donor references without false
   positives on the product name. One occurrence; not folding yet, but
   flagging.

**Self-review accuracy holds.** All three deviations declared upfront,
including the *spec calibration issue* the executor noticed and surfaced
(item 3). That's the maturity bar phase-04 / phase-05a set: declare even
the *grey* cases the executor isn't sure about. Going forward, M6 / M7
specs should pre-validate grep patterns against the layout they describe.

**Bounces:** 0.
**Scope deviations:** 3 declared (all defensible, all retroactively
accepted).

**Executor:** opencode (Qwen/Qwen3.6-27B-FP8). Approved first try. M6
phase-02 (embedded templates) is the natural next draft.
