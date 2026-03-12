use crate::graph::tool_types::ToolCallArguments;
use crate::graph::{BackgroundTaskKind, GitFileStatus, TaskStatus};
use crate::llm::ChatMessage;
use crate::tools::PlanExtractionResult;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitFileSnapshot {
    pub path: String,
    pub status: GitFileStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSnapshot {
    pub name: String,
    pub description: String,
}

/// Read-only snapshot of graph context for background tasks.
/// Cloned from the live graph before spawning — no shared mutable state.
#[derive(Debug, Clone)]
pub struct ContextSnapshot {
    pub messages: Vec<ChatMessage>,
    pub tools: Vec<ToolSnapshot>,
    pub trigger_message_id: Uuid,
}

/// Outcome of a tool extraction background task.
#[derive(Debug)]
pub enum ToolExtractionOutcome {
    Plan(PlanExtractionResult),
}

#[derive(Debug)]
pub enum TaskMessage {
    GitFilesUpdated(Vec<GitFileSnapshot>),
    ToolsDiscovered(Vec<ToolSnapshot>),
    TaskStatusChanged {
        task_id: Uuid,
        kind: BackgroundTaskKind,
        status: TaskStatus,
        description: String,
    },
    ToolExtractionComplete {
        trigger_message_id: Uuid,
        result: ToolExtractionOutcome,
    },
    ToolCallDispatched {
        tool_call_id: Uuid,
        parent_message_id: Uuid,
        arguments: ToolCallArguments,
    },
    ToolCallCompleted {
        tool_call_id: Uuid,
        content: String,
        is_error: bool,
    },
}

pub fn spawn_git_watcher(tx: mpsc::UnboundedSender<TaskMessage>) {
    tokio::task::spawn_blocking(move || {
        let task_id = Uuid::new_v4();
        let _ = tx.send(TaskMessage::TaskStatusChanged {
            task_id,
            kind: BackgroundTaskKind::GitIndex,
            status: TaskStatus::Running,
            description: "Git file indexing".to_string(),
        });

        match run_git_watcher(&tx) {
            Ok(()) => {
                let _ = tx.send(TaskMessage::TaskStatusChanged {
                    task_id,
                    kind: BackgroundTaskKind::GitIndex,
                    status: TaskStatus::Completed,
                    description: "Git file indexing".to_string(),
                });
            }
            Err(e) => {
                let _ = tx.send(TaskMessage::TaskStatusChanged {
                    task_id,
                    kind: BackgroundTaskKind::GitIndex,
                    status: TaskStatus::Failed,
                    description: format!("Git file indexing: {e}"),
                });
            }
        }
    });
}

fn run_git_watcher(tx: &mpsc::UnboundedSender<TaskMessage>) -> anyhow::Result<()> {
    use notify_debouncer_mini::{new_debouncer, DebouncedEventKind};
    use std::time::Duration;

    // Initial scan
    if let Ok(files) = scan_git_files() {
        let _ = tx.send(TaskMessage::GitFilesUpdated(files));
    }

    // Find the repo root to watch
    let repo = git2::Repository::open_from_env()?;
    let workdir = repo
        .workdir()
        .ok_or_else(|| anyhow::anyhow!("Bare repository, cannot watch"))?
        .to_path_buf();
    drop(repo);

    let (notify_tx, notify_rx) = std::sync::mpsc::channel();
    let mut debouncer = new_debouncer(Duration::from_millis(500), notify_tx)?;

    debouncer
        .watcher()
        .watch(&workdir, notify::RecursiveMode::Recursive)?;

    loop {
        match notify_rx.recv() {
            Ok(Ok(events)) => {
                let has_changes = events.iter().any(|e| e.kind == DebouncedEventKind::Any);
                if has_changes {
                    if let Ok(files) = scan_git_files() {
                        if tx.send(TaskMessage::GitFilesUpdated(files)).is_err() {
                            break;
                        }
                    }
                }
            }
            Ok(Err(e)) => {
                eprintln!("File watcher error: {e}");
            }
            Err(_) => break,
        }
    }

    Ok(())
}

fn scan_git_files() -> anyhow::Result<Vec<GitFileSnapshot>> {
    let repo = git2::Repository::open_from_env()?;
    let statuses = repo.statuses(Some(
        git2::StatusOptions::new()
            .include_untracked(true)
            .recurse_untracked_dirs(true),
    ))?;

    let mut files = Vec::new();
    for entry in statuses.iter() {
        let Some(path) = entry.path() else {
            continue;
        };
        let s = entry.status();
        let status = if s.contains(git2::Status::INDEX_NEW)
            || s.contains(git2::Status::INDEX_MODIFIED)
            || s.contains(git2::Status::INDEX_DELETED)
        {
            GitFileStatus::Staged
        } else if s.contains(git2::Status::WT_MODIFIED) || s.contains(git2::Status::WT_DELETED) {
            GitFileStatus::Modified
        } else if s.contains(git2::Status::WT_NEW) {
            GitFileStatus::Untracked
        } else {
            GitFileStatus::Tracked
        };
        files.push(GitFileSnapshot {
            path: path.to_string(),
            status,
        });
    }

    Ok(files)
}

pub fn spawn_tool_discovery(tx: mpsc::UnboundedSender<TaskMessage>) {
    let task_id = Uuid::new_v4();
    tokio::spawn(async move {
        let _ = tx.send(TaskMessage::TaskStatusChanged {
            task_id,
            kind: BackgroundTaskKind::ToolDiscovery,
            status: TaskStatus::Running,
            description: "Tool discovery".to_string(),
        });

        // Hardcoded initial tool list; will be extended with config-based
        // or MCP-based tool discovery.
        let tools = vec![
            ToolSnapshot {
                name: "web_search".to_string(),
                description: "Search the web for information".to_string(),
            },
            ToolSnapshot {
                name: "read_file".to_string(),
                description: "Read a file from the filesystem".to_string(),
            },
            ToolSnapshot {
                name: "write_file".to_string(),
                description: "Write content to a file".to_string(),
            },
        ];
        let _ = tx.send(TaskMessage::ToolsDiscovered(tools));

        let _ = tx.send(TaskMessage::TaskStatusChanged {
            task_id,
            kind: BackgroundTaskKind::ToolDiscovery,
            status: TaskStatus::Completed,
            description: "Tool discovery".to_string(),
        });
    });
}

pub fn spawn_context_summarization(tx: mpsc::UnboundedSender<TaskMessage>) {
    let task_id = Uuid::new_v4();
    tokio::spawn(async move {
        let _ = tx.send(TaskMessage::TaskStatusChanged {
            task_id,
            kind: BackgroundTaskKind::ContextSummarize,
            status: TaskStatus::Running,
            description: "Context summarization".to_string(),
        });

        // Stub: context summarization will use the LLM provider to summarize
        // older messages when conversation exceeds a threshold.
        // For now, this is a no-op.

        let _ = tx.send(TaskMessage::TaskStatusChanged {
            task_id,
            kind: BackgroundTaskKind::ContextSummarize,
            status: TaskStatus::Completed,
            description: "Context summarization".to_string(),
        });
    });
}
