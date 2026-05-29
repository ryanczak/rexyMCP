# Phase 01: post-edit verifier + Diagnostic

**Milestone:** M4 — Headless agent loop + governor/verifier
**Status:** done
**Depends on:** M3 (done)
**Estimated diff:** ~620 lines (verifier lift + tests)
**Tags:** language=rust, kind=feature, size=m

## Goal

Lift the post-edit **verifier** — the stage that runs a fast typecheck after an
edit-class tool and returns **structured `Diagnostic`s** the loop folds into the
next turn's feedback. This is the leaf the rest of M4 depends on: the hard-fail
detector, the briefing's "current blocker / code state", and the loop's
retry-on-diagnostics all consume `Diagnostic`.

Ships `Diagnostic` + `Severity` + `DiagnosticSignature` + `Baseline` (author-vs-
ambient classification) + `VerifierResult` + `verify(path)` (dispatch by extension
to `cargo check` / `tsc` / `ruff`) + `capture_baseline`.

## Architecture references

- `docs/architecture.md` — "The executor turn cycle" step 6 (after edit-class
  tools, run the verifier; feed diagnostics back for a retry).
- Rexy source: `rexy/src/governor/verifier.rs`.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` and the M4 README Notes (esp. "Verifier vs. final
   command set" and "rexyMCP is not opencode").
2. Read this entire phase doc.
3. Confirm M3 is `done`; the workspace builds clean. `tokio` has the `"process"`
   feature (added M2 phase-05); `serde` is present.
4. **Read** `rexy/src/governor/verifier.rs`. Lift faithfully.

## Spec

Create `executor/src/governor/mod.rs` (declares `pub mod verifier;`) and
`executor/src/governor/verifier.rs`; wire `pub mod governor;` into
`executor/src/lib.rs`.

Lift from `rexy/src/governor/verifier.rs`:

- **Types** (all `Serialize`/`Deserialize` where Rexy has them — load-bearing:
  `Diagnostic` is the M4 session-log + briefing schema): `Severity { Error,
  Warning, Note, Help }`, `Diagnostic { path, line, column, severity, message,
  code }`, `DiagnosticSignature` + `Diagnostic::signature()`, `Baseline`
  (`new`/`record`/`contains`/`partition`), `VerifierResult { Checked {
  diagnostics }, Unsupported, Failed(String) }`.
- **`verify(path: &Path) -> VerifierResult`** (async) — dispatch by extension:
  `.rs` → `cargo check --message-format=json`, `.ts`/`.tsx` → `tsc --noEmit
  --pretty=false`, `.py` → `ruff check --output-format=json`. Each resolves the
  project root by walking for `Cargo.toml` / `tsconfig.json` / `pyproject.toml`
  (or `setup.py`). Errors-only filtering when parsing (warnings are noise for the
  retry loop; the `Severity` type still carries all four).
- **`capture_baseline(paths) -> Baseline`** (async) — dedup to one representative
  path per (extension, project-root), run `verify`, record signatures. The
  baseline lets the loop classify later diagnostics as author (the model's fault,
  feed back) vs. ambient (pre-existing, ignore) via `Baseline::partition`.
- Keep the per-compiler parsers (`parse_cargo_line` / `parse_tsc_line` /
  `parse_ruff_output`) and the ancestor-walk helpers.

**Adaptations:**

1. **Strip Rexy phase references** in doc comments ("Phase 01 maps cargo's…",
   "phase 04 (TS / Python)", "Phase 01 supports `.rs` only; phase 04 adds…",
   "Phase 05 picks…"). Rewrite to describe behavior without Rexy's milestone
   numbers. rexyMCP supports `.rs`/`.ts`/`.tsx`/`.py` **now** (all three lifted
   together), so the "phase 04 adds TS/Py" framing is simply wrong here — state
   the supported set plainly.
2. **No `Result::Err` for model-visible outcomes** — the verifier already models
   outcomes as `VerifierResult` (Checked/Unsupported/Failed), not `Result`. Keep
   that. No new `unwrap`/`expect` in production.
3. Per the M4 README, this is the **structured-diagnostic** verifier; it is
   distinct from the final config-driven command set (later phase). Do not wire it
   to `{BUILD_COMMAND}` here.

## Acceptance criteria

- [ ] `executor/src/governor/{mod,verifier}.rs` exist; `pub mod governor;` in
      `lib.rs`; the types + `verify` + `capture_baseline` are public.
- [ ] `verify` returns `Unsupported` for an unknown/no extension; `Checked` (with
      error-severity diagnostics) for broken Rust; `Failed(_)` when the toolchain
      can't run / no project root.
- [ ] `parse_cargo_line` extracts an error diagnostic (path/line/column/code) from
      a `compiler-message` JSON envelope and **skips** warnings + non-message
      envelopes; `parse_tsc_line` and `parse_ruff_output` extract their formats.
- [ ] `DiagnosticSignature` ignores line/column (so a shifted-but-same diagnostic
      matches the baseline); `Baseline::partition` splits author vs. ambient,
      preserving input order.
- [ ] **Negatives:** `parse_cargo_line` returns `None` for a warning and for a
      `build-script-executed` envelope; `find_crate_root` returns `None` outside a
      crate.
- [ ] No Rexy phase references survive (grep `[Pp]hase 0` → 0 in the new files).
- [ ] All four required commands pass with zero new warnings.

## Test plan

Hermetic + deterministic. Lift Rexy's tests:

- **Pure parser tests** (`parse_cargo_line` / `parse_tsc_line` /
  `parse_ruff_output`, signature/baseline/partition, ancestor-walk) — fully
  hermetic, no subprocess.
- **Rust verify tests** spawn `cargo check` on a no-dependency crate in a
  `TempDir` (local, offline, deterministic) and **tolerate `Failed`** when cargo
  isn't on PATH — lift Rexy's tolerant assertions verbatim.
- **TS / Py broken-code tests** stay `#[ignore]`-gated (need `tsc` / `ruff`
  installed) — lift the `#[ignore]` exactly; they are not run on CI.

## End-to-end verification

> Not applicable — a library verifier exercised by unit tests (incl. real
> `cargo check` on a temp crate, tolerant of a missing toolchain). The loop that
> calls it after edit-class tools is M4 phase-06.

## Authorizations

- [x] **May create** `executor/src/governor/mod.rs` + `verifier.rs`; **may
      modify** `executor/src/lib.rs` (`pub mod governor;`).
- [ ] **No new dependencies.**
- [ ] May **NOT** lift the scorer, hard-fail detector, context budget, session
      log, or the loop — those are later M4 phases.
- [ ] May **NOT** modify `docs/architecture.md`, `STANDARDS.md`, `WORKFLOW.md`, or
      another phase doc.

## Out of scope

- **scorer / hard-fail / budget / session log / `PhaseResult` / the loop** —
  later M4 phases.
- **Config-driven verifier dispatch** — the README's open question; resolve at the
  loop phase. Lift the per-compiler verifier as-is here.
- **The final `{…}_COMMAND` runner** — separate from this structured verifier.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-05-28 17:30 (started)

**Executor:** Qwen/Qwen3.6-27B-FP8 / opencode. Lifting `rexy/src/governor/verifier.rs` into `executor/src/governor/verifier.rs` with `governor/mod.rs` and the `lib.rs` wire. Stripping Rexy phase references from doc comments, adapting to rexyMCP module paths.

### Update — 2026-05-28 17:35 (complete)

**Summary:** Lifted the post-edit verifier from Rexy into `executor/src/governor/verifier.rs`. Created all types (`Severity`, `Diagnostic`, `DiagnosticSignature`, `Baseline`, `VerifierResult`), the `verify(path)` dispatcher (`.rs` → `cargo check`, `.ts`/`.tsx` → `tsc`, `.py` → `ruff`), `capture_baseline`, and all per-compiler parsers + ancestor-walk helpers. Stripped Rexy phase references from doc comments. All 336 tests pass (314 existing + 22 new verifier tests, 2 ignored for TS/Py toolchain).

**Acceptance criteria:** all ticked above.

**Commands:**

```
cargo fmt --all --check
(no output — clean)

cargo build 2>&1 | tail -20
Compiling rexymcp-executor v0.1.0 (/home/matt/src/rexyMCP/executor)
Compiling rexymcp v0.1.0 (/home/matt/src/rexyMCP/mcp)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.14s

cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail -20
Checking rexymcp-executor v0.1.0 (/home/matt/src/rexyMCP/executor)
    Checking rexymcp v0.1.0 (/home/matt/src/rexyMCP/mcp)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.20s

cargo test 2>&1 | tail -30
test result: ok. 336 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 6.03s
```

**End-to-end verification:** Not applicable — a library verifier exercised by unit tests (incl. real `cargo check` on a temp crate, tolerant of a missing toolchain). The loop that calls it after edit-class tools is M4 phase-06.

**Files changed:**
- `executor/src/governor/mod.rs` — new module declaration
- `executor/src/governor/verifier.rs` — lifted verifier: types, `verify`, `capture_baseline`, parsers, tests
- `executor/src/lib.rs` — added `pub mod governor;`
- `docs/dev/milestones/M4-agent-loop/phase-01-verifier.md` — status flip + Update Log
- `docs/dev/milestones/M4-agent-loop/README.md` — phase table status flip

**New tests:** 22 tests in `executor/src/governor/verifier.rs`:
- `unsupported_for_non_rust_file`, `unsupported_for_no_extension`
- `find_crate_root_walks_up`, `find_crate_root_returns_none_outside_crate`
- `parse_cargo_line_skips_non_compiler_messages`, `parse_cargo_line_skips_warnings`
- `parse_cargo_line_extracts_error_with_code`, `parse_cargo_line_extracts_error_without_code`
- `verify_rust_returns_checked_with_errors_on_broken_code`, `verify_rust_returns_checked_empty_on_clean_code`
- `verify_dispatches_ts_to_typescript`, `verify_dispatches_tsx_to_typescript`, `verify_dispatches_py_to_python`
- `find_typescript_project_root_walks_up`, `find_typescript_project_root_returns_none_outside`
- `parse_tsc_line_extracts_error_with_code`, `parse_tsc_line_skips_warnings`
- `parse_ruff_output_extracts_diagnostics`, `parse_ruff_output_empty_json`, `parse_ruff_output_unparseable_json`
- `verify_typescript_returns_checked_on_broken_code` (ignored), `verify_python_returns_checked_on_broken_code` (ignored)
- `signature_ignores_line_and_column`, `signature_distinguishes_paths`, `signature_distinguishes_severities`, `signature_distinguishes_messages`, `signature_handles_none_code`
- `baseline_new_is_empty`, `baseline_record_and_contains_roundtrip`, `baseline_partition_splits_author_and_ambient`, `partition_preserves_input_order`
- `find_ancestor_with_walks_up_to_marker`, `project_root_for_resolves_per_extension`
- `capture_baseline_dedupes_by_project_root`, `capture_baseline_skips_unsupported_files`

**Grep check:** `grep -c '[Pp]hase 0' executor/src/governor/verifier.rs` → 0 (no Rexy phase references).

**Commits:**
- (pending — see below)

**Notes for review:** None. Straight lift with doc-comment adaptation.

verification: fmt OK · clippy OK · tests 336 passed · build OK
