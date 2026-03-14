# Graph-Based Context Building Strategies

> Research conducted 2026-03-14. Investigates strategies for constructing per-agent
> LLM context windows from a directed property graph, covering traversal algorithms,
> token budget management, relevance scoring, and event-driven context invalidation.

---

## 1. Executive Summary

The context-orchestrator must construct a different context window for each agent
invocation. Today, `extract_messages` (`src/app/context.rs:7-55`) walks the
`RespondsTo` ancestor chain and serializes messages linearly — adequate for a
single-agent chat, but insufficient for multi-agent orchestration where each agent
needs a purpose-specific view of the graph. This document surveys strategies for
building context from graphs, grounded in the system's existing architecture and
22 prior research documents.

**The core problem:** Given a directed property graph with 9 node types, 10 edge
types, and an anchor point (a WorkItem, Question, or user message), construct the
optimal subset of nodes to fill a token-budget-constrained context window that
maximizes the agent's ability to complete its assigned task.

**Key finding:** Context building decomposes into 5 orthogonal stages that can be
mixed and matched. No single strategy fits all agent types — the system needs a
**context policy** per agent role that configures each stage:

| Stage | What It Decides | Key Trade-off |
|-------|----------------|---------------|
| **1. Anchor** | Where traversal starts | Specificity vs. coverage |
| **2. Expand** | Which nodes to gather | Recall vs. noise |
| **3. Score** | Which nodes matter most | Precision vs. cost |
| **4. Budget** | What fits in the window | Completeness vs. focus |
| **5. Render** | How to serialize nodes | Fidelity vs. token efficiency |

**Recommendation:** A `ContextPolicy` trait with pluggable strategies per stage.
Phase 1: three hardcoded policies (Conversational, TaskExecution, BackgroundAnalysis).
Phase 2: graph-derived policy selection based on agent labels. Phase 3: LLM-assisted
policy tuning via feedback loops.

---

## 2. Current Architecture & Gap Analysis

### 2.1 What Exists Today

Context construction lives in two files:

**`src/app/context.rs`** — The main pipeline:
1. `extract_messages()` walks `RespondsTo` edges from branch leaf to root
2. Filters to `Message` + `SystemDirective` nodes only (all other 7 types skipped)
3. `build_plan_section()` injects active plan state into system prompt
4. `finalize_context()` counts tokens via API, truncates oldest messages if over budget
5. `sanitize_message_boundaries()` fixes orphaned tool results after truncation

**`src/app/plan/context.rs`** — Plan injection:
1. Finds all `WorkItem(Plan)` nodes with status ≠ Done
2. Recursively renders children with `SubtaskOf` edges
3. Shows `DependsOn` edges per plan
4. Injects as a `## Active Plans` section in the system prompt

### 2.2 Gaps for Multi-Agent

| Capability | Current State | Gap |
|-----------|--------------|-----|
| Anchor selection | Always branch leaf | Cannot anchor on WorkItem, Question, or arbitrary node |
| Neighborhood expansion | `RespondsTo` chain only | Ignores `DependsOn`, `About`, `RelevantTo`, `Invoked`/`Produced` edges |
| Node type inclusion | Messages + SystemDirective only | WorkItems, ToolResults, GitFiles, Memories never appear in context |
| Relevance scoring | None — all ancestors included | No way to prioritize relevant nodes over noise |
| Token budget allocation | Single budget, oldest-first truncation | No per-section budgets (e.g., 40% conversation, 30% tools, 20% plan, 10% memory) |
| Per-agent context | One context for all | Background rater and interactive coder get identical context |
| Context freshness | Static per request | No invalidation when graph mutates during long-running agents |

### 2.3 Existing Infrastructure to Reuse

- **Graph query methods** (`src/graph/mod.rs`): `get_branch_history()`, `children_of()`,
  `dependencies_of()`, `sources_by_edge()`, `nodes_by()` — foundation for expansion
- **Node mutation history** (`src/graph/mutation.rs`): `mutate_node` captures `NodeSnapshot`
  — timestamps available for recency scoring
- **Token counting** (`src/llm/mod.rs`): `count_tokens()` via Anthropic API — accurate
  but async (requires API call)
- **Tool result compaction** (`src/app/context.rs:83-150`): `build_assistant_message_with_tools`
  already pairs ToolCall/ToolResult — pattern for structured context building
- **Plan section builder** (`src/app/plan/context.rs`): recursive tree rendering with
  dependency display — pattern for injecting structured graph views

---

## 3. Requirements

Derived from VISION.md, existing research, and architectural constraints:

1. **Deterministic.** Same graph state + same anchor + same policy = same context.
   (VISION.md §3.2: "given the same graph state and anchor, the same context is produced")
2. **Graph-native.** Context is built by graph traversal, not by querying a separate index.
   Expansion follows typed edges, not keyword matching.
3. **Token-budget-aware.** Each context section has an allocated budget. Total never
   exceeds `max_context_tokens`. Budget allocation is policy-configurable.
4. **Agent-role-specific.** A code implementation agent, a review agent, and a
   background rater build different contexts from the same graph state.
5. **Incremental.** When one node changes, only affected context sections need
   rebuilding (not the entire context).
6. **Synchronous graph access.** Context extraction must work with a read-only graph
   snapshot (per doc 07's GraphCoordinator model — no holding locks during async work).
7. **Compaction-aware.** When `CompactedMessage` nodes exist (VISION.md §4.2), prefer
   the compaction variant matching the current context perspective.

---

## 4. Context Building Strategies — Full Taxonomy

### 4.1 Stage 1: Anchoring

The anchor determines *where* context building starts. Different anchors produce
radically different contexts from the same graph.

#### Strategy A: Branch Leaf Anchor (Current)

Start from the active branch's leaf node. Walk `RespondsTo` edges to root.
**Best for:** Interactive conversation (the current use case).
**Weakness:** Irrelevant for task-focused agents that don't participate in the conversation.

#### Strategy B: WorkItem Anchor

Start from a specific `WorkItem` node. Expand to: parent plan (via `SubtaskOf`),
sibling tasks, dependencies (via `DependsOn`), relevant messages (via `RelevantTo`),
and associated tool results.
**Best for:** Task execution agents that implement a specific plan task.
**Weakness:** May miss conversational context that explains *why* the task exists.

#### Strategy C: Question Anchor

Start from a `Question` node (per doc 21). Follow `About` edges to referenced nodes,
`DependsOn` edges to blocking questions, and `Asks` edges to originating tool calls.
**Best for:** Q/A routing agents and question-answering backends.

#### Strategy D: Multi-Anchor

Start from multiple anchors simultaneously and merge results. For example: the
current WorkItem + the most recent 3 messages + all pinned SystemDirectives.
**Best for:** Agents needing both task context and conversational awareness.
**Implementation:** Run expansion from each anchor independently, union the node sets,
then apply scoring and budgeting to the merged set.

### 4.2 Stage 2: Expansion

Expansion determines *which nodes* are reachable from the anchor. This is the core
graph traversal stage.

#### Strategy E: Ancestor Walk (Current)

Follow `RespondsTo` edges backward from anchor to root. Linear, O(depth).
Produces a strict conversation chain.

#### Strategy F: Typed Edge Fan-Out

From the anchor, follow specific edge types to a configurable depth. Different edge
types have different traversal priorities:

```
Priority 1 (always follow):  RespondsTo, SubtaskOf, DependsOn
Priority 2 (follow if budget allows): RelevantTo, About, Invoked/Produced
Priority 3 (follow only on demand): Indexes, Provides, Triggers
```

Each hop reduces the "relevance score" of reached nodes (see Stage 3). This is
a bounded BFS with edge-type-aware filtering.

**Implementation sketch:**

```rust
struct ExpansionPolicy {
    /// Which edge types to follow at each hop distance.
    edge_priorities: Vec<Vec<EdgeKind>>,
    /// Maximum hops from anchor.
    max_depth: u32,
    /// Maximum total nodes to gather before scoring.
    max_candidates: usize,
}
```

#### Strategy G: Subgraph Extraction (GraphRAG-Inspired)

Extract a connected subgraph around the anchor using a budget-aware BFS:
1. Start with anchor in the frontier
2. For each frontier node, compute edge-weighted scores for all neighbors
3. Add highest-scoring neighbors to frontier until node budget exhausted
4. Result: a connected subgraph optimized for relevance

This is the "Local Search" pattern from Microsoft GraphRAG (VISION.md §4.1), adapted
for our typed property graph. The edge weights come from:
- Edge type (DependsOn > RelevantTo > Indexes)
- Temporal recency (newer nodes score higher)
- Access frequency (frequently-referenced nodes score higher)

#### Strategy H: Dependency Closure

For task execution, compute the transitive closure of `DependsOn` edges from the
anchor WorkItem. Include all dependency nodes and their results. This gives the agent
everything it needs to understand *what must be true* before it can proceed.

Uses `has_dependency_path()` from `src/graph/mod.rs` which already handles cycle
detection.

#### Strategy I: Community-Based Expansion

Group related nodes into communities (clusters) using graph structure. When the anchor
belongs to a community, include the community summary and key members. This is the
"Global Search" pattern from GraphRAG.

**Implementation:** Detect communities via connected component analysis on `RelevantTo`
and `About` edges. Generate community summaries as `CompactedMessage` nodes during
background processing. Include the anchor's community summary in context.

**Phase:** 3+ (requires background community detection and summarization).

#### Strategy J2: Prize-Collecting Steiner Tree (PCST)

Formulate subgraph extraction as an optimization problem: find the connected subgraph
that maximizes total node prize values minus edge costs. Nodes receive prizes from
relevance scores (embedding similarity, graph distance, or rating); edges have costs
proportional to their semantic distance.

PCST naturally discovers "bridge" nodes that connect relevant but distant information,
preserving graph topology during extraction. The extracted subgraph is then serialized
for the LLM.

**Source:** G-Retriever (NeurIPS 2024, [arXiv 2402.07630](https://arxiv.org/abs/2402.07630)).
**Complexity:** NP-hard in general, but approximable with 2-approximation algorithms.
For our graph sizes (100-10K nodes), exact solvers via `pcst_rs` or heuristics suffice.
**Phase:** 2+ (requires node prize computation infrastructure).

#### Strategy J3: Steiner Tree Bridge Discovery (AriadneMem)

When an agent needs context from non-adjacent parts of the graph:
1. Identify terminal nodes (direct dependencies + query-relevant nodes)
2. Approximate Steiner Tree: find bridge nodes connecting disconnected terminals via
   `b* = argmax cos(E(query), v_m)` constrained to timestamps between endpoints
3. DFS path mining: chains up to L=3 hops, node budget 8-25 nodes, prioritized by
   path length and temporal coherence
4. Serialize subgraph + paths into context

**Source:** AriadneMem ([arXiv 2603.03290](https://arxiv.org/abs/2603.03290)).
**Performance:** 15.2% Multi-Hop F1 improvement, 77.8% runtime reduction over
iterative planning approaches.
**Relevance:** Bridge tasks connect seemingly unrelated but logically linked work
items — critical for understanding cross-cutting concerns.

#### Strategy J4: Coarse-to-Fine Exploration (GraphReader)

Agent-driven context expansion in two phases:
1. **Coarse scan:** Read atomic facts/summaries for all candidate nodes (cheap overview)
2. **Fine selection:** Selectively load full content of the most relevant nodes

The agent maintains a running notebook, updating notes at each step. Multiple
exploration paths use majority voting to resolve inconsistencies. Operates on 4K
token windows but matches GPT-4 128K on benchmarks.

**Source:** GraphReader (EMNLP 2024, [arXiv 2406.14550](https://arxiv.org/abs/2406.14550)).
**Relevance:** This is a **pull-based** expansion strategy — the agent decides what
context it needs rather than receiving a pre-built context. Addresses the red team's
critique about lack of pull-based strategies. Maps to an agent calling a `query()`
method on the ContextPolicy during execution.

### 4.3 Stage 3: Scoring

Scoring determines *how important* each expanded node is relative to the anchor.
Nodes below a threshold are pruned before budgeting.

#### Strategy J: Graph Distance Scoring

Score = 1.0 / (1.0 + distance_from_anchor). Nodes directly connected to the anchor
score ~0.5, two hops away ~0.33, etc. Simple, deterministic, zero-cost.

**Weakness:** Treats all edge types equally. A `Provides` edge (Tool → root) is
less semantically meaningful than a `DependsOn` edge (Task → Task).

#### Strategy K: Edge-Weighted Distance Scoring

Assign weights to edge types reflecting semantic strength:

| EdgeKind | Weight | Rationale |
|----------|--------|-----------|
| RespondsTo | 1.0 | Direct conversation thread |
| DependsOn | 0.9 | Strong causal relationship |
| SubtaskOf | 0.85 | Hierarchical containment |
| About | 0.8 | Contextual reference |
| Invoked/Produced | 0.7 | Tool provenance chain |
| RelevantTo | 0.6 | Weak topical association |
| Triggers | 0.5 | Causal but indirect |
| Indexes/Provides | 0.3 | Structural, not semantic |

Score = product of edge weights along the shortest path from anchor. A node two
`DependsOn` hops away scores 0.9 × 0.9 = 0.81; a node one `Provides` hop away
scores only 0.3.

#### Strategy L: Composite Scoring

Combine multiple signals with configurable weights:

```rust
struct ScoringPolicy {
    /// Weight for graph distance score (Strategy K).
    topology_weight: f32,
    /// Weight for temporal recency (newer = higher).
    recency_weight: f32,
    /// Weight for node type preference.
    type_weight: f32,
    /// Weight for access frequency (how often this node appeared in prior contexts).
    frequency_weight: f32,
}
```

This mirrors CrewAI's composite scoring (semantic 0.5 + recency 0.3 + importance 0.2)
but replaces "semantic" with "topology" since we don't use embeddings in Phase 1.

#### Strategy M: Relevance Rating Integration

When the multi-rater relevance system exists (VISION.md §4.4), incorporate stored
`Rating` nodes as an additional scoring signal. Developer ratings (confidence 1.0)
override computed scores. Background rater scores (confidence 0.6-0.8) supplement
topology-based scoring.

#### Strategy M2: Personalized PageRank

The dominant scoring algorithm in graph-based context extraction. Starting from anchor
nodes, PPR propagates relevance through the graph topology with a damping factor
(typically α = 0.85). Each node's score reflects its topological importance *relative
to the anchor*, not global importance.

**Key property:** Bidirectional PPR achieves O(1/ε) complexity regardless of graph
size — making it scalable to 100K+ node graphs where BFS-based scoring degrades.

**Implementation:** PPR is a single-pass algorithm that scores all reachable nodes at
once, addressing the red team's critique about per-node `score()` calls. A `score_batch`
implementation using PPR replaces N individual shortest-path computations with one
iterative diffusion. Available in `petgraph` via power iteration or the
`pagerank::pagerank` crate.

**Source:** Standard algorithm, validated across GraphRAG, Neo4j, and multiple knowledge
graph retrieval systems as the go-to relevance propagation method.

#### Strategy M3: GNN-Learned Scoring

Train a lightweight GNN (Graph Neural Network) or MLP to score node relevance to a
query. GNN-RAG scores answer candidates on knowledge graphs, then retrieves shortest
paths from query entities to top candidates. Outperforms competing approaches by
8.9-15.5% on multi-hop questions while using 9x fewer tokens.

**Source:** GNN-RAG (ACL 2025, [arXiv 2405.20139](https://arxiv.org/abs/2405.20139)).
**Phase:** 3+ (requires training data from context usage telemetry).
**Relevance:** Once we have telemetry on which context sections agents actually
reference, a lightweight MLP scoring model could replace hand-tuned edge weights.

### 4.4 Stage 4: Budget Allocation

Budget allocation determines *how much space* each content type gets within the
token limit.

#### Strategy N: Proportional Budget

Divide the token budget into fixed proportions per content section:

```rust
struct BudgetPolicy {
    /// Fraction of budget for system prompt (directives, memories, plan state).
    system_fraction: f32,       // e.g., 0.15
    /// Fraction for conversation history.
    conversation_fraction: f32, // e.g., 0.50
    /// Fraction for tool results and artifacts.
    tools_fraction: f32,        // e.g., 0.20
    /// Fraction for work item context (dependencies, descriptions).
    work_context_fraction: f32, // e.g., 0.15
}
```

Within each section, nodes are ordered by score (Stage 3) and accumulated until
the section budget is exhausted.

**Why proportional:** Research shows that beyond ~20% of context consumed by system
instructions, the instructions become noise (VISION.md §4.7). Fixed proportions
enforce this ceiling.

#### Strategy O: Priority-Based Greedy Allocation

Sort all candidate nodes by score. Greedily add nodes until the total budget is
exhausted, regardless of type. Higher-scored nodes get in; lower-scored nodes don't.

**Advantage:** Maximizes total relevance. A highly-relevant tool result beats a
low-relevance conversation message.
**Weakness:** Can produce unbalanced contexts (all tool results, no conversation)
which may confuse the LLM.

#### Strategy P: Tiered Allocation with Minimum Guarantees

Combine proportional and greedy: each section has a minimum guarantee and a maximum
ceiling. After minimums are filled, remaining budget is allocated greedily across
sections by score.

```rust
struct TieredBudget {
    sections: Vec<BudgetSection>,
}

struct BudgetSection {
    name: String,
    /// Minimum token allocation (guaranteed).
    min_tokens: u32,
    /// Maximum token allocation (ceiling).
    max_tokens: u32,
    /// Candidate nodes for this section, pre-sorted by score.
    candidates: Vec<ScoredNode>,
}
```

This prevents pathological cases while allowing flexibility. The system prompt always
gets at least 2000 tokens; conversation history always gets at least 30% of budget.

#### Strategy P2: Knapsack Optimization

Formalize context selection as the 0-1 knapsack problem:
- **w[i]** = token cost of node i
- **v[i]** = relevance score from Stage 3
- **W = L - P - R** where L = total context length, P = prompt tokens, R = reserved
  response tokens

**Algorithms:**
- Greedy approximation: O(N log N), select by highest value-to-weight ratio.
  Practical for real-time context assembly.
- Dynamic programming: O(N × W), optimal but pseudo-polynomial. Feasible when N
  and W are bounded (e.g., fewer than 200 candidate nodes with budget ≤ 200K tokens).

**Source:** [LLM Context as Knapsack Problem](https://www.awelm.com/posts/knapsack).
**Relevance:** Upgrades Strategy O (greedy) from an ad-hoc "sort by score and pack"
to a formally optimal selection. The value-to-weight ratio naturally penalizes verbose
nodes — a 500-token message scoring 0.5 has ratio 0.001, while a 50-token summary
scoring 0.4 has ratio 0.008 and gets selected first.

#### Strategy P3: Dynamic Budget Estimation (TALE)

Instead of static budget proportions, estimate the optimal token budget per task via
binary search. The TALE framework (ACL 2025) found:
- An "ideal budget range" exists per task minimizing cost
- Below the ideal range, token cost actually *increases* (counterintuitive)
- 68.64% average token reduction with only 2.72% accuracy loss (GPT-4o-mini)

**Source:** [TALE (arXiv 2412.18547)](https://arxiv.org/abs/2412.18547).
**Implementation:** Use the LLM's own estimate of needed tokens as the upper bound.
Binary search between minimum viable budget and this upper bound using a quality
validation signal (task completion rate).
**Phase:** 3 (requires quality feedback loop).

#### Strategy P4: Priority-Tiered Allocation

Hierarchical budget tiers inspired by production systems:

```
P0 (never cut):  Task instructions, schema, immediate inputs
P1 (trim last):  Direct dependency outputs, recent tool results
P2 (summarize):  Sibling/parallel task outputs
P3 (if space):   Broader graph context, community summaries
```

Within each tier, apply knapsack selection (Strategy P2). Monitor for pre-rot
threshold (~95% for Claude, ~256K effective for 1M advertised) and trigger compression
before performance degradation.

**Source:** Convergent pattern across LangChain, Anthropic, and production agent
systems. Claude Code triggers auto-compaction at 95% window capacity.

### 4.5 Stage 5: Rendering

Rendering determines *how* selected nodes are serialized into the LLM's input format.

#### Strategy Q: Verbatim Rendering (Current)

Serialize each Message node's content as-is. Tool results include full content.
WorkItems rendered as structured text.

#### Strategy R: Compaction-Aware Rendering

For each node, check if a `CompactedMessage` variant exists (linked via
`compacted_from` edge). Select the compaction level that fits the remaining budget:
- Full text if budget allows
- Light summary if budget is tight
- Aggressive summary if budget is very tight
- Metadata-only (title + type + timestamp) as last resort

This is the multi-perspective compaction from VISION.md §4.2. The *perspective* is
determined by the agent's role — a security review agent selects the security-focused
compaction; a performance agent selects the performance-focused compaction.

#### Strategy S: Structured XML Rendering

Render context in structured XML tags that help the LLM parse sections:

```xml
<system-context>
  <directives>...</directives>
  <active-plan>...</active-plan>
  <memories>...</memories>
</system-context>
<work-context>
  <current-task title="..." status="..." id="...">
    <description>...</description>
    <dependencies>...</dependencies>
  </current-task>
</work-context>
<conversation>
  <message role="user">...</message>
  <message role="assistant">...</message>
</conversation>
<tool-results>
  <result tool="read_file" file="src/main.rs">...</result>
</tool-results>
```

Anthropic's documentation recommends XML tags for structured prompt assembly. This
aligns with VISION.md §3.2 step 6: "Render: Serialize to the model's expected
format (XML tags, as Anthropic recommends)."

#### Strategy T: Progressive Detail Rendering

Render nodes at varying detail levels based on their score:
- Score > 0.8: full content
- Score 0.5-0.8: summary + key details
- Score 0.3-0.5: one-line summary
- Score < 0.3: excluded (or metadata-only reference)

This creates a natural "focus gradient" — the most relevant content is detailed,
peripheral content is summarized, distant content is briefly referenced. Mimics how
humans describe context: detail at center, broad strokes at periphery.

#### Strategy U: Observation Masking (JetBrains)

Replace older environment observations (tool outputs, file contents) with placeholder
references, while preserving the agent's own action/reasoning history in full.
Maintain a rolling window of N recent full observations (optimal: ~10 turns).

**Key finding:** JetBrains/NeurIPS 2025 showed observation masking matched or exceeded
LLM summarization in 4/5 configurations with ~50% cost reduction. LLM summaries
obscure stopping signals, causing agents to overshoot.

**Source:** [The Complexity Trap (NeurIPS 2025)](https://github.com/JetBrains-Research/the-complexity-trap).
**Implementation:** During rendering, nodes older than N turns with `ToolResult` or
`GitFile` content are replaced with `"[Tool result: read_file src/main.rs — see node {id}]"`.
The agent can request full content via a `query()` tool call if needed (pull-based).
**Relevance:** Directly addresses the budget problem — tool results often dominate
token consumption but are rarely referenced after the turn they were produced.

#### Strategy V: Sequential Accumulation (Chain of Agents)

For long-context tasks, divide work across a chain of worker agents. Each worker
receives its assigned chunk + a message from the previous worker. Information
accumulates through the chain. A manager agent synthesizes the final worker's output.

**Source:** Chain of Agents (NeurIPS 2024, [arXiv 2406.02818](https://arxiv.org/abs/2406.02818)).
**Performance:** Reduces complexity from O(n²) to O(nk) where k = context limit.
Up to 10% improvement over RAG/full-context baselines; ~100% on inputs >400K tokens.
**Relevance:** For tasks with many dependencies, instead of packing all dependency
outputs into one context, chain worker agents that each process one dependency's
output and pass a compressed summary forward.

---

## 5. Industry Approaches — How Others Solve This

### 5.0 The Canonical Framework: Write / Select / Compress / Isolate

LangChain and Anthropic converge on four canonical strategies for agent context:

| Strategy | Description | Example |
|----------|-------------|---------|
| **Write** | Persist info outside context for later retrieval | Scratchpads, long-term memories, plan files |
| **Select** | Pull relevant info into the active window | RAG, grep, knowledge graph retrieval |
| **Compress** | Reduce tokens while retaining essentials | Compaction (reversible), summarization (lossy) |
| **Isolate** | Split context across agents for parallelism | Sub-agents with focused context slices |

Priority hierarchy for compression: Raw > Compaction > Summarization. Multi-agent
isolation uses up to 15x more total tokens but each agent gets a narrow, focused slice.

The "Agent-as-Tool" pattern treats sub-agents as deterministic functions: main agent
calls `invoke_researcher(goal="...")`, harness spawns a temporary sub-agent loop
returning structured output. This eliminates management agent complexity and is the
pattern our `spawn_agent_loop` already follows.

Source: [LangChain Context Engineering](https://blog.langchain.com/context-engineering-for-agents/),
[Anthropic Context Engineering](https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents)

### 5.1 LangGraph: Centralized State with Reducer Merging

LangGraph's context model is a single `TypedDict` state object shared by all nodes.
Each node reads the full state, does its work, and returns a partial update that
gets merged via **reducer functions** (e.g., append to list, overwrite key).

**Subgraph isolation:** LangGraph supports private state per subgraph. Parent and
child graphs can have different state schemas. A wrapper node transforms parent state
to child state on entry and child results back on exit. Each subgraph gets a unique
namespace for checkpoint isolation.

**What we can learn:** The reducer pattern for merging partial context updates from
multiple agents. When Agent A updates tool results and Agent B updates plan state,
reducers define how to merge without conflicts. The subgraph isolation pattern maps
to our per-agent context policies — each agent gets a view (policy), not a copy.

**What doesn't fit:** LangGraph's state is flat (a dictionary), not a graph. LangGraph
0.4+ added structured state with nested schemas, but the fundamental model remains a
single shared dictionary rather than a typed property graph with multi-hop edges.

Source: [LangGraph State Management](https://sparkco.ai/blog/mastering-langgraph-state-management-in-2025),
[LangGraph Subgraphs](https://docs.langchain.com/oss/python/langgraph/use-subgraphs)

### 5.2 Microsoft GraphRAG: Two-Mode Graph Search

GraphRAG implements two context-building strategies from knowledge graphs:

**Local Search:** Start from a specific entity, fan out to K-hop neighbors, collect
entities + relationships + text chunks, rank by relevance, pack into context.
This maps directly to our Strategy G (Subgraph Extraction).

**Global Search:** Use pre-computed community summaries at multiple hierarchy levels.
For broad questions, include top-level summaries; for specific questions, drill into
relevant communities. This maps to our Strategy I (Community-Based Expansion).

**Key metric:** 71.2% accuracy at 1.6K tokens vs. 60.2% accuracy at 115K tokens
full-context (Graphiti/Zep benchmarks). Focused context *outperforms* full context.

Source: [GraphRAG Indexing Architecture](https://microsoft.github.io/graphrag/)

### 5.3 AutoGen: Message-Passing with Context Filtering

AutoGen 0.2's `AgentChat` layer manages per-agent context through:
- **Message filtering:** Agents subscribe to message types they care about
- **Context transforms:** `TransformMessages` middleware that can summarize, filter,
  or restructure messages before an agent sees them
- **Memory modules:** Pluggable memory (chat history, summarization, RAG)

AutoGen 0.4 redesigned its architecture with a `Memory` protocol and event-driven
agents, replacing the 0.2 `TransformMessages` approach with per-agent memory stores.

**What we can learn:** The middleware/transform pattern — context building as a
pipeline of composable transforms rather than a monolithic function.

Source: [AutoGen 0.2 Architecture](https://microsoft.github.io/autogen/0.2/docs/Use-Cases/agent_chat/)

### 5.4 CrewAI: Composite Scoring for Memory Retrieval

CrewAI retrieves agent memories using a composite score:
`score = 0.5 * semantic_similarity + 0.3 * recency + 0.2 * importance`

This maps to our Strategy L (Composite Scoring). The insight: no single signal
is sufficient. Topology alone misses recency; recency alone misses relevance.

Source: [CrewAI Memory System](https://docs.crewai.com/concepts/memory)

### 5.5 Temporal.io: Workflow State as Context

Temporal workflows carry state across activity executions. When a workflow resumes,
it replays the event log to reconstruct state. This is event sourcing applied to
workflow context.

**What we can learn:** State reconstruction from events. If our GraphCoordinator
(doc 07) maintains an event log, context can be rebuilt by replaying events that
affect the anchor's neighborhood — enabling time-travel context construction.

Source: [Temporal Event History](https://docs.temporal.io/encyclopedia/event-history)

### 5.6 Dagster: Asset-Centric Materialization

Dagster's "asset materialization" concept treats data assets as first-class citizens
with upstream/downstream dependencies. When an upstream asset changes, downstream
assets know they're stale and can re-materialize.

**What we can learn:** Context staleness detection. When a dependency node changes
(new answer to a blocking question, updated tool result), downstream agent contexts
become stale. The system should signal re-evaluation rather than serving stale context.

Source: [Dagster Asset Materialization](https://docs.dagster.io/guides/build/assets/)

### 5.7 Letta (MemGPT): Tiered Memory with Sleep-Time Compute

Letta organizes context into three tiers:
- **Core Memory:** Always in context, editable by the agent (< 2K tokens)
- **Archival Memory:** External store, retrieved on demand via tool calls
- **Recall Memory:** Recent interaction history, auto-managed

The "sleep-time compute" innovation processes memories during idle periods, improving
accuracy by up to 18% on math reasoning benchmarks (Stateful AIME).

**What we can learn:** The three-tier model maps to our rendering strategy. High-score
nodes go into the system prompt (Core), medium-score nodes are available via tool
calls (Archival), low-score nodes are summarized (Recall).

Source: [Letta Sleep-Time Compute](https://www.letta.com/blog/sleep-time-compute)

### 5.8 Blackboard Architecture (arXiv:2507.01701)

A 2025 paper on LLM blackboard systems found that agents communicating solely through
a shared blackboard (our graph) consumed fewer tokens and achieved competitive
performance vs. direct agent-to-agent messaging. The key: the blackboard *is* the
context — agents read what they need rather than receiving everything.

**What we can learn:** Pull-based context is more efficient than push-based. Agents
should query the graph for relevant context rather than receiving pre-built context
pushed to them. This argues for Strategy G (Subgraph Extraction) over Strategy E
(Ancestor Walk).

Source: [LLM Blackboard Architecture](https://arxiv.org/abs/2507.01701)

### 5.9 Google ADK: Session / State / Memory Separation

Google's Agent Development Kit mandates three distinct context layers:
- **Session:** Ephemeral, per-conversation interaction history
- **State:** Persistent key-value store scoped to the agent or session
- **Memory:** Long-term cross-session knowledge (semantic, episodic)

ADK enforces "minimum required context" — agents start with minimal state and pull
additional context via tool calls. This validates the pull-based expansion pattern
(Strategy J4) and the observation masking approach (Strategy U).

Source: [ADK Multi-Agent Patterns](https://developers.googleblog.com/developers-guide-to-multi-agent-patterns-in-adk/)

### 5.10 LEGO-GraphRAG: Optimal Module Combination (VLDB 2025)

LEGO-GraphRAG decomposes graph retrieval into interchangeable modules for subgraph
extraction and path retrieval, then benchmarks all combinations. **Key finding:**
structure-based extraction (PPR, k-hop) + semantic-augmented scoring (embedding
reranking) is the optimal combination — beating pure semantic or pure structural
approaches alone.

This validates our hybrid approach: graph-native expansion (Strategies F-I) for
recall, followed by optional embedding-based scoring (per doc 09) for precision.

Source: [LEGO-GraphRAG (VLDB 2025)](https://arxiv.org/abs/2504.00988)

---

## 6. Comparison Matrix — Strategy Combinations

| Agent Role | Anchor | Expansion | Scoring | Budget | Rendering |
|-----------|--------|-----------|---------|--------|-----------|
| **Interactive chat** | Branch leaf (A) | Ancestor walk (E) | Distance (J) | Proportional (N) | Verbatim (Q) |
| **Task implementer** | WorkItem (B) | Typed fan-out (F) + Dep closure (H) | Edge-weighted (K) | Tiered (P) | Structured XML (S) |
| **Code reviewer** | WorkItem (B) | Subgraph (G) | Composite (L) | Tiered (P) | Progressive detail (T) |
| **Background rater** | Target node (B) | Typed fan-out (F), depth=1 | Distance (J) | Small fixed (N) | Metadata + summary (T) |
| **Q/A responder** | Question (C) | About + DependsOn (F) | Edge-weighted (K) | Proportional (N) | Structured XML (S) |
| **Plan creator** | Multi-anchor (D) | Full graph scan | Composite (L) | Greedy (O) | Progressive detail (T) |

---

## 7. VISION.md Alignment

| VISION.md Section | How Context Strategies Align |
|--------------------|----------------------------|
| §3.2 Context Construction (6 steps) | Anchor → Expand → Select compaction → Order → Budget → Render maps exactly to our 5 stages (Select compaction is part of Render) |
| §4.1 Graph-Based Context | Strategies F, G, H, I are graph-native traversal patterns |
| §4.2 Multi-Perspective Compaction | Strategy R (Compaction-Aware Rendering) selects perspective-specific summaries |
| §4.3 Background Processing | Community detection (Strategy I) and scoring (Strategy M) run as background tasks |
| §4.4 Multi-Rater Relevance | Strategy M integrates stored ratings into scoring |
| §4.7 Developer Pinning | Pinned nodes get score 1.0, bypassing scoring stage |
| §5.4 Multi-Model (cost table) | Budget policies differ by model — Sonnet gets larger budgets, DeepSeek smaller |

---

## 8. Recommended Architecture

### 8.1 The ContextPolicy Trait

```rust
/// A context policy defines how to build an LLM context for a specific agent role.
/// Each method corresponds to one stage of the context-building pipeline.
pub trait ContextPolicy: Send + Sync {
    /// Stage 1: Determine the anchor node(s) for context traversal.
    fn anchors(&self, graph: &ConversationGraph, trigger: &ContextTrigger) -> Vec<Uuid>;

    /// Stage 2: Expand from anchors to gather candidate nodes.
    fn expand(&self, graph: &ConversationGraph, anchors: &[Uuid]) -> Vec<Uuid>;

    /// Stage 3: Score each candidate node for relevance to the task.
    fn score(&self, graph: &ConversationGraph, anchor: &[Uuid], candidate: Uuid) -> f32;

    /// Stage 4: Allocate token budget across content sections.
    fn budget(&self) -> BudgetAllocation;

    /// Stage 5: Render a node into a context fragment at the given detail level.
    fn render(&self, node: &Node, detail: DetailLevel) -> String;
}

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

/// Token budget allocation per content section.
pub struct BudgetAllocation {
    pub system_prompt: TokenRange,
    pub conversation: TokenRange,
    pub work_context: TokenRange,
    pub tool_results: TokenRange,
    pub memories: TokenRange,
    pub total: u32,
}

pub struct TokenRange {
    pub min: u32,
    pub max: u32,
}

/// How much detail to render a node with.
pub enum DetailLevel {
    Full,
    Summary,
    OneLine,
    MetadataOnly,
}
```

### 8.2 The Context Builder Pipeline

```rust
/// Build context for an agent using its assigned policy.
pub fn build_context(
    graph: &ConversationGraph,
    policy: &dyn ContextPolicy,
    trigger: &ContextTrigger,
) -> ContextResult {
    // Stage 1: Anchor
    let anchors = policy.anchors(graph, trigger);

    // Stage 2: Expand
    let candidates = policy.expand(graph, &anchors);

    // Stage 3: Score
    let mut scored: Vec<ScoredNode> = candidates
        .iter()
        .map(|&id| ScoredNode {
            id,
            score: policy.score(graph, &anchors, id),
        })
        .collect();
    scored.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(Ordering::Equal));

    // Stage 4: Budget
    let budget = policy.budget();
    let sections = allocate_to_sections(graph, &scored, &budget);

    // Stage 5: Render
    let context = render_sections(graph, policy, &sections);

    context
}
```

### 8.3 Phase 1: Three Hardcoded Policies

**ConversationalPolicy** — For interactive chat (replaces current `extract_messages`):
- Anchor: branch leaf
- Expand: ancestor walk (RespondsTo)
- Score: distance from leaf
- Budget: 15% system, 60% conversation, 15% tools, 10% work context
- Render: verbatim messages, structured XML for plan section

**TaskExecutionPolicy** — For agents executing work items:
- Anchor: assigned WorkItem
- Expand: typed fan-out (SubtaskOf, DependsOn depth 2, RelevantTo depth 1) +
  dependency closure + last 5 conversation messages for conversational awareness
- Score: edge-weighted distance
- Budget: 20% system + plan, 25% conversation, 25% tool results, 20% work context, 10% memory
- Render: structured XML with progressive detail

**BackgroundAnalysisPolicy** — For background raters and compaction agents:
- Anchor: target node
- Expand: 1-hop neighbors only
- Score: distance (trivial — all 1 hop)
- Budget: 5% system, 80% target + neighbors, 15% metadata
- Render: metadata + summary (compact)

### 8.4 How Policies Map to Agent Labels

Per doc 20's K8s-inspired scheduling, agents have labels. Context policies map to
label values:

```rust
/// Select context policy based on agent labels.
fn policy_for_agent(labels: &[Label]) -> Box<dyn ContextPolicy> {
    let role = labels.iter()
        .find(|l| l.key == "context-policy")
        .map(|l| l.value.as_str());
    match role {
        Some("conversational") => Box::new(ConversationalPolicy),
        Some("task-execution") => Box::new(TaskExecutionPolicy),
        Some("background") => Box::new(BackgroundAnalysisPolicy),
        _ => Box::new(ConversationalPolicy), // default
    }
}
```

### 8.5 Phased Delivery

**Phase 1: Refactor current context building into the pipeline.**
- Extract `ContextPolicy` trait and `build_context` pipeline
- Implement `ConversationalPolicy` that produces identical output to current
  `extract_messages` (behavioral equivalence — no new features yet)
- Add `TaskExecutionPolicy` with typed fan-out expansion
- Wire into `spawn_agent_loop` via `AgentLoopConfig`

**Phase 2: Scoring and budget allocation.**
- Implement edge-weighted distance scoring (Strategy K)
- Implement tiered budget allocation (Strategy P)
- Add `BackgroundAnalysisPolicy`
- Integrate with Memory nodes (doc 19) for memory section injection

**Phase 3: Advanced strategies.**
- Compaction-aware rendering (Strategy R) — requires CompactedMessage nodes
- Community-based expansion (Strategy I) — requires background community detection
- LLM-assisted policy tuning — agents report which context sections were useful
- Progressive detail rendering (Strategy T) — score-based detail levels

---

## 9. Integration Design

### 9.1 Data Flow

```
ContextTrigger (user message / task assignment / question routing)
  |
  v
Select ContextPolicy (from agent labels or default)
  |
  v
Stage 1: policy.anchors(graph, trigger) -> Vec<Uuid>
  |
  v
Stage 2: policy.expand(graph, anchors) -> Vec<Uuid>  [graph read-only snapshot]
  |
  v
Stage 3: policy.score(graph, anchors, candidate) -> f32 per node
  |
  v
Stage 4: policy.budget() -> BudgetAllocation
          allocate_to_sections(scored_nodes, budget) -> Vec<Section>
  |
  v
Stage 5: policy.render(node, detail_level) -> String per node
          assemble_sections() -> (Option<String>, Vec<ChatMessage>)
  |
  v
finalize_context(system_prompt, messages, provider, model, max_tokens, tools)
  |
  v
LLM API call
```

### 9.2 Reconciliation with GraphCoordinator (Doc 07)

Context building takes a **read-only snapshot** of the graph. It does NOT hold a
lock during async operations (token counting, API calls). The flow:

1. Agent receives task assignment via `GraphEvent`
2. Agent requests graph snapshot from GraphCoordinator via oneshot channel
3. `build_context()` runs synchronously on the snapshot (microsecond-scale)
4. `finalize_context()` runs asynchronously (token counting API call)
5. LLM API call proceeds with the finalized context
6. If the graph mutated during steps 3-5, the context is stale but safe —
   the worst case is slightly outdated information, not inconsistency

### 9.3 Reconciliation with Scheduler (Doc 20)

The scheduler assigns a WorkItem to an agent. The assignment includes context
metadata:

```rust
struct TaskAssignment {
    work_item_id: Uuid,
    agent_id: AgentId,
    context_policy: String,  // label value for policy selection
    context_trigger: ContextTrigger,
}
```

The agent loop reads the assignment, selects the policy, builds context, and proceeds.

### 9.4 Files to Modify

| File | Change |
|------|--------|
| `src/app/context.rs` | Refactor into `ContextPolicy` trait + pipeline; keep `extract_messages` as `ConversationalPolicy` |
| `src/app/context_policy.rs` | **New.** `ContextPolicy` trait, `ContextTrigger`, `BudgetAllocation`, `DetailLevel` types |
| `src/app/context_policies/mod.rs` | **New.** Module for policy implementations |
| `src/app/context_policies/conversational.rs` | **New.** Current behavior wrapped in trait |
| `src/app/context_policies/task_execution.rs` | **New.** WorkItem-anchored context building |
| `src/app/context_policies/background.rs` | **New.** Compact context for background agents |
| `src/app/agent_loop.rs` | Pass `ContextPolicy` to agent loop; use it instead of `extract_messages` |
| `src/graph/mod.rs` | Add graph traversal helpers: `neighbors_by_edge()`, `k_hop_neighborhood()` |

---

## 10. Red/Green Team

### Code Accuracy Audit

**All 10 primary claims verified accurate. Zero type conflicts.** Conducted by
independent code audit agent reading all referenced source files.

| Claim | Verdict |
|-------|---------|
| `extract_messages` at `context.rs:7-55` | Verified: function, signature, behavior all match |
| `build_plan_section` at `plan/context.rs:11-56` | Verified: recursive rendering, DependsOn display |
| `build_assistant_message_with_tools` at `context.rs:83-150` | Verified: ToolUse/ToolResult pairing |
| `Node` enum 9 variants at `node.rs:131-206` | Verified: all 9 names correct |
| `EdgeKind` 10 variants at `node.rs:106-118` | Verified: all 10 names correct |
| `dependencies_of`, `has_dependency_path` in `graph/mod.rs` | Verified: lines 251-276 |
| `sources_by_edge` at `context.rs:88` | Verified: exact match |
| Context.rs skips 7 non-Message node types | Verified: lines 37-43 |
| plan/context.rs recursive SubtaskOf rendering | Verified: `render_children()` lines 59-77 |
| All proposed types conflict-free | Verified: no `ContextPolicy`, `BudgetAllocation`, etc. in codebase |

### Green Team

**Codebase references:** All 12 verified accurate (see Code Accuracy Audit above).

**Framework claims verified via web search:**
- GraphRAG Local Search (entity fan-out) and Global Search (community summaries) — confirmed
- LangGraph reducer-based state merging via `TypedDict` + `Annotated` — confirmed
- CrewAI composite scoring weights (0.5 semantic + 0.3 recency + 0.2 importance) — confirmed
- Letta three-tier memory (Core/Archival/Recall) — confirmed. Sleep-time compute verified;
  the "18%" improvement is specifically on the Stateful AIME math benchmark, not general quality.
- Temporal event-replay state reconstruction — confirmed: "Durable Execution" via
  event history replay is core to Temporal's architecture.

**Graphiti accuracy claim:** 71.2% vs. 60.2% at 1.6K vs. 115K tokens confirmed from
the Graphiti paper (arXiv 2501.13956). Note: this is a knowledge QA benchmark, not
an agent task execution benchmark (see Red Team challenge 3b).

**Corrections applied:**
- Letta's "up to 18%" improvement: qualified as benchmark-specific (Stateful AIME), not
  general response quality.
- Neo4j "54.2% improvement" claim: sourced from Gartner data via Neo4j blog; this is
  an aggregate statistic across use cases, not a controlled experiment.

### Red Team

#### Critical (2)

**1. Embedding-based scoring contradicts doc 09's Phase 1 recommendation.**
Doc 09 (`09-embedding-based-connection-suggestions.md`) designs a full embedding
pipeline with `fastembed`, cascade evaluation, and `RelevantTo` edge suggestions —
recommended for Phase 1. Doc 22 replaces "semantic" with "topology" and defers
embeddings entirely. These documents contradict each other. If embeddings are Phase 1
in doc 09, they should at least be acknowledged in doc 22's scoring strategies.

*Resolution:* Valid contradiction. Strategy L (Composite Scoring) should include an
optional embedding signal alongside topology. The scoring interface should accept an
`Option<EmbeddingIndex>` — when available (per doc 09), embeddings supplement graph
distance; when absent, topology-only scoring applies. This preserves compatibility
with both documents.

**2. No pull-based or incremental context extension.**
Section 5.8 cites research showing "pull-based context is more efficient than
push-based" but every strategy in the document is push-based. The `ContextPolicy`
trait has no method for agents to request additional context mid-execution.

*Resolution:* Add a `query()` method to `ContextPolicy` for incremental context:
`fn query(&self, graph: &ConversationGraph, existing: &[Uuid], request: &str) -> Vec<Uuid>`.
Agents call this when they discover they need more context (e.g., reading a file
reveals a dependency on another file). Phase 1 can implement this as a no-op that
returns empty; Phase 2 implements graph-based search.

#### High (4)

**3. `score()` per-node interface forces O(N * path_cost) instead of O(E log V).**
The trait's `fn score(graph, anchors, candidate: Uuid) -> f32` requires N separate
calls for N candidates. If scoring requires shortest-path computation, a single
Dijkstra pass from anchors would score all nodes at once.

*Resolution:* Change the trait method to batch scoring:
`fn score_batch(&self, graph: &ConversationGraph, anchors: &[Uuid], candidates: &[Uuid]) -> Vec<(Uuid, f32)>`.
Implementations can then use a single BFS/Dijkstra pass. The per-node interface
remains available as a convenience default that delegates to `score_batch`.

**4. `render()` return type cannot express tool call/result pairing.**
Current code returns `(ChatMessage, Vec<ChatMessage>)` from
`build_assistant_message_with_tools`. The trait's `render(node, detail) -> String`
is a lossy simplification that cannot produce paired ToolUse/ToolResult blocks.

*Resolution:* Change render return type to `Vec<ChatMessage>` or a `RenderedFragment`
enum that can express multi-message structures. The pipeline's final assembly step
concatenates fragments respecting API constraints.

**5. `budget()` takes no parameters — cannot adapt to model or graph size.**
Different models have different context window sizes. A static `BudgetAllocation`
cannot scale. Additionally, `BudgetAllocation` has hardcoded section names — adding
a new section requires modifying the struct.

*Resolution:* Change to `fn budget(&self, max_tokens: u32) -> Vec<BudgetSection>`,
returning a dynamic list of sections. Use `Vec<BudgetSection>` (which Strategy P
already defines) instead of the rigid named-field struct. Sections are identified
by name strings, making the system extensible.

**6. No `sanitize()` or post-processing step in the pipeline.**
Current code has `sanitize_message_boundaries()` fixing orphaned tool results and
API structural invariants. This critical safety net has no place in the proposed
5-stage pipeline.

*Resolution:* Add a 6th stage: `fn post_process(messages: &mut Vec<ChatMessage>)`.
This runs `sanitize_message_boundaries()` and any policy-specific cleanup. The
default implementation calls the existing sanitization logic.

#### Medium (5)

**7. No cold-start policy for sparse graphs.**
On fresh projects with <50 nodes, scored expansion, budget allocation, and
multi-anchor strategies degenerate to "include everything." Edge-weighted scoring
is meaningless at 1-2 hops. Budget proportions give tiny allocations to sections
with no content.

*Resolution:* Add a `min_nodes_for_scoring` threshold to `ContextPolicy`. Below
this threshold, skip scoring and budget allocation — just include all gathered
nodes verbatim. This avoids wasted computation and produces better output for
small graphs.

**8. `access_frequency` scoring signal breaks determinism requirement.**
Requirement 1 demands identical output from identical graph state. But "how often
a node appeared in prior contexts" is mutable external state not captured in the
graph snapshot. Two runs with different access histories produce different contexts.

*Resolution:* Either store `access_count` on the node itself (making it graph state)
or remove access frequency from the deterministic scoring pipeline. Use it only as
a background signal for memory decay (doc 19), not for context selection.

**9. The 5-stage pipeline may be YAGNI for Phase 1.**
Only `ConversationalPolicy` has a consumer today. `TaskExecutionPolicy` and
`BackgroundAnalysisPolicy` have no callers in the current codebase. Building the
full trait + 8 new files + 3 policies when only 1 is used risks premature
abstraction.

*Resolution:* Phase 1 alternative: refactor `extract_messages` to accept an
anchor parameter and add a `score_nodes` step. Extract the full `ContextPolicy`
trait only when the second consumer (task execution agent) actually exists. The
strategies and types documented here remain the design vocabulary — implementation
follows demand.

**10. XML rendering token overhead never quantified.**
`<current-task title="Implement auth" status="active" id="abc-123">` consumes ~20
tokens of structural overhead per node. With 50 rendered nodes, XML scaffolding
alone could consume 1000+ tokens.

*Resolution:* Benchmark XML vs. plain-text rendering on sample contexts. If overhead
exceeds 5% of total budget, use lightweight delimiters (`---` sections, markdown
headers) instead of full XML. Reserve XML for sections where structure aids parsing
(tool results, plans).

**11. Graph-native bias — never compares against embedding-only baseline.**
The document never evaluates a simpler alternative: embed the WorkItem description,
find top-K similar nodes by cosine similarity, render them. No graph traversal, no
edge weights, no BFS. Doc 09 already has the embedding infrastructure designed.

*Resolution:* Add to Phase 2 evaluation: run the embedding-only baseline (doc 09)
alongside the graph-traversal pipeline on the same tasks. If embedding-only achieves
comparable quality with less complexity, it should be the default for sparse graphs.

### Summary of Audit Changes

| Finding | Severity | Resolution |
|---------|----------|------------|
| Embedding contradiction with doc 09 | Critical | Add optional embedding signal to composite scoring |
| No pull-based context extension | Critical | Add `query()` method to ContextPolicy trait |
| `score()` forces per-node calls | High | Change to `score_batch()` for single-pass algorithms |
| `render()` can't express multi-message structures | High | Return `Vec<ChatMessage>` or `RenderedFragment` |
| `budget()` takes no parameters | High | Accept `max_tokens`, return `Vec<BudgetSection>` |
| No post-processing/sanitization step | High | Add Stage 6 for `sanitize_message_boundaries()` |
| No cold-start policy | Medium | `min_nodes_for_scoring` threshold |
| Access frequency breaks determinism | Medium | Store on node or remove from scoring |
| YAGNI: full trait before second consumer | Medium | Phase 1 can use simpler refactor |
| XML token overhead | Medium | Benchmark and use lightweight delimiters if >5% |
| No embedding-only baseline comparison | Medium | Add to Phase 2 evaluation |
| Letta 18% is benchmark-specific | Low | Corrected in text |
| Neo4j 54.2% is aggregate, not controlled | Low | Noted in text |

---

## 11. Open Questions

1. **Should the ContextPolicy be per-agent-instance or per-agent-role?** Per-role means
   all Sonnet code agents share one policy. Per-instance allows an agent that has been
   working on auth for 20 minutes to build different context than a fresh agent.

2. **What is the right default max_candidates?** Too low (50) misses relevant distant
   nodes. Too high (500) wastes scoring cycles. Needs empirical measurement.

3. **Should conversation messages always be included?** A pure task execution agent
   (background compaction) may not need *any* conversation history. Should the
   `conversation_fraction` be 0 for some policies?

4. **How to handle context for long-running agents?** An agent running for 10 minutes
   accumulates tool results. Should these be incorporated into its own context
   incrementally, or should the agent re-build context periodically?

---

## 12. Key Algorithms Summary

| Algorithm | Use Case | Complexity | Source |
|-----------|----------|-----------|--------|
| Topological sort | Dependency ordering | O(V+E) | petgraph |
| Personalized PageRank | Anchor-relative relevance scoring | O(1/ε) | Standard |
| Leiden community detection | Hierarchical clustering | Near-linear | GraphRAG |
| PCST (Prize-Collecting Steiner Tree) | Optimal subgraph extraction | NP-hard (2-approx) | G-Retriever |
| Approximate Steiner Tree | Bridge node discovery | Polynomial | AriadneMem |
| DFS path mining | Reasoning chain extraction | O(V+E) per path | AriadneMem |
| Greedy knapsack | Token budget packing | O(N log N) | General |
| Binary search budget estimation | Optimal token budget | O(log B) | TALE |
| Token classification (LLMLingua-2) | Prompt compression | Linear | Microsoft |
| Observation masking | Long trajectory management | O(1) per step | JetBrains |
| Map-reduce scoring | Parallel context assessment | O(C × T) | GraphRAG |

---

## 13. Rust Ecosystem

### Graph Libraries

- **petgraph** ([crates.io](https://crates.io/crates/petgraph)): Rust's dominant graph
  library (2.1M+ downloads). `Graph`/`StableGraph`/`GraphMap` types. BFS/DFS iterators.
  14 built-in algorithms including `toposort` (O(|V|+|E|)), `Topo` for incremental
  topological traversal, `filter_map` for subgraph extraction.
- **daggy** ([crates.io](https://crates.io/crates/daggy)): DAG-specific wrapper around
  petgraph. Cycle detection, `Walker` trait for safe mutable traversal.

### Agent Frameworks

- **AutoAgents** ([GitHub](https://github.com/auto-agents-framework/auto-agents-framework)):
  Most mature Rust agent framework. Ractor-based with typed pub/sub. 43.7% lower
  latency than LangGraph, 84% higher throughput. Builder-pattern agent construction.
- **Rig** ([rig.rs](https://rig.rs)): Builder-pattern agent construction with static and
  dynamic (RAG'd) context. Strong embedding/vector store integrations.
- **graph-flow / rs-graph-llm**: LangGraph-inspired workflow execution with `Context`
  key-value store and PostgreSQL session persistence.
- **GraphRAG-rs**: Full Rust GraphRAG with Leiden community detection, HNSW indexing,
  and LightRAG integration.

### Tokenization

- **tiktoken-rs** ([crates.io](https://crates.io/crates/tiktoken-rs)): OpenAI tiktoken
  for Rust. Token counting and encoding.
- **bpe** ([crates.io](https://crates.io/crates/bpe)): Novel BPE algorithms for
  performance/accuracy.

### Relevance to Implementation

Our `ConversationGraph` currently uses `HashMap<Uuid, Node>` + `Vec<Edge>` (custom).
petgraph would add: BFS/DFS iterators for expansion, topological sort for dependency
ordering, and `filter_map` for subgraph extraction — all needed for Strategies F-I.
Migration to petgraph's `StableGraph` (stable indices across serialization) is viable
but requires converting our `Uuid`-based node identity to petgraph's `NodeIndex` system,
which the MVP explicitly deferred (doc MVP.md §6: "petgraph earns its place when we need
algorithms").

---

## 14. Sources

### Context Construction in Multi-Agent Systems
- [LangGraph State Management (2025)](https://sparkco.ai/blog/mastering-langgraph-state-management-in-2025) — Centralized state with reducer merging
- [LangGraph Subgraphs](https://docs.langchain.com/oss/python/langgraph/use-subgraphs) — Namespace-isolated child graph state
- [LangChain Context Engineering](https://blog.langchain.com/context-engineering-for-agents/) — Write/Select/Compress/Isolate framework
- [AutoGen v0.4 Architecture](https://www.microsoft.com/en-us/research/articles/autogen-v0-4-reimagining-the-foundation-of-agentic-ai-for-scale-extensibility-and-robustness/) — Event-driven Memory protocol
- [AutoGen 0.2 AgentChat](https://microsoft.github.io/autogen/0.2/docs/Use-Cases/agent_chat/) — Message filtering and context transforms
- [CrewAI Memory System](https://docs.crewai.com/concepts/memory) — Composite scoring (semantic 0.5 + recency 0.3 + importance 0.2)
- [CrewAI Flows](https://docs.crewai.com/en/concepts/flows) — Event-driven state machine with @start/@listen/@router
- [Temporal Event History](https://docs.temporal.io/encyclopedia/event-history) — Event-replay state reconstruction
- [Dagster Asset Materialization](https://docs.dagster.io/guides/build/assets/) — Upstream/downstream staleness detection
- [Google ADK Multi-Agent Patterns](https://developers.googleblog.com/developers-guide-to-multi-agent-patterns-in-adk/) — Session/State/Memory separation
- [DSPy](https://dspy.ai/) — Programmatic context optimization via GEPA

### Graph-Based Retrieval
- [Microsoft GraphRAG](https://microsoft.github.io/graphrag/) — Local Search (entity fan-out) and Global Search (community summaries)
- [LazyGraphRAG](https://www.microsoft.com/en-us/research/blog/lazygraphrag-setting-a-new-standard-for-quality-and-cost/) — 700x cheaper query cost via deferred summarization
- [LEGO-GraphRAG (VLDB 2025)](https://arxiv.org/abs/2504.00988) — Structure + semantic is the optimal combination
- [Graphiti/Zep Paper (arXiv 2501.13956)](https://arxiv.org/abs/2501.13956) — 71.2% accuracy at 1.6K tokens vs. 60.2% at 115K
- [Neo4j Knowledge Graph + LLM Multi-Hop Reasoning](https://neo4j.com/blog/genai/knowledge-graph-llm-multi-hop-reasoning/) — 54.2% average accuracy improvement
- [Context Graphs for AI](https://www.cloudraft.io/blog/context-graph-for-ai-agents) — Traversal patterns, hybrid retrieval

### Papers — Subgraph Extraction and Scoring
- [G-Retriever / PCST (NeurIPS 2024)](https://arxiv.org/abs/2402.07630) — Prize-Collecting Steiner Tree for optimal subgraph extraction
- [SubgraphRAG (ICLR 2025)](https://arxiv.org/abs/2410.20724) — Size-constrained extraction with DDE scoring
- [GNN-RAG (ACL 2025)](https://arxiv.org/abs/2405.20139) — GNN-learned relevance scoring, 9x fewer tokens
- [AriadneMem (2025)](https://arxiv.org/abs/2603.03290) — Steiner Tree bridge discovery + DFS path mining
- [GraphReader (EMNLP 2024)](https://arxiv.org/abs/2406.14550) — Coarse-to-fine graph exploration agent
- [Chain of Agents (NeurIPS 2024)](https://arxiv.org/abs/2406.02818) — Sequential accumulation, O(nk)
- [HiAgent (ACL 2025)](https://arxiv.org/abs/2408.09559) — Hierarchical working memory with subgoal chunking

### Papers — Token Budget and Compression
- [TALE / Token-Budget-Aware Reasoning (ACL 2025)](https://arxiv.org/abs/2412.18547) — 68.64% token reduction via binary search
- [LLMLingua-2 (ACL 2024)](https://arxiv.org/abs/2403.12968) — Token classification compression
- [The Complexity Trap (NeurIPS 2025)](https://github.com/JetBrains-Research/the-complexity-trap) — Observation masking beats LLM summarization
- [LLM Context as Knapsack Problem](https://www.awelm.com/posts/knapsack) — Formal 0-1 knapsack formulation

### Memory and Context Systems
- [Letta Sleep-Time Compute](https://www.letta.com/blog/sleep-time-compute) — Tiered memory with idle-time processing
- [Mem0 Architecture (arXiv 2504.19413)](https://arxiv.org/abs/2504.19413) — Layered episodic/semantic/procedural memory
- [Graph-Based Agent Memory Survey (2026)](https://arxiv.org/abs/2602.05665) — Taxonomy, storage structures, retrieval algorithms
- [LLM Blackboard Architecture (arXiv 2507.01701)](https://arxiv.org/abs/2507.01701) — Shared blackboard outperforms direct messaging

### Context Engineering
- [Anthropic Context Engineering Guide](https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents) — Right information at the right moment
- [Context Rot (Chroma Research)](https://research.trychroma.com/context-rot) — Focused 300 tokens outperform unfocused 113K
- [Event Sourcing for Agentic AI (Akka)](https://akka.io/blog/event-sourcing-the-backbone-of-agentic-ai) — Durable events, CQRS, inter-agent communication

### Rust Ecosystem
- [petgraph](https://crates.io/crates/petgraph) — Graph library (2.1M+ downloads)
- [daggy](https://crates.io/crates/daggy) — DAG-specific wrapper
- [AutoAgents](https://github.com/auto-agents-framework/auto-agents-framework) — Ractor-based agent framework
- [Rig](https://rig.rs) — Builder-pattern agent construction
- [tiktoken-rs](https://crates.io/crates/tiktoken-rs) — Token counting
- [Awesome-GraphMemory](https://github.com/DEEP-PolyU/Awesome-GraphMemory) — Resource collection

### Internal References
- `src/app/context.rs:7-55` — Current `extract_messages` pipeline
- `src/app/plan/context.rs:11-56` — Plan section builder
- `src/graph/node.rs:106-118` — EdgeKind enum (10 variants)
- `src/graph/node.rs:131-206` — Node enum (9 variants)
- `src/graph/mod.rs` — `dependencies_of()`, `has_dependency_path()`, `sources_by_edge()`
- `docs/VISION.md` §3.2 — Context construction (6 steps)
- `docs/VISION.md` §4.1-4.8 — Core ideas (graph, compaction, background, relevance, pinning)
- `docs/research/07-inter-agent-communication.md` — GraphCoordinator, three-layer architecture
- `docs/research/09-embedding-based-connection-suggestions.md` — Embedding pipeline, Phase 1 embeddings
- `docs/research/19-llm-memory-management.md` — Memory nodes and context injection
- `docs/research/20-kubernetes-inspired-agent-scheduling.md` — Agent labels, Filter+Score
- `docs/research/21-graph-scheduler-qa-relationships.md` — Q/A edges, self-scheduling
