# MVP.md — Context Manager v0.1

> The most minimal version that proves graph-based context works.

**Status:** Specification
**Target:** 1-2 weeks for one developer
**Prerequisite:** `ANTHROPIC_API_KEY` environment variable

---

## 1. What the MVP Proves

The MVP proves exactly one graph advantage over linear chat: **conversation branching**.

A user can fork a conversation at any point, explore a different direction with the LLM, and switch between branches. Each branch gets exactly its own history — no contamination from sibling branches. This is impossible in linear chat and trivial with a graph.

Why branching over pinning, compaction, or relevance scoring:
- Branching is a **pure graph operation** with no LLM overhead beyond the conversation itself.
- It requires only: creating a new edge from an existing node, and walking ancestors to build context.
- It is immediately useful and immediately visible to the developer.

Additionally, the MVP proves:
- Graph storage works for chat (every message is a node, every response is an edge).
- Context construction via graph traversal works (ancestor walk per branch).
- Real Claude API integration with streaming works.
- The architecture is extensible (LLM trait, graph structure, TUI layout all have clear extension points).

---

## 2. Scope

### 2.1 Node Types (2 only)

| Type | Fields |
|------|--------|
| `Message` | `id: Uuid`, `role: Role` (User/Assistant/System), `content: String`, `created_at: DateTime`, `model: Option<String>`, `token_count: Option<u32>` |
| `SystemDirective` | `id: Uuid`, `content: String`, `created_at: DateTime` |

Everything else from VISION.md (CompactedMessage, Requirement, WorkItem, ToolCall, ToolResult, Artifact, Rating) is deferred.

### 2.2 Edge Types (1 only)

| Type | Semantics |
|------|-----------|
| `responds_to` | Message B is a response to Message A |

The graph is a tree. Each user message `responds_to` the previous assistant message. Each assistant message `responds_to` the user message it answers. Branching creates a second child from the same parent node.

Everything else from VISION.md (compacted_from, requires, subtask_of, invoked, produced, relevant_to, pinned_by, supersedes) is deferred.

### 2.3 Context Construction (Simplified)

Three steps (from VISION.md's six):

1. **Anchor:** The node the user is currently replying to.
2. **Walk ancestors:** Follow `responds_to` edges backward to the root, collecting all messages on this branch.
3. **Render:** Serialize collected messages in chronological order as the `messages` array for the Claude API.

No expand, no compaction selection, no budget pruning. The full ancestor chain is sent. If it exceeds the context window, truncate from the oldest messages with a warning.

---

## 3. TUI Layout

```
┌─────────────────────────────────────────────────────┐
│ Context Manager v0.1              [branch: main]    │  <- Status bar
├──────────────┬──────────────────────────────────────┤
│              │                                      │
│  Branches    │  Conversation                        │
│              │                                      │
│  > main      │  [system] You are a helpful...       │
│    explore   │                                      │
│    refactor  │  [you] How do I parse JSON in Rust?  │
│              │                                      │
│              │  [assistant] You can use serde...     │
│              │                                      │
│              │                                      │
├──────────────┴──────────────────────────────────────┤
│ > Type your message...                              │  <- Input area
│                                                     │
│ Enter: send | Ctrl+B: branch | Up/Down: switch      │
│ branch | Ctrl+Q: quit                               │
└─────────────────────────────────────────────────────┘
```

**Left pane (narrow):** Branch list. Current branch highlighted. Navigate with Up/Down when the pane is focused.

**Center pane (wide):** Conversation for the active branch. Messages scroll. Streaming assistant response appears in real-time.

**Bottom:** Text input. Enter to send. Shift+Enter for newlines.

**Key bindings:**
- `Enter` — Send message
- `Ctrl+B` — Create branch from current position (prompts for name)
- `Up/Down` (in branch pane) — Switch branches
- `Tab` — Toggle focus between branch pane and input
- `Ctrl+Q` — Quit

### 3.1 What's NOT in the MVP TUI

- Context inspector pane (right pane showing what the model sees)
- Token budget display
- Work item tree
- Graph minimap or visualization
- Command palette
- Search
- Message pinning UI

---

## 4. LLM Integration

### 4.1 Provider Trait

```rust
#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn chat(
        &self,
        messages: Vec<ChatMessage>,
        config: &ChatConfig,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>>>;

    fn name(&self) -> &str;
}
```

### 4.2 MVP Implementation

`AnthropicProvider` only. Uses the Anthropic Messages API with streaming (SSE).

- Reads `ANTHROPIC_API_KEY` from environment.
- Model: `claude-sonnet-4-5-20250514` by default, configurable via `CONTEXT_MANAGER_MODEL` env var.
- Streaming: token-by-token into the TUI.

### 4.3 What's NOT in MVP LLM

- Multiple providers (OpenAI, DeepSeek, Ollama)
- Prompt caching
- Token counting (beyond what the API returns)
- Cost tracking
- Batch API
- Tool use / function calling

---

## 5. Persistence

### 5.1 Format: JSON on Disk

```
~/.context-manager/
  conversations/
    <conversation-id>/
      graph.json       # Nodes + edges + branches
      metadata.json    # Name, created_at, last_modified
```

### 5.2 graph.json Structure

```json
{
  "nodes": [
    {
      "id": "550e8400-...",
      "type": "Message",
      "role": "user",
      "content": "How do I parse JSON?",
      "created_at": "2026-03-11T10:00:00Z",
      "model": null,
      "token_count": null
    },
    {
      "id": "6ba7b810-...",
      "type": "Message",
      "role": "assistant",
      "content": "You can use serde...",
      "created_at": "2026-03-11T10:00:05Z",
      "model": "claude-sonnet-4-5-20250514",
      "token_count": 245
    }
  ],
  "edges": [
    {
      "from": "6ba7b810-...",
      "to": "550e8400-...",
      "type": "responds_to"
    }
  ],
  "branches": {
    "main": "6ba7b810-..."
  },
  "active_branch": "main"
}
```

A "branch" is a named pointer to a leaf node. Full history is reconstructed by walking `responds_to` edges from leaf to root.

### 5.3 Save Strategy

Save after every message exchange (user + assistant). Simple and safe.

### 5.4 What's NOT in MVP Persistence

- petgraph (HashMap is sufficient for tree-walking)
- Cozo or any embedded DB
- sled
- Storage trait abstraction (hardcode JSON; extract trait when second backend needed)
- Graph versioning / undo

---

## 6. In-Memory Graph

Plain data structures, no petgraph:

```rust
pub struct ConversationGraph {
    /// All nodes by ID
    nodes: HashMap<Uuid, Node>,
    /// child_id -> parent_id (the responds_to relationship)
    edges: HashMap<Uuid, Uuid>,
    /// branch_name -> leaf_node_id
    branches: HashMap<String, Uuid>,
    /// Currently active branch
    active_branch: String,
}
```

Key operations:
- **`get_branch_history(branch) -> Vec<&Node>`** — Walk from leaf to root via `edges`, reverse, return chronological.
- **`add_message(parent_id, node) -> Uuid`** — Insert node, add edge, update branch leaf pointer.
- **`create_branch(name, fork_point)`** — Add branch entry pointing at fork_point.
- **`get_children(node_id) -> Vec<Uuid>`** — Find nodes whose parent is node_id (for detecting branch points in UI).

Why not petgraph: its `NodeIndex` is unstable across serialization, and MVP graph operations (ancestor walk, child lookup) are trivially implementable with HashMap. petgraph earns its place when we need algorithms (PageRank, community detection, shortest path).

---

## 7. Project Structure

```
context-manager/
  Cargo.toml
  src/
    main.rs              # Entry point, tokio runtime, app loop
    app.rs               # App state, event handling, coordination
    graph.rs             # ConversationGraph, Node, Edge types
    persistence.rs       # Load/save JSON
    llm/
      mod.rs             # LlmProvider trait, ChatMessage, ChatConfig
      anthropic.rs       # AnthropicProvider with streaming SSE
    tui/
      mod.rs             # Terminal setup/teardown, event loop
      ui.rs              # Layout rendering (draw function)
      input.rs           # Key event handling
      widgets/
        branch_list.rs   # Left pane branch selector
        conversation.rs  # Center pane message display
        input_box.rs     # Bottom text input
```

### 7.1 Dependencies

```toml
[dependencies]
ratatui = "0.29"
crossterm = "0.28"
tokio = { version = "1", features = ["full"] }
reqwest = { version = "0.12", features = ["stream"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
uuid = { version = "1", features = ["v4", "serde"] }
chrono = { version = "0.4", features = ["serde"] }
anyhow = "1"
futures = "0.3"
async-trait = "0.1"
```

~11 crates. No more.

---

## 8. User Flow

1. **Launch:** `context-manager` starts. Shows list of existing conversations or `[New Conversation]`.
2. **New conversation:** User enters a name. Root `SystemDirective` node created with default system prompt.
3. **Chat:** User types message → `Message` node (role: User) created with `responds_to` edge → ancestor chain collected → sent to Claude API → streaming response displayed → `Message` node (role: Assistant) created → graph saved to disk.
4. **Branch:** `Ctrl+B` → user enters branch name → new branch created from current position → user is now on the new branch → next message diverges from the original conversation.
5. **Switch branch:** Up/Down in branch pane → conversation view updates to show that branch's history.
6. **Quit:** `Ctrl+Q`. State already saved.

---

## 9. Implementation Sequence

### Week 1: Foundations

| Day | Task |
|-----|------|
| 1 | Project skeleton: `cargo init`, dependencies, module structure. Graph data structures (`graph.rs`) with unit tests for ancestor walking, branching, child detection. |
| 2 | JSON persistence (`persistence.rs`) with round-trip tests. Load/save/list conversations. |
| 3 | Anthropic provider (`llm/anthropic.rs`) with SSE streaming. Test standalone (no TUI) — send a message, print streaming response. |
| 4-5 | Basic TUI shell: terminal setup, event loop, single conversation pane rendering hardcoded messages. Input box that captures text. |

### Week 2: Integration & Polish

| Day | Task |
|-----|------|
| 1-2 | Wire graph + LLM + TUI together in `app.rs`. Real chat works end-to-end: type message, see streaming response, graph persists. |
| 3 | Branch pane: render branch list, Up/Down navigation, branch switching updates conversation view. |
| 4 | `Ctrl+B` branching workflow: create branch from current position, name it, switch to it. |
| 5 | Conversation selection screen. Error display. Loading indicators. Scroll behavior. Edge cases. |

---

## 10. Explicit Non-Goals

Everything in VISION.md that is NOT in this MVP:

| Feature | Why deferred | VISION.md section |
|---------|-------------|-------------------|
| Message compaction | Requires background LLM calls, perspective system | 4.2 |
| Background processing | Requires async task scheduling, cost budgeting | 4.3 |
| Multi-rater relevance | Requires rating nodes, judge pipeline | 4.4 |
| Non-linear cell interface | Branching is sufficient for MVP; full cell model adds complexity | 4.5 |
| Work management | Requires new node types, edge types, UI panels | 4.6 |
| Pinning | Requires context construction changes, pin management UI | 4.7 |
| Tool call provenance | Requires MCP integration, tool result handling | 4.8 |
| Storage trait abstraction | Extract when second backend is needed | 5.2 |
| Multiple LLM providers | Trait is there; implementations are additive | 5.4 |
| Token budgeting | Requires tokenizer, budget algorithm | 3.2 |
| Context inspector pane | Useful but not required to prove the concept | 5.3 |
| External PM integration (Jira, Linear, etc.) | Work items are internal graph nodes only; no external sync | — |
| Bootstrapping from Git/issues | No existing project data to import in MVP | 6 |
| Graph visualization | Hard in TUI, not needed for branching | 5.3 |
| Web escape hatch | Requires web server, xterm.js | 5.3 |

---

## 11. Risks

| Risk | Severity | Mitigation |
|------|----------|------------|
| SSE streaming parsing is fiddly | Medium | Use `eventsource-stream` crate or hand-roll simple parser; test against real API on day 3 |
| ratatui state management gets complex | Medium | Keep all state in `App` struct; pass immutable refs to draw functions; avoid frameworks until needed |
| Long branches exceed Claude's context window | Low | Truncate oldest messages with a visible warning; acceptable for MVP |
| JSON persistence slow for large graphs | Low | Single conversation won't have >few hundred nodes; revisit when it matters |
| Branch naming collisions | Low | Auto-generate unique names (`branch-1`, `branch-2`); allow user override |

---

## 12. Success Criteria

The MVP is done when:

1. You can launch it, create a conversation, and chat with Claude via streaming responses.
2. You can press `Ctrl+B`, name a branch, and continue the conversation in a different direction.
3. You can switch between branches and see different conversation histories.
4. Conversations persist across restarts.
5. The graph structure is visible in `~/.context-manager/conversations/*/graph.json` and is human-readable.
