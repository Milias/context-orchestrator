use crate::graph::{EdgeKind, Node, WorkItemStatus};
use crate::llm::{background_llm_call, BackgroundLlmConfig, ChatMessage, LlmProvider};
use crate::tasks::{ContextSnapshot, TaskMessage, ToolExtractionOutcome};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::{mpsc, Semaphore};
use uuid::Uuid;

use crate::graph::BackgroundTaskKind;
use crate::graph::TaskStatus;

// ── Trigger Parsing ─────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum TriggerCommand {
    Plan { args: String },
}

/// Parse `~tool_name args` triggers from message text.
/// The `~` must be at start of line or preceded by whitespace.
/// Unknown tool names are ignored.
pub fn parse_triggers(text: &str) -> Vec<TriggerCommand> {
    let mut triggers = Vec::new();

    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("~plan") {
            // Ensure ~plan is the full token (not ~planning, etc.)
            if rest.is_empty() || rest.starts_with(' ') {
                let args = rest.trim().to_string();
                if !args.is_empty() {
                    triggers.push(TriggerCommand::Plan { args });
                }
            }
        }
    }

    triggers
}

// ── LLM Extraction Results ──────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanExtractionResult {
    pub title: String,
    pub description: Option<String>,
}

// ── Background Tool Extraction ──────────────────────────────────────

pub fn spawn_tool_extraction(
    trigger: TriggerCommand,
    snapshot: ContextSnapshot,
    provider: Arc<dyn LlmProvider>,
    semaphore: Arc<Semaphore>,
    bg_config: BackgroundLlmConfig,
    tx: mpsc::UnboundedSender<TaskMessage>,
) {
    tokio::spawn(async move {
        run_tool_extraction(trigger, snapshot, provider, semaphore, bg_config, tx).await;
    });
}

async fn run_tool_extraction(
    trigger: TriggerCommand,
    snapshot: ContextSnapshot,
    provider: Arc<dyn LlmProvider>,
    semaphore: Arc<Semaphore>,
    bg_config: BackgroundLlmConfig,
    tx: mpsc::UnboundedSender<TaskMessage>,
) {
    match trigger {
        TriggerCommand::Plan { args } => {
            run_plan_extraction(args, snapshot, provider, semaphore, bg_config, tx).await;
        }
    }
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
    let truncated_desc = if user_args.len() > 40 {
        format!("{}...", &user_args[..40])
    } else {
        user_args.clone()
    };

    let _ = tx.send(TaskMessage::TaskStatusChanged {
        task_id,
        kind: BackgroundTaskKind::ToolExtraction,
        status: TaskStatus::Running,
        description: format!("Extracting plan: {truncated_desc}"),
    });

    let messages = vec![ChatMessage {
        role: "user".to_string(),
        content: build_plan_prompt(&user_args, &snapshot),
    }];
    let config = bg_config.to_chat_config(Some(
        "You extract structured data from user requests. \
         Respond with ONLY valid JSON, no markdown fences, no explanation."
            .to_string(),
    ));

    let result = match background_llm_call(&*provider, messages, &config, &semaphore).await {
        Ok(response) => serde_json::from_str::<PlanExtractionResult>(&response.content).unwrap_or(
            PlanExtractionResult {
                title: user_args,
                description: None,
            },
        ),
        Err(_) => PlanExtractionResult {
            title: user_args,
            description: None,
        },
    };

    let _ = tx.send(TaskMessage::ToolExtractionComplete {
        trigger_message_id: snapshot.trigger_message_id,
        result: ToolExtractionOutcome::Plan(result),
    });
    let _ = tx.send(TaskMessage::TaskStatusChanged {
        task_id,
        kind: BackgroundTaskKind::ToolExtraction,
        status: TaskStatus::Completed,
        description: "Plan extracted".to_string(),
    });
}

fn build_plan_prompt(user_args: &str, snapshot: &ContextSnapshot) -> String {
    let recent: Vec<String> = snapshot
        .messages
        .iter()
        .rev()
        .take(6)
        .rev()
        .map(|m| format!("{}: {}", m.role, truncate_content(&m.content, 200)))
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
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max])
    }
}

#[cfg(test)]
#[path = "tools_tests.rs"]
mod tests;
