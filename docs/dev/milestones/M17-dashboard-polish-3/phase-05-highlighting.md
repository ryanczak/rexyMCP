# Phase 05: Markdown + extension-detected syntax highlighting

**Milestone:** M17 — Dashboard Polish (Round 3)
**Status:** done
**Depends on:** none (independent of 01–04; touches `transcript.rs` / `highlight.rs`)
**Estimated diff:** ~200 lines (two highlight paths + record pairing + tests)
**Tags:** language=rust, kind=feature, size=m

## Goal

Two Activity-panel highlighting upgrades, both built on the **existing syntect**
infrastructure (no new dependency):

1. **Markdown for Completion bodies.** Completion prose is Markdown but renders
   unstyled today. Highlight the answer text as Markdown (keep `<think>` text
   dim-italic).
2. **Extension-detected language for `read_file` results.** Today the
   tool-result highlighter guesses the language from *content* heuristics
   (`detect_syntax`), which only confidently catches diff/JSON/TOML/Rust. Use the
   file **extension** from the `read_file` call to pick the syntect grammar, so
   `.py` / `.ts` / `.sh` / `.md` results highlight correctly.

## Background: what already exists (read this first)

`mcp/src/dashboard/highlight.rs` already uses **syntect** (`mcp/Cargo.toml:21`,
`syntect = "5"` with `default-syntaxes` + `default-themes`). Its default syntax
set bundles Rust, Python, JSON, JavaScript, TypeScript, Markdown, Bash/Shell,
TOML, YAML and more. The relevant existing pieces:

- `syntax_set() -> &'static SyntaxSet` (highlight.rs:14) — the loaded grammar set.
- `theme_set()` (highlight.rs:18, private) — `base16-ocean.dark` is the theme
  used by `highlighted_body_lines`.
- `detect_syntax(content, ss) -> Option<&SyntaxReference>` (highlight.rs:27) —
  content-based detection (diff/JSON/TOML/Rust only; no Markdown/Python/TS/Bash).
- `highlighted_body_lines(content)` (highlight.rs:134) — diff special-case →
  `detect_syntax` → syntect highlight loop → plain soft-white fallback. Used by
  the `ToolResult` arm of `record_lines` (transcript.rs:103).
- `completion_body_lines(raw)` (highlight.rs:266) — splits `<think>…</think>`
  reasoning (dim-italic) from answer text (soft-white); **no** syntax
  highlighting. Used by the `Completion` arm of `record_lines` (transcript.rs:67).

**This phase extends those, it does not replace them.** No tree-sitter, no new
crate.

## Architecture references

Read before starting:

- `mcp/src/dashboard/highlight.rs` — all of it; you will add two functions and
  refactor `highlighted_body_lines` to delegate.
- `mcp/src/dashboard/transcript.rs:17–43` — `transcript_lines` (the `flat_map`
  over `visible` records); `record_lines` (transcript.rs:47), the `Parsed` and
  `ToolResult` arms (lines 69–105).
- `executor/src/tools/read_file.rs:35` — the tool name is `"read_file"`; its
  argument key is `"path"` (read_file.rs:47,66).

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom (note §2.1: no `unwrap`/`expect`
   in production; no `unwrap_or_default()` on a `Result` whose error you care
   about — use explicit fallbacks here).
2. Read the architecture references and the Background section above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Spec

### 1. Markdown-highlight Completion answer text (`highlight.rs`)

Add a private per-line Markdown highlighter, and route Completion **answer**
lines through it (think lines stay dim-italic).

```rust
/// Highlight one line of Markdown into indented styled spans via syntect's
/// markdown grammar. Falls back to a single soft-white span if the grammar is
/// missing or highlighting fails (content is always preserved).
fn markdown_line(text: &str) -> Line<'static> {
    let answer = Style::new().fg(Color::Rgb(200, 200, 200));
    let ss = syntax_set();
    let Some(syntax) = ss.find_syntax_by_extension("md") else {
        return Line::from(Span::styled(format!("    {text}"), answer));
    };
    let theme = &theme_set().themes["base16-ocean.dark"];
    let mut h = HighlightLines::new(syntax, theme);
    let line_nl = format!("{text}\n");
    let Ok(ranges) = h.highlight_line(&line_nl, ss) else {
        return Line::from(Span::styled(format!("    {text}"), answer));
    };
    let mut spans = vec![Span::raw("    ")];
    for (style, t) in ranges {
        let t = t.trim_end_matches('\n').to_string();
        if t.is_empty() {
            continue;
        }
        spans.push(Span::styled(
            t,
            Style::new().fg(Color::Rgb(
                style.foreground.r,
                style.foreground.g,
                style.foreground.b,
            )),
        ));
    }
    Line::from(spans)
}
```

Then, in `completion_body_lines`, change the per-line mapping so **answer** lines
(`!is_think`) use `markdown_line(&text)` while **think** lines keep the existing
dim-italic single span. Keep the `TRANSCRIPT_CONTENT_MAX_LINES` cap and the
overflow-marker line exactly as they are (the overflow marker stays soft-white).
Concretely, the `.map(|(text, is_think)| …)` closure becomes:

```rust
.map(|(text, is_think)| {
    if is_think {
        Line::from(Span::styled(format!("    {text}"), think))
    } else {
        markdown_line(&text)
    }
})
```

Each answer line is highlighted independently (a fresh `HighlightLines` per
line). Multi-line constructs that depend on cross-line state (a fenced code
block spanning several lines) will not carry fence context line-to-line — that is
an accepted limitation for this phase (see Out of scope).

### 2. Extension-aware tool-result highlighting (`highlight.rs`)

Refactor `highlighted_body_lines` to delegate to a path-aware variant that
prefers the file extension's grammar, falling back to the current content
detection.

```rust
/// Render `content` as indented, syntax-highlighted lines. When `path` is
/// `Some`, the file extension picks the grammar (falling back to content
/// detection if the extension is unknown); when `None`, behavior is identical to
/// the prior content-only path.
pub(crate) fn highlighted_body_lines_for(content: &str, path: Option<&str>) -> Vec<Line<'static>> {
    if is_diff_content(content) {
        return diff_body_lines(content);
    }
    let ss = syntax_set();
    let syntax = path
        .and_then(ext_of)
        .and_then(|ext| ss.find_syntax_by_extension(ext))
        .or_else(|| detect_syntax(content, ss));

    let Some(syntax) = syntax else {
        return body_lines(content)
            .into_iter()
            .map(|l| Line::from(Span::styled(l, Style::new().fg(Color::Rgb(200, 200, 200)))))
            .collect();
    };

    // ... (move the existing theme + HighlightLines + capped-line loop here,
    //      unchanged from the current `highlighted_body_lines` body) ...
}

/// Existing entry point, now a thin delegate (preserves all current callers).
pub(crate) fn highlighted_body_lines(content: &str) -> Vec<Line<'static>> {
    highlighted_body_lines_for(content, None)
}

/// File extension (without the dot) from a path, if any. `"a/b/foo.py"` → `"py"`.
fn ext_of(path: &str) -> Option<&str> {
    std::path::Path::new(path).extension().and_then(|e| e.to_str())
}
```

Move the existing highlight loop (theme lookup, `HighlightLines::new`, the
`all/capped/overflow` line loop) verbatim into `highlighted_body_lines_for`'s
`Some(syntax)` branch — do not rewrite it; only the syntax *selection* changed.

### 3. Thread the `read_file` path into the `ToolResult` render (`transcript.rs`)

Keep `record_lines(rec)`'s public signature so its existing callers/tests are
untouched; add a lang-aware variant it delegates to.

```rust
/// Existing entry point — unchanged signature, delegates with no hint.
pub(crate) fn record_lines(rec: &SessionRecord) -> Vec<Line<'static>> {
    record_lines_with_lang(rec, None)
}

/// As `record_lines`, but a `read_file` `ToolResult` body is highlighted using
/// the grammar for `path_hint`'s extension when provided.
pub(crate) fn record_lines_with_lang(
    rec: &SessionRecord,
    path_hint: Option<&str>,
) -> Vec<Line<'static>> {
    // ... identical body to today's record_lines, EXCEPT the ToolResult arm: ...
}
```

In the `ToolResult` arm, replace
`Some(highlighted_body_lines(output_preview))` with
`Some(highlighted_body_lines_for(output_preview, path_hint))`.

Then make `transcript_lines` track the most recent `read_file` path and pass it
to the following `read_file` `ToolResult`. Rewrite the `flat_map` as a `for`
loop that threads a `last_read_path: Option<String>`:

```rust
let base_ts = records.first().map(|r| r.ts).unwrap_or(0);
let mut out: Vec<Line<'static>> = Vec::new();
let mut last_read_path: Option<String> = None;
for r in &visible {
    // Capture the path from a read_file call so the matching result can use it.
    if let SessionEvent::Parsed { tool_call } = &r.event
        && tool_call.name == "read_file"
    {
        last_read_path = tool_call
            .arguments
            .get("path")
            .and_then(|v| v.as_str())
            .map(str::to_string);
    }
    let hint = match &r.event {
        SessionEvent::ToolResult { name, .. } if name == "read_file" => last_read_path.as_deref(),
        _ => None,
    };
    let mut lines = record_lines_with_lang(r, hint);
    if matches!(&r.event, SessionEvent::ToolResult { name, .. } if name == "read_file") {
        last_read_path = None; // consume it
    }
    // Prepend the relative-timestamp header span (unchanged logic).
    if let Some(header) = lines.first_mut() {
        let mut spans = Vec::with_capacity(header.spans.len() + 1);
        spans.push(Span::styled(
            format!("[{}] ", relative_ts(r.ts, base_ts)),
            Style::new().fg(Color::Rgb(180, 150, 50)),
        ));
        spans.append(&mut header.spans);
        *header = Line::from(spans);
    }
    out.extend(lines);
}
out
```

Keep the empty-`visible` early return (`"(no activity yet)"`) and the `base_ts`
computation exactly as they are.

**Best-effort note:** if the `Parsed` read_file event is filtered out by the
active `ActivityFilter` while its `ToolResult` is shown, the hint is absent and
the result falls back to content detection. That is acceptable degradation, not a
bug.

## Acceptance criteria

- [ ] No new `Cargo.toml` dependency (syntect only).
- [ ] `completion_body_lines("# Heading")` highlights the answer line as Markdown
      (more than the single plain span the old path produced); `<think>` text
      stays dim-italic.
- [ ] `highlighted_body_lines_for("x = 1\n", Some("a.py"))` selects the Python
      grammar (multi-span / non-plain), while `highlighted_body_lines_for("x = 1\n",
      None)` falls back to plain (single soft-white span) — proving the extension
      drives selection.
- [ ] `highlighted_body_lines(content)` behaves identically to today (delegates
      with `None`); the existing `highlighted_body_lines_*` tests pass unchanged.
- [ ] `record_lines(rec)` behaves identically to today (delegates with `None`);
      its existing callers/tests are untouched.
- [ ] In `transcript_lines`, a `read_file` `ToolResult` preceded by a `read_file`
      `Parsed` call with `{"path":"foo.py"}` is highlighted with the Python
      grammar.
- [ ] All four gates pass on an independent re-run.

## Test plan

In `highlight.rs`'s test module:

- `markdown_line_highlights_heading` — `markdown_line("# Heading")` returns a
  `Line` whose concatenated span text (minus the `    ` indent) equals
  `"# Heading"` and which has more than one styled span (the `#` marker styled
  apart from the text). Mutation-resistant vs a plain single-span impl.
- `completion_body_lines_highlights_answer_markdown` — a no-`<think>` completion
  `"# Title\n\nbody"` yields answer lines with markdown spans (the title line has
  >1 span). 
- `completion_body_lines_keeps_think_dim` — a completion with a `<think>reason
  </think>answer`: the reason line keeps the dim-italic style; the answer line is
  markdown-highlighted. (Adapt the existing think/answer test rather than
  duplicating.)
- **Update the existing `completion_body_no_markers_matches_plain` test** (it
  asserts a no-marker completion equals `plain_body_lines`). That equality no
  longer holds — answer text is now Markdown-highlighted. Replace it with
  `completion_body_no_markers_preserves_content`: assert the concatenated span
  text per line (stripped of the `    ` indent) equals the raw lines, i.e.
  content is preserved even though styling differs.
- `highlighted_body_lines_for_prefers_extension` — `highlighted_body_lines_for(
  "x = 1\n", Some("a.py"))` has a content line with >1 span (Python highlighted);
  `highlighted_body_lines_for("x = 1\n", None)` has a single soft-white span
  (plain). Load-bearing, mutation-resistant: an impl that ignores `path` fails
  the first assertion.
- `highlighted_body_lines_for_unknown_ext_falls_back` —
  `highlighted_body_lines_for(content, Some("a.xyz"))` falls back to content
  detection (== the `None` result for the same content).
- Keep `highlighted_body_lines_preserves_content`,
  `highlighted_body_lines_falls_back_for_plain_text`,
  `highlighted_body_lines_routes_diff_to_diff_renderer` — they call the delegate
  and must still pass.

In `transcript.rs`'s test module:

- `record_lines_delegates_to_with_lang_none` — for a sample record,
  `record_lines(rec)` equals `record_lines_with_lang(rec, None)` (same rendered
  text).
- `transcript_lines_highlights_read_file_by_extension` — records: a `Parsed`
  read_file with `arguments = {"path":"foo.py"}`, then a `ToolResult { name:
  "read_file", succeeded: true, output_preview: "def f():\n    pass" }`. Assert
  the tool-result body has a Python-highlighted content line (>1 span on a code
  line). Mutation-resistant: without the path-threading, `"def f():"` is not
  caught by `detect_syntax` (no Python content rule), so it would render plain —
  the test distinguishes the two.
- `transcript_lines_read_file_without_call_falls_back` — a lone `read_file`
  `ToolResult` with no preceding `Parsed` call renders via content detection
  (plain for non-detectable content), no panic.

All tests hermetic — construct `SessionRecord`s in-memory (see the existing
`rec(...)` / `start_event()` helpers in the transcript test module). No network,
no real files.

## End-to-end verification

Run `cargo run -p rexymcp -- dashboard --repo . --config rexymcp.toml` against a
session whose transcript contains a `read_file` of a `.py`/`.ts`/`.sh` file and a
Completion with Markdown. Confirm the read_file body is syntax-colored per its
language and the Completion answer shows Markdown styling (headers/emphasis/code).
Quote one highlighted line in the completion log. If no such live session is
available, the unit tests pin the behavior — restate that, consistent with prior
dashboard-panel phases.

## Authorizations

None required beyond the dashboard's normal `mcp`-crate scope. **No new
dependency** — syntect is already a dependency. No `docs/architecture.md` change.
No session-event change.

(The milestone originally floated tree-sitter for this phase; that was dropped in
favor of extending the existing syntect path — decided with the user 2026-06-11.
Do **not** add tree-sitter or any grammar crate.)

## Out of scope

- Cross-line Markdown/code-fence state (each Completion answer line is
  highlighted independently; a fenced block won't carry fence context).
- Highlighting `bash`-tool *command output* (only `read_file` results get the
  extension hint; bash output keeps content detection).
- Highlighting the `Prompt` or `Parsed`-args bodies (left as-is).
- Adding languages syntect's default set lacks, or swapping the theme.
- Any change to `detect_syntax`'s content heuristics (the extension path sits in
  *front* of it; the heuristics stay the fallback).

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-06-11 20:16 (started)

**Executor:** Claude (Sonnet 4.5)
**Tasks:** Implement Markdown highlighting for Completion answer text, extension-aware highlighting for `read_file` results, and thread `read_file` path through `transcript_lines`.

### Update — 2026-06-11 20:20 (complete)

**Summary:** Implemented both highlighting upgrades — Markdown for Completion answers and extension-detected language for `read_file` results — plus threaded the `read_file` path through `transcript_lines` to pair calls with their results.

**Verification commands:**

```
$ cargo fmt --all --check
(check: clean)

$ cargo build 2>&1 | tail -20
Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.01s

$ cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail -10
Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.69s

$ cargo test 2>&1 | tail -10
test result: ok. 734 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 6.05s
```

**Verification summary:** `cargo fmt --all --check` clean, `cargo build` zero warnings, `cargo clippy --all-targets --all-features -- -D warnings` clean, `cargo test` 734 passed / 0 failed.

**Files changed:**
- `mcp/src/dashboard/highlight.rs` — added `markdown_line()`, `highlighted_body_lines_for()`, `ext_of()`; refactored `completion_body_lines` to route answer lines through `markdown_line`; refactored `highlighted_body_lines` to delegate to `highlighted_body_lines_for(content, None)`; updated tests for new behavior.
- `mcp/src/dashboard/transcript.rs` — added `record_lines_with_lang()`; refactored `record_lines` to delegate; rewrote `transcript_lines` as a `for` loop threading `last_read_path` from `read_file` `Parsed` calls to matching `ToolResult`s; updated existing test for markdown-highlighted answer lines; added new tests for path threading and fallback.

**End-to-end verification:** Unit tests pin the behavior for both Markdown highlighting and extension-driven grammar selection. A live dashboard session with a `read_file` of a `.py` file and a Markdown Completion is not available in the test environment; consistent with prior dashboard-panel phases, the hermetic tests serve as the verification artifact.

**Grep for spec-pinned literals:**
```
$ grep -rn 'find_syntax_by_extension("md")' mcp/src/dashboard/highlight.rs
285:    let Some(syntax) = ss.find_syntax_by_extension("md") else {
```
Literal confirmed present in `markdown_line`.

**Notes for review:**
- The `highlighted_body_lines` and `record_lines` functions are retained as thin delegates (to preserve existing callers/tests) and are now only used in the `None`/fallback path. They show as "never used" by the binary target but are exercised by tests — this is intentional per the spec's "preserves all current callers" requirement.
- `markdown_line` creates a fresh `HighlightLines` per line, so fenced code blocks spanning multiple lines lose fence context — this is an accepted limitation per Out of Scope.

### Review verdict — 2026-06-11

- **Verdict:** approved_first_try
- **Bounces:** none
- **Executor:** Qwen/Qwen3.6-27B-FP8 (the Update Log self-stamps "Claude (Sonnet 4.5)" — the recurring local-LLM identity quirk; machine records are correct)
- **Scope deviations:** none. Two cosmetic, semantically-identical departures from the spec's literal form, both accepted: (a) the `ToolResult` arm uses `match path_hint { Some(p) => …_for(.., Some(p)), None => highlighted_body_lines(..) }` rather than always `…_for(.., path_hint)` (the `None` path delegates identically); (b) `transcript_lines` calls `record_lines(r)` on the no-hint branch rather than `record_lines_with_lang(r, None)` (delegates identically). Both keep the documented entry points exercised.
- **Calibration:** none. Clean 82-turn first-try. All four gates green on independent re-run (fmt/build/clippy clean; **734 executor + 370 mcp** pass, 2 ignored). The pre-existing `unwrap_or_default()` at `highlight.rs:166` was moved verbatim per the spec's "move the existing highlight loop unchanged" instruction (confirmed present in parent `86c01f1`) — not a new STANDARDS §2.1 violation. Load-bearing tests confirmed mutation-resistant: `highlighted_body_lines_for_prefers_extension` distinguishes the extension path from the content fallback (`detect_syntax` does not catch `"x = 1"`, so an impl ignoring `path` renders plain and fails the `Some("a.py")` → >1-span assertion); `transcript_lines_highlights_read_file_by_extension` pins the path-threading end to end. E2E is a TUI render (no headless harness) — N/A per all prior M13/M15/M17 dashboard-panel precedent; the unit tests render the real `Line`/`Span` output. **Not the milestone close:** M17 phase-06 (further dashboard UI) is planned with the user — do not auto-close M17 at this boundary; `/rexymcp:architect next` drafts phase-06.
