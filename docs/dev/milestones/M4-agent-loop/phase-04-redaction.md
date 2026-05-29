# Phase 04: Secret redaction primitive

**Milestone:** M4 — Headless agent loop + governor/verifier
**Status:** todo
**Depends on:** phase-03 (done) — supplies the `SessionRecord`/`SessionEvent` the
redactor protects, though this phase does not wire into them (that is phase-07).
**Estimated diff:** ~400 lines (net-new redactor + tests)
**Tags:** language=rust, kind=feature, size=m

## Goal

The **content redaction primitive**: a `Redactor` that masks secrets in a string
before that string reaches disk. The executor reads files from the target repo,
so any turn can capture a key, token, or credential; every record is redacted
**upstream** of the session-log writer (phase-03's writer is deliberately
redaction-agnostic). This phase ships only the redactor; the loop that calls it
on each record is phase-07.

This is **net-new for rexyMCP**, like `scope` and `bash_classify`. Rexy's
`security/redact.rs` is a `TODO: implement` stub — there is no working code to
lift. Rexy's *running* masker lives in `ai/filter.rs::mask_sensitive`, but it is
built for a long-lived daemon (global `OnceLock` pattern set, process-lifetime
atomic counters, `tracing` on compile failure, `<TYPE>` markers). rexyMCP wants
none of that machinery: an **instance-held** redactor, no globals, no counters,
no `tracing`, and the `[REDACTED:<type>]` marker the stub's design specifies.
Use `ai/filter.rs` as a **pattern reference only** (its regexes are good); do not
lift its structure.

## Architecture references

Read before starting:

- `docs/architecture.md` — "Session log & troubleshooting tools" § Redaction
  (lines ~206–208): "Every record is passed through the redaction layer before
  it is written." And Status §M4 (line ~459): redaction sits between the turn
  event and the JSONL writer.
- Rexy source (**reference only**): `rexy/src/ai/filter.rs` — `builtin_defs()`
  has the battle-tested regexes (AWS `AKIA…`, PEM blocks, JWT, GitHub PATs, DB
  URLs, tagged `key=value`). `rexy/src/security/redact.rs` — the stub's header
  comment defines the intended design (three layers + `[REDACTED:<type>]`).

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above and the M4 README Note "Redaction is
   its own phase (04)".
3. Read this entire phase doc before touching any code.
4. Read `executor/src/security/bash_classify.rs` — it is the **convention to
   match**: net-new security module, `static NAME: LazyLock<Regex>` for each
   compiled pattern, a `Severity`-style enum, tests at the file bottom.
5. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

`executor/src/security/` contains `scope.rs`, `bash_classify.rs`, and `mod.rs`
(which re-exports `Scope`/`ScopeError` and `Severity`/`classify`). There is **no**
`redact.rs`. `regex` is already an `executor` dependency (workspace dep, used by
`bash_classify`) — no new dependency is needed. rexyMCP has no `tracing`.

`bash_classify.rs` establishes the in-repo idiom:

```rust
use regex::Regex;
use std::sync::LazyLock;

static RM_RF_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"…").unwrap());
```

(The `.unwrap()` is acceptable here under STANDARDS §2.1 — a regex literal is a
compile-time constant, so the invariant is proven; matching the existing file's
style is fine, or use `.expect("<pattern> is a valid regex")`.)

## Spec

Create `executor/src/security/redact.rs` and export it from
`executor/src/security/mod.rs`. Two public types plus the redact entry point.

### `SecretKind` — the label that names the marker

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretKind {
    OpenAiKey,      // sk-… / sk-ant-…
    GitHubToken,    // ghp_/gho_/ghu_/ghs_/ghr_… and github_pat_…
    AwsAccessKey,   // AKIA…
    SlackToken,     // xox[baprs]-…
    GoogleToken,    // ya29.…
    StripeKey,      // sk_live_… / rk_live_…
    Jwt,            // eyJ….….…
    PrivateKey,     // -----BEGIN … PRIVATE KEY----- … -----END … PRIVATE KEY-----
    DbUrl,          // postgres://user:pass@host, mysql://…, redis://…, mongodb…
    TaggedValue,    // password=… / api_key: … / bearer … / ?token=…
    HighEntropy,    // long high-entropy run (opt-in layer only)
}
```

`SecretKind` exposes its marker tag — the **snake_case** name used in
`[REDACTED:<tag>]`. Pin the tag spelling (it is load-bearing — M5 log-query tools
filter on it): `openai_key`, `github_token`, `aws_access_key`, `slack_token`,
`google_token`, `stripe_key`, `jwt`, `private_key`, `db_url`, `tagged_value`,
`high_entropy`. So a masked OpenAI key renders as `[REDACTED:openai_key]`. (Your
call whether this is a `fn tag(&self) -> &'static str` method, a `Display` impl,
or a `const` table — pin only the output strings, not the mechanism.)

### `Redactor` — instance-held, no globals

```rust
pub struct Redactor { /* compiled patterns + the entropy flag */ }

impl Redactor {
    pub fn new() -> Self;              // built-in patterns; high-entropy OFF
    pub fn with_high_entropy(self) -> Self;  // opt-in: enable the entropy layer
    pub fn redact(&self, text: &str) -> String;
}
```

- **No global state.** Compile the patterns into `static LazyLock<Regex>`
  items (one per pattern, `bash_classify` style) or into the `Redactor` at
  `new()`. Do **not** replicate Rexy's process-global `OnceLock` pattern set or
  its atomic redaction counters — counters are daemon telemetry and out of scope
  (`PhaseRun` is phase-08).
- **No `tracing`.** A regex literal that fails to compile is a programmer bug,
  not a runtime condition; `.unwrap()`/`.expect()` on the literal is correct
  (STANDARDS §2.1) — do not add `tracing` to "warn and skip" like Rexy does.
- `redact` returns an owned `String`. Prefer `Cow`-style borrow-if-unchanged
  internally if you like, but the public return is `String`.

### Redaction layers — applied in this order

`redact` applies patterns in **specificity order** so a value matched by a
precise prefix pattern is labelled by its specific `SecretKind`, not the generic
`tagged_value`:

1. **Layer 1 — known prefixes / shapes** (high confidence, always on):
   `OpenAiKey`, `GitHubToken`, `AwsAccessKey`, `SlackToken`, `GoogleToken`,
   `StripeKey`, `Jwt`, `PrivateKey`. Borrow Rexy's `builtin_defs()` regexes for
   the ones it has (AWS, PEM, JWT, GitHub); write the rest (`sk-`/`sk-ant-`,
   `xox*`, `ya29.`, `sk_live_`/`rk_live_`) in the same style.
2. **Layer 2 — tagged values & connection strings** (always on): `DbUrl`
   (credential-bearing `scheme://user:pass@host`), then `TaggedValue` —
   `(?i)(password|passwd|secret|token|api[_-]?key|apikey)\s*[=:]\s*\S+`, URL
   query-param secrets (`[?&](password|token|api_key)=…`), and `bearer \S+`.
3. **Layer 3 — high-entropy heuristic** (**opt-in, OFF by default**): only when
   `with_high_entropy()` was called. A long base64/hex run above an entropy
   threshold → `[REDACTED:high_entropy]`. Off by default because it
   false-positives on UUIDs, git SHAs, and build IDs.

**Security invariant (the point of the phase):** after `redact`, the output must
not contain the original secret as a substring. Pin this directly (below) — it is
more important than which marker a given secret gets.

## Acceptance criteria

- [ ] `executor/src/security/redact.rs` exists; `security/mod.rs` re-exports
      `Redactor` and `SecretKind`.
- [ ] `Redactor::new()` masks every Layer-1 and Layer-2 kind with the
      `[REDACTED:<tag>]` marker, using the exact snake_case tags pinned above.
- [ ] The high-entropy layer is **off** under `new()` and **on** under
      `with_high_entropy()`.
- [ ] No global mutable state (no `OnceLock` pattern registry, no atomic
      counters); no `tracing`; no new dependency added to `Cargo.toml`.
- [ ] Security invariant: for every positive test secret, `redact`'s output
      contains neither the raw secret nor any ≥8-char substring of it.
- [ ] All four required commands pass with zero new warnings.

## Test plan

Pure-function tests at the bottom of `redact.rs`; no filesystem, no clock, no
network. Pin both positives and the boundary negatives (WORKFLOW § "Pin negative
cases").

**Positive — each kind is masked with its tag** (one test per kind, or a
table-driven test):
- `redacts_openai_key` — `sk-` and `sk-ant-` strings → `[REDACTED:openai_key]`.
- `redacts_github_token` — classic `ghp_…` and `github_pat_…` → `[REDACTED:github_token]`.
- `redacts_aws_access_key`, `redacts_jwt`, `redacts_private_key_pem_block`,
  `redacts_slack_token`, `redacts_google_token`, `redacts_stripe_key`.
- `redacts_db_url_with_embedded_credentials` — `postgres://u:p@host/db` → `[REDACTED:db_url]`.
- `redacts_tagged_value` — `api_key = hunter2`, `password: s3cret`,
  `Authorization: bearer abc.def` each → `[REDACTED:tagged_value]`.

**Security invariant:**
- `redacted_output_never_contains_the_raw_secret` — for a representative secret of
  each kind, assert the output contains neither the full secret nor an 8-char
  slice of it.

**Negative — must NOT be redacted under `new()`** (these are the boundary; they
look secret-ish but are not):
- `leaves_plain_prose_untouched` — `"the password rotation policy is documented"`
  (the word "password" with no `=`/`:` assignment) is returned unchanged.
- `does_not_redact_uuid_or_git_sha_without_entropy_layer` — a UUID and a 40-char
  hex SHA pass through `new()` unchanged (no Layer-3).
- `does_not_redact_normal_identifier` — e.g. `sktech` / `ya29things` prose words
  that merely *start* like a prefix but lack the full token shape stay intact
  (tune the regexes so the prefix patterns require their real token grammar).

**High-entropy layer:**
- `high_entropy_layer_masks_only_when_enabled` — a long random base64 token is
  unchanged under `new()` but becomes `[REDACTED:high_entropy]` under
  `with_high_entropy()`; assert the same UUID from the negative test is *still*
  left alone even with entropy on (UUIDs are below the threshold / dashed).

## End-to-end verification

> Not applicable — phase ships no runtime-loadable artifact. The redactor is a
> library primitive exercised by unit tests; the loop that calls it on each
> record before the JSONL write is phase-07.

## Authorizations

- [x] **May create** `executor/src/security/redact.rs`; **may modify**
      `executor/src/security/mod.rs` (add `pub mod redact;` + the
      `Redactor`/`SecretKind` re-exports).
- [ ] **No new dependencies** — `regex` is already present; do not add `tracing`
      or anything else.
- [ ] May **NOT** modify `docs/architecture.md`, `STANDARDS.md`, `WORKFLOW.md`,
      or any other phase doc.

## Out of scope

- **Wiring redaction into the loop / records** — phase-07 composes redact → log
  and decides which `SessionEvent` string fields to pass through. This phase
  ships the primitive only; it does not import or touch `store::sessions`.
- **Path-based read-refusal** (".env / *.pem / ~/.ssh/** are never read at all").
  Despite the README's phase-04 sketch grouping it here, refusing to *read* a
  secret-bearing path is a **read-tool / `scope` concern**, not the log-content
  redactor's job, and conflates "never read" with "redact before write." Leave it
  out; if it is wanted, it belongs in a separate read-tool phase. Note it in
  "Notes for review," do not implement it.
- **Redaction counters / metrics** — daemon telemetry; `PhaseRun` is phase-08.
- **User-configurable extra patterns** (`init_masking(extra)`) — no config
  surface yet; built-in patterns only.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
