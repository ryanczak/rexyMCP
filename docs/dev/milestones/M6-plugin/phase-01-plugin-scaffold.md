# Phase 01: plugin scaffold + .mcp.json + slash-command stubs

**Milestone:** M6 — Plugin + architect/review skills
**Status:** todo
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
`docs/`). Inside, the **architect's reference sketch** (verify against
Pre-flight 3 — the real Claude Code convention takes precedence):

```
plugin/
├── README.md              # this layout, what's stubbed vs. to-come
├── .mcp.json              # MCP server registration + per-tool timeout
├── commands/              # slash-command definitions
│   ├── architect.md       # stub for /architect (M6 phase-04 fills it)
│   ├── dispatch.md        # stub for /dispatch (M6 phase-05 fills it)
│   └── review.md          # stub for /review (M6 phase-05 fills it)
└── (skills/ + templates/ added in later phases — not this one)
```

If Claude Code's plugin format uses a different convention (e.g. a top-level
manifest file, a different command file extension, a `.claude/` prefix), use
that instead and document the chosen layout in `plugin/README.md`.

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

### 3. Slash-command stubs

Three Markdown files under `plugin/commands/` (or wherever Claude Code expects
them — verify in pre-flight). Each is a short stub with:

- A one-line title and description (suitable for a slash-command listing).
- A "TODO — filled in by M6 phase-04/05" note pointing at the relevant
  phase doc.
- The eventual command's intended arg shape, as a placeholder:
  - `/architect` — args: `[next | next-phase]` (no args = explore + design;
    `next` / `next-phase` = draft the next phase doc).
  - `/dispatch <phase>` — args: `<phase>` (the phase doc path or a short id
    like `phase-01`).
  - `/review <phase>` — args: `<phase>` (same shape as dispatch).

The stub content is intentionally minimal — phase-04 will rewrite
`architect.md` with the full skill-invocation prompt, phase-05 will rewrite
`dispatch.md` and `review.md`. Phase-01 just ships the file scaffolding so
the registration is testable end-to-end (Claude Code can enable the plugin
and see the commands listed, even if invoking them is a no-op).

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

- [ ] Top-level `plugin/` directory exists with the four named files (or the
      verified-equivalent Claude Code layout): `README.md`, `.mcp.json`,
      `commands/architect.md`, `commands/dispatch.md`, `commands/review.md`.
- [ ] `.mcp.json` is **valid JSON** (`jq . plugin/.mcp.json` is a clean
      acceptance check; or use `python -m json.tool`).
- [ ] `.mcp.json` registers a server named `rexymcp` whose command launches
      `rexymcp serve --config <path>` (with a sensible default for `<path>`).
- [ ] `.mcp.json` configures a **raised timeout on `execute_phase`** toward
      the 10-minute ceiling (the exact field shape per Claude Code's schema —
      pinned in pre-flight). If only server-wide timeouts are supported,
      raise server-wide.
- [ ] Each of the three slash-command stub files exists, contains a clear
      "filled in by M6 phase-04/05" note pointing at the right phase, and
      describes the eventual arg shape as a placeholder.
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
      `plugin/commands/architect.md`, `plugin/commands/dispatch.md`,
      `plugin/commands/review.md`. Verified-equivalent paths (per pre-flight
      3) are fine — flag the chosen layout in Notes for review.
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
