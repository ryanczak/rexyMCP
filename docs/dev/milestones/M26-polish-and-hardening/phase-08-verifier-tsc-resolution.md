# Phase 08: Verifier `tsc` resolution (`node_modules/.bin` → `npx` → PATH)

**Milestone:** M26 — Polish & Hardening
**Status:** review
**Depends on:** none
**Estimated diff:** ~160 lines
**Tags:** language=rust, kind=bugfix, size=m

## Goal

The TypeScript verifier invokes `tsc` as a bare PATH binary
(`executor/src/governor/verifier.rs:431`). In real Node repos `tsc` is almost
never global — it lives in `node_modules/.bin/tsc` — so the verifier `Skipped`s
(NotFound) and the executor loses incremental TS feedback on exactly the projects
that need it. This phase resolves `tsc` in priority order — **local
`node_modules/.bin/tsc` (walking up to catch monorepo hoisting) → `npx
--no-install tsc` → bare `tsc` on PATH** — before spawning. Rust and Python
verification are untouched.

This is the last M26 phase; approving it closes the milestone (a human-gated
boundary — see WORKFLOW § "Milestone boundaries are always a human gate").

## Architecture references

Read before starting:

- `docs/dev/codebase-review-2026-07-07.md` § "Verifier practicality" — the review
  finding this phase closes (the `tsc` bullet at line 87).
- `docs/architecture.md` § Status #26 — milestone context.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

`verify_typescript` in `executor/src/governor/verifier.rs` (lines 420–455)
resolves the project root, then spawns a **bare** `tsc`:

```rust
async fn verify_typescript(path: &Path) -> VerifierResult {
    let project_root = match find_typescript_project_root(path) {
        Some(root) => root,
        None => {
            return VerifierResult::Failed(format!(
                "no tsconfig.json found at or above {}",
                path.display(),
            ));
        }
    };

    let output = match Command::new("tsc")
        .arg("--noEmit")
        .arg("--pretty=false")
        .current_dir(&project_root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
    {
        Ok(o) => o,
        Err(e) => {
            return spawn_failure("tsc", "install TypeScript (npm install -g typescript)", &e);
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut diagnostics = Vec::new();
    for line in stdout.lines() {
        if let Some(diag) = parse_tsc_line(line, &project_root) {
            diagnostics.push(diag);
        }
    }

    VerifierResult::Checked { diagnostics }
}
```

`spawn_failure` (lines 241–250) already maps a `NotFound` spawn error to a
`Skipped` advisory — that behavior stays; we just make NotFound far less likely
by resolving the real binary first.

**Relevant idioms already in the codebase** (mirror these, don't reinvent):

- `mcp/src/doctor.rs:15` `resolve_binary` — the exact PATH-scan shape (`.is_file()`
  on `dir.join(name)`), including the guard that a *directory* named `tsc` is not
  a match, and `path_dirs()` at line 118 (`std::env::var_os("PATH")` → `split_paths`).
  `doctor.rs` is in the `mcp` **binary** crate; the verifier is in the `executor`
  **lib** crate, and `mcp` depends on `executor` (not the reverse) — so you **cannot**
  import `resolve_binary`. Write sibling helpers in `verifier.rs`, mirroring the shape.
- `find_typescript_project_root` (line 457) and `find_ancestor_with` (line 205) —
  the ancestor-walk shape for the new `find_local_tsc`.

## Spec

Numbered tasks in execution order. All edits are in
`executor/src/governor/verifier.rs` and its test file
`executor/src/governor/verifier_tests.rs`.

1. **Add a resolved-command struct and three pure resolver helpers** — in
   `executor/src/governor/verifier.rs`, above `verify_typescript`. Copy these
   verbatim (they are complete):

   ```rust
   /// A resolved `tsc` invocation: the program to spawn plus any
   /// prefix args that must precede tsc's own flags. `prefix_args`
   /// is non-empty only for the `npx` form (`npx --no-install tsc`).
   struct TscCommand {
       program: PathBuf,
       prefix_args: &'static [&'static str],
   }

   /// The PATH search directories, or empty if PATH is unset.
   fn path_dirs() -> Vec<PathBuf> {
       std::env::var_os("PATH")
           .map(|p| std::env::split_paths(&p).collect())
           .unwrap_or_default()
   }

   /// Walk from `project_root` up to the filesystem root looking for
   /// `node_modules/.bin/tsc`. Returns the first existing *file*
   /// (a directory of that name is not a match). Walking up catches
   /// monorepo dependency hoisting, where `node_modules` sits at the
   /// workspace root above the package's `tsconfig.json`.
   fn find_local_tsc(project_root: &Path) -> Option<PathBuf> {
       let mut current = Some(project_root);
       while let Some(dir) = current {
           let candidate = dir.join("node_modules").join(".bin").join("tsc");
           if candidate.is_file() {
               return Some(candidate);
           }
           current = dir.parent();
       }
       None
   }

   /// True if `name` resolves to an existing file in any of the
   /// given search directories. Mirrors `doctor::resolve_binary`'s
   /// bare-name branch; kept local because the `mcp` crate (where
   /// that lives) depends on this one, not the reverse.
   fn binary_in_dirs(name: &str, search_paths: &[PathBuf]) -> bool {
       search_paths.iter().any(|dir| dir.join(name).is_file())
   }

   /// Resolve which `tsc` invocation to spawn, in priority order:
   /// local `node_modules/.bin/tsc` → `npx --no-install tsc` → bare
   /// `tsc` on PATH. `npx_on_path` is threaded in (not read from the
   /// environment here) so the resolution stays a pure, hermetically
   /// testable function.
   fn resolve_tsc_command(project_root: &Path, npx_on_path: bool) -> TscCommand {
       if let Some(local) = find_local_tsc(project_root) {
           return TscCommand {
               program: local,
               prefix_args: &[],
           };
       }
       if npx_on_path {
           return TscCommand {
               program: PathBuf::from("npx"),
               prefix_args: &["--no-install", "tsc"],
           };
       }
       TscCommand {
           program: PathBuf::from("tsc"),
           prefix_args: &[],
       }
   }
   ```

   `--no-install` keeps `npx` from reaching the network to fetch TypeScript: it
   runs only an already-resolvable `tsc`. (Known limitation, **out of scope** —
   see below: if `npx` is present but no `tsc` resolves anywhere, `npx` exits
   non-zero with empty stdout and the parse yields `Checked { diagnostics: [] }`,
   read as "clean". This is no worse than today's bare-`tsc` NotFound path and
   fixing it needs npx-stderr parsing, deferred.)

2. **Rewire `verify_typescript` to spawn the resolved command** — in
   `executor/src/governor/verifier.rs`, replace the `Command::new("tsc")` block
   (lines 431–444 above) so the program and prefix args come from
   `resolve_tsc_command`. The `--noEmit --pretty=false`, `current_dir`, piping,
   and the `stdout.lines()` parse loop stay exactly as they are. The new head of
   the spawn:

   ```rust
   let cmd = resolve_tsc_command(&project_root, binary_in_dirs("npx", &path_dirs()));
   let output = match Command::new(&cmd.program)
       .args(cmd.prefix_args)
       .arg("--noEmit")
       .arg("--pretty=false")
       .current_dir(&project_root)
       .stdout(Stdio::piped())
       .stderr(Stdio::piped())
       .output()
       .await
   {
       Ok(o) => o,
       Err(e) => {
           return spawn_failure(
               "tsc",
               "install TypeScript locally (npm install -D typescript) or globally \
                (npm install -g typescript)",
               &e,
           );
       }
   };
   ```

   Note the expanded install hint (Task 3 folds into this edit). Resolution
   happens **after** the `find_typescript_project_root` None → `Failed` check, so
   the "no tsconfig.json" behavior is byte-identical.

3. **(Folded into Task 2)** Expand the `spawn_failure` install hint to mention the
   local install, since local resolution is now the primary path. No separate edit.

4. **Add unit tests for the pure resolvers** — in
   `executor/src/governor/verifier_tests.rs`, in the existing `mod tests` block.
   Cover the resolution matrix and the ancestor walk:

   - `find_local_tsc_finds_at_project_root` — create `<root>/node_modules/.bin/tsc`
     as a file; assert `find_local_tsc(<root>)` is `Some(that path)`.
   - `find_local_tsc_walks_up_to_hoisted_node_modules` — put `tsconfig.json` in
     `<root>/pkg` and `node_modules/.bin/tsc` at `<root>` (hoisted); assert
     `find_local_tsc(<root>/pkg)` returns `<root>/node_modules/.bin/tsc`.
   - `find_local_tsc_none_when_absent` — no `node_modules`; assert `None`.
   - `find_local_tsc_ignores_directory_named_tsc` — create
     `<root>/node_modules/.bin/tsc` as a **directory**; assert `None` (the
     `.is_file()` guard — the load-bearing negative, mirroring
     `doctor::resolve_binary_rejects_directory_of_same_name`).
   - `resolve_tsc_command_prefers_local_over_npx` — local `tsc` present **and**
     `npx_on_path = true`; assert `program` is the local path and `prefix_args`
     is empty (local wins even when npx is available).
   - `resolve_tsc_command_uses_npx_when_no_local` — no local, `npx_on_path = true`;
     assert `program == PathBuf::from("npx")` and `prefix_args == ["--no-install",
     "tsc"]`.
   - `resolve_tsc_command_falls_back_to_path_tsc` — no local, `npx_on_path = false`;
     assert `program == PathBuf::from("tsc")` and `prefix_args` is empty.
   - `binary_in_dirs_finds_file` / `binary_in_dirs_false_when_absent` — a TempDir
     search path with and without an `npx` file.

5. **Add a real-spawn E2E test proving resolution wires into the spawn** — in
   `verifier_tests.rs`, `#[cfg(unix)]`-gated (needs an executable bit + `/bin/sh`;
   the project targets linux and CI is linux). Write a fake local `tsc` shell
   script that emits one tsc-format error line, then assert `verify_typescript`
   returns `Checked` carrying that diagnostic — proving it spawned the **local**
   binary, not a PATH `tsc`, and parsed its output end-to-end:

   ```rust
   #[cfg(unix)]
   #[tokio::test]
   async fn verify_typescript_spawns_resolved_local_binary() {
       use std::os::unix::fs::PermissionsExt;

       let dir = tempfile::TempDir::new().unwrap();
       let root = dir.path();
       fs::write(root.join("tsconfig.json"), "{}").unwrap();
       let bin_dir = root.join("node_modules").join(".bin");
       fs::create_dir_all(&bin_dir).unwrap();
       let fake = bin_dir.join("tsc");
       fs::write(
           &fake,
           "#!/bin/sh\necho \"src/main.ts(3,7): error TS9999: fake diagnostic\"\n",
       )
       .unwrap();
       fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
       let src = root.join("src");
       fs::create_dir_all(&src).unwrap();
       fs::write(src.join("main.ts"), "").unwrap();

       let result = verify_typescript(&src.join("main.ts")).await;
       match result {
           VerifierResult::Checked { diagnostics } => {
               assert_eq!(diagnostics.len(), 1);
               assert_eq!(diagnostics[0].code, Some("TS9999".to_string()));
               assert_eq!(diagnostics[0].line, 3);
           }
           other => panic!("expected Checked from local fake tsc, got {:?}", other),
       }
   }
   ```

   This test is hermetic (everything under `TempDir`, no network) and
   deterministic — the fake binary always prints the same line. The existing
   `#[ignore]` live test (`verify_typescript_returns_checked_on_broken_code`) is
   left as-is.

## Acceptance criteria

- [ ] `find_local_tsc` walks ancestors and matches only a *file* named
      `node_modules/.bin/tsc`.
- [ ] `resolve_tsc_command` returns local → npx (`--no-install tsc`) → bare `tsc`
      in that priority order, with local winning even when npx is available.
- [ ] `verify_typescript` spawns the resolved program; the `--noEmit
      --pretty=false` flags, `current_dir`, and parse loop are unchanged.
- [ ] The "no tsconfig.json" `Failed` path is byte-identical (resolution runs
      after the project-root check).
- [ ] `verify_typescript_spawns_resolved_local_binary` passes (proves the local
      binary is actually spawned and its output parsed).
- [ ] `cargo build` succeeds with zero new warnings.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes.
- [ ] `cargo fmt --all --check` passes.
- [ ] `cargo test` passes (existing TS-verifier tests unchanged in behavior).

## Test plan

Unit tests (hermetic, `TempDir`):

- `find_local_tsc_finds_at_project_root` — direct hit.
- `find_local_tsc_walks_up_to_hoisted_node_modules` — monorepo hoist.
- `find_local_tsc_none_when_absent` — no node_modules.
- `find_local_tsc_ignores_directory_named_tsc` — `.is_file()` negative pin.
- `resolve_tsc_command_prefers_local_over_npx` — priority: local beats npx.
- `resolve_tsc_command_uses_npx_when_no_local` — npx form + prefix args.
- `resolve_tsc_command_falls_back_to_path_tsc` — bare PATH fallback.
- `binary_in_dirs_finds_file` / `binary_in_dirs_false_when_absent`.

Real-spawn E2E (hermetic, `#[cfg(unix)]`):

- `verify_typescript_spawns_resolved_local_binary` — fake local `tsc` script →
  `Checked` with the emitted diagnostic.

Unchanged: `verify_dispatches_ts_to_typescript` /
`verify_dispatches_tsx_to_typescript` still return `Failed("no tsconfig.json")`
(they never reach the spawn); `parse_tsc_line_*` untouched.

## End-to-end verification

The real artifact this phase ships is the verifier's runtime spawn behavior. The
`#[cfg(unix)]` `verify_typescript_spawns_resolved_local_binary` test **is** the
end-to-end check: it plants a real executable at `node_modules/.bin/tsc`, calls
`verify_typescript`, and confirms the resolved local binary was spawned and its
output parsed into a `Diagnostic`. Paste that test's `cargo test
verify_typescript_spawns_resolved_local_binary` output in the completion Update
Log.

(The `#[ignore]` real-TypeScript test is not runnable on hosts without `tsc`
installed and is not part of the DoD run.)

## Authorizations

None.

**Toolchain note (no new declaration required):** `tsc` and `npx` are part of the
Node toolchain already declared as a Tier-1 verifier enhancer (see
`mcp/src/doctor.rs:98` and WORKFLOW § "Validation features depend on the target
toolchain"). This phase improves *resolution* of that same enhancer; it introduces
no new runtime binary and the missing-binary behavior is unchanged (`spawn_failure`
→ `Skipped` advisory).

## Out of scope

Do **not** do these, even though the review mentions them near this finding:

- **Making verifier commands configurable per-language in `rexymcp.toml`** (review
  § "Verifier practicality", last bullet). A larger design change; separate phase.
- **A Go (`go vet`) verifier or feeding warning-level diagnostics back** (same
  review section). Not this phase.
- **The per-edit `cargo check` cost.** Untouched.
- **Parsing `npx` stderr to turn "npx present but no tsc" into `Skipped`.** The
  `--no-install` npx form can exit non-zero with empty stdout and be read as
  clean; accepted as a known limitation (no worse than today). Fixing it needs
  npx failure-mode parsing — deferred.
- **Windows `.cmd`/`.ps1` shim resolution.** The project targets linux.
- Any change to `verify_rust` / `verify_python` / `parse_tsc_line` /
  `find_typescript_project_root`.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-07-08 14:23 (started)

**Executor:** rexyMCP executor

**Progress:** Implementing tsc resolution: local `node_modules/.bin/tsc` → `npx --no-install tsc` → bare `tsc` on PATH. Adding resolver helpers, rewiring `verify_typescript`, and adding unit + E2E tests.

### Update — 2026-07-08 14:23 (complete)

**Executor:** rexyMCP executor

**Tasks completed:** 1–5

**Verification commands:**
- `cargo fmt --all --check` — passed
- `cargo build` — passed, zero new warnings
- `cargo clippy --all-targets --all-features -- -D warnings` — passed
- `cargo test` — 915 passed, 0 failed, 2 ignored
- `cargo test verify_typescript_spawns_resolved_local_binary` — 1 passed (E2E: local fake tsc spawned, output parsed into Diagnostic)

**End-to-end verification:** `verify_typescript_spawns_resolved_local_binary` planted a real executable at `node_modules/.bin/tsc`, called `verify_typescript`, and confirmed the resolved local binary was spawned and its output parsed into a `Diagnostic` with code `TS9999` at line 3.

**Grep for pinned literals:**
```
$ grep -c "find_local_tsc\|resolve_tsc_command\|binary_in_dirs\|TscCommand" executor/src/governor/verifier.rs
9
$ grep -c "find_local_tsc\|resolve_tsc_command\|binary_in_dirs" executor/src/governor/verifier_tests.rs
12
```

**Files changed:**
- `executor/src/governor/verifier.rs` — added `TscCommand`, `path_dirs`, `find_local_tsc`, `binary_in_dirs`, `resolve_tsc_command`; rewired `verify_typescript` spawn + expanded install hint
- `executor/src/governor/verifier_tests.rs` — added 10 unit tests + 1 E2E test

**Notes for review:** None. All spec items implemented verbatim.
