# Phase 02: format extractors (hermes, fenced, loose_json, yaml, xml, text)

**Milestone:** M3 — Forgiving parser
**Status:** done
**Depends on:** phase-01 (done)
**Estimated diff:** ~1050 lines (six extractors + extract/mod.rs + tests)
**Tags:** language=rust, kind=feature, size=l

## Goal

Lift the six format **extractors** from Rexy into `executor/src/parser/extract/`.
Each turns its slice of a model response into `Candidate`s (the phase-01 type):
`extract(response: &str) -> Vec<Candidate>`. They are the stage that follows
`detect` — `parse()` (M3 phase-05) will run the formats `detect` returned through
the matching extractor.

## Architecture references

- `docs/architecture.md` — lift/drop map (parser row, "Lift").
- Rexy source: `rexy/src/agent/parser/extract/` — `mod.rs` + `hermes.rs`,
  `fenced.rs`, `loose_json.rs`, `yaml.rs`, `xml.rs`, `text.rs`.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` and the M3 README Notes (re-rooting; strip Rexy
   plan references).
2. Read this entire phase doc.
3. Confirm phase-01 is `done`; `executor::parser::{Candidate, Format}` exist.
4. **Read** each `rexy/src/agent/parser/extract/*.rs`. Lift faithfully — these are
   vendored; do not reshape beyond the adaptations below.

## Spec

Create `executor/src/parser/extract/mod.rs` declaring the six submodules, and one
file per extractor. Each exposes `pub fn extract(response: &str) -> Vec<Candidate>`
and builds `Candidate { format, name, arguments, score: 0, repairs_attempted:
vec![], raw_body }`. Wire `pub mod extract;` into `executor/src/parser/mod.rs`.

The `use super::super::{Candidate, Format};` paths resolve correctly in rexyMCP
(`extract/<x>.rs` → `parser::extract` → `parser`) — lift as-is.

Per-extractor (lift verbatim except where noted):

- **`hermes.rs`** — `<tool_call>…</tool_call>`; string-aware `find_balanced_end`
  brace matcher (the close tag is a hint, not a delimiter); one `Candidate` per
  opening tag, `name`/`arguments` from the JSON object, malformed body →
  `name: None, arguments: None`.
- **`fenced.rs`** — ` ```json ` blocks, same brace-matching approach.
- **`loose_json.rs`** — balanced-brace objects in prose; **only** emits a
  candidate when the parsed object has a string `"name"` field (filters incidental
  `{}`); does not rescan nested `arguments`.
- **`xml.rs`** — `<function=NAME>{json}</function>`; the **name comes from the
  tag, not the body**; arguments are the JSON object.
- **`text.rs`** — `name(arg=value, …)` via regex; values parsed as JSON, falling
  back to raw strings; uses `regex` (already a dep).
- **`yaml.rs`** — ` ```yaml ` fences or a `name:`/`arguments:` block at equal
  indent; parsed with `serde_yaml` then converted to `serde_json::Value`.

**Adaptations / fixes (do not lift these defects):**

1. **`yaml.rs` redundant call.** Rexy's `extract` computes `let region =
   detect_region(...)` and then immediately re-binds `let Some(region) =
   detect_region(...) else …`, calling it twice and leaving an unused binding.
   Keep only the `let Some(region) = detect_region(response, pos) else { break };`
   form — the first `let region = …` line is a bug that trips `-D warnings`.
2. **`yaml.rs` `use serde_yaml;`.** Drop the bare `use serde_yaml;` import;
   reference `serde_yaml::from_str` / `serde_yaml::Value` fully-qualified (the bare
   import is unused-style and warns).
3. **Strip Rexy plan/stage references** in doc comments ("Stage 3 …", "stage 5
   (repair)"). Rewrite to describe the extractor without Rexy's milestone numbers.
4. **`find_balanced_end` stays per-file.** It is duplicated across hermes/fenced/
   loose_json/xml in Rexy; keep it that way (vendored fidelity — the architecture
   says don't reshape lifted parser code). Do not extract a shared helper in this
   phase.

## Dependencies

- **`serde_yaml = "0.9"`** (new — matches Rexy), for `yaml.rs`. Authorized below.
  Note: `serde_yaml` 0.9 is unmaintained upstream; acceptable for the faithful
  lift now, a maintained fork can be swapped in later if needed — not this phase.

## Acceptance criteria

- [ ] `executor/src/parser/extract/{mod,hermes,fenced,loose_json,yaml,xml,text}.rs`
      exist; `pub mod extract;` is declared in `parser/mod.rs`; each extractor is
      `pub fn extract(&str) -> Vec<Candidate>`.
- [ ] Each extractor returns the documented `Candidate`(s) for its format with the
      right `Format`, `name`, `arguments`, and populated `raw_body`.
- [ ] String-literal robustness: hermes/xml handle a body whose string value
      contains the literal close tag (`</tool_call>` / `</function>`); fenced
      handles a backtick inside a string value.
- [ ] xml takes `name` from the tag even when the body has a conflicting
      `"name"`.
- [ ] **Negative cases:** `loose_json` does **not** emit for an object lacking a
      `"name"` field; every extractor returns an **empty** `Vec` when its marker is
      absent; a malformed JSON body yields a malformed candidate (hermes/fenced:
      `name: None`) rather than a panic.
- [ ] No Rexy plan/stage references survive (grep the new files for `Stage [0-9]`,
      `phase 0` → zero hits).
- [ ] All four required commands pass with zero new warnings.

## Test plan

Hermetic, pure-function unit tests per extractor (lift Rexy's, adapt). Pin
behavior incl. the negatives above (empty on absent marker; loose_json name
filter; malformed-body handling).

## End-to-end verification

> Not applicable — pure library functions exercised by unit tests. `parse()`
> (phase-05) and the loop (M4) consume them. Restate in the completion entry.

## Authorizations

- [x] **May add dependency:** `serde_yaml = "0.9"`.
- [x] **May create** `executor/src/parser/extract/` (mod + six files); **may
      modify** `executor/src/parser/mod.rs` (declare `pub mod extract;`) and the
      two `Cargo.toml` files.
- [ ] May **NOT** lift `score`, `repair`, `validate`, `feedback`, or `parse()`.
- [ ] May **NOT** modify `docs/architecture.md`, `STANDARDS.md`, `WORKFLOW.md`, or
      another phase doc.

## Out of scope

- **score / validate / repair / feedback / `parse()`** — later M3 phases.
- **Deduplicating `find_balanced_end`** — kept per-file (vendored).
- **Replacing `serde_yaml`** with a maintained fork — later, if at all.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-05-28 (in-progress, executor handoff)

**Executor:** Claude Code (Opus 4.7) — executing directly. The extractors are
dense with close-tag / fence / backtick / escaped-quote content (the exact shapes
that corrupt opencode's tool-call serializer, per AGENTS.md), so opencode can't
write these files reliably.

### Update — 2026-05-28 (complete)

**Executor:** Claude Code (Opus 4.7).

**Summary:** Lifted all six extractors into `executor/src/parser/extract/`
(`mod.rs` + `hermes`, `fenced`, `loose_json`, `xml`, `text`, `yaml`) and declared
`pub mod extract;` in `parser/mod.rs`. `super::super::{Candidate, Format}` paths
resolve as-is. `find_balanced_end` kept per-file (vendored). Added `serde_yaml =
"0.9"` for the YAML extractor.

**Lift fixes (defects not ported):**
- `yaml.rs`: dropped the redundant double `detect_region` call (unused binding →
  `-D warnings`).
- `yaml.rs`: dropped the bare `use serde_yaml;` (referenced fully-qualified).
- `yaml.rs`: **removed a production `.unwrap()`** — Rexy did
  `line.find("name:").unwrap()` after already matching; rebound `name_col` from the
  `if let` instead (STANDARDS §2.1 forbids the unwrap). Not in the spec's fix list
  but mandatory.
- Rexy stage/plan doc references stripped.

**Acceptance criteria:** all met, incl. negatives (loose_json name filter; empty
on absent marker; malformed body → malformed candidate, no panic) and the
string-literal-robustness cases (close tag / backtick inside a value).

**Commands:**

```
cargo fmt --all --check        # clean (after rustfmt on the new files)
cargo build                    # clean, 0 warnings
cargo clippy --all-targets --all-features -- -D warnings   # clean
cargo test                     # 228 passed; 0 failed
```

**End-to-end verification:** N/A — pure library functions exercised by unit tests;
`parse()` (phase-05) and the loop (M4) consume them.

**Files changed:**
- `executor/src/parser/extract/{mod,hermes,fenced,loose_json,xml,text,yaml}.rs` — new
- `executor/src/parser/mod.rs` — `pub mod extract;`
- `Cargo.toml`, `executor/Cargo.toml` — `serde_yaml = "0.9"`

**Grep proof:** `grep -rniE 'stage [0-9]|phase 0' executor/src/parser/extract/`
→ 0 hits.

**Notes for review:** executed by Claude Code, not opencode (the format content
trips opencode's serializer). Not self-approved — flipped to `review`. The
`serde_yaml` 0.9 dependency is unmaintained upstream (acceptable for the faithful
lift; swappable later).

verification: fmt OK · clippy OK · tests 228 passed · build OK
