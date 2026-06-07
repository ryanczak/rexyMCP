# Phase 01: recoverable output filter for bash command output

**Milestone:** M10 — Context optimization
**Status:** todo
**Depends on:** none (first phase of M10)
**Estimated diff:** ~250 lines
**Tags:** language=rust, kind=feature, size=m

## Goal

Today the `bash` tool truncates oversized command output **losslessly with no
recovery** — it keeps head+tail and drops the middle with the marker
`full output not retained in this run`. The dropped content is gone; if the
executor needs it, it must re-run the command. This phase makes that truncation
**recoverable** and **normalized**: strip ANSI escapes, collapse consecutive
duplicate lines, and when output is still too long, write the *full normalized*
output to a rotated recovery file under the already-git-ignored `.rexymcp/output/`
and point the elision marker at it — a file the model can re-read with `read_file`.
A config kill-switch (`[context] output_filter`, default on) restores the old
behavior. This is Arc A's foundation (boundary output filtering); later phases
add structured per-toolchain filters on top of the same module.

## Architecture references

Read before starting:

- `docs/dev/milestones/M10-context-optimization/README.md` — the milestone design,
  especially "What we take from RTK" (diagnostic-level losslessness, list-level
  compression, the recovery "tee" file) and the Arc A / Arc B split. This phase is
  Arc A, phase 1.
- `docs/architecture.md#the-executor-turn-cycle` — step 5 (tool dispatch). The
  `bash` tool's output becomes the model-facing message; this phase shapes that
  output. No change to verifier pass/fail (step 6) or the final command set.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes
   (`git status`).
5. Run `cargo test` and record the passing count — the completion log must show
   the same count plus the new tests.

## Current state

**The bash tool already truncates** — `executor/src/tools/bash.rs:220-246`:

```rust
fn truncate_output(body: &str) -> (String, bool) {
    let lines: Vec<&str> = body.lines().collect();
    let total = lines.len();

    if total <= 100 {
        return (body.to_string(), false);
    }

    let head_count = 20;
    let tail_count = 80;
    let omitted = total - head_count - tail_count;

    let mut result = String::new();
    for line in &lines[..head_count] {
        result.push_str(line);
        result.push('\n');
    }
    result.push_str(&format!(
        "[… {omitted} lines omitted; full output not retained in this run …]\n"
    ));
    for line in &lines[total - tail_count..] {
        result.push_str(line);
        result.push('\n');
    }

    (result, true)
}
```

It is called once, in `execute` (`bash.rs:159`):

```rust
let (body, truncated) = truncate_output(&combined);
```

The `Bash` struct and its constructor (`bash.rs:31-34`, `252-257`):

```rust
pub struct Bash {
    scope: Scope,
    default_timeout_secs: u32,
}

pub fn bash(scope: Scope, default_timeout_secs: u32) -> Arc<dyn Tool> {
    Arc::new(Bash {
        scope,
        default_timeout_secs,
    })
}
```

`self.scope.root()` (used at `bash.rs:110`) is the target-repo root — the place
`.rexymcp/output/` lives. `/.rexymcp` is already in `.gitignore` (line 2), so the
recovery directory is git-ignored for free.

**`bash(scope, timeout)` has ~10 call sites** (one real in `mcp/src/runner.rs:128`,
the rest in `executor/src/parser/**` tests). **Do not change its signature** — add
a sibling constructor instead (see Spec task 3), so those call sites stay untouched.

The `context` module (`executor/src/context/mod.rs`) currently has three submodules:

```rust
pub mod budget;
pub mod compactor;
pub mod tokens;
```

The config pattern for a defaulted sub-section — mirror this exactly
(`executor/src/config.rs:8-24`, `DashboardConfig`):

```rust
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct DashboardConfig {
    pub saved_input_per_mtok: f64,
    pub saved_output_per_mtok: f64,
}

impl Default for DashboardConfig {
    fn default() -> Self {
        Self {
            saved_input_per_mtok: 0.0,
            saved_output_per_mtok: 0.0,
        }
    }
}
```

`Config` itself (`config.rs:26-34`):

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub executor: ExecutorConfig,
    pub commands: CommandConfig,
    pub budget: BudgetConfig,
    pub telemetry: TelemetryConfig,
    pub dashboard: DashboardConfig,
}
```

`build_registry` (`mcp/src/runner.rs:115`) constructs the bash tool and has exactly
**two** call sites — `runner.rs:160` (real) and `runner.rs:374` (a test). Line 128
is `tools::bash(scope.clone(), bash_timeout_secs)`. The function at line 160 has
`inp.cfg` in scope (it reads `inp.cfg.budget.*` a few lines below).

`regex` is already an executor dependency (`executor/Cargo.toml:21`,
`regex.workspace = true`) — use it for ANSI stripping; **no `Cargo.toml` change.**

## Spec

Numbered tasks in execution order.

### 1. New module `executor/src/context/output_filter.rs`

Create the module. It owns normalization + recoverable truncation. The reference
implementation below is the intended shape — match its **behavior** (pinned by the
tests in the Test plan); minor structural choices are yours.

```rust
//! Boundary output filtering (M10, Arc A). Normalizes a command's raw output
//! (strip ANSI, collapse consecutive duplicate lines) and, when it is still too
//! long, truncates to head+tail while writing the full normalized output to a
//! rotated recovery file the model can re-read with `read_file`.
//!
//! Diagnostic-preserving by construction: the tail (where errors and panics
//! live) is always kept verbatim, and nothing is discarded without a recovery
//! file to recover it from.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use regex::Regex;
use std::sync::OnceLock;

/// Output longer than this (in lines, post-normalization) is truncated. Matches
/// the bash tool's prior threshold so behavior is familiar.
const LINE_CAP: usize = 100;
const HEAD_LINES: usize = 20;
const TAIL_LINES: usize = 80;
/// Recovery files retained per directory before the oldest are pruned.
const MAX_RECOVERY_FILES: usize = 20;
/// Subdirectory under the repo root where recovery files are written.
const RECOVERY_SUBDIR: &str = ".rexymcp/output";

/// Monotonic recovery-file sequence (process-global). Filenames sort by age.
static RECOVERY_SEQ: AtomicU64 = AtomicU64::new(0);

fn ansi_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\x1b\[[0-9;]*[A-Za-z]").expect("valid ANSI regex"))
}

/// Strip ANSI escape sequences, then collapse runs of >= 2 identical consecutive
/// lines into one line plus an ` (xN)` count. Lossless for diagnostics: only
/// color codes and exact consecutive repetition are removed. Pure.
pub fn normalize(raw: &str) -> String {
    let stripped = ansi_re().replace_all(raw, "");
    let mut out = String::new();
    let mut prev: Option<&str> = None;
    let mut run = 0usize;
    let flush = |out: &mut String, line: &str, run: usize| {
        if run >= 2 {
            out.push_str(&format!("{line} (x{run})\n"));
        } else {
            out.push_str(line);
            out.push('\n');
        }
    };
    for line in stripped.lines() {
        match prev {
            Some(p) if p == line => run += 1,
            Some(p) => {
                flush(&mut out, p, run);
                run = 1;
            }
            None => run = 1,
        }
        prev = Some(line);
    }
    if let Some(p) = prev {
        flush(&mut out, p, run);
    }
    out
}

/// Normalize `raw`; if the normalized result still exceeds `LINE_CAP` lines,
/// write the full normalized text to a rotated recovery file under
/// `<project_root>/.rexymcp/output/` and return a head+tail view whose elision
/// marker points at that file (a root-relative path the model can `read_file`).
/// Returns `(body, truncated)`.
///
/// Best-effort recovery: if the recovery write fails, the marker falls back to
/// "full output not retained" and the tool still returns truncated output — a
/// failed write is never an error to the caller.
pub fn compact_with_recovery(raw: &str, project_root: &Path) -> (String, bool) {
    let normalized = normalize(raw);
    let lines: Vec<&str> = normalized.lines().collect();
    if lines.len() <= LINE_CAP {
        return (normalized, false);
    }
    let total = lines.len();
    let omitted = total - HEAD_LINES - TAIL_LINES;

    let marker = match write_recovery(&normalized, project_root) {
        Some(rel) => format!("[… {omitted} lines omitted — full output: {rel} …]"),
        None => format!("[… {omitted} lines omitted; full output not retained …]"),
    };

    let mut result = String::new();
    for line in &lines[..HEAD_LINES] {
        result.push_str(line);
        result.push('\n');
    }
    result.push_str(&marker);
    result.push('\n');
    for line in &lines[total - TAIL_LINES..] {
        result.push_str(line);
        result.push('\n');
    }
    (result, true)
}

/// Write `content` to a fresh recovery file under `<root>/.rexymcp/output/`,
/// prune to `MAX_RECOVERY_FILES`, and return the root-relative path (e.g.
/// `.rexymcp/output/cmd-output-7.log`). `None` on any I/O failure.
fn write_recovery(content: &str, project_root: &Path) -> Option<String> {
    let dir = project_root.join(RECOVERY_SUBDIR);
    std::fs::create_dir_all(&dir).ok()?;
    let seq = RECOVERY_SEQ.fetch_add(1, Ordering::Relaxed);
    let name = format!("cmd-output-{seq}.log");
    std::fs::write(dir.join(&name), content).ok()?;
    prune_recovery(&dir);
    Some(format!("{RECOVERY_SUBDIR}/{name}"))
}

/// Keep at most `MAX_RECOVERY_FILES` `cmd-output-*.log` files, deleting the
/// oldest (lowest sequence). Best-effort.
fn prune_recovery(dir: &Path) {
    let mut files: Vec<_> = match std::fs::read_dir(dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("cmd-output-") && n.ends_with(".log"))
            })
            .collect(),
        Err(_) => return,
    };
    if files.len() <= MAX_RECOVERY_FILES {
        return;
    }
    // Sort by the numeric sequence so the oldest prune first (lexical sort would
    // put cmd-output-10 before cmd-output-2).
    files.sort_by_key(|p| {
        p.file_stem()
            .and_then(|s| s.to_str())
            .and_then(|s| s.rsplit('-').next())
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0)
    });
    let excess = files.len() - MAX_RECOVERY_FILES;
    for p in files.into_iter().take(excess) {
        let _ = std::fs::remove_file(p);
    }
}
```

Register the module in `executor/src/context/mod.rs`:

```rust
pub mod budget;
pub mod compactor;
pub mod output_filter;
pub mod tokens;
```

### 2. Wire the filter into the bash tool

In `executor/src/tools/bash.rs`:

- Add a `filter: bool` field to `struct Bash`.
- In `execute`, replace the single `truncate_output` call (`bash.rs:159`) with a
  branch on `self.filter`:

  ```rust
  let (body, truncated) = if self.filter {
      crate::context::output_filter::compact_with_recovery(&combined, self.scope.root())
  } else {
      truncate_output(&combined)
  };
  ```

- **Keep** the existing `truncate_output` function unchanged — it is the
  kill-switch (`filter == false`) path.

### 3. Add an additive constructor (do NOT change `bash`)

In `executor/src/tools/bash.rs`, keep `pub fn bash` working for its ~10 existing
call sites by delegating, and add a sibling that takes the flag:

```rust
pub fn bash(scope: Scope, default_timeout_secs: u32) -> Arc<dyn Tool> {
    bash_with_filter(scope, default_timeout_secs, true)
}

pub fn bash_with_filter(scope: Scope, default_timeout_secs: u32, filter: bool) -> Arc<dyn Tool> {
    Arc::new(Bash {
        scope,
        default_timeout_secs,
        filter,
    })
}
```

Export `bash_with_filter` from `executor/src/tools/mod.rs` **the same way `bash` is
exported** (find the existing `pub use`/`pub fn` re-export of `bash` and mirror it).

### 4. Config kill-switch

In `executor/src/config.rs`, add a `ContextConfig` section mirroring
`DashboardConfig` (quoted in Current state), defaulting `output_filter = true`:

```rust
/// Context-optimization settings (M10). `output_filter` is the kill-switch for
/// boundary output filtering — default on; set false to restore raw head+tail
/// truncation with no recovery file.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct ContextConfig {
    pub output_filter: bool,
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            output_filter: true,
        }
    }
}
```

Add the field to `Config`:

```rust
pub struct Config {
    pub executor: ExecutorConfig,
    pub commands: CommandConfig,
    pub budget: BudgetConfig,
    pub telemetry: TelemetryConfig,
    pub dashboard: DashboardConfig,
    pub context: ContextConfig,
}
```

### 5. Thread the flag through `build_registry` (exactly 2 call sites)

In `mcp/src/runner.rs`:

- Add a `filter_output: bool` parameter to `build_registry` (`runner.rs:115`).
- Change line 128 from `tools::bash(scope.clone(), bash_timeout_secs)` to
  `tools::bash_with_filter(scope.clone(), bash_timeout_secs, filter_output)`.
- Update the real caller at `runner.rs:160`:
  `build_registry(&scope, 30, inp.cfg.context.output_filter)`.
- Update the test caller at `runner.rs:374`:
  `build_registry(&scope, 30, true)`.

Those are the only two `build_registry(` call sites — verified by
`grep -rn 'build_registry(' mcp/src/runner.rs`. Re-grep before finishing to
confirm none were missed.

## Acceptance criteria

- [ ] `grep -n 'pub mod output_filter' executor/src/context/mod.rs` matches.
- [ ] `normalize` strips ANSI color codes and collapses consecutive duplicate
      lines with an `(xN)` count; non-consecutive duplicates are left intact.
- [ ] With filtering on, a bash command emitting > 100 lines produces a recovery
      file under `<root>/.rexymcp/output/cmd-output-*.log` and an elision marker
      whose text contains `full output: .rexymcp/output/cmd-output-`.
- [ ] The last (tail) lines of long output — where errors/panics appear — are
      preserved verbatim in the returned body.
- [ ] With `output_filter = false`, bash uses the legacy `truncate_output`: marker
      reads `full output not retained` and **no** recovery file is written.
- [ ] `ContextConfig::default().output_filter` is `true`; a `[context]` section
      with `output_filter = false` parses to `false`; a config with no `[context]`
      section still loads and defaults to `true`.
- [ ] `bash(scope, timeout)` signature is unchanged; the ~10 existing call sites in
      `executor/src/parser/**` compile without edits.
- [ ] `cargo build`, `cargo clippy --all-targets --all-features -- -D warnings`,
      `cargo fmt --all --check`, and `cargo test` all pass; test count is the
      pre-flight count plus the new tests.

## Test plan

Unit tests in `executor/src/context/output_filter.rs` (`#[cfg(test)] mod tests`),
using `tempfile::TempDir` for the recovery-file cases:

- `normalize_strips_ansi_escape_sequences` — input with `\x1b[31m…\x1b[0m` → no
  escape bytes remain, text content intact.
- `normalize_collapses_consecutive_duplicate_lines` — three identical consecutive
  lines → one line ending `(x3)`.
- `normalize_does_not_collapse_nonconsecutive_duplicates` — `a\nb\na` stays three
  lines, no `(x` count. **(negative case)**
- `normalize_leaves_distinct_lines_verbatim` — distinct lines pass through
  unchanged, no spurious counts.
- `compact_returns_short_output_unchanged` — ≤ 100 lines → `truncated == false`,
  body equals the normalized input, and **no** `.rexymcp` directory is created in
  the TempDir. **(negative case: no recovery file when not needed)**
- `compact_truncates_long_output_keeping_head_and_tail` — 200 distinct lines →
  `truncated == true`, body contains the first and last lines and the `omitted`
  marker.
- `compact_writes_recovery_file_referenced_by_marker` — long output → a
  `cmd-output-*.log` exists under `<tempdir>/.rexymcp/output/`, its contents equal
  the full normalized output, and the body marker contains that file's
  root-relative path.
- `compact_preserves_trailing_diagnostic_line` — build a long input whose final
  line is `error[E0425]: cannot find value` → that exact line survives verbatim in
  the returned body. **(diagnostic-preservation guarantee)**
- `dedupe_can_drop_long_output_below_truncation_threshold` — ~150 lines that are
  mostly consecutive duplicates collapsing to < 100 → `truncated == false`.
- `recovery_rotation_keeps_at_most_max_files` — force > `MAX_RECOVERY_FILES`
  recovery writes into one dir → at most `MAX_RECOVERY_FILES` `cmd-output-*.log`
  remain and the highest-sequence (newest) file is among the survivors.

Bash-tool tests in `executor/src/tools/bash.rs`:

- `filtered_bash_truncation_writes_recovery_file` — `bash_with_filter(scope, 30,
  true)` running a >100-line command → output marker references
  `.rexymcp/output/`, and a `cmd-output-*.log` exists under the scope root.
- `kill_switch_off_uses_legacy_truncation_without_recovery` —
  `bash_with_filter(scope, 30, false)` running the same command → marker reads
  `full output not retained`, and **no** `.rexymcp/output/` directory exists.
  **(negative case)**

Config tests in `executor/src/config.rs`:

- `context_config_defaults_output_filter_on` — `ContextConfig::default()
  .output_filter` is `true`, and a `Config` parsed from TOML with no `[context]`
  section has `context.output_filter == true`.
- `context_output_filter_can_be_disabled` — TOML `[context]\noutput_filter = false`
  parses to `false`.

(Add structural tests as needed; the names above pin the behaviors that matter.)

## End-to-end verification

The bash-tool tests are **not** hermetic fakes — they spawn real `sh` subprocesses
and write real recovery files to a real `TempDir` filesystem, so they exercise the
shipped artifact directly. For E2E:

1. Run `cargo test filtered_bash_truncation_writes_recovery_file -- --nocapture`
   and quote, in the completion log, the actual elision marker line produced and
   the actual `cmd-output-*.log` path created.
2. Confirm the new config section loads on the real binary: append
   `\n[context]\noutput_filter = false\n` to a scratch `rexymcp.toml` copy and run
   `cargo run -p rexymcp -- health --config <that file>` — it must start without a
   config-parse error. Quote the command and its exit status.

## Authorizations

None. `regex` is already an executor dependency (`executor/Cargo.toml:21`); no
`Cargo.toml` change. No `docs/architecture.md` change. No new external dependency.

## Out of scope

- **Structured per-toolchain filters** (cargo test/build/clippy failures-only,
  diagnostic grouping) — that is phase-02/03. This phase ships only the generic
  normalize + recoverable-truncate pipeline.
- **Arc B context-lifecycle work** (superseded-read eviction, re-read dedupe,
  value-ranked compaction) — later phases; do not touch `compactor.rs`, the
  message history, or the working set.
- **Filtering tools other than `bash`** — `read_file` already has its own cap;
  `write_file`/`patch` return short confirmations; `search`/`symbols`/`find_files`
  have bounded structured output. Only `bash` output is filtered here.
- **Changing the truncation thresholds** (`LINE_CAP` 100 / `HEAD_LINES` 20 /
  `TAIL_LINES` 80) — keep the bash tool's prior values.
- **Changing hard-fail thresholds** (`RUNAWAY_OUTPUT_BYTES`,
  `IDENTICAL_CALL_THRESHOLD`) in `governor/hard_fail.rs`.
- **`PhaseRun` context-efficiency metrics** — phase-06.
- Do **not** change `pub fn bash`'s signature (its ~10 call sites must stay
  untouched) — add `bash_with_filter` instead.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
