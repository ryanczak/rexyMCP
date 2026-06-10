# Bug 1 on phase-06c: new `.lock().unwrap()` in production path violates STANDARDS §2.1

**Severity:** major
**Status:** verified
**Filed:** 2026-06-09
**Fixed:** 2026-06-10 (commit 2648cbb)

## What's wrong

`executor/src/tools/update_task.rs:84` introduces a new `unwrap()` in a
production (non-test) path:

```rust
let mut tasks = self.tasks.lock().unwrap();
```

This is in `UpdateTask::execute`, the live tool-dispatch path. STANDARDS §2.1
("Never `.unwrap()` / `.expect()` in production paths") and the Definition of
Done §1 ("No new `unwrap()` / `expect()` / `panic!()` in production paths") both
forbid this. The phase doc's Authorizations section is "None," so nothing waives
the rule. The DoD unwrap/expect/panic checkbox therefore cannot be ticked, and
the phase is not done.

The codebase already establishes the poison-tolerant idiom for production
`Mutex` locks, and this new code is the sole production-path deviation:

- `executor/src/ai/mod.rs:41,50,70` — `self.open_until.lock().unwrap_or_else(|e| e.into_inner())`
- `executor/src/agent/mod.rs:168` — `h.lock().ok().map(...)`
- `executor/src/store/sessions/jsonl.rs:21` — `if let Ok(mut logger) = handle.lock()`

(The only other `.lock().unwrap()` in `ai/mod.rs`, at line 230, is inside a
`#[cfg(test)]` block — test code is exempt.)

## What should happen

The lock acquisition must not introduce a new production-path `unwrap()`. Per
STANDARDS §2.1 and the in-repo precedent in `ai/mod.rs`, recover the inner guard
on poison rather than panicking. A flip is the only mutation under the lock and
leaves the `Vec<Task>` consistent, so `into_inner()` on a poisoned guard is
correct — there is no torn state to fear.

## How to fix

In `executor/src/tools/update_task.rs:84`, change:

```rust
let mut tasks = self.tasks.lock().unwrap();
```

to the poison-tolerant form already used in `ai/mod.rs`:

```rust
let mut tasks = self.tasks.lock().unwrap_or_else(|e| e.into_inner());
```

No other change is needed; the existing tests already cover the flip path.

## Verification

- [ ] `grep -nE '\.(unwrap|expect)\(|panic!\(' executor/src/tools/update_task.rs`
      reports matches only inside the `#[cfg(test)] mod tests` block (none above it).
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes.
- [ ] `cargo test -p rexymcp-executor update_task` passes (all six unit tests green).
- [ ] `cargo test` passes (existing + new tests).
