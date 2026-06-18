# Phase 01: `patch` no-op recovery context

**Milestone:** M24 — Edit-Loop Recovery
**Status:** todo
**Depends on:** none
**Estimated diff:** ~95 lines
**Tags:** language=rust, kind=bugfix, size=s

## Goal

When a `patch` call has `old_str == new_str`, the tool currently returns a
dead-end error — `no-op patch: old_str equals new_str` — that tells the model
*what* is wrong but nothing it can act on. A MEDIUM-tier model that hit this
re-emitted the identical call six times until the governor halted the run (see §
"Why this phase"). Replace that flat string with a recovery-oriented message: show
**where** the text already lives in the file (a line-numbered context window),
flag when it appears **more than once** (the duplicate tell), and name the **next
step** (`read_file`, then move on). Move the no-op check below the file read so the
message can be built from the file's actual content.

## Why this phase

From `session-phase-03-6a342a42.jsonl` (netviz e2e, `google/gemma-4-26b-a4b-qat`):
turn 3 patched `const ETHERTYPE_IPV4 = 0x0800;` to insert `ETHERTYPE_IPV6` +
`IPV6_HEADER_LENGTH` after it — but those constants already existed lower in the
file, so the edit introduced duplicates. Turns 5–10 tried to fix this by patching
the 5-constant block, but each call sent a `new_str` byte-identical to its
`old_str`, so every call was a no-op:

```
Turn 5  patch  old_str == new_str  →  "no-op patch: old_str equals new_str"
Turn 6  patch  (identical call)    →  "no-op patch: old_str equals new_str"
...
Turn 10 hard_fail: identical patch call repeated 6 times
```

The error gave the model no new information, so it could not break the loop. The
governor's `IdenticalToolCallRepetition` stall (the backstop) fired three turns
later. This phase makes the **tool message** carry the context the model needs to
recover on turn 6 instead.

## Architecture references

Read before starting:

- `executor/src/tools/patch.rs` — the whole file. The pieces this phase touches:
  - The **early no-op guard** (lines **82–88**): `if parsed.old_str ==
    parsed.new_str { … "no-op patch: old_str equals new_str" … }`. This block is
    **removed** and re-added lower, after the file is read.
  - The **file read** (lines **117–133**): `let content = match
    std::fs::read_to_string(&path) { … }`. The relocated no-op check goes
    **after** this and after `let match_count` (line **135**), immediately before
    the `match match_count {` at line **137**.
  - The **`match match_count`** arms (**137–190**): unchanged. With the no-op
    check relocated above them, the `1 =>` apply arm is still only reached when
    `old_str != new_str`, so `replacen` always makes a real change (no behavior
    change in that arm).
  - The **`fuzzy_hint` helper** (**194–233**): the windowed-context shape to
    mirror. The new `noop_hint` helper lives next to it. Note `fuzzy_hint` joins
    its window with `lines[a..b].join("\n")` (no line-number gutter); `noop_hint`
    adds a `"{lineno:>4} | {line}"` gutter so the model sees absolute line
    numbers.
  - The existing test `rejects_identical_old_and_new` (**389+**) — must pass
    unmodified (it asserts the error `contains("no-op")`; the new message still
    starts with `no-op patch`).

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read `executor/src/tools/patch.rs` end to end (it is short).
3. Read this entire phase doc before touching code.
4. Confirm `cargo build` and `cargo test` exit 0 (no pre-existing failures).

## Current state

### The early no-op guard returns a dead-end string — `patch.rs:82–88`

```rust
        if parsed.old_str == parsed.new_str {
            return Ok(ToolResult {
                output: String::new(),
                error: Some("no-op patch: old_str equals new_str".to_string()),
                metadata: None,
            });
        }
```

This runs **before** the path is resolved or the file is read, so it cannot
reference the file's content. The fix relocates it below the read.

### The file read and match count — `patch.rs:117–137`

```rust
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            // … UTF-8 / read-error arms …
        };

        let match_count = content.matches(&parsed.old_str).count();

        match match_count {
```

The relocated no-op check goes between `let match_count = …;` and `match
match_count {`.

### `fuzzy_hint` — the windowed-context precedent — `patch.rs:194+`

```rust
fn fuzzy_hint(path: &str, content: &str, old_str: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    // … finds the best-matching window, returns it joined with "\n" …
}
```

`noop_hint` is the same idea but for an **exact** match (the text *is* present, so
`content.find(old_str)` locates it) with a line-number gutter.

## Spec

### Task 1 — Remove the early no-op guard in `patch.rs`

Delete the block at lines **82–88** (the `if parsed.old_str == parsed.new_str { …
}` that returns `"no-op patch: old_str equals new_str"`). Leave the
`old_str.is_empty()` guard above it (lines 74–80) **unchanged** — an empty
`old_str` is still rejected first, before any file I/O.

**Precedence note (intentional):** moving this check below the path-resolution and
file-read steps means a no-op patch targeting a *missing* file now reports `file
not found` instead of `no-op …`. That ordering is correct — the no-op message's
value is showing the file's current content, which requires the file to exist. No
existing test pins the old precedence (`rejects_missing_file` uses a non-no-op
`old_str`/`new_str`).

### Task 2 — Re-add the no-op check after the file read in `patch.rs`

Insert, between `let match_count = content.matches(&parsed.old_str).count();`
(line 135) and `match match_count {` (line 137):

```rust
        if parsed.old_str == parsed.new_str {
            return Ok(ToolResult {
                output: String::new(),
                error: Some(noop_hint(&parsed.path, &content, &parsed.old_str)),
                metadata: None,
            });
        }
```

This sits above the `match match_count` arms, so the `1 =>` apply arm is now only
reached when `old_str != new_str` (a real edit) — no change to that arm's logic.

### Task 3 — Add the `noop_hint` helper in `patch.rs`

Add next to `fuzzy_hint` (a free `fn` in the same module, below the `impl`):

```rust
fn noop_hint(path: &str, content: &str, old_str: &str) -> String {
    let preamble =
        "no-op patch: old_str and new_str are identical, so this patch would change nothing.";

    let Some(offset) = content.find(old_str) else {
        return format!(
            "{preamble}\n\nold_str was not found in {path}, so the file does not \
             currently contain this text. Call read_file to see the current content \
             before patching."
        );
    };

    let lines: Vec<&str> = content.lines().collect();
    let start_line = content[..offset].bytes().filter(|&b| b == b'\n').count() + 1;
    let span = old_str.lines().count().max(1);
    let end_line = start_line + span - 1;

    let ctx_start = start_line.saturating_sub(2).max(1);
    let ctx_end = (end_line + 2).min(lines.len());

    let window: String = lines
        .iter()
        .enumerate()
        .filter(|(idx, _)| {
            let lineno = idx + 1;
            lineno >= ctx_start && lineno <= ctx_end
        })
        .map(|(idx, line)| format!("{:>4} | {line}\n", idx + 1))
        .collect();

    let occurrences = content.matches(old_str).count();
    let multiplicity = if occurrences > 1 {
        format!(
            "\n\nNote: this text appears {occurrences} times in {path}. If you meant \
             to remove a duplicate, give old_str a larger window that includes the \
             unique lines around the copy you want to change."
        )
    } else {
        String::new()
    };

    format!(
        "{preamble}\n\nThe file already contains this exact text at \
         {path}:{start_line}-{end_line}:\n{window}\nTo make an edit, old_str and \
         new_str must differ. If the file already has the content you intended, no \
         patch is needed — re-read the file with read_file to confirm the current \
         state, then move on to the next step.{multiplicity}"
    )
}
```

Notes for the implementer:
- `content.find(old_str)` returns a **byte** offset at a char boundary (the start
  of the match), so `content[..offset]` never splits a multi-byte character — no
  UTF-8 panic. (The same char-safety concern the codebase just fixed in
  `feedback.rs`.)
- `start_line` counts the newlines before the match and adds 1; `span` is the
  line count of `old_str` (at least 1, so a single-line match spans one line).
- Build the window by `filter`+`map`+`collect` (as above), not a `push_str` loop —
  it avoids needing `use std::fmt::Write` and is the cleaner idiom.
- The exact wording above is a guide; keep the four load-bearing elements the
  tests pin: the `no-op` preamble, the `path:start-end` location, the
  line-numbered window, and the `read_file` next-step. Keep the multiplicity note
  for the `occurrences > 1` case.

### Task 4 — Tests in `patch.rs` (`#[cfg(test)] mod tests`)

Add these. Construct the real `Patch` tool over a `TempDir`-scoped `Scope` exactly
as the existing tests do (`make_scope`).

- `noop_patch_shows_location_and_context_window` — write a multi-line file; call
  `patch` with `old_str == new_str` equal to one line that appears **once**.
  Assert the error contains `"no-op"`, the file's `path` with a `:N-` line
  reference, the matched line's text inside a gutter (`" | "`), and `"read_file"`.
  Mutation-resistant: pins that the location + window are shown, not just the flat
  `no-op` string.
- `noop_patch_flags_duplicate_occurrences` — write a file containing the same line
  **twice**; call `patch` with `old_str == new_str` equal to that line. Assert the
  error contains `"2 times"` (the multiplicity note). This is the direct
  duplicate-introduction signal.
- `noop_patch_when_old_str_absent_suggests_read_file` — write a file that does
  **not** contain the text; call `patch` with `old_str == new_str` set to the
  absent text. Assert the error contains `"not found"` and `"read_file"`, does
  **not** contain a `" | "` gutter, and the call does not panic.

## Acceptance criteria

- [ ] The early `old_str == new_str` block (old `patch.rs:82–88`) is gone; the
      no-op check now runs after the file is read (`grep -n 'no-op patch: old_str
      equals new_str' executor/src/tools/patch.rs` returns nothing — the old flat
      string is replaced).
- [ ] A no-op `patch` whose `old_str` is present returns the `path:start-end`
      location and a line-numbered context window.
- [ ] A no-op `patch` whose `old_str` appears > 1 time includes an
      occurrence-count note.
- [ ] A no-op `patch` whose `old_str` is absent returns a "not found … read_file"
      message with no fabricated location.
- [ ] `rejects_identical_old_and_new` passes unmodified (the new message still
      starts with `no-op patch`).
- [ ] The governor and `patch_lines` are untouched.
- [ ] `cargo fmt --all --check`, `cargo build` (zero new warnings), `cargo clippy
      --all-targets --all-features -- -D warnings`, `cargo test` all exit 0.

## Test plan

- `noop_patch_shows_location_and_context_window` — mutation-resistant: a flat
  `no-op` string (the old behavior) fails the location/window assertions.
- `noop_patch_flags_duplicate_occurrences` — pins the `occurrences > 1` branch;
  removing the multiplicity note fails it.
- `noop_patch_when_old_str_absent_suggests_read_file` — pins the `content.find`
  `None` branch and guards against a panic / fabricated location.
- `rejects_identical_old_and_new` (existing) — passes unmodified; pins backward
  compatibility of the `no-op` prefix.

## End-to-end verification

The unit tests invoke the **real** `Patch` tool (the same `Tool::execute` the
agent loop dispatches) over a `TempDir`-scoped `Scope` — the tool *is* the
runtime artifact; there is no separate CLI/binary wrapper to verify. The error
string the tests assert on is byte-for-byte what the executor loop injects back to
the model. Quote one rendered no-op error (location + window + multiplicity note)
in the completion Update Log.

## Authorizations

None. No new dependency, no `Cargo.toml` edit, no `docs/architecture.md` edit
(the architect added the § Status #24 entry at kickoff).

## Out of scope

- **`patch_lines`** — its range-based errors are separate and were not implicated;
  do not touch it.
- **The `0 =>` (fuzzy_hint) and `n =>` (ambiguous) arms** — they already return
  actionable context; leave their messages alone.
- **The governor / `IdenticalToolCallRepetition` stall** — it stays the backstop;
  this phase only enriches the upstream signal. No `governor` change.
- **Auto-detecting or auto-removing duplicates** — the tool surfaces the
  occurrence count; it does not edit on the model's behalf.
- **The success-path output or `metadata`** — unchanged.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
