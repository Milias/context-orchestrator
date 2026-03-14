# Design 04: Graph Coordination, Q/A Relationships, and Context Pipeline

> Status: Draft
> Date: 2026-03-14
> Depends on: [03-tool-call-foundation.md](./03-tool-call-foundation.md)
> Research: [21-graph-scheduler-qa-relationships.md](../research/21-graph-scheduler-qa-relationships.md),
>           [22-graph-context-building-strategies.md](../research/22-graph-context-building-strategies.md),
>           [19-llm-question-surfacing.md](../research/19-llm-question-surfacing.md),
>           [07-inter-agent-communication.md](../research/07-inter-agent-communication.md)

---

## Table of Contents

1. [Motivation](#1-motivation)
2. [System Overview — How the Three Systems Interlock](#2-system-overview)
3. [Q/A as Graph Citizens](#3-qa-as-graph-citizens)
4. [Question Lifecycle State Machine](#4-question-lifecycle-state-machine)
5. [Edge Taxonomy](#5-edge-taxonomy)
6. [Context Pipeline Architecture](#6-context-pipeline-architecture)
7. [ContextPolicy Trait and Built-in Policies](#7-contextpolicy-trait-and-built-in-policies)
8. [Ask Tool Design](#8-ask-tool-design)
9. [Answer Routing Architecture](#9-answer-routing-architecture)
10. [Multi-Agent Coordination Model](#10-multi-agent-coordination-model)
11. [EventBus Broadcast Layer](#11-eventbus-broadcast-layer)
12. [Self-Scheduling Loop](#12-self-scheduling-loop)
13. [Migration Strategy](#13-migration-strategy)
14. [Future: GraphCoordinator, Advanced Context Strategies](#14-future)

---

## 1. Motivation

Design 03 established tool calls as first-class graph citizens — every invocation is traceable via
`ToolCall` and `ToolResult` nodes with `Invoked` and `Produced` edges. But three fundamental
capabilities remain missing:

**Agents cannot ask questions.** When an LLM encounters ambiguity (which auth library? approve this
migration?), it has no mechanism to surface a question, route it to the right responder (user or
another LLM), and block on the answer. Questions and answers must be graph nodes — not special-cased
strings — so they participate in the same edge-typed, version-tracked, context-injectable system as
every other node.

**Agents get identical context.** Today, `extract_messages()` walks the `RespondsTo` ancestor chain
and serializes all messages linearly. Every agent invocation sees the same conversation history
regardless of its purpose. A task-execution agent implementing "write JWT middleware" gets the same
context as a background rater evaluating node relevance. VISION.md §3.2 describes a 6-stage context
pipeline where context is *constructed* from graph traversal, not merely *serialized* from message
history. This document designs that pipeline.

**Agents cannot coordinate.** The system runs a single agent loop. When an LLM-destined question
requires spawning a second agent, or when two plans can execute in parallel, there is no mechanism
for agents to claim work, avoid duplicate execution, or discover what work is ready. Doc 21's
self-scheduling model — where agents query the graph for ready work and claim it atomically — provides
the coordination primitive.

These three systems are not independent features. They form a closed loop:

```
Agent claims work → ContextPolicy builds purpose-specific context
  → Agent executes → produces answer / asks question / completes task
  → Graph mutated → EventBus broadcasts state change
  → Self-scheduling: check for ready work → claim → select policy → repeat
```

Designing them separately would produce three incompatible subsystems. This document designs them
as one integrated architecture.

---

## 2. System Overview

### The Agent Lifecycle

```
                          ┌─────────────────────┐
                          │     Graph State      │
                          │  (ConversationGraph) │
                          └───────┬───────┬──────┘
                                  │       │
                    ┌─────────────┘       └──────────────┐
                    ▼                                     ▼
           ┌───────────────┐                    ┌────────────────┐
           │ Self-Schedule  │                    │   EventBus     │
           │ ready_unclaimed│                    │  (broadcast)   │
           │ try_claim()    │                    └───────┬────────┘
           └───────┬───────┘                            │
                   │                                    │
                   ▼                                    ▼
           ┌───────────────┐                  ┌─────────────────┐
           │ Select Policy  │                  │  TUI / Agents   │
           │ based on       │                  │  (subscribers)  │
           │ ContextTrigger │                  └─────────────────┘
           └───────┬───────┘
                   │
                   ▼
           ┌───────────────┐
           │ build_context  │  5+1 stage pipeline
           │ anchor→expand  │  Anchor → Expand → Score →
           │ →score→budget  │  Budget → Render → Sanitize
           │ →render→sanitize│
           └───────┬───────┘
                   │
                   ▼
           ┌───────────────┐
           │  Agent Loop    │  stream LLM → apply iteration
           │  (concurrent)  │  → dispatch tools → wait
           └───────┬───────┘
                   │
                   ▼
           ┌───────────────┐
           │ Graph Mutation  │  add answer, complete task,
           │ + EventBus emit │  ask new question
           └────────────────┘
```

### Why One System, Not Three

The context pipeline needs Q/A nodes: `QuestionResponsePolicy` anchors on a `Question` node and
follows `About` edges to gather context. Without Q/A nodes, the policy has nothing to anchor on.

Multi-agent coordination needs the context pipeline: when an agent claims a `WorkItem`, the system
must build task-specific context via `TaskExecutionPolicy`. Without the pipeline, all agents get
the same conversational context regardless of their assignment.

Q/A routing needs multi-agent: when a question is routed to an LLM backend, a new agent loop must
be spawned concurrently. Without multi-agent coordination, LLM questions must queue behind the
current agent — defeating the purpose of asynchronous question routing.

---

## 3. Q/A as Graph Citizens

Questions and answers are `Node` variants in the `ConversationGraph`, not special-cased strings or
side-channel data. This follows the design principle from Design 01: "Everything is a graph node."

### Question Node

```rust
Question {
    id: Uuid,
    content: String,
    destination: QuestionDestination,
    status: QuestionStatus,
    requires_approval: bool,
    created_at: DateTime<Utc>,
}
```

- `destination` determines routing: `User` (TUI prompt), `Llm` (spawn agent), `Auto` (heuristic).
- `requires_approval` gates auto-acceptance of LLM answers. When true, the answer enters as
  `PendingApproval` and the user must explicitly accept before the question resolves.
- `status` tracks the lifecycle state machine (§4).

### Answer Node

```rust
Answer {
    id: Uuid,
    content: String,
    question_id: Uuid,
    created_at: DateTime<Utc>,
}
```

The `question_id` field is denormalized convenience — the canonical relationship is the `Answers`
edge (Answer → Question). This follows the same pattern as `tool_call_id` on `ToolResult`: the
field enables quick lookups without edge traversal, while the edge provides graph-queryable
provenance.

### Why Not Reuse Message Nodes?

Questions could be `Message` nodes with a `question` metadata field. We chose dedicated variants
because:

1. **State machine.** Questions have a lifecycle (Pending → Claimed → Answered) that Messages don't.
   Adding lifecycle fields to Message would bloat every message with unused fields.
2. **Typed routing.** `QuestionDestination` determines which backend handles the question. Messages
   have `Role` (User/Assistant/System) which is a different axis.
3. **Edge semantics.** `Asks`, `Answers`, `About` edges are specific to Q/A. Using `RespondsTo` for
   answers would conflate conversation threading with question resolution.
4. **Query efficiency.** `graph.pending_questions()` is a simple type filter. With Message nodes, it
   would require checking metadata on every message.

---

## 4. Question Lifecycle State Machine

```
                        ┌──────────┐
                   ┌───▶│  Pending  │◀──────────────────────┐
                   │    └─────┬────┘                        │
                   │          │ try_claim(agent_id)          │
                   │          ▼                              │
                   │    ┌──────────┐                        │
                   │    │  Claimed  │                        │
                   │    └─────┬────┘                        │
                   │          │                              │
                   │    ┌─────┴──────────────┐              │
                   │    │                    │              │
                   │    ▼                    ▼              │
              ┌──────────┐         ┌─────────────────┐     │
              │ Answered  │         │ PendingApproval │     │
              └──────────┘         └────────┬────────┘     │
                                      ┌─────┴─────┐        │
                                      │           │        │
                                      ▼           ▼        │
                                ┌──────────┐ ┌──────────┐  │
                                │ Answered │ │ Rejected  │──┘
                                └──────────┘ └──────────┘

              Pending ──timeout──▶ TimedOut
```

### Transition Rules

| From | To | Trigger | Guard |
|------|----|---------|-------|
| Pending | Claimed | `try_claim(agent_id)` | No existing ClaimedBy edge |
| Pending | TimedOut | Timeout (5 min configurable) | — |
| Claimed | Answered | `add_answer()` | `!requires_approval` |
| Claimed | PendingApproval | `add_answer()` | `requires_approval` |
| PendingApproval | Answered | User accepts | — |
| PendingApproval | Rejected | User rejects | — |
| Rejected | Pending | Automatic (immediate) | Re-claimable |

**Rejected is transient.** When a user rejects an LLM answer, the question transitions through
Rejected (captured in version history via `mutate_node`) and immediately back to Pending. The
Rejected state exists only in the audit trail. Steady-state values are always one of
{Pending, Claimed, PendingApproval, Answered, TimedOut}.

**Invalid transitions are rejected.** `update_question_status()` validates the state machine and
returns `Err` for invalid transitions (e.g., Pending → Answered, skipping Claimed). This prevents
bugs where a code path bypasses the claiming mechanism.

---

## 5. Edge Taxonomy

The graph grows from 10 to 16 edge types. All 16 are listed here for completeness.

### Existing Edges (10)

| EdgeKind | From → To | Purpose |
|----------|-----------|---------|
| RespondsTo | Message → Message | Conversation threading |
| Invoked | ToolCall → Message | Tool call provenance |
| Produced | ToolResult → ToolCall | Tool result linkage |
| SubtaskOf | WorkItem → WorkItem | Task hierarchy |
| DependsOn | WorkItem → WorkItem | Plan-to-plan prerequisites |
| RelevantTo | WorkItem → Message | Topical association |
| ThinkingOf | ThinkBlock → Message | Extended thinking linkage |
| Indexes | GitFile → branch leaf | File context membership |
| Provides | Tool → branch leaf | Tool availability |
| Tracks | (unused) | Reserved |

### New Edges (6)

| EdgeKind | From → To | Purpose | Key Queries |
|----------|-----------|---------|-------------|
| Asks | ToolCall → Question | "Who asked this question?" | Provenance tracing |
| Answers | Answer → Question | "What resolved this question?" | Q/A pairing |
| About | Question → AnyNode | "What is this question about?" | Context policy expansion |
| Triggers | Answer → AnyNode | "What did this answer cause?" | Causality tracking |
| Supersedes | Answer → Answer | "Which answer is current?" | Answer versioning |
| ClaimedBy | AnyNode → agent Uuid | "Who is working on this?" | Multi-agent coordination |

### Why ClaimedBy Is an Edge, Not a Status Field

A status field (`claimed_by: Option<Uuid>`) would work for single-node-type claiming. But
`ClaimedBy` applies to any claimable node — Questions, WorkItems, and future node types. An edge:

1. **Generalizes** across node types without adding `claimed_by` to every variant.
2. **Enables debugging**: "Show me everything agent X has claimed" = `sources_by_edge(agent_id, ClaimedBy)`.
3. **Supports stale claim recovery**: `release_all_claims()` removes all ClaimedBy edges in one pass.
4. **Scales to multi-agent**: when ClaimedBy targets become Agent nodes (future), the edge naturally
   becomes a typed relationship with rich metadata.

### Why Triggers and Supersedes Have No Producers Yet

Both edges are defined for forward compatibility with doc 21's Phase 2:

- **Triggers**: When agent loops carry `answer_context_id: Option<Uuid>`, all nodes created in that
  context get Triggers edges from the answer. This is deferred until multi-agent is mature.
- **Supersedes**: When a question is re-answered (answer rejected → new answer), the new answer
  supersedes the old. The edge creation is automatic in `add_answer()` when a prior answer exists.

`EdgeKind` variants are zero-runtime-cost. They exist in the enum definition and in serde
deserialization tables, but no edges of these types are created until the producing code exists.

---

## 6. Context Pipeline Architecture

### The 5+1 Stage Pipeline

Today's `extract_messages()` is a monolithic function that walks `RespondsTo` edges and serializes
messages. The new pipeline decomposes context construction into 6 independent stages:

```
Graph Snapshot (read-only)
  │
  ▼
Stage 1: ANCHOR — Determine starting node(s) for traversal
  │  Branch leaf (conversational), WorkItem (task), Question (Q/A)
  ▼
Stage 2: EXPAND — Gather candidate nodes via edge traversal
  │  Ancestor walk, typed fan-out, dependency closure
  ▼
Stage 3: SCORE — Rank candidates by relevance to anchor
  │  Edge-weighted distance, batch BFS scoring
  ▼
Stage 4: BUDGET — Allocate token budget across sections
  │  Tiered min/max per section, high-score nodes fill first
  ▼
Stage 5: RENDER — Serialize selected nodes into chat messages
  │  Verbatim, progressive detail, observation masking
  ▼
Stage 6: SANITIZE — Enforce API structural constraints
  │  Orphan removal, boundary fixing, tool result pairing
  ▼
(Option<String>, Vec<ChatMessage>) → finalize_context() → LLM API
```

### Why 6 Stages?

Doc 22's red team found that the original 5-stage proposal lacked a sanitization step. The current
`sanitize_message_boundaries()` fixes critical API constraint violations (orphaned tool results,
leading assistant messages) after truncation. Omitting this stage would cause API errors. Stage 6
preserves this safety net.

### Why Batch Scoring?

Doc 22's red team found that per-node `score()` calls force O(N × path_cost) when a single BFS
pass from anchors scores all nodes at O(V + E). The trait uses `score_batch()` to enable efficient
single-pass algorithms. Implementations can still score individually if batch isn't beneficial.

### Why `budget()` Returns `Vec<BudgetSection>`?

A fixed struct with named fields (`system_fraction`, `conversation_fraction`) is rigid — adding a
new section requires modifying the struct. `Vec<BudgetSection>` is extensible: each section has a
name, min/max token allocation, and scored candidates. Budget policies can add memory sections,
question context sections, or any future content type without trait changes.

---

## 7. ContextPolicy Trait and Built-in Policies

### The Trait

```rust
/// Configures how to build an LLM context window for a specific agent role.
/// Each method corresponds to one stage of the context-building pipeline.
pub trait ContextPolicy: Send + Sync {
    /// Stage 1: Determine anchor node(s) for context traversal.
    fn anchors(&self, graph: &ConversationGraph, trigger: &ContextTrigger) -> Vec<Uuid>;

    /// Stage 2: Expand from anchors to gather candidate nodes.
    fn expand(&self, graph: &ConversationGraph, anchors: &[Uuid]) -> Vec<Uuid>;

    /// Stage 3: Score all candidates in one pass. Returns (node_id, score) pairs.
    fn score_batch(
        &self,
        graph: &ConversationGraph,
        anchors: &[Uuid],
        candidates: &[Uuid],
    ) -> Vec<ScoredNode>;

    /// Stage 4: Allocate token budget across content sections.
    fn budget(&self, max_tokens: u32) -> Vec<BudgetSection>;

    /// Stage 5: Render a node into context fragments at the given detail level.
    fn render(
        &self,
        graph: &ConversationGraph,
        node: &Node,
        detail: DetailLevel,
    ) -> Vec<RenderedFragment>;

    /// Stage 6: Post-process assembled messages (sanitize boundaries, fix pairing).
    fn post_process(&self, messages: &mut Vec<ChatMessage>);
}
```

### Supporting Types

```rust
/// What triggered context construction.
pub enum ContextTrigger {
    /// User sent a message on a conversation branch.
    UserMessage { branch: String },
    /// Agent is executing a work item.
    TaskExecution { work_item_id: Uuid },
    /// Agent is answering a question.
    QuestionResponse { question_id: Uuid },
    /// Background analysis of a target node.
    BackgroundAnalysis { target_id: Uuid },
}

/// A node with its computed relevance score.
pub struct ScoredNode {
    pub id: Uuid,
    pub score: f32,
}

/// Token budget for one content section.
pub struct BudgetSection {
    pub name: String,
    pub min_tokens: u32,
    pub max_tokens: u32,
}

/// How much detail to render a node with.
pub enum DetailLevel {
    Full,
    Summary,
    OneLine,
    MetadataOnly,
}

/// A rendered piece of context that assembles into chat messages.
pub enum RenderedFragment {
    SystemSection(String),
    UserMessage(String),
    AssistantMessage { text: String, tool_uses: Vec<ToolUseBlock> },
    ToolResult { api_id: String, content: String },
}
```

### Built-in Policies

#### ConversationalPolicy

For interactive chat. **Must produce identical output to current `extract_messages()`.** This is the
behavioral equivalence guarantee — no regression when migrating to the pipeline.

| Stage | Strategy | Detail |
|-------|----------|--------|
| Anchor | Branch leaf | `graph.branch_leaf(active_branch)` |
| Expand | Ancestor walk | `RespondsTo` edges to root, plus Invoked/Produced for tool calls |
| Score | Distance from leaf | 1.0 / (1 + distance). All ancestors included (no pruning). |
| Budget | Include all | System 15%, Conversation 60%, Tools 15%, Work 10% |
| Render | Verbatim | Messages as-is. Plan section injected into system prompt. |
| Sanitize | Standard | `sanitize_message_boundaries()` after truncation |

#### TaskExecutionPolicy

For agents executing a specific WorkItem.

| Stage | Strategy | Detail |
|-------|----------|--------|
| Anchor | WorkItem | The assigned `work_item_id` |
| Expand | Typed fan-out + dependency closure + last 5 messages | SubtaskOf (depth 2), DependsOn (closure), RelevantTo (depth 1), last 5 conversation messages for conversational awareness |
| Score | Edge-weighted distance | DependsOn=0.9, SubtaskOf=0.85, About=0.8, Invoked/Produced=0.7, RelevantTo=0.6, Triggers=0.5, Indexes/Provides=0.3 |
| Budget | Tiered | System 20%, Conversation 25%, Tools 25%, Work 20%, Memory 10% |
| Render | Progressive detail | Score > 0.8 → Full, 0.5-0.8 → Summary, < 0.5 → OneLine |
| Sanitize | Standard | Same as Conversational |

#### QuestionResponsePolicy

For agents answering a graph Question.

| Stage | Strategy | Detail |
|-------|----------|--------|
| Anchor | Question | The `question_id` being answered |
| Expand | About refs + DependsOn + originating chain | Follow About edges to referenced nodes, DependsOn to blocking questions, Asks edge back to originating ToolCall → parent Message chain |
| Score | Edge-weighted distance | Same weights as TaskExecution |
| Budget | Proportional | System 20%, Conversation 30%, Tools 30%, About-context 20% |
| Render | Structured | Question text as system directive, About refs as structured context |
| Sanitize | Standard | Same as Conversational |

---

## 8. Ask Tool Design

The `ask` tool follows the unified tool pipeline established in Design 03:
`ToolName` → `ToolCallArguments` → stateless executor → side-effects in handler.

### Tool Parameters

```rust
ToolName::Ask
ToolCallArguments::Ask {
    question: String,
    destination: QuestionDestination,  // user, llm, auto
    about_node_id: Option<Uuid>,
    requires_approval: Option<bool>,   // default: false
}
```

### Registry Entry

```
ask: "Ask a question to the user, an LLM, or auto-route.
      Returns a question UUID. The answer will arrive asynchronously
      and resolve any DependsOn edges."
```

### Execution and Side-Effects

**Stateless executor** (`tool_executor/qa_tools.rs`): Returns placeholder text
`"Question submitted: {question}"`. No graph access.

**Side-effect** (`app/qa/effects.rs`): Runs in `handle_tool_call_completed` with graph write access:
1. Create `Question` node with `status: Pending`
2. Add `Asks` edge: `tool_call_id → question_id`
3. Add `About` edge: `question_id → about_node_id` (if provided and target node exists)
4. Return enriched content with UUID: `"Created question '{question}' (id: {uuid}). Answer pending."`
5. Return `PendingQuestion` for routing

### User Trigger

`/ask user What JWT library should we use?` parses as:
- First word after `/ask`: destination (user/llm/auto, default: user)
- Remainder: question text
- `about_node_id` and `requires_approval` only available via LLM tool_use

Both `/ask user ...` (user trigger) and LLM `tool_use` (agent trigger) converge on the same
`ToolCallArguments::Ask` variant through the same dispatch path.

---

## 9. Answer Routing Architecture

### Routing as Claiming

Question routing IS claiming. The backend that routes a question creates the `ClaimedBy` edge. This
unifies routing and coordination — there is no separate routing layer.

### User Backend

When `destination == User`:
1. Transition Question to `Claimed` (ClaimedBy edge with a "user-backend" UUID)
2. Set `tui_state.pending_question = Some(PendingQuestionDisplay { question_id, content })`
3. TUI renders question prompt above input area (Yellow border, distinct from normal chat)
4. User types answer → `Action::AnswerQuestion(String)`
5. Handler calls `graph.add_answer(question_id, text)` → status transitions
6. Clear `pending_question` from TUI state

**Serialization**: One user question at a time. If a second question arrives while one is pending,
it stays in `Pending` state until the first is answered.

**Timeout**: 5-minute configurable timeout. On expiry: transition to `TimedOut`, create Answer with
content "Question timed out without user response."

### LLM Backend

When `destination == Llm`:
1. Claim the question for the primary agent (using its existing `agent_id` from `primary_agent_id`)
2. Transition Question to `Claimed` (ClaimedBy edge points to primary agent)
3. If no primary agent is running, question stays `Pending` — `check_ready_work()` routes
   it when an agent starts
4. On the next agent iteration, the context pipeline surfaces claimed questions via
   `build_qa_section()` in the system prompt
5. The agent calls the `answer` tool with `question_id` and `content`
6. `qa::effects::apply_answer()` calls `graph.add_answer()` — creates Answer node,
   Answers edge, transitions to Answered, emits `QuestionAnswered`

This is **self-Q&A as structured reasoning**: the agent decomposes a problem via `ask(llm, ...)`,
the question becomes a graph citizen, it appears in context on the next iteration, and the agent
produces a formal answer via the standard tool pipeline.

**Cancellation**: If the agent loop finishes without answering, the `Finished` handler releases
ClaimedBy edges. `check_ready_work()` detects the stale claim and re-routes the question.

### Auto Routing

Heuristic in `app/qa/routing.rs`:
- If `about_node_id` references a code-related node (GitFile, ToolResult): route to LLM
- If question text contains approval words ("approve", "confirm", "choose", "pick"): route to User
- Default: route to User (safe fallback — avoids surprise LLM cost)

---

## 10. Multi-Agent Coordination Model

### Concurrent Agent Loops

Multiple agent loops coexist under `Arc<RwLock<ConversationGraph>>`. Each loop:
1. Acquires brief read locks for context extraction
2. Releases lock before async work (LLM streaming)
3. Acquires brief write locks for graph mutations

Graph operations are microsecond-scale. LLM latency (seconds) dominates. Lock contention is
negligible for 2-5 concurrent agents.

### Agent Tracking

```rust
/// Tracks active agent loops.
agents: HashMap<Uuid, AgentHandle>,

/// Per-agent metadata.
struct AgentHandle {
    tool_tx: mpsc::UnboundedSender<AgentToolResult>,
    cancel_token: CancellationToken,
    task_tokens: HashMap<Uuid, CancellationToken>,
    active_phase_ids: HashSet<Uuid>,
}
```

The primary conversation agent is tracked by `primary_agent_id: Option<Uuid>`. LLM-directed
questions are claimed for this agent (not spawned as separate agent loops).

### Atomic Claiming

`try_claim(node_id, agent_id) -> bool` runs under a write lock:
1. Check for existing `ClaimedBy` edge on `node_id`
2. If found: return `false` (already claimed)
3. If not: add `ClaimedBy` edge (`node_id → agent_id`), return `true`

Atomicity is guaranteed by the RwLock — only one writer at a time.

### Stale Claim Recovery

On startup, after `expire_stale_tasks()`:
1. `release_all_claims()` — remove all ClaimedBy edges
2. Transition any `Claimed` Questions to `Pending`

This handles crashes where an agent held a claim but never completed.

---

## 11. EventBus Broadcast Layer

### Design

`EventBus` wraps `tokio::broadcast::Sender<GraphEvent>` with a buffer of 256 events. It lives
inside `ConversationGraph` as an `Option<EventBus>` field marked `#[serde(skip)]`.

This follows the established pattern for runtime-only state: `responds_to` and `invoked_by` are
also `#[serde(skip)]` runtime indexes rebuilt on deserialization.

### GraphEvent Enum

```rust
#[derive(Debug, Clone)]
pub enum GraphEvent {
    MessageAdded { node_id: Uuid, role: Role },
    ToolCallCompleted { node_id: Uuid, is_error: bool },
    WorkItemStatusChanged { node_id: Uuid, new_status: WorkItemStatus },
    QuestionAdded { node_id: Uuid, destination: QuestionDestination },
    QuestionAnswered { question_id: Uuid, answer_id: Uuid },
    NodeClaimed { node_id: Uuid, agent_id: Uuid },
    GitFilesRefreshed { count: usize },
    ToolsRefreshed { count: usize },
    BackgroundTaskChanged { node_id: Uuid, status: TaskStatus },
    DependencyAdded { from_id: Uuid, to_id: Uuid },
}
```

Events are **semantic** (domain operations), not **structural** (field changes). Subscribers care
about "a question was answered" not "field `status` changed to `Answered` on node X."

### Emission

Each mutation method calls `self.emit(event)` after applying the change. `emit()` sends to the
broadcast if the bus is present, silently ignores errors (no subscribers = no-op). All event types
contain only `Copy` types (Uuid, enum variants, bool, usize) — clone cost is negligible.

### Subscribers

- **TUI**: Sets `dirty = true` on any event. Skips redundant redraws when no graph changes occurred.
- **Self-scheduling hook**: Receives QuestionAnswered and WorkItemStatusChanged events to check if
  blocked work has become ready.
- **Future agents**: Subscribe to discover new work without polling.

---

## 12. Self-Scheduling Loop

### The Core Loop

When an agent finishes (AgentEvent::Finished):

```rust
fn check_ready_work(&mut self) {
    let g = self.graph.read();

    // 1. Route pending questions
    let pending = g.pending_questions();
    drop(g);
    for q in pending {
        self.route_question(q.id());
    }

    // 2. Check for ready work items (DependsOn all resolved, not claimed)
    let g = self.graph.read();
    let ready = g.ready_unclaimed_nodes();
    drop(g);

    for node_id in ready {
        if self.active_agents.len() >= self.config.max_concurrent_agents {
            break; // at capacity
        }
        let mut g = self.graph.write();
        let agent_id = Uuid::new_v4();
        if g.try_claim(node_id, agent_id) {
            drop(g);
            self.spawn_agent_for_node(node_id, agent_id);
        }
    }
}
```

### The Graph IS the Work Queue

There is no separate scheduler state. Ready work is discovered by querying the graph:
- `ready_unclaimed_nodes()` returns nodes where all `DependsOn` targets are resolved and no
  `ClaimedBy` edge exists.
- `pending_questions()` returns Questions in `Pending` status.

This means the scheduling state is always consistent with the graph — no synchronization between
a scheduler and the graph needed. Adding a `DependsOn` edge or completing a WorkItem automatically
changes what's ready.

### Agent-to-Node Routing

When ready work is discovered, routing depends on node type:

```rust
fn route_ready_node(&mut self, node_id: Uuid) {
    let g = self.graph.read();
    match g.node(node_id) {
        Some(Node::Question { destination, .. }) => {
            // LLM questions claimed for primary agent — answered via `answer` tool.
            // User questions routed to TUI prompt.
            drop(g);
            self.route_question(node_id, *destination);
        }
        Some(Node::WorkItem { .. }) => {
            // Future: task execution with dedicated ContextPolicy.
            drop(g);
        }
}
```

---

## 13. Migration Strategy

### Graph Version: V3 → V4

New `Node` variants (Question, Answer) and `EdgeKind` variants (Asks, Answers, About, Triggers,
Supersedes, ClaimedBy) are **additive**. V3 graphs never contain these types, so serde
deserialization succeeds without transformation. The migration is a version number bump with no data
changes.

### Behavioral Equivalence

`ConversationalPolicy` must produce **identical output** to the current `extract_messages()`
function when given the same graph state. This is verified by a regression test that runs both
the old and new pipelines on a reference graph and asserts output equality.

This guarantee means the migration is invisible to existing users — the first deployment changes
internal structure without changing external behavior. New policies (TaskExecution,
QuestionResponse) add new behavior for new entry points.

### Startup Recovery

After loading the graph:
1. `expire_stale_tasks()` — mark running/pending tasks as Failed (existing)
2. `release_all_claims()` — remove all ClaimedBy edges (new)
3. Transition `Claimed` Questions to `Pending` (new)

---

## 14. Future

### GraphCoordinator Actor (Doc 07)

The current `Arc<RwLock<>>` model works for 2-5 agents. When scaling beyond ~10:

1. Extract `GraphCoordinator` actor owning `ConversationGraph` exclusively
2. `GraphHandle` wraps `mpsc::Sender<GraphCommand>` + `watch::Receiver<Arc<ConversationGraph>>`
3. Commands serialized via bounded mpsc — zero lock contention by construction
4. `watch` channel for TUI reads (zero-cost on unchanged frames)
5. EventBus moves into the coordinator (single emission point)

### Advanced Context Strategies

- **PCST subgraph extraction** (G-Retriever): Optimal connected subgraph maximizing relevance
- **Community-based expansion** (GraphRAG): Pre-computed community summaries for broad context
- **Embedding-augmented scoring** (doc 09): Combine topology with cosine similarity
- **Observation masking** (JetBrains): Replace old tool outputs with placeholders
- **Pull-based context extension**: `query()` method on ContextPolicy for mid-execution requests

### Compaction-Aware Rendering

When `CompactedMessage` nodes exist (VISION.md §4.2), the render stage selects the compaction
variant matching the current context perspective. A security review agent gets security-focused
compactions; a performance agent gets performance-focused compactions.

### LLM-Assisted Policy Tuning

Agents report which context sections were actually referenced in their output. This feedback trains
scoring weights and budget allocations over time, converging on empirically optimal policies per
agent role.
