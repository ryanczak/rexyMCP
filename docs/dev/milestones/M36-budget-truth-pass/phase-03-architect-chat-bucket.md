# Phase 03: Rename the `other` architect bucket to `architect chat`

**Milestone:** M36 — Budget Truth Pass
**Status:** todo
**Depends on:** phase-01 (sequencing only — avoids two phases editing
`harvest.rs` and its display consumers concurrently; no code dependency)
**Estimated diff:** ~70 lines
**Tags:** language=rust, kind=refactor, size=s

## Goal

`other` is the second-largest architect bucket (18.7 % of project spend, 359 M
tokens) and its name tells the user nothing. It is not a leftover: it is
untagged Claude work — whole non-skill sessions plus the user↔architect
conversation between phase runs — and all of it is architect spend. Display it
as **`architect chat`** so the per-skill table accounts for every token in terms
the user can act on.

## Architecture references

Read before starting:

- `docs/dev/milestones/M36-budget-truth-pass/README.md` — what `other` was
  verified to contain, and why this is a display-layer change.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

**`mcp/src/harvest.rs:111-114`** writes the stored key:

```rust
    let skill = match v.get("attributionSkill").and_then(|s| s.as_str()) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => "other".to_string(),
    };
```

**`mcp/src/costs.rs:88-126`** — `skill_costs` accumulates per-skill totals into a
`HashMap<String, (u64, f64)>` keyed by `l.skill`, then sorts by cost descending
with a `skill`-name tiebreak. This is the single aggregation point: the
`rexymcp costs` per-skill table (`costs.rs:319-343`) and the dashboard top-skill
hint (`mcp/src/dashboard/mod.rs:100` → `render.rs:227`) both consume its output.

**Design decision — rename at display, not at write.** The stored ledger key
stays `"other"`. Changing what `harvest.rs` writes would leave every
already-harvested record keyed `other` while new records say `architect chat`;
`fold_ledger` keys on `(session, model, skill)`, so the two would persist
forever as **two separate rows** for the same thing. Mapping at the aggregation
point instead needs no migration, cannot produce a split row, and keeps the
stored key stable and machine-readable. So `harvest.rs` is **not** modified by
this phase.

## Spec

### 1. Add a display-name mapping in `mcp/src/costs.rs`

Add a small pure function next to `skill_costs`:

```rust
/// Display name for a stored architect-ledger skill key.
///
/// The harvester buckets messages with no `attributionSkill` under the stable
/// storage key `other`. That is untagged architect work — non-skill sessions and
/// the user↔architect conversation between phase runs — so it renders as
/// `architect chat`. Mapping here rather than at write time keeps already-
/// harvested records valid and cannot split one bucket across two rows.
pub(crate) fn display_skill(skill: &str) -> &str {
    match skill {
        "other" => "architect chat",
        s => s,
    }
}
```

### 2. Apply it at the aggregation point

In `skill_costs` (`mcp/src/costs.rs:88-126`), key the accumulator by
`display_skill(&l.skill)` rather than `l.skill`. Two consequences to get right:

- If a future record ever *is* stored as `architect chat`, it folds into the
  same row rather than creating a duplicate — that is the point of mapping at
  the key, not at the format string.
- The existing `skill`-name tiebreak in the sort now compares display names.
  That is correct and keeps the ordering deterministic.

Do **not** apply the mapping in `format_costs` or in the dashboard renderer.
One code point only — both consumers read `SkillCost.skill`, so mapping once at
the source covers the `costs` table and the top-skill hint together.

### 3. Tests

Write the tests named in § Test plan.

## Acceptance criteria

- [ ] `cargo build` is green.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` is clean.
- [ ] `cargo fmt --all --check` reports no diff in the files this phase touched.
- [ ] `cargo test -p rexymcp` passes.
- [ ] `rexymcp costs` shows a row labelled `architect chat` and no row labelled
      `other`.
- [ ] `mcp/src/harvest.rs` is unmodified by this phase.

## Test plan

In `mcp/src/costs.rs` tests:

- `display_skill_maps_other_to_architect_chat` — asserts
  `display_skill("other") == "architect chat"`.
- `display_skill_passes_through_named_skills` — asserts
  `display_skill("rexymcp:dispatch") == "rexymcp:dispatch"` and
  `display_skill("rexymcp:auto") == "rexymcp:auto"`. (Negative case: guards
  against a mapping that rewrites more than it should.)
- `skill_costs_renders_other_as_architect_chat` — two ledger records for one
  project, one with skill `other` and one with `rexymcp:dispatch`; asserts the
  returned `SkillCost.skill` values are exactly `["rexymcp:dispatch",
  "architect chat"]` or the cost-sorted equivalent, and that no entry equals
  `"other"`.
- `skill_costs_folds_other_and_architect_chat_into_one_row` — two ledger
  records for the same project, one stored as `other` and one already stored as
  `architect chat`; asserts the result has **one** row whose tokens and cost are
  the sum of both. This pins the key-level mapping; a format-string mapping
  would produce two rows and fail.

Use the existing `fn ledger(model: &str)` test helper at `mcp/src/costs.rs:713`
as the fixture builder — extend it or add a sibling that also takes a skill
name, rather than hand-rolling a new record shape.

## End-to-end verification

```bash
cargo run -p rexymcp -- costs --config rexymcp.toml --repo .
```

Paste the actual output in the completion Update Log. Expected: the
`By skill (architect)` table shows an `architect chat` row where `other`
previously appeared, with the same token and cost figures and the same position
in the cost-sorted order. Confirm the `%` column still sums to ~100.

Then confirm the dashboard hint reads from the same mapping:

```bash
cargo run -p rexymcp -- dashboard --repo .
```

If `architect chat` happens to be the top bucket by cost, the one-line top-skill
hint must show `architect chat`. If a named skill outranks it, state that in the
Update Log and note the hint was verified unchanged — do not contrive a fixture
to force the row.

## Authorizations

None. No new dependencies. No edits to `docs/architecture.md` or `README.md`.

## Out of scope

- Modifying `mcp/src/harvest.rs`. The stored key stays `other` — see § Current
  state for why.
- Any migration or rewrite of existing ledger records.
- Renaming any other bucket, including `rexymcp:auto`. `auto` was verified to be
  disjoint from the other skill buckets with no double-counting; it needs no
  change.
- Splitting `architect chat` into finer sub-buckets (e.g. non-skill sessions vs.
  between-phase conversation). The transcripts carry no field that distinguishes
  them.
- Any Budget-panel row or label change (phase 02).

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
