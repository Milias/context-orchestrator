use crate::graph::{BackgroundTaskKind, GitFileStatus, TaskStatus, ToolResultContent};
use crate::llm::ChatMessage;
use crate::tools::PlanExtractionResult;
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
    Progress(AgentPhase),
    UserTokensCounted {
        node_id: Uuid,
        count: u32,
    },
    StreamDelta {
        text: String,
        is_thinking: bool,
    },
    IterationDone {
        response: String,
        think_text: String,
        output_tokens: Option<u32>,
        stop_reason: Option<String>,
    },
    ToolCallRequest {
        tool_call_id: Uuid,
        assistant_id: Uuid,
        api_id: String,
        name: String,
        input: String,
    },
    Finished,
    Error(String),
}

/// Tool result forwarded from the main loop to the agent task.
#[derive(Debug)]
pub struct AgentToolResult {
    pub tool_call_id: Uuid,
    pub content: ToolResultContent,
    pub is_error: bool,
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
    ToolCallCompleted {
        tool_call_id: Uuid,
        content: ToolResultContent,
        is_error: bool,
    },
    Agent(AgentEvent),
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
            ToolSnapshot {
                name: "list_directory".to_string(),
                description: "List files and directories at a given path".to_string(),
            },
            ToolSnapshot {
                name: "search_files".to_string(),
                description: "Search for a regex pattern across files".to_string(),
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
