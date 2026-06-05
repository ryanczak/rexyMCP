use std::collections::HashMap;
use std::path::{Path, PathBuf};

use similar::{ChangeTag, TextDiff};

use crate::security::redact::Redactor;
use crate::store::sessions::event::{FileNumstat, SessionEvent};
use crate::store::sessions::jsonl::SessionLogHandle;

use super::log::log_event;

/// The callback the loop invokes at each emission point. `Send + Sync` so it
/// can cross await points and be shared across the rmcp request task (05b).
pub trait ProgressCallback: Send + Sync {
    fn on_progress(&self, event: &ProgressEvent);
}

/// Blanket impl over closures so callers can pass a `|e| { ... }` directly.
impl<F: Fn(&ProgressEvent) + Send + Sync> ProgressCallback for F {
    fn on_progress(&self, event: &ProgressEvent) {
        self(event);
    }
}

/// One liveness event. Mirrors the payload of `SessionEvent::Progress` so the
/// loop converts directly when logging. Kept as a separate type so the
/// callback contract is independent of the log schema's evolution.
#[derive(Debug, Clone)]
pub struct ProgressEvent {
    pub turn: usize,
    /// Short stage tag: `"turn_start"`, `"awaiting_model"`, `"tool:<name>"`,
    /// `"verify"`, `"command:<name>"`. See § 3 for the canonical set.
    pub stage: String,
    /// Per-file +/- counts from `pre_edit_content` vs. on-disk content.
    pub files_changed: Vec<FileNumstat>,
    /// One-line human-readable summary (architecture's encoded-as-message
    /// requirement). Format pinned in § 4.
    pub message: String,
}

/// Compute the per-file numstat from the loop's working-set. Reads each file
/// from disk and compares against its pre-edit content. Best-effort: a file
/// that's vanished or now unreadable contributes `(0, 0)` rather than
/// erroring — the heartbeat is never a second source of truth.
pub fn numstat_from_pre_edit(
    pre_edit_content: &HashMap<PathBuf, Option<String>>,
    project_root: &Path,
) -> Vec<FileNumstat> {
    let mut paths: Vec<&PathBuf> = pre_edit_content.keys().collect();
    paths.sort();

    let mut result = Vec::new();

    for path in paths {
        let before = pre_edit_content
            .get(path)
            .and_then(|b| b.clone())
            .unwrap_or_default();

        let after = std::fs::read_to_string(path).unwrap_or_default();

        let text_diff = TextDiff::from_lines(&before, &after);

        let mut added = 0u32;
        let mut removed = 0u32;

        for change in text_diff.iter_all_changes() {
            match change.tag() {
                ChangeTag::Insert => added += 1,
                ChangeTag::Delete => removed += 1,
                ChangeTag::Equal => {}
            }
        }

        if added == 0 && removed == 0 {
            continue;
        }

        let rel = path
            .strip_prefix(project_root)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();

        result.push(FileNumstat {
            path: rel,
            added,
            removed,
        });
    }

    result
}

/// Format a progress message string per the spec's single-line format.
/// Top-N truncation: include at most 5 per-file segments; if more, append
/// ` …+<K>`. The structured `files_changed` field carries the full list.
pub fn format_message(turn: usize, stage: &str, files_changed: &[FileNumstat]) -> String {
    let total_add: u32 = files_changed.iter().map(|f| f.added).sum();
    let total_del: u32 = files_changed.iter().map(|f| f.removed).sum();
    let file_count = files_changed.len();

    let mut parts = Vec::new();
    parts.push(format!(
        "turn={turn} stage={stage} +{total_add}/-{total_del} files={file_count}"
    ));

    let top_n = files_changed.iter().take(5);
    for f in top_n {
        parts.push(format!("{}:+{}/-{}", f.path, f.added, f.removed));
    }

    if file_count > 5 {
        parts.push(format!(" …+{}", file_count - 5));
    }

    parts.join(" ")
}

pub(super) struct EmitCtx<'a> {
    pub(super) progress: Option<&'a dyn ProgressCallback>,
    pub(super) log_handle: &'a Option<SessionLogHandle>,
    pub(super) redactor: &'a Redactor,
    pub(super) clock: &'a (dyn Fn() -> u64 + Send + Sync),
    pub(super) pre_edit_content: &'a HashMap<PathBuf, Option<String>>,
    pub(super) project_root: &'a Path,
    pub(super) turn: usize,
}

/// Emit a progress event. The two consumers are independent: the
/// `SessionEvent::Progress` record is always logged (so `rexymcp status` and
/// Claude's post-return log queries can see liveness even when no live watcher
/// is attached), while the live callback fires only when one is present. The
/// log write self-gates on the session-log handle, so this is a no-op only
/// when there is neither a handle nor a callback.
pub(super) fn emit_progress(ctx: &EmitCtx<'_>, stage: String) {
    let numstat = numstat_from_pre_edit(ctx.pre_edit_content, ctx.project_root);
    let message = format_message(ctx.turn, &stage, &numstat);

    if let Some(cb) = ctx.progress {
        cb.on_progress(&ProgressEvent {
            turn: ctx.turn,
            stage: stage.clone(),
            files_changed: numstat.clone(),
            message: message.clone(),
        });
    }

    log_event(
        ctx.log_handle,
        ctx.redactor,
        ctx.clock,
        ctx.turn,
        SessionEvent::Progress {
            turn: ctx.turn,
            stage,
            files_changed: numstat,
            message,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::sessions::event::FileNumstat;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use tempfile::TempDir;

    #[test]
    fn numstat_empty_map_returns_empty() {
        let dir = TempDir::new().unwrap();
        let map: HashMap<PathBuf, Option<String>> = HashMap::new();
        let result = numstat_from_pre_edit(&map, dir.path());
        assert!(result.is_empty());
    }

    #[test]
    fn numstat_clean_file_is_skipped() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("clean.txt");
        std::fs::write(&path, "unchanged\n").unwrap();
        let mut map = HashMap::new();
        map.insert(path, Some("unchanged\n".to_string()));

        let result = numstat_from_pre_edit(&map, dir.path());
        assert!(
            result.is_empty(),
            "clean file should be skipped (added=0, removed=0)"
        );
    }

    #[test]
    fn numstat_edited_file_counts_added_and_removed() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("edited.txt");
        std::fs::write(&path, "line1\nline2\nline3\nnewline\n").unwrap();
        let mut map = HashMap::new();
        map.insert(path, Some("line1\nline2\noldline\n".to_string()));

        let result = numstat_from_pre_edit(&map, dir.path());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].path, "edited.txt");
        assert_eq!(result[0].added, 2);
        assert_eq!(result[0].removed, 1);
    }

    #[test]
    fn numstat_deleted_file_shows_removed_lines() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("deleted.txt");
        let mut map = HashMap::new();
        map.insert(path.clone(), Some("line1\nline2\nline3\n".to_string()));

        let result = numstat_from_pre_edit(&map, dir.path());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].path, "deleted.txt");
        assert_eq!(result[0].added, 0);
        assert_eq!(result[0].removed, 3);
    }

    #[test]
    fn numstat_nested_path_is_relative() {
        let dir = TempDir::new().unwrap();
        let nested = dir.path().join("src").join("lib.rs");
        std::fs::create_dir_all(nested.parent().unwrap()).unwrap();
        std::fs::write(&nested, "fn new() {}\n").unwrap();
        let mut map = HashMap::new();
        map.insert(nested, Some("fn old() {}\n".to_string()));

        let result = numstat_from_pre_edit(&map, dir.path());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].path, "src/lib.rs");
    }

    #[test]
    fn numstat_output_sorted_by_path() {
        let dir = TempDir::new().unwrap();
        let a_path = dir.path().join("a.txt");
        let b_path = dir.path().join("b.txt");
        std::fs::write(&a_path, "edited_a\n").unwrap();
        std::fs::write(&b_path, "edited_b\n").unwrap();
        let mut map = HashMap::new();
        map.insert(b_path, Some("original_b\n".to_string()));
        map.insert(a_path, Some("original_a\n".to_string()));

        let result = numstat_from_pre_edit(&map, dir.path());
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].path, "a.txt");
        assert_eq!(result[1].path, "b.txt");
    }

    #[test]
    fn numstat_new_file_shows_only_added() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("new.txt");
        std::fs::write(&path, "line1\nline2\n").unwrap();
        let mut map = HashMap::new();
        map.insert(path, None);

        let result = numstat_from_pre_edit(&map, dir.path());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].path, "new.txt");
        assert_eq!(result[0].added, 2);
        assert_eq!(result[0].removed, 0);
    }

    #[test]
    fn format_message_empty_files() {
        let msg = format_message(1, "turn_start", &[]);
        assert_eq!(msg, "turn=1 stage=turn_start +0/-0 files=0");
    }

    #[test]
    fn format_message_few_files() {
        let files = vec![
            FileNumstat {
                path: "src/lib.rs".to_string(),
                added: 12,
                removed: 2,
            },
            FileNumstat {
                path: "src/util.rs".to_string(),
                added: 6,
                removed: 1,
            },
        ];
        let msg = format_message(4, "tool:patch", &files);
        assert!(msg.starts_with("turn=4 stage=tool:patch +18/-3 files=2"));
        assert!(msg.contains("src/lib.rs:+12/-2"));
        assert!(msg.contains("src/util.rs:+6/-1"));
    }

    #[test]
    fn format_message_truncates_after_five_files() {
        let files: Vec<FileNumstat> = (0..8)
            .map(|i| FileNumstat {
                path: format!("f{i}.rs"),
                added: 1,
                removed: 0,
            })
            .collect();
        let msg = format_message(2, "verify", &files);
        assert!(msg.starts_with("turn=2 stage=verify +8/-0 files=8"));
        assert!(msg.contains("f0.rs:+1/-0"));
        assert!(msg.contains("f4.rs:+1/-0"));
        assert!(!msg.contains("f5.rs:+1/-0"));
        assert!(msg.contains(" …+3"));
    }

    #[test]
    fn format_message_totals_sum_all_not_just_top_5() {
        let files: Vec<FileNumstat> = (0..10)
            .map(|i| FileNumstat {
                path: format!("f{i}.rs"),
                added: 10,
                removed: 5,
            })
            .collect();
        let msg = format_message(3, "command:test", &files);
        assert!(
            msg.contains("+100/-50"),
            "totals must sum all 10 files, not just top 5"
        );
        assert!(msg.contains("files=10"));
    }
}
