# Phase 06: update dashboard spinner animation

**Milestone:** M9 — Executor runtime hardening
**Status:** review
**Depends on:** phase-05b
**Estimated diff:** ~15 lines changed in one file
**Tags:** language=rust, kind=chore, size=xs

## Goal

Replace the paw-print spinner in `mcp/src/dashboard/transcript.rs` with a
dog-chasing-brain animation. No other files change. All tests pass with updated
assertions.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read this entire phase doc before touching any code.
3. Confirm `git status` is clean.
4. Run `cargo test` and capture the test count.

## Spec

**Do not read `transcript.rs` before patching.** All required content is
pre-injected below. Apply four patches in order using `patch_file`.

### Patch 1 — Replace `SPINNER_FRAMES` constant

old_str (exact):
```
pub(crate) const SPINNER_FRAMES: &[&str] =
    &["🐾", "🐾🐾", "🐾🐾🐾", "🐾🐾🐾🐾", "🐾🐾🐾", "🐾🐾", "🐾"];
```

new_str:
```
pub(crate) const SPINNER_FRAMES: &[&str] = &[
    "🐕       🧠",
    " 🐕     🧠",
    "  🐕   🧠   ",
    "   🐕 🧠  ",
    "    🐕🧠 ",
    "  🧠🐕💨",
    " 🧠🐕",
    "🧠🐕",
    "🐕",
];
```

### Patch 2 — Update `spinner_appended_when_active` test

old_str (exact):
```
        assert_eq!(last, "🐾");
```

new_str:
```
        assert_eq!(last, "🐕       🧠");
```

### Patch 3 — Update `spinner_frame_cycles_through_all_frames` wrap test

old_str (exact):
```
        // Index 7 wraps to frame 0
        let lines = transcript_lines(&records, &ActivityFilter::default(), Some(7));
        let last = format!("{}", lines.last().unwrap());
        assert_eq!(last, SPINNER_FRAMES[0], "frame 7 should wrap to 0");
```

new_str:
```
        // Index 9 wraps to frame 0 (9 frames total, 9 % 9 == 0)
        let lines = transcript_lines(&records, &ActivityFilter::default(), Some(9));
        let last = format!("{}", lines.last().unwrap());
        assert_eq!(last, SPINNER_FRAMES[0], "frame 9 should wrap to 0");
```

### Patch 4 — Update `spinner_absent_when_none` and `spinner_appended_to_empty_records` tests

old_str (exact):
```
        assert!(!last.contains("🐾"), "spinner should not appear: {last}");
    }

    #[test]
    fn spinner_appended_to_empty_records() {
        let lines = transcript_lines(&[], &ActivityFilter::default(), Some(3));
        assert_eq!(lines.len(), 2);
        assert_eq!(format!("{}", lines[0]), "(no activity yet)");
        assert_eq!(format!("{}", lines[1]), "🐾🐾🐾🐾");
    }
```

new_str:
```
        assert!(!last.contains("🐕"), "spinner should not appear: {last}");
    }

    #[test]
    fn spinner_appended_to_empty_records() {
        let lines = transcript_lines(&[], &ActivityFilter::default(), Some(3));
        assert_eq!(lines.len(), 2);
        assert_eq!(format!("{}", lines[0]), "(no activity yet)");
        assert_eq!(format!("{}", lines[1]), "   🐕 🧠  ");
    }
```

### Task — Verify

```bash
cargo build
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all --check
cargo test
```

Run each separately. Test count must match pre-flight.

## Acceptance criteria

- [ ] `SPINNER_FRAMES` in `transcript.rs` contains exactly the 9 new frames.
- [ ] `cargo build` succeeds with zero new warnings.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes.
- [ ] `cargo fmt --all --check` passes.
- [ ] `cargo test` passes with the same count as pre-flight.
- [ ] No `🐾` remains anywhere in `transcript.rs`.

## Test plan

No new tests. Four assertions updated to match the new frame content.

## Authorizations

None. No new dependencies. No `docs/architecture.md` changes.

## Out of scope

- Do not change any other file.
- Do not alter the `transcript_lines` or `record_lines` logic.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-06-05 (complete)

**Executor:** rexyMCP executor

**Summary:** Replaced paw-print spinner with dog-chasing-brain animation in `mcp/src/dashboard/transcript.rs`. Applied 4 patches: new `SPINNER_FRAMES` constant (9 frames), updated `spinner_appended_when_active` assertion, updated wrap test (index 9 → frame 0), updated `spinner_absent_when_none` and `spinner_appended_to_empty_records` assertions.

**Files changed:**
- `mcp/src/dashboard/transcript.rs` — SPINNER_FRAMES constant + 4 test assertions

**Verification commands:**
```
cargo build → ok (0 new warnings)
cargo clippy --all-targets --all-features -- -D warnings → ok
cargo fmt --all --check → ok
cargo test → 585 passed; 0 failed; 2 ignored (matches pre-flight count)
```

**Grep check:** `grep -n '🐾' mcp/src/dashboard/transcript.rs` → 0 matches (exit 1), confirming no paw prints remain.

**End-to-end verification:** N/A — this phase changes only a display constant and its test assertions; no new runtime behavior, CLI output, or config artifacts.

**Notes for review:** None. Straightforward constant + assertion update.
