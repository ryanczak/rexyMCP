# Phase 09: `reqwest` 0.12→0.13

**Milestone:** M25 — Polish & Config Pass
**Status:** done
**Depends on:** none (phase-08 landed the `toml` bump; this is independent)
**Estimated diff:** ~1 line (one workspace-root `Cargo.toml` constraint + the `Cargo.lock` churn)
**Tags:** language=rust, kind=refactor, size=s

## Goal

Bump `reqwest` from the `0.12` line (currently pinned `0.12.28`) to `0.13`
(latest `0.13.4`). `reqwest` is used in exactly **two** source files,
`executor/src/ai/mod.rs` and `executor/src/ai/backends/openai.rs`, through a
small, stable client/request/response API. The breaking changes published in
`0.13.0` do **not** touch any of the items those files use (see the analysis
below) — the one consequential change is the **default TLS backend** switching
from native-tls (openssl) to **rustls + aws-lc-rs**, which is a dependency-tree
and runtime change, **not** a source-API change. The expected diff is the single
version constraint plus the `Cargo.lock` churn, with **no source edit**.

This is the **last** of the five M25 dependency bumps (after `similar` 2→3,
`tree-sitter` 0.25→0.26 / `tree-sitter-python` 0.23→0.25, `toml_edit` 0.22→0.25,
and `toml` 0.8→1, all `done`). Approving it closes the M25 dependency thread.

## Decision already made (do not re-litigate)

The architect and the user decided **2026-06-30** to **accept reqwest 0.13's new
rustls default** rather than pin the old native-tls backend. So the bump is the
**bare** `reqwest = { version = "0.13", features = ["json", "stream"] }` form —
keep the existing `["json", "stream"]` features, do **not** add `default-tls`,
`native-tls`, `rustls`, or any TLS feature to the array, and do **not** set
`default-features = false`. The TLS backend switching to rustls + aws-lc-rs is
the **intended, accepted** outcome of this phase, not a regression to guard
against. (Adding a TLS feature to "preserve" native-tls would be a scope
deviation reversing a decision the user already made.)

## Architecture references

Read before starting:

- `docs/architecture.md` § Status #25 — names M25 as the polish & config pass; the
  dep bumps are committed scope.
- This milestone's [README](README.md) § "Dependency update phases (05–09)" — the
  per-phase ordering and the uniform recipe (bump constraint → update lock → fix
  compiler-flagged breaks → verify four gates). Phase-08 (`toml` 0.8→1, `done`) is
  the immediately preceding worked example of this same recipe; it is the closest
  analogue because `reqwest`, like `toml`, is a **workspace** dependency.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.
5. **Verify the build toolchain for aws-lc-rs is present** before building. The
   new rustls default pulls in `aws-lc-rs` / `aws-lc-sys`, whose build scripts
   shell out to a **C compiler and `cmake`**. The architect verified at draft time
   (2026-06-30) that this host has them — `cmake` 4.3.4, plus `cc`/`clang`/`gcc` on
   PATH — so the build is expected to succeed. Confirm with `cmake --version` and
   `cc --version`. **If `cargo build` later fails *inside* `aws-lc-sys`/`aws-lc-rs`
   with a missing-`cmake`/missing-compiler/codegen error, that is a host-toolchain
   problem, not a code defect — file a blocker quoting the exact build error and
   stop.** Do not attempt to fix it by editing the dependency, pinning a TLS
   feature, or reverting to native-tls (the user chose rustls; reversing that is
   the architect's call, not the executor's).

## Current state

`reqwest` is a **workspace** dependency: it is declared once at the workspace root
and consumed by the `executor` crate via `reqwest.workspace = true`. There is
exactly one constraint to change and it **is** in the workspace-root `Cargo.toml`
(this is the same shape as phase-08's `toml`, not phase-07's `mcp`-direct
`toml_edit`).

Workspace-root `Cargo.toml`, line 20:

```toml
reqwest = { version = "0.12", features = ["json", "stream"] }
```

Consumed by `executor/Cargo.toml:17`:

```toml
reqwest.workspace = true
```

(`mcp` does **not** depend on `reqwest` — leave `mcp/Cargo.toml` alone.)

### The complete reqwest API surface (quoted so you can confirm none changed)

`reqwest` is referenced in exactly two source files
(`grep -rn 'reqwest' executor/src mcp/src` — there are no `reqwest::` references
anywhere else in the workspace). The full set of items they touch:

`executor/src/ai/mod.rs`:

```rust
// :19   static HTTP_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
// :99-106
pub fn http() -> &'static reqwest::Client {
    HTTP_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .unwrap()
    })
}
// :113-114  make_req: impl Fn() -> reqwest::RequestBuilder, -> Result<reqwest::Response>
// :118      req.send().await
// :120      resp.status()  (then .is_success() / .is_server_error())
// :124      reqwest::StatusCode::TOO_MANY_REQUESTS
// :126,134  resp.bytes().await
// :172      impl futures_util::Stream<Item = Result<B, reqwest::Error>>   (the "stream" feature)
```

`executor/src/ai/backends/openai.rs`:

```rust
// :197-203  request construction (the live request the loop sends)
let response = send_with_retry(|| {
    http()
        .post(format!("{}/chat/completions", self.base_url))
        .bearer_auth(&self.api_key)
        .json(&body)               // the "json" feature
})
.await?;
// :205      let mut stream = response.bytes_stream();   // the "stream" feature
// :413      e.downcast_ref::<reqwest::Error>().is_some()
// :986 (test)  reqwest::Client::new().get("not-a-url").build().unwrap_err()
```

The features the code relies on are exactly the two already declared:
**`json`** (`.json(&body)`) and **`stream`** (`.bytes_stream()` and the
`reqwest::Error`-keyed `futures_util::Stream`). Both feature names are
**unchanged** in 0.13.

## What changed in `reqwest` 0.13.0 (and why only one item matters)

Verified against the upstream `reqwest` `CHANGELOG.md` at draft time (latest
published is `0.13.4`, 2026-05-25). The breaking changes in `0.13.0`:

1. **rustls is now the default TLS backend, instead of native-tls.**
2. rustls crypto provider defaults to **aws-lc** instead of *ring*.
3. The `rustls-tls` feature was renamed to `rustls`.
4. rustls roots features removed; `rustls-platform-verifier` is used by default.
5. `native-tls` now includes ALPN (disable via `native-tls-no-alpn`).
6. **`query` and `form` are now crate features, disabled by default.**
7. Long-deprecated methods and crate features removed (e.g. `trust-dns`).
8. Many TLS-related methods renamed for autocompletion (old names kept as soft
   deprecations).

**Only items 1–2 and 4 affect us, and only as an accepted dependency-tree/runtime
change — none change a source API this code calls.** Specifically:

- **The TLS swap (1, 2, 4) is the intended outcome of this phase** — see "Decision
  already made" above. It changes which crates link (native-tls/openssl out,
  rustls/aws-lc-rs in), not any line of `mod.rs`/`openai.rs`. `Client::builder()`,
  `.timeout()`, `Client::new()`, `.post()`, `.get()`, `.bearer_auth()`, `.json()`,
  `.send()`, `.build()`, `RequestBuilder`, `Response`, `.bytes_stream()`,
  `.bytes()`, `.status()`, `StatusCode::TOO_MANY_REQUESTS`, and `reqwest::Error`
  are all **unchanged** across 0.13.
- **The `query`/`form` gating (6) does not affect us** — the code uses **neither**.
  Grep-confirm with `grep -rn '\.query(\|\.form(' executor/src mcp/src` (expect
  **zero** matches). Because we don't use them, we do **not** add the `query`/`form`
  features; the existing `["json", "stream"]` array is complete and correct. (This
  is a pinned negative: do **not** add `query`/`form` "just in case".)
- **The feature rename (3) does not affect us** — the manifest names neither
  `rustls-tls` nor `rustls`; it keeps the unchanged `json`/`stream` and relies on
  the (now rustls) default TLS, so there is no feature string to rename.
- **The TLS-method renames (8) do not affect us** — `mod.rs`/`openai.rs` call no
  TLS-configuration methods (no `.use_rustls_tls()`, `.tls_built_in_root_certs()`,
  etc.); the only `ClientBuilder` method used is `.timeout()`, which is untouched.
- **Removed deprecated methods (7) do not affect us** — none of the touched items
  is on the removed list.

The expected outcome is therefore a **clean bump with no source edits**, exactly
like phases 05–08.

### The lock churn — the native-tls subtree drops, the rustls/aws-lc subtree appears

Verified at draft time. The native-tls/openssl crates currently in the lock are
pulled in **only by reqwest 0.12** — nothing else in the workspace uses them:

```
$ cargo tree -i native-tls          # only reverse-dep path is reqwest 0.12
native-tls v0.2.18
├── hyper-tls v0.6.0 → reqwest v0.12.28 → rexymcp-executor → rexymcp
├── reqwest v0.12.28 (*)
└── tokio-native-tls v0.3.1 → hyper-tls / reqwest (*)

$ cargo tree -i openssl-sys         # only via the native-tls subtree above
openssl-sys v0.9.117
├── native-tls v0.2.18 → … (only reqwest)
└── openssl v0.10.81 → native-tls v0.2.18 (*)
```

So after the bump the lock **drops** the reqwest-only native-tls subtree
(`native-tls`, `openssl`, `openssl-sys`, `hyper-tls`, `tokio-native-tls`) and
**adds** the rustls + aws-lc subtree (`rustls` becomes actively linked — it is
currently a stale, reverse-dep-less lock entry — plus `aws-lc-rs`, `aws-lc-sys`,
`rustls-platform-verifier`, and their supporting crates). **This is the expected,
correct consolidation — the whole point of accepting the rustls default. Do not
treat the disappearance of the openssl/native-tls crates as a problem to fix, and
do not treat the appearance of aws-lc-rs/rustls crates as an unauthorized
dependency add — both are automatic resolver consequences of the single
authorized `reqwest` bump** (analogous to phase-08, where bumping `toml` dropped
the orphaned `toml_edit 0.22` subtree).

## Spec

1. **Bump the one version constraint** — in the **workspace-root** `Cargo.toml`,
   change line 20 from
   `reqwest = { version = "0.12", features = ["json", "stream"] }` to
   `reqwest = { version = "0.13", features = ["json", "stream"] }`. Change **only**
   the version string; keep the `features = ["json", "stream"]` array exactly as
   is. Do **not** add `default-features = false`, do **not** add any TLS feature
   (`default-tls`/`native-tls`/`rustls`), do **not** add `query`/`form`. This is a
   workspace dependency; `executor` consumes it via `reqwest.workspace = true` and
   needs **no** edit. Do **not** touch `executor/Cargo.toml` or `mcp/Cargo.toml`.

2. **Update the lockfile** — run the **package-scoped** update:
   `cargo update -p reqwest`. Confirm `Cargo.lock` now contains a `reqwest` entry
   at a `0.13.x` version (expected `0.13.4`) and that the old `reqwest 0.12.28`
   entry — along with its now-orphaned `native-tls`, `openssl`, `openssl-sys`,
   `hyper-tls`, and `tokio-native-tls` companions — is **gone**, replaced by the
   rustls/aws-lc subtree (see "The lock churn" above; this swap is correct). If
   `cargo update -p reqwest` reports an ambiguous-package error or otherwise
   declines, run `cargo build` instead — a plain build resolves the new `^0.13`
   constraint and writes the lock. A bare `cargo update` (no `-p` filter) would
   churn unrelated crates and is a scope violation — use the package-scoped form
   (or let `cargo build` do the resolve). Commit the `Cargo.lock` change together
   with the workspace `Cargo.toml` change.

3. **Build and react only to what the compiler flags.** Run `cargo build`. The
   expectation (per the analysis above) is that it succeeds with no source change.
   The first build will compile `aws-lc-sys` (C/cmake build) and may take noticeably
   longer than usual — that is normal, not a hang. **If the compiler flags an error
   in `executor/src/ai/mod.rs` or `executor/src/ai/backends/openai.rs`** (the only
   two reqwest sites), fix that specific call to the `reqwest 0.13` API and record
   the change in "Notes for review". Do **not** preemptively edit either file; the
   bump itself is the change. **If the build fails *inside* `aws-lc-sys`/`aws-lc-rs`
   (a C-toolchain/cmake error, not a Rust type error in our code), file a blocker**
   with the exact error (see Pre-flight 5) — do not work around it. If any other
   break appears that you cannot resolve from this phase doc's analysis, file a
   blocker with the exact `cargo build` error rather than guessing.

## Acceptance criteria

- [ ] Workspace-root `Cargo.toml` line 20 reads
      `reqwest = { version = "0.13", features = ["json", "stream"] }` — version
      string bumped, features array unchanged, **no** `default-features`/TLS/`query`/
      `form` keys added.
- [ ] `executor/Cargo.toml` and `mcp/Cargo.toml` are unchanged.
- [ ] `Cargo.lock` contains a `reqwest` entry at a `0.13.x` version (run
      `cargo tree -i reqwest`, or `grep -A2 'name = "reqwest"' Cargo.lock`, to
      confirm) and **no** `reqwest 0.12.x` entry remains.
- [ ] `Cargo.lock` no longer contains `native-tls` / `openssl-sys` / `hyper-tls`
      entries (`cargo tree -i native-tls` should error with "package ID
      specification … did not match any packages"), and **does** now contain
      `aws-lc-rs` and a reverse-dep-bearing `rustls` (`cargo tree -i rustls`
      resolves to `reqwest`).
- [ ] `cargo build` succeeds with zero new warnings.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes.
- [ ] `cargo fmt --all --check` passes.
- [ ] `cargo test` passes (existing suite — no new tests).
- [ ] Neither `executor/src/ai/mod.rs` nor `executor/src/ai/backends/openai.rs`
      was edited unless the compiler required it; any such edit is recorded in
      "Notes for review".

## Test plan

No new tests. `reqwest` is a vendored HTTP-client utility crate (STANDARDS §3.2 —
vendored wire/utility code is covered upstream); this phase adds no behavior. Two
existing guards exercise the bumped crate:

- `is_retriable_transport_true_for_reqwest_error` (`openai.rs:982`) constructs a
  **real** `reqwest::Error` via
  `reqwest::Client::new().get("not-a-url").build().unwrap_err()` and asserts it
  downcasts — this both compiles against the 0.13 `Client`/`get`/`build` API and
  exercises `reqwest::Error` under the new version.
- The `ai` loop/backend unit tests (`openai.rs` test module) run against
  `MockAiClient`/the harness seams (`drain_stream` etc.), so they pass without a
  live endpoint, but they compile the full `openai.rs` module against `reqwest
  0.13` — a green build of that module is itself the API-compatibility evidence.

## End-to-end verification

The shipped artifact is the rebuilt `rexymcp` binary linking `reqwest 0.13` (with
the rustls/aws-lc TLS stack) in its AI-backend HTTP path — used by every
`execute_phase` call the MCP server services. Verify against it directly:

1. After the bump, run `cargo test -p rexymcp-executor 2>&1 | tail -30` and confirm
   the executor suite (which compiles and runs the `ai`/`openai` modules against
   the new crate, including the real-`reqwest::Error` test above) passes — quote
   the pass/fail line in the completion Update Log.
2. Run `cargo tree -i reqwest` and quote the resolved `0.13.x` version in the
   completion Update Log, confirming the new version is actually linked into
   `rexymcp-executor`/`rexymcp` (not a stale lock entry).
3. Run `cargo tree -i native-tls` and quote its output (it should report no match)
   and `cargo tree -i rustls` (it should now resolve through `reqwest`), confirming
   the TLS backend actually swapped.

This is **not** an N/A case: the bump changes a linked dependency — and its TLS
backend — in the HTTP path every dispatch uses, and the green executor suite plus
the `cargo tree` evidence are the real-artifact proof that `reqwest`'s client
API still binds and the rustls stack links.

## Authorizations

- [x] May edit the workspace-root `Cargo.toml`: bump `reqwest` version `"0.12"` →
      `"0.13"` (features array unchanged).
- [x] May update `Cargo.lock` via `cargo update -p reqwest` (or a `cargo build`
      resolve). The resulting addition of the rustls/aws-lc subtree
      (`rustls`, `aws-lc-rs`, `aws-lc-sys`, `rustls-platform-verifier`, …) and
      removal of the native-tls/openssl subtree are authorized **as automatic
      resolver consequences of the `reqwest` bump** — not as independent dependency
      changes.

This is **only** authorization to bump `reqwest`. No other dependency may be
added, removed, or bumped by hand. A stray `cargo update` with no `-p` filter
would churn unrelated crates — use the package-scoped form.

## Out of scope

- Bumping or hand-editing any other dependency. This is the last M25 dep bump;
  there is no phase-10. A bare `cargo update` (no `-p` filter) is a scope violation.
- **Pinning a TLS feature to preserve native-tls.** The user chose the rustls
  default (2026-06-30); adding `default-tls`/`native-tls`/`default-features = false`
  to "be safe" reverses that decision and is a scope deviation. Keep the bare
  `version = "0.13", features = ["json", "stream"]` form.
- Adding the `query` or `form` features (the code uses neither) or any other
  feature to the array.
- Adopting any new `reqwest` 0.13 API (the renamed TLS-config methods, HTTP/3, the
  new resolver hooks, etc.). The goal is the version bump with the smallest possible
  diff, not feature adoption — leave the `mod.rs`/`openai.rs` call sites exactly as
  they are unless the compiler forces a change.
- Refactoring or "improving" the HTTP client construction, retry, or streaming
  logic in `ai/mod.rs` / `ai/backends/openai.rs`.
- "Fixing" the dependency-tree churn (the dropped openssl subtree / the added
  aws-lc subtree) by editing the lock by hand or adding features. It is automatic
  and correct.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-06-30 14:57 (started)

**Executor:** rexyMCP executor

Bumped `reqwest` version constraint from `"0.12"` to `"0.13"` in workspace-root `Cargo.toml`. Ran `cargo update -p reqwest` — lockfile updated, `reqwest 0.13.4` resolved, native-tls/openssl subtree removed, rustls/aws-lc subtree added. Proceeding to build.

### Update — 2026-06-30 14:58 (complete)

**Executor:** rexyMCP executor

**Verification summary:** All four gates passed clean. `cargo build` succeeded with zero warnings, `cargo clippy --all-targets --all-features -- -D warnings` passed, `cargo fmt --all --check` passed, `cargo test` passed (871 passed, 0 failed, 2 ignored).

**End-to-end verification:**
- `cargo test -p rexymcp-executor`: `test result: ok. 871 passed; 0 failed; 2 ignored`
- `cargo tree -i reqwest`: resolved to `reqwest v0.13.4` linked through `rexymcp-executor → rexymcp`
- `cargo tree -i native-tls`: `error: package ID specification 'native-tls' did not match any packages` (confirmed removed)
- `cargo tree -i rustls`: resolves through `reqwest v0.13.4 → rexymcp-executor` (confirmed present)
- `grep -rn '\.query(\|\.form(' executor/src mcp/src`: zero matches (confirmed not used)

**Files changed:** `Cargo.toml` (line 20: version `"0.12"` → `"0.13"`), `Cargo.lock` (resolver churn: native-tls/openssl subtree removed, rustls/aws-lc subtree added)

**Source edits:** None — neither `executor/src/ai/mod.rs` nor `executor/src/ai/backends/openai.rs` was touched. The `reqwest` 0.13 API is fully compatible with our call sites.

**Grep for pinned literal:** `grep 'version = "0.13"' Cargo.toml` → matches line 20.

**Commit:** `refactor: bump reqwest 0.12→0.13 (rustls default TLS)`

### Review verdict — 2026-06-30

- **Verdict:** approved_first_try
- **Bounces:** none
- **Executor:** Qwen/Qwen3.6-27B-PrismaAURA
- **Scope deviations:** none
- **Calibration:** none

Independent re-run: all four gates green (fmt clean, build zero warnings, clippy
clean, 871 passed / 2 ignored). `cargo tree -i reqwest` → `0.13.4` linked through
`rexymcp-executor → rexymcp`; `cargo tree -i native-tls` / `-i openssl-sys` both
report no match (native-tls/openssl subtree dropped); `cargo tree -i rustls`
resolves through `reqwest v0.13.4` and `aws-lc-rs` is present — the rustls/aws-lc
TLS swap landed as the accepted, intended outcome. `Cargo.toml:20` carries the
bare `version = "0.13", features = ["json", "stream"]` form with no
`default-features`/TLS/`query`/`form` keys; `executor/Cargo.toml` and
`mcp/Cargo.toml` untouched; no source edits to `ai/mod.rs` or
`ai/backends/openai.rs`. The real-`reqwest::Error` guard
`is_retriable_transport_true_for_reqwest_error` compiles and passes against the
0.13 client API. Last M25 dep bump — closes the M25 dependency thread.
