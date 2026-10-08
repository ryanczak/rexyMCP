# Phase 02: Refuse a zero stall threshold at config load

**Milestone:** M48 — Lossless Boundary, Loud Backstop, Exact Repetition
**Status:** in-progress
**Depends on:** none
**Estimated diff:** ~90 lines (one private validation fn + its call in `Config::load`; two doc comments; two template comments; two tests)
**Tags:** language=rust, kind=bugfix, size=s

## Goal

A `rexymcp.toml` that sets `read_only_stall_threshold = 0` — globally or
in a `[models."<id>"]` override — fails to load with an `Error::Config`
that names the key and the default. The read-only backstop is the only
terminator for non-mutating loops; a value that silently switches it off
has already cost one downstream project a 570-turn run, and the config
comment in that project records the `0` was set by mistake.

## Architecture references

Read before starting:

- `docs/architecture.md` § Status #48 — why `0` is refused.
- `docs/architecture.md` § Status #34 — what `NoProgressStall` is for.
- `docs/dev/milestones/M48-lossless-boundary/README.md` § Exit criteria,
  second bullet.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

Line numbers are current as of drafting (2026-10-08, master `f7613e4`);
re-derive with `grep -n` before editing.

**`executor/src/config.rs`**

- The field and its doc (`:161-165`):

  ```rust
  /// Consecutive read-only tool calls (no `patch`/`write_file` among them)
  /// before `NoProgressStall` hard-fail — a high pure-runaway backstop below
  /// `max_turns`. The novelty detector (below) is the early catch; this only
  /// bounds volume. The run resets on any file edit. `0` disables. Default 60.
  pub read_only_stall_threshold: usize,
  ```

  Default `60` (`:196`). The per-model override field
  `pub read_only_stall_threshold: Option<usize>` (`:226`) is applied by
  `resolve_for_model` (`:585-587`), which returns `()` — do not change its
  signature.
- `Config::load` (`:485-504`):

  ```rust
  pub fn load(path: &Path) -> Result<Self> {
      let mut config = Config::default();

      if path.exists() {
          let content = std::fs::read_to_string(path)?;
          let loaded: Config =
              toml::from_str(&content).map_err(|e| Error::Config(e.to_string()))?;
          config = loaded;
      }

      if !config.telemetry.enabled {
          config.telemetry.dir = None;
      } else if config.telemetry.dir.is_none() {
          config.telemetry.dir = default_telemetry_dir(|k| std::env::var(k).ok());
      } else {
          config.telemetry.dir = config.telemetry.dir.map(expand_tilde);
      }

      Ok(config)
  }
  ```

  `Error::Config(String)` is `executor/src/error.rs:8`. `load_with_env`
  (`:526`) calls `load`, so every binary entrypoint (`serve`, `doctor`,
  `run-phase`, …) goes through it. `models` is
  `HashMap<String, ModelOverride>` (`:251`).
- A load test to mirror — `context_output_filter_can_be_disabled`
  (`:1053-1082`): writes a minimal TOML to a `tempfile::tempdir()` path,
  calls `Config::load`, asserts. `load_malformed_toml_is_config_error`
  (`:656`) shows the `Error::Config` match shape:

  ```rust
  match err {
      Error::Config(_) => {}
      other => panic!("expected Error::Config, got {other:?}"),
  }
  ```

**`executor/src/governor/hard_fail.rs`** — `check_read_only_stall`
(`:352`) keeps its `if threshold == 0 { return None; }` guard; two agent
tests (`agent/tests.rs:1531`, `:1603`) construct `GovernorConfig` directly
with `read_only_stall_threshold: 0` to isolate other detectors, and
`read_only_stall_disabled_when_threshold_zero` pins the guard. None of
them go through `Config::load`. Only the doc comment (`:343-351`, the
sentence "`threshold == 0` disables") changes.

**`mcp/src/init.rs`** — the generated template says, at `:48`:

```
read_only_stall_threshold = 60    # consecutive non-mutating tool calls → hard-fail; resets on any patch/write_file (0 disables)
```

The template's own load test (`init.rs:273`, "generated config must
load") keeps passing because the value is 60.

**The binary today** accepts `0`: `rexymcp doctor --config <toml with 0>`
exits 0 (dry-run at drafting).

## Spec

### 1. Validation function

In `executor/src/config.rs`, add a private function next to `load`:

```rust
/// `read_only_stall_threshold = 0` switches off the only terminator for
/// non-mutating loops; it is refused rather than honoured so a stray zero
/// cannot silently leave a run to `max_turns`.
fn reject_zero_stall_threshold(config: &Config) -> Result<()> {
    if config.governor.read_only_stall_threshold == 0 {
        return Err(Error::Config(
            "[governor] read_only_stall_threshold = 0 disables the read-only \
             backstop; set a positive value (default 60) or remove the key"
                .to_string(),
        ));
    }
    for (model, over) in &config.models {
        if over.read_only_stall_threshold == Some(0) {
            return Err(Error::Config(format!(
                "[models.\"{model}\"] read_only_stall_threshold = 0 disables the \
                 read-only backstop; set a positive value (default 60) or remove the key"
            )));
        }
    }
    Ok(())
}
```

Call it in `load` immediately after the `config = loaded;` assignment
(inside the `if path.exists()` block — a missing file yields the default
60 and needs no check):

```rust
config = loaded;
reject_zero_stall_threshold(&config)?;
```

### 2. Doc comments

- `config.rs:165`: replace `` `0` disables. Default 60. `` with
  `` `0` is refused at load. Default 60. ``
- `hard_fail.rs:351`: replace `` `threshold == 0` disables. `` with
  `` `threshold == 0` disables (reachable only by constructing `GovernorConfig` directly — `Config::load` refuses it). ``

### 3. Init template

`mcp/src/init.rs:48`: replace `(0 disables)` with `(must be > 0; 0 is
refused at load)`. Leave the other four `0 disables` comments in the
template alone — they describe other knobs (`oscillation_window`,
`novelty_window`, …) whose zero is still honoured.

### 4. Tests (`config.rs` `mod tests`)

- `load_refuses_zero_read_only_stall_threshold` — write the minimal TOML
  from `context_output_filter_can_be_disabled` with a
  `[governor]\nread_only_stall_threshold = 0` section; assert
  `Config::load` is `Err(Error::Config(msg))` and `msg` contains
  `"[governor] read_only_stall_threshold"`. Then rewrite the same file with
  `read_only_stall_threshold = 30` and assert it loads with
  `cfg.governor.read_only_stall_threshold == 30` (the positive path is
  pinned in the same test so a validator that rejects everything cannot
  pass).
- `load_refuses_zero_read_only_stall_threshold_in_model_override` — the
  minimal TOML plus `[models."other"]\nread_only_stall_threshold = 0`;
  assert `Err(Error::Config(msg))` with `msg` containing `"other"` and
  `"read_only_stall_threshold"`.

### 5. Capture the end-to-end evidence

Run the block in § End-to-end verification **verbatim and unmodified**,
paste the artifact file into a new Update Log entry headed
`### Update — <date> (end-to-end verification)`, and append the
`PASTE MATCH` / `PASTE MISMATCH` line the self-check prints. On
`PASTE MISMATCH`, fix the pasted fence from the file and re-run the
self-check until it prints `PASTE MATCH`. The server-authored `(complete)`
entry does not satisfy this task.

## Acceptance criteria

- [ ] Test `load_refuses_zero_read_only_stall_threshold` passes.
- [ ] Test `load_refuses_zero_read_only_stall_threshold_in_model_override`
      passes.
- [ ] **Preservation criteria:** `read_only_stall_disabled_when_threshold_zero`
      (`hard_fail.rs`), `load_missing_file_returns_default`, and the
      `init.rs` "generated config must load" test still pass.
- [ ] `sed -n '1,/^#\[cfg(test)\]/p' executor/src/config.rs | grep -c 'fn reject_zero_stall_threshold('`
      prints `1` (now 0).
- [ ] `sed -n '1,/^#\[cfg(test)\]/p' executor/src/config.rs | grep -c 'reject_zero_stall_threshold(&config)'`
      prints `1` (now 0).
- [ ] `grep -c '#\[test\]' executor/src/config.rs` prints `65` — (now 63)
      + 2 named tests.
- [ ] `grep -c 'read_only_stall_threshold.*0 disables' mcp/src/init.rs`
      prints `0` (now 1).
- [ ] `grep -c 'is refused at load' executor/src/config.rs executor/src/governor/hard_fail.rs | awk -F: '{s+=$2} END {print s}'`
      prints `2` (now 0) — one doc comment in each file.
- [ ] In the E2E artifact, `zero_exit=1` and the line before it contains
      `read_only_stall_threshold`; `sixty_exit=0`.
- [ ] The E2E Update Log entry ends with the line `PASTE MATCH`.
- [ ] All four gates green (`cargo fmt --all --check`, `cargo build`,
      `cargo clippy --all-targets --all-features -- -D warnings`,
      `cargo test` — separate invocations).

## Test plan

- `load_refuses_zero_read_only_stall_threshold` in
  `executor/src/config.rs` — asserts a global `0` is `Error::Config`
  naming the key, and `30` loads as 30.
- `load_refuses_zero_read_only_stall_threshold_in_model_override` in
  `executor/src/config.rs` — asserts a per-model `0` is `Error::Config`
  naming the model id and the key.

## End-to-end verification

The real artifact is the `rexymcp` binary refusing the config. Run exactly
this block from the repo root; it is Spec's final task:

```bash
mkdir -p target/e2e
A=target/e2e/m48-phase-02.txt; : > "$A"
printf '[executor]\nprovider = "openai"\nmodel = "m"\nbase_url = "http://localhost:1234/v1"\n[commands]\n[budget]\ncontext_length = 32768\nmax_context_pct = 70\nmax_turns = 40\n[governor]\nread_only_stall_threshold = 0\n' > target/e2e/m48-zero.toml
sed 's/= 0$/= 60/' target/e2e/m48-zero.toml > target/e2e/m48-sixty.toml
cargo run -q -p rexymcp -- doctor --config target/e2e/m48-zero.toml 2>&1 | sed 's/\x1b\[[0-9;]*m//g' | tail -2 >> "$A"; echo "zero_exit=${PIPESTATUS[0]}" >> "$A"
cargo run -q -p rexymcp -- doctor --config target/e2e/m48-sixty.toml 2>&1 | sed 's/\x1b\[[0-9;]*m//g' | tail -1 >> "$A"; echo "sixty_exit=${PIPESTATUS[0]}" >> "$A"
cargo test -p rexymcp-executor load_refuses_zero_read_only_stall_threshold 2>&1 | sed 's/\x1b\[[0-9;]*m//g' | grep -E '^test |test result' >> "$A"; echo "exit=${PIPESTATUS[0]}" >> "$A"
```

Expected: the first two lines show the `Error:` text naming
`read_only_stall_threshold`, then `zero_exit=1`; a doctor report line then
`sixty_exit=0`; two `test … ok` lines, `test result: ok. 2 passed`,
`exit=0`. (At drafting, against the unmodified tree, both exits are `0`.)

Paste `target/e2e/m48-phase-02.txt` byte-for-byte into a new Update Log
entry headed `### Update — <date> (end-to-end verification)`, then run the
paste self-check and append its verdict line to the same entry:

```bash
D=docs/dev/milestones/M48-lossless-boundary/phase-02-refuse-zero-stall-threshold.md
L=$(grep -n '^### Update .*(end-to-end verification)' "$D" | tail -1 | cut -d: -f1)
tail -n +"$L" "$D" | awk '/^```/{c++; next} c==1{print} c==2{exit}' > target/e2e/pasted-m48-02.txt
diff target/e2e/pasted-m48-02.txt target/e2e/m48-phase-02.txt && echo "PASTE MATCH" || echo "PASTE MISMATCH"
```

The server-authored `(complete)` entry does not satisfy this section.

## Authorizations

None. No dependencies, no `Cargo.toml`; the config *schema* is unchanged
(same key, same type), only the accepted range narrows.

## Out of scope

- **Do not change the type** of `read_only_stall_threshold` (e.g. to
  `NonZeroUsize`) — that is a wider blast radius across every
  `GovernorConfig` construction site, including two agent tests that rely
  on `0` to isolate other detectors.
- **Do not change `resolve_for_model`'s signature** or validate inside it;
  the per-model check happens at load over the whole `models` map.
- **Other `0 disables` knobs** (`oscillation_window`, `novelty_window`,
  `wall_clock_secs`, …) keep their semantics.
- **`rexymcp doctor` output** — no new report row; the load error is the
  loud part.
- **`docs/architecture.md`, `README.md`, plugin skills** — architect-owned;
  the README's two `0 disables` comments for this key are updated at
  milestone close.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-10-08 09:18 (progress)

Started phase-02 (executor: Claude Opus 4.6). Flipped the phase doc Status
and the milestone README's phase-02 row to `in-progress`. Implementing the
spec in order: the `reject_zero_stall_threshold` validator and its call in
`Config::load` (Spec §1), the two doc comments (Spec §2), the init-template
comment (Spec §3), the two load tests (Spec §4), then the E2E block
verbatim (Spec §5).

### Update — 2026-10-08 09:47 (end-to-end verification)

All acceptance criteria met. The real artifact is the `rexymcp` binary:
`doctor --config` on the zero-threshold TOML exits 1 with the
`Error: config: [governor] read_only_stall_threshold …` text naming the
key (zero_exit=1), while the same TOML with `= 60` loads and the doctor
report runs (sixty_exit=0) — the positive control that the refusal is
specific to the zero and not a broken load path. Both new tests pass
with exit=0. Count criteria: the two `reject_zero_stall_threshold` greps
each print 1, `#[test]` count is 65, the template's `0 disables` comment
for this key is gone (0), and the two `is refused at load` doc comments
sum to 2. `target/e2e/m48-phase-02.txt`, pasted byte-for-byte:

```
Error: config: [governor] read_only_stall_threshold = 0 disables the read-only backstop; set a positive value (default 60) or remove the key
zero_exit=1

sixty_exit=0
test config::tests::load_refuses_zero_read_only_stall_threshold ... ok
test config::tests::load_refuses_zero_read_only_stall_threshold_in_model_override ... ok
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 1050 filtered out; finished in 0.00s
exit=0
```

PASTE MATCH
