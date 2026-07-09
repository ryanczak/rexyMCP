# Phase 1: Actionable missing-field recovery hint for `write_file` + `patch`

**Milestone:** M28 — Edit-Tool Arg Recovery
**Status:** todo
**Depends on:** none
**Estimated diff:** ~140 lines
**Tags:** language=rust, kind=bugfix, size=m

## Goal

Replace the dead-end raw `invalid arguments: missing field \`path\`` error that
`write_file` and `patch` surface when a small model truncates a tool call near max
context ([issue #1](https://github.com/ryanczak/rexyMCP/issues/1)) with an
**actionable recovery message** the model can act on: it names the missing
required field(s), echoes which required fields *were* supplied (breadcrumbs), and
gives an example shape + next-step. Deterministic and message-only — it does
**not** guess the missing `path` value.

## Architecture references

Read before starting:

- `docs/architecture.md` § Status #24 (M24 — Edit-Loop Recovery) — the precedent:
  a dead-end tool error enriched into a model-visible recovery message.
- `executor/src/tools/update_task.rs:26,35` — the `advisory()` + `invalid_args_hint()`
  worked example this phase mirrors (quoted below).

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

Both edit tools deserialize their args with `serde_json::from_value` and, on
failure, surface the **raw serde error** verbatim. This is the entire bug.

`executor/src/tools/write_file.rs:16-21` (the args struct) and `:58-68` (the
failing arm):

```rust
#[derive(Deserialize)]
struct WriteFileArgs {
    path: String,
    content: String,
    append: Option<bool>,
}
// ...
    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let parsed = match serde_json::from_value::<WriteFileArgs>(args) {
            Ok(a) => a,
            Err(e) => {
                return Ok(ToolResult {
                    output: String::new(),
                    error: Some(format!("invalid arguments: {e}")),
                    metadata: None,
                });
            }
        };
```

`executor/src/tools/patch.rs:19-24` and `:62-72` are the identical shape
(`PatchArgs { path, old_str, new_str }`; same `Err(e) => "invalid arguments: {e}"`).

The **worked example to mirror** — `update_task.rs:26` and `:35`:

```rust
fn advisory(msg: &str) -> ToolResult {
    ToolResult { output: String::new(), error: Some(msg.to_string()), metadata: None }
}
// ...
    fn invalid_args_hint(&self) -> String {
        // ... builds an actionable string naming the fields + an example shape ...
        format!(
            "update_task: missing or invalid arguments. Call it with both \
             fields — e.g. {{\"id\": \"2\", \"state\": \"done\"}}. `id` is a \
             Spec item number (a string); `state` is one of: active, done, \
             pending. {tail}"
        )
    }
// ...
        let parsed = match serde_json::from_value::<UpdateTaskArgs>(args) {
            Ok(a) => a,
            Err(_) => return Ok(advisory(&self.invalid_args_hint())),
        };
```

## Spec

### 1. Add a shared `missing_args_hint` helper

In `executor/src/tools/registry.rs` (already imported by every tool via
`super::registry`), add a `pub(crate)` free function:

```rust
pub(crate) fn missing_args_hint(tool: &str, required: &[&str], present: &[&str]) -> String
```

Behavior (pin this, not the exact wording):

- `missing` = the `required` fields **not** in `present`.
- **When `missing` is non-empty** (the truncation case): return a message that
  (a) names the `tool`, (b) lists the missing field(s), (c) echoes what *was*
  supplied (`present`, or "(none)"), (d) gives an example shape built from
  `required` (e.g. `{"path": "<string>", "content": "<string>"}`), and (e) a
  next-step: re-emit with all required fields, and if the content is large and the
  call keeps truncating, make a smaller `patch` edit instead. Example rendering
  (wording is the executor's; the **content** is pinned):

  > `write_file: could not parse arguments — missing required field(s): path. You supplied: content. Re-send with all required fields, e.g. {"path": "<string>", "content": "<string>"}. If the content is large and calls keep truncating, make a smaller patch edit instead.`

- **When `missing` is empty** (all required keys present but deserialization still
  failed — a type/value mismatch, not truncation): return a message that says the
  required fields are present but one has an invalid type/value, and gives the
  example shape to re-check against. Do **not** embed the raw serde error.

- Must **not** panic on any input (all args are `&str` slices; no unwrap).

Add unit tests for the helper directly (see Test plan).

### 2. Rewrite the `write_file` failing arm to use the helper

In `write_file.rs`, compute the present required keys from `&args` **before** the
`from_value` move, then use the helper on failure. **Gotcha — do NOT clone
`args`:** `content` can be very large; cloning the whole `Value` to inspect it
after the move would be wasteful. Capture only the tiny present-key list first:

```rust
    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let required = ["path", "content"];
        let present: Vec<&str> = args
            .as_object()
            .map(|m| required.iter().copied().filter(|k| m.contains_key(*k)).collect())
            .unwrap_or_default();
        let parsed = match serde_json::from_value::<WriteFileArgs>(args) {
            Ok(a) => a,
            Err(_) => {
                return Ok(ToolResult {
                    output: String::new(),
                    error: Some(missing_args_hint("write_file", &required, &present)),
                    metadata: None,
                });
            }
        };
```

The rest of `execute` is unchanged. Import `missing_args_hint` from
`super::registry`.

### 3. Rewrite the `patch` failing arm identically

In `patch.rs`, the same rewrite with `required = ["path", "old_str", "new_str"]`.
Everything after the deserialization is unchanged.

## Acceptance criteria

- [ ] `cargo build` (zero new warnings), `clippy`, `fmt --check`, and `cargo test`
      all pass.
- [ ] A `write_file` call with args `{"content": "x"}` (no `path`) returns an
      error naming `path` as missing **and** `content` as supplied — not the bare
      `invalid arguments: missing field \`path\`` string.
- [ ] A `patch` call missing `path` returns an error naming `path` missing and the
      supplied fields.
- [ ] A **valid** `write_file` (`{"path": "...", "content": "..."}`) and a valid
      `patch` still succeed unchanged (happy path byte-identical).
- [ ] Non-object args (e.g. `json!("oops")` / `json!(5)`) return the hint with all
      required fields listed as missing and `present` = "(none)" — **no panic**.
- [ ] The message never fabricates a `path` value and never auto-retries.
- [ ] `grep -n "invalid arguments" executor/src/tools/write_file.rs
      executor/src/tools/patch.rs` returns **nothing** (both raw arms removed).

## Test plan

Unit tests. For the helper, in the `#[cfg(test)] mod tests` of `registry.rs`
(or alongside it); for the tools, in each tool's existing test module against a
`TempDir` scope (mirror the existing `write_file`/`patch` tool tests).

- `missing_args_hint_names_missing_and_present` — `missing_args_hint("write_file",
  &["path","content"], &["content"])` contains `"path"` (missing) and `"content"`
  (supplied); a **negative**: it does **not** contain the word `path` in the
  supplied list (i.e. the two lists are distinct — assert `path` appears in the
  missing clause, not the supplied clause).
- `missing_args_hint_all_present_reports_type_mismatch` — with `present ==
  required`, the message indicates the fields are present but malformed (no
  "missing required field" claim), and contains **no** raw serde text.
- `missing_args_hint_empty_present_says_none` — `present == []` renders a "(none)"
  supplied clause without panicking.
- `write_file_missing_path_returns_recovery_hint` — `WriteFile::execute(json!({
  "content": "x" }))` → `error` names `path` and mentions `content` supplied; is
  **not** the string `"invalid arguments: missing field"`.
- `write_file_valid_args_unaffected` — a valid create still writes the file and
  returns success (guards the happy path).
- `write_file_non_object_args_do_not_panic` — `execute(json!("oops"))` returns an
  `error` (hint), does not panic, writes nothing.
- `patch_missing_path_returns_recovery_hint` — analogous for `patch`.
- `patch_valid_args_unaffected` — a valid patch still applies.

Make the hint assertions mutation-resistant: assert on the **distinct** presence
of the missing field in the missing clause AND the supplied field in the supplied
clause, so a helper that mislabels the two lists fails.

## End-to-end verification

> Not applicable as a standalone CLI/binary E2E — `write_file` and `patch` have no
> command surface; they are exercised only through `execute_phase`. The unit tests
> above call the **real** `WriteFile::execute` / `Patch::execute` methods against a
> `TempDir` scope, which is the shipped code path (not a fake). The ultimate
> confirmation — a live max-context dispatch reproducing issue #1 and observing the
> recovery — is non-hermetic (needs a near-full context on a real model) and is
> out of the gate; note in the completion entry that the unit tests exercise the
> shipped `execute` path directly.

## Authorizations

None. (No new dependency; no `docs/architecture.md` edit — the milestone is
already recorded in § Status.)

## Out of scope

- The other **8** tools that surface `invalid arguments: {e}` (`patch_lines`,
  `move_file`, `delete_file`, `bash`, `search`, `find_files`, `symbols`,
  `read_file`) — a possible phase-02 reusing this helper. Do **not** touch them.
- **Auto-reconstructing** the missing `path` from context/breadcrumbs (issue #1
  solution 1) — risks writing to the wrong file; explicitly excluded.
- **Context-pressure guards** biasing toward smaller edits when budget is low
  (issue #1 solution 3) — a loop-behavior change, not a tool-error change.
- Changing the tools' `schema()` / `required` JSON or the args structs — the
  fields stay required; only the *failure message* changes.

## Update Log

(Filled in by the executor.)

<!-- entries appended below this line -->
