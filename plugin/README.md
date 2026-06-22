# rexyMCP Claude Code Plugin

This directory is the Claude Code plugin package for rexyMCP. It bundles the MCP
server (`rexymcp serve`) with the architect/executor workflow: four skills (slash
commands) and the embedded `STANDARDS` / `WORKFLOW` templates that bootstrap a
target project.

See the [repository README](../README.md) for the full product overview,
configuration reference, and CLI documentation.

## Layout

```
plugin/
├── .claude-plugin/
│   └── plugin.json            # plugin manifest (name: rexymcp, version 0.1.1)
├── .mcp.json                  # MCP server registration → `rexymcp serve --config ./rexymcp.toml`
├── README.md                  # this file
├── skills/
│   ├── architect/SKILL.md     # /rexymcp:architect — bootstrap, design, author phase docs
│   ├── dispatch/SKILL.md      # /rexymcp:dispatch  — send the executor at a drafted phase
│   ├── review/SKILL.md        # /rexymcp:review    — review a phase against the DoD, approve or bounce
│   └── escalate/SKILL.md      # /rexymcp:escalate  — handle a hard_fail briefing
└── templates/
    ├── STANDARDS.md           # generalized Definition of Done (placeholders resolved per project)
    └── WORKFLOW.md            # generalized phase-lifecycle workflow
```

The marketplace manifest lives at the **repo root**
(`../.claude-plugin/marketplace.json`) and points at `source: "./plugin"`.

## Install

```bash
# test mode — no permanent install
claude --plugin-dir ./plugin

# persistent install from the local checkout
claude plugin install ./plugin

# install from GitHub via the repo-root marketplace.json
claude plugin install github:<owner>/rexyMCP
```

### Google Antigravity

To use the rexyMCP workflow in Google Antigravity, add it as a plugin by copying or symlinking the `plugin` directory to your global customization root (e.g. `~/.gemini/config/plugins/rexymcp-plugin`).

You must also register the MCP server in your global `~/.gemini/antigravity/mcp_config.json`:

```json
{
  "mcpServers": {
    "rexymcp": {
      "command": "rexymcp",
      "args": ["serve", "--config", "./rexymcp.toml"]
    }
  }
}
```

The `rexymcp` binary must be on `$PATH` (e.g. `cargo install --path mcp`) — the
plugin's `.mcp.json` launches `rexymcp serve --config ./rexymcp.toml`.

## What the plugin provides

**Four skills** (`/rexymcp:<name>`):

| Skill | Model | Args | Role |
|-------|-------|------|------|
| `/rexymcp:architect` | opus | `[next]` | Bootstrap a project, explore & design it, and author phase docs. `next` drafts the next phase. |
| `/rexymcp:dispatch` | sonnet | `<phase>` | Send the executor at a drafted phase via `execute_phase`, then route to review or escalate. |
| `/rexymcp:review` | opus | `<phase>` | Rerun format/build/lint/test, check the STANDARDS DoD, then approve (→ `done`) or file a bug. |
| `/rexymcp:escalate` | opus | `<phase>` | Decide what to do with a `hard_fail` briefing: refined re-dispatch (default) or session takeover. |

**Seven MCP tools** exposed to Claude Code: `execute_phase`, `executor_health`,
`executor_log_search`, `executor_log_tail`, `get_turn`, `model_scorecard`, and
`model_profile`. See the [repository README](../README.md#mcp-tools) for details.

**Two templates** (`templates/`): `STANDARDS.md` and `WORKFLOW.md` carry
`{FORMAT_COMMAND}` / `{BUILD_COMMAND}` / `{LINT_COMMAND}` / `{TEST_COMMAND}`
placeholders that the `architect` skill resolves per target project and writes
into `<repo>/docs/dev/`. The `WORKFLOW.md` embedded here is the canonical copy —
calibration folds made to it propagate to every project that bootstraps from the
plugin.

## Note on long-running calls

`execute_phase` is long-running by design (a full phase can take many minutes).
Claude Code's `.mcp.json` schema does not expose a tool-timeout setting, so
liveness during a dispatch is surfaced out-of-band via the session log — tail it
with `rexymcp dashboard` / `rexymcp status`, or with the `executor_log_tail` /
`get_turn` MCP tools.
