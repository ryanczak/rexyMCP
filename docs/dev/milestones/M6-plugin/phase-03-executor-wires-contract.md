# Phase 03: executor wires the embedded contract

**Milestone:** M6 — Plugin + architect/review skills
**Status:** review
**Depends on:** M6 phase-02 (done) — `executor/templates/executor_contract.md` exists with the four `{...}_COMMAND` placeholders. M4 phase-07a — `agent::prompt::assemble_system_prompt` is the prompt-assembly seam.
**Estimated diff:** ~300 lines (new contract module + signature change + cross-cutting drop of `executor_contract` plumbing + tests)
**Tags:** language=rust, kind=feature, size=m

## Goal

Wire the **embedded executor contract** into the prompt-assembly path so every
`execute_phase` call prepends the substituted contract to the system prompt.
Phase-02 shipped the template content; phase-03 makes it load-bearing.

Three things land:

1. **A new `executor/src/agent/contract.rs` module** that `include_str!`s
   `executor/templates/executor_contract.md` and exposes
   `assemble_executor_contract(commands: &CommandConfig) -> String` —
   substitutes the four `{...}_COMMAND` placeholders from `cfg.commands`.
2. **`assemble_system_prompt`'s signature changes** — drops the
   `executor_contract: &str` parameter, takes `commands: &CommandConfig`
   instead, computes the contract internally via `assemble_executor_contract`.
3. **The `executor_contract` plumbing dies workspace-wide** — `PhaseInput`'s
   field, the `mcp` wiring through `RunPhaseConfig` / `run_phase` /
   `execute_phase_inner` / the CLI handler, the test constructors. All 13
   current `executor_contract` references go away or are repointed.

The contract is now **truly embedded-only** per the architecture: the executor
crate carries it as a baked-in template, the loop substitutes the project's
commands at every turn-cycle step 1. The MCP server no longer passes an
empty-string placeholder — the parameter ceases to exist.

## Architecture references

- `docs/architecture.md` — Layer 3 "Embedded templates": *"the executor
  contract and `STANDARDS.md` are what the `executor` crate prepends to every
  phase's system prompt (Layer 1, turn-cycle step 1); the contract is
  **embedded-only** — a rexyMCP-driven project never carries a root
  `AGENTS.md` or an executor-contract file."*
- "The executor turn cycle" step 1 (assemble prompt from contract + standards
  + phase doc).
- M6 README — "Executor contract is embedded-only" (this is the *load-bearing*
  design choice this phase finally honors).
- M6 phase-02: `executor/templates/executor_contract.md` — the template this
  phase consumes.
- M4 phase-07a: `agent::prompt::assemble_system_prompt` — the existing seam.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` and the M6 README.
2. Read this entire phase doc.
3. Confirm `executor/templates/executor_contract.md` exists (phase-02
   shipped it) and contains exactly the four authorized placeholders.
4. Confirm `executor::config::CommandConfig` shape:
   `{ format: Option<String>, build: Option<String>, lint: Option<String>,
   test: Option<String> }`.
5. Confirm `assemble_system_prompt`'s current signature
   (`executor/src/agent/prompt.rs`) is
   `(executor_contract: &str, standards: &str, phase_doc: &str) -> String`.
6. Confirm the current `executor_contract` plumbing sites (13 refs across
   `executor/src/agent/mod.rs`, `mcp/src/runner.rs`, `mcp/src/server.rs`,
   `mcp/src/main.rs`).

## Spec

### 1. New module — `executor/src/agent/contract.rs`

```rust
use crate::config::CommandConfig;

/// The embedded executor-contract template, baked in at compile time.
/// Lives at executor/templates/executor_contract.md; see M6 phase-02.
const TEMPLATE: &str = include_str!("../../templates/executor_contract.md");

/// Marker used when a CommandConfig field is `None`. The contract template
/// references all four commands; substituting an empty string would produce
/// confusing output like `run \`\` (the configured format-check command)`.
/// This sentinel is unambiguous and tells the model the situation when it
/// reads the assembled prompt.
const UNCONFIGURED: &str = "(not configured)";

/// Substitute the four `{…_COMMAND}` placeholders in the embedded contract
/// template with values from `commands`. Unset commands render as the
/// `UNCONFIGURED` sentinel.
///
/// Returns the substituted contract body. Pure; no I/O.
pub fn assemble_executor_contract(commands: &CommandConfig) -> String;
```

Implementation: plain `str::replace` for each placeholder, four passes.
Order doesn't matter (placeholders are distinct, no nesting). Each
`commands.<field>.as_deref().unwrap_or(UNCONFIGURED)` gives the substitution
value.

Declared in `executor/src/agent/mod.rs` as `pub mod contract;`.

### 2. Signature change — `executor/src/agent/prompt.rs`

```rust
// BEFORE
pub fn assemble_system_prompt(
    executor_contract: &str,
    standards: &str,
    phase_doc: &str,
) -> String;

// AFTER
pub fn assemble_system_prompt(
    commands: &CommandConfig,
    standards: &str,
    phase_doc: &str,
) -> String;
```

Body now calls `let executor_contract = contract::assemble_executor_contract(commands);`
internally, then concatenates as before (contract + standards + phase doc,
with the same separator format).

The existing test in `prompt.rs` (`assemble_system_prompt("CONTRACT_BODY",
"STANDARDS_BODY", "PHASE_BODY")`) updates to construct a `CommandConfig`
fixture and assert the assembled prompt contains the *embedded* contract
(or rather, the substituted form — see Test plan).

### 3. Drop `executor_contract` from `PhaseInput`

In `executor/src/agent/mod.rs`:

- Remove `pub executor_contract: String` from `PhaseInput`.
- Update the `execute_phase` call site (line ~111) to pass
  `deps.commands` instead of `&input.executor_contract` to
  `assemble_system_prompt`.
- Update the test fixture (line ~1109 — `executor_contract:
  "CONTRACT".to_string()`) to drop the field.

### 4. Drop the field through the mcp layer

Cross-cutting deletion. Each of these constructions / signatures loses the
`executor_contract` field/param:

- **`mcp/src/runner.rs`:**
  - `RunPhaseConfig` struct (phase-05b's struct-grouping fix) — drop
    `executor_contract: &'a str` field.
  - `run_phase` / `run_phase_with` — stop building `PhaseInput` with it.
- **`mcp/src/server.rs`:**
  - `execute_phase_inner` — drop the implicit empty-string we currently
    pass downstream.
  - `execute_phase_inner_with_client` (phase-05b's testability seam) — same.
  - Any tests constructing these calls — drop the param.
- **`mcp/src/main.rs`:**
  - The `RunPhase` clap handler stops passing `executor_contract: ""` into
    `RunPhaseConfig`.

The CLI `run-phase` subcommand keeps reading `STANDARDS.md` from the target
repo (that side stays). Only the contract plumbing dies.

After the change, `grep -rn 'executor_contract' executor/ mcp/` should show
**zero hits** (production code; test code may still mention it in
test-name strings or comments, but no `executor_contract` *parameter* or
*field* should survive).

### 5. The four phase-05b CLI parse tests still pass

The CLI parse tests for `run-phase` (`cli_parse_run_phase_with_all_args`,
etc.) don't reference the contract directly — `RunPhase` is a clap
subcommand and the contract was never on its CLI surface. They should
pass unchanged. Confirm.

## Adaptations / decisions

1. **`UNCONFIGURED` sentinel instead of empty-string substitution.** Empty
   string produces awkward prompt text (`run \`\` …`) and obscures the
   "this isn't set" signal. The sentinel is unambiguous and the model
   reading the assembled prompt knows to file a blocker if the missing
   command matters for the phase.
2. **Placeholder substitution is plain `str::replace`, not a templating
   engine.** Phase-02 pinned `{NAME}` literal syntax — no Jinja, no
   conditionals, no loops. Four `str::replace` calls is enough.
3. **`PhaseInput.executor_contract` is removed, not deprecated.** rexyMCP
   has one consumer (the loop); a deprecation cycle is pure cost. Cleanest
   to drop the field and update the few constructors at once.
4. **`assemble_system_prompt` takes `&CommandConfig`, not `&Config`.**
   Smaller blast radius — the function only needs the command set, not
   the executor endpoint or budget knobs. Matches the existing seam
   shape (other `LoopDeps` fields are likewise narrowly typed).
5. **No new dependency.** `include_str!` is a stdlib macro; `str::replace`
   is stdlib.

## Acceptance criteria

- [ ] `executor/src/agent/contract.rs` exists. Declared `pub mod contract;`
      in `executor/src/agent/mod.rs`. Exports `assemble_executor_contract`
      + the `UNCONFIGURED` sentinel as a `pub const` (so tests can reference
      it by name).
- [ ] `assemble_executor_contract(commands)` produces a string that:
  - is non-empty and contains the contract template's opening preamble;
  - has all four `{...}_COMMAND` placeholders **substituted** (zero
    `{...}_COMMAND` substrings survive in the output);
  - substitutes a `None` field with the `UNCONFIGURED` sentinel;
  - substitutes a `Some("foo")` field with the literal `foo` (no extra
    quoting).
- [ ] `assemble_system_prompt`'s signature is
      `(commands: &CommandConfig, standards: &str, phase_doc: &str) -> String`.
      Internally calls `contract::assemble_executor_contract(commands)` and
      composes the three pieces in order (contract first, standards next,
      phase doc last) with the same separator the M4 implementation used.
- [ ] `PhaseInput` no longer has an `executor_contract` field.
- [ ] **Zero `executor_contract` symbols survive** in production code —
      `grep -rn '\bexecutor_contract\b' executor/src/ mcp/src/` returns nothing
      outside `#[cfg(test)]` blocks. (Test-code mentions in comments
      describing the *historical* parameter are tolerated only if they're
      genuinely explanatory; prefer removing them.)
- [ ] All existing tests across the workspace compile and pass after the
      cross-cutting drop (test fixtures that constructed `PhaseInput { …,
      executor_contract: "…", … }` are updated; tests that called
      `assemble_system_prompt("CONTRACT_BODY", …)` are updated).
- [ ] **Two new tests** in `executor/src/agent/contract.rs`:
  - `substitutes_all_four_commands_when_set` — fixture
    `CommandConfig { format: Some("a"), build: Some("b"),
    lint: Some("c"), test: Some("d") }`; assert output contains
    `a` / `b` / `c` / `d` and zero `{...}_COMMAND` substrings.
  - `unset_command_renders_as_unconfigured_sentinel` — fixture with
    one or more `None` fields; assert the rendered string contains
    `UNCONFIGURED` exactly N times (N = count of `None`s) at the
    placeholder positions.
- [ ] **One new test** in `executor/src/agent/prompt.rs`:
  - `system_prompt_includes_substituted_contract` — construct a
    `CommandConfig` fixture + sample standards + sample phase doc;
    assert the assembled prompt contains both the substituted contract
    body *and* the standards body *and* the phase-doc body, in that
    order.
- [ ] **Handler success-path test** (calibration carry-forward —
      phase-04 → phase-05a bar): no new mcp-side handler test required
      *only because* the existing phase-02/03 handler tests already exercise
      `execute_phase_inner` end-to-end. Verify those still pass; that's
      the regression net for this phase.
- [ ] No `#[allow]`; no `unwrap()` / `expect()` / `panic!()` in production
      paths (test code exempt); no Rexy refs in new files; no new
      dependency.
- [ ] **Calibration carry-forward (mandatory):** declare every scope
      deviation in "Notes for review". M6's `git status`-before-commit
      lesson on the architect side doesn't apply to the executor, but
      the *declare-deviations* discipline does.
- [ ] All four required commands pass with zero new warnings.

## Test plan

Hermetic + deterministic. In `executor/src/agent/contract.rs`
`#[cfg(test)] mod tests`:

- **`substitutes_all_four_commands_when_set`** — fixture above; assert no
  `{...}_COMMAND` substring survives; assert each provided value appears
  in the output at least once.
- **`unset_command_renders_as_unconfigured_sentinel`** — exhaustive
  variants (one `None`, all `None`, mixed); assert sentinel count at
  each placeholder position.
- **`output_starts_with_contract_preamble`** — assert the rendered
  string starts with the template's first non-empty line (the "Executor
  Contract" heading or the preamble paragraph — pick whichever is more
  stable as a fingerprint).
- **`placeholder_set_is_exactly_the_four_authorized`** — regex/scan the
  *template* (`TEMPLATE` const) for any `{...}` substring; assert the
  set is exactly `{FORMAT_COMMAND, BUILD_COMMAND, LINT_COMMAND,
  TEST_COMMAND}`. This is a regression net against future template
  edits accidentally introducing a new placeholder the substitution
  code doesn't handle.

In `executor/src/agent/prompt.rs` `#[cfg(test)] mod tests` (extend or
replace existing):

- **`system_prompt_includes_substituted_contract`** — as in Acceptance.
- **`system_prompt_order_is_contract_then_standards_then_phase_doc`** —
  assert the three sections appear in the expected order in the output
  string (find their offsets, compare).

No new tests required in `mcp/`; existing phase-02 / 03 server tests
exercise the path end-to-end. Confirm they pass post-change.

## End-to-end verification

> Not applicable — this is internal prompt-assembly wiring. The contract
> reaching a live local LLM is exercised end-to-end at M6 phase-06
> (dogfood). Phase-03's job is to make the wiring correct; phase-06's
> job is to confirm the assembled prompt actually steers the model.

## Authorizations

- [x] **May create** `executor/src/agent/contract.rs`.
- [x] **May modify** `executor/src/agent/mod.rs` (declare `pub mod
      contract;`; drop `PhaseInput.executor_contract`; update the call
      site; update the test fixture at line ~1109).
- [x] **May modify** `executor/src/agent/prompt.rs` (signature change +
      body change + extend tests).
- [x] **May modify** `mcp/src/runner.rs` (drop
      `RunPhaseConfig.executor_contract` + the field passing).
- [x] **May modify** `mcp/src/server.rs` (drop `executor_contract` from
      `execute_phase_inner` / `execute_phase_inner_with_client` + the test
      sites that constructed it).
- [x] **May modify** `mcp/src/main.rs` (drop the empty-string contract
      from the `RunPhase` handler's `RunPhaseConfig` construction).
- [ ] **No new dependencies.** `include_str!` + `str::replace` are stdlib.
- [ ] May **NOT** modify `executor/templates/executor_contract.md`
      (phase-02's deliverable; this phase consumes it as-is).
- [ ] May **NOT** modify `plugin/templates/STANDARDS.md` or
      `plugin/templates/WORKFLOW.md` (phase-02's deliverables).
- [ ] May **NOT** modify `docs/architecture.md`, `STANDARDS.md`,
      `WORKFLOW.md`, `AGENTS.md`, or any other phase doc.
- [ ] May **NOT** write any skill body (phase-04/05), implement the
      bootstrap routine (phase-04), or run the dogfood (phase-06).

## Out of scope

- **Skills + bootstrap** — phases 04/05.
- **Dogfood** — phase-06.
- **Per-language tool-set documentation** baked into the contract — the
  contract is universal; per-project specifics are the architect skill's
  *pre-injection* responsibility (phase-04).
- **A `Result`-returning variant of `assemble_executor_contract`** —
  substitution is infallible; the only error mode would be "template is
  malformed", which `include_str!` makes a compile-time error.
- **Validating the rendered contract length against the model's context
  window** — phases 02 of M4 (Budget) already bounds the overall prompt;
  the contract is part of the system prompt that the budget accounts for.
- **A "test the contract isn't too long" assertion** — premature; revisit
  if dogfood shows it eats too much context.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-05-31 15:00 (started)

**Executor:** opencode (Qwen/Qwen3.6-27B-FP8)

Implementing spec tasks 1-5: new `contract.rs` module with `include_str!` + `str::replace` substitution; signature change on `assemble_system_prompt`; cross-cutting removal of `executor_contract` from `PhaseInput`, `RunPhaseConfig`, `AssemblyInput`, `execute_phase_inner`/`_with_client`, and the CLI handler.

### Update — 2026-05-31 15:15 (complete)

**Summary:** Created `executor/src/agent/contract.rs` with `include_str!` of the template and `assemble_executor_contract(commands)` substituting the four `{...}_COMMAND` placeholders via `str::replace`. Changed `assemble_system_prompt` to take `&CommandConfig` instead of `executor_contract: &str`, computing the contract internally. Removed `executor_contract` from `PhaseInput`, `RunPhaseConfig`, `AssemblyInput`, `execute_phase_inner`, `execute_phase_inner_with_client`, and the CLI `RunPhase` handler. All test fixtures updated. No scope deviations.

**Acceptance criteria:** all ticked above.

**Commands:**

```
cargo fmt --all --check
(no output — clean)

cargo build 2>&1 | tail -20
Compiling rexymcp-executor v0.1.0 (/home/matt/src/rexyMCP/executor)
Compiling rexymcp v0.1.0 (/home/matt/src/rexyMCP/mcp)
    Finished `dev` profile [unoptimized+debuginfo] target(s) in 1.43s

cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail -20
Checking rexymcp-executor v0.1.0 (/home/matt/src/rexyMCP/executor)
Checking rexymcp v0.1.0 (/home/matt/src/rexyMCP/mcp)
    Finished `dev` profile [unoptimized+debuginfo] target(s) in 1.14s

cargo test 2>&1 | tail -30
test result: ok. 518 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out (executor)
test result: ok. 117 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out (mcp)
```

**End-to-end verification:** Not applicable — this is internal prompt-assembly wiring. The contract reaching a live local LLM is exercised end-to-end at M6 phase-06 (dogfood).

**Files changed:**
- `executor/src/agent/contract.rs` — new module: `include_str!` template + `assemble_executor_contract` + `UNCONFIGURED` const + 4 tests
- `executor/src/agent/mod.rs` — declared `pub mod contract`; dropped `PhaseInput.executor_contract`; updated `execute_phase` call site and test fixture
- `executor/src/agent/prompt.rs` — signature change to `(commands: &CommandConfig, standards: &str, phase_doc: &str)`; computes contract internally; updated existing test + 2 new tests
- `mcp/src/runner.rs` — dropped `executor_contract` from `RunPhaseConfig`, `AssemblyInput`, and `run_phase_with`; updated 2 test fixtures
- `mcp/src/server.rs` — dropped `executor_contract` from `execute_phase_inner`/`_with_client`
- `mcp/src/main.rs` — dropped `executor_contract` from the `RunPhase` handler's `RunPhaseConfig` construction

**New tests:**
- `substitutes_all_four_commands_when_set` in `executor/src/agent/contract.rs`
- `unset_command_renders_as_unconfigured_sentinel` in `executor/src/agent/contract.rs`
- `output_starts_with_contract_preamble` in `executor/src/agent/contract.rs`
- `placeholder_set_is_exactly_the_four_authorized` in `executor/src/agent/contract.rs`
- `system_prompt_includes_substituted_contract` in `executor/src/agent/prompt.rs`
- `system_prompt_order_is_contract_then_standards_then_phase_doc` in `executor/src/agent/prompt.rs`

**Commits:**
- Pending — will commit after status flip.

**Notes for review:** None. Implementation matches spec exactly.

verification: fmt OK · clippy OK · tests 635 passed (518 executor + 117 mcp) · build OK
