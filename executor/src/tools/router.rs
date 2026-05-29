// 2-stage tool routing primitive: categorize built-in tools so the agent loop
// can show a weak model a small, relevant slice of schemas.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Category {
    Read,
    Write,
    Search,
    Run,
}

/// Map a built-in tool name to its router category. `None` for an unknown name.
pub fn categorize(tool_name: &str) -> Option<Category> {
    Some(match tool_name {
        "read_file" | "symbols" => Category::Read,
        "write_file" | "patch" => Category::Write,
        "search" | "find_files" => Category::Search,
        "bash" => Category::Run,
        _ => return None,
    })
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
    fn categorize_unknown_name_returns_none() {
        assert_eq!(categorize("frobnicate"), None);
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
            "bash",
        ];
        for name in built_ins {
            assert!(
                categorize(name).is_some(),
                "built-in tool {name} did not categorize to Some(_)"
            );
        }
    }
}
