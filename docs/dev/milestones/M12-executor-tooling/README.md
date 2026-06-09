# M12 — Executor Tooling

**Goal:** Give the local-LLM executor net-new capability that makes a weak model
more effective and more efficient, where **every intervention's value is
measurable against the scorecard** (`bounces_to_approval`, `first_pass_rate`).
Two arcs, run **Arc B first**: (B) incremental code intelligence harvested
cheaply from parsers already in the tree, then (A) architect-seeded structured
task tracking gated for a clean A/B.

**Status:** in-progress

**Depends on:** M1–M11 (all complete).

## Motivation

| Pain point | Intervention |
|---|---|
| Breaking multi-site changes run out of verifier runway before the cascade compiles (folded in WORKFLOW § "Prefer additive change shapes") | **B — find-references**: enumerate every call site *before* the breaking edit |
| The verifier parses cargo JSON but discards rustc's machine-applicable `suggested_replacement` spans | **B — suggested-fixes**: feed "rustc suggests X→Y at line N" to the model |
| `cargo test` failures reach the model as raw text; the retry loop has no structured expected-vs-actual signal | **B — structured test-failure parsing** (extends the M10 cargo filter) |
| Dropped-subtask stalls (a run did tasks 1–4 then stalled) and premature/false completion | **A — task tracking**: a checklist the executor flips pending → active → done |
| No way to measure whether task tracking helps | **A — config-gated A/B** (`[executor] task_tracking`, **default on**) + a dashboard `Tasks` panel |

## Architecture references

- `docs/architecture.md#status` — M12 entry (two arcs, locked scope, non-goals).
- `docs/architecture.md` § "No internal cloud escalation" — the executor stays
  offline; M12 adds no network tooling.

## Phases

Run in order; Arc B (01–03) before Arc A (04–05). The architect expands each
phase doc on demand (`/rexymcp:architect next`), not all at once.

| Phase | Title | Status | Kind | Size |
|---|---|---|---|---|
| 01 | Arc B — find-references in `symbols` (tree-sitter call-site search) | todo | feature | m |
| 02 | Arc B — surface rustc machine-applicable suggested-fix spans | todo | feature | s |
| 03 | Arc B — structured `cargo test` failure parsing (expected-vs-actual) | todo | feature | m |
| 04 | Arc A — task-tracking substrate: `SessionEvent::TaskUpdate`, Spec-seeded list, config-gated (`task_tracking`, default on) | todo | feature | l |
| 05 | Arc A — dashboard `Tasks` panel above Files (Files height halved) | todo | feature | m |

## Exit criteria

- [ ] `symbols` can return the **call sites / references** of a named symbol via
  tree-sitter (Rust + Python, the languages it already supports), with pinned
  negative cases (a same-named symbol in an unrelated scope must **not** match by
  bare substring).
- [ ] The verifier surfaces rustc's machine-applicable `suggested_replacement`
  spans (span + replacement text) to the model when present.
- [ ] `cargo test` failures are parsed into structured expected-vs-actual records
  available to the verifier-retry loop.
- [ ] `[executor] task_tracking` (**default on**) seeds a per-session task list
  from the phase doc's numbered Spec; the executor emits `SessionEvent::TaskUpdate`
  as it flips items pending → active → done and may append discovered sub-steps,
  but does **not** author the initial list. With the toggle **off**, behavior is
  byte-identical to pre-M12 so on/off runs are directly comparable on the scorecard.
- [ ] The dashboard shows a `Tasks` panel (active/pending/done) above the Files
  panel, with the Files panel's height halved to make room.
- [ ] All gates pass: `cargo fmt --all --check`, `cargo build` (zero warnings),
  `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test`.

## Non-goals

- **No executor-side planning/decomposition.** Arc A *tracks* an
  architect-authored list (seeded from the Spec); it does not generate one. The
  dropped Rexy planner stays dropped.
- **No full LSP client** until bounce-cause tagging shows symbol-resolution /
  wrong-API-usage is a *dominant* failure class. If the data says so, it is its
  own milestone (transport → lifecycle → doc-sync → tool surfaces → hermetic
  tests), not smuggled into M12.
- **No new cloud/network tooling.** The executor stays offline.

## Notes

### Kickoff decisions (2026-06-09, with the user)

- **Scope = both arcs, Arc B first.** Lead with the certain-value code-intelligence
  wins; task tracking (the config-gated A/B experiment) follows.
- **Arc B depth = all three cheap wins** (find-references + suggested-fix spans +
  structured test-failure parsing). All are small, independent, and reuse parsers
  already in the tree (tree-sitter `symbols`; the verifier's cargo-JSON parsing;
  the M10 cargo output filter).
- **`task_tracking` default = on.** Tracking becomes the new normal; control runs
  flip it off. Local-LLM tokens are free and the 131071-token window has never
  hit a compaction event, so the added context is affordable.

### Pre-injection watch-items for the drafting architect

- **Phase 04 adds a new `SessionEvent` variant** (`TaskUpdate`). New-variant
  match-arm blast radius is the **known wall** that hard-failed three earlier
  phases (M10 phase-03/04/06): the exhaustive arms in `dashboard/filter.rs` (the
  **seven** per-event-kind sites — const + field + Default + allows/toggle/
  is_enabled/item_label), `dashboard/transcript.rs::record_lines`,
  `log_query::event_type_str`/`event_kind`. Enumerate every arm with a grep-verified
  site list in the phase-04 doc, or split the mechanical arm-fixups into a micro-step.
  Consider whether phase-04 should itself split (substrate/emit vs. seeding logic)
  to stay single-concern.
- **Phase 01/02/03 reuse existing parsers** — quote the current `symbols`
  tree-sitter query shape and the verifier's cargo-JSON `Diagnostic` parsing as
  worked examples; do not say "follow the existing pattern."
- The **off-switch byte-identity** requirement (phase 04) is a pinned negative
  case: a test must assert that with `task_tracking = false` no `TaskUpdate` event
  is emitted and the prompt/transcript are unchanged.

<!-- retrospective written here after milestone close -->
