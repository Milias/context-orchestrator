//! Path validation for file operations with configurable root directory.
//!
//! All file tools resolve paths relative to a root directory (process CWD by
//! default, or a git worktree path for task agents). Paths that escape the root
//! via `..` traversal or symlinks are rejected.

use super::ToolExecutionResult;
use crate::graph::tool_types::ToolResultContent;
use std::path::{Path, PathBuf};

/// A validated, canonical path safe for file operations.
pub struct ValidatedPath {
    pub canonical: PathBuf,
}

/// Resolve the root directory for file operations. Returns `working_dir` if
/// provided, otherwise falls back to the process working directory.
async fn resolve_root(working_dir: Option<&Path>) -> Result<PathBuf, ToolExecutionResult> {
    if let Some(dir) = working_dir {
        tokio::fs::canonicalize(dir)
            .await
            .map_err(|e| ToolExecutionResult {
                content: ToolResultContent::text(format!("Error resolving working directory: {e}")),
                is_error: true,
            })
    } else {
        let cwd = std::env::current_dir().map_err(|_| ToolExecutionResult {
            content: ToolResultContent::text("Error: could not determine working directory"),
            is_error: true,
        })?;
        tokio::fs::canonicalize(&cwd)
            .await
            .map_err(|_| ToolExecutionResult {
                content: ToolResultContent::text("Error: could not resolve working directory"),
                is_error: true,
            })
    }
}

/// Validate a path for read operations. The file must exist and be within the
/// root directory.
pub async fn validate_path(
    path: &str,
    working_dir: Option<&Path>,
) -> Result<ValidatedPath, ToolExecutionResult> {
    let canonical_root = resolve_root(working_dir).await?;
    let root = working_dir.map_or_else(
        || std::env::current_dir().unwrap_or_default(),
        PathBuf::from,
    );
    let requested = if std::path::Path::new(path).is_absolute() {
        PathBuf::from(path)
    } else {
        root.join(path)
    };
    let canonical = tokio::fs::canonicalize(&requested)
        .await
        .map_err(|e| ToolExecutionResult {
            content: ToolResultContent::text(format!("Error resolving path: {e}")),
            is_error: true,
        })?;
    if !canonical.starts_with(&canonical_root) {
        return Err(ToolExecutionResult {
            content: ToolResultContent::text(format!(
                "Error: path escapes working directory: {path}"
            )),
            is_error: true,
        });
    }
    Ok(ValidatedPath { canonical })
}

/// Validate a path for write operations. The parent directory must exist within
/// the root directory; the file itself may not exist yet.
///
/// Walks up to the first existing ancestor to validate containment BEFORE
/// creating any directories. Prevents `create_dir_all` from creating
/// directories outside the root via `..` traversal or symlinks.
pub async fn validate_path_for_write(
    path: &str,
    working_dir: Option<&Path>,
) -> Result<ValidatedPath, ToolExecutionResult> {
    let canonical_root = resolve_root(working_dir).await?;
    let root = working_dir.map_or_else(
        || std::env::current_dir().unwrap_or_default(),
        PathBuf::from,
    );
    let requested = if std::path::Path::new(path).is_absolute() {
        PathBuf::from(path)
    } else {
        root.join(path)
    };
    let parent = requested.parent().ok_or_else(|| ToolExecutionResult {
        content: ToolResultContent::text(format!("Error: no parent directory for path: {path}")),
        is_error: true,
    })?;
    // Find the first existing ancestor so we can canonicalize and check
    // containment before creating anything on disk.
    let mut ancestor = parent.to_path_buf();
    while !ancestor.exists() {
        ancestor = ancestor
            .parent()
            .ok_or_else(|| ToolExecutionResult {
                content: ToolResultContent::text(format!(
                    "Error: no existing ancestor for path: {path}"
                )),
                is_error: true,
            })?
            .to_path_buf();
    }
    let canonical_ancestor =
        tokio::fs::canonicalize(&ancestor)
            .await
            .map_err(|e| ToolExecutionResult {
                content: ToolResultContent::text(format!("Error resolving path: {e}")),
                is_error: true,
            })?;
    if !canonical_ancestor.starts_with(&canonical_root) {
        return Err(ToolExecutionResult {
            content: ToolResultContent::text(format!(
                "Error: path escapes working directory: {path}"
            )),
            is_error: true,
        });
    }
    // Safe to create directories — the ancestor is verified within root.
    tokio::fs::create_dir_all(parent)
        .await
        .map_err(|e| ToolExecutionResult {
            content: ToolResultContent::text(format!("Error creating directories: {e}")),
            is_error: true,
        })?;
    let canonical_parent =
        tokio::fs::canonicalize(parent)
            .await
            .map_err(|e| ToolExecutionResult {
                content: ToolResultContent::text(format!("Error resolving parent directory: {e}")),
                is_error: true,
            })?;
    // Re-check after creation: a symlink within the newly created subtree
    // could redirect outside the root (TOCTOU defense).
    if !canonical_parent.starts_with(&canonical_root) {
        return Err(ToolExecutionResult {
            content: ToolResultContent::text(format!(
                "Error: path escapes working directory: {path}"
            )),
            is_error: true,
        });
    }
    let file_name = requested.file_name().ok_or_else(|| ToolExecutionResult {
        content: ToolResultContent::text(format!("Error: no file name in path: {path}")),
        is_error: true,
    })?;
    Ok(ValidatedPath {
        canonical: canonical_parent.join(file_name),
    })
}
