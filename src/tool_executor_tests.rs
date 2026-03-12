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
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hello.txt");
    std::fs::write(&path, "hello world").unwrap();
    let args = ToolCallArguments::ReadFile {
        path: path.to_str().unwrap().to_string(),
    };
    let result = execute_tool(&args).await;
    assert!(!result.is_error);
    assert_eq!(result.content, "hello world");
}

#[tokio::test]
async fn test_read_file_nonexistent_returns_error() {
    let args = ToolCallArguments::ReadFile {
        path: "/tmp/nonexistent_file_abc123xyz".to_string(),
    };
    let result = execute_tool(&args).await;
    assert!(result.is_error);
    assert!(result.content.contains("Error reading file"));
}

#[tokio::test]
async fn test_read_file_truncates_large_files() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("large.txt");
    let content = "a".repeat(150_000);
    std::fs::write(&path, &content).unwrap();
    let args = ToolCallArguments::ReadFile {
        path: path.to_str().unwrap().to_string(),
    };
    let result = execute_tool(&args).await;
    assert!(!result.is_error);
    assert!(result.content.contains("[truncated, 150000 bytes total]"));
    assert!(result.content.len() < 150_000);
}

#[tokio::test]
async fn test_spawn_execution_sends_completion() {
    let dir = tempfile::tempdir().unwrap();
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
            assert_eq!(content, "spawn content");
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
    assert!(result.content.contains("nonexistent"));
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
