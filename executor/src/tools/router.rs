// 2-stage tool routing primitive: categorize built-in tools so the agent loop
// can show a weak model a small, relevant slice of schemas.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Category {
    Read,
    Write,
    Search,
    Run,
    Meta,
}

/// Map a built-in tool name to its router category. `None` for an unknown name.
pub fn categorize(tool_name: &str) -> Option<Category> {
    Some(match tool_name {
        "read_file" | "symbols" => Category::Read,
        "write_file" | "patch" | "patch_lines" | "delete_file" | "move_file" => Category::Write,
        "search" | "find_files" => Category::Search,
        "bash" => Category::Run,
        "update_task" => Category::Meta,
        _ => return None,
    })
}

/// Does this built-in tool mutate files on disk? The single source of truth for
/// "the model made file progress" — the no-progress stall governor and the
/// escalation briefing both ask this instead of each keeping a private list that
/// can (and did) drift from the `Category::Write` set above.
pub fn mutates_files(tool_name: &str) -> bool {
    categorize(tool_name) == Some(Category::Write)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categorize_read_file() {
        assert_eq!(categorize("read_file"), Some(Category::Read));
    }

    #[test]
    fn categorize_symbols() {
        assert_eq!(categorize("symbols"), Some(Category::Read));
    }

    #[test]
    fn categorize_write_file() {
        assert_eq!(categorize("write_file"), Some(Category::Write));
    }

    #[test]
    fn categorize_patch() {
        assert_eq!(categorize("patch"), Some(Category::Write));
    }

    #[test]
    fn categorize_search() {
        assert_eq!(categorize("search"), Some(Category::Search));
    }

    #[test]
    fn categorize_find_files() {
        assert_eq!(categorize("find_files"), Some(Category::Search));
    }

    #[test]
    fn categorize_bash() {
        assert_eq!(categorize("bash"), Some(Category::Run));
    }

    #[test]
    fn categorize_update_task() {
        assert_eq!(categorize("update_task"), Some(Category::Meta));
    }

    #[test]
    fn categorize_unknown_name_returns_none() {
        assert_eq!(categorize("frobnicate"), None);
    }

    #[test]
    fn mutates_files_covers_every_write_tool() {
        for tool in [
            "write_file",
            "patch",
            "patch_lines",
            "delete_file",
            "move_file",
        ] {
            assert!(
                mutates_files(tool),
                "{tool} should count as a file mutation"
            );
        }
    }

    #[test]
    fn mutates_files_false_for_non_write_tools() {
        for tool in [
            "read_file",
            "symbols",
            "search",
            "find_files",
            "bash",
            "update_task",
        ] {
            assert!(
                !mutates_files(tool),
                "{tool} should not count as a file mutation"
            );
        }
        assert!(!mutates_files("frobnicate"));
    }

    #[test]
    fn all_built_in_tools_categorize_to_some() {
        let built_ins = [
            "read_file",
            "symbols",
            "search",
            "find_files",
            "write_file",
            "patch",
            "patch_lines",
            "delete_file",
            "move_file",
            "bash",
            "update_task",
        ];
        for name in built_ins {
            assert!(
                categorize(name).is_some(),
                "built-in tool {name} did not categorize to Some(_)"
            );
        }
    }
}
