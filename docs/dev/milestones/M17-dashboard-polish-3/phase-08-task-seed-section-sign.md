# Phase 08: `seed_from_spec` — recognise `§N` Spec headings

**Milestone:** M17 — Dashboard Polish (Round 3)
**Status:** todo
**Depends on:** none (pure addition to `executor/src/agent/tasks.rs`)
**Estimated diff:** ~50 lines across 1 file
**Tags:** language=rust, kind=fix, size=xs

## Goal

`parse_heading_task_line` in `executor/src/agent/tasks.rs` already handles two
Spec-heading formats (`### N. Title` and `### Task N — Title`). Architect-authored
phase docs consistently use a third format — `### §N — Title` (§ is U+00A7 SECTION
SIGN) — which the parser does not recognise. When a phase doc's Spec uses `§`
headings, `seed_from_spec` returns an empty task list, the Tasks panel shows nothing,
and every `update_task` call by the executor fails with `"no task with id …"`.

This phase adds a `§N` branch to `parse_heading_task_line` alongside the two
existing branches — no other file changes, no new dependency.

## Architecture references

Read before starting:

- `executor/src/agent/tasks.rs:89–127` — `parse_heading_task_line`, full body
  (quoted in Current state below).
- `executor/src/agent/tasks.rs:277–294` — `parse_heading_task_line_accepts_task_prefix_separators`
  test (the test to mirror for the `§` branch).

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read this entire phase doc before touching any code.
3. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

`parse_heading_task_line` at `executor/src/agent/tasks.rs:92–127`:

```rust
fn parse_heading_task_line(line: &str) -> Option<Task> {
    let rest = line.trim().strip_prefix("### ")?;

    // `### Task N <sep> Title` where <sep> is em-dash, colon, or dot.
    if let Some(after_task) = rest.strip_prefix("Task ") {
        let (digits, title_part) = after_task.split_once(['\u{2014}', ':', '.'])?;
        let digits = digits.trim();
        let title = title_part.trim();
        if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        if title.is_empty() {
            return None;
        }
        return Some(Task {
            id: digits.to_string(),
            title: title.to_string(),
            state: TaskState::Pending,
        });
    }

    // `### N. Title` (unchanged — dot separator, whitespace-after-dot rejects
    // decimals like `### 1.5x`).
    let (digits, title_part) = rest.split_once('.')?;
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    if !title_part.starts_with(' ') && !title_part.starts_with('\t') {
        return None;
    }
    Some(Task {
        id: digits.to_string(),
        title: title_part.trim().to_string(),
        state: TaskState::Pending,
    })
}
```

## Spec

### §1 — Add the `§N` branch to `parse_heading_task_line`

Insert the new branch immediately before the `"Task "` branch (i.e., as the
first `if let` inside the function body, after `strip_prefix("### ")`). The
`§` branch uses the same structure as the `Task N` branch: strip the prefix,
`split_once` on the same three separators, validate digits and title, return a
`Task`. **Do not modify the `Task N` or `N.` branches.**

```rust
fn parse_heading_task_line(line: &str) -> Option<Task> {
    let rest = line.trim().strip_prefix("### ")?;

    // `### §N <sep> Title` where `§` is U+00A7 (SECTION SIGN) and <sep> is
    // em-dash, colon, or dot — the architect's natural §-numbered section style.
    if let Some(after_sign) = rest.strip_prefix('§') {
        let (digits, title_part) = after_sign.split_once(['\u{2014}', ':', '.'])?;
        let digits = digits.trim();
        let title = title_part.trim();
        if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        if title.is_empty() {
            return None;
        }
        return Some(Task {
            id: digits.to_string(),
            title: title.to_string(),
            state: TaskState::Pending,
        });
    }

    // `### Task N <sep> Title` where <sep> is em-dash, colon, or dot.
    if let Some(after_task) = rest.strip_prefix("Task ") {
        // ... unchanged ...
    }

    // `### N. Title` — unchanged.
    // ...
}
```

In the actual file, write the full function — do not leave `// ... unchanged ...`
placeholders. The `Task N` and `N.` branches are already present; copy them
through verbatim from `executor/src/agent/tasks.rs:96–127`. The only addition is
the `§` branch above them.

## Acceptance criteria

- [ ] `cargo build` succeeds with zero warnings.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes.
- [ ] `cargo fmt --all --check` passes.
- [ ] `cargo test` passes (existing tests + 3 new ones).
- [ ] `parse_heading_task_line("### §3 — Some title")` returns a `Task` with
      `id == "3"` and `title == "Some title"`.
- [ ] `parse_heading_task_line("### §3 — ")` (empty title) returns `None`.
- [ ] `parse_heading_task_line("### §one — Title")` (non-numeric) returns `None`.
- [ ] `parse_heading_task_line("### §3 no separator")` (missing sep) returns `None`.
- [ ] `seed_from_spec` on a doc whose `## Spec` uses `§N —` headings returns one
      task per heading with the correct id and title.

## Test plan

Add three tests in the existing `#[cfg(test)] mod tests` block at the bottom of
`executor/src/agent/tasks.rs`. Place them after the existing
`parse_heading_task_line_rejects_malformed_task_prefix` test (line ~303).

### Test shape to mirror

The existing `parse_heading_task_line_accepts_task_prefix_separators` test
(lines 277–294) is the direct analogue — copy its structure exactly:

```rust
#[test]
fn parse_heading_task_line_accepts_task_prefix_separators() {
    let dash = parse_heading_task_line("### Task 4 — Em dash title").unwrap();
    assert_eq!(dash.id, "4");
    assert_eq!(dash.title, "Em dash title");

    let colon = parse_heading_task_line("### Task 5: Colon title").unwrap();
    assert_eq!(colon.id, "5");
    assert_eq!(colon.title, "Colon title");

    let dot = parse_heading_task_line("### Task 6. Dot title").unwrap();
    assert_eq!(dot.id, "6");
    assert_eq!(dot.title, "Dot title");

    let ten = parse_heading_task_line("### Task 10 — Tenth").unwrap();
    assert_eq!(ten.id, "10");
}
```

### New tests

```rust
#[test]
fn parse_heading_task_line_accepts_section_sign_separators() {
    // em-dash (the architect's standard)
    let dash = parse_heading_task_line("### §1 — Em dash title").unwrap();
    assert_eq!(dash.id, "1");
    assert_eq!(dash.title, "Em dash title");

    // colon separator
    let colon = parse_heading_task_line("### §2: Colon title").unwrap();
    assert_eq!(colon.id, "2");
    assert_eq!(colon.title, "Colon title");

    // dot separator
    let dot = parse_heading_task_line("### §3. Dot title").unwrap();
    assert_eq!(dot.id, "3");
    assert_eq!(dot.title, "Dot title");

    // multi-digit section number
    let ten = parse_heading_task_line("### §10 — Tenth").unwrap();
    assert_eq!(ten.id, "10");
    assert_eq!(ten.title, "Tenth");
}

#[test]
fn parse_heading_task_line_rejects_malformed_section_sign() {
    // Empty title after separator.
    assert!(parse_heading_task_line("### §1 — ").is_none());
    // Non-numeric section number.
    assert!(parse_heading_task_line("### §one — Title").is_none());
    // No separator at all.
    assert!(parse_heading_task_line("### §1 no separator").is_none());
    // Empty section number (bare § with no digits).
    assert!(parse_heading_task_line("### § — Title").is_none());
}

#[test]
fn seed_from_spec_parses_section_sign_headings() {
    let doc = "\
## Spec\n\
\n\
### §1 — Add phase_doc_path to PhaseInput\n\
\n\
Some detail.\n\
\n\
### §2 — Add phase_doc_path to PhaseRun\n\
\n\
More detail.\n\
\n\
### §3 — Thread telemetry_dir through the stack\n\
\n\
## Acceptance criteria\n";

    let tasks = seed_from_spec(doc);
    assert_eq!(tasks.len(), 3, "one task per §N heading: {tasks:?}");
    assert_eq!(tasks[0].id, "1");
    assert_eq!(tasks[0].title, "Add phase_doc_path to PhaseInput");
    assert_eq!(tasks[1].id, "2");
    assert_eq!(tasks[1].title, "Add phase_doc_path to PhaseRun");
    assert_eq!(tasks[2].id, "3");
    assert_eq!(tasks[2].title, "Thread telemetry_dir through the stack");
    for t in &tasks {
        assert_eq!(t.state, TaskState::Pending);
    }
}
```

## End-to-end verification

After gates pass, confirm manually that the Spec section of *this* phase doc would
now seed tasks correctly. A quick repl check:

```
// The headings above (§1, §2, §3) should parse to three tasks.
// Run: cargo test -p rexymcp-executor seed_from_spec_parses_section_sign
// Expected: test result: ok. 1 passed
```

If a live session is available, dispatch a test phase whose Spec uses `§N —`
headings and confirm the Tasks panel shows entries rather than remaining empty.

## Authorizations

- Edit `executor/src/agent/tasks.rs`.

## Out of scope

- Any change to `parse_task_line` (the numbered-list-item parser).
- Support for `§N` as a top-level list item (e.g. `§1. Something`).
- Any change to `update_task`, `list_tasks`, or any other tool.
- Any change to `mcp/`.

## Update Log

<!-- entries appended below this line -->
