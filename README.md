# rexyMCP

**Let Claude be the architect. Let a local model do the typing.**

rexyMCP turns Claude Code into a principal engineer that delegates real
implementation work to a small local LLM — Qwen, Gemma, or anything else you can
serve over an OpenAI-compatible endpoint. Claude decomposes your idea into
spec'd phases, hands each one to the executor, and reviews what comes back.
The local model writes the code. You keep the loop on your own hardware.

```
   you ──▶ Claude (architect) ──▶ MCP ──▶ local LLM (executor) ──▶ your repo
                  ▲                                  │
                  └────────── review / escalate ─────┘
```

## Why this exists

Claude Code's native subagents run **Claude models only** — there's no field for
a custom endpoint, and the provider overrides retarget the *entire* harness, not
one delegate. The single supported way to make Claude hand work to a
non-Anthropic model is an **MCP server**: Claude calls a tool, and whatever
happens inside that tool is opaque to it. rexyMCP puts a complete agentic loop
*inside* that tool.

The payoff: the local model's thousands of tokens of trial-and-error, tool
calls, and parser repairs never touch Claude's context. Claude sees one clean
structured result — and, when the executor gets stuck, a tight briefing it can
act on. Cheap, private, high-volume implementation work runs locally; Claude's
judgment is spent only where it counts.

## How it works

A single `execute_phase` call runs a headless agent loop against a phase spec and
a target repo:

1. Load the engineering standards + the phase doc; build the executor prompt.
2. Drive the local model through a tool-using loop — every file and shell
   operation **scoped to the target-repo root**.
3. Run model output through a **forgiving parser** that repairs the malformed
   tool calls small models love to emit, instead of just failing.
4. After edits, run the project's verifier (typecheck/build) and feed diagnostics
   back for a retry.
5. Watch for repetition loops, repeated verifier failures, and budget overflow —
   if the model is stuck, assemble a **briefing** and hand control back to Claude.
6. On clean completion, run the project's full `format`/`build`/`lint`/`test`
   command set and return a structured `PhaseResult`.

Escalation isn't a cloud call — it's **Claude itself**. A stuck executor returns
its briefing up through MCP, and Claude (already the architect) picks a lever:
re-dispatch with a sharper spec, take the session over, or resume. Every turn is
also written to a redacted **JSONL session log** Claude can query on demand, so a
lean result never means lost detail.

## Status

🚧 **Under active construction.** rexyMCP is being built phase-by-phase by its own
workflow (Claude architecting, a local model executing — dogfooding the product).

| Milestone | What it lands | State |
|---|---|---|
| M1 — Foundations | Cargo workspace, config, error model, CI, AI client, health check | ✅ done |
| M2 — Executor tools & security | tools + registry/router + scope confinement | 🔨 in progress |
| M3 — Forgiving parser | the 6-stage tool-call parser pipeline | ⏳ planned |
| M4 — Agent loop + governor | turn cycle, verifier, hard-fail detection, `PhaseResult`, session log | ⏳ planned |
| M5 — MCP server | `rmcp` stdio server, progress notifications, log-query tools | ⏳ planned |
| M6 — Plugin + skills | Claude Code plugin, architect/review/escalate skills, slash commands | ⏳ planned |
| M7 — Scorecard & routing | per-model competency matrix, tag-based routing | ⏳ planned |

The full design lives in [`docs/architecture.md`](docs/architecture.md).

## Quick start

Requires a recent Rust toolchain (`edition = "2024"`).

```bash
cargo build

# Point at a running OpenAI-compatible endpoint (vLLM / LM Studio / Ollama)
# and confirm rexyMCP can reach it and list models:
cargo run -p rexymcp -- health --config rexymcp.toml
cargo run -p rexymcp -- health --base-url http://localhost:11434/v1
```

The end-to-end "Claude drives a local model through your repo" flow arrives with
the MCP server (M5) and plugin (M6).

## Layout

```
executor/   library — the headless single-phase agent loop
mcp/         binary  — CLI today, the rmcp stdio MCP server tomorrow
docs/        architecture + the architect/executor dev process
```

rexyMCP borrows heavily from **Rexy**, the terminal agent for small local
models — lifting its AI client, forgiving parser, tools, governor, and security
layer. Rexy is a **parts donor, not a dependency**: code is vendored
and adapted, nothing links against it, and the parts that don't fit a headless
bridge (the TUI, the local planner, the cloud-escalation transport) are left
behind.

## Development

Contributors and AI agents work the phase-driven process documented in
[`AGENTS.md`](AGENTS.md), [`docs/dev/STANDARDS.md`](docs/dev/STANDARDS.md), and
[`docs/dev/WORKFLOW.md`](docs/dev/WORKFLOW.md). The gates:

```bash
cargo fmt --all --check
cargo build
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```
