use crate::graph::tool_types::ToolCallArguments;

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
        "plan" => ToolCallArguments::Plan {
            title: args.to_string(),
            description: None,
        },
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

#[cfg(test)]
#[path = "tools_tests.rs"]
mod tests;
