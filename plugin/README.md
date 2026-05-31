# rexyMCP Claude Code Plugin

This directory contains the Claude Code plugin package for rexyMCP. It bundles
the MCP server (`rexymcp serve`) with the architect/executor workflow: skills,
slash commands, and embedded templates.

## Layout

```
plugin/
├── .claude-plugin/
│   └── plugin.json            # Plugin manifest (name: rexymcp)
├── .mcp.json                  # MCP server registration
├── README.md                  # This file
└── skills/
    ├── architect/SKILL.md     # /rexymcp:architect — explore + design (stub, phase-04)
    ├── dispatch/SKILL.md      # /rexymcp:dispatch — send executor to a phase (stub, phase-05)
    └── review/SKILL.md        # /rexymcp:review — review executor output (stub, phase-05)
```

## Install

The plugin can be tested locally during development:

```bash
claude --plugin-dir ./plugin
```

After installation, the skills appear as `/rexymcp:architect`,
`/rexymcp:dispatch`, and `/rexymcp:review`.

The `rexymcp` binary must be on `$PATH` (e.g., `cargo install --path mcp`).

## Status

**M6 phase-01 (current):** Plugin scaffold complete — manifest, MCP server
registration, and three skill stubs. The stubs have no behavior yet.

**Coming in later M6 phases:**

| Phase | What lands |
|-------|-----------|
| phase-02 | Embedded templates (`executor_contract`, `STANDARDS`, `WORKFLOW`) |
| phase-03 | Executor wires embedded contract at runtime |
| phase-04 | `architect` skill body (bootstrap + pre-injection + design) |
| phase-05 | `dispatch` and `review` skill bodies |
| phase-06 | End-to-end dogfood against a real third-party repo |

See `docs/dev/milestones/M6-plugin/README.md` for the full milestone plan.

## Known limitations

Claude Code's `.mcp.json` schema does not currently expose a tool-timeout
setting. The architecture's "raise toward 10 minutes" target depends on
the client honoring long-running tool calls without enforcing a default
interrupt. **Open question for the M6 phase-06 dogfood:** does Claude
Code's MCP client interrupt long `execute_phase` calls? If yes, a follow-up
is needed (either upstream to Claude Code or to add a heartbeat-driven
keepalive path). The M5 phase-05b progress notifications are the primary
liveness signal the client sees during long calls.
