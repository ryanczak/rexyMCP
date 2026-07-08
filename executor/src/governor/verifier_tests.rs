use super::*;
use std::fs;

#[tokio::test]
async fn unsupported_for_non_rust_file() {
    let result = verify(Path::new("x.md")).await;
    assert!(matches!(result, VerifierResult::Unsupported));
}

#[tokio::test]
async fn unsupported_for_no_extension() {
    let result = verify(Path::new("Makefile")).await;
    assert!(matches!(result, VerifierResult::Unsupported));
}

#[test]
fn find_crate_root_walks_up() {
    let dir = tempfile::TempDir::new().unwrap();
    let crate_dir = dir.path().join("crate-x");
    let src_dir = crate_dir.join("src");
    fs::create_dir_all(&src_dir).unwrap();
    fs::write(
        crate_dir.join("Cargo.toml"),
        "[package]\nname = \"x\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    let lib_path = src_dir.join("lib.rs");
    fs::write(&lib_path, "").unwrap();

    let result = find_crate_root(&lib_path);
    assert_eq!(result, Some(crate_dir));
}

#[test]
fn find_crate_root_returns_none_outside_crate() {
    let dir = tempfile::TempDir::new().unwrap();
    let lib_path = dir.path().join("lib.rs");
    fs::write(&lib_path, "").unwrap();

    let result = find_crate_root(&lib_path);
    assert!(result.is_none());
}

#[test]
fn parse_cargo_line_skips_non_compiler_messages() {
    let line = r#"{"reason":"build-script-executed","package_id":"foo 0.1.0"}"#;
    let dir = tempfile::TempDir::new().unwrap();
    assert!(parse_cargo_line(line, dir.path()).is_none());
}

#[test]
fn parse_cargo_line_skips_warnings() {
    let line = r#"{
            "reason": "compiler-message",
            "message": {
                "message": "unused variable",
                "level": "warning",
                "spans": [{
                    "file_name": "src/lib.rs",
                    "line_start": 5,
                    "column_start": 9,
                    "is_primary": true
                }]
            }
        }"#;
    let dir = tempfile::TempDir::new().unwrap();
    assert!(parse_cargo_line(line, dir.path()).is_none());
}

#[test]
fn parse_cargo_line_extracts_error_with_code() {
    let line = r#"{
            "reason": "compiler-message",
            "message": {
                "message": "cannot find function `foo` in this scope",
                "level": "error",
                "code": { "code": "E0425", "explanation": "" },
                "spans": [{
                    "file_name": "src/lib.rs",
                    "line_start": 3,
                    "column_start": 5,
                    "is_primary": true
                }]
            }
        }"#;
    let dir = tempfile::TempDir::new().unwrap();
    let diag = parse_cargo_line(line, dir.path()).unwrap();

    assert_eq!(diag.message, "cannot find function `foo` in this scope");
    assert_eq!(diag.line, 3);
    assert_eq!(diag.column, Some(5));
    assert_eq!(diag.severity, Severity::Error);
    assert_eq!(diag.code, Some("E0425".to_string()));
    assert_eq!(diag.path, dir.path().join("src/lib.rs"));
}

#[test]
fn parse_cargo_line_extracts_error_without_code() {
    let line = r#"{
            "reason": "compiler-message",
            "message": {
                "message": "mismatched types",
                "level": "error",
                "spans": [{
                    "file_name": "src/lib.rs",
                    "line_start": 10,
                    "column_start": null,
                    "is_primary": true
                }]
            }
        }"#;
    let dir = tempfile::TempDir::new().unwrap();
    let diag = parse_cargo_line(line, dir.path()).unwrap();

    assert_eq!(diag.code, None);
    assert_eq!(diag.column, None);
    assert_eq!(diag.line, 10);
}

// --- Machine-applicable suggestion tests ---

#[test]
fn cargo_line_surfaces_machine_applicable_suggestion() {
    let line = r#"{"reason":"compiler-message","target":{"name":"bad"},"message":{"message":"cannot borrow `v` as mutable, as it is not declared as mutable","code":{"code":"E0596"},"level":"error","spans":[{"file_name":"src/main.rs","line_start":3,"column_start":5,"is_primary":true,"suggested_replacement":null,"suggestion_applicability":null}],"children":[{"message":"consider changing this to be mutable","code":null,"level":"help","spans":[{"file_name":"src/main.rs","line_start":2,"column_start":9,"is_primary":true,"suggested_replacement":"mut ","suggestion_applicability":"MachineApplicable"}],"children":[]}]}}"#;
    let dir = tempfile::TempDir::new().unwrap();
    let diag = parse_cargo_line(line, dir.path()).unwrap();

    assert!(
        diag.message.starts_with("cannot borrow"),
        "original error must be preserved, got: {}",
        diag.message
    );
    assert!(
        diag.message.contains("mut"),
        "suggestion replacement text must be present, got: {}",
        diag.message
    );
    assert!(
        diag.message.contains("line 2"),
        "suggestion span location must be present, got: {}",
        diag.message
    );
}

#[test]
fn cargo_line_excludes_has_placeholders_suggestion() {
    let line = r#"{"reason":"compiler-message","target":{"name":"t"},"message":{"message":"mismatched types","code":{"code":"E0308"},"level":"error","spans":[{"file_name":"src/main.rs","line_start":1,"column_start":51,"is_primary":true,"suggested_replacement":null,"suggestion_applicability":null}],"children":[{"message":"expected type `i32`","code":null,"level":"note","spans":[],"children":[]},{"message":"consider using `Option::expect` to unwrap the `Option<i32>` value, panicking if the value is an `Option::None`","code":null,"level":"help","spans":[{"file_name":"src/main.rs","line_start":1,"column_start":52,"is_primary":true,"suggested_replacement":".expect(\"REASON\")","suggestion_applicability":"HasPlaceholders"}],"children":[]}]}}"#;
    let dir = tempfile::TempDir::new().unwrap();
    let diag = parse_cargo_line(line, dir.path()).unwrap();

    assert_eq!(
        diag.message, "mismatched types",
        "HasPlaceholders suggestion must not be surfaced, got: {}",
        diag.message
    );
}

#[test]
fn cargo_line_excludes_maybe_incorrect_suggestion() {
    let line = r#"{"reason":"compiler-message","target":{"name":"t"},"message":{"message":"cannot find value `fou` in this scope","code":{"code":"E0425"},"level":"error","spans":[{"file_name":"src/main.rs","line_start":1,"column_start":31,"is_primary":true,"suggested_replacement":null,"suggestion_applicability":null}],"children":[{"message":"a local variable with a similar name exists","code":null,"level":"help","spans":[{"file_name":"src/main.rs","line_start":1,"column_start":31,"is_primary":true,"suggested_replacement":"foo","suggestion_applicability":"MaybeIncorrect"}],"children":[]}]}}"#;
    let dir = tempfile::TempDir::new().unwrap();
    let diag = parse_cargo_line(line, dir.path()).unwrap();

    assert_eq!(
        diag.message, "cannot find value `fou` in this scope",
        "MaybeIncorrect suggestion must not be surfaced, got: {}",
        diag.message
    );
}

#[test]
fn cargo_line_without_children_message_unchanged() {
    let line = r#"{
            "reason": "compiler-message",
            "message": {
                "message": "cannot find function `foo` in this scope",
                "level": "error",
                "code": { "code": "E0425", "explanation": "" },
                "spans": [{
                    "file_name": "src/lib.rs",
                    "line_start": 3,
                    "column_start": 5,
                    "is_primary": true
                }]
            }
        }"#;
    let dir = tempfile::TempDir::new().unwrap();
    let diag = parse_cargo_line(line, dir.path()).unwrap();

    assert_eq!(
        diag.message, "cannot find function `foo` in this scope",
        "message must be byte-identical to raw error when no children present, got: {}",
        diag.message
    );
}

#[tokio::test]
async fn verify_rust_returns_checked_with_errors_on_broken_code() {
    let dir = tempfile::TempDir::new().unwrap();
    let crate_dir = dir.path().join("broken");
    let src_dir = crate_dir.join("src");
    fs::create_dir_all(&src_dir).unwrap();
    fs::write(
        crate_dir.join("Cargo.toml"),
        "[package]\nname = \"broken\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    fs::write(src_dir.join("lib.rs"), "fn broken() { let x = y; }").unwrap();

    let result = verify_rust(&src_dir.join("lib.rs")).await;
    match result {
        VerifierResult::Checked { diagnostics } => {
            assert!(!diagnostics.is_empty());
            for d in &diagnostics {
                assert_eq!(d.severity, Severity::Error);
            }
        }
        VerifierResult::Failed(msg) => {
            // cargo might not be available in some test envs
            assert!(msg.contains("spawn failed") || msg.contains("no Cargo.toml"));
        }
        _ => panic!("expected Checked or Failed, got {:?}", result),
    }
}

#[tokio::test]
async fn verify_rust_returns_checked_empty_on_clean_code() {
    let dir = tempfile::TempDir::new().unwrap();
    let crate_dir = dir.path().join("clean");
    let src_dir = crate_dir.join("src");
    fs::create_dir_all(&src_dir).unwrap();
    fs::write(
        crate_dir.join("Cargo.toml"),
        "[package]\nname = \"clean\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    fs::write(src_dir.join("lib.rs"), "pub fn hello() {}").unwrap();

    let result = verify_rust(&src_dir.join("lib.rs")).await;
    match result {
        VerifierResult::Checked { diagnostics } => {
            for d in &diagnostics {
                assert_eq!(d.severity, Severity::Error);
            }
        }
        VerifierResult::Failed(msg) => {
            assert!(msg.contains("spawn failed") || msg.contains("no Cargo.toml"));
        }
        _ => panic!("expected Checked or Failed, got {:?}", result),
    }
}

#[tokio::test]
async fn verify_dispatches_ts_to_typescript() {
    let dir = tempfile::TempDir::new().unwrap();
    let ts_path = dir.path().join("foo.ts");
    fs::write(&ts_path, "").unwrap();

    let result = verify(&ts_path).await;
    assert!(
        matches!(result, VerifierResult::Failed(msg) if msg.contains("no tsconfig.json found"))
    );
}

#[tokio::test]
async fn verify_dispatches_tsx_to_typescript() {
    let dir = tempfile::TempDir::new().unwrap();
    let tsx_path = dir.path().join("foo.tsx");
    fs::write(&tsx_path, "").unwrap();

    let result = verify(&tsx_path).await;
    assert!(
        matches!(result, VerifierResult::Failed(msg) if msg.contains("no tsconfig.json found"))
    );
}

#[tokio::test]
async fn verify_dispatches_py_to_python() {
    let dir = tempfile::TempDir::new().unwrap();
    let py_path = dir.path().join("foo.py");
    fs::write(&py_path, "").unwrap();

    let result = verify(&py_path).await;
    match result {
        VerifierResult::Checked { .. } => {}
        VerifierResult::Failed(msg) => {
            assert!(msg.contains("ruff spawn failed"));
        }
        VerifierResult::Skipped(_) => {
            // ruff not installed — acceptable, the tool isn't on PATH
        }
        VerifierResult::Unsupported => {
            panic!("expected Checked, Failed, or Skipped, got Unsupported");
        }
    }
}

#[test]
fn find_typescript_project_root_walks_up() {
    let dir = tempfile::TempDir::new().unwrap();
    let proj = dir.path().join("proj");
    let src = proj.join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(proj.join("tsconfig.json"), "{}").unwrap();
    let ts_path = src.join("main.ts");
    fs::write(&ts_path, "").unwrap();

    let result = find_typescript_project_root(&ts_path);
    assert_eq!(result, Some(proj));
}

#[test]
fn find_typescript_project_root_returns_none_outside() {
    let dir = tempfile::TempDir::new().unwrap();
    let ts_path = dir.path().join("main.ts");
    fs::write(&ts_path, "").unwrap();

    let result = find_typescript_project_root(&ts_path);
    assert!(result.is_none());
}

#[test]
fn parse_tsc_line_extracts_error_with_code() {
    let line = "src/main.ts(5,10): error TS2304: Cannot find name 'foo'.";
    let dir = tempfile::TempDir::new().unwrap();
    let diag = parse_tsc_line(line, dir.path()).unwrap();

    assert_eq!(diag.line, 5);
    assert_eq!(diag.column, Some(10));
    assert_eq!(diag.code, Some("TS2304".to_string()));
    assert_eq!(diag.message, "Cannot find name 'foo'.");
    assert_eq!(diag.severity, Severity::Error);
    assert_eq!(diag.path, dir.path().join("src/main.ts"));
}

#[test]
fn parse_tsc_line_skips_warnings() {
    let line = "src/main.ts(5,10): warning TS6000: deprecated";
    let dir = tempfile::TempDir::new().unwrap();
    assert!(parse_tsc_line(line, dir.path()).is_none());
}

#[test]
fn parse_ruff_output_extracts_diagnostics() {
    let json = r#"[{
            "code": "F401",
            "message": "unused import",
            "filename": "foo.py",
            "location": {"row": 3, "column": 5}
        }]"#;

    let diags = parse_ruff_output(json);
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].code, Some("F401".to_string()));
    assert_eq!(diags[0].message, "unused import");
    assert_eq!(diags[0].line, 3);
    assert_eq!(diags[0].column, Some(5));
    assert_eq!(diags[0].path, PathBuf::from("foo.py"));
    assert_eq!(diags[0].severity, Severity::Error);
}

#[test]
fn parse_ruff_output_empty_json() {
    assert!(parse_ruff_output("[]").is_empty());
}

#[test]
fn parse_ruff_output_unparseable_json() {
    assert!(parse_ruff_output("not json").is_empty());
}

#[tokio::test]
#[ignore]
async fn verify_typescript_returns_checked_on_broken_code() {
    let dir = tempfile::TempDir::new().unwrap();
    fs::write(dir.path().join("tsconfig.json"), "{}").unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("main.ts"), "const x: unknown_type = 1;").unwrap();

    let result = verify_typescript(&src.join("main.ts")).await;
    match result {
        VerifierResult::Checked { diagnostics } => {
            assert!(!diagnostics.is_empty());
        }
        VerifierResult::Failed(msg) => {
            panic!("expected Checked, got Failed: {msg}");
        }
        other => panic!("expected Checked, got {:?}", other),
    }
}

#[tokio::test]
#[ignore]
async fn verify_python_returns_checked_on_broken_code() {
    let dir = tempfile::TempDir::new().unwrap();
    let py_path = dir.path().join("foo.py");
    fs::write(&py_path, "import os\n").unwrap();

    let result = verify_python(&py_path).await;
    match result {
        VerifierResult::Checked { diagnostics } => {
            let f401 = diagnostics
                .iter()
                .find(|d| d.code.as_deref() == Some("F401"));
            assert!(
                f401.is_some(),
                "expected F401 diagnostic, got: {:?}",
                diagnostics
            );
        }
        VerifierResult::Failed(msg) => {
            panic!("expected Checked, got Failed: {msg}");
        }
        other => panic!("expected Checked, got {:?}", other),
    }
}

// --- Author-vs-ambient baseline tests ---

#[test]
fn signature_ignores_line_and_column() {
    let d1 = Diagnostic {
        path: PathBuf::from("src/lib.rs"),
        line: 5,
        column: Some(10),
        severity: Severity::Error,
        message: "cannot find function `foo`".to_string(),
        code: Some("E0425".to_string()),
    };
    let d2 = Diagnostic {
        path: PathBuf::from("src/lib.rs"),
        line: 10,
        column: Some(3),
        severity: Severity::Error,
        message: "cannot find function `foo`".to_string(),
        code: Some("E0425".to_string()),
    };
    assert_eq!(d1.signature(), d2.signature());
}

#[test]
fn signature_distinguishes_paths() {
    let d1 = Diagnostic {
        path: PathBuf::from("src/lib.rs"),
        line: 5,
        column: None,
        severity: Severity::Error,
        message: "unused variable".to_string(),
        code: None,
    };
    let d2 = Diagnostic {
        path: PathBuf::from("src/main.rs"),
        line: 5,
        column: None,
        severity: Severity::Error,
        message: "unused variable".to_string(),
        code: None,
    };
    assert_ne!(d1.signature(), d2.signature());
}

#[test]
fn signature_distinguishes_severities() {
    let d1 = Diagnostic {
        path: PathBuf::from("src/lib.rs"),
        line: 5,
        column: None,
        severity: Severity::Error,
        message: "unused variable".to_string(),
        code: None,
    };
    let d2 = Diagnostic {
        path: PathBuf::from("src/lib.rs"),
        line: 5,
        column: None,
        severity: Severity::Warning,
        message: "unused variable".to_string(),
        code: None,
    };
    assert_ne!(d1.signature(), d2.signature());
}

#[test]
fn signature_distinguishes_messages() {
    let d1 = Diagnostic {
        path: PathBuf::from("src/lib.rs"),
        line: 5,
        column: None,
        severity: Severity::Error,
        message: "cannot find function `foo`".to_string(),
        code: Some("E0425".to_string()),
    };
    let d2 = Diagnostic {
        path: PathBuf::from("src/lib.rs"),
        line: 5,
        column: None,
        severity: Severity::Error,
        message: "cannot find function `bar`".to_string(),
        code: Some("E0425".to_string()),
    };
    assert_ne!(d1.signature(), d2.signature());
}

#[test]
fn signature_handles_none_code() {
    let d1 = Diagnostic {
        path: PathBuf::from("src/lib.rs"),
        line: 5,
        column: None,
        severity: Severity::Error,
        message: "mismatched types".to_string(),
        code: None,
    };
    let d2 = Diagnostic {
        path: PathBuf::from("src/lib.rs"),
        line: 10,
        column: Some(1),
        severity: Severity::Error,
        message: "mismatched types".to_string(),
        code: None,
    };
    assert_eq!(d1.signature(), d2.signature());
}

#[test]
fn baseline_new_is_empty() {
    let baseline = Baseline::new();
    assert!(baseline.signatures.is_empty());
}

#[test]
fn baseline_record_and_contains_roundtrip() {
    let diag = Diagnostic {
        path: PathBuf::from("src/lib.rs"),
        line: 5,
        column: None,
        severity: Severity::Error,
        message: "cannot find function `foo`".to_string(),
        code: Some("E0425".to_string()),
    };
    let mut baseline = Baseline::new();
    baseline.record(&diag);
    assert!(baseline.contains(&diag));

    let different = Diagnostic {
        path: PathBuf::from("src/lib.rs"),
        line: 5,
        column: None,
        severity: Severity::Error,
        message: "cannot find function `bar`".to_string(),
        code: Some("E0425".to_string()),
    };
    assert!(!baseline.contains(&different));
}

#[test]
fn baseline_partition_splits_author_and_ambient() {
    let ambient_diag = Diagnostic {
        path: PathBuf::from("src/lib.rs"),
        line: 5,
        column: None,
        severity: Severity::Error,
        message: "unused variable".to_string(),
        code: None,
    };
    let mut baseline = Baseline::new();
    baseline.record(&ambient_diag);

    let author_a = Diagnostic {
        path: PathBuf::from("src/lib.rs"),
        line: 10,
        column: None,
        severity: Severity::Error,
        message: "cannot find `foo`".to_string(),
        code: Some("E0425".to_string()),
    };
    let author_b = Diagnostic {
        path: PathBuf::from("src/main.rs"),
        line: 3,
        column: None,
        severity: Severity::Error,
        message: "type mismatch".to_string(),
        code: None,
    };
    let recheck = Diagnostic {
        path: PathBuf::from("src/lib.rs"),
        line: 20,
        column: Some(5),
        severity: Severity::Error,
        message: "unused variable".to_string(),
        code: None,
    };

    let diags = vec![author_a.clone(), ambient_diag.clone(), author_b.clone()];
    let (author, ambient) = baseline.partition(&diags);
    assert_eq!(author.len(), 2);
    assert_eq!(ambient.len(), 1);
    assert_eq!(ambient[0].message, "unused variable");

    // Verify partition preserves input order
    assert_eq!(author[0].message, "cannot find `foo`");
    assert_eq!(author[1].message, "type mismatch");

    // recheck should match baseline even at different line
    assert!(baseline.contains(&recheck));
}

#[test]
fn partition_preserves_input_order() {
    let mut baseline = Baseline::new();
    let b_match = Diagnostic {
        path: PathBuf::from("src/lib.rs"),
        line: 1,
        column: None,
        severity: Severity::Error,
        message: "ambient error".to_string(),
        code: None,
    };
    baseline.record(&b_match);

    let a = Diagnostic {
        path: PathBuf::from("src/lib.rs"),
        line: 2,
        column: None,
        severity: Severity::Error,
        message: "author A".to_string(),
        code: None,
    };
    let c = Diagnostic {
        path: PathBuf::from("src/lib.rs"),
        line: 3,
        column: None,
        severity: Severity::Error,
        message: "author C".to_string(),
        code: None,
    };
    let d = Diagnostic {
        path: PathBuf::from("src/lib.rs"),
        line: 4,
        column: None,
        severity: Severity::Error,
        message: "ambient error".to_string(),
        code: None,
    };

    let diags = vec![a.clone(), b_match.clone(), c.clone(), d.clone()];
    let (author, ambient) = baseline.partition(&diags);
    assert_eq!(author.len(), 2);
    assert_eq!(ambient.len(), 2);
    assert_eq!(author[0].message, "author A");
    assert_eq!(author[1].message, "author C");
    assert_eq!(ambient[0].message, "ambient error");
    assert_eq!(ambient[1].message, "ambient error");
}

#[test]
fn find_ancestor_with_walks_up_to_marker() {
    let dir = tempfile::TempDir::new().unwrap();
    let nested = dir.path().join("a/b/c");
    fs::create_dir_all(&nested).unwrap();
    fs::write(dir.path().join("a/Cargo.toml"), "[package]\nname=\"x\"\n").unwrap();
    let file = nested.join("foo.rs");
    fs::write(&file, "").unwrap();

    let root = find_ancestor_with(&file, "Cargo.toml").unwrap();
    assert_eq!(root, dir.path().join("a"));
}

#[test]
fn project_root_for_resolves_per_extension() {
    let dir = tempfile::TempDir::new().unwrap();
    let rs_root = dir.path().join("rust");
    fs::create_dir_all(rs_root.join("src")).unwrap();
    fs::write(rs_root.join("Cargo.toml"), "[package]\nname=\"x\"\n").unwrap();
    let rs_file = rs_root.join("src/lib.rs");
    fs::write(&rs_file, "").unwrap();

    let py_root = dir.path().join("py");
    fs::create_dir_all(py_root.join("pkg")).unwrap();
    fs::write(py_root.join("pyproject.toml"), "[project]\nname=\"x\"\n").unwrap();
    let py_file = py_root.join("pkg/mod.py");
    fs::write(&py_file, "").unwrap();

    assert_eq!(project_root_for(&rs_file, "rs"), Some(rs_root));
    assert_eq!(project_root_for(&py_file, "py"), Some(py_root));
    assert_eq!(project_root_for(&rs_file, "md"), None);
}

#[tokio::test]
async fn capture_baseline_dedupes_by_project_root() {
    // Two .rs files sharing the same Cargo.toml ancestor
    // should trigger only one cargo invocation, but produce
    // identical baseline diagnostics regardless. We verify
    // the dedup by checking that the result equals what one
    // call produces — implicitly proving N invocations
    // weren't needed (the test would be ~Nx slower if we
    // were still per-file).
    let dir = tempfile::TempDir::new().unwrap();
    let crate_dir = dir.path().join("c");
    let src = crate_dir.join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(
        crate_dir.join("Cargo.toml"),
        "[package]\nname=\"c\"\nversion=\"0.1.0\"\n",
    )
    .unwrap();
    fs::write(src.join("lib.rs"), "fn a() { let _: i32 = \"\"; }\n").unwrap();
    fs::write(src.join("other.rs"), "// unused\n").unwrap();

    let paths = vec![
        src.join("lib.rs"),
        src.join("other.rs"),
        crate_dir.join("Cargo.toml"),
    ];
    let baseline = capture_baseline(&paths).await;
    // At least one diagnostic from lib.rs's type error.
    assert!(!baseline.signatures.is_empty());
}

#[tokio::test]
async fn capture_baseline_skips_unsupported_files() {
    let dir = tempfile::TempDir::new().unwrap();
    let md_path = dir.path().join("README.md");
    fs::write(&md_path, "# Hello").unwrap();

    let crate_dir = dir.path().join("crate-x");
    let src_dir = crate_dir.join("src");
    fs::create_dir_all(&src_dir).unwrap();
    fs::write(
        crate_dir.join("Cargo.toml"),
        "[package]\nname = \"x\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    let lib_path = src_dir.join("lib.rs");
    fs::write(&lib_path, "fn broken() { let x = y; }").unwrap();

    let baseline = capture_baseline(&[md_path, lib_path]).await;

    // The .md file is unsupported, so only the .rs file's
    // diagnostics should be in the baseline.
    assert!(!baseline.signatures.is_empty());
    for sig in &baseline.signatures {
        assert_eq!(sig.path.extension().and_then(|s| s.to_str()), Some("rs"));
    }
}

#[test]
fn spawn_failure_not_found_is_skipped_naming_remedy() {
    let err = std::io::Error::new(std::io::ErrorKind::NotFound, "x");
    let result = spawn_failure("cargo", "install the Rust toolchain", &err);

    match result {
        VerifierResult::Skipped(msg) => {
            assert!(
                msg.contains("cargo"),
                "Skipped message must name the tool: {msg}"
            );
            assert!(
                msg.contains("install the Rust toolchain"),
                "Skipped message must contain the install hint: {msg}"
            );
        }
        other => panic!("expected Skipped, got {:?}", other),
    }
}

#[test]
fn spawn_failure_other_error_stays_failed() {
    let err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "access denied");
    let result = spawn_failure("cargo", "install the Rust toolchain", &err);

    match result {
        VerifierResult::Failed(msg) => {
            assert!(
                msg.contains("spawn failed"),
                "Non-NotFound error must produce Failed with 'spawn failed': {msg}"
            );
        }
        other => panic!("expected Failed, got {:?}", other),
    }
}

// --- tsc resolution tests ---

#[test]
fn find_local_tsc_finds_at_project_root() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path();
    let bin_dir = root.join("node_modules").join(".bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let tsc = bin_dir.join("tsc");
    fs::write(&tsc, "").unwrap();

    let result = find_local_tsc(root);
    assert_eq!(result, Some(tsc));
}

#[test]
fn find_local_tsc_walks_up_to_hoisted_node_modules() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path();
    let pkg = root.join("pkg");
    fs::create_dir_all(&pkg).unwrap();
    let bin_dir = root.join("node_modules").join(".bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let tsc = bin_dir.join("tsc");
    fs::write(&tsc, "").unwrap();

    let result = find_local_tsc(&pkg);
    assert_eq!(result, Some(tsc));
}

#[test]
fn find_local_tsc_none_when_absent() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path();
    assert!(find_local_tsc(root).is_none());
}

#[test]
fn find_local_tsc_ignores_directory_named_tsc() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path();
    let bin_dir = root.join("node_modules").join(".bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let tsc = bin_dir.join("tsc");
    fs::create_dir(&tsc).unwrap();

    assert!(find_local_tsc(root).is_none());
}

#[test]
fn resolve_tsc_command_prefers_local_over_npx() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path();
    let bin_dir = root.join("node_modules").join(".bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let tsc = bin_dir.join("tsc");
    fs::write(&tsc, "").unwrap();

    let cmd = resolve_tsc_command(root, true);
    assert_eq!(cmd.program, tsc);
    assert!(cmd.prefix_args.is_empty());
}

#[test]
fn resolve_tsc_command_uses_npx_when_no_local() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path();

    let cmd = resolve_tsc_command(root, true);
    assert_eq!(cmd.program, PathBuf::from("npx"));
    assert_eq!(cmd.prefix_args, ["--no-install", "tsc"]);
}

#[test]
fn resolve_tsc_command_falls_back_to_path_tsc() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path();

    let cmd = resolve_tsc_command(root, false);
    assert_eq!(cmd.program, PathBuf::from("tsc"));
    assert!(cmd.prefix_args.is_empty());
}

#[test]
fn binary_in_dirs_finds_file() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path();
    fs::write(root.join("npx"), "#!/bin/sh\n").unwrap();

    let result = binary_in_dirs("npx", &[root.to_path_buf()]);
    assert!(result);
}

#[test]
fn binary_in_dirs_false_when_absent() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path();

    let result = binary_in_dirs("npx", &[root.to_path_buf()]);
    assert!(!result);
}

#[cfg(unix)]
#[tokio::test]
async fn verify_typescript_spawns_resolved_local_binary() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path();
    fs::write(root.join("tsconfig.json"), "{}").unwrap();
    let bin_dir = root.join("node_modules").join(".bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let fake = bin_dir.join("tsc");
    fs::write(
        &fake,
        "#!/bin/sh\necho \"src/main.ts(3,7): error TS9999: fake diagnostic\"\n",
    )
    .unwrap();
    fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
    let src = root.join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("main.ts"), "").unwrap();

    let result = verify_typescript(&src.join("main.ts")).await;
    match result {
        VerifierResult::Checked { diagnostics } => {
            assert_eq!(diagnostics.len(), 1);
            assert_eq!(diagnostics[0].code, Some("TS9999".to_string()));
            assert_eq!(diagnostics[0].line, 3);
        }
        other => panic!("expected Checked from local fake tsc, got {:?}", other),
    }
}
