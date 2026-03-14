//! Git worktree lifecycle management for task agent file isolation.
//!
//! Each task agent operates in its own git worktree, providing real filesystem
//! isolation. Worktrees are created on agent spawn and persist for review on
//! successful completion.

use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Directory name under the project root for all agent worktrees.
const WORKTREE_DIR: &str = ".worktrees";

/// Create a git worktree for a task agent. Returns the worktree path.
///
/// Creates branch `task-{work_item_id}` from current HEAD. If a stale worktree
/// or branch exists from a previous failed attempt, removes it first.
pub async fn create_worktree(project_root: &Path, work_item_id: Uuid) -> anyhow::Result<PathBuf> {
    let worktree_path = project_root
        .join(WORKTREE_DIR)
        .join(format!("task-{work_item_id}"));
    let branch_name = format!("task-{work_item_id}");

    // Ensure the .worktrees directory exists.
    tokio::fs::create_dir_all(project_root.join(WORKTREE_DIR)).await?;

    // If a stale worktree exists from a previous attempt, clean it up.
    if worktree_path.exists() {
        remove_worktree(&worktree_path).await?;
    }

    // Delete the branch if it exists (leftover from a previous worktree).
    let _ = tokio::process::Command::new("git")
        .args(["branch", "-D", &branch_name])
        .current_dir(project_root)
        .output()
        .await;

    // Create the worktree with a new branch from current HEAD.
    let output = tokio::process::Command::new("git")
        .args([
            "worktree",
            "add",
            worktree_path.to_str().unwrap_or_default(),
            "-b",
            &branch_name,
        ])
        .current_dir(project_root)
        .output()
        .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git worktree add failed: {stderr}");
    }

    Ok(worktree_path)
}

/// Remove a git worktree and its associated branch.
pub async fn remove_worktree(worktree_path: &Path) -> anyhow::Result<()> {
    let output = tokio::process::Command::new("git")
        .args([
            "worktree",
            "remove",
            "--force",
            worktree_path.to_str().unwrap_or_default(),
        ])
        .output()
        .await?;

    if !output.status.success() {
        // Fallback: if git worktree remove fails, try deleting the directory.
        if worktree_path.exists() {
            tokio::fs::remove_dir_all(worktree_path).await?;
        }
    }

    Ok(())
}

/// Clean up stale worktrees from previous sessions. Returns paths of removed
/// worktrees. Run during `App::startup()`.
///
/// Enumerates `git worktree list --porcelain`, identifies worktrees under
/// `.worktrees/task-*` that exist on disk, and removes them.
pub async fn cleanup_stale_worktrees(project_root: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let worktree_dir = project_root.join(WORKTREE_DIR);
    if !worktree_dir.exists() {
        return Ok(Vec::new());
    }

    let mut removed = Vec::new();
    let mut entries = tokio::fs::read_dir(&worktree_dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.starts_with("task-") {
                    tracing::info!("Cleaning up stale worktree: {}", path.display());
                    let _ = remove_worktree(&path).await;
                    removed.push(path);
                }
            }
        }
    }

    Ok(removed)
}
