# Phase 03: Seeder precision — bold-name tasks + de-dup

**Milestone:** M22 — Bookkeeping-Loop Resilience
**Status:** review
**Depends on:** none (independent of phases 01/02/04/05)
**Estimated diff:** ~140 lines
**Tags:** language=rust, kind=feature, size=m

## Goal

Stop `seed_from_spec` from seeding prose numbered lists as tasks, and guarantee
the seeded list has no duplicate ids or titles. Two changes in
`executor/src/agent/tasks.rs`: (B4) the `N.` **list-item** form requires a
`**bold**` name — matching the documented `WORKFLOW.md` § Spec convention
(`N. **<Task name>** — …`) — so a prose ordered list (`1. If packet.tcp…`) is not
mistaken for a task; (B5) de-duplicate the seeded list by id and by title.

In `session-phase-04-6a32f237`, a `#### update()` method's RTT algorithm (`1. If
packet.tcp…`, `2. If packet.tcp…`) was seeded as two tasks with **byte-identical
truncated titles** ("If `packet.tcp` is not null and `packet.tcp.synFlag` is true
and"), colliding with the real test-list ids. The model couldn't tell them apart
and re-marked task 1 to a hard_fail. Both algorithm lines are **non-bold**; the
real task list items are bold (`1. **"FlowTable creates…"**`).

## Architecture references

Read before starting:

- `executor/src/agent/tasks.rs` — `seed_from_spec` (15–34), `parse_task_line`
  (51–76), `extract_title` (79–88), and the `#[cfg(test)] mod tests` block
  (148–389).
- `docs/dev/WORKFLOW.md` § Spec — documents the three accepted task formats; the
  list-item form is written `N. **<Task name>** — …` (bold name). This phase makes
  the seeder match that documented shape for the list form.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm `cargo build` and `cargo test` exit 0.

## Current state

```rust
fn parse_task_line(line: &str) -> Option<Task> {
    let first_char = line.chars().next()?;
    if !first_char.is_ascii_digit() {
        return None;
    }
    let (digits, rest) = line.split_once('.')?;
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    if !rest.starts_with(' ') && !rest.starts_with('\t') {
        return None;
    }
    let title = extract_title(rest);          // <- B4 changes this
    Some(Task { id: digits.to_string(), title, state: TaskState::Pending })
}

/// Extract the task title from the remainder after `<digits>. `.
/// If the trimmed text starts with `**`, extract the bold span.
/// Otherwise, use the whole trimmed remainder.
fn extract_title(rest: &str) -> String {
    let trimmed = rest.trim_start();
    if let Some(after_open) = trimmed.strip_prefix("**")
        && let Some(title) = after_open.split_once("**")
    {
        return title.0.trim().to_string();
    }
    trimmed.trim_end().to_string()           // <- the non-bold fallback we are removing
}
```

`extract_title` is called only at line 68. `parse_heading_task_line` (the `### N.`
/ `### Task N` forms) does **not** use it and is **unchanged** by this phase —
heading-form tasks keep working without a bold name; only the bare `N.` list form
gains the bold requirement.

The `## Spec` scan in `seed_from_spec` collects `parse_task_line(line)` or
`parse_heading_task_line(line)` matches into `tasks` until the next `## ` heading.

## Spec

### Task 1 — B4: require a bold name in `parse_task_line` (`tasks.rs`)

Replace the title line + `Some(...)` (lines 68–74) with a bold-required
extraction that returns `None` when there is no bold span:

```rust
let trimmed = rest.trim_start();
let title = trimmed
    .strip_prefix("**")
    .and_then(|after| after.split_once("**"))
    .map(|(name, _)| name.trim().to_string())
    .filter(|name| !name.is_empty())?;
Some(Task {
    id: digits.to_string(),
    title,
    state: TaskState::Pending,
})
```

Then **delete `extract_title`** (lines 79–88) — it now has no callers, and
STANDARDS §2.2 says delete unused symbols rather than leave them.

Rationale: the documented list-item convention is `N. **<Task name>** — …`. A
prose ordered list (`1. If packet.tcp…`) has no bold name, so it no longer seeds.
Because a bold name is a single span on one line, the title is always complete —
the mid-line truncation that produced duplicate titles is structurally gone.

### Task 2 — B5: de-duplicate seeded tasks by id and title (`seed_from_spec`)

After the collection loop, before `tasks` is returned, drop any task whose id or
title collides with an earlier kept one (first occurrence wins):

```rust
let mut seen_ids = HashSet::new();
let mut seen_titles = HashSet::new();
tasks.retain(|t| {
    if seen_ids.contains(&t.id) || seen_titles.contains(&t.title) {
        false
    } else {
        seen_ids.insert(t.id.clone());
        seen_titles.insert(t.title.clone());
        true
    }
});
```

Add `use std::collections::HashSet;` at the top of `tasks.rs` (it is not yet
imported).

### Task 3 — Update the three tests that pin the old lenient (bare-item) contract

These existing tests assert that a **non-bold** `N.` item seeds — the behavior
B4 deliberately removes. Update each to the new contract (this is an authorized
contract change, not test-gutting):

- `seeds_top_level_numbered_items` (152) — its item `2. Second task — do this
  second` is non-bold. Make all three items bold and keep asserting 3 tasks with
  the bold-span titles, e.g. doc `"## Spec\n\n1. **First task** — do this
  first\n2. **Second task** — do this second\n3. **Third** — last one\n"`, with
  `tasks[1].title == "Second task"`.
- `seeds_plain_title_keeps_whole_remainder` (177) — its premise (bare item seeds
  the whole remainder) is now invalid. Repurpose it to
  `ignores_list_item_without_bold_name`: doc `"## Spec\n\n2. plain text\n"`,
  assert `seed_from_spec(doc).is_empty()`.
- `parses_multi_digit_ids` (218) — make both items bold
  (`10. **Tenth item**\n11. **Eleventh item**\n`) and keep asserting the
  multi-digit ids and titles.

Do **not** touch the heading-form tests (`seed_from_spec_parses_heading_format_tasks`,
`seed_from_spec_mixed_formats`, `_parses_task_dash_heading_format`,
`_parses_section_sign_headings`, etc.) — the heading forms are unchanged.
`seeds_bold_title_strips_to_bold_span` is already the canonical bold case and
must pass unmodified.

### Task 4 — New tests pinning the fix

- `ignores_prose_numbered_list_without_bold` — the netviz regression:

  ```rust
  let doc = "## Spec\n\n#### Algorithm\n\n1. If foo is set and bar\n2. If baz\n\n### Tasks\n\n1. **Parse the header** — do it\n";
  let tasks = seed_from_spec(doc);
  assert_eq!(tasks.len(), 1, "prose 1./2. must not seed; only the bold task");
  assert_eq!(tasks[0].title, "Parse the header");
  ```

- `dedupes_colliding_ids` — a `### 1. Foo` heading and a `1. **Bar**` list item
  both claim id `1`; assert `seed_from_spec` yields exactly one task with id `1`
  (the first in document order).

- `dedupes_identical_titles` — two items with distinct ids but the same bold
  title; assert only the first survives.

**Pinned negatives:**

- `ignores_prose_numbered_list_without_bold` — the exact failure mode this phase
  closes: a non-bold numbered list must seed zero tasks.
- `ignores_list_item_without_bold_name` (repurposed) — a bare `N. text` item is
  not a task.

## Acceptance criteria

- [ ] `ignores_prose_numbered_list_without_bold` passes.
- [ ] `ignores_list_item_without_bold_name` passes.
- [ ] `dedupes_colliding_ids` passes.
- [ ] `dedupes_identical_titles` passes.
- [ ] `seeds_bold_title_strips_to_bold_span` passes **unmodified**.
- [ ] The three contract-update tests (Task 3) pass in their updated form.
- [ ] All heading-form seeder tests pass **unmodified**.
- [ ] `cargo fmt --all --check`, `cargo build`, `cargo clippy`, `cargo test` exit 0.

## Test plan

- New: `ignores_prose_numbered_list_without_bold`, `dedupes_colliding_ids`,
  `dedupes_identical_titles` (Task 4).
- Updated: `seeds_top_level_numbered_items`, `ignores_list_item_without_bold_name`
  (was `seeds_plain_title_keeps_whole_remainder`), `parses_multi_digit_ids`
  (Task 3).
- Unmodified-and-must-pass: `seeds_bold_title_strips_to_bold_span`, all
  `parse_heading_task_line` / heading-format tests.

## End-to-end verification

Not applicable — phase ships no runtime-loadable artifact. `seed_from_spec` is a
pure function; the unit tests exercise it against real `## Spec` fixtures
including the netviz regression shape.

## Authorizations

None. No new dependency, no `Cargo.toml`/`architecture.md` edit. **No
`WORKFLOW.md` edit** — the list-item convention is already documented as
`N. **<Task name>** — …`; this phase makes the seeder match the existing doc. (If
a one-line WORKFLOW clarification that the bold name is *required* for the list
form is later wanted, that is a separate fold for the architect to discuss with
the user — out of scope here.)

## Out of scope

- The `### N.` / `### Task N` heading forms — unchanged; they do not require a
  bold name (the heading is the delimiter).
- A turn-0 warning when de-dup drops a colliding task — the existing
  `seeded.is_empty()` warning still covers the "0 tasks seeded" case; a
  collision-specific warning is a possible follow-up, not this phase.
- Editing `WORKFLOW.md` § Spec (see Authorizations).

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-06-18 00:56 (started)

**By:** executor

### Update — 2026-06-18 00:56 (complete)

**Summary:** Made `parse_task_line` require a bold `**...**` name (B4), so prose numbered lists like `1. If packet.tcp...` no longer seed as tasks. Added de-duplication by id and title in `seed_from_spec` (B5). Deleted the now-unused `extract_title` function. Updated two integration tests in `tests.rs` whose spec fixtures used non-bold list items.

**Acceptance criteria:** all ticked above.

**Commands:**

```
cargo fmt --all --check
<exit 0, no diff>

cargo build 2>&1 | tail -20
<exit 0>

cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail -20
<exit 0>

cargo test 2>&1 | tail -30
test result: ok. 832 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out
```

**End-to-end verification:**

N/A — phase ships no runtime-loadable artifact. `seed_from_spec` is a pure function; unit tests exercise it against real `## Spec` fixtures including the netviz regression shape.

**Files changed:**
- `executor/src/agent/tasks.rs` — B4: require bold name in `parse_task_line`, delete `extract_title`; B5: de-dup by id/title; Task 3: update 3 contract tests; Task 4: add 3 new tests
- `executor/src/agent/tests.rs` — fix 2 integration tests whose spec fixtures used non-bold list items
- `docs/dev/milestones/M22-bookkeeping-resilience/phase-03-seeder-precision.md` — status flip + update log
- `docs/dev/milestones/M22-bookkeeping-resilience/README.md` — phase table status flip

**New tests:**
- `ignores_prose_numbered_list_without_bold` in `executor/src/agent/tasks.rs`
- `dedupes_colliding_ids` in `executor/src/agent/tasks.rs`
- `dedupes_identical_titles` in `executor/src/agent/tasks.rs`

**Updated tests:**
- `seeds_top_level_numbered_items` — made all items bold
- `ignores_list_item_without_bold_name` (was `seeds_plain_title_keeps_whole_remainder`) — repurposed
- `parses_multi_digit_ids` — made items bold
- `loop_seeds_task_updates_from_spec` in `tests.rs` — made item 2 bold
- `loop_still_seeds_task_updates_when_tracking_on` in `tests.rs` — made item 2 bold

**Commits:**
- `5623004` — feat: require bold name for list-form tasks and de-dup seeded tasks

**Notes for review:** Two integration tests in `executor/src/agent/tests.rs` (`loop_seeds_task_updates_from_spec` and `loop_still_seeds_task_updates_when_tracking_on`) had spec fixtures with a non-bold list item (`2. Second task — do that`). These were updated to use bold names (`2. **Second task** — do that`) to match the new contract. This was not called out in the phase spec but was required for the tests to pass — the spec only mentioned the three unit tests in `tasks.rs`.
