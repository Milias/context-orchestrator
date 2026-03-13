use crate::graph::tool_types::ToolCallArguments;
use crate::graph::{EdgeKind, Node, WorkItemStatus};
use crate::llm::{background_llm_call, BackgroundLlmConfig, ChatMessage, LlmProvider};
use crate::tasks::{ContextSnapshot, TaskMessage, ToolExtractionOutcome};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::{mpsc, Semaphore};
use uuid::Uuid;

use crate::graph::{BackgroundTaskKind, TaskStatus};

// ── Trigger Parsing ─────────────────────────────────────────────────

/// A parsed user trigger: the tool name and the raw argument string.
#[derive(Debug, Clone)]
pub struct ParsedTrigger {
    pub tool_name: String,
    pub args: String,
}

/// Parse `/tool_name args` triggers from message text.
/// Only tools registered in the tool registry are matched.
/// The `/` must be at start of line (after optional whitespace).
pub fn parse_triggers(text: &str) -> Vec<ParsedTrigger> {
    let registry = crate::tool_executor::tool_registry();
    let mut triggers = Vec::new();

    for line in text.lines() {
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix('/') else {
            continue;
        };
        for entry in registry {
            if let Some(after) = rest.strip_prefix(entry.name) {
                // Ensure the tool name is a full token (not a prefix of a longer word)
                if after.is_empty() || after.starts_with(' ') {
                    let args = after.trim().to_string();
                    if !args.is_empty() {
                        triggers.push(ParsedTrigger {
                            tool_name: entry.name.to_string(),
                            args,
                        });
                    }
                    break;
                }
            }
        }
    }

    triggers
}

/// Parse positional user trigger args into typed `ToolCallArguments`.
pub fn parse_user_trigger_args(tool_name: &str, args: &str) -> ToolCallArguments {
    match tool_name {
        "set" => {
            let mut parts = args.splitn(2, ' ');
            let key = parts.next().unwrap_or("").trim().to_string();
            let value = parts.next().unwrap_or("").trim().to_string();
            ToolCallArguments::Set { key, value }
        }
        "read_file" => ToolCallArguments::ReadFile {
            path: args.to_string(),
        },
        "write_file" => {
            let mut parts = args.splitn(2, ' ');
            let path = parts.next().unwrap_or("").to_string();
            let content = parts.next().unwrap_or("").to_string();
            ToolCallArguments::WriteFile { path, content }
        }
        "list_directory" => ToolCallArguments::ListDirectory {
            path: args.to_string(),
            recursive: None,
        },
        "search_files" => ToolCallArguments::SearchFiles {
            pattern: args.to_string(),
            path: None,
        },
        _ => ToolCallArguments::Unknown {
            tool_name: tool_name.to_string(),
            raw_json: args.to_string(),
        },
    }
}

// ── LLM Extraction (plan) ──────────────────────────────────────────

/// Spawn a background LLM call to extract structured plan args from free text.
/// Only used for `/plan` triggers — all other tools go through the unified
/// `handle_tool_call_dispatched` → `spawn_tool_execution` path.
pub fn spawn_plan_extraction(
    user_args: String,
    snapshot: ContextSnapshot,
    provider: Arc<dyn LlmProvider>,
    semaphore: Arc<Semaphore>,
    bg_config: BackgroundLlmConfig,
    tx: mpsc::UnboundedSender<TaskMessage>,
) {
    tokio::spawn(async move {
        run_plan_extraction(user_args, snapshot, provider, semaphore, bg_config, tx).await;
    });
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanExtractionResult {
    pub title: String,
    pub description: Option<String>,
}

async fn run_plan_extraction(
    user_args: String,
    snapshot: ContextSnapshot,
    provider: Arc<dyn LlmProvider>,
    semaphore: Arc<Semaphore>,
    bg_config: BackgroundLlmConfig,
    tx: mpsc::UnboundedSender<TaskMessage>,
) {
    let task_id = Uuid::new_v4();
    let truncated_desc = truncate_content(&user_args, 40);

    let _ = tx.send(TaskMessage::TaskStatusChanged {
        task_id,
        kind: BackgroundTaskKind::ToolExtraction,
        status: TaskStatus::Running,
        description: format!("Extracting plan: {truncated_desc}"),
    });

    let messages = vec![ChatMessage::text(
        "user",
        build_plan_prompt(&user_args, &snapshot),
    )];
    let config = bg_config.to_chat_config(Some(
        "You extract structured data from user requests. \
         Respond with ONLY valid JSON, no markdown fences, no explanation."
            .to_string(),
    ));

    let (result, final_status) =
        match background_llm_call(&*provider, messages, &config, &semaphore).await {
            Ok(response) => {
                let plan = serde_json::from_str::<PlanExtractionResult>(&response.content)
                    .unwrap_or(PlanExtractionResult {
                        title: user_args,
                        description: None,
                    });
                (plan, TaskStatus::Completed)
            }
            Err(_) => (
                PlanExtractionResult {
                    title: user_args,
                    description: None,
                },
                TaskStatus::Failed,
            ),
        };

    let status_desc = if final_status == TaskStatus::Completed {
        "Plan extracted".to_string()
    } else {
        "Plan extraction failed, used raw input".to_string()
    };

    let _ = tx.send(TaskMessage::ToolExtractionComplete {
        trigger_message_id: snapshot.trigger_message_id,
        result: ToolExtractionOutcome::Plan(result),
    });
    let _ = tx.send(TaskMessage::TaskStatusChanged {
        task_id,
        kind: BackgroundTaskKind::ToolExtraction,
        status: final_status,
        description: status_desc,
    });
}

fn build_plan_prompt(user_args: &str, snapshot: &ContextSnapshot) -> String {
    let recent: Vec<String> = snapshot
        .messages
        .iter()
        .rev()
        .take(6)
        .rev()
        .map(|m| {
            let text = m.text_content().unwrap_or("");
            format!("{}: {}", m.role, truncate_content(text, 200))
        })
        .collect();

    let tool_list: String = snapshot
        .tools
        .iter()
        .map(|t| format!("- {}: {}", t.name, t.description))
        .collect::<Vec<_>>()
        .join("\n");

    let context_block = if recent.is_empty() {
        String::new()
    } else {
        format!(
            "\n\nRecent conversation for context:\n{}",
            recent.join("\n")
        )
    };

    let tools_block = if tool_list.is_empty() {
        String::new()
    } else {
        format!("\n\nAvailable tools:\n{tool_list}")
    };

    format!(
        "The user wants to create a work item (plan). They wrote: \"{user_args}\"\
         {context_block}{tools_block}\n\n\
         Extract a structured work item. Respond with ONLY valid JSON:\n\
         {{\"title\": \"concise title\", \"description\": \"detailed description or null\"}}"
    )
}

// ── Helpers ─────────────────────────────────────────────────────────

/// Create a `WorkItem` node from a `PlanExtractionResult`.
pub fn plan_result_to_node(result: &PlanExtractionResult) -> Node {
    Node::WorkItem {
        id: Uuid::new_v4(),
        title: result.title.clone(),
        status: WorkItemStatus::Todo,
        description: result.description.clone(),
        created_at: Utc::now(),
    }
}

/// The edge kind used to link a tool-created node to its source message.
pub fn tool_result_edge_kind() -> EdgeKind {
    EdgeKind::RelevantTo
}

fn truncate_content(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max).collect();
        format!("{truncated}...")
    }
}

#[cfg(test)]
#[path = "tools_tests.rs"]
mod tests;
