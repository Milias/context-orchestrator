use crate::graph::{
    BackgroundTaskKind, GitFileStatus, StopReason, TaskStatus, ToolCallArguments, ToolResultContent,
};
use crate::storage::TokenTotals;
use serde::{Deserialize, Serialize};
use std::fmt;
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

// ── Agent loop types ─────────────────────────────────────────────────

/// A `tool_use` block received during streaming.
#[derive(Debug)]
pub struct ToolUseRecord {
    pub tool_call_id: Uuid,
    /// The Anthropic-assigned `tool_use` ID (e.g. `toolu_xxx`).
    pub api_id: String,
    pub name: String,
    pub input: String,
}

/// Progress phase of the background agent loop.
#[derive(Debug, Clone)]
pub enum AgentPhase {
    CountingTokens,
    BuildingContext,
    Connecting { attempt: u32, max: u32 },
    Receiving,
    ExecutingTools { count: usize },
}

impl fmt::Display for AgentPhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CountingTokens => write!(f, "Counting tokens..."),
            Self::BuildingContext => write!(f, "Building context..."),
            Self::Connecting { attempt, max } if *attempt > 1 => {
                write!(f, "Connecting ({attempt}/{max})...")
            }
            Self::Connecting { .. } => write!(f, "Connecting..."),
            Self::Receiving => write!(f, "Receiving..."),
            Self::ExecutingTools { count } => write!(f, "Executing {count} tool call(s)..."),
        }
    }
}

/// Events sent from the background agent loop to the main event loop.
#[derive(Debug)]
pub enum AgentEvent {
    Progress {
        phase_id: Uuid,
        phase: AgentPhase,
    },
    PhaseCompleted {
        phase_id: Uuid,
    },
    StreamDelta {
        text: String,
        is_thinking: bool,
    },
    /// The agent committed an assistant message to the shared graph.
    /// Main loop uses this for TUI display tracking only — no graph mutation needed.
    IterationCommitted {
        assistant_id: Uuid,
        stop_reason: Option<StopReason>,
    },
    /// The agent added tool call nodes to the shared graph and needs executors spawned.
    /// Main loop spawns tool execution + tracks cancel tokens. No graph mutation needed.
    ToolCallDispatched {
        tool_call_id: Uuid,
        arguments: ToolCallArguments,
    },
    /// API returned a non-retryable error. Record in graph, do NOT cancel agent.
    /// The agent loop will retry with rebuilt context.
    ApiError {
        phase_id: Uuid,
        message: String,
    },
    /// Non-fatal status message for TUI display (e.g., retry progress).
    /// Do NOT cancel agent.
    StatusMessage(String),
    Finished,
    /// Fatal error — agent cannot recover. Triggers cancellation.
    Error(String),
}

/// Notification from the main loop to the agent that a tool call completed.
/// The main loop already applied the result to the shared graph.
#[derive(Debug)]
pub struct AgentToolResult {
    pub tool_call_id: Uuid,
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
    ToolCallCompleted {
        tool_call_id: Uuid,
        content: ToolResultContent,
        is_error: bool,
    },
    Agent {
        agent_id: Uuid,
        event: AgentEvent,
    },
    /// A git worktree was created for a task agent. Updates the registry
    /// so subsequent tool executions use the correct working directory.
    WorktreeCreated {
        agent_id: Uuid,
        path: std::path::PathBuf,
    },
    /// Fresh lifetime token totals from the analytics DB.
    TokenTotalsUpdated(TokenTotals),
    /// Non-fatal analytics error to display in the status bar.
    AnalyticsError(String),
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

        // Derive tool list from the single source of truth (tool registry).
        let tools: Vec<ToolSnapshot> = crate::tool_executor::tool_registry()
            .iter()
            .map(|entry| ToolSnapshot {
                name: entry.name.as_str().to_string(),
                description: entry.description.to_string(),
            })
            .collect();
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

#[cfg(test)]
#[path = "tasks_tests.rs"]
mod tests;
