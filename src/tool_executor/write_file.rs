use super::security;
use super::ToolExecutionResult;
use crate::graph::tool_types::ToolResultContent;

const MAX_WRITE_FILE_BYTES: usize = 500_000;

pub async fn execute(path: &str, content: &str, working_dir: Option<&std::path::Path>) -> ToolExecutionResult {
    if content.len() > MAX_WRITE_FILE_BYTES {
        return ToolExecutionResult {
            content: ToolResultContent::text(format!(
                "Error: content exceeds maximum write size ({} bytes, limit {MAX_WRITE_FILE_BYTES})",
                content.len()
            )),
            is_error: true,
        };
    }
    let validated = match security::validate_path_for_write(path, working_dir).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    match tokio::fs::write(&validated.canonical, content).await {
        Ok(()) => ToolExecutionResult {
            content: ToolResultContent::text(format!("Wrote {} bytes to {path}", content.len())),
            is_error: false,
        },
        Err(e) => ToolExecutionResult {
            content: ToolResultContent::text(format!("Error writing file: {e}")),
            is_error: true,
        },
    }
}
