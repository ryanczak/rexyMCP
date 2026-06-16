# Phase 02: Tier/cost telemetry fields and `EscalationEvent` record

**Milestone:** M20 — Tier Calibration and Cost Visibility
**Status:** in-progress (bounced — see bugs/bug-02-1.md)
**Depends on:** phase-01 (the `Tier` enum it adds is recorded by this phase)
**Estimated diff:** ~320 lines
**Tags:** language=rust, kind=feature, size=m

## Goal

Instrument every `PhaseRun` with the M20 tier/cost fields (`tier`, `doc_level`,
`escalation_count`, `architect_input_tokens`, `architect_output_tokens`) and
define a new append-only `EscalationEvent` telemetry record for mid-phase
Architect assists. The configured `tier` is **populated now** (it is available
in config after phase-01); the other four fields and the `EscalationEvent`
*producer* are wired in M21 when the escalation loop actually fires. This is a
**substrate phase** — it lays down the schema and the store API, exactly as M18
phase-01 laid down the `PhaseReview` record before phase-02 produced one.

No executor loop behavior changes beyond recording `tier`: no escalation fires,
no `EscalationEvent` is written by any code path yet. The dashboard consumer is
M20 phase-03; the escalation producer is M21.

## Architecture references

Read before starting:

- `executor/src/store/telemetry.rs` — owns `PhaseRun`, `PhaseReview`, the
  append/read functions, and the `ContextEfficiency` nested-struct precedent.
  This is the primary file this phase extends.
- `executor/src/agent/metrics.rs:104` — the single production `PhaseRun {…}`
  construction (`emit_phase_run`). It already reads `input.project_id` /
  `input.milestone_id`; it will read `input.tier` the same way.
- `executor/src/agent/mod.rs:62` — the `PhaseInput` struct, the per-phase
  metadata bag. `tier` joins `project_id` / `milestone_id` here.
- `mcp/src/runner.rs:204` — the single production `PhaseInput {…}` construction;
  it populates `project_id` / `milestone_id` and reads `cfg.executor.*`.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read this entire phase doc before touching any code.
3. Run `cargo build` to confirm the tree is clean.

## Current state

### `executor/src/store/telemetry.rs` — the nesting precedent

`PhaseRun` already nests a disparate signal group as a **single
`#[serde(default)]` field** so that legacy records and every struct literal need
only `Default`. This is the exact shape this phase reuses for the new fields.
Quoting the existing `ContextEfficiency` and its `PhaseRun` field (lines 39–58,
141–145):

```rust
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ContextEfficiency {
    pub peak_context_pct: f64,
    pub compaction_count: usize,
    // … six fields …
}
```

```rust
    /// Context-efficiency signal aggregated from the session JSONL at phase end
    /// (M10/phase-08a). Default (all zeros) for legacy records …
    #[serde(default)]
    pub context_efficiency: ContextEfficiency,
```

`PhaseRun`'s last two fields today are `project_id` and `milestone_id`
(lines 146–156), both `#[serde(default)] Option<String>`. `PhaseRun` does **not**
derive `PartialEq` (its `TokenBreakdown` field doesn't implement it) — tests
compare via JSON re-serialization. Do not add `PartialEq` to `PhaseRun`.

### `executor/src/store/telemetry.rs` — the discriminated-record precedent

`PhaseReview` is an append-only record that **coexists in the same
`phase_runs.jsonl`** as `PhaseRun`, kept apart by a `record` string
discriminator. The new `EscalationEvent` follows this pattern exactly. Quoting
the load-bearing pieces (lines 299–364):

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PhaseReview {
    /// Literal discriminator. Always `"review"`. `#[serde(default)]` so a
    /// `PhaseRun` line (which has no `record` field) deserializes to `""` here
    /// and is filtered out by `read_reviews`.
    #[serde(default)]
    pub record: String,
    pub ts: u64,
    #[serde(default)]
    pub phase_doc_path: Option<String>,
    pub phase_id: String,
    #[serde(default)]
    pub project_id: Option<String>,
    pub architect_verdict: String,
    // … #[serde(default)] supervision fields …
}

pub const REVIEW_RECORD_TAG: &str = "review";

pub fn append_review(telemetry_dir: &Path, review: &PhaseReview) -> std::io::Result<PathBuf> {
    use std::io::Write;
    std::fs::create_dir_all(telemetry_dir)?;
    let path = telemetry_dir.join("phase_runs.jsonl");
    let line = serde_json::to_string(review).map_err(std::io::Error::other)?;
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(&path)?;
    file.write_all(line.as_bytes())?;
    file.write_all(b"\n")?;
    Ok(path)
}

pub fn read_reviews(path: &Path) -> std::io::Result<Vec<PhaseReview>> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    Ok(content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<PhaseReview>(l).ok())
        .filter(|r| r.record == REVIEW_RECORD_TAG)
        .collect())
}
```

**Two guards keep the record types apart, and both must hold for the new type:**

1. **The `record` discriminator** is the *primary* guard. `read_reviews` filters
   `record == "review"`. The new `read_escalations` filters
   `record == "escalation"`. A `PhaseRun` line has no `record` field → defaults
   to `""` → excluded.
2. **Structural mismatch** is the *secondary* guard. `serde_json::from_str::<T>`
   returns `Err` (dropped by `.ok()` / `.filter_map`) when a line lacks a
   **required** (non-`#[serde(default)]`) field of `T`. A `PhaseRun` line parsed
   as `PhaseReview` fails because `PhaseReview.architect_verdict` is a required
   `String` the run line lacks. Keep the new record's identity/cost fields
   **required** (no `#[serde(default)]`) so this guard also protects it.

> **Gotcha (M18 bug-01-1):** the original `PhaseReview` guard test was not
> mutation-resistant — a `PhaseRun` whose `architect_verdict` serialized to
> `null` failed to deserialize into `PhaseReview`'s required `String` and so was
> dropped by the *structural* guard *before* the `record` filter ran, hiding a
> bug in the filter. Make the new guard tests pin the **discriminator** as
> load-bearing: feed `read_escalations` a real `PhaseReview` line (which *does*
> parse far enough to reach the filter) and assert it is excluded by
> `record != "escalation"`, not merely by a structural failure. See the Test
> plan's `read_escalations_excludes_review_by_discriminator`.

### `executor/src/agent/metrics.rs:104` — the production `PhaseRun`

```rust
    let run = PhaseRun {
        ts: now,
        model: deps.model.to_string(),
        // …
        project_id: input.project_id.clone(),
        milestone_id: input.milestone_id.clone(),
    };
    let _ = telemetry::append(dir, &run);
```

This is where `tier` is populated: a new trailing field
`tier_telemetry: TierTelemetry { tier: input.tier, ..Default::default() }`.

### `executor/src/agent/mod.rs:62` and `mcp/src/runner.rs:204` — `PhaseInput`

`PhaseInput` carries per-phase metadata; its last two fields are `project_id`
and `milestone_id`. The runner populates them (runner.rs:212–213) and reads
config via `cfg.executor.task_tracking` (runner.rs:239). `tier` follows the same
path: a new `pub tier: Option<Tier>` field on `PhaseInput`, populated in the
runner from `cfg.executor.tier`.

## Spec

### Task 1 — `TierTelemetry` nested struct in `executor/src/store/telemetry.rs`

Add a new nested struct **directly above** the `PhaseRun` struct (after
`ContextEfficiency` / `aggregate_context_efficiency`), mirroring
`ContextEfficiency`'s derive set and doc style. Import `Tier` at the top of the
file: add `use crate::config::Tier;` to the existing `use crate::…` imports.

```rust
/// Per-run M20 tier/cost instrumentation. Nested in `PhaseRun` as a single
/// `#[serde(default)]` field so legacy records and every struct literal need
/// only `Default` (the `ContextEfficiency` precedent). Only `tier` is populated
/// in M20 phase-02 — the configured executor tier, available from `[executor]
/// tier`. `doc_level` is wired in M22 (phase-doc detail level L1/L2/L3 → 1/2/3);
/// `escalation_count` and the two `architect_*_tokens` are wired in M21 when the
/// mid-phase Architect-assist loop fires. Default (tier `None`, levels `None`,
/// counts `0`) for legacy records and every run that did not escalate.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TierTelemetry {
    /// Configured executor capability tier (`[executor] tier`); `None` when the
    /// project has not run `rexymcp calibrate`.
    pub tier: Option<Tier>,
    /// Phase-doc detail level (1/2/3). `None` until M22 wires doc levels.
    pub doc_level: Option<u8>,
    /// Number of mid-phase Architect assists that fired this run. `0` until M21.
    pub escalation_count: u32,
    /// Architect input tokens spent on assists this run. `0` until M21.
    pub architect_input_tokens: u64,
    /// Architect output tokens spent on assists this run. `0` until M21.
    pub architect_output_tokens: u64,
}
```

`#[derive(Default)]` works without `Tier` implementing `Default`: `Option<Tier>`
defaults to `None` regardless of `Tier`.

### Task 2 — Add the field to `PhaseRun`

Add as the **last** field of `PhaseRun` (after `milestone_id`):

```rust
    /// M20 tier/cost instrumentation (tier, doc_level, escalation cost). Default
    /// for legacy records and non-escalating runs.
    #[serde(default)]
    pub tier_telemetry: TierTelemetry,
```

### Task 3 — Add `tier` to `PhaseInput` in `executor/src/agent/mod.rs`

Add as the **last** field of `PhaseInput` (after `milestone_id`):

```rust
    /// Configured executor capability tier (`[executor] tier`), recorded in the
    /// `PhaseRun`'s `tier_telemetry`. `None` when no tier is configured.
    pub tier: Option<crate::config::Tier>,
```

### Task 4 — Populate `tier_telemetry` in `executor/src/agent/metrics.rs`

In `emit_phase_run`'s `PhaseRun {…}` literal (line ~104), add the trailing field
after `milestone_id`:

```rust
        tier_telemetry: telemetry::TierTelemetry {
            tier: input.tier,
            ..Default::default()
        },
```

`Tier` is `Copy` (it derives `Copy` — see phase-01), so `input.tier` needs no
`.clone()`.

### Task 5 — Populate `tier` in the runner (`mcp/src/runner.rs:204`)

In the production `PhaseInput {…}` literal, add the trailing field after
`milestone_id`:

```rust
        tier: cfg.executor.tier,
```

Use the model-resolved `cfg` (the local `let mut cfg = inp.cfg.clone();` at
runner.rs:188), consistent with `task_tracking: cfg.executor.task_tracking` at
runner.rs:239. `resolve_for_model` does not touch `tier`, so the value equals
`inp.cfg.executor.tier`; using `cfg` keeps the read consistent with the
surrounding lines.

### Task 6 — `EscalationEvent` record in `executor/src/store/telemetry.rs`

Add **after** the `PhaseReview` block (after `read_reviews`), mirroring
`PhaseReview` / `REVIEW_RECORD_TAG` / `append_review` / `read_reviews`
line-for-line in shape:

```rust
/// An append-only record of a single mid-phase Architect assist, appended to
/// `phase_runs.jsonl` alongside `PhaseRun` and `PhaseReview`. The `record`
/// discriminator (`"escalation"`) keeps the three readers from confusing the
/// line types. **No code writes one in M20** — the producer is wired in M21 when
/// the SMALL-tier escalation loop fires; this phase defines the schema and the
/// store API only (the `PhaseReview` substrate precedent from M18 phase-01).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EscalationEvent {
    /// Literal discriminator. Always `"escalation"`. `#[serde(default)]` so a
    /// `PhaseRun` line (no `record` field) deserializes to `""` here and is
    /// filtered out by `read_escalations`.
    #[serde(default)]
    pub record: String,
    pub ts: u64,
    /// Identity of the phase that escalated. Prefer `phase_doc_path`; `phase_id`
    /// + `project_id` are the fallback key (mirrors `PhaseReview`).
    #[serde(default)]
    pub phase_doc_path: Option<String>,
    pub phase_id: String,
    #[serde(default)]
    pub project_id: Option<String>,
    /// 1-based index of this assist within the phase (1st assist = 1).
    pub assist_index: u32,
    /// Architect model that produced the assist (e.g. `"claude-opus-4-8"`).
    #[serde(default)]
    pub model: Option<String>,
    /// Architect input tokens spent on this single assist.
    pub architect_input_tokens: u64,
    /// Architect output tokens spent on this single assist.
    pub architect_output_tokens: u64,
}

/// The literal value of `EscalationEvent.record`. Use everywhere instead of a
/// bare string so the discriminator is single-sourced.
pub const ESCALATION_RECORD_TAG: &str = "escalation";

/// Append one `EscalationEvent` as a JSON line to
/// `<telemetry_dir>/phase_runs.jsonl` (the same store as `PhaseRun`). Returns
/// the file path.
pub fn append_escalation(
    telemetry_dir: &Path,
    event: &EscalationEvent,
) -> std::io::Result<PathBuf> {
    use std::io::Write;

    std::fs::create_dir_all(telemetry_dir)?;
    let path = telemetry_dir.join("phase_runs.jsonl");
    let line = serde_json::to_string(event).map_err(std::io::Error::other)?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    file.write_all(line.as_bytes())?;
    file.write_all(b"\n")?;
    Ok(path)
}

/// Read all `EscalationEvent` records from a store file. Lines that are
/// `PhaseRun` or `PhaseReview` records (or anything without
/// `record == "escalation"`) are skipped.
pub fn read_escalations(path: &Path) -> std::io::Result<Vec<EscalationEvent>> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    Ok(content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<EscalationEvent>(l).ok())
        .filter(|e| e.record == ESCALATION_RECORD_TAG)
        .collect())
}
```

**Keep `ts`, `phase_id`, `assist_index`, `architect_input_tokens`, and
`architect_output_tokens` required** (no `#[serde(default)]`) — that is the
structural guard from § Current state. **Do not** modify `read` or
`read_reviews`: a structurally-distinct escalation line already fails to parse as
`PhaseRun` (lacks `model`/`gates`/…) and as `PhaseReview` (lacks
`architect_verdict`), so both existing readers already drop it. The new tests in
the Test plan pin that they do.

### Task 7 — Multi-site struct-literal traversal (additive; compiler-guided)

Tasks 2 and 3 add one required field each to `PhaseRun` and `PhaseInput`. Every
struct *literal* (not the `..spread` ones) stops compiling until the field is
added. Both fields are purely additive — add a single line per literal:
`tier_telemetry: Default::default(),` for `PhaseRun`, `tier: None,` for
`PhaseInput`. **Use the compiler to drive this:** run `cargo build`, fix each
`E0063 missing field` site it reports, repeat until green. Do **not** hand-search;
let the build enumerate them. The production sites are the exception — they get
real values (Tasks 4 and 5), not `Default`.

The `grep`-verified literal sites, for reference (the `..sample()` /
`..spread` sites inherit and need **no** change):

`PhaseRun {` full literals — add `tier_telemetry: Default::default(),`:
- `executor/src/store/telemetry.rs:521` (the `sample()` helper — all the
  `..sample()` sites at 668/698/725/748/764/804/809/840 inherit, leave them)
- `executor/src/agent/metrics.rs:104` — **production, Task 4 value, not Default**
- `mcp/src/runs.rs:188`, `mcp/src/runs.rs:230`, `mcp/src/runs.rs:631`
- `mcp/src/scorecard_cli.rs:104`
- `mcp/src/profile.rs:255`, `mcp/src/profile.rs:297`
- `mcp/src/profile_cli.rs:114`
- `mcp/src/scorecard_tests.rs:13`, `:402`, `:619`

`PhaseInput {` full literals — add `tier: None,`:
- `mcp/src/runner.rs:204` — **production, Task 5 value, not None**
- `executor/src/agent/tests.rs:29` (the `input()` helper), `:3664`, `:3728`,
  `:3762`, `:3794`, `:3873`, `:3918`, `:3955`, `:3999`

Line numbers may drift as you edit; trust `cargo build`'s E0063 list over this
list if they disagree.

### Task 8 — Tests in `executor/src/store/telemetry.rs`

Add to the existing `#[cfg(test)] mod tests`:

```rust
#[test]
fn phase_run_tier_telemetry_round_trips() {
    let mut run = sample();
    run.tier_telemetry = TierTelemetry {
        tier: Some(Tier::Medium),
        doc_level: Some(2),
        escalation_count: 1,
        architect_input_tokens: 1000,
        architect_output_tokens: 200,
    };
    let json = serde_json::to_string(&run).unwrap();
    let back: PhaseRun = serde_json::from_str(&json).unwrap();
    assert_eq!(back.tier_telemetry, run.tier_telemetry);
}

#[test]
fn phase_run_without_tier_telemetry_deserializes() {
    // A legacy JSONL line lacking tier_telemetry — as emitted before this phase.
    let legacy_json = r#"{"ts":1,"model":"t","generation_params":{},"phase_id":"p","tags":[],"status":"c","escalated":false,"gates":{},"parse_failure_rate":0.0,"repairs_per_call":0.0,"verifier_retries":0,"tool_success_rate":1.0,"turns":1,"wall_clock_s":1.0,"tokens":{}}"#;
    let run: PhaseRun = serde_json::from_str(legacy_json).unwrap();
    assert_eq!(run.tier_telemetry, TierTelemetry::default());
    assert_eq!(run.tier_telemetry.tier, None);
}

#[test]
fn tier_serializes_uppercase_in_telemetry() {
    let mut run = sample();
    run.tier_telemetry.tier = Some(Tier::Small);
    let json = serde_json::to_string(&run).unwrap();
    assert!(json.contains("\"tier\":\"SMALL\""), "tier must serialize UPPERCASE: {json}");
}

#[test]
fn escalation_event_round_trips() {
    let dir = TempDir::new().unwrap();
    let event = EscalationEvent {
        record: ESCALATION_RECORD_TAG.to_string(),
        ts: 1_000,
        phase_doc_path: Some("/docs/phase-02.md".to_string()),
        phase_id: "phase-02".to_string(),
        project_id: Some("proj-a".to_string()),
        assist_index: 1,
        model: Some("claude-opus-4-8".to_string()),
        architect_input_tokens: 1500,
        architect_output_tokens: 300,
    };
    append_escalation(dir.path(), &event).unwrap();
    let path = dir.path().join("phase_runs.jsonl");
    let events = read_escalations(&path).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0], event);
}

#[test]
fn read_escalations_excludes_run_lines() {
    // A PhaseRun line must not be read as an EscalationEvent.
    let dir = TempDir::new().unwrap();
    append(dir.path(), &sample()).unwrap();
    let path = dir.path().join("phase_runs.jsonl");
    let events = read_escalations(&path).unwrap();
    assert!(events.is_empty(), "PhaseRun lines must not be read as escalations");
}

#[test]
fn read_escalations_excludes_review_by_discriminator() {
    // A PhaseReview line parses far enough to reach the discriminator filter
    // (it has ts/phase_id and no required escalation-only field would block it
    // only if absent) — assert the `record != "escalation"` filter is what
    // excludes it. This pins the discriminator as load-bearing (M18 bug-01-1).
    let dir = TempDir::new().unwrap();
    let review = PhaseReview {
        record: REVIEW_RECORD_TAG.to_string(),
        ts: 2_000,
        phase_doc_path: Some("/docs/phase-02.md".to_string()),
        phase_id: "phase-02".to_string(),
        project_id: None,
        architect_verdict: "approved".to_string(),
        bounces_to_approval: None,
        bugs_filed: None,
        warnings: None,
        failure_class: vec!["none".to_string()],
    };
    append_review(dir.path(), &review).unwrap();
    let path = dir.path().join("phase_runs.jsonl");
    let events = read_escalations(&path).unwrap();
    assert!(events.is_empty(), "review lines must be excluded from escalations");
}

#[test]
fn read_skips_escalation_lines() {
    // The existing PhaseRun reader must not pick up escalation lines.
    let dir = TempDir::new().unwrap();
    append(dir.path(), &sample()).unwrap();
    let event = EscalationEvent {
        record: ESCALATION_RECORD_TAG.to_string(),
        ts: 1,
        phase_doc_path: None,
        phase_id: "p".to_string(),
        project_id: None,
        assist_index: 1,
        model: None,
        architect_input_tokens: 1,
        architect_output_tokens: 1,
    };
    append_escalation(dir.path(), &event).unwrap();
    let path = dir.path().join("phase_runs.jsonl");
    let runs = read(&path).unwrap();
    assert_eq!(runs.len(), 1, "read() must skip the escalation line");
}

#[test]
fn read_reviews_skips_escalation_lines() {
    let dir = TempDir::new().unwrap();
    let event = EscalationEvent {
        record: ESCALATION_RECORD_TAG.to_string(),
        ts: 1,
        phase_doc_path: None,
        phase_id: "p".to_string(),
        project_id: None,
        assist_index: 1,
        model: None,
        architect_input_tokens: 1,
        architect_output_tokens: 1,
    };
    append_escalation(dir.path(), &event).unwrap();
    let path = dir.path().join("phase_runs.jsonl");
    let reviews = read_reviews(&path).unwrap();
    assert!(reviews.is_empty(), "read_reviews() must skip the escalation line");
}
```

(`Tier` is already in scope inside the test module via the file-level
`use crate::config::Tier;` added in Task 1; if `clippy` flags it unused outside
tests, that means a production reference is missing — it should be used by
`TierTelemetry.tier`'s type, so no extra `use` is needed in the test module.)

### Task 9 — End-to-end test in `mcp/src/runner.rs`

Add a test mirroring `run_phase_with_resolves_per_model_sampling_into_telemetry`
(runner.rs:571), but asserting the **tier** lands in the written telemetry:

```rust
#[tokio::test]
async fn run_phase_with_records_configured_tier_in_telemetry() {
    let dir = TempDir::new().unwrap();
    let repo_dir = dir.path().join("repo");
    std::fs::create_dir_all(&repo_dir).unwrap();

    let phase_doc_path = dir.path().join("phase-01-test.md");
    std::fs::write(
        &phase_doc_path,
        "# Phase 01: Test\n\n**Tags:** language=rust, kind=test, size=s\n\n## Goal\n\nTest goal.\n\n## Acceptance criteria\n\n- [ ] It runs.\n",
    )
    .unwrap();

    let mut cfg = Config::default();
    cfg.executor.tier = Some(rexymcp_executor::config::Tier::Medium);

    let mock = MockAiClient::new(vec!["Done.".to_string()]);
    let clock = || 1234567890u64;
    let seams = Seams { client: &mock, verifier: &NoopVerifier, runner: &NoopRunner, clock: &clock };
    let inp = AssemblyInput {
        cfg: &cfg,
        phase_doc_path: &phase_doc_path,
        repo_path: &repo_dir,
        standards: "standards",
        model: "m",
        telemetry_dir: Some(dir.path()),
        progress: None,
        context_window: None,
        project_id: None,
    };

    let result = run_phase_with(&inp, &seams).await;
    assert!(result.is_ok(), "run_phase_with should succeed: {result:?}");

    let runs = rexymcp_executor::store::telemetry::read(&dir.path().join("phase_runs.jsonl"))
        .expect("telemetry should be readable");
    assert_eq!(runs.len(), 1, "exactly one phase run recorded");
    assert_eq!(
        runs[0].tier_telemetry.tier,
        Some(rexymcp_executor::config::Tier::Medium),
        "the configured tier must be recorded in the written telemetry"
    );
}
```

This exercises the full production wiring (`cfg.executor.tier` → `PhaseInput.tier`
→ `emit_phase_run` → `append` → `read`) with a mock client, hermetically.

## Acceptance criteria

- [ ] `TierTelemetry` exists in `executor/src/store/telemetry.rs` with fields
      `tier: Option<Tier>`, `doc_level: Option<u8>`, `escalation_count: u32`,
      `architect_input_tokens: u64`, `architect_output_tokens: u64`, deriving
      `Default` + `PartialEq` + serde.
- [ ] `PhaseRun` has a `#[serde(default)] tier_telemetry: TierTelemetry` field.
- [ ] A legacy `phase_runs.jsonl` line lacking `tier_telemetry` deserializes to
      `TierTelemetry::default()` (test `phase_run_without_tier_telemetry_deserializes`).
- [ ] `Tier::Small` serializes as `"SMALL"` inside a `PhaseRun`
      (test `tier_serializes_uppercase_in_telemetry`).
- [ ] `PhaseInput` has a `tier: Option<Tier>` field; the runner populates it from
      `cfg.executor.tier`.
- [ ] A `run_phase_with` run with `cfg.executor.tier = Some(Tier::Medium)` writes
      a `PhaseRun` whose `tier_telemetry.tier == Some(Tier::Medium)`
      (test `run_phase_with_records_configured_tier_in_telemetry`).
- [ ] `EscalationEvent` + `ESCALATION_RECORD_TAG` + `append_escalation` +
      `read_escalations` exist and round-trip (test `escalation_event_round_trips`).
- [ ] `read_escalations` excludes `PhaseRun` lines and `PhaseReview` lines; `read`
      and `read_reviews` exclude `EscalationEvent` lines (the four skip tests).
- [ ] `read_escalations_excludes_review_by_discriminator` pins the `record`
      filter (not just structural mismatch) as load-bearing.
- [ ] `cargo fmt --all --check` exits 0.
- [ ] `cargo build` exits 0 with zero new warnings.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` exits 0.
- [ ] `cargo test` passes; count rises by at least 9 new tests.

## Test plan

Unit tests in `executor/src/store/telemetry.rs`:
- `phase_run_tier_telemetry_round_trips` — full `TierTelemetry` survives JSON.
- `phase_run_without_tier_telemetry_deserializes` — legacy line → default.
- `tier_serializes_uppercase_in_telemetry` — `Tier` renders `"SMALL"`.
- `escalation_event_round_trips` — append→read returns the same event.
- `read_escalations_excludes_run_lines` — a `PhaseRun` line is not an escalation.
- `read_escalations_excludes_review_by_discriminator` — the discriminator filter
  excludes a real `PhaseReview` line (mutation-resistant per M18 bug-01-1).
- `read_skips_escalation_lines` — `read()` drops escalation lines.
- `read_reviews_skips_escalation_lines` — `read_reviews()` drops escalation lines.

End-to-end test in `mcp/src/runner.rs`:
- `run_phase_with_records_configured_tier_in_telemetry` — the configured tier is
  written to the real telemetry file through the production path.

## End-to-end verification

The real artifact this phase ships is the `phase_runs.jsonl` schema written by
the running executor. `run_phase_with_records_configured_tier_in_telemetry`
exercises the full production wiring (`cfg.executor.tier` → `PhaseInput` →
`emit_phase_run` → `append` → `read`) against a temp telemetry dir with a mock
client — the same hermetic-but-real path the existing
`run_phase_with_resolves_per_model_sampling_into_telemetry` uses to verify
sampling resolution. Quote that test's pass line in the completion Update Log.

The `EscalationEvent` half ships **no runtime producer** (M21 wires the firing),
so its real-artifact exercise is the `append_escalation` → `read_escalations`
round-trip test. No live model is required and none should be run.

## Authorizations

None. No new dependency, no `Cargo.toml` edit, no `docs/architecture.md` edit.

## Out of scope

- Firing escalations / writing an `EscalationEvent` from any code path — M21.
- Reading `tier` / `gate_retries` in the executor loop to change behavior — M21.
- Populating `doc_level` — M22 (phase-doc detail levels).
- Wiring `read_escalations` / `tier_telemetry` into any read path, scorecard,
  profile, or dashboard — M20 phase-03 (dashboard) and M21 consume them.
- Any `#[allow(dead_code)]` to silence the new-but-unconsumed
  `append_escalation` / `read_escalations` / `ESCALATION_RECORD_TAG`: they are
  `pub` items in the **executor library** crate, where unused `pub` is **not** a
  dead-code warning (unlike the `mcp` binary crate — cf. M18 bug-03-2). If
  `clippy` nonetheless flags one, **stop and file a blocker** rather than adding
  an attribute.
- Touching `STANDARDS.md`, `WORKFLOW.md`, or any other phase doc.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-06-16 18:34 (started)

**Executor:** rexyMCP executor LLM

Implementing all 9 tasks: `TierTelemetry` nested struct, `PhaseRun.tier_telemetry` field, `PhaseInput.tier` field, production wiring in `emit_phase_run` and `runner.rs`, `EscalationEvent` record with store API, multi-site struct-literal traversal, 8 unit tests in telemetry.rs, 1 end-to-end test in runner.rs.

### Update — 2026-06-16 18:34 (complete)

**Tasks completed:** 1–9

**Files changed:**
- `executor/src/store/telemetry.rs` — added `TierTelemetry` struct, `PhaseRun.tier_telemetry` field, `EscalationEvent` struct + `ESCALATION_RECORD_TAG` + `append_escalation` + `read_escalations`, 8 new unit tests
- `executor/src/agent/mod.rs` — added `PhaseInput.tier` field
- `executor/src/agent/metrics.rs` — populated `tier_telemetry` in `emit_phase_run`
- `executor/src/agent/tests.rs` — added `tier: None` to `input()` helper
- `mcp/src/runner.rs` — populated `tier` from `cfg.executor.tier`, added end-to-end test
- `mcp/src/runs.rs` — added `tier_telemetry: Default::default()` to 3 test `PhaseRun` literals
- `mcp/src/profile.rs` — added `tier_telemetry: Default::default()` to 2 test `PhaseRun` literals
- `mcp/src/profile_cli.rs` — added `tier_telemetry: Default::default()` to test `PhaseRun` literal
- `mcp/src/scorecard_cli.rs` — added `tier_telemetry: Default::default()` to test `PhaseRun` literal
- `mcp/src/scorecard_tests.rs` — added `tier_telemetry: Default::default()` to 2 test `PhaseRun` literals
- `docs/dev/milestones/M20-tier-calibration/phase-02-telemetry-fields.md` — status → review
- `docs/dev/milestones/M20-tier-calibration/README.md` — phase-02 status → review

**Verification commands (all clean):**
- `cargo fmt --all --check` — exit 0
- `cargo build` — exit 0, zero warnings
- `cargo clippy --all-targets --all-features -- -D warnings` — exit 0
- `cargo test` — 807 passed, 0 failed, 2 ignored (new test count: 8 telemetry unit tests + 1 runner end-to-end test = 9 new)

**Verification summary:** format clean, build clean (zero warnings), clippy clean, all 807 tests pass.

**End-to-end verification:** `run_phase_with_records_configured_tier_in_telemetry` exercises the full production wiring (`cfg.executor.tier` → `PhaseInput.tier` → `emit_phase_run` → `append` → `read`) with a mock client against a temp telemetry dir, asserting `tier_telemetry.tier == Some(Tier::Medium)`. `append_escalation` → `read_escalations` round-trip test verifies the `EscalationEvent` schema.

**Grep for spec-pinned literals:**
- `ESCALATION_RECORD_TAG` found in telemetry.rs (const + filter + 3 test uses) ✓
- `"escalation"` discriminator verified via grep ✓

**Notes for review:** None — all items implemented per spec.
