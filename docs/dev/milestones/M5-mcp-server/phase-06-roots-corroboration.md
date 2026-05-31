# Phase 06: roots corroboration (M5 closer)

**Milestone:** M5 — MCP server
**Status:** todo
**Depends on:** M5 phase-05b (done) — extends the `execute_phase_inner_with_client` seam introduced for testability there. M5 phase-02 — the `RexyMcpServer` + manual `ServerHandler` impl.
**Estimated diff:** ~300 lines (roots module + server hook + tests)
**Tags:** language=rust, kind=feature, size=m

## Goal

Before `execute_phase` does any work, **corroborate the caller-supplied
`repo_path` against two independent sources**:

1. The MCP client's **`roots/list`** (server-to-client request, per MCP spec).
2. **`CLAUDE_PROJECT_DIR`** (env var Claude Code sets to the project directory
   that initiated the conversation).

If either source is present and **none** of its entries contain `repo_path`
(either equal to, or an ancestor of), refuse the call with a clear `Err`. If
both sources are absent (no roots advertised + env var unset), proceed
without corroboration — log the absence in the Update Log path but don't
refuse (the M2 `Scope` is the actual security boundary; corroboration is a
*safety* check against misconfiguration).

This is the **M5 closer.** On approval, the milestone gets its retrospective +
the calibration folds the verdicts have been queuing.

## Architecture references

- `docs/architecture.md` — Layer 2 "Practical concerns": "**Roots.** The server
  queries Claude Code's `roots/list` (and reads `CLAUDE_PROJECT_DIR`) to
  **corroborate the target-repo root** — a second source for the scope
  boundary alongside `execute_phase`'s `repo_path` argument, so a mismatch can
  be caught rather than silently trusted. (Sampling and elicitation are
  deliberately *not* used: Claude Code doesn't support server-initiated
  sampling, and we don't pull the human into the loop mid-phase.)"
- Status §M5: "Queries `roots/list` / `CLAUDE_PROJECT_DIR` to corroborate the
  target-repo root against `execute_phase`'s `repo_path`."
- M5 README Notes — output-capping pattern (not relevant here; mentioned for
  context).
- M5 phase-05b: `execute_phase_inner_with_client` seam — the extension point.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` and the M5 README.
2. Read this entire phase doc.
3. **Verify rmcp 1.7's `roots/list` client API.** The server's `Peer` should
   expose a `list_roots()` method (or equivalent) that returns a `ListRootsResult
   { roots: Vec<Root> }`. The `Root` shape is `{ uri: String, name: Option<String> }`
   per MCP spec; verify rmcp's exact type. **Check whether the client must
   declare the `roots` capability for the call to be valid** — if the client
   doesn't, the call should be skipped (not errored), since not all clients
   support roots. Pre-flight 3 discipline (phase-02 / 05b): trust the docs over
   the architect's sketch; flag divergence in "Notes for review".
4. Confirm `execute_phase_inner_with_client` is the testable seam (phase-05b).
   The corroboration call site lives in the manual `ServerHandler` /
   `call_tool` for `execute_phase` — at the top, before token extraction or
   any other work.

## Spec

### 1. New module — `mcp/src/roots.rs`

A pure module (no rmcp, no I/O) holding the corroboration logic. Declared `mod
roots;` in `mcp/src/main.rs`.

```rust
use std::path::{Path, PathBuf};

/// Result of the corroboration check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Corroboration {
    /// At least one source matched. Names the winning source for the log.
    Matched(MatchedSource),
    /// Sources existed but none matched. The handler turns this into an Err.
    Mismatch {
        repo_path: PathBuf,
        roots: Vec<String>,            // raw URIs as advertised
        project_dir: Option<PathBuf>,
    },
    /// No sources to check (no roots, no env var). Pass-through; the
    /// handler logs and proceeds.
    NoSources,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MatchedSource {
    Root { uri: String },               // the root URI that contains repo_path
    ProjectDir(PathBuf),                // CLAUDE_PROJECT_DIR (canonicalized)
}

/// Pure corroboration. `roots` are raw URIs (`file:///foo/bar`) as advertised
/// by the client; `project_dir` is `CLAUDE_PROJECT_DIR` already read by the
/// caller (None when unset/empty).
pub fn corroborate(
    repo_path: &Path,
    roots: &[String],
    project_dir: Option<&Path>,
) -> Corroboration;

/// Format a Mismatch into the error string returned by the tool handler.
/// Public so the handler maps `Corroboration::Mismatch` → `Err(String)`
/// uniformly.
pub fn format_mismatch_error(
    repo_path: &Path,
    roots: &[String],
    project_dir: Option<&Path>,
) -> String;
```

### 2. Corroboration algorithm

For each call:

1. **Canonicalize `repo_path`** (`std::fs::canonicalize`). Resolves symlinks,
   normalizes `..`, etc. On failure (path doesn't exist), return
   `Corroboration::Mismatch` with the un-canonicalized inputs — a nonexistent
   `repo_path` is its own form of misconfiguration that the architect should
   see, not silently let through.
2. **Convert each root URI to a path:** strip the `file://` prefix (the only
   scheme MCP roots support per spec). URIs without `file://` are skipped (not
   an error — unknown scheme). URL-encoded path components (`%20` etc.) are
   **not** decoded in this phase; pin in tests and document as a follow-up
   (Adaptation 3). For each resulting path, canonicalize; on failure, skip
   that root (best-effort).
3. **Match logic:** `repo_path` matches a source iff
   `repo_path == source || repo_path.starts_with(source)` *after* canonicalization
   of both sides. (`starts_with` is a path-component-aware prefix on `PathBuf`,
   which is what we want — *not* a string prefix.)
4. **Resolution order:**
   - If `roots.is_empty() && project_dir.is_none()` → `NoSources`.
   - Else, try each root in advertised order. On match → `Matched(Root { uri:
     <original_uri> })`.
   - Then try `project_dir`. On match → `Matched(ProjectDir(<canonical>))`.
   - Else → `Mismatch { repo_path, roots: <originals>, project_dir }`.

   Order is documented; resolution returns the **first** match, not all.

5. **`format_mismatch_error`** produces a single multi-line string:
   ```
   repo_path <repo> does not corroborate against any MCP root or CLAUDE_PROJECT_DIR.
     Inspected roots: [<uri1>, <uri2>, …]   (or "none advertised")
     CLAUDE_PROJECT_DIR: <path>             (or "(unset)")
   This usually means the architect passed the wrong repo_path, or the MCP
   client roots / CLAUDE_PROJECT_DIR are misconfigured. Fix one of those and
   re-dispatch.
   ```

### 3. Server wiring — `mcp/src/server.rs`

The corroboration happens **first thing** inside `call_tool` for
`execute_phase` (the manual `ServerHandler::call_tool` branch, before progress
token extraction or any other work):

```rust
// Pseudo-code — verify rmcp's exact peer + roots API in pre-flight 3.
if name == "execute_phase" {
    let params: ExecutePhaseParams = serde_json::from_value(arguments)?;
    let repo_path = PathBuf::from(&params.repo_path);

    // (1) Query roots — only if client declared the capability.
    let roots: Vec<String> = if peer.client_declares_roots() {
        match peer.list_roots().await {
            Ok(result) => result.roots.into_iter().map(|r| r.uri).collect(),
            Err(_) => Vec::new(),  // best-effort; log later, don't fail here
        }
    } else {
        Vec::new()
    };

    // (2) Read env.
    let project_dir = std::env::var_os("CLAUDE_PROJECT_DIR")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty());

    // (3) Corroborate.
    match roots::corroborate(&repo_path, &roots, project_dir.as_deref()) {
        Corroboration::Matched(_) => { /* proceed */ }
        Corroboration::NoSources => { /* proceed; log it */ }
        Corroboration::Mismatch { .. } => {
            return Err(roots::format_mismatch_error(
                &repo_path, &roots, project_dir.as_deref()
            ));
        }
    }

    // Continue with existing progress-token extraction + execute_phase_inner.
}
```

**Where to log the result?** The session log doesn't exist yet at this point
(it's opened inside the loop). The corroboration outcome should appear in the
**`PhaseResult.update_log`** field on success — but that's also constructed
by the loop. Simplest: don't log on success at all (success is the silent
case). On `NoSources`, return a normal `PhaseResult` but log the absence to
stderr (the rmcp server's stderr; visible to the human running `rexymcp
serve`). On `Mismatch`, the `Err(String)` is the entire signal. Document in
Adaptation 4.

### 4. Test seam

The corroboration check uses two inputs (`roots`, `project_dir`) that arrive
from rmcp / env. The pure `roots::corroborate` is fully unit-testable on its
own — that's most of the coverage. For the *handler* (the rmcp call site),
either:

- **Option A — extract the inputs at the handler boundary:** the manual
  `call_tool` branch builds `roots` + `project_dir`, calls `corroborate`, then
  the rest of the handler path is unchanged. Tests cover `roots::corroborate`
  exhaustively; the wiring is a 5-line shim.
- **Option B — add another seam:** factor a `pub(crate) async fn
  corroborate_repo_path_via_peer(peer, repo_path) -> Result<(), String>` that
  encapsulates the rmcp call + env read + corroborate + format. Test the pure
  module; skip integration testing of the rmcp peer call (no mock peer in
  rmcp 1.7 — same situation as phase-05b's notifier).

**Recommended: Option A.** The pure module has all the logic; the handler
shim is too small to be worth a separate testable layer. The rmcp call is the
one thing tests can't cover hermetically; that's M6 dogfood territory.

If pre-flight 3 reveals that rmcp 1.7 *does* expose a way to mock the peer
inside tests, escalate to Option B (note in Notes for review).

### 5. `execute_phase_inner_with_client` is not modified

Corroboration happens **outside** the inner fn — in the manual `call_tool`
branch — because the inner fn is also called from the wrapper-level tests
(phase-05b's `execute_phase_inner_forwards_progress_to_loop`,
`execute_phase_inner_with_none_captures_nothing`). Those tests do not have a
client peer; they shouldn't be gated by corroboration. Keep the inner fn
unchanged.

This is the right layering: corroboration is an **MCP-boundary concern**
(needs peer + env), so it belongs at the MCP-boundary code path
(`call_tool`), not deeper in the assembler.

### 6. Documentation

Add a one-line tool description on `execute_phase` (the existing
`#[rmcp::tool(description = "…")]` text) mentioning the corroboration: "…
`repo_path` is corroborated against the MCP client's `roots/list` and
`CLAUDE_PROJECT_DIR`; a mismatch refuses the call." This is the only
external-visible behavior change; the description should reflect it.

## Adaptations / decisions

1. **Hard refusal on mismatch, not soft warning.** Architecture says "flag a
   mismatch rather than trusting it"; running with the wrong `repo_path`
   would create files in the wrong place and run the project's commands
   against the wrong tree — fail-fast is the right posture.
2. **No-sources permissive.** When neither roots nor `CLAUDE_PROJECT_DIR`
   exists, proceed. The M2 `Scope` is the actual security boundary; this
   phase is a *safety* check that requires at least one source to fail
   meaningfully.
3. **URL-encoded path components not decoded.** A root URI like
   `file:///foo%20bar/baz` will be treated as path `/foo%20bar/baz` (literal
   `%20`), which won't match `/foo bar/baz`. Pin in a test; document as a
   follow-up. Real-world repo paths rarely have spaces; revisit if dogfood
   surfaces it.
4. **Corroboration outcome is silent on success, stderr-logged on no-sources,
   `Err(String)` on mismatch.** Session log doesn't exist this early; the
   `PhaseResult.update_log` is constructed by the loop. Keeping the success
   path silent avoids a chatty server; the no-sources case warrants a notice
   so the human can verify intentional config.
5. **Canonicalize both sides.** Otherwise `/foo/bar` vs `/foo/bar/` or a
   symlinked path would produce false negatives. Failure to canonicalize
   `repo_path` is a mismatch (the path doesn't exist); failure to canonicalize
   a root or env var skips that source (best-effort).
6. **No new dependency.** No URL parser, no env-handling crate;
   `std::fs::canonicalize` + `std::env::var_os` + string prefix-strip is
   sufficient for the spec.
7. **No `executor/` edits.** Corroboration is mcp-boundary-only.

## Acceptance criteria

- [ ] `mcp/src/roots.rs` exists; `mod roots;` is wired in `mcp/src/main.rs`;
      `Corroboration`, `MatchedSource`, `corroborate`, `format_mismatch_error`
      are reachable.
- [ ] **`corroborate` correctness:**
  - `repo_path` == a root → `Matched(Root { uri })`
  - `repo_path` is a descendant of a root → `Matched(Root { uri })`
  - `repo_path` == `project_dir` → `Matched(ProjectDir(_))`
  - `repo_path` is a descendant of `project_dir` → `Matched(ProjectDir(_))`
  - Multiple roots, only one matches → `Matched(Root { uri: <that one> })`
  - First-match resolution order (roots before project_dir)
  - Sources exist but none match → `Mismatch { … }`
  - `roots.is_empty() && project_dir.is_none()` → `NoSources`
  - `file://` prefix stripped correctly (`file:///foo/bar` → `/foo/bar`)
  - Non-`file://` URIs (e.g. `http://…`) are skipped, not errored
- [ ] **`corroborate` negatives (pinned):**
  - `repo_path` doesn't exist on disk → `Mismatch` (canonicalize fails;
    document the rationale)
  - A root URI is malformed / un-canonicalizable → that root skipped, others
    still considered
  - URL-encoded path (`%20`) does **not** match an unencoded equivalent
    (Adaptation 3 pin)
  - Symlinks: a symlinked `repo_path` whose target is under a root → matches
    via canonicalization
- [ ] **`format_mismatch_error`** produces the prescribed multi-line string
      with the inspected roots list (or `"none advertised"`), the
      `CLAUDE_PROJECT_DIR` (or `"(unset)"`), and the architect-facing fix
      hint.
- [ ] **Handler integration in `mcp/src/server.rs` `call_tool` for
      `execute_phase`:** corroboration happens **before** any other work
      (before token extraction, before `execute_phase_inner`). On `Matched`
      or `NoSources` → proceed unchanged. On `Mismatch` → return
      `Err(format_mismatch_error(…))`.
- [ ] **`execute_phase_inner_with_client` is NOT modified** — it remains
      callable from the phase-05b wrapper-level tests without a peer.
- [ ] **Capability check:** when the client doesn't declare `roots`, skip the
      `list_roots` call. When the client *declares* but `list_roots()` errors,
      treat `roots` as empty (best-effort) — falls through to `project_dir`
      and possibly `NoSources` / `Mismatch`.
- [ ] **The `execute_phase` tool description** is updated to mention
      corroboration (one short sentence appended).
- [ ] **No `#[allow]`**; no `unwrap()` / `expect()` / `panic!()` in production
      paths; no Rexy phase references; no new dependency; no `executor/` edits.
- [ ] **Calibration carry-forward (mandatory):** declare every scope deviation
      in "Notes for review", even defensible ones. M5's discipline is mature
      — keep the bar.
- [ ] All four required commands pass with zero new warnings.

## Test plan

Hermetic + deterministic. In `mcp/src/roots.rs` `#[cfg(test)] mod tests`:

- **Match positives** (one each):
  - `repo_path == root` → Matched(Root)
  - `repo_path` descendant of root → Matched(Root)
  - `repo_path == project_dir` → Matched(ProjectDir)
  - `repo_path` descendant of project_dir → Matched(ProjectDir)
- **Resolution order:**
  - root matches AND project_dir would also match → returns Root (first-match)
  - first root matches, second wouldn't → returns first root
- **No-sources:**
  - empty roots + None project_dir → NoSources
- **Mismatch:**
  - roots present, all wrong; project_dir present, also wrong → Mismatch
- **URI parsing:**
  - `file:///foo/bar` → matches `/foo/bar`
  - `http://example.com/foo` → skipped (non-file scheme)
  - `file:///foo%20bar/baz` does **not** match `/foo bar/baz` (Adaptation 3 pin)
- **Path edges:**
  - nonexistent `repo_path` → Mismatch (canonicalize fails)
  - symlinked `repo_path` whose canonical target is under a root → matches
    (use a `TempDir` + `std::os::unix::fs::symlink`; `#[cfg(unix)]`-gated)
  - root that doesn't exist on disk → skipped, others still considered
- **`format_mismatch_error` shape:**
  - includes the architect-facing fix hint
  - "none advertised" / "(unset)" for absent sources
  - lists each root URI

In `mcp/src/server.rs` `#[cfg(test)] mod tests` (extend):

- **Handler smoke (Option A):** a small test that constructs a `repo_path`
  not matching any root nor env var, and a `corroborate` call returns
  `Mismatch`. *Not* a full `call_tool` test (no peer available); just verify
  the wiring would call `corroborate` correctly. If a small helper like `fn
  evaluate_corroboration(repo_path, roots, env) -> Result<(), String>` is
  factored out of `call_tool`, test that.
- **No regression in existing tests** — all 100 mcp tests + 512 executor
  tests still pass.

## End-to-end verification

> Partial — same as phases 02–05. The corroboration *logic* is fully unit-tested.
> The rmcp peer call + actual `roots/list` over stdio is M6 dogfood territory.
> Manual smoke: launch `rexymcp serve` against an MCP client that supports
> roots, dispatch `execute_phase` with a deliberately wrong `repo_path`,
> observe the `Err(format_mismatch_error)` come back.

## Authorizations

- [x] **May create** `mcp/src/roots.rs`; **may modify** `mcp/src/server.rs`
      (corroboration shim in the `execute_phase` `call_tool` branch + small
      helper if needed for testability + tool description), `mcp/src/main.rs`
      (declare `mod roots;`).
- [ ] **No new dependencies.**
- [ ] **No `executor/` edits.** Corroboration is mcp-boundary-only.
- [ ] May **NOT** modify `execute_phase_inner` or `execute_phase_inner_with_client`
      (those are downstream of corroboration; the check happens earlier).
- [ ] May **NOT** modify `mcp/src/runner.rs`, `mcp/src/cap.rs`,
      `mcp/src/log_query.rs`, `mcp/src/scorecard.rs`, `mcp/src/progress.rs`-equivalent
      (none exists — 05a/05b live in executor and server, no separate progress
      module in mcp), or any other phase doc.
- [ ] May **NOT** modify `docs/architecture.md`, `STANDARDS.md`, `WORKFLOW.md`,
      `AGENTS.md`.

## Out of scope

- **URL-encoded path decoding** — Adaptation 3 (follow-up if dogfood needs).
- **MCP `notifications/roots/list_changed` subscription** — the client tells
  us when roots change; the server doesn't react. We re-query on each
  `execute_phase` call, which is sufficient. Subscription is a future
  optimization (only matters for very long-running servers).
- **Capability re-negotiation** — assume the client's declared capabilities
  are stable for the connection.
- **Logging corroboration outcomes into the JSONL session log** — the log
  doesn't exist this early (opens inside the loop). Stderr notice for
  NoSources; `Err(String)` for Mismatch; silent on Matched.
- **`Scope` integration** — the M2 `Scope` is the actual security boundary
  and is constructed downstream from `repo_path` regardless. Corroboration
  is a separate, additional safety check.

## M5 close (after approval)

When this phase is approved + signed off, the **M5 milestone closes**. That
triggers the human-gate ritual the workflow defines:

1. **M5 retrospective** appended to the M5 README (mirror the M4 retrospective
   format).
2. **Calibration folds** queued across phases 02–05b:
   - **Derive-vs-wrap rule** (exercised 4× across M5): wrap with
     `serde_json::Value` when the schema tree is large or foreign (PhaseResult,
     SessionRecord); derive `JsonSchema` directly when the schema is small and
     locally-owned (Health, ScorecardRow). Fold into `STANDARDS.md` or a
     `WORKFLOW.md` note.
   - **Cross-boundary trait bounds:** plan for `Serialize` /
     `Deserialize` / `Send+Sync` / `JsonSchema` whenever a new boundary lands.
     Recurrences: M4 phase-03 (`Deserialize` on parser types), M5 phase-02
     (`Send+Sync` on `LoopDeps.clock`, `JsonSchema` on `Health`), M5 phase-03
     (`Value`-wrap for large foreign tree), M5 phase-04 (`JsonSchema` derive
     for small mcp-owned tree), M5 phase-05a (`Send+Sync` on
     `ProgressCallback`). Five recurrences = a fold-worthy rule.
3. **Phase-01's bug + 05b's bug** as recurring lessons:
   - bug-01-1: CLI parse test (acceptance criterion miss). Resolved
     cleanly; calibration now firm.
   - bug-05b-1: `#[allow]` hard-rule violation + missing wrapper tests.
     Resolved with the struct-grouping pattern (Issue 1) + seam-extraction
     pattern (Issue 2). The latter is *itself* a pattern worth noting:
     when a wrapper-test is wanted, the spec must pin the testability
     mechanism, not just name the test.
4. **NEXT.md** points at M6 kickoff (the next milestone), awaiting human gate.

Phase-06 itself does **not** write the retrospective — that's the architect's
post-approval ritual. Phase-06's job ends when bug-06-1-if-any is verified
and gates are clean.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
