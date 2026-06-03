# Phase 07: Compaction events — emit `SessionEvent::Compaction`

**Milestone:** M8 — Live session dashboard
**Status:** todo
**Depends on:** phase-06a (done) — established the "executor flushes a per-turn
metric record to the JSONL" pattern this phase mirrors for compaction.
**Estimated diff:** ~60 lines (`event.rs` variant + `agent/mod.rs` emit + 2 match
arms + 1 loop test).
**Tags:** language=rust, kind=feature, size=s

## Goal

Close **Gap C** of the measurement roadmap (M8 README § Notes): the context
compactor fires silently. When `compact()` runs on budget overflow it already
computes a full `CompactionReport` (tokens before/after, messages signaturized,
messages evicted) — but the call site at `executor/src/agent/mod.rs:182`
**discards the return value**, so nothing is ever recorded. We never see when
compaction ran or how much it freed.

This phase adds a `SessionEvent::Compaction` variant and emits it from that call
site, mapping the existing `CompactionReport` straight into the event. That makes
compaction firings visible in the session JSONL — greppable via the
`executor_log_search` event-type filter, and available as a forensic replay record.

**Emit only.** This phase does **not** render compaction in the dashboard and does
**not** fold it into `StatusSummary`. M8 is staying open for a dashboard redesign
(phase-09+) that will rework the panels; adding a panel now would be throwaway. The
render half is deliberately deferred to that redesign.

## Architecture references

Read before starting:

- M8 README § "Measurement roadmap" → "Gap C" — names this exact work: emit
  `SessionEvent::Compaction` from the silent `compact()` firing.
- `executor/src/context/compactor.rs` — the `compact()` function and the
  `CompactionReport` it already returns (the data source for the new event).

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read `executor/src/context/compactor.rs` end to end (small) — note that
   `compact()` **returns** a `CompactionReport`; this phase stops throwing it away.
3. Read `executor/src/store/sessions/event.rs` end to end (small) — you add a variant.
4. Read this entire phase doc before touching code.
5. Confirm `cargo clippy --all-targets --all-features -- -D warnings` and
   `cargo test` are green before changing anything.

## Current state

### `compact()` already returns everything we need (`executor/src/context/compactor.rs`)

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactionReport {
    pub tokens_before: usize,
    pub tokens_after: usize,
    pub messages_signaturized: usize,
    pub messages_evicted: usize,
}

pub fn compact(
    messages: &mut Vec<Message>,
    budget: &Budget,
    system_prompt: &str,
) -> CompactionReport { … }
```

### The call site that throws the report away (`executor/src/agent/mod.rs:179-210`)

```rust
    loop {
        // Step 2 — budget: compact on overflow, give up if still over.
        if deps.budget.would_overflow(&system, &messages) {
            compact(&mut messages, deps.budget, &system);   // ← report discarded
            if deps.budget.would_overflow(&system, &messages) {
                log_session_end(&log_handle, &redactor, deps.clock, "budget_exceeded", turns);
                …
```

`compact()` is only ever called here, and only when `would_overflow` is already
true — so every call is a real compaction firing worth recording. `turns` is in
scope (the count of turns completed so far); the adjacent `log_session_end` uses it.

### The emit helper and an event already emitted the same way (`executor/src/agent/mod.rs`)

`log_event` is the one-liner used for every JSONL record:

```rust
fn log_event(
    handle: &Option<SessionLogHandle>,
    redactor: &Redactor,
    clock: &dyn Fn() -> u64,
    turn: usize,
    event: SessionEvent,
) { … }
```

The per-turn `Metrics` emit (added in 06a, `agent/mod.rs:405-415`) is the exact
shape to copy:

```rust
        log_event(
            &log_handle,
            &redactor,
            deps.clock,
            turns,
            SessionEvent::Metrics {
                input_tokens: metrics.tokens.input_tokens,
                output_tokens: metrics.tokens.output_tokens,
                context_pct: deps.budget.fraction_used(&system, &messages),
            },
        );
```

### The enum (`executor/src/store/sessions/event.rs`)

`SessionEvent` is `#[serde(tag = "event_type", rename_all = "snake_case")]`. The most
recent variant (06a) shows the doc-comment + shape convention:

```rust
    /// Per-turn resource snapshot: cumulative token usage and the fraction of
    /// the context-window budget consumed going into this turn. …
    Metrics {
        input_tokens: u32,
        output_tokens: u32,
        context_pct: f64,
    },
```

## Spec

Numbered tasks in execution order. **This change crosses both crates** (the enum
lives in `executor`, one consumer match lives in `mcp`). It is purely additive — a
new variant, no field changes to existing variants — so no `#[serde(default)]` is
needed (old logs simply never contain a `compaction` record; new logs round-trip
fine). Follow the build order in task 5 so you see the two exhaustive matches that
need a new arm.

1. **Add the `Compaction` variant** — in `executor/src/store/sessions/event.rs`, add
   to `SessionEvent` (mirroring `CompactionReport`'s fields as `usize` — see the
   note below on why not `u32`):

   ```rust
       /// Emitted each time the context compactor runs (on budget overflow at the
       /// top of a turn). Mirrors `CompactionReport`: token totals before/after and
       /// the message counts touched. Tokens freed = `tokens_before - tokens_after`.
       Compaction {
           tokens_before: usize,
           tokens_after: usize,
           messages_signaturized: usize,
           messages_evicted: usize,
       },
   ```

   **Field type — use `usize`, not `u32`.** `CompactionReport`'s fields are `usize`,
   so this lets the emit be a direct field copy with no `as` cast (a cast would be
   one more place to get wrong, and lossless-ness would have to be argued). The 06a
   `Metrics` variant used `u32` because its source (`TokenBreakdown`) is `u32`; match
   *your* source the same way. The JSONL is local-only, so cross-platform `usize`
   width is not a concern.

2. **Capture the report and emit the event** — in `executor/src/agent/mod.rs` at the
   `compact()` call site (line ~182), bind the report and log it **before** the inner
   `would_overflow` re-check:

   ```rust
        if deps.budget.would_overflow(&system, &messages) {
            let report = compact(&mut messages, deps.budget, &system);
            log_event(
                &log_handle,
                &redactor,
                deps.clock,
                turns,
                SessionEvent::Compaction {
                    tokens_before: report.tokens_before,
                    tokens_after: report.tokens_after,
                    messages_signaturized: report.messages_signaturized,
                    messages_evicted: report.messages_evicted,
                },
            );
            if deps.budget.would_overflow(&system, &messages) {
                log_session_end(…);  // unchanged
                …
   ```

   Emit on **every** `compact()` call (it only runs on real overflow). Use `turns` as
   the record's turn, matching the adjacent `log_session_end`.

3. **Add the production match arm** — in `mcp/src/log_query.rs`, `event_type_str`
   (line ~14) is an **exhaustive** match (no `_` arm). Add, keeping snake_case to
   match the serde discriminant:

   ```rust
           SessionEvent::Compaction { .. } => "compaction",
   ```

4. **Add the test-helper match arm** — in `executor/src/agent/mod.rs`, the
   `event_kind` test helper (line ~2172, inside `#[cfg(test)] mod tests`) is also
   exhaustive. Add the same arm:

   ```rust
           SessionEvent::Compaction { .. } => "compaction",
   ```

5. **Build order (cross-crate, additive).** After task 1, `cargo build` will fail at
   exactly two non-exhaustive matches: `mcp/src/log_query.rs` (`event_type_str`) and
   the `event_kind` test helper. Add both arms (tasks 3–4), then add the emit (task
   2), then build clean. These are the **only** two matches that need a new arm —
   verified by grep (see "Do NOT touch" below).

## Do NOT add arms to these (they are immune or out of scope)

These also match `SessionEvent` but must be **left alone** — adding arms here is
scope creep or a no-op:

- `mcp/src/status.rs` `summarize` — has a `_ => {}` catch-all. The `Compaction`
  record correctly falls through it for now. **Rendering / folding into
  `StatusSummary` is the deferred render half — do not touch `summarize`.**
- `executor/src/agent/mod.rs` `redact_event` — redacts via JSON round-trip
  (`serde_json::to_string` → redact → `from_str`); it does not match variants, so the
  new variant passes through automatically.
- `mcp/src/log_query.rs` `matches_tool_name_filter` — has `_ => false`. Compaction is
  not a tool event; leave the catch-all.
- The `jsonl.rs` round-trip tests — each matches one variant with `_ => panic!(…)`;
  immune.

## Acceptance criteria

- [ ] `SessionEvent::Compaction { tokens_before, tokens_after, messages_signaturized,
      messages_evicted }` exists in `executor/src/store/sessions/event.rs` with `usize`
      fields.
- [ ] The `compact()` call site binds the report and emits `SessionEvent::Compaction`
      before the inner `would_overflow` re-check; no other `compact()` call exists.
- [ ] `event_type_str` (mcp) and the `event_kind` test helper both map
      `Compaction` → `"compaction"`.
- [ ] `cargo build` is clean (both crates) with zero new warnings.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` is clean.
- [ ] `cargo fmt --all --check` is clean (run `rustfmt` only on files you touched;
      do **not** run the writing form of `cargo fmt --all`).
- [ ] `cargo test` passes (existing + the new emit test).

## Test plan

Add **one** loop-integration test in `executor/src/agent/mod.rs`'s `#[cfg(test)] mod
tests`, modeled on `logs_metrics_event_per_turn` (`agent/mod.rs:3318`). Use a tiny
budget so overflow fires on turn 0: `would_overflow` is `estimate >= ceiling`, and the
system prompt alone is hundreds of tokens, so `Budget::new(10)` overflows immediately
— `compact()` runs (emitting the record), the re-check still overflows (system prompt
≫ 10), and the loop returns `budget_exceeded` (an `Ok(..)` result, so `.await.unwrap()`
is fine). The model is never called, so the `MockAiClientScript` need not script a
usable turn.

- `logs_compaction_event_when_budget_overflows` — drive `execute_phase` with
  `Budget::new(10)`; read the session records (the `records(dir.path())` helper the
  metrics test uses); filter for `SessionEvent::Compaction`. Assert:
  - at least one `Compaction` record exists (on failure, print the event kinds via
    `event_kind`, as the metrics test does);
  - for that record, `tokens_before >= tokens_after` and `tokens_before > 0`.

  Pin **behavior, not exact counts** — do not assert specific `messages_evicted` /
  `tokens_after` numbers (they depend on the tokenizer and prompt size). The negative
  side is already covered: when no overflow occurs, `compact()` is never called and no
  record is emitted — `logs_metrics_event_per_turn` (which uses `Budget::new(100_000)`)
  exercises that path, so no separate "no compaction" test is required.

## End-to-end verification

This phase ships a runtime-loadable artifact (a new JSONL record the running executor
emits and the `executor_log_search`/`executor_log_tail` tools can filter on). Verify
against the real serialization, not only the loop test:

1. **Round-trip serialization** — the new emit test reads back real `SessionRecord`s
   parsed from the on-disk JSONL the loop wrote (the `records()` helper reads the
   actual log file), so a green test *is* the end-to-end proof that the variant
   serializes with `"event_type":"compaction"` and parses back. Paste the test output.
2. **Discriminant string** — confirm the serde tag is `compaction` by quoting one
   emitted line, or by asserting `event_type_str` returns `"compaction"` for a
   `Compaction` value in the test. State which you did in "Notes for review".

(There is no CLI surface to exercise in this phase — the dashboard render is
deferred. The "real artifact" is the JSONL record, verified via the round-trip above.)

## Authorizations

None. No new dependency (the change reuses `CompactionReport`, `log_event`, and serde
derives already present). No `docs/architecture.md` / `STANDARDS.md` / `WORKFLOW.md`
edit. Editing both `executor/` and `mcp/` is in-scope for this phase (the variant and
its consumer span the two crates).

## Out of scope

- **Any dashboard / `StatusSummary` change.** No new panel, no `summarize` arm. The
  render half is deferred to the M8 dashboard redesign (phase-09+).
- **A `PhaseRun` / scorecard field for compaction.** The roadmap notes compaction
  could feed the scorecard later; that's a separate change with its own blast radius.
  This phase only emits the live JSONL record.
- **Changing `compact()` itself** — its logic, the `CompactionReport` shape, or the
  `TARGET_FRACTION`. Consume the report as-is.
- **A `tokens_freed` field on the variant.** It is `tokens_before - tokens_after`;
  derive it in consumers, don't store it.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
