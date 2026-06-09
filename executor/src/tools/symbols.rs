// symbols: tree-sitter symbol-definition search across Rust and Python files.
//
// Finds symbol definitions (functions, structs, classes, …) by exact identifier
// name, tagged with their kind. Where `search` is a blind regex grep, `symbols`
// parses each file and returns only definitions.

use anyhow::Result;
use async_trait::async_trait;
use ignore::Walk;
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::Path;
use std::sync::Arc;
use tree_sitter::{Parser, Query, QueryCursor, StreamingIterator};

use crate::security::scope::Scope;

use super::registry::{Tool, ToolResult};

const RUST_QUERY: &str = r#"
(function_item name: (identifier) @function)
(struct_item name: (type_identifier) @struct)
(enum_item name: (type_identifier) @enum)
(trait_item name: (type_identifier) @trait)
(mod_item name: (identifier) @module)
(const_item name: (identifier) @const)
(static_item name: (identifier) @static)
(type_item name: (type_identifier) @type)
(macro_definition name: (identifier) @macro)
"#;

const PYTHON_QUERY: &str = r#"
(function_definition name: (identifier) @function)
(class_definition name: (identifier) @class)
"#;

const VALID_KINDS: &[&str] = &[
    "function", "struct", "enum", "trait", "module", "const", "static", "type", "macro", "class",
];

const VALID_MODES: &[&str] = &["definitions", "references"];

const RUST_REF_QUERY: &str = r#"
(identifier) @ref
(type_identifier) @ref
(field_identifier) @ref
"#;

const PYTHON_REF_QUERY: &str = r#"
(identifier) @ref
"#;

#[derive(Deserialize)]
struct SymbolsArgs {
    name: Option<String>,
    path: Option<String>,
    kind: Option<String>,
    mode: Option<String>,
    max_results: Option<usize>,
}

struct SymbolHit {
    rel_path: String,
    line: usize,
    col: usize,
    kind: String,
    name: String,
}

struct RefHit {
    rel_path: String,
    line: usize,
    col: usize,
    snippet: String,
}

pub struct Symbols {
    scope: Scope,
}

#[async_trait]
impl Tool for Symbols {
    fn name(&self) -> &str {
        "symbols"
    }

    fn description(&self) -> &str {
        "Find symbol definitions or references by name across the scoped repo using tree-sitter. Supports Rust and Python. In references mode, `kind` is not supported."
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Symbol identifier to find. Exact match."
                },
                "path": {
                    "type": "string",
                    "description": "File or directory to search. Defaults to the project root. Confined to the project root."
                },
                "kind": {
                    "type": "string",
                    "description": "Restrict to one kind (definitions mode only). Valid: function, struct, enum, trait, module, const, static, type, macro, class."
                },
                "mode": {
                    "type": "string",
                    "description": "Search mode: \"definitions\" (default) or \"references\". In references mode, every syntactic usage of the name is returned."
                },
                "max_results": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Hard cap on total matches. Defaults to 100."
                }
            },
            "required": ["name"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let parsed = match serde_json::from_value::<SymbolsArgs>(args) {
            Ok(a) => a,
            Err(e) => {
                return Ok(ToolResult {
                    output: String::new(),
                    error: Some(format!("invalid arguments: {e}")),
                    metadata: None,
                });
            }
        };

        let name = match parsed.name {
            Some(n) if !n.is_empty() => n,
            _ => {
                return Ok(ToolResult {
                    output: String::new(),
                    error: Some("name must not be empty".to_string()),
                    metadata: None,
                });
            }
        };

        if let Some(ref k) = parsed.kind
            && !VALID_KINDS.contains(&k.as_str())
        {
            return Ok(ToolResult {
                output: String::new(),
                error: Some(format!(
                    "invalid kind: {k}. Valid kinds: {}",
                    VALID_KINDS.join(", ")
                )),
                metadata: None,
            });
        }

        let mode = parsed.mode.as_deref();
        if let Some(m) = mode
            && !VALID_MODES.contains(&m)
        {
            return Ok(ToolResult {
                output: String::new(),
                error: Some(format!(
                    "invalid mode: {m}. Valid modes: {}",
                    VALID_MODES.join(", ")
                )),
                metadata: None,
            });
        }

        if mode == Some("references") && parsed.kind.is_some() {
            return Ok(ToolResult {
                output: String::new(),
                error: Some("kind filter is not supported in references mode".to_string()),
                metadata: None,
            });
        }

        let is_references = mode == Some("references");

        let max_results = parsed.max_results.unwrap_or(100);
        if max_results < 1 {
            return Ok(ToolResult {
                output: String::new(),
                error: Some("max_results must be >= 1".to_string()),
                metadata: None,
            });
        }

        let search_root = match self.scope.resolve(parsed.path.as_deref().unwrap_or(".")) {
            Ok(p) => p,
            Err(e) => {
                return Ok(ToolResult {
                    output: String::new(),
                    error: Some(e.to_string()),
                    metadata: None,
                });
            }
        };

        if !search_root.exists() {
            return Ok(ToolResult {
                output: String::new(),
                error: Some(format!(
                    "search root does not exist: {}",
                    parsed.path.as_deref().unwrap_or(".")
                )),
                metadata: None,
            });
        }

        let abs_root = search_root
            .canonicalize()
            .unwrap_or_else(|_| search_root.clone());

        if search_root.is_file() {
            return execute_single_file(
                &abs_root,
                &name,
                parsed.kind.as_deref(),
                is_references,
                max_results,
            );
        }

        if is_references {
            let mut hits: Vec<RefHit> = Vec::new();

            for entry in Walk::new(&abs_root) {
                let entry = match entry {
                    Ok(e) => e,
                    Err(_) => continue,
                };

                if !entry.file_type().is_some_and(|ft| ft.is_file()) {
                    continue;
                }

                let ext = entry.path().extension().and_then(|e| e.to_str());
                let lang = match ext {
                    Some("rs") => Language::Rust,
                    Some("py") => Language::Python,
                    _ => continue,
                };

                let content = match std::fs::read_to_string(entry.path()) {
                    Ok(c) => c,
                    Err(_) => continue,
                };

                let file_hits = match parse_references(&content, lang, &name) {
                    Ok(h) => h,
                    Err(_) => continue,
                };

                let rel_path = entry
                    .path()
                    .strip_prefix(&abs_root)
                    .unwrap_or(entry.path())
                    .to_string_lossy()
                    .to_string();

                for h in file_hits {
                    hits.push(RefHit {
                        rel_path: rel_path.clone(),
                        ..h
                    });
                    if hits.len() >= max_results {
                        return Ok(format_references(&hits, &abs_root, &name, true));
                    }
                }
            }

            if hits.is_empty() {
                Ok(ToolResult {
                    output: String::new(),
                    error: Some(format!(
                        "no references to `{name}` in {}",
                        abs_root.display()
                    )),
                    metadata: None,
                })
            } else {
                Ok(format_references(&hits, &abs_root, &name, false))
            }
        } else {
            let mut hits: Vec<SymbolHit> = Vec::new();

            for entry in Walk::new(&abs_root) {
                let entry = match entry {
                    Ok(e) => e,
                    Err(_) => continue,
                };

                if !entry.file_type().is_some_and(|ft| ft.is_file()) {
                    continue;
                }

                let ext = entry.path().extension().and_then(|e| e.to_str());
                let lang = match ext {
                    Some("rs") => Language::Rust,
                    Some("py") => Language::Python,
                    _ => continue,
                };

                let content = match std::fs::read_to_string(entry.path()) {
                    Ok(c) => c,
                    Err(_) => continue,
                };

                let file_hits = match parse_file(&content, lang, &name, parsed.kind.as_deref()) {
                    Ok(h) => h,
                    Err(_) => continue,
                };

                let rel_path = entry
                    .path()
                    .strip_prefix(&abs_root)
                    .unwrap_or(entry.path())
                    .to_string_lossy()
                    .to_string();

                for h in file_hits {
                    hits.push(SymbolHit {
                        rel_path: rel_path.clone(),
                        ..h
                    });
                    if hits.len() >= max_results {
                        return Ok(format_output(&hits, &abs_root, &name, true));
                    }
                }
            }

            if hits.is_empty() {
                Ok(ToolResult {
                    output: String::new(),
                    error: Some(format!(
                        "no symbols named `{name}` in {}",
                        abs_root.display()
                    )),
                    metadata: None,
                })
            } else {
                Ok(format_output(&hits, &abs_root, &name, false))
            }
        }
    }
}

enum Language {
    Rust,
    Python,
}

fn execute_single_file(
    path: &std::path::PathBuf,
    name: &str,
    kind_filter: Option<&str>,
    is_references: bool,
    max_results: usize,
) -> Result<ToolResult> {
    let ext = path.extension().and_then(|e| e.to_str());
    let lang = match ext {
        Some("rs") => Language::Rust,
        Some("py") => Language::Python,
        _ => {
            return Ok(ToolResult {
                output: String::new(),
                error: Some(format!("unsupported file type: {}", path.display())),
                metadata: None,
            });
        }
    };

    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => {
            return Ok(ToolResult {
                output: String::new(),
                error: Some(format!("no symbols named `{name}` in {}", path.display())),
                metadata: None,
            });
        }
    };

    if is_references {
        let file_hits = match parse_references(&content, lang, name) {
            Ok(h) => h,
            Err(_) => {
                return Ok(ToolResult {
                    output: String::new(),
                    error: Some(format!("no references to `{name}` in {}", path.display())),
                    metadata: None,
                });
            }
        };

        let mut hits: Vec<RefHit> = file_hits;
        let truncated = hits.len() >= max_results;
        hits.truncate(max_results);

        if hits.is_empty() {
            Ok(ToolResult {
                output: String::new(),
                error: Some(format!("no references to `{name}` in {}", path.display())),
                metadata: None,
            })
        } else {
            Ok(format_references(&hits, path, name, truncated))
        }
    } else {
        let file_hits = match parse_file(&content, lang, name, kind_filter) {
            Ok(h) => h,
            Err(_) => {
                return Ok(ToolResult {
                    output: String::new(),
                    error: Some(format!("no symbols named `{name}` in {}", path.display())),
                    metadata: None,
                });
            }
        };

        let mut hits: Vec<SymbolHit> = file_hits;
        let truncated = hits.len() >= max_results;
        hits.truncate(max_results);

        if hits.is_empty() {
            Ok(ToolResult {
                output: String::new(),
                error: Some(format!("no symbols named `{name}` in {}", path.display())),
                metadata: None,
            })
        } else {
            Ok(format_output(&hits, path, name, truncated))
        }
    }
}

fn parse_file(
    source: &str,
    lang: Language,
    requested_name: &str,
    kind_filter: Option<&str>,
) -> Result<Vec<SymbolHit>> {
    let (language, query_str) = match lang {
        Language::Rust => (tree_sitter_rust::LANGUAGE.into(), RUST_QUERY),
        Language::Python => (tree_sitter_python::LANGUAGE.into(), PYTHON_QUERY),
    };

    let language: tree_sitter::Language = language;
    let query = Query::new(&language, query_str)?;

    let mut parser = Parser::new();
    parser.set_language(&language)?;

    let Some(tree) = parser.parse(source.as_bytes(), None) else {
        return Ok(Vec::new());
    };

    let capture_names = query.capture_names();
    let mut cursor = QueryCursor::new();
    let mut it = cursor.matches(&query, tree.root_node(), source.as_bytes());

    let mut hits = Vec::new();
    while let Some(m) = it.next() {
        for cap in m.captures {
            let idx = cap.index as usize;
            let kind = capture_names.get(idx).copied().unwrap_or("");
            let Some(text) = source.get(cap.node.byte_range()) else {
                continue;
            };
            if text != requested_name {
                continue;
            }
            if let Some(want) = kind_filter
                && want != kind
            {
                continue;
            }
            let p = cap.node.start_position();
            hits.push(SymbolHit {
                rel_path: String::new(),
                line: p.row + 1,
                col: p.column + 1,
                kind: kind.to_string(),
                name: text.to_string(),
            });
        }
    }

    Ok(hits)
}

fn parse_references(source: &str, lang: Language, requested_name: &str) -> Result<Vec<RefHit>> {
    let (language, query_str) = match lang {
        Language::Rust => (tree_sitter_rust::LANGUAGE.into(), RUST_REF_QUERY),
        Language::Python => (tree_sitter_python::LANGUAGE.into(), PYTHON_REF_QUERY),
    };

    let language: tree_sitter::Language = language;
    let query = Query::new(&language, query_str)?;

    let mut parser = Parser::new();
    parser.set_language(&language)?;

    let Some(tree) = parser.parse(source.as_bytes(), None) else {
        return Ok(Vec::new());
    };

    let mut cursor = QueryCursor::new();
    let mut it = cursor.matches(&query, tree.root_node(), source.as_bytes());

    let mut hits = Vec::new();
    while let Some(m) = it.next() {
        for cap in m.captures {
            let Some(text) = source.get(cap.node.byte_range()) else {
                continue;
            };
            if text != requested_name {
                continue;
            }
            let p = cap.node.start_position();
            let snippet = source.lines().nth(p.row).unwrap_or("").trim().to_string();
            hits.push(RefHit {
                rel_path: String::new(),
                line: p.row + 1,
                col: p.column + 1,
                snippet,
            });
        }
    }

    Ok(hits)
}

fn format_output(hits: &[SymbolHit], abs_root: &Path, name: &str, truncated: bool) -> ToolResult {
    let file_set: std::collections::HashSet<&str> =
        hits.iter().map(|h| h.rel_path.as_str()).collect();
    let file_count = file_set.len();

    let mut output = format!("✓ {} definitions of `{name}`\n\n", hits.len());

    let mut current_file = "";
    for hit in hits {
        if hit.rel_path != current_file {
            if !current_file.is_empty() {
                output.push('\n');
            }
            output.push_str(&format!("{}:\n", hit.rel_path));
            current_file = &hit.rel_path;
        }
        output.push_str(&format!(
            "  {}:{}  {} {}\n",
            hit.line, hit.col, hit.kind, hit.name
        ));
    }

    if truncated {
        output.push_str(&format!(
            "\n[… truncated at {} definitions; narrow your path or add a kind filter to see more …]",
            hits.len()
        ));
    }

    let metadata = json!({
        "path": abs_root.to_string_lossy(),
        "name": name,
        "definitions": hits.len(),
        "files": file_count,
        "truncated": truncated,
    });

    ToolResult {
        output,
        error: None,
        metadata: Some(metadata),
    }
}

fn format_references(hits: &[RefHit], abs_root: &Path, name: &str, truncated: bool) -> ToolResult {
    let file_set: std::collections::HashSet<&str> =
        hits.iter().map(|h| h.rel_path.as_str()).collect();
    let file_count = file_set.len();

    let mut output = format!("✓ {} references to `{name}`\n\n", hits.len());

    let mut current_file = "";
    for hit in hits {
        if hit.rel_path != current_file {
            if !current_file.is_empty() {
                output.push('\n');
            }
            output.push_str(&format!("{}:\n", hit.rel_path));
            current_file = &hit.rel_path;
        }
        output.push_str(&format!("  {}:{}  {}\n", hit.line, hit.col, hit.snippet));
    }

    if truncated {
        output.push_str(&format!(
            "\n[… truncated at {} references; narrow your path or add a kind filter to see more …]",
            hits.len()
        ));
    }

    let metadata = json!({
        "path": abs_root.to_string_lossy(),
        "name": name,
        "references": hits.len(),
        "files": file_count,
        "truncated": truncated,
    });

    ToolResult {
        output,
        error: None,
        metadata: Some(metadata),
    }
}

pub fn symbols(scope: Scope) -> Arc<dyn Tool> {
    Arc::new(Symbols { scope })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_scope(dir: &tempfile::TempDir) -> Scope {
        Scope::new(dir.path()).unwrap()
    }

    fn write_files(dir: &std::path::Path, files: &[(&str, &str)]) {
        for (path, content) in files {
            let full = dir.join(path);
            if let Some(parent) = full.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(full, content).unwrap();
        }
    }

    #[tokio::test]
    async fn finds_rust_function_by_name() {
        let dir = tempfile::TempDir::new().unwrap();
        write_files(
            dir.path(),
            &[("src/lib.rs", "pub fn foo() {}\nfn bar() {}\n")],
        );

        let tool = symbols(make_scope(&dir));
        let result = tool
            .execute(json!({
                "name": "foo",
                "path": dir.path().to_string_lossy()
            }))
            .await
            .unwrap();

        assert!(
            result.error.is_none(),
            "unexpected error: {:?}",
            result.error
        );
        assert!(result.output.contains("✓ 1 definitions of `foo`"));
        assert!(result.output.contains("function foo"));
    }

    #[tokio::test]
    async fn finds_rust_struct_and_trait() {
        let dir = tempfile::TempDir::new().unwrap();
        write_files(
            dir.path(),
            &[(
                "src/lib.rs",
                "struct MyStruct {}\ntrait MyTrait {}\nenum MyEnum {}\n",
            )],
        );

        let tool = symbols(make_scope(&dir));
        let result = tool
            .execute(json!({
                "name": "MyStruct",
                "path": dir.path().to_string_lossy()
            }))
            .await
            .unwrap();

        assert!(result.error.is_none());
        assert!(result.output.contains("struct MyStruct"));

        let result = tool
            .execute(json!({
                "name": "MyTrait",
                "path": dir.path().to_string_lossy()
            }))
            .await
            .unwrap();

        assert!(result.error.is_none());
        assert!(result.output.contains("trait MyTrait"));

        let result = tool
            .execute(json!({
                "name": "MyEnum",
                "path": dir.path().to_string_lossy()
            }))
            .await
            .unwrap();

        assert!(result.error.is_none());
        assert!(result.output.contains("enum MyEnum"));
    }

    #[tokio::test]
    async fn kind_filter_returns_only_matching_kind() {
        let dir = tempfile::TempDir::new().unwrap();
        write_files(
            dir.path(),
            &[("src/lib.rs", "fn foo() {}\nstruct foo {}\n")],
        );

        let tool = symbols(make_scope(&dir));
        let result = tool
            .execute(json!({
                "name": "foo",
                "kind": "struct",
                "path": dir.path().to_string_lossy()
            }))
            .await
            .unwrap();

        assert!(result.error.is_none());
        assert!(result.output.contains("struct foo"));
        assert!(!result.output.contains("function foo"));
    }

    #[tokio::test]
    async fn exact_match_no_substring() {
        let dir = tempfile::TempDir::new().unwrap();
        write_files(
            dir.path(),
            &[("src/lib.rs", "fn foobar() {}\nfn foo() {}\n")],
        );

        let tool = symbols(make_scope(&dir));
        let result = tool
            .execute(json!({
                "name": "foo",
                "path": dir.path().to_string_lossy()
            }))
            .await
            .unwrap();

        assert!(result.error.is_none());
        assert!(result.output.contains("function foo"));
        assert!(!result.output.contains("foobar"));
    }

    #[tokio::test]
    async fn respects_gitignore() {
        let dir = tempfile::TempDir::new().unwrap();
        write_files(
            dir.path(),
            &[
                (".gitignore", "ignored.rs\n"),
                ("keep.rs", "fn foo() {}\n"),
                ("ignored.rs", "fn foo() {}\n"),
            ],
        );

        std::process::Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        let tool = symbols(make_scope(&dir));
        let result = tool
            .execute(json!({
                "name": "foo",
                "path": dir.path().to_string_lossy()
            }))
            .await
            .unwrap();

        assert!(result.error.is_none());
        assert!(result.output.contains("keep.rs"));
        assert!(!result.output.contains("ignored.rs"));
    }

    #[tokio::test]
    async fn caps_at_max_results() {
        let dir = tempfile::TempDir::new().unwrap();
        let content = (1..=10)
            .map(|i| format!("fn foo{i}() {{}}"))
            .collect::<Vec<_>>()
            .join("\n");
        write_files(dir.path(), &[("src/lib.rs", &content)]);

        let tool = symbols(make_scope(&dir));
        let result = tool
            .execute(json!({
                "name": "foo1",
                "path": dir.path().to_string_lossy(),
                "max_results": 1
            }))
            .await
            .unwrap();

        assert!(result.error.is_none());
        let meta = result.metadata.unwrap();
        assert!(meta["truncated"].as_bool().unwrap());
    }

    #[tokio::test]
    async fn finds_python_function_and_class() {
        let dir = tempfile::TempDir::new().unwrap();
        write_files(
            dir.path(),
            &[(
                "src/main.py",
                "def foo():\n    pass\n\nclass Bar:\n    pass\n",
            )],
        );

        let tool = symbols(make_scope(&dir));

        let result = tool
            .execute(json!({
                "name": "foo",
                "path": dir.path().to_string_lossy()
            }))
            .await
            .unwrap();

        assert!(result.error.is_none());
        assert!(result.output.contains("function foo"));

        let result = tool
            .execute(json!({
                "name": "Bar",
                "path": dir.path().to_string_lossy()
            }))
            .await
            .unwrap();

        assert!(result.error.is_none());
        assert!(result.output.contains("class Bar"));
    }

    #[tokio::test]
    async fn unsupported_extension_skipped_in_dir_walk() {
        let dir = tempfile::TempDir::new().unwrap();
        write_files(
            dir.path(),
            &[
                ("src/lib.rs", "fn foo() {}\n"),
                ("notes.txt", "fn foo() {}\n"),
            ],
        );

        let tool = symbols(make_scope(&dir));
        let result = tool
            .execute(json!({
                "name": "foo",
                "path": dir.path().to_string_lossy()
            }))
            .await
            .unwrap();

        assert!(result.error.is_none());
        assert!(result.output.contains("lib.rs"));
        assert!(!result.output.contains("notes.txt"));
    }

    #[tokio::test]
    async fn single_file_unsupported_extension_advisory_error() {
        let dir = tempfile::TempDir::new().unwrap();
        let file = dir.path().join("notes.txt");
        std::fs::write(&file, "fn foo() {}\n").unwrap();

        let tool = symbols(make_scope(&dir));
        let result = tool
            .execute(json!({
                "name": "foo",
                "path": file.to_string_lossy()
            }))
            .await
            .unwrap();

        assert!(result.error.is_some());
        assert!(
            result
                .error
                .as_ref()
                .unwrap()
                .contains("unsupported file type")
        );
    }

    #[tokio::test]
    async fn rejects_empty_name() {
        let dir = tempfile::TempDir::new().unwrap();
        write_files(dir.path(), &[("src/lib.rs", "fn foo() {}\n")]);

        let tool = symbols(make_scope(&dir));
        let result = tool
            .execute(json!({
                "name": "",
                "path": dir.path().to_string_lossy()
            }))
            .await
            .unwrap();

        assert!(result.error.is_some());
        assert!(
            result
                .error
                .as_ref()
                .unwrap()
                .contains("name must not be empty")
        );
    }

    #[tokio::test]
    async fn rejects_invalid_kind() {
        let dir = tempfile::TempDir::new().unwrap();
        write_files(dir.path(), &[("src/lib.rs", "fn foo() {}\n")]);

        let tool = symbols(make_scope(&dir));
        let result = tool
            .execute(json!({
                "name": "foo",
                "kind": "invalid_kind",
                "path": dir.path().to_string_lossy()
            }))
            .await
            .unwrap();

        assert!(result.error.is_some());
        let err = result.error.as_ref().unwrap();
        assert!(err.contains("invalid kind"));
        assert!(err.contains("function"));
        assert!(err.contains("struct"));
    }

    #[tokio::test]
    async fn rejects_max_results_zero() {
        let dir = tempfile::TempDir::new().unwrap();
        write_files(dir.path(), &[("src/lib.rs", "fn foo() {}\n")]);

        let tool = symbols(make_scope(&dir));
        let result = tool
            .execute(json!({
                "name": "foo",
                "max_results": 0,
                "path": dir.path().to_string_lossy()
            }))
            .await
            .unwrap();

        assert!(result.error.is_some());
        assert!(
            result
                .error
                .as_ref()
                .unwrap()
                .contains("max_results must be >= 1")
        );
    }

    #[tokio::test]
    async fn rejects_path_outside_root() {
        let dir = tempfile::TempDir::new().unwrap();
        write_files(dir.path(), &[("src/lib.rs", "fn foo() {}\n")]);

        let tool = symbols(make_scope(&dir));
        let result = tool
            .execute(json!({
                "name": "foo",
                "path": "/nonexistent/outside/root"
            }))
            .await
            .unwrap();

        assert!(result.error.is_some());
        assert!(result.error.as_ref().unwrap().contains("escapes"));
    }

    #[tokio::test]
    async fn rejects_nonexistent_path() {
        let dir = tempfile::TempDir::new().unwrap();
        write_files(dir.path(), &[("src/lib.rs", "fn foo() {}\n")]);

        let tool = symbols(make_scope(&dir));
        let result = tool
            .execute(json!({
                "name": "foo",
                "path": "nonexistent_dir"
            }))
            .await
            .unwrap();

        assert!(result.error.is_some());
        assert!(result.error.as_ref().unwrap().contains("does not exist"));
    }

    #[tokio::test]
    async fn reports_line_and_column() {
        let dir = tempfile::TempDir::new().unwrap();
        write_files(dir.path(), &[("src/lib.rs", "fn foo() {}\n")]);

        let tool = symbols(make_scope(&dir));
        let result = tool
            .execute(json!({
                "name": "foo",
                "path": dir.path().to_string_lossy()
            }))
            .await
            .unwrap();

        assert!(result.error.is_none());
        assert!(result.output.contains("1:4"));
    }

    #[tokio::test]
    async fn defaults_to_scope_root_when_no_path_given() {
        let dir = tempfile::TempDir::new().unwrap();
        write_files(dir.path(), &[("src/lib.rs", "fn foo() {}\n")]);

        let tool = symbols(make_scope(&dir));
        let result = tool
            .execute(json!({
                "name": "foo"
            }))
            .await
            .unwrap();

        assert!(result.error.is_none());
        assert!(result.output.contains("function foo"));
    }

    #[tokio::test]
    async fn no_symbols_returns_advisory_error() {
        let dir = tempfile::TempDir::new().unwrap();
        write_files(dir.path(), &[("src/lib.rs", "fn foo() {}\n")]);

        let tool = symbols(make_scope(&dir));
        let result = tool
            .execute(json!({
                "name": "nonexistent",
                "path": dir.path().to_string_lossy()
            }))
            .await
            .unwrap();

        assert!(result.error.is_some());
        assert!(result.error.as_ref().unwrap().contains("no symbols named"));
    }

    #[tokio::test]
    async fn metadata_carries_definitions_and_files_count() {
        let dir = tempfile::TempDir::new().unwrap();
        write_files(
            dir.path(),
            &[("a.rs", "fn foo() {}\n"), ("b.rs", "fn foo() {}\n")],
        );

        let tool = symbols(make_scope(&dir));
        let result = tool
            .execute(json!({
                "name": "foo",
                "path": dir.path().to_string_lossy()
            }))
            .await
            .unwrap();

        assert!(result.error.is_none());
        let meta = result.metadata.unwrap();
        assert_eq!(meta["definitions"].as_u64().unwrap(), 2);
        assert_eq!(meta["files"].as_u64().unwrap(), 2);
        assert!(!meta["truncated"].as_bool().unwrap());
    }

    #[tokio::test]
    async fn references_finds_call_sites() {
        let dir = tempfile::TempDir::new().unwrap();
        write_files(
            dir.path(),
            &[("src/lib.rs", "fn foo() { foo(); foo(); }\n")],
        );

        let tool = symbols(make_scope(&dir));
        let result = tool
            .execute(json!({
                "name": "foo",
                "mode": "references",
                "path": dir.path().to_string_lossy()
            }))
            .await
            .unwrap();

        assert!(
            result.error.is_none(),
            "unexpected error: {:?}",
            result.error
        );
        assert!(result.output.contains("✓ 3 references to `foo`"));
        let meta = result.metadata.as_ref().unwrap();
        assert_eq!(meta["references"].as_u64().unwrap(), 3);
    }

    #[tokio::test]
    async fn references_exclude_substring() {
        let dir = tempfile::TempDir::new().unwrap();
        write_files(
            dir.path(),
            &[("src/lib.rs", "fn foobar() {}\nfn foo() { foo(); }\n")],
        );

        let tool = symbols(make_scope(&dir));
        let result = tool
            .execute(json!({
                "name": "foo",
                "mode": "references",
                "path": dir.path().to_string_lossy()
            }))
            .await
            .unwrap();

        assert!(
            result.error.is_none(),
            "unexpected error: {:?}",
            result.error
        );
        let meta = result.metadata.as_ref().unwrap();
        let count = meta["references"].as_u64().unwrap();
        // foo def + foo call = 2, foobar occurrences should not match
        assert_eq!(count, 2);
    }

    #[tokio::test]
    async fn references_exclude_strings_and_comments() {
        let dir = tempfile::TempDir::new().unwrap();
        write_files(
            dir.path(),
            &[(
                "src/lib.rs",
                "fn foo() { foo(); }\n// foo again\nfn other() { let s = \"foo\"; }\n",
            )],
        );

        let tool = symbols(make_scope(&dir));
        let result = tool
            .execute(json!({
                "name": "foo",
                "mode": "references",
                "path": dir.path().to_string_lossy()
            }))
            .await
            .unwrap();

        assert!(
            result.error.is_none(),
            "unexpected error: {:?}",
            result.error
        );
        let meta = result.metadata.as_ref().unwrap();
        // def + call = 2; comment and string literal excluded
        assert_eq!(meta["references"].as_u64().unwrap(), 2);
    }

    #[tokio::test]
    async fn references_across_multiple_files() {
        let dir = tempfile::TempDir::new().unwrap();
        write_files(
            dir.path(),
            &[
                ("a.rs", "fn foo() { foo(); }\n"),
                ("b.rs", "fn bar() { foo(); }\n"),
            ],
        );

        let tool = symbols(make_scope(&dir));
        let result = tool
            .execute(json!({
                "name": "foo",
                "mode": "references",
                "path": dir.path().to_string_lossy()
            }))
            .await
            .unwrap();

        assert!(
            result.error.is_none(),
            "unexpected error: {:?}",
            result.error
        );
        let meta = result.metadata.as_ref().unwrap();
        assert_eq!(meta["files"].as_u64().unwrap(), 2);
        // a.rs: foo def + foo call = 2; b.rs: foo call = 1; total = 3
        assert_eq!(meta["references"].as_u64().unwrap(), 3);
    }

    #[tokio::test]
    async fn references_python_identifier() {
        let dir = tempfile::TempDir::new().unwrap();
        write_files(
            dir.path(),
            &[("src/lib.py", "def foo():\n    pass\nfoo()\n")],
        );

        let tool = symbols(make_scope(&dir));
        let result = tool
            .execute(json!({
                "name": "foo",
                "mode": "references",
                "path": dir.path().to_string_lossy()
            }))
            .await
            .unwrap();

        assert!(
            result.error.is_none(),
            "unexpected error: {:?}",
            result.error
        );
        let meta = result.metadata.as_ref().unwrap();
        // def + call = 2
        assert_eq!(meta["references"].as_u64().unwrap(), 2);
    }

    #[tokio::test]
    async fn references_respects_max_results() {
        let dir = tempfile::TempDir::new().unwrap();
        write_files(
            dir.path(),
            &[("src/lib.rs", "fn foo() { foo(); foo(); foo(); }\n")],
        );

        let tool = symbols(make_scope(&dir));
        let result = tool
            .execute(json!({
                "name": "foo",
                "mode": "references",
                "max_results": 1,
                "path": dir.path().to_string_lossy()
            }))
            .await
            .unwrap();

        assert!(
            result.error.is_none(),
            "unexpected error: {:?}",
            result.error
        );
        let meta = result.metadata.as_ref().unwrap();
        assert!(meta["truncated"].as_bool().unwrap());
    }

    #[tokio::test]
    async fn references_kind_filter_rejected() {
        let dir = tempfile::TempDir::new().unwrap();
        write_files(dir.path(), &[("src/lib.rs", "fn foo() {}\n")]);

        let tool = symbols(make_scope(&dir));
        let result = tool
            .execute(json!({
                "name": "foo",
                "mode": "references",
                "kind": "function",
                "path": dir.path().to_string_lossy()
            }))
            .await
            .unwrap();

        assert!(result.error.is_some());
        assert!(
            result
                .error
                .as_ref()
                .unwrap()
                .contains("not supported in references mode")
        );
    }

    #[tokio::test]
    async fn references_invalid_mode_rejected() {
        let dir = tempfile::TempDir::new().unwrap();
        write_files(dir.path(), &[("src/lib.rs", "fn foo() {}\n")]);

        let tool = symbols(make_scope(&dir));
        let result = tool
            .execute(json!({
                "name": "foo",
                "mode": "usages",
                "path": dir.path().to_string_lossy()
            }))
            .await
            .unwrap();

        assert!(result.error.is_some());
        assert!(result.error.as_ref().unwrap().contains("invalid mode"));
    }

    #[tokio::test]
    async fn references_single_file_path() {
        let dir = tempfile::TempDir::new().unwrap();
        let file = dir.path().join("src/lib.rs");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "fn foo() { foo(); }\n").unwrap();

        let tool = symbols(make_scope(&dir));
        let result = tool
            .execute(json!({
                "name": "foo",
                "mode": "references",
                "path": file.to_string_lossy()
            }))
            .await
            .unwrap();

        assert!(
            result.error.is_none(),
            "unexpected error: {:?}",
            result.error
        );
        let meta = result.metadata.as_ref().unwrap();
        assert_eq!(meta["references"].as_u64().unwrap(), 2);
    }

    #[tokio::test]
    async fn references_no_matches_advisory() {
        let dir = tempfile::TempDir::new().unwrap();
        write_files(dir.path(), &[("src/lib.rs", "fn foo() {}\n")]);

        let tool = symbols(make_scope(&dir));
        let result = tool
            .execute(json!({
                "name": "nonexistent",
                "mode": "references",
                "path": dir.path().to_string_lossy()
            }))
            .await
            .unwrap();

        assert!(result.error.is_some());
        assert!(result.error.as_ref().unwrap().contains("no references to"));
    }

    #[tokio::test]
    async fn references_snippet_shows_source_line() {
        let dir = tempfile::TempDir::new().unwrap();
        write_files(dir.path(), &[("src/lib.rs", "fn foo() { foo(); }\n")]);

        let tool = symbols(make_scope(&dir));
        let result = tool
            .execute(json!({
                "name": "foo",
                "mode": "references",
                "path": dir.path().to_string_lossy()
            }))
            .await
            .unwrap();

        assert!(
            result.error.is_none(),
            "unexpected error: {:?}",
            result.error
        );
        assert!(result.output.contains("foo();"));
    }
}
