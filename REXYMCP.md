# REXYMCP.md

The rexyMCP architect/executor workflow contract for this repository — whatever
agent acts as the architect reads this first. (Claude Code reaches it via the
`@REXYMCP.md` import in `CLAUDE.md`.)

## Read these first

This repo runs a strict **architect / executor** development process. Before writing code, read in order:

1. `docs/dev/STANDARDS.md` — the Definition of Done your work is reviewed against.
2. `docs/dev/WORKFLOW.md` — phase lifecycle, status transitions, Update Log templates, bug-report cycle.
3. `docs/dev/NEXT.md` — names the active phase; then read that phase doc end-to-end and its milestone README.
4. `docs/architecture.md` — the source of truth for the intended product design.

**Source-of-truth precedence when docs disagree:** `docs/architecture.md` > active phase doc > `STANDARDS.md`. If you spot a conflict, stop and file a blocker — do not pick a side.

## Commands

```bash
cargo build                                              # build
cargo clippy --all-targets --all-features -- -D warnings # lint gate (warnings are errors)
cargo fmt --all --check                                  # format check — verify only, writes nothing
cargo test                                               # all tests
cargo test <name>                                        # single test by name substring
cargo test -p rexymcp-executor                           # one crate
cargo run -p rexymcp -- health --config rexymcp.toml     # health-check entrypoint
```

Run the lint and test steps as **separate** invocations, not chained with `&&` — chaining against the same target dir can race the build cache.

**Never run `cargo fmt --all` (the writing form)** to pass the format gate — it reformats vendored/lifted code outside your phase scope. To fix formatting, run `rustfmt <file>` only on files the phase touched. If `--check` reports diffs in files you didn't modify, stop and file a blocker.

## Workspace layout

Two-crate Cargo workspace (`edition = "2024"`, clippy `all` denied at workspace level):

- **`executor/`** (lib `rexymcp-executor`, crate name `executor`) — the headless single-phase agent loop. Modules: `ai/` (OpenAI-compatible client + `AiClient` trait + `MockAiClient` in `ai/testing.rs`), `config`, `error`, `health`, `security/` (`scope` confinement to the target-repo root), `tools/` (`Tool` trait + `ToolRegistry` + `ToolResult` in `registry.rs`, built-in tools like `read_file`).
- **`mcp/`** (bin `rexymcp`) — currently a clap CLI exposing `health`; becomes the `rmcp` stdio MCP server (M5). Depends on `executor` in-process.

## Architecture (the big picture)

rexyMCP bridges **Claude Code (architect)** to a **local LLM (executor)** over an **MCP server**. Claude decomposes a spec into phase docs and dispatches each; a local model (Qwen/Gemma via vLLM/LM Studio/Ollama over an OpenAI-compatible endpoint) implements one phase; Claude reviews. The MCP boundary is load-bearing: the executor's inner transcript stays opaque, Claude sees only a structured `PhaseResult` (+ a `briefing` on failure).

Three layers (see `docs/architecture.md`): the `executor` library (turn cycle: parse → tool dispatch via governor → verifier → final command set), the `mcp` server, and a Claude Code plugin package (skills + slash commands, M6). Escalation returns the briefing to Claude rather than calling any cloud LLM — rexyMCP never links a cloud provider.

Milestones M1→M7 are listed in `docs/architecture.md` §Status; M1–M6 are done. Active work is M7 (model scorecard & routing) — see `docs/dev/NEXT.md`.

## Error model

- Programmer / infrastructure failures → `executor::error::Error` (a `thiserror` enum), propagated with `?`. `anyhow` is only acceptable at binary entry points (`main`).
- Model-visible outcomes (failed tool calls, parse failures, verifier disagreements) → a `ToolResult`-style value, **not** `Result::Err`.
- No `.unwrap()` / `.expect()` / `panic!()` in production paths (test code is exempt). No `unwrap_or_default()` on a `Result` you care about.

## Hard rules (stop-and-file-a-blocker triggers)

Do not, without explicit phase-doc authorization: add a dependency (`Cargo.toml`) or run `cargo add`/`cargo remove`; write `unsafe`; widen scope (note adjacent bugs in "Notes for review", don't fix them); add `#[allow]`/`#[ignore]` to mask a diagnostic; leave `TODO`/`FIXME`/`dbg!`/`println!`/commented-out code; or edit `Cargo.toml`, `rustfmt.toml`, `clippy.toml`, `.github/workflows/*`, `docs/architecture.md`, `STANDARDS.md`, `WORKFLOW.md`, or any phase doc other than the active one.

## Testing

Hermetic (no real network, no host state outside a `tempfile::TempDir`) and deterministic (no `sleep`, no real `Utc::now()` — inject a clock, no unseeded RNG). Use `MockAiClient` for any `AiClient` interaction. Unit tests in a `#[cfg(test)] mod tests` block at the file bottom; integration tests in `tests/`. Live-LLM tests are `#[ignore]`-gated and only written when a phase asks. Test names describe behavior in present tense (`loads_default_when_no_config`).

## Commits

One conventional commit per logical change (`feat:`/`fix:`/`refactor:`/`test:`/`docs:`/`chore:`); body explains *why*, not *what*. A phase doc's status flip + Update Log additions are committed together with the code. CI (`.github/workflows/ci.yml`) runs fmt-check, clippy, and tests on push/PR to `master`.
