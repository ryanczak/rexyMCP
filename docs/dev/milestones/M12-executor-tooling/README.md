# M12 — Executor Tooling

**Goal:** Give the local-LLM executor net-new capability that makes a weak model
more effective and more efficient, where **every intervention's value is
measurable against the scorecard** (`bounces_to_approval`, `first_pass_rate`).
Three arcs: (0) toolchain robustness — make the validation features degrade
gracefully when a tool is absent and let the human see what's missing; (B)
incremental code intelligence harvested cheaply from parsers already in the tree;
(A) architect-seeded structured task tracking gated for a clean A/B. Order: Arc 0
(foundational), then **Arc B**, then Arc A.

**Status:** in-progress

**Depends on:** M1–M11 (all complete).

## Motivation

| Pain point | Intervention |
|---|---|
| A missing toolchain binary makes the verifier return `Failed("cargo check spawn failed: …os error 2")` (`verifier.rs:250`); the loop appends that raw, remedy-less io-error to the conversation **every edit turn** (`mod.rs:804`) — opaque and repeated | **0 — degrade**: missing binary → a distinct `Skipped` advisory that names the binary + how to install it (and, being non-`Checked`, accrues no governor strike — same as `Failed`/`Unsupported` today) |
| No way for the human to see, before dispatching, whether the target toolchain is installed | **0 — `rexymcp doctor`**: a CLI report of per-language toolchain availability (the architect runs it at bootstrap; also scriptable/CI) |
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

Run in order: Arc 0 (01–02, foundational) → Arc B (03–05) → Arc A (06–07). The
architect expands each phase doc on demand (`/rexymcp:architect next`), not all at
once.

| Phase | Title | Status | Kind | Size |
|---|---|---|---|---|
| 01 | Arc 0 — verifier missing-binary → `Skipped` advisory ([phase-01-verifier-degrade.md](phase-01-verifier-degrade.md)) | done | bugfix | s |
| 02 | Arc 0 — `rexymcp doctor` toolchain-availability command ([phase-02-doctor.md](phase-02-doctor.md)) | done | feature | m |
| 03 | Arc B — find-references in `symbols` (tree-sitter call-site search) ([phase-03-find-references.md](phase-03-find-references.md)) | done | feature | m |
| 04 | Arc B — surface rustc machine-applicable suggested-fix spans ([phase-04-suggested-fixes.md](phase-04-suggested-fixes.md)) | done | feature | s |
| 05 | Arc B — structured `cargo test` failure digest ([phase-05-structured-test-failures.md](phase-05-structured-test-failures.md)) | done | feature | m |
| 06a | Arc A — task-tracking substrate: `SessionEvent::TaskUpdate`, pure Spec seeder, `rexymcp status` consumer (unconditional emit; no gate) ([phase-06a-task-substrate.md](phase-06a-task-substrate.md)) | done | feature | m |
| 06b | Arc A — `[executor] task_tracking` gate: config + `LoopDeps` field gating 06a's seeding emit (the 9-site literal churn + A/B off-switch byte-identity) ([phase-06b-task-tracking-gate.md](phase-06b-task-tracking-gate.md)) | review | feature | s |
| 06c | Arc A — model-facing flip tool (`update_task`) + `router::categorize` arm + prompt injection (gated by 06b's flag; no `LoopDeps` churn) | todo | feature | m |
| 07 | Arc A — dashboard `Tasks` panel above Files (Files height halved) | todo | feature | m |

## Exit criteria

- [ ] When a verifier toolchain binary is **absent** (`cargo`/`tsc`/`ruff` not on
  PATH), the verifier returns a `Skipped` advisory naming the missing binary and
  the remedy — distinct from `Failed` (a genuine infra error) and from "the tool
  ran and found diagnostics" — and the loop surfaces it as a *skipped* (not
  *failed*) notice without accruing a verifier-persistence strike. A pinned test
  simulates the missing-binary path via the pure spawn-error classifier.
- [ ] `rexymcp doctor` reports, per language, whether the Tier-0 command-set
  toolchain and the Tier-1 enhancer binaries are installed and on PATH, with a
  non-zero exit when a **required** (Tier-0) tool is missing.
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
- **Arc 0 — toolchain robustness added (2026-06-09, with the user)** after a
  design discussion on fail-open vs. fail-hard for missing validation tooling.
  Resolution: **fail-hard-advisory where a human can act, fail-open at runtime.**
  Two phases: (01) the verifier missing-binary degrade fix — today a missing
  binary returns `Failed` with a raw, remedy-less io-error repeated each edit
  turn (it does *not* strike — only `Checked` feeds the governor); it should be a
  distinct `Skipped` advisory that names the binary + remedy; (02) a standalone
  `rexymcp doctor` command the architect runs at
  bootstrap to detect+report toolchain availability. **Detection is the
  architect's job + `doctor`, NOT `rexymcp init`** — init stays a static
  scaffolder so it never becomes opinionated about which languages are supported
  (a project in an unsupported language like Zig runs on the Tier-0 command set
  alone). Discipline folded into WORKFLOW.md/STANDARDS.md/architect SKILL.md
  (commit `5cc2ff2`).

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
- The **off-switch byte-identity** requirement is a pinned negative case: a test
  must assert that with `task_tracking = false` no `TaskUpdate` event is emitted
  and the prompt/transcript are unchanged. **This lives in phase-06b** (where the
  gate exists), not 06a. The *event*-suppression half (no `TaskUpdate` when off)
  is 06b's test; the *prompt*-suppression half (no task section when off) is 06c's
  (06b adds no prompt change, so its off/on `Prompt` records are already identical).

### Phase-06 split decision (2026-06-09, drafting `/architect next`)

The single `l` phase-06 was split into **06a / 06b** to isolate two stall-class
risks the calibration history flags (NEXT.md): the new-`SessionEvent`-variant
exhaustive-match wall (M10 phase-03/04/06) and the cross-crate struct-literal
churn (phase-08a's 9-site `LoopDeps`/literal wall).

- **06a (drafted)** — the `TaskUpdate` variant + the pure `seed_from_spec`
  parser + the loop emitting one `pending` update per Spec item **unconditionally**
  + a `rexymcp status` consumer. Its only *logic* is one pure parser; it adds
  **no** `LoopDeps`/`PhaseInput`/config field, so it carries the variant
  match-arm wall **without** the struct-literal churn. Emitting unconditionally
  is harmless (additive log events + a status line; no model-facing change).
- **06b** — adds the `[executor] task_tracking` gate (the `LoopDeps` field — the
  9-site literal churn, pre-injected as a complete site list + a compiler-guided
  recipe) and wraps 06a's seeding emit behind it. **Plumbing only**, so the
  literal-churn stall class is isolated into its own phase.
- **06c** *(split from the original 06b, 2026-06-09 with the user)* — the
  model-facing flip tool (`update_task`) + its `router::categorize` arm + prompt
  injection, all gated by 06b's now-existing flag (so 06c carries **no** further
  `LoopDeps` churn). The A/B *model-behavior* off-switch (off → byte-identical
  prompt + no flip tool) is meaningful and testable here.

**Why 06b was split into 06b/06c (2026-06-09, with the user):** the original
combined 06b stacked two documented stall classes in one executor session — the
`LoopDeps` struct-literal churn (phase-08a/08d: a new field touches ~12
construction sites) *and* a new model-facing tool + `router::categorize` arm +
prompt injection. Same medicine as the 06/06a-06b split: isolate one stall class
per phase. 06b is now ~120 lines of pure plumbing; 06c gets the field for free.

<!-- retrospective written here after milestone close -->
