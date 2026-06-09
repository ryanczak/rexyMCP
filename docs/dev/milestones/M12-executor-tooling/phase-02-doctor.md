# Phase 02: `rexymcp doctor` — toolchain-availability command

**Milestone:** M12 — Executor Tooling
**Status:** done
**Depends on:** phase-01 (the `Skipped` runtime-degrade fix; this is its
human-present counterpart)
**Estimated diff:** ~200 lines
**Tags:** language=rust, kind=feature, size=m

## Goal

Give the human a single command — `rexymcp doctor` — that reports, before any
phase is dispatched, whether the toolchains rexyMCP shells out to are installed
and on PATH. It checks two tiers and treats them differently per the M12 Arc 0
rule:

- **Tier 0 — the configured `[commands]` toolchain** (`format`/`build`/`lint`/
  `test`/`lint_fix`). These are **required**: a phase cannot reach `done` without
  `build`/`test` passing. A missing Tier-0 binary makes `doctor` **exit non-zero**.
- **Tier 1 — the per-language verifier enhancers** (`cargo`/`tsc`/`ruff`). These
  **augment** Tier 0 with incremental checks and **fail open** — phase-01 already
  degrades a missing one to a `Skipped` advisory at runtime. `doctor` reports them
  as advisory and a missing one **never** changes the exit code.

This is the **fail-hard-advisory where a human can act** half of Arc 0 (phase-01
was the **fail-open at runtime** half). Detection lives here and in the architect
bootstrap — **never** in `rexymcp init`, which stays a static scaffolder (so a
project in a language with no built-in verifier, e.g. Zig, runs on the Tier-0
command set alone and `doctor` says so rather than flagging it as broken).

## Architecture references

Read before starting:

- `docs/architecture.md#status` — M12 Arc 0 ("toolchain robustness"): a
  `rexymcp doctor` command + architect bootstrap detection present a resolution
  plan; detection never lives in `rexymcp init`.
- `docs/dev/STANDARDS.md` §2.6 — runtime toolchain binaries are distinct from
  crate deps; a missing **runtime** binary is a detect-and-advise concern, not a
  compile-time one.
- `docs/dev/WORKFLOW.md` § "Validation features depend on the target toolchain" —
  the Tier-0 (required) vs Tier-1 (enhancer, fail-open) split this command reports
  against.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom (note §2.6 and §3.2 — pure plumbing
   that only forwards args needs no test, but the classifier/resolver helpers
   here are pure functions and **do** need tests per §3.1).
2. Read this entire phase doc before touching any code.
3. Run `cargo test -p rexymcp 2>&1 | tail -3` and record the result line (the
   **mcp** crate — this phase adds code only there; expected baseline **270
   passed**). After this phase the *passed* count rises by the new tests you add.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

`rexymcp` is a clap CLI (`mcp/src/main.rs`). Each subcommand has a `Commands`
enum variant + a match arm in `main()`, and most non-trivial commands live in
their own module (`mcp/src/init.rs`, `runs.rs`, `status.rs`, …) declared at the
top of `main.rs`. There is **no** `doctor` command today.

### The config the command reads — `executor/src/config.rs`

```rust
pub struct CommandConfig {
    pub format: Option<String>,
    pub build: Option<String>,
    pub lint: Option<String>,
    pub test: Option<String>,
    pub lint_fix: Option<String>,
}
```

`Config::load_with_env(&path)` loads the whole `Config` (which has a `commands:
CommandConfig` field). Each command is a **full shell string** like
`"cargo fmt --all --check"` — the binary is the first whitespace-delimited token.

### The Tier-1 enhancer binaries — `executor/src/governor/verifier.rs`

The verifier shells out to exactly three per-language binaries (confirmed at
`verify_rust` `:263`, `verify_typescript` `:375`, `verify_python` `:441`):

| Language | Binary | Invocation | Install hint (reuse phase-01's wording) |
|---|---|---|---|
| Rust | `cargo` | `cargo check --message-format=json` | `install the Rust toolchain via https://rustup.rs` |
| TypeScript | `tsc` | `tsc --noEmit --pretty=false` | `npm install -g typescript` |
| Python | `ruff` | `ruff check …` | `pip install ruff` |

These are the same three binaries + hints phase-01's `spawn_failure` names; keep
the wording identical so `doctor` and the runtime advisory agree.

### Worked example — the established CLI command shape

**Module + pure logic + `run`** (mirror `mcp/src/init.rs`): a module file with
pure helpers, hermetic `TempDir` unit tests, and a thin `run` entry point.

**Dispatch arm with non-zero exit on failure** (`main.rs`, the `Health` arm,
`:163`): the canonical pattern for "print a report, exit 1 when something's
wrong":

```rust
Commands::Health { config, base_url } => {
    let config_path = config.unwrap_or_else(|| PathBuf::from("rexymcp.toml"));
    let mut cfg = Config::load_with_env(&config_path)?;
    // …
    if result.reachable {
        // print …
        Ok(())
    } else {
        eprintln!("unreachable: {}", result.base_url);
        std::process::exit(1);
    }
}
```

**`--json` alongside a human render** (the `Runs`/`Scorecard`/`Status` arms): each
serializes the report struct with `serde_json::to_string_pretty` when `--json` is
passed, else prints a human string. `doctor` follows the same convention.

## Spec

Additive throughout: one new module, one new clap variant, one new dispatch arm,
one `mod` declaration. Nothing existing changes shape.

### Task 1 — new module `mcp/src/doctor.rs`

Create the file with the following **pure** helpers and report types. The
signatures are the contract the tests bind to — keep them exact; internal
representation is yours.

**1a. Binary extraction:**

```rust
/// The binary a configured command shells out to: its first
/// whitespace-delimited token. `None` for a blank/empty command.
pub fn command_binary(command: &str) -> Option<&str> {
    command.split_whitespace().next()
}
```

(Known limitation, acceptable: an env-var-prefixed command like
`RUSTFLAGS=… cargo build` would report the assignment as the binary. The
configured `[commands]` are plain invocations; do **not** add parsing for this —
note it in a one-line code comment only if you think a future reader needs it.)

**1b. PATH resolution (pure — search dirs are injected, never read from the real
environment here):**

```rust
/// Resolve a binary against a list of search directories. A name
/// containing a path separator is treated as a path and checked
/// directly; a bare name is probed as `dir.join(name)` in each
/// search dir. Returns the first match that is an existing *file*.
pub fn resolve_binary(binary: &str, search_paths: &[PathBuf]) -> Option<PathBuf>
```

Behavior that **must** hold (these are the pinned boundaries — see Test plan):
- A bare name resolves to `dir.join(name)` only when that path **`is_file()`** —
  a *directory* of that name in a search dir does **not** count as found.
- Matching is **exact**, not substring: searching for `cargo` must **not** match
  a file named `cargo-clippy` or `cargocult` in a search dir.
- A name containing `std::path::MAIN_SEPARATOR` (e.g. `/usr/bin/cargo`) is checked
  as that literal path via `is_file()`, ignoring `search_paths`.

**1c. Report types** (serialized for `--json`, so the `Serialize` derive is
load-bearing — keep it):

```rust
#[derive(Debug, Clone, Serialize)]
pub struct ToolStatus {
    pub binary: String,
    pub found: bool,
    pub note: String, // command role(s) for Tier 0; language + remedy for Tier 1
}

#[derive(Debug, Clone, Serialize)]
pub struct DoctorReport {
    pub tier0: Vec<ToolStatus>,
    pub tier1: Vec<ToolStatus>,
}

impl DoctorReport {
    /// True iff every required (Tier-0) tool was found. Tier-1
    /// status never affects this — enhancers fail open.
    pub fn tier0_ok(&self) -> bool {
        self.tier0.iter().all(|t| t.found)
    }
}
```

**1d. Report builder (pure — takes the search paths, does not read PATH):**

```rust
/// Build the toolchain report from the configured commands and
/// the known per-language verifier enhancers, resolving each
/// binary against `search_paths`.
pub fn build_report(commands: &CommandConfig, search_paths: &[PathBuf]) -> DoctorReport
```

- **Tier 0:** walk the five configured commands in this fixed order — `format`,
  `build`, `lint`, `test`, `lint_fix` — skipping any that are `None`. For each
  present command, take its `command_binary`. Produce **one `ToolStatus` per
  distinct binary** (dedup by binary name): the first time a binary is seen,
  resolve it against `search_paths` and record `found`; on a later command using
  the same binary, append that command's role to the existing entry's `note`
  rather than adding a second row. (A Rust project where all five are `cargo …`
  yields exactly **one** Tier-0 row.)
- **Tier 1:** always emit all three enhancer rows in the order Rust, TypeScript,
  Python, each resolved against `search_paths`, each `note` naming the language
  and the install remedy from the table above. `doctor` does **not** detect which
  languages the project uses — it reports all three advisory, and the human reads
  the ones that matter. (Pin the *behavior* — three rows, correct `found`, remedy
  present in the note — not the exact note string.)

**1e. PATH accessor + `run` entry (the only impure code; thin plumbing, no test
required per §3.2):**

```rust
/// The PATH search directories, or empty if PATH is unset.
fn path_dirs() -> Vec<PathBuf> {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default()
}

/// Build the report against the real PATH, print it (human or JSON),
/// and return whether all required tools were found.
pub fn run(commands: &CommandConfig, json: bool) -> bool {
    let report = build_report(commands, &path_dirs());
    if json {
        // serde_json::to_string_pretty(&report), same unwrap_or_else
        // fallback shape the other arms use
    } else {
        println!("{}", format_report(&report));
    }
    report.tier0_ok()
}
```

**1f. Human renderer `format_report(&DoctorReport) -> String`:** a readable
report with a Tier-0 section and a Tier-1 section, each row showing found/missing
+ the binary + its note, and a closing line. **Pin behavior, not exact layout:**
the rendered string must (a) contain each binary name, (b) clearly mark missing
tools distinctly from present ones (e.g. an `ok`/`MISSING` marker — your choice of
exact glyph/word), and (c) when a Tier-0 tool is missing, include a line telling
the human a required tool is missing. Do not over-engineer the formatting.

### Task 2 — `Commands::Doctor` variant — `main.rs`

Add a clap variant mirroring `Health`'s arg shape (config-only, plus `--json`):

```rust
/// Report whether the configured toolchain + verifier enhancers are on PATH
Doctor {
    /// Path to the config file
    #[arg(long)]
    config: Option<PathBuf>,

    /// Emit the report as JSON instead of a human summary
    #[arg(long)]
    json: bool,
},
```

### Task 3 — dispatch arm — `main.rs`

```rust
Commands::Doctor { config, json } => {
    let config_path = config.unwrap_or_else(|| PathBuf::from("rexymcp.toml"));
    let cfg = Config::load_with_env(&config_path)?;
    let ok = doctor::run(&cfg.commands, json);
    if ok {
        Ok(())
    } else {
        std::process::exit(1);
    }
}
```

### Task 4 — `mod doctor;` declaration — `main.rs`

Add `mod doctor;` to the module-declaration block at the top of `main.rs`
(alphabetical with its siblings: it sorts between `dashboard` and `init`).

### Step — verify

```bash
cargo fmt --all --check
cargo build 2>&1 | tail -5
cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail -5
cargo test -p rexymcp 2>&1 | tail -3
```

## Acceptance criteria

- [ ] `command_binary` returns the first token (`"cargo +nightly fmt"` → `"cargo"`)
  and `None` for a blank string.
- [ ] `resolve_binary` finds a bare name as a *file* in a search dir, treats a
  separator-bearing name as a literal path, matches **exactly** (no substring),
  and does **not** match a directory of that name.
- [ ] `build_report` produces one Tier-0 row per distinct configured binary
  (dedup, roles merged into the note), skips `None` commands, and always emits the
  three Tier-1 enhancer rows with correct `found` flags and remedies in their notes.
- [ ] `DoctorReport::tier0_ok()` is true iff all Tier-0 tools are found, and is
  **unaffected** by Tier-1 status (a missing `tsc`/`ruff` leaves it `true` when the
  command set is present).
- [ ] `rexymcp doctor` exits `0` when all Tier-0 tools are present and **non-zero**
  when a Tier-0 tool is missing; Tier-1 absence never changes the exit code.
- [ ] `cargo build` succeeds with zero warnings; `clippy` passes; `cargo test -p
  rexymcp` passes (existing + new); `cargo fmt --all --check` passes.

## Test plan

Unit tests in `mcp/src/doctor.rs`'s `#[cfg(test)] mod tests` (hermetic — build a
`TempDir`, `touch` fake binary files / `create_dir` fake dirs inside it, pass the
TempDir path(s) as `search_paths`; **never** read or mutate the real PATH):

- `command_binary_returns_first_token` — `"cargo +nightly fmt --all"` → `Some("cargo")`.
- `command_binary_none_for_blank` — `"   "` and `""` → `None`. (negative case)
- `resolve_binary_finds_file_in_search_dir` — a `TempDir` containing a file
  `cargo`; `resolve_binary("cargo", &[dir])` → `Some(_)`.
- `resolve_binary_rejects_directory_of_same_name` — a `TempDir` containing a
  **directory** named `cargo`; resolve → `None`. **Pinned negative case.**
- `resolve_binary_is_exact_not_substring` — a `TempDir` containing only
  `cargo-clippy`; `resolve_binary("cargo", …)` → `None`. **Pinned negative case.**
- `resolve_binary_absolute_path_checked_directly` — pass the full path of a file
  that exists inside the TempDir (a name containing the separator); resolve → `Some(_)`
  even with an empty `search_paths`.
- `build_report_dedupes_tier0_by_binary` — a `CommandConfig` where `format`,
  `build`, `lint`, `test` are all `cargo …`; assert `tier0.len() == 1`.
- `build_report_skips_unset_commands` — only `build` set (rest `None`); assert
  exactly one Tier-0 row for that binary.
- `build_report_emits_three_tier1_rows` — assert `tier1.len() == 3` and the
  binaries are `cargo`/`tsc`/`ruff`, regardless of config.
- `tier0_ok_true_when_all_present_ignoring_tier1` — construct a report (via
  `build_report` against a TempDir holding the Tier-0 binary but **not** the
  Tier-1 ones) and assert `tier0_ok()` is `true`. **Pins the fail-open property:
  missing enhancers don't fail the gate.**
- `tier0_ok_false_when_a_required_tool_missing` — a Tier-0 binary that isn't in
  the search dir → `tier0_ok()` is `false`.

A clap-parse test in `main.rs`'s `mod tests` mirroring the existing
`cli_parse_*` tests:

- `cli_parse_doctor_with_config_and_json` — asserts `--config`/`--json` parse into
  the `Doctor` variant; and the no-arg form leaves `config: None`, `json: false`.

## End-to-end verification

`doctor` is a real binary entrypoint, so verify against it (quote actual output in
the completion Update Log):

1. **All Tier-0 present (this repo, Rust toolchain installed):**
   `cargo run -p rexymcp -- doctor --config rexymcp.toml` — expect the Tier-0
   `cargo` row marked present, the three Tier-1 rows (`cargo` present; `tsc`/`ruff`
   likely missing on this host — that's fine, advisory), and **exit code 0**
   (`echo $?` → `0`). Quote the output and the exit code.
2. **A missing Tier-0 tool forces exit 1:** write a throwaway config in a
   `TempDir` (or `/tmp`) whose `[commands] build` points at a guaranteed-absent
   binary, e.g. `build = "definitely-not-a-real-binary-xyz build"`, and run
   `cargo run -p rexymcp -- doctor --config <that-file>; echo $?` — expect the
   `definitely-not-a-real-binary-xyz` row marked MISSING and **exit code 1**.
   Quote both. (Do not commit the throwaway config.)
3. **`--json` shape:** `cargo run -p rexymcp -- doctor --config rexymcp.toml --json`
   — confirm it emits parseable JSON with `tier0`/`tier1` arrays. Quote a snippet.

## Authorizations

None. No new dependency (PATH resolution uses only `std::env::var_os`,
`std::env::split_paths`, and `Path::is_file`); no `unsafe`; no edit to
`Cargo.toml`, the architecture doc, `rexymcp init`'s template, or any other phase
doc. `doctor.rs` is the one new file the command requires.

## Out of scope

- **Do not add language detection.** `doctor` reports all three Tier-1 enhancers
  unconditionally; it does not inspect the repo for `Cargo.toml`/`tsconfig.json`/
  `pyproject.toml` to decide which apply. (A future phase may, if the data asks
  for it — not here.)
- **Do not touch `rexymcp init`.** Detection never lives in the scaffolder
  (architecture rule). The init template already documents `[commands]`; leave it.
- **Do not change the verifier, the `Skipped` variant, or phase-01's runtime
  advisory.** This is the human-present reporting half; the runtime half is done.
- **Do not version-check binaries** (parsing `--version` output). Presence on PATH
  is the whole scope; minimum-version enforcement is not in this phase.
- **Do not wire `doctor` into the architect bootstrap skill or any MCP tool.** The
  CLI command is the deliverable; the bootstrap already references "run `rexymcp
  doctor` once it exists" (WORKFLOW § bootstrap checklist) and needs no code here.
- **Do not add a `SessionEvent` or dashboard surface** for doctor output.

## Notes for executor

- **Why a new variant rather than folding into `health`:** `health` checks the
  *LLM endpoint* (a network concern); `doctor` checks the *local toolchain* (a
  filesystem/PATH concern). They answer different "is this ready?" questions for
  different audiences and must not be conflated.
- **Keep the pure/impure split clean:** `command_binary`, `resolve_binary`,
  `build_report`, `format_report`, and `tier0_ok` are pure and unit-tested;
  `path_dirs` and `run` are the only code that touches the real environment / does
  I/O, and they're thin plumbing (§3.2 — no test required, exercised by the E2E).
  Inject `search_paths` into `build_report`/`resolve_binary` so tests never depend
  on the host's real PATH.
- **The fail-open property is the whole point of the tier split:** `tier0_ok()`
  must ignore Tier-1 entirely. The pinned test
  `tier0_ok_true_when_all_present_ignoring_tier1` guards it — do not let a missing
  `tsc`/`ruff` bleed into the exit code.
- **Reuse phase-01's install-hint wording verbatim** for the three enhancers so
  the runtime `Skipped` advisory and `doctor` give the human the same remedy.
- Commit as a single `feat:` commit; the body explains *why* (the human needs a
  pre-dispatch readiness check that distinguishes required Tier-0 tools from
  fail-open Tier-1 enhancers), not *what*.

## Update Log

<!-- entries appended below this line -->

### Update — 2026-06-09 00:00 (complete)

**By:** executor

**Summary:** Implemented `rexymcp doctor` — toolchain-availability command.

**Files changed:**
- `mcp/src/doctor.rs` — new module: `command_binary`, `resolve_binary`, `build_report`, `DoctorReport`, `ToolStatus`, `format_report`, `run`, plus 18 unit tests.
- `mcp/src/main.rs` — added `mod doctor`, `Commands::Doctor` variant, dispatch arm, and `cli_parse_doctor_with_config_and_json` test.
- `docs/dev/milestones/M12-executor-tooling/phase-02-doctor.md` — status `todo` → `review`, Update Log entry.
- `docs/dev/milestones/M12-executor-tooling/README.md` — phase table row `todo` → `review`.

**Verification commands (all clean):**
- `cargo fmt --all --check` — pass
- `cargo build` — zero warnings
- `cargo clippy --all-targets --all-features -- -D warnings` — pass
- `cargo test -p rexymcp` — 289 passed (was 270, +19 new tests)

**End-to-end verification:**
1. All Tier-0 present: `cargo run -p rexymcp -- doctor --config rexymcp.toml` → exit 0, cargo row marked `ok`, three Tier-1 rows (cargo ok, tsc MISSING, ruff ok), "All required tools are present."
2. Missing Tier-0: throwaway config with `build = "definitely-not-a-real-binary-xyz build"` → exit 1, binary marked `MISSING`, "A required tool is missing" warning present.
3. `--json` shape: `cargo run -p rexymcp -- doctor --config rexymcp.toml --json` → parseable JSON with `tier0`/`tier1` arrays, each containing `binary`/`found`/`note`.

**Notes for review:**
- Tier-1 install-hint wording matches phase-01's `spawn_failure` strings verbatim.
- `resolve_binary` uses `is_file()` which inherently rejects directories and does exact matching (no substring matches).
- Tier-0 dedup merges roles into the `note` field (e.g. "format, build, lint, test" for a Rust project where all commands use `cargo`).

### Review verdict — 2026-06-09

- **Verdict:** approved_first_try
- **Bounces:** none
- **Executor:** Qwen/Qwen3.6-27B-FP8
- **Scope deviations:** none — additive throughout (one new module, one clap
  variant, one dispatch arm, one `mod` declaration); no existing type changed
  shape, no new dependency, no `unsafe`, no `#[allow]`/`#[ignore]`.
- **Calibration:** none. Clean 58-turn first-try with full bookkeeping (status
  flip + Update Log + single `feat:` commit `45a4c6f` all landed by the
  executor). All four gates green on independent re-run (fmt clean, build
  zero-warnings, clippy clean, **289 passed** mcp = 270 baseline + 19 new). All
  three E2E cases reproduced at review against the real binary: all-Tier-0
  present → exit 0 (`cargo` row, 4 roles merged); a `definitely-not-a-real-binary-xyz`
  Tier-0 binary → `MISSING` + warning + exit 1; `--json` → parseable
  `tier0`/`tier1` arrays. The fail-open property (`tier0_ok()` ignores Tier 1)
  is pinned by `tier0_ok_true_when_all_present_ignoring_tier1`; the two pinned
  negatives (`resolve_binary_rejects_directory_of_same_name`,
  `resolve_binary_is_exact_not_substring`) were spot-checked as
  mutation-resistant (`.exists()`-for-`.is_file()` and substring-match mutations
  each fail them). Cosmetic-only quirk (no fold): the completion Update Log
  self-stamps `00:00` and "executor" — the recurring local-LLM clock/identity
  quirk that phase-06's datetime injection fixes once `rexymcp serve` is
  restarted (still pending).
