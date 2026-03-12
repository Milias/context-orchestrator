# Design 02: Background LLM and Inline Tool Invocation

> Status: Draft
> Date: 2026-03-12
> Depends on: [01-graph-extensions-and-context-panel.md](./01-graph-extensions-and-context-panel.md)

---

## Table of Contents

1. [Motivation](#motivation)
2. [Architecture Overview](#architecture-overview)
3. [Shared LLM Provider Design](#shared-llm-provider-design)
4. [Graph Snapshot Pattern](#graph-snapshot-pattern)
5. [Inline Tool Invocation Design](#inline-tool-invocation-design)
6. [Work Tab](#work-tab)
7. [Integration with Existing Systems](#integration-with-existing-systems)
8. [Future Extensibility](#future-extensibility)
9. [Trade-offs and Alternatives](#trade-offs-and-alternatives)
10. [Implementation Phases](#implementation-phases)

---

## Motivation

The context-manager currently uses LLM calls in exactly one place: the main conversation loop in
`handle_send_message`. The user types a message, the app streams a response, done. But the
application's future depends on LLM calls happening _everywhere else_:

- Context summarization (compacting old messages to stay within token limits)
- Tool parameter extraction (turning `~plan fix the auth bug` into a structured WorkItem)
- Relevance scoring (deciding which GitFile or Tool nodes belong in context)
- Agent loops (multi-step reasoning chains that run autonomously)

These are all **background** operations. They do not stream tokens to the TUI. They do not block
user input. They produce structured results that get folded back into the graph. This document
establishes background LLM calls as the foundational execution pattern and builds inline tool
invocation as the first concrete use of that pattern.

---

## Architecture Overview

### The Core Loop Today

```
User Input --> build_context --> stream LLM --> create Message node
                                    ^
                                    |
                            Box<dyn LlmProvider>
                            (exclusive to main conversation)
```

Background tasks (git watcher, tool discovery) run independently but have no access to the LLM
provider. They produce `TaskMessage` values that the main loop processes.

### The Core Loop After This Design

```
                              Arc<dyn LlmProvider>
                             /         |          \
                            /          |           \
User Input -----> main conversation   background    background
  |               (streams to TUI)    LLM task 1    LLM task 2
  |                     |                 |              |
  |               no semaphore      Semaphore(2) --------
  |                     |                 |
  v                     v                 v
parse for       create Message     TaskMessage results
~tool triggers       node          fed back to main loop
  |                                      |
  v                                      v
spawn background                   graph mutations
LLM extraction                     (main thread only)
```

### Data Flow: Inline Tool Invocation

The full lifecycle of a `~plan` invocation:

```
+------------------+     +-------------------+     +---------------------+
| 1. User types:   |     | 2. Parse phase    |     | 3. Snapshot phase   |
| "~plan fix the   | --> | detect_trigger()  | --> | clone relevant      |
|  auth module"    |     | finds ~plan with  |     | graph data into     |
|                  |     | raw_args="fix the |     | ContextSnapshot     |
|                  |     |  auth module"     |     |                     |
+------------------+     +-------------------+     +---------------------+
                                                            |
                                                            v
+------------------+     +-------------------+     +---------------------+
| 6. Graph mutated |     | 5. Main loop      |     | 4. Background task  |
| - WorkItem node  | <-- | receives          | <-- | acquires semaphore  |
|   created        |     | TaskMessage::      |     | calls LLM with     |
| - RelevantTo     |     | ToolResult        |     | extraction prompt   |
|   edge added     |     |                   |     | parses structured   |
| - status bar     |     |                   |     | response            |
|   updated        |     |                   |     |                     |
+------------------+     +-------------------+     +---------------------+
```

### Key Invariants

1. **Graph mutations happen only on the main thread.** Background tasks never hold a reference to
   the graph. They receive a snapshot, do their work, and send results back via `TaskMessage`.

2. **The main conversation never waits for a semaphore.** Interactive responsiveness is
   non-negotiable. Only background tasks contend for permits.

3. **Every background LLM call has a cost budget.** The `BackgroundLlmConfig` specifies model and
   token limits independently of the main conversation config.

4. **No `serde_json::Value`.** Every LLM response is parsed into a typed struct. Malformed
   responses are errors, not silent degradation.

---

## Shared LLM Provider Design

### Why Arc<dyn LlmProvider>

The `AnthropicProvider` holds three fields:

```rust
pub struct AnthropicProvider {
    api_key: String,         // immutable after construction
    base_url: String,        // immutable after construction
    client: reqwest::Client, // internally Arc'd, designed for concurrent use
}
```

There is no mutable state. The `LlmProvider` trait takes `&self` for all methods and already
requires `Send + Sync`. Wrapping in `Arc` is the minimal change that enables sharing:

```rust
// Before
pub struct App {
    provider: Box<dyn LlmProvider>,
    // ...
}

// After
pub struct App {
    provider: Arc<dyn LlmProvider>,
    // ...
}
```

This is a one-line change at construction and all existing call sites remain identical because
`Arc<T>` implements `Deref<Target = T>`. No method signatures change.

### Concurrency Control with Semaphore

Background LLM calls need throttling. Without it, a burst of tool invocations could fire 10+
concurrent API calls, burning budget and hitting rate limits. A `tokio::sync::Semaphore` provides
cooperative throttling:

```rust
pub struct App {
    provider: Arc<dyn LlmProvider>,
    bg_semaphore: Arc<Semaphore>,
    bg_llm_config: BackgroundLlmConfig,
    // ...
}
```

```rust
pub struct BackgroundLlmConfig {
    /// Model to use for background tasks (e.g., "claude-haiku-4-20260310").
    pub model: String,
    /// Maximum output tokens for background responses (e.g., 1024).
    pub max_tokens: u32,
    /// Maximum input context tokens for background prompts (e.g., 8_000).
    pub max_context_tokens: u32,
    /// Number of semaphore permits controlling concurrent background calls.
    pub max_concurrent: usize, // default: 2
}
```

The `BackgroundLlmConfig` is derived from `AppConfig` with sensible defaults. Background tasks
typically need far less context and output than the main conversation, so a cheaper, faster model
with tighter token limits is the right default. Users who want to override this can add fields to
their config file.

### Why the Main Conversation Bypasses the Semaphore

The semaphore exists to protect against cost runaway and rate limiting from background tasks. The
main conversation is user-initiated, one-at-a-time, and must remain responsive. If a user sends a
message while two background tasks hold permits, we must not block the conversation behind them.

The separation is simple: `handle_send_message` calls `provider.chat()` directly.
`spawn_background_llm_task` acquires a permit first.

```
Main conversation path:    provider.chat(messages, config)
Background task path:      semaphore.acquire() --> provider.chat(messages, config)
```

There is no risk of the main conversation starving background tasks either, because the main
conversation is inherently serial (one user message at a time, streaming until complete). By the
time the user sends a second message, the previous stream has finished and the provider is free.

### Non-Streaming Background Helper

Background tasks do not need streaming. There is no TUI element to update incrementally. A helper
function collects the full response:

```rust
/// Collects a full LLM response without streaming to the TUI.
/// Used by all background tasks that need LLM intelligence.
pub async fn background_llm_call(
    provider: &dyn LlmProvider,
    messages: Vec<ChatMessage>,
    config: &ChatConfig,
) -> Result<BackgroundLlmResponse> {
    let mut stream = provider.chat(messages, config).await?;
    let mut text = String::new();
    let mut input_tokens = None;
    let mut output_tokens = None;

    while let Some(chunk) = stream.next().await {
        match chunk? {
            StreamChunk::TextDelta(t) => text.push_str(&t),
            StreamChunk::Done {
                input_tokens: i,
                output_tokens: o,
            } => {
                input_tokens = i;
                output_tokens = o;
                break;
            }
            StreamChunk::Error(e) => anyhow::bail!("LLM error: {e}"),
        }
    }

    Ok(BackgroundLlmResponse {
        text,
        input_tokens,
        output_tokens,
    })
}
```

```rust
pub struct BackgroundLlmResponse {
    pub text: String,
    pub input_tokens: Option<u32>,
    pub output_tokens: Option<u32>,
}
```

This function lives in `src/llm/mod.rs` alongside the trait, usable by any background task. It
reuses the existing streaming infrastructure (no new HTTP path needed) and simply buffers the
chunks into a `String`. The token counts are preserved for cost tracking.

---

## Graph Snapshot Pattern

### Why Not Arc<RwLock<ConversationGraph>>

The most obvious approach to sharing the graph is wrapping it in `Arc<RwLock<_>>`. This is wrong
for several reasons:

1. **Lock infection.** Every graph access -- rendering the TUI, building context, adding a message
   -- must acquire the lock. A background task holding a read lock while doing a slow LLM call
   (seconds to tens of seconds) blocks all writes. A write lock for adding a node blocks all reads,
   including TUI rendering.

2. **Deadlock risk.** The main thread renders the TUI (read lock) and processes user input (write
   lock) in the same `tokio::select!` loop. If a background task holds a read lock and sends a
   `TaskMessage` that triggers a write, and the main thread is already holding a read lock for
   rendering, we have a deadlock scenario that depends on timing.

3. **Complexity sprawl.** Every function that touches the graph gains `RwLock` noise. Error
   handling for poisoned locks infects the entire codebase. The `responds_to` runtime index
   (rebuilt on deserialize, mutated on node insertion) becomes a synchronization hazard.

4. **Testing burden.** Unit tests for graph operations must now set up `Arc<RwLock<_>>` wrappers.
   Integration tests must reason about lock ordering. The testing surface area doubles for no
   functional benefit.

### Why Not Request/Response Channels

An alternative: background tasks send "give me context" requests to the main thread and wait for
responses. This avoids shared state but introduces:

1. **Async round-trips.** The background task blocks waiting for the main thread to respond. If
   the main thread is busy (rendering, handling input, processing another TaskMessage), latency
   spikes. An LLM extraction that should take 1 second now takes 1 second + queueing delay.

2. **Channel complexity.** Each background task needs a dedicated response channel (or a
   multiplexed one with correlation IDs). This is a mini actor system -- substantial infrastructure
   for a problem that does not require it.

3. **Ordering hazards.** Multiple tasks requesting context interleave with graph mutations,
   creating subtle race conditions in what each task "sees." The system becomes harder to reason
   about than the shared-state approach it was meant to replace.

### The Snapshot Approach

Background tasks receive a `ContextSnapshot` at spawn time: a cheaply-cloned subset of graph data
relevant to their work. The graph continues to evolve on the main thread. The snapshot is
immutable and owned entirely by the background task.

```rust
/// An immutable, cloned subset of graph state provided to background tasks.
/// Created on the main thread at task spawn time. Owned entirely by the task.
pub struct ContextSnapshot {
    /// Recent messages from the active branch, oldest first.
    pub recent_messages: Vec<SnapshotMessage>,
    /// Active branch name at snapshot time.
    pub branch: String,
    /// Source node ID that triggered this background task (e.g., the user message
    /// containing a ~tool trigger).
    pub trigger_node_id: Uuid,
    /// Available tools known to the graph at snapshot time.
    pub available_tools: Vec<SnapshotTool>,
    /// Active work items at snapshot time.
    pub active_work_items: Vec<SnapshotWorkItem>,
}

pub struct SnapshotMessage {
    pub id: Uuid,
    pub role: String,
    pub content: String,
    pub timestamp: DateTime<Utc>,
}

pub struct SnapshotTool {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
}

pub struct SnapshotWorkItem {
    pub id: Uuid,
    pub title: String,
    pub status: WorkItemStatus,
}
```

### Context Assembly Per Task Type

Different background tasks need different slices of the graph. The `build_snapshot` method on
`ConversationGraph` takes parameters controlling what to include:

```rust
impl ConversationGraph {
    pub fn build_snapshot(
        &self,
        branch: &str,
        message_limit: usize,
        trigger_node_id: Uuid,
        include_tools: bool,
        include_work_items: bool,
    ) -> Result<ContextSnapshot> {
        // Walk branch history, take last `message_limit` messages
        // Optionally collect Tool and WorkItem nodes
        // Clone into owned snapshot structs
    }
}
```

The parameters vary by task type:

```
+------------------------+-----------+-------+------------+------------------+
| Task Type              | Messages  | Tools | Work Items | Notes            |
+------------------------+-----------+-------+------------+------------------+
| Tool extraction (~plan)| Last 10   | Yes   | Yes        | Needs context to |
|                        |           |       |            | understand intent |
+------------------------+-----------+-------+------------+------------------+
| Context summarization  | Range to  | No    | No         | Summarizes a     |
|                        | summarize |       |            | specific span    |
+------------------------+-----------+-------+------------+------------------+
| Relevance scoring      | Last 5    | No    | No         | + candidate      |
| (future)               |           |       |            | node list        |
+------------------------+-----------+-------+------------+------------------+
| Agent loop step        | Last 20+  | Yes   | Yes        | Full working     |
| (future)               |           |       |            | context needed   |
+------------------------+-----------+-------+------------+------------------+
```

### Staleness

A snapshot is stale the moment it is created. A user could send another message, a git watcher
could update files, another background task could complete -- all while the snapshot is being
processed. This is acceptable because:

- **Tool extraction** operates on the context as it existed when the user typed `~plan`. If the
  user sends a follow-up message, that message was not part of the original intent. The extraction
  should reflect the moment of invocation.

- **Summarization** compacts a historical range of messages. New messages do not retroactively
  change old ones. The summary of messages 1-50 is the same whether message 51 exists or not.

- **Relevance scoring** is inherently approximate. Stale-by-one-message does not meaningfully
  degrade quality. The next scoring pass will incorporate the new message.

The snapshot pattern trades perfect consistency for architectural simplicity and freedom from
locking. This is the right trade-off for a TUI application where responsiveness is paramount and
eventual consistency is natural.

---

## Inline Tool Invocation Design

### Trigger Syntax

Tools are invoked inline using `~tool_name` at the start of a message or after a newline:

```
~plan fix the authentication module
~search recent errors in payment processing
~summarize this conversation so far
```

**Parsing rules:**

- The tilde `~` must appear at position 0 of the trimmed message or at the start of a line
  (after `\n` and optional whitespace).
- The tool name is the contiguous ASCII alphanumeric/underscore sequence immediately following `~`.
- Everything after the tool name (trimmed of leading whitespace) is the raw argument string.
- A message may contain at most one tool trigger. The first valid trigger wins.
- If the tool name does not match any known tool, the message is treated as a normal conversation
  message with no error. The `~` is just text.

**Why tilde?** It is visually distinct, rarely used at line starts in natural text, and evokes
Unix home-directory shorthand -- a "shortcut to something." It does not conflict with Markdown
syntax (`#`, `*`, `-`, `>`), code fences, or any common chat convention.

### Parsing Implementation

Trigger detection is deterministic string parsing. No LLM call, no regex crate dependency, no
ambiguity:

```rust
pub struct ToolTrigger {
    pub tool: InlineTool,
    pub raw_args: String,
    pub full_message: String,
}

/// Scans a user message for a ~tool trigger. Returns None if no valid trigger found.
pub fn detect_trigger(message: &str) -> Option<ToolTrigger> {
    let trimmed = message.trim();

    // Find the first line starting with ~
    let tool_line = if trimmed.starts_with('~') {
        trimmed
    } else {
        trimmed
            .lines()
            .find(|line| line.trim_start().starts_with('~'))?
            .trim_start()
    };

    // Extract tool name: contiguous [a-zA-Z0-9_] after ~
    let after_tilde = &tool_line[1..]; // safe: we confirmed ~ exists
    let name_end = after_tilde
        .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .unwrap_or(after_tilde.len());
    let name = &after_tilde[..name_end];

    if name.is_empty() {
        return None; // bare ~ with no name
    }

    let tool = InlineTool::from_name(name)?;
    let raw_args = after_tilde[name_end..].trim().to_string();

    Some(ToolTrigger {
        tool,
        raw_args,
        full_message: message.to_string(),
    })
}
```

### Tool Registry: Enum-Based

Tools are an enum, not trait objects. The set of inline tools is small (single digits), known at
compile time, and each tool has completely different extraction logic and result types. An enum
with match dispatch is simpler, faster, and produces better compiler diagnostics than dynamic
dispatch:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InlineTool {
    Plan,
    // Future: Search, Summarize, Status, ...
}

impl InlineTool {
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "plan" => Some(Self::Plan),
            _ => None,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Plan => "plan",
        }
    }
}
```

Adding a tool means adding a variant and match arms. The compiler enforces exhaustive matching
across the codebase, so forgetting to handle a new tool anywhere is a compile error.

### Two-Step Pipeline: Parse Then Extract

**Step 1 (parse)** is synchronous and happens inline in `handle_send_message`.
**Step 2 (extract)** is an async background task that uses the LLM.

```
handle_send_message(user_input)
  |
  +--> detect_trigger(user_input)
  |      |
  |      +--> Some(ToolTrigger { tool: Plan, raw_args, .. })
  |      |       |
  |      |       +--> build ContextSnapshot from graph
  |      |       |
  |      |       +--> spawn_tool_extraction(trigger, snapshot, provider, semaphore, task_tx)
  |      |       |
  |      |       +--> update status bar: "Extracting plan..."
  |      |
  |      +--> None --> (no additional work)
  |
  +--> create user Message node in graph (always, regardless of trigger)
  |
  +--> build_context --> stream LLM --> create assistant Message node (always)
```

The user message is **always** added to the graph and **always** sent to the main conversation
LLM, regardless of whether it contains a tool trigger. The tool trigger spawns additional
background work that runs concurrently with the main conversation response. The user gets
immediate conversational feedback AND structured tool results.

### LLM Extraction Prompt

The background task constructs a focused prompt for parameter extraction. For `~plan`:

```rust
fn build_plan_extraction_prompt(
    trigger: &ToolTrigger,
    snapshot: &ContextSnapshot,
) -> Vec<ChatMessage> {
    let recent_context: String = snapshot
        .recent_messages
        .iter()
        .map(|m| format!("{}: {}", m.role, m.content))
        .collect::<Vec<_>>()
        .join("\n\n");

    let existing_items: String = if snapshot.active_work_items.is_empty() {
        "None".to_string()
    } else {
        snapshot
            .active_work_items
            .iter()
            .map(|w| format!("- [{}] {}", w.status, w.title))
            .collect::<Vec<_>>()
            .join("\n")
    };

    vec![ChatMessage {
        role: "user".to_string(),
        content: format!(
            "Extract a work item from the following user request. \
             Use the conversation context to understand what they mean.\n\n\
             <conversation_context>\n{recent_context}\n</conversation_context>\n\n\
             <existing_work_items>\n{existing_items}\n</existing_work_items>\n\n\
             <user_request>\n{raw_args}\n</user_request>\n\n\
             Respond with ONLY a JSON object in this exact format:\n\
             {{\"title\": \"short imperative title\", \"description\": \"detailed description or null\"}}\n\n\
             Rules:\n\
             - title: 3-10 words, imperative mood (\"Fix auth module\", not \"Fixing the auth module\")\n\
             - description: 1-3 sentences expanding on the title, or null if the title is self-explanatory\n\
             - Do not duplicate an existing work item; differentiate the title if the topic overlaps\n\
             - Do not include any text outside the JSON object",
            recent_context = recent_context,
            existing_items = existing_items,
            raw_args = trigger.raw_args,
        ),
    }]
}
```

The prompt includes existing work items to prevent duplicates. It uses XML-style section tags for
clear delimiter boundaries, following Anthropic's guidance on context engineering. The "ONLY a JSON
object" instruction and explicit format minimize the chance of non-parseable responses.

### Typed Result Structs

Every tool extraction result is a typed struct. No `serde_json::Value` anywhere in the pipeline.

```rust
/// Raw LLM response for ~plan extraction. Deserialized directly from JSON.
#[derive(Debug, Deserialize)]
pub struct PlanExtractionResponse {
    pub title: String,
    pub description: Option<String>,
}

/// Validated, ready-to-apply result of a tool invocation.
/// Each variant carries all data needed for the main loop to mutate the graph.
#[derive(Debug, Clone)]
pub enum ToolResult {
    Plan(PlanResult),
    // Future: Search(SearchResult), Summarize(SummaryResult), ...
}

#[derive(Debug, Clone)]
pub struct PlanResult {
    pub title: String,
    pub description: Option<String>,
    /// The user message that contained the ~plan trigger.
    pub source_message_id: Uuid,
    /// Token usage for cost tracking.
    pub input_tokens: Option<u32>,
    pub output_tokens: Option<u32>,
}
```

### Extraction Execution

The spawn function ties together semaphore acquisition, LLM call, response parsing, and result
delivery:

```rust
pub fn spawn_tool_extraction(
    trigger: ToolTrigger,
    snapshot: ContextSnapshot,
    provider: Arc<dyn LlmProvider>,
    semaphore: Arc<Semaphore>,
    bg_config: BackgroundLlmConfig,
    task_tx: UnboundedSender<TaskMessage>,
) {
    tokio::spawn(async move {
        // Acquire semaphore permit -- waits if at capacity
        let _permit = semaphore.acquire().await.expect("semaphore closed");

        let result = match trigger.tool {
            InlineTool::Plan => {
                extract_plan(&trigger, &snapshot, &*provider, &bg_config).await
            }
        };

        let msg = match result {
            Ok(tool_result) => TaskMessage::ToolResult(tool_result),
            Err(e) => TaskMessage::ToolError {
                tool: trigger.tool.name().to_string(),
                error: e.to_string(),
            },
        };

        // Ignore send error: receiver dropped means the app is shutting down
        let _ = task_tx.send(msg);
    });
}

async fn extract_plan(
    trigger: &ToolTrigger,
    snapshot: &ContextSnapshot,
    provider: &dyn LlmProvider,
    bg_config: &BackgroundLlmConfig,
) -> Result<ToolResult> {
    let messages = build_plan_extraction_prompt(trigger, snapshot);
    let config = ChatConfig {
        model: bg_config.model.clone(),
        max_tokens: bg_config.max_tokens,
        system_prompt: None,
    };

    let response = background_llm_call(provider, messages, &config).await?;

    let parsed: PlanExtractionResponse = serde_json::from_str(&response.text)
        .map_err(|e| {
            anyhow::anyhow!(
                "Failed to parse plan extraction response: {e}\nRaw response: {}",
                response.text
            )
        })?;

    Ok(ToolResult::Plan(PlanResult {
        title: parsed.title,
        description: parsed.description,
        source_message_id: snapshot.trigger_node_id,
        input_tokens: response.input_tokens,
        output_tokens: response.output_tokens,
    }))
}
```

### Error Handling

LLM extraction can fail in several ways. Each is handled explicitly:

| Failure Mode | Handling |
|---|---|
| Semaphore closed | `expect` panic -- only happens during shutdown, not recoverable |
| Network error / API error from `provider.chat()` | Propagated as `Err`, becomes `TaskMessage::ToolError` |
| Non-JSON response from LLM | `serde_json::from_str` fails, becomes `TaskMessage::ToolError` |
| JSON with wrong shape (missing fields) | `serde_json::from_str` fails, becomes `TaskMessage::ToolError` |
| Valid JSON but nonsensical content | Accepted -- title/description are best-effort human-readable strings |
| `task_tx.send()` fails | Silently ignored -- app is shutting down |

The main loop's `handle_task_message` displays `ToolError` in the status bar. The user sees the
error and can retry by sending the `~plan` message again with better phrasing or more context.
There are no automatic retries -- they burn budget without user intent, and the user can provide
better input on a manual retry.

---

## Work Tab

### New Context Tab

The existing `ContextTab` enum gains a `Work` variant:

```rust
pub enum ContextTab {
    Outline,
    Files,
    Tools,
    Tasks,
    Work, // new
}
```

The tab header bar becomes: `Outline | Files | Tools | Tasks | Work`. Tab-cycling order follows
the enum variant order.

### Work Tab Layout

The Work tab displays WorkItem nodes from the graph, grouped by status:

```
+-- Work ----------------------------------------+
|                                                 |
|  ACTIVE                                         |
|  > Fix authentication module                    |
|    Created 2m ago                                |
|    Harden the auth middleware against token      |
|    replay attacks and add refresh rotation.      |
|                                                 |
|  TODO                                           |
|    Refactor payment validation                  |
|    Add rate limiting to API endpoints           |
|                                                 |
|  DONE (2 items)                                 |
|    Set up CI pipeline                           |
|    Configure deployment secrets                 |
|                                                 |
+-------------------------------------------------+
```

**Display rules:**

- **Active** items are expanded: title + age + description (if present).
- **Todo** items show title only.
- **Done** items are collapsed with a count, titles listed but dimmed.
- Groups with zero items are hidden.
- Items within each group are sorted by `created_at`, newest first.

### WorkItem Creation via Graph Mutation

When `handle_task_message` receives `TaskMessage::ToolResult(ToolResult::Plan(plan))`:

1. Create a `Node::WorkItem` with `status: WorkItemStatus::Todo`, the extracted `title` and
   `description`, and a new UUID.
2. Add an `Edge` with `kind: EdgeKind::RelevantTo` from the new WorkItem node to
   `plan.source_message_id` (the user message that triggered `~plan`).
3. Persist to disk via `save_conversation`.
4. Update the status bar: `"Created work item: {title}"`.
5. If the Work tab is currently focused, trigger a re-render.

Status transitions (Todo -> Active -> Done) are driven by future keyboard shortcuts in the Work
tab. That interaction design is out of scope for this document.

---

## Integration with Existing Systems

### Changes to App Struct

```rust
pub struct App {
    config: AppConfig,
    graph: ConversationGraph,
    metadata: ConversationMetadata,
    provider: Arc<dyn LlmProvider>,          // changed from Box<dyn LlmProvider>
    bg_semaphore: Arc<Semaphore>,            // new
    bg_llm_config: BackgroundLlmConfig,      // new
    tui_state: TuiState,
    task_rx: mpsc::UnboundedReceiver<TaskMessage>,
    task_tx: mpsc::UnboundedSender<TaskMessage>,
}
```

Construction changes minimally:

```rust
impl App {
    pub fn new(config: AppConfig) -> Result<Self> {
        let provider: Arc<dyn LlmProvider> =
            Arc::new(AnthropicProvider::from_config(&config)?);
        let bg_llm_config = BackgroundLlmConfig::from_app_config(&config);
        let bg_semaphore = Arc::new(Semaphore::new(bg_llm_config.max_concurrent));
        // ... rest unchanged, provider usage is identical via Deref
    }
}
```

### New TaskMessage Variants

```rust
pub enum TaskMessage {
    // Existing
    GitFilesUpdated(Vec<GitFileInfo>),
    ToolsDiscovered(Vec<ToolInfo>),
    TaskStatusChanged { task_id: Uuid, status: String },

    // New
    ToolResult(ToolResult),
    ToolError { tool: String, error: String },
}
```

No new `BackgroundTaskKind` variant is needed. The `spawn_tool_extraction` function is the entry
point for tool extraction tasks and its result types (`ToolResult`, `ToolError`) are sufficient
for the main loop to determine what happened. If future observability requires tracking
in-progress background tasks (e.g., for a "pending tasks" indicator), a `BackgroundTaskKind::ToolExtraction`
variant can be added at that time.

### Changes to handle_send_message

The current flow is linear: user message -> build context -> stream LLM -> assistant message.

The new flow adds a branching step after creating the user node:

```rust
async fn handle_send_message(&mut self, input: String) -> Result<()> {
    // 1. Create user Message node (unchanged)
    let user_node_id = self.graph.add_message("user", &input, &self.tui_state.active_branch);

    // 2. Check for tool trigger (new)
    if let Some(trigger) = detect_trigger(&input) {
        let snapshot = self.graph.build_snapshot(
            &self.tui_state.active_branch,
            10, // last 10 messages for tool extraction context
            user_node_id,
            true,  // include tools
            true,  // include work items
        )?;

        spawn_tool_extraction(
            trigger,
            snapshot,
            Arc::clone(&self.provider),
            Arc::clone(&self.bg_semaphore),
            self.bg_llm_config.clone(),
            self.task_tx.clone(),
        );

        self.set_status("Extracting tool parameters...");
    }

    // 3. Build context and stream LLM response (unchanged)
    let context = self.build_context()?;
    let config = ChatConfig::from_app_config(&self.config);
    let mut stream = self.provider.chat(context, &config).await?;

    // 4. Stream response and create assistant node (unchanged)
    // ...
}
```

The tool extraction runs concurrently with the main conversation response. The user sees the
assistant's reply streaming in while the background task extracts structured data. When the
extraction completes (typically 1-3 seconds later), the main loop picks up the `TaskMessage` on
the next `tokio::select!` cycle and applies the graph mutation.

### Changes to handle_task_message

```rust
fn handle_task_message(&mut self, msg: TaskMessage) {
    match msg {
        // ... existing handlers unchanged ...

        TaskMessage::ToolResult(result) => match result {
            ToolResult::Plan(plan) => {
                let item_id = self.graph.add_node(Node::WorkItem(WorkItem {
                    id: Uuid::new_v4(),
                    title: plan.title.clone(),
                    status: WorkItemStatus::Todo,
                    description: plan.description.clone(),
                    created_at: Utc::now(),
                }));
                self.graph.add_edge(Edge {
                    from: item_id,
                    to: plan.source_message_id,
                    kind: EdgeKind::RelevantTo,
                });
                self.set_status(format!("Created work item: {}", plan.title));
                self.save();
            }
        },

        TaskMessage::ToolError { tool, error } => {
            self.set_status(format!("Tool '{}' failed: {}", tool, error));
        }
    }
}
```

### Context Summarization Evolution

The existing `spawn_context_summarization` is a stub that does nothing. With the background LLM
infrastructure in place, it becomes a real implementation following the exact same pattern:

```rust
pub fn spawn_context_summarization(
    snapshot: ContextSnapshot, // contains the message range to summarize
    provider: Arc<dyn LlmProvider>,
    semaphore: Arc<Semaphore>,
    bg_config: BackgroundLlmConfig,
    task_tx: UnboundedSender<TaskMessage>,
) {
    tokio::spawn(async move {
        let _permit = semaphore.acquire().await.expect("semaphore closed");

        let messages = build_summarization_prompt(&snapshot);
        let config = ChatConfig {
            model: bg_config.model.clone(),
            max_tokens: bg_config.max_tokens,
            system_prompt: None,
        };

        let result = background_llm_call(&*provider, messages, &config).await;

        let msg = match result {
            Ok(response) => {
                // Parse typed summary response
                // TaskMessage::SummarizationComplete { ... }
                todo!("implement when summarization is activated")
            }
            Err(e) => TaskMessage::ToolError {
                tool: "summarize".to_string(),
                error: e.to_string(),
            },
        };

        let _ = task_tx.send(msg);
    });
}
```

The pattern is identical to tool extraction: snapshot in, semaphore acquire, LLM call, typed
parse, TaskMessage out. Only the prompt construction and result struct differ. This uniformity
is the payoff of establishing background LLM calls as a foundational pattern rather than a
one-off feature.

---

## Future Extensibility

### Adding New Inline Tools

Adding a new tool (e.g., `~search`) requires exactly four changes:

1. **Add enum variant**: `InlineTool::Search` in `src/tools.rs`
2. **Add parse match**: `"search" => Some(Self::Search)` in `InlineTool::from_name`
3. **Add extraction function**: `async fn extract_search(...)` with its own prompt and typed
   `SearchExtractionResponse` struct
4. **Add result handling**: new arm in `handle_task_message` for `ToolResult::Search(result)`

No new traits, no new registries, no new message channel types. The enum enforces exhaustive
matching via `match`, so the compiler rejects any build where a tool variant is not handled in
all relevant locations. This is the primary advantage over a trait-object or function-map
registry: the compiler is the enforcement mechanism.

### Agent Loops

An agent loop is a background task that makes multiple LLM calls in sequence, each informed by
the result of the previous call. The infrastructure supports this directly because the semaphore
operates per-call, not per-task:

```rust
async fn agent_loop(
    provider: Arc<dyn LlmProvider>,
    semaphore: Arc<Semaphore>,
    bg_config: BackgroundLlmConfig,
    initial_snapshot: ContextSnapshot,
    task_tx: UnboundedSender<TaskMessage>,
) {
    let mut accumulated_context = initial_snapshot;

    for step in 0..MAX_AGENT_STEPS {
        // Acquire permit for THIS step only
        let _permit = semaphore.acquire().await.expect("semaphore closed");

        let response = background_llm_call(/* ... */).await;
        // ... process response, update accumulated_context ...

        // _permit drops here, releasing the semaphore for other tasks
        // between steps. This prevents a single agent loop from
        // monopolizing the LLM.
    }

    // Send final result
    let _ = task_tx.send(TaskMessage::AgentLoopComplete { /* ... */ });
}
```

The semaphore naturally interleaves agent steps with other background work. Each step acquires
and releases a permit, preventing a runaway loop from starving other tasks. The
`MAX_AGENT_STEPS` constant provides a hard ceiling.

### Multi-Step Tool Chains

A tool result could trigger another tool. For example, `~plan` creates a WorkItem, and a future
`~decompose` breaks a WorkItem into subtasks. The chaining mechanism requires no special
infrastructure:

```
handle_task_message(ToolResult::Plan(plan))
  |
  +--> create WorkItem node
  |
  +--> (future) check if auto-decompose is enabled
  |       |
  |       +--> build new ContextSnapshot including the new WorkItem
  |       |
  |       +--> spawn_tool_extraction(decompose_trigger, snapshot, ...)
```

The main loop is the orchestrator. Background tasks produce results. The main loop decides what
to do next. This keeps control flow linear and debuggable -- there is no hidden state machine
in the background tasks, and the full chain of events is visible in the `handle_task_message`
match arms.

### Parallel Tool Execution

If a future scenario requires multiple background LLM calls simultaneously (e.g., auto-scoring
relevance of 5 candidate files), multiple tasks are spawned and the semaphore limits concurrency
automatically:

```
spawn task A ---> semaphore.acquire() ---> runs
spawn task B ---> semaphore.acquire() ---> runs
spawn task C ---> semaphore.acquire() ---> waits (2 permits taken)
                  task A completes, drops permit
                  task C acquires permit ---> runs
spawn task D ---> semaphore.acquire() ---> waits
                  ...
```

No additional coordination is needed. The semaphore is the only synchronization primitive. Tasks
are independent and communicate results solely through `TaskMessage`.

### Cost Runaway Prevention

The semaphore is the first line of defense: at most `max_concurrent` (default 2) background LLM
calls can execute simultaneously. But the semaphore alone does not prevent cost runaway from a
long queue of tasks.

Future additions (out of scope for this document, noted for architectural awareness):

- **Token budget per conversation**: Track cumulative background token usage in `App`. Refuse to
  spawn new background tasks when the budget is exhausted. Display remaining budget in the TUI.
- **Rate limiting**: Enforce a maximum number of background LLM calls per minute, independent of
  the semaphore (which controls concurrency, not rate).
- **Cost display**: Show cumulative input/output token counts and estimated cost in the status bar
  or a dedicated metrics panel.
- **Model fallback**: If the configured background model is unavailable, fall back to a cheaper
  alternative rather than failing.

For now, the semaphore combined with the manual nature of `~tool` triggers (users must type them
explicitly) provides sufficient protection. Automated background tasks (like summarization
triggered by message count thresholds) will need budget controls before being enabled by default.

### Tool Call Infrastructure (see Design 03)

The background LLM infrastructure established here — `Arc<dyn LlmProvider>`, semaphore,
`TaskMessage` channels, `ContextSnapshot` — is the foundation for the tool call system described
in [03-tool-call-foundation.md](./03-tool-call-foundation.md). That design extends `TaskMessage`
with `ToolCallDispatched`/`ToolCallCompleted` variants, adds `ToolCall`/`ToolResult` graph nodes
with `Invoked`/`Produced` edges, and integrates Anthropic `tool_use` API support. The channel
architecture, snapshot pattern, and semaphore-based concurrency control carry forward unchanged.

---

## Trade-offs and Alternatives

### Snapshot Staleness vs Shared Mutable State

| Approach | Consistency | Complexity | Deadlock Risk | Contention |
|---|---|---|---|---|
| **Snapshot (chosen)** | Eventual | Low | None | None |
| `Arc<RwLock<Graph>>` | Strong | High | Real | Lock contention on every read/write |
| Request/response channels | Strong | Very high | Low | Channel queueing delay |

Snapshot staleness is acceptable for all current and planned use cases. If a future use case
requires strong consistency with the graph (difficult to imagine in a TUI application where the
user is the primary source of graph mutations), a targeted request/response channel can be added
for that specific case without changing the general pattern.

### No Streaming for Background Tasks

Background tasks collect the full LLM response before processing it. Alternatives considered:

- **Stream to a buffer with progress updates**: Adds complexity (progress tracking, partial
  parse attempts) for marginal UX benefit. The user is not watching background task output
  character by character.
- **Stream to a log/debug panel**: Useful for development debugging but not worth the plumbing
  for the initial implementation. Can be added later by replacing `background_llm_call` with a
  streaming variant that also writes to a ring buffer, without changing the task structure.

### Trigger Parsing: Simple String vs Regex vs LLM

| Approach | Latency | Cost | Reliability | Flexibility |
|---|---|---|---|---|
| **Simple string (chosen)** | ~0ns | $0 | Deterministic, 100% | Fixed syntax |
| Regex (`regex` crate) | ~0ns | $0 | Deterministic | More expressive patterns |
| LLM-based detection | 500ms-3s | $$$ | Probabilistic, ~95% | Natural language triggers |

Simple string parsing is correct here. The trigger syntax is a deliberate UI convention, not
natural language understanding. Users learn `~plan` just as they learn `/slash` commands in Slack
or Discord. Adding regex later (e.g., for `~plan[high]` priority syntax or `~plan @alice`
assignment syntax) is a backward-compatible extension that does not change the architecture.

LLM-based trigger detection is categorically wrong for this use case: it adds latency, cost, and
nondeterminism to something that must be instant and reliable. Every message would require an LLM
round-trip just to determine if it contains a tool trigger -- an absurd tax on normal
conversation.

### Tool Registry: Enum vs Trait Objects vs Function Map

| Approach | Type Safety | Extensibility | Boilerplate | Runtime Cost |
|---|---|---|---|---|
| **Enum (chosen)** | Exhaustive matching | Add variant + match arms | Low for <20 tools | Zero (static dispatch) |
| Trait objects | Runtime dispatch | Add struct + impl Trait | High (trait def, Box, registration) | vtable indirection |
| `HashMap<String, fn>` | None | Add entry | Low | Hash lookup |

The enum is correct for a small, compile-time-known set of tools. The inline tool set will not
grow large -- users will not memorize more than ~10 tilde commands, and the UI should surface
them via help/autocomplete rather than expecting memorization. If the tool count somehow exceeds
~20, the migration path to trait objects is straightforward: the enum becomes a thin dispatch
layer over trait objects, preserving the exhaustive-match property at the top level.

### Tool Triggers Coexisting with Conversation

The user's `~plan fix auth` message goes to both the background extraction AND the main
conversation LLM. Alternatives:

- **Strip the trigger before sending to main LLM**: The conversation becomes incoherent. The
  assistant responds to a message that does not match what the user typed. The conversation
  history shows the original text but the assistant's response addresses something different.
- **Replace trigger with a system note**: Adds prompt engineering complexity for marginal benefit.
  The main LLM needs to understand a synthetic notation it was not trained on.
- **Suppress main LLM call entirely**: The user gets no conversational response, only a background
  task. This breaks the expectation that sending a message produces a reply.
- **Send as-is (chosen)**: The main LLM sees `~plan fix auth` and responds naturally. It might
  say "I'll help you plan that" or "Good idea to track that as a work item." The tilde syntax is
  transparent to the LLM -- it processes it as ordinary text. The conversation remains coherent
  and the user gets both a conversational acknowledgment and a structured tool result.

---

## Implementation Phases

### Phase 1: Shared Provider and Background Infrastructure

**Files**: `src/app.rs`, `src/config.rs`, `src/llm/mod.rs`, `src/graph.rs`, `src/tasks.rs`

- Change `Box<dyn LlmProvider>` to `Arc<dyn LlmProvider>` in `App`
- Add `BackgroundLlmConfig` struct to `src/config.rs`
- Add `Semaphore` field to `App`, initialized from `BackgroundLlmConfig::max_concurrent`
- Implement `background_llm_call()` and `BackgroundLlmResponse` in `src/llm/mod.rs`
- Add `ContextSnapshot` and supporting structs to `src/graph.rs`
- Implement `ConversationGraph::build_snapshot()`
- Add `ToolResult` and `ToolError` variants to `TaskMessage` in `src/tasks.rs`

### Phase 2: Inline Tool Parsing and Extraction

**Files**: `src/tools.rs` (new), `src/app.rs`, `src/tasks.rs`

- Create `src/tools.rs` with `InlineTool`, `ToolTrigger`, `detect_trigger()`
- Implement `PlanExtractionResponse`, `PlanResult`, `ToolResult` enum
- Implement `build_plan_extraction_prompt()` and `extract_plan()`
- Implement `spawn_tool_extraction()`
- Wire `detect_trigger()` into `handle_send_message`
- Wire `ToolResult::Plan` handling into `handle_task_message`
- Wire `ToolError` handling into `handle_task_message`

### Phase 3: Work Tab

**Files**: `src/tui/mod.rs`, `src/tui/context.rs`

- Add `ContextTab::Work` variant
- Implement Work tab rendering (grouped by status, active expanded, done collapsed)
- Update tab cycling to include Work tab
- Ensure WorkItem creation triggers re-render when Work tab is focused

### Phase 4: Context Summarization (Stub to Real)

**Files**: `src/tasks.rs`, `src/app.rs`

- Replace `spawn_context_summarization` stub with real implementation using `background_llm_call`
- Define `SummarizationResponse` typed struct for LLM output
- Add `SummarizationComplete` variant to `TaskMessage`
- Determine trigger conditions (message count threshold, context token threshold)
- Wire summarization results into graph (create summary SystemDirective nodes)

---

## File Map

```
src/
  app.rs          -- Arc<dyn LlmProvider>, Semaphore, handle_send_message changes,
                     handle_task_message new arms
  config.rs       -- BackgroundLlmConfig struct and defaults
  graph.rs        -- ContextSnapshot, SnapshotMessage, SnapshotTool, SnapshotWorkItem,
                     ConversationGraph::build_snapshot()
  tools.rs        -- NEW: InlineTool, ToolTrigger, detect_trigger(), ToolResult,
                     PlanExtractionResponse, PlanResult, build_plan_extraction_prompt(),
                     extract_plan(), spawn_tool_extraction()
  tasks.rs        -- TaskMessage::ToolResult, TaskMessage::ToolError
  llm/
    mod.rs        -- background_llm_call(), BackgroundLlmResponse
    anthropic.rs  -- (unchanged)
  tui/
    mod.rs        -- ContextTab::Work added to enum
    context.rs    -- Work tab rendering logic
```
