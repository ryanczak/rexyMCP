# Phase 01: Contract-doc & plugin-manifest consistency

**Milestone:** M26 — Polish & Hardening
**Status:** todo
**Depends on:** none
**Estimated diff:** ~30 lines
**Tags:** language=markdown, kind=bugfix, size=s

## Goal

Fix two consistency defects flagged by the post-M25 codebase review: `REXYMCP.md`
(loaded into every architect session via the `CLAUDE.md` `@import`) still
describes the pre-M5 world and asserts a milestone frontier that rotted; and the
plugin ships two manifests that disagree on the plugin's name (`rexymcp` vs
`rexymcp-plugin`). After this phase there is one plugin identity everywhere and
`REXYMCP.md` points at the status sources of truth instead of duplicating them.

## Architecture references

Read before starting:

- `docs/dev/codebase-review-2026-07-07.md` §1 items 2 and 5 — the findings this
  phase fixes.
- `docs/architecture.md` § "Layer 3 — Plugin package" — context for what the
  manifests are. **Context only — do not edit `docs/architecture.md`.**

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

Every stale string this phase replaces is quoted below — the spec gives the
exact replacement text; no wording decisions are open.

`REXYMCP.md:39` (the `mcp/` bullet in § Workspace layout) — stale since M5:

```markdown
- **`mcp/`** (bin `rexymcp`) — currently a clap CLI exposing `health`; becomes the `rmcp` stdio MCP server (M5). Depends on `executor` in-process.
```

`REXYMCP.md:47` (the milestone line in § Architecture) — stale since M7 closed:

```markdown
Milestones M1→M7 are listed in `docs/architecture.md` §Status; M1–M6 are done. Active work is M7 (model scorecard & routing) — see `docs/dev/NEXT.md`.
```

`plugin/plugin.json` (the Antigravity-facing manifest) — the whole file:

```json
{
  "name": "rexymcp-plugin",
  "version": "0.1.1",
  "description": "rexyMCP architect/executor workflow plugin for Google Antigravity.",
  "author": {
    "name": "Matt Ryanczak"
  },
  "license": "MIT",
  "keywords": [
    "local-llm",
    "executor",
    "mcp",
    "architect",
    "workflow",
    "phases"
  ]
}
```

`plugin/.claude-plugin/plugin.json` (the Claude Code manifest) — the whole file:

```json
{
  "name": "rexymcp",
  "displayName": "RexyMCP",
  "version": "0.1.1",
  "description": "Architect/executor workflow over MCP — Claude Code drives a local LLM as an executor with structured phases, review gates, and telemetry."
}
```

`.claude-plugin/marketplace.json` `plugins[0].description` (line 10):

```json
      "description": "Architect/executor workflow over MCP — Claude Code drives a local LLM through structured phases, review gates, and telemetry.",
```

Install-path examples still using the old name:

- `README.md:290`: `` `plugin/` directory there (e.g. `~/.gemini/config/plugins/rexymcp-plugin`), then ``
- `plugin/README.md:47`: `` …copying or symlinking the `plugin` directory to your global customization root (e.g. `~/.gemini/config/plugins/rexymcp-plugin`). ``

The **canonical description** all three manifests converge on (defined here,
used verbatim in tasks 3–5):

> Architect/executor workflow over MCP — Claude Code or Google Antigravity
> drives a local LLM executor through structured phases, review gates, and
> telemetry.

## Spec

1. **Refresh the `mcp` crate description in `REXYMCP.md`** — in `REXYMCP.md`,
   replace the line-39 bullet quoted above with exactly:

   ```markdown
   - **`mcp/`** (bin `rexymcp`) — the `rmcp` stdio MCP server (`rexymcp serve`; tools: `execute_phase`, `executor_health`, `executor_log_search`, `executor_log_tail`, `get_turn`, `model_scorecard`, `model_profile`) plus a clap CLI (`health`, `init`, `run-phase`, `status`, `dashboard`, `doctor`, `review`, `runs`, `scorecard`, `profile`, `calibrate`, `serve`). Depends on `executor` in-process.
   ```

2. **Replace the stale milestone-status line in `REXYMCP.md`** — replace the
   line-47 sentence quoted above with exactly:

   ```markdown
   The milestone roadmap and per-milestone status live in `docs/architecture.md` §Status; the active phase is named in `docs/dev/NEXT.md`. This file deliberately does not duplicate that status.
   ```

   Why this shape: the old line rotted precisely because it duplicated state
   owned by `NEXT.md`/`architecture.md`. The replacement is a pointer, so it
   cannot rot.

3. **Unify the Antigravity manifest identity** — in `plugin/plugin.json`, set
   `"name"` to `"rexymcp"` and `"description"` to the canonical description
   quoted in § Current state. Keep `version`, `author`, `license`, and
   `keywords` exactly as they are. Do not add fields.

4. **Align the Claude Code manifest description** — in
   `plugin/.claude-plugin/plugin.json`, set `"description"` to the canonical
   description. Keep `name`, `displayName`, and `version` unchanged.

5. **Align the marketplace description** — in `.claude-plugin/marketplace.json`,
   set `plugins[0].description` to the canonical description. Keep every other
   field (including the top-level marketplace `description`) unchanged.

6. **Sync the Antigravity install-path examples** — in `README.md:290` and
   `plugin/README.md:47`, change `~/.gemini/config/plugins/rexymcp-plugin` to
   `~/.gemini/config/plugins/rexymcp`. Change nothing else on those lines.

## Acceptance criteria

- [ ] `grep -c "Active work is M7" REXYMCP.md` prints `0`.
- [ ] `grep -c "clap CLI exposing" REXYMCP.md` prints `0`.
- [ ] `grep -c "docs/dev/NEXT.md" REXYMCP.md` prints at least `1` (the pointer
      replaced the assertion; the file still routes readers to the status source).
- [ ] `grep -c '"name": "rexymcp"' plugin/plugin.json` prints `1`, and
      `grep -rc "rexymcp-plugin" README.md plugin/` prints `0` for every file.
- [ ] The three manifests carry the identical canonical description:
      `grep -c "Claude Code or Google Antigravity" plugin/plugin.json plugin/.claude-plugin/plugin.json .claude-plugin/marketplace.json` prints `1` for each file.
- [ ] `git diff --stat` (before commit) lists **only** these six files:
      `REXYMCP.md`, `README.md`, `plugin/README.md`, `plugin/plugin.json`,
      `plugin/.claude-plugin/plugin.json`, `.claude-plugin/marketplace.json`.
      In particular `docs/dev/codebase-review-2026-07-07.md` (which mentions
      `rexymcp-plugin` as a historical finding) must **NOT** be edited.
- [ ] All four gates green (no Rust source changes, so this is a regression
      check, not new coverage).

## Test plan

No Rust code changes — no new tests. The four gates (`cargo fmt --all --check`,
`cargo build`, `cargo clippy --all-targets --all-features -- -D warnings`,
`cargo test`) are re-run as separate invocations and must stay green.

## End-to-end verification

The real artifacts are the docs and manifests themselves. Run and paste the
output of each acceptance-criteria command above into the completion Update Log,
plus:

```
git diff --stat
```

quoting the six-file list.

## Authorizations

None from STANDARDS.md §5. For scope clarity: this phase authorizes edits to
exactly `REXYMCP.md`, `README.md`, `plugin/README.md`, `plugin/plugin.json`,
`plugin/.claude-plugin/plugin.json`, and `.claude-plugin/marketplace.json` —
nothing else.

## Out of scope

- The manifest/plugin `version` fields (`0.1.1` everywhere) — release-numbering
  policy is a separate conversation; do not bump.
- Any other section of `REXYMCP.md` (the Commands, Error model, Hard rules,
  Testing, and Commits sections are current — leave them byte-identical).
- `docs/architecture.md`, `docs/dev/STANDARDS.md`, `docs/dev/WORKFLOW.md`, the
  codebase-review doc, and every file under `plugin/skills/` — even where they
  mention milestones or the plugin name in prose.
- The top-level `description` of `.claude-plugin/marketplace.json` (it describes
  the marketplace, not the plugin — intentionally different).
- Adding `displayName` or any other field to `plugin/plugin.json`.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
