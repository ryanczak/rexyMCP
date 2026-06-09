# Phase 04: Surface rustc machine-applicable suggested-fix spans

**Milestone:** M12 — Executor Tooling
**Status:** todo
**Depends on:** none (additive enrichment of the existing Rust verifier path)
**Estimated diff:** ~130 lines (≈45 production + ≈85 tests)
**Tags:** language=rust, kind=feature, size=s

## Goal

When `cargo check` reports an error, rustc often attaches a **machine-applicable
suggested fix** — an exact span + replacement text it is confident enough to
apply automatically (e.g. "add `mut ` at line 2:9"). The verifier today parses
the error message, line, column, and code, but **throws the suggestion away**.
This phase surfaces those suggestions to the model: when a rustc error carries a
machine-applicable `suggested_replacement`, the diagnostic the model sees gains a
line like *"rustc suggests (machine-applicable): replace at line 2:9 with
`mut `"*. That is a cheap, high-signal upgrade — the compiler is handing the
weak executor the precise edit, and we are currently discarding it.

The win is **precision filtering**: only `MachineApplicable` suggestions are
surfaced. rustc also emits `MaybeIncorrect`, `HasPlaceholders`, and `Unspecified`
suggestions — those are guesses (a `HasPlaceholders` fix literally contains
`REASON`/placeholder text). Surfacing those would mislead the model, so they are
**excluded**. That exclusion is the pinned correctness boundary of this phase
(Test plan), not an accident.

## Architecture references

Read before starting:

- `docs/architecture.md#status` — M12 Arc B, second bullet: "**Surface compiler
  suggested-fixes.** The verifier parses cargo JSON and has a `Help` severity but
  does not extract rustc's machine-applicable `suggested_replacement` spans;
  feeding those to the model ('rustc suggests X→Y at line N') is a cheap
  diagnostic upgrade."
- `docs/dev/STANDARDS.md` §2.2 — "No premature abstraction. Three similar lines
  are better than a generic helper." This phase deliberately does **not** add a
  structured `Suggestion` field to `Diagnostic` (see § "Why message-enrichment,
  not a new struct field" below) — it enriches the existing model-facing
  `message`. There is no consumer for structured suggestion data yet; per
  WORKFLOW § "Derive intentionally" we do not wire in state whose consumer
  doesn't exist.
- `docs/dev/WORKFLOW.md` § "Prefer additive change shapes" — the reason this
  phase is a single-file, single-function enrichment rather than a new
  `Diagnostic` field: a new field would break ~33 `Diagnostic { … }` struct
  literals across 9 files in both crates (the exact multi-site struct-literal
  churn that stalled the executor on M10 08a/08d). Message-enrichment touches
  **one function in one file** and breaks **zero** literals.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.
5. **(Optional, recommended) Confirm the live rustc JSON shape.** The three JSON
   fixtures in § "Reference: real rustc JSON" below were captured from a real
   `rustc --error-format=json` run on 2026-06-09 and are accurate. If you want to
   re-confirm the field names (`children[].spans[].suggested_replacement`,
   `suggestion_applicability`), run a broken snippet through
   `cargo check --message-format=json` yourself and compare. **Trust the live
   compiler output over this sketch** if they ever diverge, and note any
   divergence in "Notes for review". They will not diverge — the fixtures are
   real captures, not hand-written.

## Current state

Everything in this phase lives in **one file**:
`executor/src/governor/verifier.rs`. The single function you modify is
`parse_cargo_line` (lines 312–362), which parses one line of `cargo check
--message-format=json` stdout into an `Option<Diagnostic>`.

### The function you extend (the worked example)

`verifier.rs:312-362` — `parse_cargo_line`. It already has the `message` JSON
`Value` in scope (line 320). It extracts the error text at line 328:

```rust
fn parse_cargo_line(line: &str, crate_root: &Path) -> Option<Diagnostic> {
    let envelope: serde_json::Value = serde_json::from_str(line).ok()?;
    if envelope.get("reason")?.as_str()? != "compiler-message" {
        return None;
    }
    let message = envelope.get("message")?;
    let level = message.get("level")?.as_str()?;
    if level != "error" {
        return None;
    }
    let text = message.get("message")?.as_str()?.to_string();   // <-- the error text
    let code = message
        .get("code")
        .and_then(|c| c.get("code"))
        .and_then(|s| s.as_str())
        .map(String::from);

    // Find the primary span. (unchanged)
    let spans = message.get("spans")?.as_array()?;
    let primary = spans.iter().find(|s| {
        s.get("is_primary").and_then(|v| v.as_bool()).unwrap_or(false)
    })?;
    let file_name = primary.get("file_name")?.as_str()?;
    let line_start = primary.get("line_start")?.as_u64()? as u32;
    let column_start = primary
        .get("column_start").and_then(|v| v.as_u64()).map(|c| c as u32);
    let path = crate_root.join(file_name);

    Some(Diagnostic {
        path,
        line: line_start,
        column: column_start,
        severity: Severity::Error,
        message: text,            // <-- this string is what the model sees
        code,
    })
}
```

`message` (the error string) is rendered to the model verbatim by
`render_diagnostics` (`executor/src/agent/tools.rs:267`), which prints
`- {path}:{line}:{col} error: {message}`. **That render function is not changed
by this phase** — because we enrich `message` itself, the suggestion text reaches
the model (and the briefing, and the JSONL forensic record) for free, with no
plumbing.

### Where rustc puts machine-applicable suggestions

Not on the top-level error's spans (those carry `suggested_replacement: null`).
They live on the error message's **`children`** array — sub-diagnostics at level
`"help"` whose **spans** carry a string `suggested_replacement` **and**
`suggestion_applicability: "MachineApplicable"`. See the real fixtures below. The
help child's `message` (e.g. "consider changing this to be mutable") is the
human-readable rationale and should be included in the surfaced line.

## Why message-enrichment, not a new struct field

A `suggestions: Vec<Suggestion>` field on `Diagnostic` is the "obvious" shape and
is the **wrong** one here:

- `Diagnostic` is constructed by bare `Diagnostic { … }` struct literals at **~33
  sites across 9 files in both crates** (`verifier.rs`, `verifier_tests.rs` ×20,
  `agent/tests.rs`, `phase/briefing.rs`, `store/sessions/jsonl.rs` ×2,
  `mcp/log_query.rs`, `mcp/status.rs`, `mcp/cap.rs`). A new non-defaulted field
  breaks every one of them; the build stays red until all ~33 are patched. That
  is precisely the mechanical multi-site struct-literal churn that hard-failed the
  executor on M10 phase-08a (`IdenticalToolCallRepetition`) and phase-08d
  (`VerifierFailurePersistent`).
- There is **no consumer** for structured suggestion data — nothing queries it;
  the only reader is the render path, which reads text. Per WORKFLOW § "Derive
  intentionally," wiring in state whose consumer doesn't exist is dead structure.

Enriching the `message` string at parse time touches **one function**, breaks
**zero** literals, changes **zero** existing tests, and the suggestion flows
everywhere a diagnostic already flows. It is the additive shape WORKFLOW
prescribes.

**Baseline-signature note (must hold):** `DiagnosticSignature` (verifier.rs:47)
includes `message` and deliberately ignores `line`/`column`. The surfaced
suggestion line you append carries a line number *inside the message string*.
This is safe because (a) the baseline is captured by the **same parser** at
session start, so the same error yields the same enriched message → same
signature → still matches; and (b) you must **not** change `signature()` or the
`Diagnostic` struct, so every existing baseline test (which constructs literals
directly) is untouched. Do not append suggestions anywhere except inside
`parse_cargo_line`'s `text`.

## Spec

All changes are in `executor/src/governor/verifier.rs`.

1. **Add a suggestion-collecting helper.** Add a private function that walks a
   rustc diagnostic `message` JSON `Value` (and its `children`, recursively) and
   returns a `Vec<String>` of model-facing suggestion lines — one per
   machine-applicable suggestion span found. Sketch:

   ```rust
   /// Collect rustc machine-applicable suggested fixes from a compiler `message`
   /// object. rustc attaches these to `help` child diagnostics whose spans carry a
   /// string `suggested_replacement` and `suggestion_applicability ==
   /// "MachineApplicable"`. Returns one model-facing line per suggestion. Only
   /// MachineApplicable is surfaced — MaybeIncorrect / HasPlaceholders /
   /// Unspecified are guesses and are excluded.
   fn collect_machine_suggestions(message: &serde_json::Value) -> Vec<String> {
       let mut out = Vec::new();
       collect_suggestions_into(message, &mut out);
       out
   }

   fn collect_suggestions_into(node: &serde_json::Value, out: &mut Vec<String>) {
       let help = node.get("message").and_then(|m| m.as_str()).unwrap_or("");
       if let Some(spans) = node.get("spans").and_then(|s| s.as_array()) {
           for span in spans {
               if span.get("suggestion_applicability").and_then(|v| v.as_str())
                   != Some("MachineApplicable")
               {
                   continue;
               }
               let Some(replacement) =
                   span.get("suggested_replacement").and_then(|v| v.as_str())
               else {
                   continue;
               };
               let line = span.get("line_start").and_then(|v| v.as_u64()).unwrap_or(0);
               let loc = match span.get("column_start").and_then(|v| v.as_u64()) {
                   Some(col) => format!("line {line}:{col}"),
                   None => format!("line {line}"),
               };
               let rationale =
                   if help.is_empty() { String::new() } else { format!(" — {help}") };
               out.push(format!(
                   "rustc suggests (machine-applicable): replace at {loc} \
                    with `{replacement}`{rationale}"
               ));
           }
       }
       if let Some(children) = node.get("children").and_then(|c| c.as_array()) {
           for child in children {
               collect_suggestions_into(child, out);
           }
       }
   }
   ```

   Notes that are load-bearing:
   - The applicability check is `!= Some("MachineApplicable") → continue`. This
     is the whole correctness boundary. `MaybeIncorrect`, `HasPlaceholders`,
     `Unspecified`, and `null` (the location-only spans on the top-level error)
     all fail it and are skipped.
   - `suggested_replacement` is read as a JSON **string**. An empty string `""`
     is a valid suggestion (delete the span) and is **included** — `as_str()`
     returns `Some("")`, which is not `None`, so it surfaces. A `null`
     replacement is skipped (the `let … else` returns).
   - The line/column come from the **suggestion's own span** (where the fix
     applies — e.g. line 2, the `let v` binding), which is often a *different*
     location than the primary error span (line 3, the `.push` call). That is
     intended: the model needs to know where the *fix* goes.

2. **Append suggestions to the diagnostic message in `parse_cargo_line`.** After
   the primary span is resolved and before constructing the `Diagnostic`, collect
   suggestions and append each as its own indented line to `text`:

   ```rust
   let mut text = message.get("message")?.as_str()?.to_string();
   // ... (code, primary span, file_name, line_start, column_start, path as today) ...
   for suggestion in collect_machine_suggestions(message) {
       text.push_str("\n  ");
       text.push_str(&suggestion);
   }
   Some(Diagnostic {
       path,
       line: line_start,
       column: column_start,
       severity: Severity::Error,
       message: text,
       code,
   })
   ```

   (Make `text` `mut`. Structural placement of the loop — anywhere after `text` is
   bound and the function is committed to returning `Some` — is yours, but it must
   run only on the return path, so place it just before the `Some(Diagnostic …)`.)

   **When there are no machine-applicable suggestions, `text` is byte-identical to
   the raw compiler message** (the loop runs zero times). This is a pinned
   property: existing `parse_cargo_line` tests that assert `diag.message ==
   "<raw>"` must stay green unchanged.

3. **Do not touch anything else.** Not the `Diagnostic` struct, not
   `DiagnosticSignature`, not `signature()`, not `render_diagnostics`, not the
   `tsc`/`ruff` paths (suggestions are a rustc-only feature this phase), not any
   `SessionEvent` / dashboard / `log_query` code (this phase emits no event and
   adds no variant — ignore the stale phase-04 watch-item in the M12 README that
   refers to `SessionEvent::TaskUpdate`; that is phase-06).

## Reference: real rustc JSON

These three lines are **real, verbatim** `cargo`-envelope outputs (captured
2026-06-09 from `rustc --error-format=json`, wrapped in the
`{"reason":"compiler-message", …, "message":{…}}` envelope that `cargo check
--message-format=json` produces). Use them as test fixtures. They are valid JSON
on a single line — keep them on one line in the test (Rust raw strings `r#"…"#`
work well).

**Fixture A — E0596, `MachineApplicable` `mut ` at line 2:9 → MUST be surfaced:**

```json
{"reason":"compiler-message","target":{"name":"bad"},"message":{"message":"cannot borrow `v` as mutable, as it is not declared as mutable","code":{"code":"E0596"},"level":"error","spans":[{"file_name":"src/main.rs","line_start":3,"column_start":5,"is_primary":true,"suggested_replacement":null,"suggestion_applicability":null}],"children":[{"message":"consider changing this to be mutable","code":null,"level":"help","spans":[{"file_name":"src/main.rs","line_start":2,"column_start":9,"is_primary":true,"suggested_replacement":"mut ","suggestion_applicability":"MachineApplicable"}],"children":[]}]}}
```

**Fixture B — E0308, `HasPlaceholders` `.expect("REASON")` → MUST be excluded:**

```json
{"reason":"compiler-message","target":{"name":"t"},"message":{"message":"mismatched types","code":{"code":"E0308"},"level":"error","spans":[{"file_name":"src/main.rs","line_start":1,"column_start":51,"is_primary":true,"suggested_replacement":null,"suggestion_applicability":null}],"children":[{"message":"expected type `i32`","code":null,"level":"note","spans":[],"children":[]},{"message":"consider using `Option::expect` to unwrap the `Option<i32>` value, panicking if the value is an `Option::None`","code":null,"level":"help","spans":[{"file_name":"src/main.rs","line_start":1,"column_start":52,"is_primary":true,"suggested_replacement":".expect(\"REASON\")","suggestion_applicability":"HasPlaceholders"}],"children":[]}]}}
```

**Fixture C — E0425, `MaybeIncorrect` `foo` → MUST be excluded:**

```json
{"reason":"compiler-message","target":{"name":"t"},"message":{"message":"cannot find value `fou` in this scope","code":{"code":"E0425"},"level":"error","spans":[{"file_name":"src/main.rs","line_start":1,"column_start":31,"is_primary":true,"suggested_replacement":null,"suggestion_applicability":null}],"children":[{"message":"a local variable with a similar name exists","code":null,"level":"help","spans":[{"file_name":"src/main.rs","line_start":1,"column_start":31,"is_primary":true,"suggested_replacement":"foo","suggestion_applicability":"MaybeIncorrect"}],"children":[]}]}}
```

For Fixture A the parsed `Diagnostic.message` must begin with the raw error
`cannot borrow \`v\` as mutable, as it is not declared as mutable` and then
contain a line carrying ``mut`` and `line 2`. For Fixtures B and C the parsed
`Diagnostic.message` must equal the raw error string exactly — **no** suffix —
because the only suggestions present are non-machine-applicable.

## Acceptance criteria

- [ ] A `cargo check` error line carrying a `MachineApplicable`
      `suggested_replacement` (Fixture A) parses to a `Diagnostic` whose
      `message` contains the replacement text (`` `mut ` ``) and the suggestion's
      span location (`line 2`), in addition to the original error text.
- [ ] A `HasPlaceholders` suggestion (Fixture B) is **not** surfaced — the parsed
      `message` equals the raw error string (`"mismatched types"`), no suffix.
- [ ] A `MaybeIncorrect` suggestion (Fixture C) is **not** surfaced — the parsed
      `message` equals the raw error string
      (`"cannot find value \`fou\` in this scope"`), no suffix.
- [ ] An error line with no `children` / no suggestion spans parses to a
      `message` byte-identical to the raw compiler message (existing
      `parse_cargo_line_extracts_error_with_code` /
      `parse_cargo_line_extracts_error_without_code` tests pass unchanged).
- [ ] The `Diagnostic` struct, `DiagnosticSignature`, `signature()`, and
      `render_diagnostics` are unchanged; **no** `Diagnostic { … }` literal
      anywhere in the workspace is modified; all existing verifier/baseline tests
      pass unchanged.
- [ ] `cargo build` (zero new warnings), `cargo clippy --all-targets
      --all-features -- -D warnings`, `cargo fmt --all --check`, `cargo test` all
      pass.

## Test plan

New `#[test]`s in the existing `mod tests` block in
`executor/src/governor/verifier_tests.rs`, mirroring the existing
`parse_cargo_line_*` test style (a raw-string JSON line + `parse_cargo_line(line,
dir.path())`). Names are pinned; placement/count details are yours.

- `cargo_line_surfaces_machine_applicable_suggestion` — Fixture A → the returned
  `diag.message` `contains("mut")` **and** `contains("line 2")` **and**
  `starts_with("cannot borrow")` (original error preserved).
- `cargo_line_excludes_has_placeholders_suggestion` — Fixture B →
  `diag.message == "mismatched types"` (no suffix appended). This is a pinned
  negative case — make the `==` assertion exact so a "surface everything" impl
  fails it.
- `cargo_line_excludes_maybe_incorrect_suggestion` — Fixture C →
  `diag.message == "cannot find value `fou` in this scope"` (no suffix).
- `cargo_line_without_children_message_unchanged` — an error line with no
  `children` key at all (reuse the shape of
  `parse_cargo_line_extracts_error_with_code`) → `diag.message` equals the raw
  error string exactly. (Guards the zero-suggestion path explicitly.)
- *(Optional, if cheap)* `cargo_line_surfaces_multiple_suggestions` — a
  hand-built line whose message has two `MachineApplicable` help children → the
  `message` contains both replacement texts. Skip if it bloats the phase.

## End-to-end verification

> Not applicable — phase ships no runtime-loadable CLI/binary artifact. The
> verifier runs only inside a full `execute_phase` local-LLM loop, which cannot
> be exercised hermetically. **However**, note for the reviewer: the unit tests
> above are *de facto* end-to-end — they feed **real, verbatim rustc
> `--error-format=json` output** (captured 2026-06-09, see § "Reference") to the
> real `parse_cargo_line`. There is no hand-written or mocked compiler output in
> the path; the fixtures are exactly what `cargo check` emits. Quote the
> `cargo_line_surfaces_machine_applicable_suggestion` and
> `cargo_line_excludes_has_placeholders_suggestion` parsed `message` values in the
> completion Update Log.

## Authorizations

None. No new dependencies (`serde_json` is already in `executor/Cargo.toml`; this
phase adds no shell-out — it parses JSON already captured by the existing `cargo
check` invocation, so there is **no new runtime toolchain binary** and no
toolchain-availability concern). No `docs/architecture.md` edit. No files outside
`executor/src/governor/verifier.rs` (production) and
`executor/src/governor/verifier_tests.rs` (tests).

## Out of scope

- **No new `Diagnostic` field and no new `Suggestion` struct.** Enrich the
  existing `message`. (See § "Why message-enrichment, not a new struct field.")
  If you believe a struct field is necessary, stop and file a blocker — do not
  add it and patch 33 literals.
- **No change to `render_diagnostics`, the `Diagnostic` struct,
  `DiagnosticSignature`, or `signature()`.**
- **No `tsc` / `ruff` suggestions.** This is a rustc-only feature; the `tsc` and
  `ruff` parse paths are untouched.
- **No `SessionEvent`, dashboard, or `log_query` changes.** This phase emits no
  event and adds no enum variant. (The M12 README's phase-04 watch-item about
  `SessionEvent::TaskUpdate` is stale/misnumbered — that belongs to phase-06.)
- **No surfacing of `warning`-level suggestions.** The verifier feeds only
  `error`-level diagnostics to the model today (verifier.rs:324); that gate is
  unchanged, so only errors' suggestions surface.
- **Not the other Arc B phases.** No structured test-failure parsing (phase-05),
  no task tracking (phase-06/07).
- **No new dependency, no `unsafe`, no `#[allow]`.** If you think you need any,
  stop and file a blocker.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
