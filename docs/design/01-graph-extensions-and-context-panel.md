# Design: Graph Extensions, Context Panel, and Background Tasks

**Date:** 2026-03-11
**Status:** Approved

---

## 1. Motivation

The current application has a minimal graph model (2 node types, 1 implicit edge type) and a TUI that shows only conversation messages. To move toward the VISION.md goal of a context orchestration engine, we need to:

1. Extend the graph to support richer node types and typed edges
2. Add a context panel above the conversation to show other graph elements
3. Run background tasks that automatically populate the graph with environment data
4. Display all of this in a tabbed context panel

**Design principle**: Everything is a graph node. Git files, tools, work items, and background tasks all become nodes with typed edges.

**Non-goal**: No integration with external project management tools (Jira, Linear, GitHub Issues). Work items are tracked internally only. This is a deliberate constraint -- see `docs/MVP.md` for scope.

---

## 2. Graph Model Extensions

### 2.1 Typed Edges

The current edge model is a `HashMap<Uuid, Uuid>` representing an implicit `responds_to` relationship. This is replaced with explicitly typed edges.

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    RespondsTo,   // Message -> parent Message (conversation threading)
    SubtaskOf,    // WorkItem -> parent WorkItem (hierarchy)
    RelevantTo,   // any node -> WorkItem (topical relevance)
    Tracks,       // BackgroundTask -> target node it operates on
    Indexes,      // GitFile -> conversation root (what context it belongs to)
    Provides,     // Tool -> conversation root (available in this context)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    pub from: Uuid,
    pub to: Uuid,
    pub kind: EdgeKind,
}
```

**Storage**: The graph stores `Vec<Edge>` as the canonical edge list. A runtime index `responds_to_index: HashMap<Uuid, Uuid>` (not serialized) is rebuilt on load for fast ancestor walking. The hot path (`get_branch_history`) uses this index unchanged.

### 2.2 New Node Types

Four new variants are added to the `Node` enum:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Node {
    // Existing
    Message {
        id: Uuid,
        role: Role,
        content: String,
        created_at: DateTime<Utc>,
        model: Option<String>,
        input_tokens: Option<u32>,
        output_tokens: Option<u32>,
    },
    SystemDirective {
        id: Uuid,
        content: String,
        created_at: DateTime<Utc>,
    },

    // New
    WorkItem {
        id: Uuid,
        title: String,
        status: WorkItemStatus,
        description: Option<String>,
        created_at: DateTime<Utc>,
    },
    GitFile {
        id: Uuid,
        path: String,
        status: GitFileStatus,
        updated_at: DateTime<Utc>,
    },
    Tool {
        id: Uuid,
        name: String,
        description: String,
        updated_at: DateTime<Utc>,
    },
    BackgroundTask {
        id: Uuid,
        kind: BackgroundTaskKind,
        status: TaskStatus,
        description: String,
        created_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
    },
}
```

**Supporting enums**:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemStatus { Todo, Active, Done }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GitFileStatus { Tracked, Modified, Staged, Untracked }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BackgroundTaskKind { GitIndex, ContextSummarize, ToolDiscovery }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus { Pending, Running, Completed, Failed }
```

### 2.3 New Graph Methods

```rust
/// Insert a node without any edges.
pub fn add_node(&mut self, node: Node) -> Uuid

/// Add a typed edge between two existing nodes.
pub fn add_edge(&mut self, from: Uuid, to: Uuid, kind: EdgeKind) -> Result<()>

/// Return all nodes matching a predicate.
pub fn nodes_by_type<F: Fn(&Node) -> bool>(&self, filter: F) -> Vec<&Node>

/// Return all branch names.
pub fn branch_names(&self) -> Vec<&str>

/// Insert or update a node. Creates if absent, replaces if present.
pub fn upsert_node(&mut self, node: Node)

/// Remove all nodes (and their edges) matching a predicate. Used for pruning stale git files.
pub fn remove_nodes_by<F: Fn(&Node) -> bool>(&mut self, filter: F)
```

### 2.4 Impact on Existing Code

- `Node::id()`, `Node::content()`: add match arms for new variants (`content` returns `title` for `WorkItem`, `path` for `GitFile`, `name` for `Tool`, `description` for `BackgroundTask`)
- `Node::input_tokens()`, `Node::output_tokens()`: return `None` for all non-`Message` variants
- `build_context` in `app.rs`: skip non-Message/non-SystemDirective nodes (`_ => continue`)
- `conversation.rs` widget: new node types won't appear in branch history (they don't use `responds_to` edges), so no rendering changes needed

---

## 3. Graph Versioning and Migration

### 3.1 Version Scheme

Each graph version is a separate fully-typed serde struct. Deserialization uses a tagged union on the `version` field. V1 files (which have no `version` field) are detected by failing to deserialize as the tagged union and falling back to `V1Graph` directly.

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "version")]
pub enum VersionedGraph {
    #[serde(rename = "2")]
    V2(V2Graph),
}
```

Version detection attempts to deserialize as `VersionedGraph` first. If it fails (V1 has no `version` field), the version is assumed to be 1.

### 3.2 Version Structs

**V1** (the current format -- no `version` field in existing files):

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct V1Graph {
    pub nodes: HashMap<Uuid, V1Node>,
    pub edges: HashMap<Uuid, Uuid>,       // child -> parent (implicit responds_to)
    pub branches: HashMap<String, Uuid>,
    pub active_branch: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum V1Node {
    Message {
        id: Uuid,
        role: Role,
        content: String,
        created_at: DateTime<Utc>,
        model: Option<String>,
        input_tokens: Option<u32>,
        output_tokens: Option<u32>,
    },
    SystemDirective {
        id: Uuid,
        content: String,
        created_at: DateTime<Utc>,
    },
}
```

**V2** (typed edges + new node types):

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct V2Graph {
    pub nodes: HashMap<Uuid, Node>,       // Node enum with all 6 variants
    pub edges: Vec<Edge>,                 // typed edges
    pub branches: HashMap<String, Uuid>,
    pub active_branch: String,
}
```

### 3.3 Migration Flow

1. Read `graph.json` as a string
2. Try to deserialize as `VersionedGraph` (tagged union on `version` field)
3. If successful (current version), extract `V2Graph` and convert to `ConversationGraph`
4. If deserialization fails (V1 files have no `version` field):
   a. Copy `graph.json` to `graph.v1.json.bak` (backup before migration)
   b. Deserialize as `V1Graph`
   c. Run the migration: `V1Graph` -> `V2Graph`
   d. Wrap in `VersionedGraph::V2` and write back to `graph.json`
5. Convert `V2Graph` to the live `ConversationGraph` via a `GraphRaw` intermediate struct

### 3.4 V1 -> V2 Migration

```rust
fn migrate_v1_to_v2(v1: V1Graph) -> V2Graph {
    let nodes = v1.nodes.into_iter()
        .map(|(id, n)| (id, v1_node_to_node(n)))
        .collect();
    let edges = v1.edges.into_iter()
        .map(|(child, parent)| Edge {
            from: child,
            to: parent,
            kind: EdgeKind::RespondsTo,
        })
        .collect();
    V2Graph {
        nodes,
        edges,
        branches: v1.branches,
        active_branch: v1.active_branch,
    }
}

fn v1_node_to_node(v1: V1Node) -> Node {
    match v1 {
        V1Node::Message { id, role, content, created_at, model, input_tokens, output_tokens } =>
            Node::Message { id, role, content, created_at, model, input_tokens, output_tokens },
        V1Node::SystemDirective { id, content, created_at } =>
            Node::SystemDirective { id, content, created_at },
    }
}
```

The live `ConversationGraph` struct corresponds to the latest version (V2). It has the same fields as `V2Graph` plus the runtime `responds_to_index` rebuilt on load.

---

## 4. TUI Context Panel

### 4.1 Layout

```
+------------------------------------------+
| Status bar (1 line)                       |
+------------------------------------------+
| CONTEXT PANEL (Percentage(30), collapse)  |
| [Outline|Files|Tools|Tasks]  <- Tabs      |
| ... list items for active tab ...         |
+------------------------------+-----------+
|          Left 75%            | Right 25% |
|   (active tab content)       | (minimap) |
+------------------------------+-----------+
| CONVERSATION (Min(5))                     |
| ... messages with Block borders ...       |
+------------------------------------------+
| INPUT BOX (Length(5))                     |
+------------------------------------------+
```

**Graceful degradation**: The context panel is hidden when terminal height < 20 rows.

**Collapsible**: `Ctrl+B` toggles panel visibility. When hidden, the conversation panel fills the space.

### 4.2 Two-Column Split

Inside the context panel:
- **Left (75%)**: Content for the active tab
- **Right (25%)**: Compact conversation outline (always visible as a minimap)

### 4.3 Tabs

Using ratatui's `Tabs` widget: `Outline | Files | Tools | Tasks`

| Tab | Content | Data Source |
|-----|---------|-------------|
| **Outline** | Branch list with message counts, session token stats | `ConversationGraph::branch_names()`, `get_branch_history()` |
| **Files** | One line per git file: `[M] src/app.rs` | `GitFile` nodes in graph |
| **Tools** | One line per tool: `web_search  Search the web` | `Tool` nodes in graph |
| **Tasks** | Status indicator per task: `[running] Git indexing...` | `BackgroundTask` nodes in graph |

### 4.4 Right Column: Message Minimap

Always visible regardless of active tab. One line per message in the active branch:

```
U  How do I parse JSON in R...
A  You can use serde_json...
S  You are a helpful assist...
```

Role prefix styled with conversation colors (Cyan=User, Green=Assistant, DarkGray=System).

### 4.5 State Extensions

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusPanel { Input, ContextPanel }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextTab { Outline, Files, Tools, Tasks }
```

New fields in `TuiState`:
- `focus: FocusPanel` (default: `Input`)
- `context_panel_visible: bool` (default: `true`)
- `context_tab: ContextTab` (default: `Outline`)
- `context_list_offset: usize` (scroll offset for tab content list)

### 4.6 Key Bindings

| Key | Context | Action |
|-----|---------|--------|
| `Tab` | Any | Cycle focus: Input -> ContextPanel -> Input |
| `Left`/`Right` | focus=ContextPanel | Cycle tabs |
| `Up`/`Down` | focus=ContextPanel | Scroll tab content list |
| `Up`/`Down` | focus=Input | Scroll conversation (existing) |
| `Ctrl+B` | Any | Toggle context panel visibility |

### 4.7 Focus Indication

- Focused panel: Yellow border
- Unfocused panel: DarkGray border

---

## 5. Background Task Infrastructure

### 5.1 Architecture

Background tasks run as tokio tasks. They communicate with the main app via a `tokio::sync::mpsc` channel. The app event loop polls this channel alongside terminal events and LLM stream chunks.

```
+------------------+     mpsc channel     +------------------+
| Background Tasks | ------------------> | App Event Loop   |
| (tokio::spawn)   |   TaskMessage        | (tokio::select!) |
+------------------+                      +------------------+
```

### 5.2 Message Types

```rust
pub enum TaskMessage {
    GitFilesUpdated(Vec<GitFileSnapshot>),
    ToolsDiscovered(Vec<ToolSnapshot>),
    TaskStatusChanged { task_id: Uuid, kind: BackgroundTaskKind, status: TaskStatus, description: String },
    SummarizationComplete { node_id: Uuid, summary: String },
}

pub struct GitFileSnapshot {
    pub path: String,
    pub status: GitFileStatus,
}

pub struct ToolSnapshot {
    pub name: String,
    pub description: String,
}
```

### 5.3 Git File Indexing

- Uses the [`git2`](https://crates.io/crates/git2) crate (libgit2 bindings, v0.20.4) -- no shelling out to git commands
- Opens repo with `git2::Repository::open()`, reads file statuses via `repo.statuses(None)` which returns `Statuses` with per-file `StatusEntry` (index status, worktree status, path)
- Maps `git2::Status` flags to `GitFileStatus`: `INDEX_NEW`/`INDEX_MODIFIED` -> `Staged`, `WT_MODIFIED` -> `Modified`, `WT_NEW` -> `Untracked`, else -> `Tracked`
- Triggered by filesystem events via [`notify`](https://crates.io/crates/notify) crate with [`notify-debouncer-mini`](https://crates.io/crates/notify-debouncer-mini) for deduplication (a single file save generates 3-5 events on Linux/inotify)
- Watches the working tree for changes; **event-based only, no periodic polling**
- On update: replaces all `GitFile` nodes in the graph (remove old, insert new)
- Creates a `BackgroundTask` node with `kind: GitIndex` to track status
- New dependencies: `git2`, `notify`, `notify-debouncer-mini`

### 5.4 Tool Discovery

- Initially: reads from a config file or hardcoded tool list
- Future: MCP tool discovery
- Triggered on startup only; re-triggered by config file change via `notify` watcher
- On update: replaces all `Tool` nodes in the graph
- Creates a `BackgroundTask` node with `kind: ToolDiscovery`

### 5.5 Context Summarization

- Triggers: when conversation exceeds a threshold (e.g., >20 messages on active branch)
- Uses the LLM provider to summarize older messages
- **Stubbed initially**: creates the task node but does not make LLM calls yet
- Creates a `BackgroundTask` node with `kind: ContextSummarize`

### 5.6 Task Node Lifecycle

1. Background task starts -> create `BackgroundTask` node with `status: Running`
2. Task completes -> update node to `status: Completed`
3. Task fails -> update node to `status: Failed`
4. Completed/failed tasks are removed after a configurable TTL (or kept until next run)

### 5.7 App Event Loop Extension

```rust
// In app.rs run() loop:
tokio::select! {
    maybe_event = event_stream.next() => { /* existing terminal event handling */ }
    maybe_task_msg = task_rx.recv() => {
        if let Some(msg) = maybe_task_msg {
            self.handle_task_message(msg)?;
        }
    }
}
```

---

## 6. Files to Modify/Create

| File | Change |
|------|--------|
| `src/graph.rs` | Add `EdgeKind`, `Edge`, new `Node` variants, supporting enums, typed edge storage, `responds_to_index`, new methods |
| `src/persistence.rs` | Add `VersionedGraph`, `V1Graph`, `V1Node`, `V2Graph` structs; version detection; backup-before-migrate; migration chain |
| `src/tui/mod.rs` | Add `FocusPanel`, `ContextTab` enums; extend `TuiState` with focus, panel visibility, tab, scroll fields |
| `src/tui/ui.rs` | 4-row vertical layout; graceful degradation for small terminals |
| `src/tui/widgets/mod.rs` | Add `pub mod context_panel;` |
| `src/tui/widgets/context_panel.rs` | **New.** Tabbed panel with 75/25 column split, tab content renderers, message minimap |
| `src/tui/input.rs` | Focus-aware key routing; new `Action` variants (`CycleFocus`, `ToggleContextPanel`, `ContextTabNext`, `ContextTabPrev`) |
| `src/app.rs` | Handle new actions; poll `mpsc` channel in event loop; `handle_task_message` method |
| `src/tasks.rs` | **New.** `TaskMessage` enum, `GitFileSnapshot`, `ToolSnapshot` structs; git indexing task; tool discovery task; summarization stub |
| `src/tui/widgets/conversation.rs` | Add match arms for new `Node` variants in helper functions |

---

## 7. Verification

1. `cargo build` -- zero warnings
2. `cargo clippy --all-targets` -- zero warnings
3. `cargo test` -- all existing tests pass + new tests:
   - Typed edge serialization round-trip
   - V1 -> V2 migration (with backup file creation)
   - New node type serialization
   - `add_node`, `add_edge`, `nodes_by_type`, `remove_nodes_by` methods
4. Visual: context panel appears above conversation with 4 tabs
5. Visual: git files populate in the Files tab after startup
6. Visual: background tasks show Running/Completed status in Tasks tab
7. Visual: context panel hides/shows with `Ctrl+B`
8. Visual: Tab key cycles focus between input and context panel
