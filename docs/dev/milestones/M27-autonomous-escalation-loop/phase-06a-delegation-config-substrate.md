# Phase 06a: Per-role model delegation config substrate

**Milestone:** M27 — Autonomous Escalation Loop
**Status:** in-progress
**Depends on:** phase-05b (done)
**Estimated diff:** ~70 lines
**Tags:** language=rust, kind=feature, size=s

## Goal

Add the two `[architect]` config keys the `/rexymcp:auto` loop (phase-06b) will
read to delegate per-role work to subagents on a chosen model: `dispatch_model`
and `review_model`. Additive and inert — nothing consumes them until 06b's skill
does. This is the dispatchable Rust half of the phase-06 split; 06b (the skill +
loop report + WORKFLOW mirror) is direct-execution and stays separate.

## Architecture references

Read before starting:

- `docs/dev/milestones/M27-autonomous-escalation-loop/README.md` § "Per-role
  model delegation (decided 2026-07-08, with the user)" — the design these two
  keys implement, including the load-bearing **inherit-by-default** rule and the
  deliberate absence of a `draft_model` key.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture reference above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

`ArchitectConfig` lives in `executor/src/config.rs:75-89` (struct) with a manual
`Default` impl at `executor/src/config.rs:91-101`. Today it carries only the cost-
rate fields (`model`, `input_per_mtok`, `output_per_mtok`,
`cache_read_per_mtok`, `cache_creation_per_mtok`). The struct already carries
`#[serde(default)]`, so adding fields is back-compat by construction.

The `rexymcp init` template's `[architect]` block is at `mcp/src/init.rs:81-88`
(commented example keys).

## Spec

The semantics, verbatim from the README: **`None` means inherit** — an unset role
model tells the loop to omit the subagent model override, which the native
subagent default resolves to the session (architect) model. An unset role model
does **NOT** fall back to `[architect] model` (that field is the cost-rate model,
a separate concern). There is deliberately **no** `draft_model` key.

1. **Add the two role-model fields** — in `executor/src/config.rs`, add
   `pub dispatch_model: Option<String>` and `pub review_model: Option<String>`
   to `ArchitectConfig` (struct at lines 75-89) and to its `Default` impl (lines
   91-101, both default to `None`). Give each a one-line `///` doc-comment naming
   the role it delegates and stating that `None` = inherit the session/architect
   model (not `[architect] model`). Do **not** add a resolver method — the two
   `Option<String>` fields *are* the resolution (`Some` = override, `None` =
   inherit); a helper here would be premature abstraction with no Rust caller
   (the consumer is 06b's prose skill, which reads the toml directly).

2. **Document the keys in the init template** — in `mcp/src/init.rs`, add two
   commented lines to the `[architect]` block (after the cache-rate lines, ~line
   88) mirroring the existing commented-key style:

   ```toml
   # dispatch_model = "claude-sonnet-5"   # /rexymcp:auto delegates dispatch to this model (default: inherit)
   # review_model = "claude-sonnet-5"     # /rexymcp:auto delegates review to this model (default: inherit)
   ```

## Acceptance criteria

- [ ] `cargo build` succeeds with zero new warnings; `clippy`, `fmt --check`,
      and `cargo test` all pass.
- [ ] A `rexymcp.toml` with `[architect] dispatch_model` / `review_model` set
      loads and both fields are `Some(...)` with the configured strings.
- [ ] A `rexymcp.toml` whose `[architect]` block omits both keys loads and both
      fields are `None` (back-compat with every existing config).
- [ ] The unset case is `None`, **not** the value of `[architect] model` (the
      no-fallback negative).

## Test plan

Add unit tests to the `#[cfg(test)] mod tests` block in `executor/src/config.rs`,
mirroring `load_parses_toml_executor_block` (config.rs:594) for the write-a-toml-
then-`Config::load` idiom:

- `load_parses_architect_role_models` — a config with `[architect] model =
  "claude-opus-4-8"`, `dispatch_model = "claude-sonnet-5"`, `review_model =
  "claude-haiku-4-5-20251001"` loads; assert each field equals its string.
- `architect_role_models_default_to_none` — a config with an `[architect]` block
  that omits both keys (e.g. only `model` set) loads with both role fields
  `None`. Include the **negative assertion** that `dispatch_model` is `None` even
  though `model` is `Some` — pinning that unset does not fall back to `model`.

## End-to-end verification

The keys are consumed by 06b's skill (which reads the toml) and by `Config::load`;
06a surfaces no new command output. Verify the real config-load path against the
running binary:

- `cargo run -p rexymcp -- doctor --config <toml-with-both-keys-set>` exits 0
  (proves the new keys parse through the real binary's config load).
- `cargo run -p rexymcp -- doctor --config <toml-without-the-keys>` exits 0
  (back-compat: an existing config still loads).

Quote both invocations' outcome in the completion Update Log.

## Authorizations

None. (No new dependency; no `docs/architecture.md` edit — the design is already
recorded in the milestone README.)

## Out of scope

- The `/rexymcp:auto` skill, the loop report, subagent delegation mechanics, and
  the WORKFLOW plugin-template mirror — all phase-06b.
- A resolver method or any Rust consumer of the new fields — none exists yet;
  adding one now would be dead, untested-in-context state.
- `rexymcp calibrate` — its `[architect]` skeleton (calibrate.rs:51) stays as-is;
  a user's explicit keys already survive re-calibrate via item preservation.
- The executor's `TokenBreakdown` and any harvester/accounting change — 05b
  already handles role-model rates architect-side.

## Update Log

### Update — 2026-07-09 18:01 (started)

**Executor:** model (phase-06a run)

**Work:** Adding `dispatch_model` and `review_model` fields to `ArchitectConfig`, updating `Default` impl, all existing struct literals, and the init template. Two new unit tests for parsing and default-to-None behavior.

<!-- entries appended below this line -->
