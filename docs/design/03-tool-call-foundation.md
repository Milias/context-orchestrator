# Design 03: Tool Call Foundation — Graph Nodes, Dispatch, and LLM tool_use

> Status: Implemented
> Date: 2026-03-12
> Depends on: [02-background-llm-and-tool-invocation.md](./02-background-llm-and-tool-invocation.md)

---

## Table of Contents

1. [Motivation](#motivation)
2. [Why Tool Calls Are Graph Nodes](#why-tool-calls-are-graph-nodes)
3. [Closed-Enum Argument Strategy](#closed-enum-argument-strategy)
4. [Channel Architecture](#channel-architecture)
5. [LLM tool_use Integration](#llm-tool_use-integration)
6. [Provenance Edges](#provenance-edges)
7. [Future: Multi-Agent and Graph-Manipulation Tools](#future-multi-agent-and-graph-manipulation-tools)

---

## Motivation

Design 02 established background LLM calls and inline tool invocation (`~plan`). But the results
of tool operations are invisible — only the final `WorkItem` node enters the graph. The operation
itself (what was called, with what arguments, when it started, when it finished, whether it failed)
is lost.

VISION.md (Sections 3.1 and 4.8) requires tool calls as first-class graph citizens. Every tool
invocation must be traceable: which message triggered it, what arguments were used, what the result
was, and how long it took. This is the foundation for:

- **Agent loops**: an LLM that can call tools, observe results, and decide what to do next
- **Debugging**: seeing exactly what happened when a tool failed
- **Replay**: re-executing a tool call with the same or modified arguments
- **Multi-agent coordination**: agents seeing each other's tool calls via the shared graph

---

## Why Tool Calls Are Graph Nodes

Tool calls could be stored as metadata on messages, logged to a side channel, or kept in a separate
data structure. We choose graph nodes because:

1. **Uniform querying**: `nodes_by(|n| matches!(n, Node::ToolCall { .. }))` works with existing
   infrastructure. No parallel data model to maintain.
2. **Edge-based provenance**: `Invoked` edges from message to tool call and `Produced` edges from
   tool call to result create a traceable chain using the same edge system.
3. **TUI integration**: tool calls appear in the Tasks tab alongside `BackgroundTask` nodes. No
   special-case rendering pipeline.
4. **Persistence**: tool calls are automatically saved/loaded with the graph. No migration needed
   for the separate storage — they're just more V2 node variants.
5. **Agent visibility**: any agent with graph access can see pending/running/completed tool calls
   from any other agent. This is the coordination primitive for multi-agent systems.

### Node Types

- `ToolCall`: represents an invocation. Has `arguments` (typed), `status` (lifecycle), and
  `parent_message_id` (the assistant message that triggered it).
- `ToolResult`: represents the output. Has `tool_call_id` (back-reference), `content` (the
  result text), and `is_error` (success/failure).

### Edge Types

- `Invoked`: message → tool call. "This message invoked this tool."
- `Produced`: tool call → tool result. "This tool call produced this result."

---

## Closed-Enum Argument Strategy

Tool arguments use `ToolCallArguments`, a closed enum with `#[serde(tag = "tool_type")]`:

```rust
pub enum ToolCallArguments {
    Plan { raw_input: String, description: Option<String> },
    ReadFile { path: String },
    WriteFile { path: String, content: String },
    WebSearch { query: String },
    Unknown { tool_name: String, raw_json: String },
}
```

**Why not `serde_json::Value`?** Project rule: never use `serde_json::Value`. But more importantly:
typed variants enable compile-time exhaustiveness checks. When a new tool is added, every `match`
on `ToolCallArguments` must handle it. The `Unknown` variant is the escape hatch for MCP tools and
tools not yet in the enum.

**Single parse point**: `parse_tool_arguments(name: &str, raw_json: &str) -> ToolCallArguments`
is the one place where stringly-typed LLM output becomes a typed Rust value. It dispatches on the
tool name, attempts typed deserialization, and falls back to `Unknown`.

**Tool name derivation**: `ToolCallArguments::tool_name()` derives the tool name from the enum
discriminant. No separate `tool_name: String` field on the node — the name IS the variant.

---

## Channel Architecture

Tool dispatch extends the existing `TaskMessage` enum rather than adding a separate channel:

```
TaskMessage::ToolCallDispatched { tool_call_id, parent_message_id, arguments }
TaskMessage::ToolCallCompleted { tool_call_id, content, is_error }
```

**Why extend TaskMessage?** The `task_handler` already processes all background task results in a
single `tokio::select!` arm. Adding tool call messages to the same channel means:

- No new `select!` arm in the main loop
- Tool calls participate in the same ordering guarantees as other task messages
- The handler has access to `&mut self` (the `App`) for graph mutations

The dispatch flow (inline, post-C1 fix):
1. `stream_llm_response` receives `StreamChunk::ToolUse` from the SSE parser and records it
2. After streaming completes, `handle_send_message` creates the assistant node
3. For each recorded tool use, it calls `handle_tool_call_dispatched` directly with the real
   assistant node ID — creating the `ToolCall` node, `Invoked` edge, and spawning the executor
4. The executor runs asynchronously, sends `ToolCallCompleted` back through `task_tx`
5. `task_handler` creates the `ToolResult` node, updates the `ToolCall` status

`ToolCallDispatched` remains in `TaskMessage` for background-triggered tool dispatch (e.g., from
future agents that don't go through the streaming path).

---

## LLM tool_use Integration

### ChatContent Evolution

`ChatMessage.content` changes from `String` to `ChatContent`:

```rust
pub enum ChatContent {
    Text(String),
    Blocks(Vec<ContentBlock>),
}
```

`ChatContent::Text` is the common case (backward compatible). `ChatContent::Blocks` is used when
the message contains tool_use or tool_result blocks alongside text.

### Anthropic SSE Events

New SSE event types for tool_use:
- `content_block_start` with `type: "tool_use"` — begins a tool call block
- `content_block_delta` with `type: "input_json_delta"` — accumulates the JSON arguments
- `content_block_stop` — finalizes the tool call, emits `StreamChunk::ToolUse`

The SSE parser accumulates partial JSON across delta events and emits a complete
`StreamChunk::ToolUse { id, name, input }` when the block stops. The `input` field is raw JSON;
typed parsing happens at dispatch time via `parse_tool_arguments`.

### Tool Definitions

Tools are described to the LLM via `ToolDefinition` structs with purpose-built schema types
(`ToolInputSchema`, `SchemaProperty`, `SchemaType`). The Anthropic provider converts these to
the API's JSON Schema format via private serialization structs.

---

## Provenance Edges

Every tool call creates an automatic provenance chain. Edges follow the same child-to-parent
convention as `RespondsTo` — the `from` node points toward the node it depends on:

```
ToolCall --[Invoked]--> Assistant Message
ToolResult --[Produced]--> ToolCall
```

Semantically: "this ToolCall was invoked by this Message" and "this ToolResult was produced by
this ToolCall." To traverse from a message to its tool calls, use `sources_by_edge(message_id,
Invoked)`. To find a tool call's results, use `sources_by_edge(tool_call_id, Produced)`.

The `invoked_by: HashMap<Uuid, Uuid>` runtime index (analogous to `responds_to`) enables fast
lookups: given a tool call, find the message that invoked it. This index is rebuilt on
deserialization, not persisted.

---

## Future: Multi-Agent and Graph-Manipulation Tools

This foundation enables:

1. **Agent loops**: when tool results come back, the context builder can inject them as
   `ChatContent::Blocks` containing `ToolUse` + `ToolResult` pairs. The LLM sees the results
   and can issue more tool calls.

2. **Graph-manipulation tools**: tools that add/remove nodes and edges. An LLM can use a tool
   to define edges for its own tool calls, creating custom provenance chains.

3. **Multi-agent messaging**: agents send messages via tool calls. Each message is a graph node
   linked to the tool call that created it. Agents discover each other's messages via graph queries.

---

## Implementation Status

_Updated as phases are implemented._

| Phase | Status | Notes |
|-------|--------|-------|
| 0: Design doc | Done | This document |
| 1: Graph types | Done | `src/graph/tool_types.rs`: `ToolCallStatus`, `ToolCallArguments`, `parse_tool_arguments()`. `Node::ToolCall`/`ToolResult` in `src/graph/mod.rs`. `EdgeKind::Invoked`/`Produced`. `invoked_by` runtime index. 5 tests in `tool_types_tests.rs`. |
| 2: Tool dispatch | Done | `TaskMessage::ToolCallDispatched`/`ToolCallCompleted` in `src/tasks.rs`. `src/tool_executor.rs`: stub executor returning errors. `src/app/task_handler.rs`: dispatch handler creates nodes, edges, spawns executor. 2 tests in `tool_executor_tests.rs`. |
| 3: LLM tool_use | Done | `src/llm/tool_types.rs`: `ToolDefinition`, `ToolInputSchema`, `SchemaType`, `ChatContent`, `ContentBlock`, `RawJson`. `ChatMessage.content` → `ChatContent` with `::text()` convenience. `StreamChunk::ToolUse`. `PendingToolUse` SSE accumulator in `src/llm/anthropic.rs`. `ContentBlock::ToolUse.input` uses `RawJson` to serialize as a JSON object (not a quoted string). 3 tests in `llm/tool_types_tests.rs`, 1 in `anthropic_tests.rs`. |
| 4: Stream provenance | Done | `StreamResult` struct, `ToolUseRecord` (with `api_id` field) in `src/app/streaming.rs`. `stream_llm_response` records `ToolUse` chunks. `handle_send_message` creates `ToolCall` nodes inline after the assistant node exists (avoids ordering bug). `ThinkSplitter` extracted to `src/app/think_splitter.rs`. Streaming logic extracted to `src/app/streaming.rs`. |
| 5: TUI updates | Done | `ToolCall`/`ToolResult` arms in `format_node_line()` in `src/tui/widgets/context_panel.rs`. Tasks tab filter includes `ToolCall`/`ToolResult`. Status markers: ○/◉/✓/✗/⊘ (`Cancelled`). |
| 6: Context builder | Done | `build_assistant_message_with_tools()` builds assistant messages as `ChatContent::Blocks` with `Text` + `ToolUse` blocks, paired with user `ToolResult` messages. `to_input_json()` strips the serde tag with JSON validation for `Unknown` variant. Takes only first `ToolResult` per `ToolCall` for 1:1 API pairing. Truncation drops orphaned `tool_result` messages. `api_tool_use_id` on `Node::ToolCall` preserves Anthropic's `toolu_xxx` IDs. 7 tests in `tool_types_tests.rs`. |
| 7: Red/green team | Done | 3 parallel review agents. C1: removed deferred dispatch, tool calls created inline with real parent ID. C2: `api_tool_use_id` field preserves Anthropic tool IDs. H1: invalid JSON in `Unknown` variant falls back to `{}`. H2: single `ToolResult` per `ToolCall`. L1: truncation drops orphaned `tool_result` user messages. L3: documented `tool_type` field assumption. L4: `test_tool_call_provenance_chain_query` covers full query pattern. |
| 8: Tool registration | Done | `registered_tool_definitions()` in `src/tool_executor.rs` returns `read_file` definition. Wired into `ChatConfig.tools` in `src/app/mod.rs`. `max_tool_loop_iterations` config in `src/config.rs`. |
| 9: read_file executor | Done | Real `tokio::fs::read_to_string` implementation with 100KB truncation and char-boundary safety. 3 tests in `tool_executor_tests.rs`. |
| 10: stop_reason + hardening | Done | `stop_reason` propagated through `StreamChunk::Done` → `StreamResult` → agent loop. `message_delta` SSE parsing captures `stop_reason`. Trailing orphaned `tool_use` cleanup in `build_context`. `count_tokens` failure non-fatal (`unwrap_or(0)`). |
| 11: Agent loop | Done | `src/app/agent_loop.rs`: `run_agent_loop` iterates up to `max_tool_loop_iterations`, dispatching tool calls and waiting for results. `wait_for_tool_completions` drains `task_rx` with TUI responsiveness, forwards non-tool messages, 60s timeout marks Failed + creates error ToolResult. `handle_send_message` simplified to delegate to agent loop. 1 test for `spawn_tool_execution` channel flow. |
| 12: Red/green team #2 | Done | C-1: stale completion guard in `handle_tool_call_completed`. C-2: `read_file` path validation (canonicalize + `starts_with` cwd). H-1: restore consumed input during tool wait. H-2: while-loop for trailing orphaned tool_use. H-3: leading assistant message cleanup. H-4: `count_tokens` includes tool definitions. H-5: fair polling (no biased select). M-1: break agent loop after timeout. M-2: warn on empty tool_use. M-3: O(n) drain truncation. M-4: clear streaming_response between iterations. M-5: comment on pure ToolResult messages. M-6: skip empty Text block in tool_use messages. L-3: improved error message for Unknown tool variant. 81 tests. |
