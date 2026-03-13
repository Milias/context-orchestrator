use crate::graph::tool_types::ToolCallStatus;
use crate::graph::{EdgeKind, Node, Role};
use crate::llm::{ChatContent, ChatMessage, ContentBlock, RawJson};

use super::App;

impl App {
    pub(super) async fn build_context(&self) -> anyhow::Result<(Option<String>, Vec<ChatMessage>)> {
        let history = self.graph.get_branch_history(self.graph.active_branch())?;

        let mut system_prompt = None;
        let mut messages = Vec::new();

        for node in history {
            match node {
                Node::SystemDirective { content, .. } => {
                    system_prompt = Some(content.clone());
                }
                Node::Message {
                    id, role, content, ..
                } => match role {
                    Role::System => {}
                    Role::User => {
                        messages.push(ChatMessage::text("user", content));
                    }
                    Role::Assistant => {
                        let (asst_msg, result_msgs) =
                            self.build_assistant_message_with_tools(*id, content);
                        messages.push(asst_msg);
                        messages.extend(result_msgs);
                    }
                },
                // Non-conversation node types are skipped in LLM context
                Node::WorkItem { .. }
                | Node::GitFile { .. }
                | Node::Tool { .. }
                | Node::BackgroundTask { .. }
                | Node::ThinkBlock { .. }
                | Node::ToolCall { .. }
                | Node::ToolResult { .. } => {}
            }
        }

        let max_tokens = self.config.max_context_tokens;
        let tools = crate::tool_executor::registered_tool_definitions();
        let token_count = self
            .provider
            .count_tokens(
                &messages,
                &self.config.anthropic_model,
                system_prompt.as_deref(),
                &tools,
            )
            .await
            .unwrap_or_default();

        if token_count > max_tokens {
            truncate_messages(&mut messages, max_tokens, token_count);
        }

        sanitize_message_boundaries(&mut messages);

        Ok((system_prompt, messages))
    }

    /// Build assistant `ChatMessage` with `ToolUse` blocks and any following
    /// user `ToolResult` messages. Ensures Anthropic API tool call/result pairing.
    pub(super) fn build_assistant_message_with_tools(
        &self,
        message_id: uuid::Uuid,
        text_content: &str,
    ) -> (ChatMessage, Vec<ChatMessage>) {
        let tool_call_ids = self.graph.sources_by_edge(message_id, EdgeKind::Invoked);
        let mut tool_use_blocks = Vec::new();
        let mut result_blocks = Vec::new();
        for tc_id in &tool_call_ids {
            let Some(Node::ToolCall {
                status,
                arguments,
                api_tool_use_id,
                ..
            }) = self.graph.node(*tc_id)
            else {
                continue;
            };
            if *status != ToolCallStatus::Completed && *status != ToolCallStatus::Failed {
                continue;
            }

            // Take only the first ToolResult per ToolCall (Anthropic API expects 1:1 pairing).
            let result_id = self
                .graph
                .sources_by_edge(*tc_id, EdgeKind::Produced)
                .into_iter()
                .next();
            let Some(result_id) = result_id else {
                continue;
            };
            let Some(Node::ToolResult {
                content, is_error, ..
            }) = self.graph.node(result_id)
            else {
                continue;
            };
            let use_id = api_tool_use_id.clone().unwrap_or_else(|| tc_id.to_string());
            tool_use_blocks.push(ContentBlock::ToolUse {
                id: use_id.clone(),
                name: arguments.tool_name().to_string(),
                input: RawJson(arguments.to_input_json()),
            });
            result_blocks.push(ContentBlock::ToolResult {
                tool_use_id: use_id,
                content: content.clone(),
                is_error: *is_error,
            });
        }

        if tool_use_blocks.is_empty() {
            return (ChatMessage::text("assistant", text_content), vec![]);
        }
        let mut blocks = Vec::new();
        if !text_content.is_empty() {
            blocks.push(ContentBlock::Text {
                text: text_content.to_string(),
            });
        }
        blocks.extend(tool_use_blocks);
        let asst = ChatMessage {
            role: "assistant".to_string(),
            content: ChatContent::Blocks(blocks),
        };
        let results = ChatMessage {
            role: "user".to_string(),
            content: ChatContent::Blocks(result_blocks),
        };
        (asst, vec![results])
    }
}

fn truncate_messages(messages: &mut Vec<ChatMessage>, max_tokens: u32, token_count: u32) {
    // char_len() returns byte length, not character count. This is an acceptable
    // approximation for the ratio-based truncation heuristic.
    let total_chars: usize = messages.iter().map(|m| m.content.char_len()).sum();
    let ratio = f64::from(max_tokens) / f64::from(token_count);
    // Truncation/sign-loss/precision-loss are acceptable here: total_chars and ratio
    // are both non-negative and the result fits comfortably in usize for any realistic
    // conversation size.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    let target_chars = (total_chars as f64 * ratio) as usize;

    // M-3: find cutoff index in one pass, then drain once — O(n) not O(n²).
    let mut current_chars = total_chars;
    let mut remove_count = 0;
    while current_chars > target_chars && remove_count < messages.len() - 1 {
        current_chars -= messages[remove_count].content.char_len();
        remove_count += 1;
    }
    if remove_count > 0 {
        messages.drain(0..remove_count);
    }
}

fn sanitize_message_boundaries(messages: &mut Vec<ChatMessage>) {
    // Drop orphaned tool_result user messages at the front after truncation.
    // The Anthropic API rejects tool_result blocks without a preceding tool_use.
    // Note: build_assistant_message_with_tools creates pure ToolResult user messages
    // (no mixed Text+ToolResult), so checking all() is correct here.
    while messages.len() > 1 && messages[0].role == "user" {
        let all_results = matches!(&messages[0].content,
            ChatContent::Blocks(b) if b.iter().all(|b| matches!(b, ContentBlock::ToolResult { .. })));
        if all_results {
            messages.remove(0);
        } else {
            break;
        }
    }

    // H-3: Drop leading assistant messages — API requires conversation start with user.
    while messages.len() > 1 && messages[0].role == "assistant" {
        messages.remove(0);
    }

    // H-2: Drop trailing assistant messages with tool_use blocks that lack a following
    // tool_result. Loop to handle multiple stacked orphans.
    while messages.len() > 1 {
        let dominated = messages.last().is_some_and(|last| {
            last.role == "assistant"
                && matches!(&last.content,
                    ChatContent::Blocks(b) if b.iter().any(|b| matches!(b, ContentBlock::ToolUse { .. })))
        });
        if dominated {
            messages.pop();
        } else {
            break;
        }
    }
}
