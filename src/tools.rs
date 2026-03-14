use crate::graph::tool_types::{ToolCallArguments, ToolName};

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
            let name_str = entry.name.as_str();
            if let Some(after) = rest.strip_prefix(name_str) {
                // Ensure the tool name is a full token (not a prefix of a longer word)
                if after.is_empty() || after.starts_with(' ') {
                    let args = after.trim().to_string();
                    if !args.is_empty() {
                        triggers.push(ParsedTrigger {
                            tool_name: name_str.to_string(),
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
/// Uses `ToolName` enum for matching; tools requiring UUID args (`add_task`,
/// `update_work_item`, `add_dependency`) fall through to `Unknown` since
/// positional parsing cannot produce `Uuid` fields.
pub fn parse_user_trigger_args(tool_name: &str, args: &str) -> ToolCallArguments {
    let Some(name) = ToolName::from_str(tool_name) else {
        return ToolCallArguments::Unknown {
            tool_name: tool_name.to_string(),
            raw_json: args.to_string(),
        };
    };

    match name {
        ToolName::Plan => ToolCallArguments::Plan {
            title: args.to_string(),
            description: None,
        },
        ToolName::Set => {
            let mut parts = args.splitn(2, ' ');
            let key = parts.next().unwrap_or("").trim().to_string();
            let value = parts.next().unwrap_or("").trim().to_string();
            ToolCallArguments::Set { key, value }
        }
        ToolName::ReadFile => ToolCallArguments::ReadFile {
            path: args.to_string(),
        },
        ToolName::WriteFile => {
            let mut parts = args.splitn(2, ' ');
            let path = parts.next().unwrap_or("").to_string();
            let content = parts.next().unwrap_or("").to_string();
            ToolCallArguments::WriteFile { path, content }
        }
        ToolName::ListDirectory => ToolCallArguments::ListDirectory {
            path: args.to_string(),
            recursive: None,
        },
        ToolName::SearchFiles => ToolCallArguments::SearchFiles {
            pattern: args.to_string(),
            path: None,
        },
        ToolName::Ask => {
            // /ask user What JWT library? → destination=User, question="What JWT library?"
            // /ask llm Should we use JWT? → destination=Llm
            use crate::graph::node::QuestionDestination;
            let mut parts = args.splitn(2, ' ');
            let dest_str = parts.next().unwrap_or("user");
            let question = parts.next().unwrap_or("").to_string();
            let destination = match dest_str {
                "llm" => QuestionDestination::Llm,
                "auto" => QuestionDestination::Auto,
                _ => QuestionDestination::User,
            };
            ToolCallArguments::Ask {
                question,
                destination,
                about_node_id: None,
                requires_approval: None,
            }
        }
        // Tools requiring UUID args — positional parsing cannot produce Uuid fields.
        ToolName::AddTask
        | ToolName::UpdateWorkItem
        | ToolName::AddDependency
        | ToolName::WebSearch
        | ToolName::Answer => ToolCallArguments::Unknown {
            tool_name: tool_name.to_string(),
            raw_json: args.to_string(),
        },
    }
}

#[cfg(test)]
#[path = "tools_tests.rs"]
mod tests;
