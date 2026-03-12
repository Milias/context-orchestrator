use super::*;
use crate::graph::tool_types::ToolCallArguments;

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
