# Phase 01: Telemetry store foundation

**Milestone:** M35 — Metrics & Cost Accounting Overhaul
**Status:** todo
**Depends on:** none
**Estimated diff:** ~350 lines
**Tags:** language=rust, kind=refactor, size=m

## Goal

Give the cross-project telemetry store an explicit schema version, make
telemetry recording **on by default** (XDG data dir), and delete the two
pieces of legacy schema tolerance the versioning replaces: the hand-rolled
`TokenBreakdown` legacy deserializer and the never-populated
`TierTelemetry.doc_level` field. Later M35 phases build the shared cost core
and CLI surfaces on top of this cleaned store.

## Architecture references

Read before starting:

- `docs/architecture.md` § "Model effectiveness metrics & the scorecard" —
  what the store records and why.
- `docs/dev/milestones/M35-metrics-cost-accounting/README.md` — the milestone
  plan and the explicit user decision that backward compatibility is waived
  (pre-M35 records go dark; that is intended, not a bug).

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

**The store** (`executor/src/store/telemetry.rs`): one JSONL file,
`<telemetry_dir>/phase_runs.jsonl`, multiplexes three record types
disambiguated by a `record` string field: `PhaseRun` (no `record` field),
`PhaseReview` (`record == "review"`), `ArchitectActivity`
(`record == "architect_activity"`). Writers: `append` (line 184),
`append_review` (~line 358), `append_architect_activity` (~line 505).
Readers: `read` (line 200), `read_reviews` (line 375),
`read_architect_activities`. `append` today:

```rust
pub fn append(telemetry_dir: &Path, run: &PhaseRun) -> std::io::Result<PathBuf> {
    use std::io::Write;

    std::fs::create_dir_all(telemetry_dir)?;
    let path = telemetry_dir.join("phase_runs.jsonl");
    let line = serde_json::to_string(run).map_err(std::io::Error::other)?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    file.write_all(line.as_bytes())?;
    file.write_all(b"\n")?;
    Ok(path)
}
```

and `read` deserializes each line straight into the record type, skipping
corrupt lines:

```rust
Ok(content
    .lines()
    .filter(|l| !l.trim().is_empty())
    .filter_map(|l| serde_json::from_str::<PhaseRun>(l).ok())
    .collect())
```

**No schema version exists.** Evolution has been `#[serde(default)]` plus two
legacy shims this phase deletes:

- `TokenBreakdown` (`executor/src/ai/types.rs:42-134`) has a hand-written
  `impl<'de> Deserialize` visitor (lines 67-134) that also accepts the legacy
  `{"prompt_tokens": …, "completion_tokens": …}` shape. Its fields already
  carry `#[serde(default)]`; the derive is only missing from the
  `#[derive(...)]` list because the manual impl exists.
- `TierTelemetry` (`executor/src/store/telemetry.rs:107-114`) carries
  `doc_level: Option<u8>` — documented "`None` until doc levels are wired" and
  never populated anywhere (`emit_phase_run` at
  `executor/src/agent/metrics.rs:132-135` sets only `tier`, via
  `..Default::default()`).

**Telemetry config** (`executor/src/config.rs:351-355`):

```rust
/// Cross-project telemetry store. `None` disables telemetry emission.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TelemetryConfig {
    pub dir: Option<PathBuf>,
}
```

`Config::load` (config.rs:528-542) tilde-expands the dir; `expand_tilde`
(config.rs:514-526) is the precedent for env-based path logic. The injected-
getter pattern for env access lives in `apply_overrides` (config.rs:544):
`get: impl Fn(&str) -> Option<String>`. Default-fn serde precedent:
`default_first_token_timeout_secs` (config.rs:369, 408).

Downstream, every consumer keys off `cfg.telemetry.dir` being `Some`
(e.g. `mcp/src/runner.rs:122-128` `resolve_telemetry_dir`;
`mcp/src/runs.rs:164-168`), so defaulting the dir at load time changes
behavior everywhere without touching any consumer.

## Spec

### Task 1 — Stamp a schema version at the write boundary

In `executor/src/store/telemetry.rs`, add:

```rust
/// Version stamped on every record this build writes; readers ignore records
/// at any other version (including pre-M35 records, which have none).
pub const TELEMETRY_SCHEMA_VERSION: u32 = 1;
```

**Do NOT add a `schema_version` field to `PhaseRun`, `PhaseReview`, or
`ArchitectActivity`.** There are ~30 `PhaseRun { … }` struct literals across
seven files (`telemetry.rs` tests ×11, `agent/metrics.rs`,
`mcp/src/scorecard_tests.rs` ×5, `scorecard_cli.rs` ×2, `profile_cli.rs` ×2,
`runs.rs` ×5, `profile.rs` ×4) and a new required field breaks every one —
the wide-blast-radius mutation WORKFLOW says to avoid. Instead stamp the
version into the serialized JSON at the write boundary. In `append`:

```rust
let mut value = serde_json::to_value(run).map_err(std::io::Error::other)?;
value["schema_version"] = TELEMETRY_SCHEMA_VERSION.into();
let line = serde_json::to_string(&value).map_err(std::io::Error::other)?;
```

Apply the same three-line change in `append_review` and
`append_architect_activity` (they have the same serialize-then-write shape).

### Task 2 — Version-gate the read boundary

Change all three readers (`read`, `read_reviews`,
`read_architect_activities`) to parse each line as `serde_json::Value`
first, keep only lines whose `schema_version` equals
`TELEMETRY_SCHEMA_VERSION`, then deserialize to the record type. For `read`:

```rust
Ok(content
    .lines()
    .filter(|l| !l.trim().is_empty())
    .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
    .filter(|v| {
        v.get("schema_version").and_then(serde_json::Value::as_u64)
            == Some(TELEMETRY_SCHEMA_VERSION as u64)
    })
    .filter_map(|v| serde_json::from_value::<PhaseRun>(v).ok())
    .collect())
```

`read_reviews` / `read_architect_activities` keep their existing `record`-tag
filter **in addition to** the version gate. Update the doc comments on all
three readers and on the module-level docs to state the rule: a record with a
missing or non-current `schema_version` is skipped (that is how pre-M35
history is retired — deliberately, per the milestone README).

### Task 3 — Telemetry on by default at an XDG data dir

In `executor/src/config.rs`:

1. Add an `enabled` flag to `TelemetryConfig`, defaulting to `true` (serde
   and `Default` both):

   ```rust
   /// Cross-project telemetry store. On by default: when `dir` is unset it
   /// resolves to `$XDG_DATA_HOME/rexymcp` (else `$HOME/.local/share/rexymcp`).
   /// `enabled = false` turns recording and reading off entirely.
   #[derive(Debug, Clone, Serialize, Deserialize)]
   #[serde(default)]
   pub struct TelemetryConfig {
       pub dir: Option<PathBuf>,
       pub enabled: bool,
   }

   impl Default for TelemetryConfig {
       fn default() -> Self {
           Self { dir: None, enabled: true }
       }
   }
   ```

   (Drop `Default` from the derive list — the manual impl replaces it. No
   struct literals of `TelemetryConfig` exist outside the definition; tests
   assign through `cfg.telemetry.dir = …`, which keeps compiling.)

2. Add a pure helper next to `expand_tilde`, taking the injected env getter
   (the `apply_overrides` pattern) so tests stay hermetic:

   ```rust
   /// Default telemetry dir when `[telemetry] dir` is unset:
   /// `$XDG_DATA_HOME/rexymcp`, else `$HOME/.local/share/rexymcp`, else `None`
   /// (no home ⇒ telemetry silently off, matching today's no-dir behavior).
   fn default_telemetry_dir(get: impl Fn(&str) -> Option<String>) -> Option<PathBuf>
   ```

3. In `Config::load`, after the existing tilde-expansion line
   (config.rs:539), resolve in this order:

   ```rust
   if !config.telemetry.enabled {
       config.telemetry.dir = None;
   } else if config.telemetry.dir.is_none() {
       config.telemetry.dir = default_telemetry_dir(|k| std::env::var(k).ok());
   }
   ```

   An explicitly configured `dir` always wins over the XDG default.
   Resolution lives **only** in `load` — `Config::default()` keeps
   `dir: None`, so unit tests constructing configs directly stay hermetic
   and telemetry-off, exactly as today.

### Task 4 — Delete the legacy `TokenBreakdown` deserializer

In `executor/src/ai/types.rs`: add `Deserialize` to the derive list on
`TokenBreakdown` (line 42) and delete the entire manual
`impl<'de> Deserialize` block (lines 67-134). The fields already carry
`#[serde(default)]`, so `{}` still deserializes to all-zero. The legacy
`{"prompt_tokens": …}` shape now deserializes to **all-zero** (unknown fields
are ignored) instead of being mapped — correct, because Task 2's version gate
already excludes every record old enough to carry that shape. Replace the
legacy-shape test around `types.rs:247` per the Test plan. Do **not** touch
`parse_openai_usage` in `executor/src/ai/backends/openai.rs` — that reads the
*API's* `prompt_tokens` wire format, which is current and unrelated.

### Task 5 — Drop `TierTelemetry.doc_level`

In `executor/src/store/telemetry.rs`: remove the `doc_level` field
(line 113) and scrub it from the struct's doc comment (lines 100-106) and
the `PhaseRun.tier_telemetry` field comment (line 176). Fix the two
affected tests: the literal at ~line 1049-1051 (`doc_level: Some(2)`) and
the legacy-shape JSON fixture at ~line 1071 (which asserts a
`tier_telemetry` object containing `doc_level`/`escalation_count`/
`architect_*_tokens` still parses — retire or rewrite it as a current-shape
test per the Test plan). `emit_phase_run` (`executor/src/agent/metrics.rs:132`)
uses `..Default::default()` and keeps compiling unchanged. A repo-wide grep
for `doc_level` must come back empty when done (docs outside your scope
excepted).

## Acceptance criteria

- [ ] `grep -rn "doc_level" executor/src mcp/src` returns nothing.
- [ ] `grep -n "impl<'de> Deserialize" executor/src/ai/types.rs` returns
      nothing.
- [ ] A line appended by `telemetry::append` contains `"schema_version":1`.
- [ ] `telemetry::read` on a file containing one pre-M35 line (no
      `schema_version`) and one current line returns exactly the current one.
- [ ] `Config::load` on a TOML with no `[telemetry]` section resolves
      `telemetry.dir` to `<XDG or HOME>/…/rexymcp`; with
      `[telemetry] enabled = false` it resolves to `None`; with an explicit
      `dir` it keeps that dir.
- [ ] `cargo fmt --all --check`, `cargo build`,
      `cargo clippy --all-targets --all-features -- -D warnings`, and
      `cargo test` all pass.

## Test plan

In `executor/src/store/telemetry.rs` tests (existing `#[cfg(test)] mod`,
`tempfile::TempDir` per the module's precedent):

- `append_stamps_schema_version` — append a `PhaseRun`, read the raw line,
  assert it contains `"schema_version":1`.
- `read_skips_records_without_current_schema_version` — hand-write one line
  with no `schema_version`, one with `"schema_version":999`, and append one
  current record; `read` returns only the current one.
- `read_reviews_version_gates_and_keeps_record_tag_filter` — a current-version
  `PhaseRun` line and a **legacy** (unversioned) review line are both
  excluded; a current review line is returned.
- `read_architect_activities_version_gates` — same shape for activities.
- Rewrite the ~line 1071 legacy-`tier_telemetry` fixture test as
  `tier_telemetry_current_shape_roundtrips` — a current `TierTelemetry`
  (`tier` only) survives a serialize/deserialize roundtrip.

In `executor/src/config.rs` tests:

- `default_telemetry_dir_prefers_xdg_data_home` — getter returns
  `XDG_DATA_HOME=/x` ⇒ `Some("/x/rexymcp")` even when `HOME` is also set.
- `default_telemetry_dir_falls_back_to_home` — only `HOME=/h` ⇒
  `Some("/h/.local/share/rexymcp")`.
- `default_telemetry_dir_none_without_home` — getter returns `None` for both
  ⇒ `None`.
- `telemetry_disabled_clears_dir_on_load` — TOML with
  `[telemetry] enabled = false` and an explicit `dir` loads with
  `telemetry.dir == None` (negative: explicit dir does NOT survive
  `enabled = false`).
- `explicit_telemetry_dir_survives_load` — TOML with `[telemetry] dir` set
  loads with that dir (tilde-expanded), not the XDG default.

In `executor/src/ai/types.rs` tests:

- `legacy_usage_shape_no_longer_mapped` — replace the legacy test at ~247:
  `{"prompt_tokens":1500,"completion_tokens":800}` now deserializes to
  all-zero (`input_tokens == 0`), pinning that the mapping is gone.
- `token_breakdown_current_shape_roundtrips` — the four-field shape
  roundtrips unchanged.

## End-to-end verification

Build the real binary and confirm the default-on behavior end to end
(read-only against the store; do not write to the real store):

```bash
cargo build
printf '[executor]\nprovider = "openai"\nmodel = "m"\nbase_url = "http://localhost:9/v1"\n' > /tmp/m35p1.toml
cargo run -p rexymcp -- runs --config /tmp/m35p1.toml
```

Expected: **no** `telemetry disabled` error — the command reads the XDG
default store (an empty/missing store prints an empty table or "no runs",
which is fine). Then:

```bash
printf '[telemetry]\nenabled = false\n' >> /tmp/m35p1.toml
cargo run -p rexymcp -- runs --config /tmp/m35p1.toml
```

Expected: the `telemetry disabled` error path. Paste both outputs in the
completion Update Log.

## Authorizations

None. (No new dependencies — the XDG paths come from `std::env::var`, the
`expand_tilde` precedent; no `dirs` crate.)

## Out of scope

- `parse_openai_usage` / anything in `executor/src/ai/backends/` — the API
  wire format is not the telemetry schema.
- The session-log store (`store/sessions/`) — versioning it is not part of
  this phase.
- Any new metrics capture (tok/s, `output_bytes`) — phase-02.
- The shared cost core, pricing table, and any CLI column/output changes —
  phases 03-07. The `telemetry disabled` error strings in
  `mcp/src/{runs,review,journal}.rs` / `server.rs` stay as they are.
- Migrating or rewriting existing `phase_runs.jsonl` files — pre-M35 records
  simply stop being read; leave them on disk.
- `docs/architecture.md`, `STANDARDS.md`, `WORKFLOW.md`, other phase docs.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
