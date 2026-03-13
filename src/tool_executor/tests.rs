use super::*;
use crate::graph::tool_types::ToolCallArguments;

#[test]
fn test_registered_tools_includes_read_file() {
    let defs = registered_tool_definitions();
    let read_file = defs.iter().find(|d| d.name == "read_file");
    assert!(read_file.is_some(), "read_file tool must be registered");
    let rf = read_file.unwrap();
    assert!(!rf.description.is_empty());
    assert!(!rf.input_schema.properties.is_empty());
}

#[tokio::test]
async fn test_read_file_returns_contents() {
    let dir = tempfile::TempDir::new_in(".").unwrap();
    let path = dir.path().join("hello.txt");
    std::fs::write(&path, "hello world").unwrap();
    let args = ToolCallArguments::ReadFile {
        path: path.to_str().unwrap().to_string(),
    };
    let result = execute_tool(&args).await;
    assert!(!result.is_error);
    assert_eq!(result.content.text_content(), "hello world");
}

#[tokio::test]
async fn test_read_file_nonexistent_returns_error() {
    let args = ToolCallArguments::ReadFile {
        path: "nonexistent_file_abc123xyz.txt".to_string(),
    };
    let result = execute_tool(&args).await;
    assert!(result.is_error);
    assert!(result.content.text_content().contains("Error"));
}

#[tokio::test]
async fn test_read_file_rejects_path_outside_cwd() {
    let args = ToolCallArguments::ReadFile {
        path: "/etc/passwd".to_string(),
    };
    let result = execute_tool(&args).await;
    assert!(result.is_error);
    assert!(result
        .content
        .text_content()
        .contains("escapes working directory"));
}

#[tokio::test]
async fn test_read_file_truncates_large_files() {
    let dir = tempfile::TempDir::new_in(".").unwrap();
    let path = dir.path().join("large.txt");
    let content = "a".repeat(150_000);
    std::fs::write(&path, &content).unwrap();
    let args = ToolCallArguments::ReadFile {
        path: path.to_str().unwrap().to_string(),
    };
    let result = execute_tool(&args).await;
    assert!(!result.is_error);
    assert!(result
        .content
        .text_content()
        .contains("[truncated, 150000 bytes total]"));
    assert!(result.content.char_len() < 150_000);
}

#[tokio::test]
async fn test_spawn_execution_sends_completion() {
    let dir = tempfile::TempDir::new_in(".").unwrap();
    let path = dir.path().join("spawn_test.txt");
    std::fs::write(&path, "spawn content").unwrap();

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let tc_id = uuid::Uuid::new_v4();
    let args = ToolCallArguments::ReadFile {
        path: path.to_str().unwrap().to_string(),
    };
    spawn_tool_execution(tc_id, args, tx);

    let msg = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
        .await
        .expect("timed out waiting for completion")
        .expect("channel closed");
    match msg {
        crate::tasks::TaskMessage::ToolCallCompleted {
            tool_call_id,
            content,
            is_error,
        } => {
            assert_eq!(tool_call_id, tc_id);
            assert!(!is_error);
            assert_eq!(content.text_content(), "spawn content");
        }
        other => panic!("Expected ToolCallCompleted, got: {other:?}"),
    }
}

/// Catches stub silently succeeding on unimplemented tools.
/// Unknown tools must always return `is_error=true`.
#[tokio::test]
async fn test_execute_unknown_tool_returns_error() {
    let args = ToolCallArguments::Unknown {
        tool_name: "nonexistent".to_string(),
        raw_json: "{}".to_string(),
    };
    let result = execute_tool(&args).await;
    assert!(result.is_error);
    assert!(result.content.text_content().contains("nonexistent"));
}

// ── write_file tests ──────────────────────────────────────────────

/// Catches basic write failure (file not created or wrong content).
#[tokio::test]
async fn test_write_file_creates_and_writes() {
    let dir = tempfile::TempDir::new_in(".").unwrap();
    let path = dir.path().join("output.txt");
    let args = ToolCallArguments::WriteFile {
        path: path.to_str().unwrap().to_string(),
        content: "hello from write_file".to_string(),
    };
    let result = execute_tool(&args).await;
    assert!(!result.is_error);
    assert!(result.content.text_content().contains("Wrote"));
    let written = std::fs::read_to_string(&path).unwrap();
    assert_eq!(written, "hello from write_file");
}

/// Catches missing `create_dir_all` — write to nested path must work.
#[tokio::test]
async fn test_write_file_creates_parent_dirs() {
    let dir = tempfile::TempDir::new_in(".").unwrap();
    let path = dir.path().join("a").join("b").join("deep.txt");
    let args = ToolCallArguments::WriteFile {
        path: path.to_str().unwrap().to_string(),
        content: "nested".to_string(),
    };
    let result = execute_tool(&args).await;
    assert!(!result.is_error);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "nested");
}

/// Catches security bypass — writing outside CWD must fail.
#[tokio::test]
async fn test_write_file_rejects_escape() {
    let args = ToolCallArguments::WriteFile {
        path: "/tmp/evil_write_test.txt".to_string(),
        content: "pwned".to_string(),
    };
    let result = execute_tool(&args).await;
    assert!(result.is_error);
    assert!(result
        .content
        .text_content()
        .contains("escapes working directory"));
}

/// Catches error-if-exists behavior — second write must overwrite.
#[tokio::test]
async fn test_write_file_overwrites_existing() {
    let dir = tempfile::TempDir::new_in(".").unwrap();
    let path = dir.path().join("overwrite.txt");
    std::fs::write(&path, "original").unwrap();
    let args = ToolCallArguments::WriteFile {
        path: path.to_str().unwrap().to_string(),
        content: "replaced".to_string(),
    };
    let result = execute_tool(&args).await;
    assert!(!result.is_error);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "replaced");
}

/// Catches missing size limit — oversized writes must be rejected.
#[tokio::test]
async fn test_write_file_rejects_oversized() {
    let dir = tempfile::TempDir::new_in(".").unwrap();
    let path = dir.path().join("big.txt");
    let args = ToolCallArguments::WriteFile {
        path: path.to_str().unwrap().to_string(),
        content: "x".repeat(600_000),
    };
    let result = execute_tool(&args).await;
    assert!(result.is_error);
    assert!(result.content.text_content().contains("exceeds maximum"));
}

// ── list_directory tests ──────────────────────────────────────────

/// Catches missing type differentiation between files and directories.
#[tokio::test]
async fn test_list_directory_shows_files_and_dirs() {
    let dir = tempfile::TempDir::new_in(".").unwrap();
    std::fs::write(dir.path().join("file.txt"), "content").unwrap();
    std::fs::create_dir(dir.path().join("subdir")).unwrap();
    let args = ToolCallArguments::ListDirectory {
        path: dir.path().to_str().unwrap().to_string(),
        recursive: None,
    };
    let result = execute_tool(&args).await;
    assert!(!result.is_error);
    let text = result.content.text_content();
    assert!(
        text.contains("dir/  subdir/"),
        "should list directory: {text}"
    );
    assert!(text.contains("file  file.txt"), "should list file: {text}");
}

/// Catches security bypass — listing outside CWD must fail.
#[tokio::test]
async fn test_list_directory_rejects_escape() {
    let args = ToolCallArguments::ListDirectory {
        path: "/etc".to_string(),
        recursive: None,
    };
    let result = execute_tool(&args).await;
    assert!(result.is_error);
    assert!(result
        .content
        .text_content()
        .contains("escapes working directory"));
}

/// Catches non-recursive behavior when recursive=true.
#[tokio::test]
async fn test_list_directory_recursive_descends() {
    let dir = tempfile::TempDir::new_in(".").unwrap();
    let nested = dir.path().join("a").join("b");
    std::fs::create_dir_all(&nested).unwrap();
    std::fs::write(nested.join("deep.txt"), "deep").unwrap();
    let args = ToolCallArguments::ListDirectory {
        path: dir.path().to_str().unwrap().to_string(),
        recursive: Some(true),
    };
    let result = execute_tool(&args).await;
    assert!(!result.is_error);
    let text = result.content.text_content();
    assert!(
        text.contains("deep.txt"),
        "recursive should find nested file: {text}"
    );
}

// ── search_files tests ────────────────────────────────────────────

/// Catches basic search failure or wrong result format.
#[tokio::test]
async fn test_search_files_finds_matches() {
    let dir = tempfile::TempDir::new_in(".").unwrap();
    std::fs::write(dir.path().join("a.rs"), "fn main() {}\nfn helper() {}").unwrap();
    std::fs::write(dir.path().join("b.txt"), "no match here").unwrap();
    let args = ToolCallArguments::SearchFiles {
        pattern: "fn main".to_string(),
        path: Some(dir.path().to_str().unwrap().to_string()),
    };
    let result = execute_tool(&args).await;
    assert!(!result.is_error);
    let text = result.content.text_content();
    assert!(text.contains("a.rs:1:"), "should show file:line: {text}");
    assert!(
        text.contains("fn main"),
        "should show matching text: {text}"
    );
    assert!(
        !text.contains("b.txt"),
        "should not match non-matching file"
    );
}

/// Catches panic or unhelpful error on invalid regex.
#[tokio::test]
async fn test_search_files_invalid_regex_returns_error() {
    let args = ToolCallArguments::SearchFiles {
        pattern: "[invalid".to_string(),
        path: None,
    };
    let result = execute_tool(&args).await;
    assert!(result.is_error);
    assert!(result.content.text_content().contains("Invalid regex"));
}

/// Catches binary file inclusion in search results.
#[tokio::test]
async fn test_search_files_skips_binary() {
    let dir = tempfile::TempDir::new_in(".").unwrap();
    let mut binary_content = b"fn main".to_vec();
    binary_content.push(0); // null byte makes it binary
    binary_content.extend_from_slice(b" more content");
    std::fs::write(dir.path().join("binary.bin"), &binary_content).unwrap();
    std::fs::write(dir.path().join("text.rs"), "fn main() {}").unwrap();
    let args = ToolCallArguments::SearchFiles {
        pattern: "fn main".to_string(),
        path: Some(dir.path().to_str().unwrap().to_string()),
    };
    let result = execute_tool(&args).await;
    assert!(!result.is_error);
    let text = result.content.text_content();
    assert!(text.contains("text.rs"), "should find text file: {text}");
    assert!(
        !text.contains("binary.bin"),
        "should skip binary file: {text}"
    );
}

/// Catches missing truncation when too many results.
#[tokio::test]
async fn test_search_files_respects_max_results() {
    let dir = tempfile::TempDir::new_in(".").unwrap();
    // Create a file with many matching lines
    let mut content = String::new();
    for i in 0..100 {
        use std::fmt::Write;
        let _ = writeln!(content, "match_line_{i}");
    }
    std::fs::write(dir.path().join("many.txt"), &content).unwrap();
    let args = ToolCallArguments::SearchFiles {
        pattern: "match_line".to_string(),
        path: Some(dir.path().to_str().unwrap().to_string()),
    };
    let result = execute_tool(&args).await;
    assert!(!result.is_error);
    let text = result.content.text_content();
    assert!(
        text.contains("search stopped"),
        "should indicate truncation: {text}"
    );
}

/// Catches security bypass — searching outside CWD must fail.
#[tokio::test]
async fn test_search_files_rejects_escape() {
    let args = ToolCallArguments::SearchFiles {
        pattern: "root".to_string(),
        path: Some("/etc".to_string()),
    };
    let result = execute_tool(&args).await;
    assert!(result.is_error);
    assert!(result
        .content
        .text_content()
        .contains("escapes working directory"));
}

/// Catches plan execution stub returning success before implementation.
/// Until the plan executor is built, it must return `is_error=true`.
#[tokio::test]
async fn test_execute_plan_stub_returns_error() {
    let args = ToolCallArguments::Plan {
        raw_input: "fix the login".to_string(),
        description: None,
    };
    let result = execute_tool(&args).await;
    assert!(result.is_error);
}
