# Design: TUI Graph Explorer — Tabbed Tree Views with Edge Navigation

**Date:** 2026-03-14
**Status:** Draft
**Prerequisites:** Research docs 02, 28; Design docs 01, 04, 06

---

## 1. Motivation

The TUI has a single `Overview` tab with a flat 3-column layout: Activity stream (40%), Work tree (30%), and a right panel stacking Agents/Running/Recent/Tools/Stats (30%). This layout has five structural problems:

1. **Invisible answers.** Questions are asked via the Q/A system (doc 04), but answers cannot be seen anywhere in the TUI. `QuestionRoutedToUser` shows a prompt, but once answered, the answer disappears. The user cannot review past Q/A exchanges.

2. **No edge visibility.** The graph has 17 typed edge kinds connecting 13 node types, but only `DependsOn` is surfaced (as inline `(depends on: ...)` text on work items). Users cannot trace tool call chains, see which context produced a response, or follow question/answer relationships.

3. **Activity dominates.** The chronological event stream occupies 40% of the screen despite being a secondary monitoring surface. Active work, questions, and execution chains are more important for understanding system state.

4. **Conversation toggles off.** The conversation panel is hidden when `FocusZone::TabContent` is active — the user must choose between exploring the graph and seeing the chat. Both should be visible simultaneously.

5. **No search.** With hundreds of nodes across 13 types, there is no way to find a specific question, tool call, or work item. The user must visually scan flat lists.

6. **Context building is opaque.** The 6-stage context pipeline (Gather→Score→Budget→Render→LLM Refine→Sanitize) records `ContextBuildingRequest` nodes with `SelectedFor` edges, but this provenance data is invisible. Users cannot see what context an agent used or why.

**Goal:** Replace the current Overview tab with a 3-tab graph explorer featuring tree-command-style connectors, a persistent detail panel for edge navigation, always-visible conversation, and search/query capabilities across all node types.

---

## 2. Master Layout — Conversation Always Visible

The current `FocusZone::TabContent | ChatPanel` visibility toggle is removed. The conversation panel is ALWAYS rendered on the right. Tab content fills the left. Both panels are always visible; focus switches between them for keyboard routing only.

```
┌───────────────────────────────────────────────────────────────────────┐
│ [Agent header: expands 1-3 lines when agents active]                  │
├─ Tab bar ────────────────────────────┬────────────────────────────────┤
│ 1:Overview | 2:Graph | 3:System      │                                │
├──────────────────────────────────────┤  Conversation                  │
│                                      │  (scrollable message history)  │
│  Tab content area                    │                                │
│  (dashboard / tree+detail / etc.)    │                                │
│                                      │                                │
│                                      ├────────────────────────────────┤
│                                      │  Input box (3 lines)           │
├──────────────────────────────────────┴────────────────────────────────┤
│ Status bar                                                            │
└───────────────────────────────────────────────────────────────────────┘
```

### 2.1 Horizontal Split

Left ~65% (tab content) | Right ~35% (conversation + input). Both always rendered. Below 100 columns, conversation collapses to a minimal fixed width (30 columns).

### 2.2 Focus Routing

Tab key switches keyboard focus between left and right.

| Focus | Behavior |
|-------|----------|
| Left (TabContent) | Arrow keys navigate trees/lists, `/` opens search, `1`-`3` switch tabs, Enter expands/follows |
| Right (ChatPanel) | Typing goes to input box, Up/Down scrolls conversation, Enter sends |

`FocusZone` remains as an enum for keyboard dispatch but no longer controls panel visibility. `NavigationState::conversation_visible()` always returns `true`.

### 2.3 Tab Bar Position

Rendered as the first line inside the left panel — a 1-line row within the left column. Tab labels are visually associated with their content, separate from the conversation.

---

## 3. Tab Architecture — 3 Tabs

### 3.1 Tab 1: Overview — Real-Time Dashboard

Everything "live" in one view. The user glances here to see system state at a glance.

| Section | Content | Data Source |
|---------|---------|-------------|
| **Agents** | Spinner, phase, streaming preview per agent | `agent_displays` in TuiState |
| **Active Work** | Plans/Tasks with Active status, tree indentation | `WorkItem` nodes, `SubtaskOf` edges |
| **Running** | Active tool calls with live duration | `ToolCall` nodes (Pending/Running) |
| **Questions** | Pending/claimed questions with answer status | `Question` nodes, `Answer` nodes via Answers edge |
| **Background** | Running background tasks | `BackgroundTask` nodes (Running) |
| **Stats** | Token usage, message count, tool call count, service status | Animated counters, graph node counts |

Layout: stacked sections within the left panel. Each section auto-sizes based on content. Empty sections collapse to zero height.

### 3.2 Tab 2: Graph — Core Explorer

Four collapsible sections, each using a tree+detail (60/40) horizontal split:

| Section | Tree Root Nodes | Children Via | Detail Shows |
|---------|----------------|-------------|--------------|
| **[Work]** | `WorkItem` (Plan, no SubtaskOf parent) | `SubtaskOf` edges | Full description, dependencies, claimed agent |
| **[Q&A]** | `Question` nodes | `Answer` nodes via `Answers` edge | Full question text, answer text, status, asker |
| **[Execution]** | `Message` (Assistant role) | `ToolCall` via `Invoked`, `ToolResult` via `Produced` | Full tool arguments, result content, duration |
| **[Context]** | `ContextBuildingRequest` nodes | Selected nodes via `SelectedFor`, grouped by tier | Pipeline stats, trigger, policy, token budget |

Sections expand/collapse with Enter on the section header. Only one section is "active" (has keyboard selection) at a time. Collapsed sections render as a single line: `▶ [Work] (3 plans, 12 tasks)`.

### 3.3 Tab 3: System — Historical/Operational Data

Five stacked sections, each collapsible:

| Section | Content | Data Source |
|---------|---------|-------------|
| **Activity** | Chronological event stream (all node types), newest first | All nodes sorted by `created_at` |
| **Files** | GitFile directory tree with status badges | `GitFile` nodes, paths parsed into directory tree |
| **Errors** | ApiError nodes with timestamps | `ApiError` nodes |
| **Tools** | Available tool registry (name + description table) | `Tool` nodes |
| **Stats** | Token totals, message counts, service status | Same as Overview stats but in static form |

Activity section includes type filter toggles at the top: `M` Messages, `T` ToolCalls, `Q` Questions, `B` Background, `E` Errors — press letter to toggle visibility.

---

## 4. Tree Rendering — `tree`-Command Style Connectors

All tree views use box-drawing characters matching the Unix `tree` command:

```
├── v Plan: Refactor TUI [Active]
│   ├── [*] Split overview tab
│   ├── [ ] Build Q&A tab          (needs: Split overview)
│   └── [v] Add detail panel
└── v Plan: API Integration [Active]
    ├── [*] Write HTTP client       ← agent:7b1c
    └── [ ] Implement retry logic
```

### 4.1 Connector Prefix Algorithm

Each tree item maintains a depth stack of `is_last_sibling` booleans. The prefix is built by:

1. For each ancestor depth level:
   - If ancestor was last sibling at that level: `    ` (4 spaces)
   - If ancestor was not last: `│   ` (pipe + 3 spaces)
2. For the item itself:
   - If last sibling: `└── `
   - If not last: `├── `

This replaces the current `"  ".repeat(depth)` indentation in `work.rs`.

### 4.2 Q&A Tree Example

```
├── ? "What model should we use?" [Answered]
│   └── A "Use claude-3.5-sonnet" [from: user]
├── ? "Should we split the file?" [Pending]
└── ? "Approve task completion?" [PendingApproval]
    └── A "Looks good, approved" [pending approval]
```

### 4.3 Execution Tree Example

```
├── A Message "Let me read that file..." [14:32:01]
│   ├── ✓ read_file src/main.rs [1.2s]
│   │   └── Result: 115 lines
│   └── ⠋ write_file src/tui/mod.rs [0.4s]
└── A Message "I'll now implement..." [14:32:15]
    └── ✓ plan "Refactor TUI" [0.1s]
```

### 4.4 Context Provenance Tree Example

```
├── CBR: TaskExecution (agent:a3f2) [Built, 8.2k tokens]
│   ├── Essential (12 nodes)
│   │   ├── Message: "Implement JWT auth..." (score: 0.92)
│   │   ├── ToolResult: src/auth.rs (score: 0.88)
│   │   └── WorkItem: JWT middleware (score: 0.85)
│   ├── Important (8 nodes)
│   │   └── Message: "What about refresh..." (score: 0.55)
│   └── Supplementary (3 nodes)
│       └── GitFile: src/main.rs (score: 0.25)
└── CBR: Conversational (agent:b4e2) [Built, 12.1k tokens]
```

---

## 5. Detail Panel — Persistent 60/40 Split

Every tree section in the Graph tab uses a horizontal `Layout` split: left 60% for the tree, right 40% for the detail panel. The detail panel updates as the user navigates the tree.

### 5.1 Detail Panel Sections

**Header** (3-4 lines):
- Node type badge + short UUID (8 chars)
- Primary content (title, question text, tool name)
- Status + timestamp
- Agent ID if claimed

**Content** (scrollable, takes remaining space):
- Full text content of the selected node
- Rendered as styled text (question text, answer text, tool arguments, result content, error message)

**Edges** (bottom section, scrollable):
- Grouped by semantic category (empty groups hidden)
- Each edge: human-readable label → target node summary (short UUID)
- Enter on an edge navigates to the target node

### 5.2 Edge Semantic Groups

| Group | Label | Edge Kinds |
|-------|-------|-----------|
| STRUCTURE | `STRUCTURE` | SubtaskOf, DependsOn, RespondsTo, Invoked, Produced |
| Q&A | `Q&A` | Asks, Answers, Triggers, Supersedes, About |
| REFERENCES | `REFS` | RelevantTo, Tracks, Indexes, Provides |
| COORDINATION | `COORD` | ClaimedBy, OccurredDuring, SelectedFor, ConsumedBy |

### 5.3 Human-Readable Edge Labels

Each `EdgeKind` maps to a user-facing label written from the node's perspective:

| EdgeKind | Display Label | Example |
|----------|--------------|---------|
| SubtaskOf | "part of" | `part of → Plan: Refactor TUI` |
| DependsOn | "depends on" | `depends on → Task: Split tab` |
| RespondsTo | "replies to" | `replies to → Message: "How do I..."` |
| Invoked | "invoked by" | `invoked by → Message: "Let me..."` |
| Produced | "produced by" | `produced by → ToolCall: read_file` |
| Asks | "asks" | `asks → Question: "What model?"` |
| Answers | "answers" | `answers → Question: "What model?"` |
| Triggers | "triggers" | `triggers → WorkItem: JWT auth` |
| Supersedes | "supersedes" | `supersedes → Answer: "Use GPT..."` |
| About | "about" | `about → WorkItem: JWT middleware` |
| ClaimedBy | "claimed by" | `claimed by → agent:7b1c` |
| RelevantTo | "relevant to" | `relevant to → WorkItem: Auth` |
| SelectedFor | "selected for" | `selected for → CBR: TaskExec` |
| ConsumedBy | "consumed by" | `consumed by → Message: "I'll..."` |

### 5.4 Edge Following — Breadcrumb Navigation

Enter on an edge in the detail panel pushes the current node onto a breadcrumb stack and navigates to the target node. This enables graph traversal without a graph visualizer.

```rust
struct Breadcrumb {
    node_id: Uuid,
    edge_index: usize,
}
```

- **Enter**: push current node, display target node's detail
- **Esc/Backspace**: pop back to previous node
- **Stack cap**: 10 entries (oldest dropped on overflow)
- **Breadcrumb trail**: shown at the top of the detail panel when non-empty

```
trail: Plan: Refactor > Task: Build Q&A > Question: "What model?" > here
```

Left-truncated to fit: `... > Question: "What model?" > here` when trail exceeds width.

### 5.5 Collapsibility

Press `d` to toggle the detail panel. When collapsed, the tree takes 100% of the section width. Detail state (scroll position, breadcrumb trail) is preserved across toggles.

---

## 6. Inline Edge Badges

High-value edges shown inline on tree items, budget-aware (omitted if no space):

| Badge | Source | Style | Meaning |
|-------|--------|-------|---------|
| `← agent:XXXX` | ClaimedBy edge | Dim cyan | Node claimed by this agent |
| `(needs: ...)` | DependsOn edge | Dim gray | Already exists in current work.rs |
| `?N` | Count of open Questions | Yellow | N open questions about this node |

Width budget: title gets priority. Badges appended only if `width - indent - icon - title_min_width > badge_width`. The detail panel is always available as a fallback for full edge visibility.

---

## 7. Context Building Visualization

The [Context] section in the Graph tab surfaces the 6-stage context pipeline's provenance data.

### 7.1 Data Model

Each `ContextBuildingRequest` node records:

| Field | Type | Meaning |
|-------|------|---------|
| `trigger` | `ContextTrigger` | `UserMessage`, `TaskExecution { work_item_id }`, `QuestionResponse { question_id }` |
| `policy` | `ContextPolicyKind` | `Conversational` or `TaskExecution` |
| `status` | `ContextBuildStatus` | `Requested` → `Building` → `Built` → `Consumed`/`FallbackUsed`/`Failed` |
| `candidates_count` | `u32` | Total candidates gathered in Phase A |
| `selected_count` | `u32` | Nodes selected after scoring/budget |
| `token_count` | `Option<u32>` | Precise token count after sanitization |
| `agent_id` | `Uuid` | Which agent performed the build |
| `built_at` | `Option<DateTime<Utc>>` | When build completed |

Connected edges:
- `SelectedFor`: CBR → each selected node (provenance)
- `ConsumedBy`: CBR → assistant Message produced from this context

### 7.2 Tree Structure

CBR nodes are roots, sorted newest first. Children are the selected nodes (via `SelectedFor` edges), grouped by selection tier:

- **Essential** (score ≥ 0.7): 60% of token budget, full content
- **Important** (score ≥ 0.4): 30% of token budget, full content
- **Supplementary** (score ≥ 0.2): 10% of token budget, summaries only

Tier assignment is reconstructed from the scoring thresholds defined in `src/app/context/scoring.rs`. If scores are not persisted on edges, the tier boundaries (0.7/0.4/0.2) are applied to re-score nodes or the tier is stored as metadata on the `SelectedFor` edge.

### 7.3 Detail Panel for CBR Nodes

```
[ContextBuildingRequest] a3f2c1d8
Pipeline: 142 candidates → 23 selected → 8.2k tokens
Trigger: TaskExecution (work_item: "JWT middleware")
Policy: TaskExecution
Selection: LlmGuided (fallback: no)
Built at: 14:32:01 (duration: 1.2s)
Status: Built

─── Edges ───
COORDINATION
  consumed by → Message: "Let me implement..." (d5a3)
REFERENCES
  selected for → 23 nodes (see tree)
```

---

## 8. Search & Query System

### 8.1 Activation

| Context | Key | Mode |
|---------|-----|------|
| Left panel (TabContent) focused | `/` | Structured node filter |
| Right panel (ChatPanel) focused | `Ctrl+F` | Text search in conversation |

### 8.2 Search Bar

1-line input below the tab bar in the left panel. Yellow border, cyan text. Shows match count and scope indicator.

```
╭─ / Search ────────────────────────────────────────────╮
│ type:question status:pending█                  3 ↕    │
╰──────────────────── Esc:close  Ctrl+G:scope ──────────╯
```

### 8.3 Query Language

Two-tier: plain text + structured prefix filters.

| Token | Meaning | Example |
|-------|---------|---------|
| Plain text | Case-insensitive substring match on `Node::content()` | `jwt auth` |
| `type:X` | Filter by node type discriminant | `type:question`, `type:toolcall` |
| `status:X` | Filter by status field (normalized across enums) | `status:pending`, `status:failed` |
| `role:X` | Filter by message role | `role:user`, `role:assistant` |
| `tool:X` | Filter by tool name | `tool:read_file` |
| `!prefix:X` | Invert match | `!status:done` |

Multiple tokens are AND-combined. Unknown prefixes are treated as plain text.

```rust
pub struct SearchQuery {
    /// Free-text substring match.
    pub text: String,
    /// Optional node type filter.
    pub node_type: Option<NodeTypeFilter>,
    /// Optional status filter (string, matched against status Display impl).
    pub status: Option<String>,
    /// Optional message role filter.
    pub role: Option<Role>,
    /// Optional tool name filter.
    pub tool_name: Option<String>,
    /// Invert the entire match.
    pub inverted: bool,
}
```

### 8.4 Result Presentation

- **Filtered tree**: matching nodes shown with parent chain preserved (for context). Non-matching branches collapsed. Tree connectors adapt to filtered structure.
- **Filtered list** (Activity): only matching rows shown.
- **Indicator**: `"FILTER ACTIVE (N matches)"` in the panel title when a filter is active.
- **Esc**: clears filter, restores full view.
- **Live**: re-evaluates on each keystroke. Graph is in-memory HashMap — linear scan of even 10k nodes completes in microseconds.

### 8.5 Conversation Text Search (Ctrl+F)

When activated in the right panel:
- Input box switches to search mode
- Matching text highlighted in the conversation scroll
- `n`/`N` jump between matches
- Match count shown: `"3 matches (1/3)"`
- Esc clears and returns to message input mode

---

## 9. Agent Status — Expandable Header Bar

The agent status display moves from the right column to a full-width header bar above the left+right split.

### 9.1 Active State

```
┌───────────────────────────────────────────────────────────────────────┐
│ ⠋ Agent a3f2  [streaming] "Let me read..."   [main]  45.3k/12.1k    │
│ ⠙ Agent b4e2  [executing tools]                                      │
├─ 1:Overview | 2:Graph | 3:System ────┬────────────────────────────────┤
│  Tab content                         │  Conversation                  │
```

Header uses the existing `AgentDisplayState` (phase, spinner_tick, streaming text) and `AgentVisualPhase` enum. One line per active agent. Branch name and token counters are right-aligned on the first agent line.

### 9.2 Idle State

Header collapses to zero lines. Branch name and token counters move to the bottom status bar. The tab bar becomes the top visual element.

### 9.3 Running Task Badge

The Overview tab label shows a count of running items: `Overview (3)` when 3 background tasks + tool calls are active. Other tabs show no badges.

---

## 10. State Management Changes

### 10.1 TopTab Enum

Replaces the current single-variant `TopTab::Overview`:

```rust
pub enum TopTab {
    Overview,
    Graph,
    System,
}
```

### 10.2 GraphSection Enum

Controls which section is expanded within the Graph tab:

```rust
pub enum GraphSection {
    Work,
    QA,
    Execution,
    Context,
}
```

### 10.3 ExplorerState

Per-section state for tree+detail navigation:

```rust
pub struct ExplorerState {
    /// Animated scroll for the tree panel.
    pub tree_scroll: AnimatedScroll,
    /// Maximum scroll offset for tree panel (set each frame).
    pub tree_max: u16,
    /// Animated scroll for the detail panel.
    pub detail_scroll: AnimatedScroll,
    /// Maximum scroll offset for detail panel (set each frame).
    pub detail_max: u16,
    /// Index of the selected item in the flattened tree.
    pub selected: usize,
    /// Total visible items (set each frame by the renderer).
    pub visible_count: usize,
    /// Set of collapsed node IDs (expanded by default).
    pub collapsed: HashSet<Uuid>,
    /// Which sub-panel has focus: Tree or Detail.
    pub focus: ExplorerFocus,
}

pub enum ExplorerFocus {
    Tree,
    Detail,
}
```

### 10.4 EdgeInspector

State for edge navigation in the detail panel:

```rust
pub struct EdgeInspector {
    /// Pre-computed display edges for the currently selected node.
    pub edges: Vec<DisplayEdge>,
    /// Selected edge index within the detail panel.
    pub selected_edge: usize,
    /// Breadcrumb trail of followed edges (capped at 10).
    pub trail: Vec<Breadcrumb>,
}

pub struct DisplayEdge {
    /// Semantic group this edge belongs to.
    pub group: EdgeGroup,
    /// Human-readable label ("part of", "answers", etc.).
    pub label: &'static str,
    /// Summary of the target node.
    pub target_summary: String,
    /// Target node UUID for follow navigation.
    pub target_id: Uuid,
}

pub struct Breadcrumb {
    /// Node we navigated away from.
    pub node_id: Uuid,
    /// Edge index that was selected when following.
    pub edge_index: usize,
}
```

### 10.5 SearchState

```rust
pub struct SearchState {
    /// Raw query text from the search input.
    pub query_text: String,
    /// Character-indexed cursor position.
    pub cursor: usize,
    /// Parsed structured query (re-parsed on each keystroke).
    pub parsed: SearchQuery,
    /// Where the search is applied.
    pub scope: SearchScope,
    /// Node IDs matching the current query (recomputed per keystroke).
    pub matching_ids: HashSet<Uuid>,
}

pub enum SearchScope {
    /// Filter within the current tab only.
    Tab,
    /// Search across all node types globally.
    Global,
}
```

### 10.6 TuiState Changes

**Removed fields** (replaced by per-section `ExplorerState`):
- `work_selected`, `work_visible_count`
- `overview_scroll`, `overview_max`
- `recent_scroll`, `recent_max`

**New fields**:
- `explorer: HashMap<GraphSection, ExplorerState>` — per-section explorer state
- `search: Option<SearchState>` — active search overlay
- `edge_inspector: EdgeInspector` — shared edge navigation state
- `active_graph_section: GraphSection` — which Graph section is expanded

**Layout change**: `NavigationState::conversation_visible()` always returns `true`. Tab key switches keyboard focus without hiding/showing panels.

---

## 11. Graph Query Additions

New methods on `ConversationGraph`:

| Method | Signature | Purpose |
|--------|-----------|---------|
| `targets_by_edge` | `(source: Uuid, kind: EdgeKind) -> Vec<Uuid>` | Outgoing edges (complement to existing `sources_by_edge`) |
| `edges_of` | `(node_id: Uuid) -> Vec<(EdgeDirection, EdgeKind, Uuid)>` | All edges involving a node |

New methods on `EdgeKind`:

| Method | Returns | Purpose |
|--------|---------|---------|
| `display_label(&self)` | `&'static str` | Human-readable label for the detail panel |
| `group(&self)` | `EdgeGroup` | Semantic category (Structure, QA, References, Coordination) |

```rust
pub enum EdgeGroup {
    Structure,
    QA,
    References,
    Coordination,
}

pub enum EdgeDirection {
    Incoming,
    Outgoing,
}
```

---

## 12. File Structure

```
src/tui/
  tabs/
    mod.rs                    # Tab module declarations + tab dispatch
    overview/
      mod.rs                  # Overview dashboard render (~350 lines)
    graph/
      mod.rs                  # Graph tab: section layout + dispatch (~200 lines)
      work.rs                 # Work tree building + rendering (~200 lines)
      qa.rs                   # Q&A tree building + rendering (~200 lines)
      execution.rs            # Execution chain tree (~200 lines)
      context.rs              # Context building provenance tree (~250 lines)
      tree_lines.rs           # Tree connector prefix computation (~100 lines)
    system/
      mod.rs                  # System tab: Activity + Files + stacked sections (~350 lines)
    detail.rs                 # Shared detail panel rendering (~300 lines)
    explorer.rs               # ExplorerState, tree navigation, expand/collapse (~150 lines)
  search/
    mod.rs                    # SearchState, SearchScope, re-exports (~100 lines)
    query.rs                  # SearchQuery, NodeTypeFilter, parse_query() (~150 lines)
    matcher.rs                # matches_node() evaluation (~150 lines)
  state/
    mod.rs                    # TopTab, GraphSection, ExplorerFocus, PanelRects
```

All files stay within the 400-line limit. Tests in separate `*_tests.rs` files.

---

## 13. Implementation Phases

| Phase | Scope | Files |
|-------|-------|-------|
| 1 | State foundation: TopTab, ExplorerState, GraphSection, EdgeInspector, SearchState structs | `state/mod.rs`, `tabs/explorer.rs` |
| 2 | Graph queries: `targets_by_edge`, `edges_of`, `display_label`, `EdgeGroup` | `graph/mod.rs`, `graph/node/enums.rs` |
| 3 | Tree connector system: `tree_lines.rs` with ├── └── │ prefix builder | `tabs/graph/tree_lines.rs` |
| 4 | Layout framework: always-visible conversation, tab dispatch, tree+detail split | `ui.rs`, `tabs/mod.rs`, `tabs/detail.rs` |
| 5 | Overview tab: dashboard with all active items | `tabs/overview/mod.rs` |
| 6 | Graph tab — Work section: migrate from work.rs, add tree connectors + detail | `tabs/graph/mod.rs`, `tabs/graph/work.rs` |
| 7 | Graph tab — Q&A section: Question→Answer trees | `tabs/graph/qa.rs` |
| 8 | Graph tab — Execution section: Message→ToolCall→ToolResult chains | `tabs/graph/execution.rs` |
| 9 | Graph tab — Context section: CBR→SelectedFor provenance trees with tier grouping | `tabs/graph/context.rs` |
| 10 | System tab: Activity (migrated), Files, Errors, Tools, Stats | `tabs/system/mod.rs` |
| 11 | Agent header: expandable bar above left+right split | `ui.rs` |
| 12 | Search: query parser, matcher, filtered rendering, search bar UI | `search/mod.rs`, `search/query.rs`, `search/matcher.rs` |
| 13 | Input handling: tab switching, tree navigation, edge following, search activation | `input/mod.rs` |
| 14 | Cleanup: remove old overview.rs, agents.rs, update event_handler.rs | Multiple files |

Each phase produces independently testable, visible results. Phases 1-4 are foundation; phases 5-10 build tabs; phases 11-14 add cross-cutting features.

---

## 14. Red/Green Team Analysis

### 14.1 Green Team

1. **3 tabs is minimal cognitive load.** Overview answers "what's happening?", Graph answers "show me the data", System answers "what happened before?". No tab feels redundant.

2. **Conversation always visible.** The user never loses chat context while exploring the graph. This is strictly better than the current toggle: the same information is available, with no hiding.

3. **Q&A becomes first-class visible.** The [Q&A] section in the Graph tab directly fixes the core complaint. Questions as tree roots, answers as children, full lifecycle visible.

4. **Context provenance is observable.** The [Context] section surfaces the 6-stage pipeline's decisions: which nodes were selected, at what tier, how many tokens consumed. This makes context building auditable and debuggable.

5. **Tree+detail is proven UX.** lazygit, gitui, k9s all use this pattern. Users understand it without documentation.

6. **Edge following via breadcrumb stack** avoids the complexity of graph visualization. The user traces relationships by navigating between nodes, not by parsing a force-directed layout.

7. **Hand-rendered trees** (no `tui-tree-widget` dependency) maintain full control over styling, inline badges, and connector characters.

8. **Tree-command connectors** (├── └── │) are universally recognized and match user expectations.

### 14.2 Red Team

| # | Issue | Severity | Resolution |
|---|-------|----------|------------|
| 1 | Always-visible conversation reduces tab content to ~65% width | MEDIUM | Detail panel collapsible with `d`; min terminal width 100 cols |
| 2 | Graph tab with 4 collapsible sections could feel cramped | MEDIUM | Only one section expanded at a time; collapsed = 1 line |
| 3 | Detail panel at 40% of 65% = ~26% total width could be tight | MEDIUM | Collapsible with `d` key; minimum terminal width check |
| 4 | Context section needs score data not stored on SelectedFor edges | HIGH | Reconstruct tiers from score thresholds (0.7/0.4/0.2) or add score field to SelectedFor edge metadata |
| 5 | Search query syntax not discoverable | LOW | Placeholder text hints at syntax; status bar legend when search active |
| 6 | 14 implementation phases is large scope | MEDIUM | Each phase independently testable; early phases produce visible results |
| 7 | `edges_of()` is O(E) per call without an index | LOW | E is small (hundreds). Index can be added later if needed. |
| 8 | Breadcrumb trail can cross sections/tabs — disorienting | MEDIUM | Trail shows node type tags; pressing Esc always returns to the tree view |

---

## 15. Sources

- `docs/research/02-developer-ux-and-workflow.md` — TUI frameworks, non-linear interfaces, graph visualization limitations
- `docs/research/28-terminal-ux-notifications.md` — Status bar patterns, visual feedback, accessibility
- `docs/design/01-graph-extensions-and-context-panel.md` — Original context panel design, graph versioning
- `docs/design/04-graph-scheduler-qa-relationships.md` — Question/Answer lifecycle, routing
- `docs/design/06-parallel-autonomous-agents.md` — ContextBuildingRequest, SelectedFor/ConsumedBy edges, context scoring pipeline
- tui-tree-widget crate: https://crates.io/crates/tui-tree-widget (evaluated, not adopted)
- tui-scrollview crate: https://crates.io/crates/tui-scrollview (reference for scroll patterns)
- Ratatui Tabs widget: https://docs.rs/ratatui/latest/ratatui/widgets/struct.Tabs.html
