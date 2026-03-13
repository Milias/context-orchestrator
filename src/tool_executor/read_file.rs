use super::security;
use super::ToolExecutionResult;
use crate::graph::tool_types::ToolResultContent;

const MAX_READ_FILE_BYTES: usize = 100_000;
const MAX_FILE_SIZE_BYTES: u64 = 50_000_000; // 50 MB — prevent OOM on huge files

pub async fn execute(path: &str) -> ToolExecutionResult {
    let validated = match security::validate_path(path).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    // Check file size before reading to prevent OOM on multi-GB files.
    if let Ok(meta) = tokio::fs::symlink_metadata(&validated.canonical).await {
        if meta.len() > MAX_FILE_SIZE_BYTES {
            return ToolExecutionResult {
                content: ToolResultContent::text(format!(
                    "Error: file too large ({} bytes, limit {MAX_FILE_SIZE_BYTES})",
                    meta.len()
                )),
                is_error: true,
            };
        }
    }
    match tokio::fs::read_to_string(&validated.canonical).await {
        Ok(contents) => {
            if contents.len() > MAX_READ_FILE_BYTES {
                let mut boundary = MAX_READ_FILE_BYTES;
                while boundary > 0 && !contents.is_char_boundary(boundary) {
                    boundary -= 1;
                }
                ToolExecutionResult {
                    content: ToolResultContent::text(format!(
                        "{}\n\n[truncated, {} bytes total]",
                        &contents[..boundary],
                        contents.len()
                    )),
                    is_error: false,
                }
            } else {
                ToolExecutionResult {
                    content: ToolResultContent::text(contents),
                    is_error: false,
                }
            }
        }
        Err(e) => ToolExecutionResult {
            content: ToolResultContent::text(format!("Error reading file: {e}")),
            is_error: true,
        },
    }
}
