# Phase 01 — Recognize `### Task N —` heading task format

**Status:** in-progress
**Milestone:** M16-seeder-robustness
**Depends on:** none (first M16 phase)

## Goal

Extend the task seeder's heading parser so a `## Spec` section written with
`### Task N — Title` subheadings seeds one task per heading. Today the parser
recognizes only `### N. Title` (digit-then-dot); a `### Task 2 — …` heading
starts with the word "Task", matches nothing, and seeds zero tasks.

**Single production file** (`executor/src/agent/tasks.rs`). The change is
**additive**: the existing `### N. Title` branch stays byte-identical (all prior
tests pass unchanged); a new branch handles the optional `Task ` prefix with a
`—` / `:` / `.` separator.

~30 lines (one new branch + one small helper + tests). No new `SessionEvent`,
no config, no `Cargo.toml`, no other file.

## Architecture references

- `executor/src/agent/tasks.rs:89–105` — `parse_heading_task_line` (to extend)
- `executor/src/agent/tasks.rs:15–35` — `seed_from_spec` (caller; **unchanged**)
- `executor/src/agent/tasks.rs:226–234` — `parse_heading_task_line_rejects_non_heading_lines`
  (existing rejection test; **must still pass unchanged**)

## Pre-flight

- [ ] `cargo test -p executor` passes on the current tree.
- [ ] Confirm `tasks.rs:91` is `fn parse_heading_task_line(line: &str) -> Option<Task> {`.

## Current state

### `parse_heading_task_line` (tasks.rs:89–105) — current

```rust
/// Parse a `### N. Title` subheading as a task. Returns `None` for any other
/// line shape.
fn parse_heading_task_line(line: &str) -> Option<Task> {
    let rest = line.trim().strip_prefix("### ")?;
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

The whitespace-after-dot check is load-bearing: it rejects `### 1.5x` (a
decimal, not a task). Keep that branch exactly as-is.

### Why this matters (session 6a2a3907 evidence)

M15 phase-02's `## Spec` used `### Task 1 — …` headings. The turn-0 warning
fired (`0 tasks were seeded`), and the executor improvised `update_task(id="02")`
→ correctly rejected `no task with id "02"`. Broadening the parser to accept the
`### Task N —` style closes the gap.

## Spec

### 1. Add the `Task ` prefix branch to `parse_heading_task_line`

Replace the function body so it first handles an optional `Task ` prefix with a
`—` / `:` / `.` separator, then falls through to the **unchanged** `### N.`
dot-branch:

```rust
/// Parse a task subheading. Recognizes both `### N. Title` (dot, decimal-safe)
/// and `### Task N — Title` / `### Task N: Title` / `### Task N. Title`
/// (the architect's natural heading style). Returns `None` for any other shape.
fn parse_heading_task_line(line: &str) -> Option<Task> {
    let rest = line.trim().strip_prefix("### ")?;

    // `### Task N <sep> Title` where <sep> is em-dash, colon, or dot.
    if let Some(after_task) = rest.strip_prefix("Task ") {
        let (digits, title_part) = after_task.split_once(['—', ':', '.'])?;
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

Notes on the new branch:
- `after_task.split_once(['—', ':', '.'])` splits at the **first** occurrence of
  any of the three separator chars. For `2 — Update foo`, the `—` is found before
  any `.` in the title, so the split is `("2 ", " Update foo")`.
- `digits.trim()` strips the trailing space in `"2 "` (from `### Task 2 —`).
- `'—'` is U+2014 EM DASH (the character used in the phase docs), not a hyphen.
- The `update_task` tool keys tasks by **id** (the digit string), so
  `### Task 2 — …` seeds id `"2"`, and the executor's `update_task(id="2", …)`
  will match. (The phase-number guess `"02"` won't — but the executor sees the
  seeded list in its prompt, so it uses the real ids.)

### 2. Keep `seed_from_spec` unchanged

`seed_from_spec` already calls
`parse_task_line(line).or_else(|| parse_heading_task_line(line))`. No change.

## Acceptance criteria

- [ ] `### Task 1 — Title`, `### Task 2: Title`, and `### Task 3. Title` headings
      in a `## Spec` section each seed one `Pending` task with id `"1"`/`"2"`/`"3"`
      and the trimmed title.
- [ ] `### N. Title` headings still seed correctly (existing behavior).
- [ ] `### 1.5x` (decimal) still does **not** seed (the dot-branch guard holds).
- [ ] `N. **Title**` list items still seed correctly (the `parse_task_line` path
      is untouched).
- [ ] `parse_heading_task_line_rejects_non_heading_lines` passes unchanged.
- [ ] All four gates pass on an independent re-run.

## Test plan

Add to the `#[cfg(test)] mod tests` block in `tasks.rs`:

```rust
#[test]
fn seed_from_spec_parses_task_dash_heading_format() {
    let doc = "## Spec\n\n### Task 1 — Update the signature\n\nDetail.\n\n### Task 2 — Fix the call site\n\n## Acceptance criteria\n";
    let tasks = seed_from_spec(doc);
    assert_eq!(tasks.len(), 2, "two ### Task N — headings should seed two tasks");
    assert_eq!(tasks[0].id, "1");
    assert_eq!(tasks[0].title, "Update the signature");
    assert_eq!(tasks[1].id, "2");
    assert_eq!(tasks[1].title, "Fix the call site");
    for t in &tasks {
        assert_eq!(t.state, TaskState::Pending);
    }
}

#[test]
fn parse_heading_task_line_accepts_task_prefix_separators() {
    // em-dash, colon, dot separators all parse after the `Task N` prefix.
    let dash = parse_heading_task_line("### Task 4 — Em dash title").unwrap();
    assert_eq!(dash.id, "4");
    assert_eq!(dash.title, "Em dash title");

    let colon = parse_heading_task_line("### Task 5: Colon title").unwrap();
    assert_eq!(colon.id, "5");
    assert_eq!(colon.title, "Colon title");

    let dot = parse_heading_task_line("### Task 6. Dot title").unwrap();
    assert_eq!(dot.id, "6");
    assert_eq!(dot.title, "Dot title");

    // Multi-digit id after the Task prefix.
    let ten = parse_heading_task_line("### Task 10 — Tenth").unwrap();
    assert_eq!(ten.id, "10");
}

#[test]
fn parse_heading_task_line_rejects_malformed_task_prefix() {
    // No separator after the number.
    assert!(parse_heading_task_line("### Task 1 no separator").is_none());
    // Empty title after the separator.
    assert!(parse_heading_task_line("### Task 1 — ").is_none());
    // Non-numeric "id".
    assert!(parse_heading_task_line("### Task one — Title").is_none());
}
```

**Mutation-resistance:** `seed_from_spec_parses_task_dash_heading_format` fails
on the current (pre-fix) parser — it seeds 0, not 2 — so it pins the new
behavior. The existing `### N.` and rejection tests pin that the old path is
untouched.

## End-to-end verification

E2E: hand a phase doc with `### Task N —` Spec headings to `seed_from_spec` (the
new test does exactly this over a literal doc string — no I/O, deterministic).
The full integration (live executor seeding + `update_task` matching the seeded
id) is exercised on the next real dispatch; quote the test output in the Update
Log.

## Authorizations

Single-file change to `executor/src/agent/tasks.rs` (production + its test
module). No new `SessionEvent`, no config, no `Cargo.toml`, no other file.

## Out of scope

- `WORKFLOW.md`'s accepted-Spec-formats documentation — a contract-doc change the
  architect makes with the user, not the executor (STANDARDS §5 forbids the
  executor touching `WORKFLOW.md`).
- Skipping fenced code blocks inside `## Spec` (a separate latent concern;
  not triggered here).
- Any change to `parse_task_line` (the list-item path) or `seed_from_spec`.

## Update Log

<!-- entries appended below this line -->
