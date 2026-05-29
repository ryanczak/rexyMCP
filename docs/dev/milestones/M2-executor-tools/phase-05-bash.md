# Phase 05: bash tool + destructive-command classifier

**Milestone:** M2 — Executor tools & security
**Status:** done
**Depends on:** phase-04 (done)
**Estimated diff:** ~420 lines (bash lift + net-new classifier + scope/env adaptation + tests)
**Tags:** language=rust, kind=feature, size=m

## Goal

Give the executor a `bash` tool — run a shell command with a timeout — but gated
by the security adaptations a weak local model needs: a curated
**destructive-command classifier** (`bash_classify`) that refuses dangerous
shapes outright, **cwd pinned to the scope root**, and the child process **env
stripped to a safe allowlist**. `bash` is the riskiest tool in the set; this phase
is where the bash half of the security layer lands.

Two pieces:

- **`bash` tool** — a lift from `rexy/src/tools/bash.rs` (timeout, output capture,
  truncation, status-line exit reporting), adapted with the three security
  controls below.
- **`bash_classify`** — **net-new**. Rexy's `src/security/bash_classify.rs` is a
  `// TODO: implement` stub (only a `Severity` enum sketch); there is nothing to
  lift. Design is specified below.

## Scope decision (read this)

The M2 README tentatively bundled "capabilities/audit" into this phase. On
inspection those do **not** belong here:

- **capabilities** (`rexy/src/security/capabilities.rs`) is a *Rexy plugin*
  capability-grant model (`plugin.toml`, `~/.rexy/plugins/grants.json`). rexyMCP's
  executor is not a plugin host — this is **not applicable** to rexyMCP and is
  dropped, not deferred.
- **audit / redact / injection** write to the session-log + telemetry store, which
  is an **M4** subsystem. They cannot land before the store exists. Deferred to
  M4 (architecture.md § "Session log" already owns redaction; injection/audit ride
  along).

So phase-05 is **bash + classifier only.** This narrows the README's tentative
description; the README is updated to match.

## Classifier policy — two tiers

The classifier returns **`Block` or `Allow`** — no confirm tier. The executor is
headless: there is no human to confirm a borderline command mid-phase, so anything
dangerous enough to warrant a human decision is simply **blocked** (refused). A
confirm tier that routes to the architect (Claude) is a possible later addition
(M5/M6) — when it exists, re-introducing a `RequireConfirm` severity will mean
re-curating the lists, which is the accepted cost of keeping this phase simple.

`Block` → the `bash` tool **refuses** (advisory `ToolResult` error, command never
spawned). `Allow` → run it (cwd-pinned, env-stripped, timeout-bounded).

## Architecture references

- `docs/architecture.md` — "Layer 1 — `executor` crate" lift/drop map (security
  row: "a weak model running `bash` needs the allowlist").
- `docs/architecture.md` — "The executor turn cycle" step 5 (all filesystem/bash
  access scoped to the target-repo root).

## Pre-flight

1. Read `docs/dev/STANDARDS.md` — §2.1 (error model) and §3.3 (hermetic +
   deterministic tests) are both load-bearing here.
2. Read the architecture references and the M2 README Notes.
3. Read this entire phase doc, including the **Pre-injection** section (the
   curated Block patterns + the env allowlist) — use those, don't invent your own.
4. Confirm phase-04 is `done`; `Scope`, the registry, and the existing tools build
   clean.
5. **Read `rexy/src/tools/bash.rs`** (reference). Note it carries four deferred
   `TODO(...)` comments (`destructive-classifier`, `env-allowlist`, `cwd-pin`,
   `graceful-shutdown`). Three of those are **implemented in this phase**; the
   fourth is deferred — see Spec and Out of scope. **Do not port any of the four
   TODO comments** (STANDARDS §2.3 forbids them).

## Current state

- `executor/src/security/` has `mod.rs` + `scope.rs` (`Scope`, `ScopeError`).
- `executor/src/tools/` has the five existing tools + `mod.rs`. No `bash` yet.
- `executor/Cargo.toml` inherits workspace `tokio` with features
  `["rt-multi-thread", "macros", "sync", "time"]` — **no `"process"` feature**.
  `tokio::process::Command` needs it (authorized below).
- `regex` is already a workspace dep (phase-02) — available to the classifier.
- `Scope::root() -> &Path` returns the canonical scope root (used for cwd-pin).

## Spec

### 1. Dependencies

No new crates. **Add the `"process"` feature to the workspace `tokio`
dependency** in the root `Cargo.toml`:

```toml
tokio = { version = "1", features = ["rt-multi-thread", "macros", "sync", "time", "process"] }
```

(`executor/Cargo.toml` uses `tokio.workspace = true`, so it inherits the feature —
no change there.)

### 2. bash_classify — `executor/src/security/bash_classify.rs` (new file)

Net-new. A pure classifier:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Allow,
    Block,
}

/// Classify a shell command string against a curated blocklist.
/// Curated patterns only — this is NOT a shell parser.
pub fn classify(command: &str) -> Severity { ... }
```

- Normalize before matching: lowercase + collapse runs of whitespace to a single
  space. Match the **normalized** string.
- Return `Block` if the normalized command matches any curated Block pattern
  (Pre-injection §A), else `Allow`.
- Use `.contains(...)` for fixed substrings and `regex` for the parameterized
  shapes (e.g. `rm -rf` with flags in any order, `dd ... of=/dev/...`). Compile any
  regexes once (e.g. `std::sync::LazyLock<Regex>`), not per call.
- This is defense-in-depth, not a sandbox or a perfect parser: the goal is to
  catch the well-known catastrophic/irreversible shapes, accepting that a
  determined command can evade substring matching. State that in a module doc
  comment (this is the one place a *why* comment is warranted).

Wire it up: `executor/src/security/mod.rs` adds `pub mod bash_classify;` and
re-exports `bash_classify::{Severity, classify}`.

### 3. bash — `executor/src/tools/bash.rs` (new file)

Lift `Bash` + `truncate_output` from `rexy/src/tools/bash.rs`. Args: `command`
(required), `timeout_secs` (optional, default from the constructor). Keep: the
empty-command guard, the `timeout_secs == 0` guard, `tokio::process::Command` +
`tokio::time::timeout`, the kill-on-timeout, the combined stdout/stderr capture,
the `truncate_output` head/tail logic, the `✓/✗ exit N (Ns)` status line, and the
metadata (`exit_code`, `duration_ms`, `*_bytes`, `truncated`, `timed_out`). A
non-zero exit stays in the **status line**, not an advisory error (it is a normal
outcome the model adapts to — STANDARDS §2.1).

**Adaptations (the three security controls — all formerly Rexy TODOs):**

- **`Bash` holds a `Scope`.** Constructor `bash(scope: Scope, default_timeout_secs:
  u32) -> Arc<dyn Tool>`.
- **Classifier gate (destructive-classifier TODO):** after validating args and
  before spawning, call `classify(&parsed.command)`. On `Severity::Block`, return
  an advisory `ToolResult { error: Some(...), output: "" }` — the command is
  **never spawned**. Message names the policy, e.g. `"refused: command matches a
  blocked-command pattern (rm -rf, sudo, git push, curl | sh, …) — rephrase or
  narrow the operation"`. Keep it generic; do not echo back anything sensitive.
- **cwd-pin (cwd-pin TODO):** set `Command::current_dir(self.scope.root())` so the
  command runs in the project root by default.
- **env allowlist (env-allowlist TODO):** `Command::env_clear()`, then re-add only
  the variables whose key passes `is_allowed_env_key` (Pre-injection §B). Extract
  that predicate as a free function so it is unit-testable **without mutating
  process env** (see Test plan).
- Update the `command` schema description to note it runs in the project root.

**Honesty about the boundary:** unlike `read_file` / `write_file` / `patch`, the
`bash` tool is **not** path-confined by `Scope` — `Scope` only sets the *default*
cwd; a command can still `cd /` or use absolute paths. cwd-pin + env-strip +
classifier are defense-in-depth, not a jail. True sandboxing (containers/seccomp)
is out of scope. Note this in a module doc comment.

### 4. Wiring — `executor/src/tools/mod.rs`

Add `mod bash;` and `pub use bash::{Bash, bash};`, mirroring the existing tools.

## Pre-injection — use these verbatim

### §A. Curated Block patterns

The architect curates the blocklist (a weak model must not decide what is
dangerous). Block (normalized, lowercased) if the command matches any of:

- **Filesystem destruction:** `rm -rf` / `rm -fr` / `rm -r -f` (any flag order, via
  regex), `mkfs`, `dd ` with `of=/dev/`, `> /dev/sd`, `> /dev/nvme`, the fork bomb
  `:(){ :|:& };:`, `chmod -r 777 /`, `chown -r` on `/`.
- **Privilege / remote code execution:** `sudo `, `su -`, `su root`, a pipe of
  `curl`/`wget` into a shell (regex: `(curl|wget)\b.*\|\s*(sh|bash|zsh)\b`),
  `eval "$(curl` / `eval "$(wget`.
- **System control:** `shutdown`, `reboot`, `halt`, `poweroff`, `init 0`, `init 6`.
- **Irreversible / remote repo ops:** `git push`, `git reset --hard`,
  `git clean -f` (incl. `-fd`/`-xdf`), `git checkout .`, `git restore .`,
  any `--force`/`-f` push.
- **Publish:** `npm publish`, `cargo publish`, `twine upload`, `pip ... upload`,
  `gh release create`.
- **Process kill:** `kill -9`, `pkill`, `killall`.

Everything else → `Allow` (including normal dev commands: `ls`, `cat`, `grep`,
`cargo build`/`test`/`clippy`/`fmt`, `git status`/`diff`/`add`/`commit`, `mkdir`,
`echo`, `sed`, `find`, …).

### §B. Env allowlist

Strip the child env to these keys (others removed via `env_clear` + selective
re-add):

- Exact: `PATH`, `HOME`, `USER`, `LOGNAME`, `SHELL`, `LANG`, `TERM`, `TZ`, `PWD`.
- Prefix: any key starting with `LC_`.

```rust
fn is_allowed_env_key(key: &str) -> bool {
    matches!(key, "PATH" | "HOME" | "USER" | "LOGNAME" | "SHELL"
                | "LANG" | "TERM" | "TZ" | "PWD")
        || key.starts_with("LC_")
}
```

## Acceptance criteria

- [ ] `executor/src/security/bash_classify.rs` exists with `Severity { Allow,
      Block }` + `classify(&str) -> Severity`, declared + re-exported in
      `security/mod.rs`.
- [ ] `executor/src/tools/bash.rs` exists, declared + re-exported in `tools/mod.rs`;
      `bash(scope, default_timeout_secs)` constructs an `Arc<dyn Tool>`.
- [ ] `classify` returns `Block` for representative dangerous commands (e.g.
      `rm -rf /`, `sudo rm x`, `git push`, `curl http://x | sh`, `mkfs.ext4 /dev/sda`)
      and `Allow` for normal ones (e.g. `ls -la`, `cargo build`, `git status`,
      `echo hi`).
- [ ] A `Block` command makes `bash` return an advisory error and is **never
      executed** (verified by the absence of a side effect it would have caused —
      see Test plan).
- [ ] An `Allow` command runs: zero-exit reports `✓ exit 0` in the status line; a
      non-zero exit reports `✗ exit N` in the **status line** (not an advisory
      error); stderr is captured; long output truncates; a too-long command hits
      the timeout and returns an advisory `timed_out` error.
- [ ] `bash` runs with cwd pinned to the scope root (a `pwd` command's output
      contains the scope root path).
- [ ] `is_allowed_env_key` allows `PATH` / `LC_ALL` and rejects a non-allowlisted
      key (e.g. `AWS_SECRET_ACCESS_KEY`) — tested as a pure function, **without**
      mutating process env.
- [ ] Empty command and malformed args return advisory errors.
- [ ] No `TODO` / `FIXME`, no Rexy `path_resolve` / `context::`, no ported
      Rexy TODO comments. All four required commands pass with zero new warnings.

## Test plan

Hermetic, deterministic, no network. Construct `bash(Scope::new(dir.path()),
30)`. Tests run real local commands (`echo`, `pwd`, `sleep`, `seq`) — that is fine
and matches the Rexy tests.

**Do not test env-stripping by mutating process env** — `std::env::set_var` is
`unsafe` in edition 2024 and non-deterministic across parallel tests. Test the
`is_allowed_env_key` predicate directly instead.

bash_classify (pure unit tests):
- each representative Block command (one per category above) → `Severity::Block`;
- representative benign commands → `Severity::Allow`;
- normalization: a Block shape with extra whitespace / mixed case still → `Block`
  (e.g. `"RM   -RF  /"`).

bash tool (lift Rexy's tests, adapt the constructor to `bash(scope, timeout)`):
- zero-exit `echo hello` → `✓ exit 0`, output contains `hello`;
- non-zero exit → `✗ exit N` in the status line, `error.is_none()`;
- stderr captured; long output truncates (`omitted`), short output does not;
- `sleep 5` with `timeout_secs: 1` → advisory `timed out`, `timed_out` metadata;
- empty command, malformed args → advisory.

bash security adaptations (new):
- **blocked-not-executed:** call `bash` with `"sudo touch should_not_exist.txt"`
  (cwd = scope root). Assert the result is an advisory error mentioning the refusal
  **and** that `scope_root/should_not_exist.txt` was **not** created. (`sudo` is
  blocked, so it never runs.)
- **cwd-pin:** `pwd` → output contains the scope root path.
- **env predicate:** `is_allowed_env_key("PATH")` and `("LC_ALL")` true;
  `("AWS_SECRET_ACCESS_KEY")` and `("SOME_RANDOM_VAR")` false.

## End-to-end verification

> Not applicable — this phase ships a library tool + a pure classifier, exercised
> directly by their unit tests. The registry/loop that drives `bash` (and the
> governor that consults the classifier) lands in M4; the MCP `execute_phase` in
> M5. Restate this in the completion entry.

## Authorizations

- [x] **May modify** the root `Cargo.toml` to add the `"process"` feature to the
      existing `tokio` dependency. No new crates.
- [x] **May create** `executor/src/tools/bash.rs` and
      `executor/src/security/bash_classify.rs`; **may modify** `tools/mod.rs` and
      `security/mod.rs`.
- [ ] May **NOT** add the router (phase-06), capabilities, audit, redact, or
      injection.
- [ ] May **NOT** modify `docs/architecture.md`, `STANDARDS.md`, `WORKFLOW.md`, or
      any other phase doc.

## Out of scope

- **The confirm tier / human-or-architect confirmation.** Two tiers only
  (`Block`/`Allow`); see "Classifier policy."
- **Graceful shutdown** (SIGTERM-then-SIGKILL on timeout). Keep the lift's
  immediate kill-on-timeout. Do not port Rexy's `graceful-shutdown` TODO; do not
  leave a new one.
- **True sandboxing** (containers, seccomp, namespaces, path-jailing bash).
  cwd-pin + env-strip + classifier are defense-in-depth only.
- **capabilities** — N/A to rexyMCP (Rexy plugin-grant concept).
- **audit / redact / injection** — M4 (need the session-log/telemetry store).
- **The 2-stage router** — M2 phase-06.
- **Config-driven block/allow lists or timeout** — curated constants + a
  constructor arg for now; config wiring is M4.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-05-28 (progress)

Implemented bash_classify.rs with Severity { Allow, Block } + classify() function covering all curated Block patterns from §A (filesystem destruction, privilege/RCE, system control, irreversible git ops, publish, process kill). Normalization lowercases + collapses whitespace. Implemented bash.rs lifted from Rexy with three security adaptations: classifier gate (Block → advisory refusal, command never spawned), cwd-pin (Command::current_dir(scope.root())), and env-strip (env_clear + is_allowed_env_key predicate). All tests written and passing.

### Update — 2026-05-28 (progress) — bug-05-1 fix

Fixed classifier over-blocking benign commands (bug-05-1). Moved system-control
words (shutdown, reboot, halt, poweroff), privilege-escalation (sudo, su), and
process-kill (kill -9, pkill, killall) out of BLOCK_SUBSTRINGS into a
command-position regex (DANGEROUS_CMD_RE) that matches only at string start or
after shell separators (;, &, |, (, newline). Folded init 0/6 regexes into the
same pattern. Added tests for benign-substring cases (cargo test shutdown, grep
halt, echo "run with sudo", etc.) and separator-after cases (echo hi && shutdown).

### Update — 2026-05-28 (complete)

**Summary:** Lifted bash tool from Rexy with security adaptations (classifier gate, cwd-pin, env-strip). Created net-new bash_classify classifier with curated Block patterns. Wired both through security/mod.rs and tools/mod.rs. Added "process" feature to tokio in root Cargo.toml. All acceptance criteria met.

**Acceptance criteria:** all ticked above.

**Commands:**

```
cargo fmt --all --check
(no output — clean)

cargo build 2>&1 | tail -20
   Compiling rexymcp-executor v0.1.0 (/home/matt/src/rexyMCP/executor)
   Compiling rexymcp v0.1.0 (/home/matt/src/rexyMCP/mcp)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.48s

cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail -20
    Checking rexymcp-executor v0.1.0 (/home/matt/src/rexyMCP/executor)
    Checking rexymcp v0.1.0 (/home/matt/src/rexyMCP/mcp)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.48s

cargo test 2>&1 | tail -30
test result: ok. 165 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 6.02s
```

**End-to-end verification:**
Not applicable — this phase ships a library tool + a pure classifier, exercised directly by their unit tests. The registry/loop that drives bash (and the governor that consults the classifier) lands in M4; the MCP execute_phase in M5. Restated from phase doc.

**Files changed:**
- `Cargo.toml` — added "process" feature to tokio workspace dependency
- `executor/src/security/mod.rs` — added bash_classify module + re-exports
- `executor/src/security/bash_classify.rs` — new file: Severity enum, classify(), is_allowed_env_key tests
- `executor/src/tools/mod.rs` — added bash module + re-exports (Bash, bash, is_allowed_env_key)
- `executor/src/tools/bash.rs` — new file: lifted from Rexy with classifier gate, cwd-pin, env-strip

**New tests:**
- `blocks_rm_rf_variants` in `bash_classify.rs`
- `blocks_mkfs` in `bash_classify.rs`
- `blocks_dd_to_dev` in `bash_classify.rs`
- `blocks_write_to_dev_sd` in `bash_classify.rs`
- `blocks_write_to_dev_nvme` in `bash_classify.rs`
- `blocks_fork_bomb` in `bash_classify.rs`
- `blocks_chmod_r_777_root` in `bash_classify.rs`
- `blocks_chown_r_on_root` in `bash_classify.rs`
- `blocks_sudo` in `bash_classify.rs`
- `blocks_su` in `bash_classify.rs`
- `blocks_curl_pipe_shell` in `bash_classify.rs`
- `blocks_eval_curl` in `bash_classify.rs`
- `blocks_eval_wget` in `bash_classify.rs`
- `blocks_system_control` in `bash_classify.rs`
- `blocks_git_push` in `bash_classify.rs`
- `blocks_git_reset_hard` in `bash_classify.rs`
- `blocks_git_clean_f` in `bash_classify.rs`
- `blocks_git_checkout_dot` in `bash_classify.rs`
- `blocks_git_restore_dot` in `bash_classify.rs`
- `blocks_git_force_push` in `bash_classify.rs`
- `blocks_publish_commands` in `bash_classify.rs`
- `blocks_process_kill` in `bash_classify.rs`
- `allows_benign_commands` in `bash_classify.rs`
- `normalization_handles_extra_whitespace_and_case` in `bash_classify.rs`
- `runs_zero_exit_command` in `bash.rs`
- `non_zero_exit_appears_in_status_line` in `bash.rs`
- `captures_stderr` in `bash.rs`
- `captures_both_streams_together` in `bash.rs`
- `truncates_long_output` in `bash.rs`
- `does_not_truncate_short_output` in `bash.rs`
- `times_out_advisory_failure` in `bash.rs`
- `default_timeout_used_when_arg_absent` in `bash.rs`
- `arg_timeout_overrides_constructor_default` in `bash.rs`
- `rejects_empty_command` in `bash.rs`
- `rejects_malformed_args` in `bash.rs`
- `blocked_command_is_not_executed` in `bash.rs`
- `cwd_is_pinned_to_scope_root` in `bash.rs`
- `is_allowed_env_key_allows_whitelisted` in `bash.rs`
- `is_allowed_env_key_allows_lc_prefix` in `bash.rs`
- `is_allowed_env_key_rejects_others` in `bash.rs`

**Commits:** pending

**Notes for review:** None.

**verification:** fmt OK · clippy OK · tests 165 passed · build OK

**Bug fix verification (bug-05-1):**
- `classify("cargo test shutdown")` → Allow ✓
- `classify("grep -rn shutdown src/")` → Allow ✓
- `classify("./scripts/shutdown_test.sh")` → Allow ✓
- `classify("grep halt notes.txt")` → Allow ✓
- `classify("echo \"run with sudo\"")` → Allow ✓
- `classify("shutdown now")` → Block ✓
- `classify("foo && reboot")` → Block ✓
- `classify("init 0")` → Block ✓
