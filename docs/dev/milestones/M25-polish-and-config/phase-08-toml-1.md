# Phase 08: `toml` 0.8→1.x

**Milestone:** M25 — Polish & Config Pass
**Status:** done
**Depends on:** none (phase-07 landed the `toml_edit` bump; this is independent)
**Estimated diff:** ~2 lines (one workspace-root `Cargo.toml` constraint + the `Cargo.lock` churn)
**Tags:** language=rust, kind=refactor, size=s

## Goal

Bump `toml` from the `0.8` line (currently pinned `0.8.23`) to `1` (latest
`1.1.2`). `toml` is used in exactly **one** source location,
`executor/src/config.rs:419`, through a single stable API call:
`toml::from_str::<Config>(&content)` (deserialize a TOML string into a `serde`
struct, error mapped via `.to_string()`). The breaking changes published across
the 0.9 and 1.0 releases do **not** touch that call (see the analysis below), so
the expected diff is the single version constraint plus the `Cargo.lock` update,
with **no source edit**.

This is the fourth of the five M25 dependency major/minor bumps (after `similar`
2→3, `tree-sitter` 0.25→0.26 / `tree-sitter-python` 0.23→0.25, and `toml_edit`
0.22→0.25, all `done`).

## Architecture references

Read before starting:

- `docs/architecture.md` § Status #25 — names M25 as the polish & config pass; the
  dep bumps are committed scope.
- This milestone's [README](README.md) § "Dependency update phases (05–09)" — the
  per-phase ordering and the uniform recipe (bump constraint → update lock → fix
  compiler-flagged breaks → verify four gates). Phase-07 (`toml_edit` 0.22→0.25,
  `done`) is the immediately preceding worked example of this same recipe.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

Unlike `toml_edit` (phase-07, a direct `mcp`-crate dependency), `toml` **is** a
**workspace dependency**: it is declared once at the workspace root and consumed
by the `executor` crate via `toml.workspace = true`. There is exactly one
constraint to change and it **is** in the workspace-root `Cargo.toml`.

Workspace-root `Cargo.toml`, line 15:

```toml
toml = "0.8"
```

Consumed by `executor/Cargo.toml:12`:

```toml
toml.workspace = true
```

(`mcp` does **not** depend on `toml` directly — only on `toml_edit`, which
phase-07 bumped to 0.25. Leave `mcp/Cargo.toml` alone.)

`Cargo.lock` currently pins (verified at draft time):

| Crate | Current pin | After this phase |
|---|---|---|
| `toml` (direct, via `executor`) | `0.8.23` | `1.x` (expected `1.1.2`) |
| `toml_edit` (transitive, via `toml 0.8`) | `0.22.27` | **removed** (see below) |
| `toml_datetime` (via the `toml 0.8` subtree) | `0.6.11` | **removed** (`1.1.1` stays, via `toml_edit 0.25`) |
| `serde_spanned` (via the `toml 0.8` subtree) | `0.6.9` | **removed** (`1.1.x` added by `toml 1`) |
| `toml_edit` (direct, via `mcp`) | `0.25.12+spec-1.1.0` | `0.25.12+spec-1.1.0` (**unchanged** — phase-07's, left in place) |

### The lock churn — this is the *inverse* of phase-07, read it before running anything

Phase-07 deliberately **left** a `toml_edit 0.22.27` entry in the lock because
`toml 0.8` still pulled it in transitively. **This phase removes that need.**
`toml 1.x` no longer depends on `toml_edit` at all (it switched to
`toml_parser`/`toml_writer`/`serde_spanned`/`toml_datetime` 1.x). Verified at
draft time, the `toml 0.8.23` subtree is closed — these four crates are reachable
**only** through it and nothing else uses them:

```
$ cargo tree -i toml_edit@0.22.27      # only reverse-dep is toml 0.8.23
toml_edit v0.22.27
└── toml v0.8.23
    └── rexymcp-executor → rexymcp

$ cargo tree -i toml_datetime@0.6.11   # only via the toml 0.8 subtree
toml_datetime v0.6.11
├── toml v0.8.23 → …
└── toml_edit v0.22.27 → toml v0.8.23 (*)

$ cargo tree -i serde_spanned@0.6.9    # only via the toml 0.8 subtree
serde_spanned v0.6.9
├── toml v0.8.23 → …
└── toml_edit v0.22.27 → toml v0.8.23 (*)
```

So after the bump the lock **drops** `toml 0.8.23`, `toml_edit 0.22.27`,
`toml_datetime 0.6.11`, and `serde_spanned 0.6.9`, and **adds** `toml 1.1.2`
plus its 1.x helpers (`serde_spanned 1.1.x`, `toml_parser`, `toml_writer`;
`toml_datetime 1.1.1` and `winnow 0.7`/`toml_parser` may already be present via
`toml_edit 0.25`). **`toml_edit 0.25.12+spec-1.1.0` — the entry phase-07 added,
used directly by `mcp` — stays untouched.** Do **not** treat the disappearance of
the four `0.x` helper crates as a problem to fix or a sign something broke: it is
the expected, correct consolidation (the whole point of the `toml` 1.x line is to
stop vendoring `toml_edit` for deserialization).

### The single `toml` usage site (quoted so you can confirm nothing changed)

`toml` is used in exactly one source file, `executor/src/config.rs`:

```rust
// executor/src/config.rs:416-420
if path.exists() {
    let content = std::fs::read_to_string(path)?;
    let loaded: Config =
        toml::from_str(&content).map_err(|e| Error::Config(e.to_string()))?;
    config = loaded;
}
```

This is the **only** `toml::` reference in the entire workspace
(`grep -rn 'toml::' executor/src mcp/src` returns this one line). It is a
turbofish-free `toml::from_str` deserializing into the `serde`-derived `Config`
struct, with the error mapped through `e.to_string()`.

## What changed in `toml` 0.9 / 1.0 (and why none of it affects us)

Verified against the upstream `toml` crate changelog at draft time. The breaking
changes published across 0.9.0 → 1.1.2 are:

- **0.9.0:** `from_str` / `Deserializer` no longer **preserve table order** by
  default (now gated behind the `preserve_order` feature, matching `Table`);
  `Deserializer::new` / `ValueDeserializer::new` now **return errors** (deprecated
  in favor of `Deserializer::parse`); `Serializer::new` / `pretty` now take a
  `&mut Buffer` rather than `&mut String`; `impl FromStr for Value` now parses a
  TOML **value**, not a document; `Spanned::span` returns `Range<usize>`; with
  `default-features = false` you must opt back into `serde,std`
  (`cargo add toml -F serde,std`).
- **1.0.0 / 1.1.x:** stabilization on top of 0.9; no further break to the basic
  `from_str`-into-`serde`-struct path. Current latest is `1.1.2+spec-1.1.0`.

**None of these touch the one call `config.rs` makes.** Specifically:

- **Order preservation is irrelevant** — `config.rs` deserializes *into a struct*
  (`Config`), not into an order-sensitive map; field order in the source TOML has
  never mattered to struct deserialization. We do **not** enable, want, or need
  `preserve_order`.
- **`Deserializer::new` / `Serializer` / `Buffer` are not used** — `config.rs`
  calls the top-level `toml::from_str` free function, not the
  `Deserializer`/`Serializer` builder APIs that changed signature.
- **`impl FromStr for Value` is not used** — there is no `Value::from_str`,
  `.parse::<toml::Value>()`, or `toml::Value` reference anywhere in the workspace;
  `from_str::<Config>` deserializes directly into the struct.
- **The error type may change, but the usage is type-agnostic** — `config.rs` maps
  the error with `.map_err(|e| Error::Config(e.to_string()))`. `to_string()` is
  `Display`, which every `toml` error version implements, so a changed error type
  cannot break this line.
- **Default features are intact** — the workspace declares `toml = "0.8"` with
  **no** `default-features = false`, so `serde` + `std` + `parse` (the features
  `from_str` needs) are on by default; bumping to `toml = "1"` with default
  features keeps them on. The `cargo add toml -F serde,std` note applies **only**
  to consumers who had disabled default features — which this workspace does not.
  Do **not** add a features array; keep the bare `toml = "1"` form.

The expected outcome is therefore a **clean bump with no source edits**.

## Spec

1. **Bump the one version constraint** — in the **workspace-root** `Cargo.toml`,
   change line 15 `toml = "0.8"` to `toml = "1"`. This is a workspace dependency;
   `executor` consumes it via `toml.workspace = true` and needs **no** edit. Do
   **not** add a `features = [...]` array or `default-features = false` — keep the
   bare `toml = "1"` form (default features carry the `serde`/`parse`/`std`
   support `from_str` requires). Do **not** touch `mcp/Cargo.toml` (it declares
   `toml_edit`, already at 0.25 from phase-07, not `toml`).

2. **Update the lockfile** — run the **package-scoped** update:
   `cargo update -p toml`. Confirm `Cargo.lock` now contains a `toml` entry at a
   `1.x` version (expected `1.1.2`) and that the old `toml 0.8.23` entry — along
   with its now-orphaned `toml_edit 0.22.27`, `toml_datetime 0.6.11`, and
   `serde_spanned 0.6.9` companions — is **gone** (see "The lock churn" above; this
   removal is correct). If `cargo update -p toml` reports an ambiguous-package
   error or otherwise declines, run `cargo build` instead — a plain build resolves
   the new `^1` constraint automatically and writes the lock. A bare `cargo update`
   (no `-p` filter) would churn unrelated crates and is a scope violation — use the
   package-scoped form (or let `cargo build` do the resolve). Commit the
   `Cargo.lock` change together with the workspace `Cargo.toml` change.

3. **Build and react only to what the compiler flags.** Run `cargo build`. The
   expectation (per the analysis above) is that it succeeds with no source change.
   **If — and only if — the compiler flags an error in `executor/src/config.rs`**
   (the one `toml::from_str` site), fix that specific call to the `toml 1` API and
   record the change in "Notes for review". Do **not** preemptively edit
   `config.rs`; the bump itself is the change. If a break appears that you cannot
   resolve from this phase doc's analysis above, **file a blocker** with the exact
   `cargo build` error rather than guessing.

## Acceptance criteria

- [ ] Workspace-root `Cargo.toml` line 15 reads `toml = "1"` (bare form, no
      `features`/`default-features` keys added).
- [ ] `mcp/Cargo.toml` is unchanged (it declares `toml_edit = "0.25"`, not `toml`).
- [ ] `Cargo.lock` contains a `toml` entry at a `1.x` version (run
      `cargo tree -i toml`, or `grep -A2 'name = "toml"' Cargo.lock`, to confirm)
      and **no** `toml 0.8.x` entry remains.
- [ ] `Cargo.lock` no longer contains a `toml_edit 0.22.27` entry — only
      `toml_edit 0.25.12+spec-1.1.0` (the `mcp` direct dep, from phase-07) remains
      (`cargo tree -i toml_edit@0.22.27` should error with "package ID specification
      … did not match any packages"; `cargo tree -i toml_edit@0.25.12+spec-1.1.0`
      should still resolve to `rexymcp`).
- [ ] `cargo build` succeeds with zero new warnings.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes.
- [ ] `cargo fmt --all --check` passes.
- [ ] `cargo test` passes (existing suite — no new tests).
- [ ] `executor/src/config.rs` was **not** edited unless the compiler required it;
      any such edit is recorded in "Notes for review".

## Test plan

No new tests. `toml` is a vendored TOML-parsing utility crate (STANDARDS §3.2 —
vendored wire/utility code is covered upstream); this phase adds no behavior. The
existing `config` test module at the bottom of `executor/src/config.rs` is the
regression guard — its tests round-trip real TOML through `Config::load` →
`toml::from_str` (e.g. the load/default/override/per-model-resolve tests added
across M18/M20/M23), exercising the bumped deserializer against the new version.

## End-to-end verification

The shipped artifact is the rebuilt `rexymcp` binary linking `toml 1.x` in its
config-loading path (`Config::load`, used by every CLI command and the MCP
server). Verify against it directly:

1. After the bump, run `cargo test -p rexymcp-executor config 2>&1 | tail -30` and
   confirm the `config` tests pass — quote the pass/fail line in the completion
   Update Log. (These tests drive the real `toml::from_str` deserialize path.)
2. Run `cargo tree -i toml` (or `grep -A2 'name = "toml"' Cargo.lock`) and quote
   the resolved `1.x` version in the completion Update Log, confirming the new
   version is actually linked into `rexymcp-executor`/`rexymcp` (not a stale lock
   entry). Also run `cargo tree -i toml_edit@0.22.27` and quote its output (it
   should report no match), confirming the orphaned transitive entry is gone.

This is **not** an N/A case: the bump changes a linked dependency in the config
loader every binary path uses, and the green `config` deserialize suite against
the new version is the real-artifact evidence that `toml::from_str` still binds
and behaves.

## Authorizations

- [x] May edit the workspace-root `Cargo.toml`: bump `toml` `"0.8"` → `"1"`.
- [x] May update `Cargo.lock` via `cargo update -p toml` (or a `cargo build`
      resolve).

This is **only** authorization to bump `toml`. No other dependency may be added,
removed, or bumped. In particular **`toml_edit` stays at `0.25`** (phase-07's
bump; the `mcp` direct dep is untouched) and the four orphaned `0.x` helper crates
(`toml_edit 0.22.27`, `toml_datetime 0.6.11`, `serde_spanned 0.6.9`, `toml 0.8.23`)
are removed by Cargo's resolver as a *consequence* of the `toml` bump, not by any
manual lock edit. A stray `cargo update` with no `-p` filter would churn unrelated
crates — use the package-scoped form.

## Out of scope

- Bumping any other dependency. Phase-09 bumps `reqwest` in its own review-gated
  phase. A bare `cargo update` (no `-p` filter) is a scope violation here.
- Re-pinning or "cleaning up" `toml_edit` in `mcp/Cargo.toml`. It is correctly at
  0.25 from phase-07; the transitive `0.22.27` entry disappearing is automatic, not
  a manifest edit.
- Adding a `features = [...]` or `default-features = false` key to the `toml`
  dependency. The bare `toml = "1"` form is correct; the workspace relies on
  default features for `serde`/`parse`. (Adding features to "be safe" is a scope
  deviation that changes the resolved feature set.)
- Adopting any new `toml` 1.x API (the `Deserializer::parse` constructor, `Buffer`
  serializer, `preserve_order`, span APIs, TOML 1.1 features). The goal is the
  version bump with the smallest possible diff, not a feature adoption — leave the
  one `toml::from_str` call exactly as it is unless the compiler forces a change.
- Refactoring or "improving" `Config::load` or the surrounding config code.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-06-30 14:34 (started)

**Executor:** rexyMCP executor

Bumping `toml` from `0.8` to `1` in workspace-root `Cargo.toml`.

### Update — 2026-06-30 14:34 (complete)

**Executor:** rexyMCP executor

**Changes:**
- `Cargo.toml`: `toml = "0.8"` → `toml = "1"` (line 15, workspace-root)
- `Cargo.lock`: resolved via `cargo update -p toml`; removed `toml 0.8.23`, `toml_edit 0.22.27`, `toml_datetime 0.6.11`, `toml_write 0.1.2`, `winnow 0.7.15`; added `toml 1.1.2+spec-1.1.0` and its 1.x helpers
- `mcp/Cargo.toml`: unchanged (as required)
- `executor/src/config.rs`: not edited (no compiler errors)

**Verification commands:**
- `cargo build` — succeeded, zero warnings
- `cargo clippy --all-targets --all-features -- -D warnings` — passed
- `cargo fmt --all --check` — passed
- `cargo test` — 871 passed, 0 failed, 2 ignored
- `cargo test -p rexymcp-executor config` — 58 passed, 0 failed (end-to-end config deserialize tests)

**End-to-end verification:**
- `cargo tree -i toml` → `toml v1.1.2+spec-1.1.0` linked into `rexymcp-executor`
- `cargo tree -i toml_edit@0.22.27` → "package ID specification … did not match any packages" (orphaned entry removed as expected)
- `cargo tree -i toml_edit@0.25.12+spec-1.1.0` → resolves to `rexymcp` (phase-07 entry intact)

**Grep for pinned literal:**
- `grep 'toml = "1"' Cargo.toml` → confirmed `toml = "1"` present at line 15

**Notes for review:** None. Clean bump, no source edits required. All acceptance criteria met.

**Commit:** `chore: bump toml 0.8 → 1`

### Review verdict — 2026-06-30

- **Verdict:** approved_first_try
- **Bounces:** none
- **Executor:** Qwen/Qwen3.6-27B-PrismaAURA
- **Scope deviations:** none
- **Calibration:** none — clean dependency bump, no source edits. All four gates
  green on independent re-run (871 passed / 2 ignored); `cargo tree -i toml` →
  `toml v1.1.2+spec-1.1.0` linked into `rexymcp-executor`; `toml 0.8.23` and the
  orphaned `toml_edit 0.22.27` / `toml_datetime 0.6.11` / `serde_spanned 0.6.9`
  subtree dropped from the lock; `toml_edit 0.25.12+spec-1.1.0` (phase-07, `mcp`
  direct) intact; `config.rs` unedited; 58 config deserialize tests green against
  the new version.
