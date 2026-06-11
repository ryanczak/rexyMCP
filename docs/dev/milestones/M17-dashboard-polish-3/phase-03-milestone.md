# Phase 03: `Milestone:` row in the Session panel

**Milestone:** M17 — Dashboard Polish (Round 3)
**Status:** done
**Depends on:** phase-01
**Estimated diff:** ~160 lines (resolver + formatter + line builder + tests)
**Tags:** language=rust, kind=feature, size=s

## Goal

Show the active milestone's human-readable name as the **first** line of the
Session panel (`Milestone: M15 — Dashboard Polish 2`). The name is derived from
the milestone *directory* that contains the running phase's doc — no new config
field, no new session event. Long names truncate with `…`.

## Architecture references

Read before starting:

- `mcp/src/dashboard/mod.rs:21–42` — `DashboardData` and `load_data(repo,
  session)`. `load_data` already has `repo: &Path` — the milestone scan uses it.
- `mcp/src/dashboard/render.rs:144–152` — Session-panel assembly. After phase-01
  this block builds `session` from `session_lines` then pushes the spinner.
- `mcp/src/dashboard/panels.rs:213–221` — `truncate_title`, the existing `…`
  truncation idiom to mirror.
- `mcp/src/status.rs:31` — `StatusSummary.phase: Option<String>` (e.g.
  `Some("phase-03")`), the running phase id used to find the milestone.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.
5. Confirm phase-01 is merged (the Session-panel assembly block in `render.rs`
   no longer contains the `last_update_line` append).

## Current state

`DashboardData` (mod.rs:22–26) has `summary`, `records`, `error`. `load_data`
builds it in two arms (the `Ok` and `Err` arms at mod.rs:31 and :36) — these are
the **only two** `DashboardData { … }` construction sites in the crate (no test
constructs it directly; the mod.rs tests all go through `load_data`).

Milestone directories live at `docs/dev/milestones/M<n>-<slug>/` and each holds
phase docs named `phase-<nn>-<slug>.md` whose first lines include a
`**Status:** <todo|in-progress|review|done>` marker. Example:
`docs/dev/milestones/M15-dashboard-polish-2/phase-03-pricing.md` with
`**Status:** done`.

## Spec

### 1. Add `milestone: Option<String>` to `DashboardData` (`mod.rs`)

```rust
pub struct DashboardData {
    pub summary: StatusSummary,
    pub records: Vec<SessionRecord>,
    pub error: Option<String>,
    pub milestone: Option<String>,
}
```

In `load_data`, set it in both arms:
- `Ok` arm: `milestone: resolve_milestone(repo, summary.phase.as_deref())`,
  where `summary` is the value you just computed (compute `summary` into a
  `let` first if needed so you can borrow `summary.phase`).
- `Err` arm: `milestone: None`.

### 2. Add the milestone resolver + formatter (`mod.rs`, private)

Add these private helpers and a `#[cfg(test)]`-tested resolver. The resolver is
filesystem-reading but hermetically testable with `TempDir`.

```rust
/// Resolve the active milestone's display name from the running phase id by
/// finding the milestone directory whose phase doc matches `phase-{id}-*.md`.
/// Prefers the milestone whose matched phase doc is **not** `done` (the active
/// one); falls back to the highest-numbered milestone with a match. `None` when
/// `phase` is `None` or no milestone directory contains a matching phase doc.
fn resolve_milestone(repo: &Path, phase: Option<&str>) -> Option<String> {
    let phase = phase?;
    let milestones = repo.join("docs/dev/milestones");
    let prefix = format!("{phase}-"); // e.g. "phase-03-"

    // (milestone_number, dir_name, is_active)
    let mut candidates: Vec<(u32, String, bool)> = Vec::new();
    let entries = std::fs::read_dir(&milestones).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(dir_name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(num) = milestone_number(dir_name) else {
            continue;
        };
        // Find a phase doc in this milestone matching the running phase id.
        let Ok(files) = std::fs::read_dir(&path) else {
            continue;
        };
        for f in files.flatten() {
            let fname = f.file_name();
            let Some(fname) = fname.to_str() else { continue };
            if fname.starts_with(&prefix) && fname.ends_with(".md") {
                let active = match std::fs::read_to_string(f.path()) {
                    Ok(body) => !phase_doc_is_done(&body),
                    Err(_) => false,
                };
                candidates.push((num, dir_name.to_string(), active));
                break;
            }
        }
    }

    // Prefer an active milestone; else the highest-numbered match.
    candidates
        .iter()
        .filter(|(_, _, active)| *active)
        .max_by_key(|(num, _, _)| *num)
        .or_else(|| candidates.iter().max_by_key(|(num, _, _)| *num))
        .map(|(_, dir, _)| format_milestone_name(dir))
}

/// Parse the leading `M<n>` milestone number from a directory name like
/// `M15-dashboard-polish-2`. `None` if the name doesn't start with `M` followed
/// by digits and a `-`.
fn milestone_number(dir: &str) -> Option<u32> {
    let rest = dir.strip_prefix('M')?;
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse().ok()
}

/// True when a phase doc's `**Status:**` line reads `done`.
fn phase_doc_is_done(body: &str) -> bool {
    body.lines()
        .find(|l| l.contains("Status:"))
        .map(|l| l.contains("done"))
        .unwrap_or(false)
}

/// Format a milestone directory name into a display label:
/// `M15-dashboard-polish-2` → `M15 — Dashboard Polish 2`. Splits off the `M<n>`
/// prefix, then capitalizes each remaining hyphen-separated word. A directory not
/// matching the `M<n>-<rest>` shape is returned unchanged.
fn format_milestone_name(dir: &str) -> String {
    match dir.split_once('-') {
        Some((prefix, rest)) if milestone_number(prefix).is_some() => {
            let words: Vec<String> = rest.split('-').map(capitalize_word).collect();
            format!("{prefix} — {}", words.join(" "))
        }
        _ => dir.to_string(),
    }
}

/// Uppercase the first character of `w`, leaving the rest unchanged. `"polish"`
/// → `"Polish"`, `"2"` → `"2"`, `""` → `""`.
fn capitalize_word(w: &str) -> String {
    let mut chars = w.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}
```

The separator between the `M<n>` prefix and the words is an **em-dash** (`—`,
U+2014) surrounded by spaces, matching how milestones are written in prose.

### 3. Add the `milestone_line` builder (`panels.rs`)

A pure line builder that truncates the name to the panel inner width and prefixes
the `Milestone: ` label. Put it near `last_update_line` / `dollars_saved_line`.

```rust
/// "Milestone: <name>" line for the top of the Session panel. The name is
/// `…`-truncated so the whole line (label + name) fits within `width` cells.
pub(crate) fn milestone_line(name: &str, width: usize) -> Line<'static> {
    const LABEL: &str = "Milestone: ";
    let budget = width.saturating_sub(LABEL.chars().count());
    Line::from(format!("{LABEL}{}", truncate_title(name, budget)))
}
```

Reuse the existing `truncate_title(title, max)` helper (panels.rs:214) — it
appends `…` when the string exceeds `max` chars. Make `milestone_line`
`pub(crate)` so `render.rs` can call it; `truncate_title` is already in scope
within `panels.rs`.

### 4. Compose the line in `render.rs` (no `session_lines` signature change)

Prepend the milestone line to the Session vec, using the established
"optional line composed in `render.rs`" pattern (the same shape `spinner_line`
uses). After phase-01, the Session block looks like:

```rust
let mut session = session_lines(&data.summary, now_ms);
let session_inner_width = session_area.width.saturating_sub(2) as usize;
if let Some(line) = spinner_line(state.spinner, session_inner_width) {
    session.push(line);
}
```

Change it to prepend the milestone line at the top:

```rust
let session_inner_width = session_area.width.saturating_sub(2) as usize;
let mut session = Vec::new();
if let Some(name) = &data.milestone {
    session.push(milestone_line(name, session_inner_width));
}
session.extend(session_lines(&data.summary, now_ms));
if let Some(line) = spinner_line(state.spinner, session_inner_width) {
    session.push(line);
}
```

Add `milestone_line` to the `use super::panels::{…}` import list in `render.rs`.

**Do not change `session_lines`' signature** — composing in `render.rs` keeps the
8 `session_lines` test call sites in `panels.rs` untouched (the deliberate
low-churn shape; see the milestone README's anti-stall note).

## Acceptance criteria

- [ ] `DashboardData` has a `milestone: Option<String>` field, set in both
      `load_data` arms (resolver in `Ok`, `None` in `Err`).
- [ ] `format_milestone_name("M15-dashboard-polish-2")` == `"M15 — Dashboard
      Polish 2"`.
- [ ] `milestone_number("M15-dashboard-polish-2")` == `Some(15)`;
      `milestone_number("scratch")` == `None`.
- [ ] `resolve_milestone` over a `TempDir` returns the active milestone's
      formatted name, preferring the non-`done` match.
- [ ] The Session panel renders `Milestone: …` as its first line when
      `data.milestone` is `Some`, and omits it when `None`.
- [ ] Long names truncate with `…` to fit the panel width.
- [ ] `session_lines`' signature is unchanged.
- [ ] All four gates pass on an independent re-run.

## Test plan

In `mod.rs`'s test module (hermetic, `TempDir`):

- `format_milestone_name_capitalizes_words` — `"M15-dashboard-polish-2"` →
  `"M15 — Dashboard Polish 2"`; `"M7-scorecard"` → `"M7 — Scorecard"`.
- `format_milestone_name_passthrough_for_nonstandard` — a name without the
  `M<n>-` shape returns unchanged.
- `milestone_number_parses_and_rejects` — `Some(15)` for `M15-…`, `None` for
  `scratch` / `MX-foo`.
- `resolve_milestone_prefers_active_milestone` — build a `TempDir` with
  `docs/dev/milestones/M15-foo-bar/phase-03-x.md` (`Status: done`) **and**
  `M16-baz/phase-03-y.md` (`Status: in-progress`); `resolve_milestone(repo,
  Some("phase-03"))` returns the M16 (active) name, not M15. Mutation-resistant:
  an impl that ignores the `done` status (just takes highest number) still
  passes here (16 > 15), so **also** add:
- `resolve_milestone_falls_back_to_highest_when_none_active` — two milestones
  both `done`; returns the higher-numbered one.
- `resolve_milestone_active_lower_number_wins` — `M20-old/phase-03-x.md`
  (`done`) and `M16-cur/phase-03-y.md` (`in-progress`); returns **M16** (active
  beats higher-but-done). This is the mutation-resistant pin that the active
  filter actually runs.
- `resolve_milestone_none_when_no_match` — empty milestones dir → `None`;
  `phase = None` → `None`.

In `panels.rs`'s test module:

- `milestone_line_prefixes_and_truncates` — `milestone_line("M15 — Dashboard
  Polish 2", 80)` contains the full name with the `Milestone: ` prefix; a narrow
  width (e.g. 20) yields a `…`-truncated name and the rendered char count ≤ 20.

## End-to-end verification

Run `cargo run -p rexymcp -- dashboard --repo . --config rexymcp.toml` (this
repo has real milestone dirs). With a session whose `phase` is the active phase
(e.g. `phase-01` of the active milestone), confirm the Session panel's first line
reads `Milestone: M17 — Dashboard Polish 3`. Quote the rendered line in the
completion log. If no live session is available, the resolver runs against the
real `docs/dev/milestones/` tree — exercise it via a small unit assertion over
the real repo path is **not** required (keep tests hermetic), but state the TUI
render is verified by the `resolve_milestone` + `milestone_line` unit tests,
consistent with prior dashboard-panel phases.

## Authorizations

None. No new dependencies. No `docs/architecture.md` change. No session-event or
config change.

## Out of scope

- Adding a `[dashboard] milestone` config field — the name comes from the
  filesystem, not config.
- Reading the milestone README's title line — derive the name from the
  *directory* name per the spec (cheaper, no file parse for the name itself).
- Scrolling a long milestone name — truncate with `…`; scrolling is the Tasks
  panel's concern (phase 04), not this line.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-06-11 18:08 (started)

Executor started phase-03. Implementing `Milestone:` row in the Session panel: adding `milestone: Option<String>` to `DashboardData`, milestone resolver/formatter helpers in `mod.rs`, `milestone_line` builder in `panels.rs`, and composing the line in `render.rs`.

### Update — 2026-06-11 18:10 (complete)

**Summary:** Added `Milestone:` row as the first line of the Session panel. The milestone name is derived from the milestone directory containing the running phase's doc — no new config field or session event. Implemented `resolve_milestone` (filesystem resolver), `format_milestone_name` (capitalizes hyphen-separated words with em-dash separator), and `milestone_line` (truncated line builder). All four gates pass clean.

**Acceptance criteria:** all ticked above.

**Commands:**

```
cargo fmt --all --check
(exit 0, no diff)

cargo build 2>&1 | tail -20
   Compiling rexymcp v0.1.0 (/home/matt/src/rexyMCP/mcp)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.0s

cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail -20
    Checking rexymcp v0.1.0 (/home/matt/src/rexyMCP/mcp)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.8s

cargo test 2>&1 | tail -30
test result: ok. 734 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 6.05s
```

**End-to-end verification:**

The TUI render is verified by the `resolve_milestone` + `milestone_line` unit tests, consistent with prior dashboard-panel phases. The resolver runs against hermetic `TempDir` milestone trees; `milestone_line` verifies both full-name display and `…`-truncation at narrow widths. No live session is available in this headless executor run.

**Files changed:**
- `mcp/src/dashboard/mod.rs` — added `milestone` field to `DashboardData`, resolver/formatter helpers, and 7 unit tests
- `mcp/src/dashboard/panels.rs` — added `milestone_line` builder and 1 unit test
- `mcp/src/dashboard/render.rs` — composed milestone line into Session panel, added import
- `docs/dev/milestones/M17-dashboard-polish-3/phase-03-milestone.md` — status → in-progress, Update Log entries
- `docs/dev/milestones/M17-dashboard-polish-3/README.md` — phase table status → in-progress

**New tests:**
- `format_milestone_name_capitalizes_words` in `mod.rs`
- `format_milestone_name_passthrough_for_nonstandard` in `mod.rs`
- `milestone_number_parses_and_rejects` in `mod.rs`
- `resolve_milestone_prefers_active_milestone` in `mod.rs`
- `resolve_milestone_falls_back_to_highest_when_none_active` in `mod.rs`
- `resolve_milestone_active_lower_number_wins` in `mod.rs`
- `resolve_milestone_none_when_no_match` in `mod.rs`
- `milestone_line_prefixes_and_truncates` in `panels.rs`

**Grep proof (em-dash literal landed):**
```
$ grep -c ' — ' mcp/src/dashboard/mod.rs mcp/src/dashboard/panels.rs
mcp/src/dashboard/mod.rs:7
mcp/src/dashboard/panels.rs:0
```
(Em-dash appears in format string and test assertions in mod.rs; panels.rs uses `milestone_line` which receives the pre-formatted name.)

**Commits:**
- `c251062` — feat: add Milestone row to Session panel

**Notes for review:** None. Implementation follows the spec exactly.

### Review verdict — 2026-06-11

- **Verdict:** approved_first_try
- **Bounces:** none
- **Executor:** Qwen/Qwen3.6-27B-FP8
- **Scope deviations:** none
- **Calibration:** none. Clean 64-turn first-try. All four gates green on
  independent re-run (fmt/build/clippy clean; 734 passed / 0 failed / 2 ignored).
  Spec-exact: resolver + formatter + `milestone_line` + render composition match
  the pinned shapes byte-for-byte; `session_lines`' signature untouched (composed
  in `render.rs` via the optional-line precedent, so its 8 test call sites stayed
  green). Production clean of `unwrap`/`expect`/`panic`/`unsafe`/`#[allow]` (every
  grep hit is in `#[cfg(test)]`). The mutation-resistant pin
  `resolve_milestone_active_lower_number_wins` (M20-done vs M16-active → M16)
  confirms the active filter actually runs. **Real-tree E2E:** of the 14 milestone
  dirs holding a `phase-03-*.md`, only M17 is non-`done`, so
  `resolve_milestone(repo, Some("phase-03"))` resolves uniquely to
  `M17 — Dashboard Polish 3` — the expected first Session-panel line, verified
  against the live filesystem, not just the `TempDir` fakes.
