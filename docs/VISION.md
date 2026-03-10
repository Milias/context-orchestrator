# VISION.md — Context Manager

> A context orchestration engine for software development with LLMs.

**Version:** 0.1.0-draft
**Date:** 2026-03-11
**Status:** Research & Design

---

## 1. Executive Summary

Context Manager is a Rust-based context orchestration engine that treats the software development conversation not as a linear chat log, but as a directed graph of interconnected knowledge. It deterministically constructs LLM inputs by traversing this graph, applying multi-perspective compaction, and surfacing precisely the information needed for each interaction.

**What makes it different:**

- **Graph-native context.** Every message, tool call, requirement, and work item is a node. Relationships are edges. Context is constructed by graph traversal, not by appending to a list.
- **Deterministic input construction.** The model sees exactly what the graph dictates — reproducible, auditable, version-controllable. No hidden prompt assembly.
- **Background processing.** Asynchronous processes compact, rate, and restructure the graph continuously, inspired by ClickHouse's MergeTree — write fast, optimize later.
- **Multi-perspective compaction.** The same message can have multiple compressed representations, selected based on the query context. A design discussion compacts differently when building the API versus writing tests.
- **Developer control.** Developers pin important nodes, rate messages, and shape the system prompt through direct interaction with the graph. The application IS the context orchestrator.

The market for AI-assisted development reached $7.37B in 2025 with ~85% developer adoption. No existing tool offers graph-based, deterministic, observable, versioned context construction. Context Manager positions itself as infrastructure — not a replacement for Claude Code or Cursor, but the engine that makes any LLM interaction more effective.

---

## 2. Problem Statement

### 2.1 Context Engineering is Hard

Context engineering — "the careful practice of populating the context window with precisely the right information at exactly the right moment" — is now recognized as a core discipline. Four strategies dominate: external memory/scratchpads, context trimming, isolation via multi-agent patterns, and compression/summarization. All current tools apply these ad-hoc.

The documented phenomenon of **context rot** — degrading model performance as context grows with poorly curated information — makes this urgent. Research shows a focused 300-token context often outperforms an unfocused 113,000-token context. The problem is not context window size; it is context window quality.

### 2.2 Current Tool Limitations

| Tool | Approach | Limitation |
|------|----------|------------|
| **Claude Code** | CLAUDE.md + 200K tokens | Manual context curation, no graph structure, linear conversation |
| **Cursor** | Local RAG with embeddings | Fragments at scale (70K-120K usable of 200K), opaque retrieval |
| **Aider** | CLI + deep Git integration | No persistent context model, linear chat |
| **ChatGPT/Claude chat** | Rolling conversation | No structure, no compaction, context eviction is invisible |

Common gaps across all tools:
- No graph-based provenance tracking
- No observable context construction (developers cannot see or control what the model receives)
- No context versioning or reproducibility
- No cost optimization beyond basic caching
- No cross-tool context sharing

### 2.3 Cost Pressure

Even with prices dropping ~80% from 2025 to 2026 (Claude Sonnet 4.5 at $3/$15 per million tokens, DeepSeek V3.2 at $0.14/$0.28), context waste multiplies cost. Prompt caching saves up to 90%, batch APIs add another 50% on top. Background processing with batch Sonnet costs ~$45/month; with DeepSeek, ~$4/month. These economics make aggressive background graph processing viable, but only if the system minimizes redundant token usage.

---

## 3. Core Concepts

### 3.1 The Graph Model

The fundamental data structure is a directed, typed property graph.

**Node Types:**

| Type | Description | Example |
|------|-------------|---------|
| `Message` | A developer or model utterance | "Implement the auth middleware" |
| `CompactedMessage` | A compressed variant of a Message | Summary of a 2000-token design discussion |
| `Requirement` | A high-level goal or constraint | "System must support offline mode" |
| `WorkItem` | A discrete unit of work (task, bug) | "Add rate limiting to /api/chat" |
| `ToolCall` | An invocation of an external tool | `read_file("src/main.rs")` |
| `ToolResult` | The output of a tool call | File contents, command output |
| `Artifact` | A produced asset | Generated code, diagram, document |
| `SystemDirective` | A pinned instruction | "Always use async/await, never spawn_blocking" |
| `Rating` | A relevance/quality assessment | Score: 0.85 for topic "authentication" |

**Edge Types:**

| Type | Semantics |
|------|-----------|
| `responds_to` | Message B responds to Message A |
| `compacted_from` | CompactedMessage derived from Message(s) |
| `requires` | WorkItem depends on Requirement |
| `subtask_of` | WorkItem hierarchy |
| `invoked` | Message triggered ToolCall |
| `produced` | ToolCall generated ToolResult/Artifact |
| `relevant_to` | Node rated relevant to a topic/WorkItem |
| `pinned_by` | Developer pinned this node |
| `supersedes` | Newer compaction replaces older one |

### 3.2 Context Construction

Building an LLM input is a graph traversal:

1. **Anchor:** Start from the current WorkItem or developer query.
2. **Expand:** Follow edges to gather relevant nodes — requirements, prior messages, tool results, pinned directives.
3. **Select compaction level:** For each gathered message, choose the compaction variant best suited to the current context (using topic-specific relevance ratings).
4. **Order:** Arrange selected nodes into a coherent sequence (topological sort with recency weighting).
5. **Budget:** Fit within the target token budget, pruning lowest-relevance nodes first.
6. **Render:** Serialize to the model's expected format (XML tags, as Anthropic recommends).

This is deterministic: given the same graph state and anchor, the same context is produced. The graph state is versioned.

---

## 4. Key Ideas

### 4.1 Graph-Based Context Management

**Description.** Replace the linear conversation log with a directed property graph. Every interaction, tool call, and work item is a node. Context is constructed by traversal, not concatenation.

**Prior Art.**
- **Microsoft GraphRAG**: Uses LLMs to extract entities/relationships, builds knowledge graphs, achieves 70-80% win rate vs. naive RAG on comprehensiveness. Two query modes: Global Search (community summaries for holistic questions) and Local Search (fan out from specific entities to neighbors).
- **Context Graphs**: A governed, queryable "memory layer" connecting entities, events, decisions, and policies — specifically engineered for AI consumption with token efficiency, relevance ranking, and provenance tracking.
- **LangGraph**: Agentic workflow framework using directed graphs for state, nodes (functions), and edges (connections). Persistent state management, multi-agent workflows. MIT-licensed.
- **Mem0**: Memory orchestration layer managing episodic, semantic, procedural, and associative memories for agents.
- **Neo4j + LLMs**: Combining graph databases with LLMs improves accuracy by 54.2% on average (Gartner data).
- **PROV-AGENT**: W3C PROV data model extended for agent systems, unified provenance graph recording tool invocations as workflow tasks.
- **Beads Viewer**: Graph-aware TUI for issue tracking that applies PageRank to prioritize nodes.

**Approaches.**
- **Durable master graph + query-specific subgraphs**: Maintain a persistent master graph; for each query, extract a targeted subgraph (neighborhood sampling). Pair with vector search within the subgraph for final ranking.
- **Community detection for hierarchical summarization**: Following GraphRAG — detect clusters of related entities, generate summaries at multiple hierarchy levels.
- **Explicit traversal rules**: Relevance edges, causality edges, dependency edges, temporal ordering.

**Green Team:**
- Graphs naturally model the non-linear structure of software development — requirements branch, conversations fork, tool calls create provenance chains.
- Enables context construction that is reproducible, debuggable, and version-controllable.
- Subgraph extraction makes multi-agent isolation trivial — each agent gets a view, not a copy.
- Graph algorithms (PageRank, shortest path, community detection) unlock context relevance scoring that linear systems cannot achieve.
- Enables **explainable decisions**: "Here's the subgraph we used to answer your question."

**Red Team:**
- Graph visualization breaks down at 1000+ nodes. Developers may struggle to navigate.
- Graph query performance can degrade with complex traversals — must benchmark early.
- Adds conceptual overhead: developers must think in graphs, not conversations.
- Schema evolution is harder for graphs than for flat data — node/edge types will change as the product evolves.
- Expensive graph maintenance: every new message requires entity extraction (LLM call), relationship detection, conflict resolution.
- Lost-in-the-middle problem doesn't vanish: even with perfect subgraphs, LLMs still fail on long contexts.

**Conclusion:** The graph model is the core differentiator and non-negotiable. Mitigate visualization complexity through layered views (current conversation, work item scope, full graph) rather than showing everything.

---

### 4.2 Multi-Perspective Message Compaction

**Description.** A single message or conversation fragment can have multiple compressed representations. When constructing context for a task about authentication, the compaction emphasizes auth-relevant details. The same source, compacted for a performance task, emphasizes latency and resource concerns. Compacted variants are graph nodes linked to originals via `compacted_from` edges.

**Prior Art.**
- **Personalized Summarization** (arXiv 2410.14545): LLMs can summarize the same meeting differently for different personas (Product Owner vs. Technical Lead vs. QA). Measurable viewpoint-specific differences.
- **Claude's Automatic Context Compaction**: At ~95% capacity, summarizes older messages. Lossy but identifies key points. Accuracy/recall degrade as token count grows ("context rot").
- **LLMLingua**: Up to 20x prompt compression with negligible accuracy loss via token-level pruning.
- **Verbatim compaction**: Delete low-signal tokens but keep survivors character-for-character identical (zero hallucination risk, lower compression).
- **Multi-Agent Debate for Evaluations**: Agents with different identities debate to reach consensus, fostering diverse interpretation.
- **Agentic Context Engineering (ACE)**: Context that self-updates based on model performance feedback.

**Critical finding:** "Aggressive compression silences minority viewpoints in long feedback collections." Hallucinations are "predictable compression failures" when information budgets fall below thresholds — 12.5% compression threshold before entering the "lossy zone" with imperfect recall.

**Approaches.**
- **Perspective indexing**: Tag messages with implicit topics (security, performance, UX). Generate multiple summaries per perspective. Store as nodes connected to original via edges.
- **Selective retention**: Different perspectives have different signal thresholds. Use multi-rater scoring to decide what to compress for whom.
- **Debate-based compression**: Spawn mini-agents with different viewpoints to jointly decide what's important. Synthesize consensus + minority-view summaries. Store both.
- **Tiered compression**: Full text → light summary → aggressive summary → metadata-only. Each tier is a node.

**Green Team:**
- Directly addresses context rot — compaction preserves signal, discards noise.
- Multi-perspective compaction is genuinely novel; no existing tool does this.
- Enables graceful degradation: as token budget shrinks, switch to more aggressive compactions rather than dropping messages entirely.
- Compaction quality is measurable — compare model performance with different compaction levels.
- Aligns with how humans think: same fact matters differently in different contexts.

**Red Team:**
- Compaction introduces information loss — bad compaction is worse than no compaction.
- Multiple variants per message multiply storage and processing costs (K perspectives = K x more summaries).
- Topic detection must be accurate; garbage topics yield garbage compactions.
- Compaction staleness: as the project evolves, old compactions may become misleading.
- Perspectives are hard to define objectively (who decides what matters?).

**Conclusion:** Start with two levels (full and single summary) before adding topic-specific variants. Use a validation loop: compare model outputs with compacted vs. full context to measure fidelity. Expire compactions after a configurable staleness window.

---

### 4.3 Background Graph Processing (MergeTree Analogy)

**Description.** Inspired by ClickHouse's MergeTree engine — which writes data quickly in unsorted parts, then merges and optimizes in the background — the Context Manager graph is append-first during active work. Background processes asynchronously compact messages, compute relevance ratings, detect topic clusters, prune stale nodes, and merge redundant information.

**Prior Art.**
- **ClickHouse MergeTree**: Data arrives as separate immutable parts. Background process continuously merges smaller parts into larger ones. Two sorted parts merge with a single linear scan (interleave rows, no re-sorting, no temporary buffers). Original parts marked inactive when no queries reference them. Result: write-optimized (append immediately), read-optimized (queries use few, large parts).
- **LSM-trees (RocksDB, LevelDB)**: Similar write-optimized + background compaction pattern.
- **Letta (MemGPT)**: Continual learning in token space — background processing to update agent memory.
- No known LLM tool applies this pattern to context management.

**Approaches.**
- **Tiered architecture**:
  ```
  Tier 1 (Hot):     Recent messages, active discussions — full fidelity
  Tier 2 (Warm):    Last week's work, occasionally queried — light summaries
  Tier 3 (Cold):    Archived decisions, rarely accessed — aggressive summaries
  Tier 4 (Archive): Historical record, search-only — metadata + embeddings
  ```
  Background processes migrate from Tier 1 → 2 → 3 → 4, compacting at each step.

- **Async compaction pipeline**:
  1. **Inbox**: Raw events arrive (messages, tool calls, results)
  2. **Analysis**: Background LLM extracts structure, detects clusters
  3. **Summarization**: Generate compactions (multi-perspective per Topic 4.2)
  4. **Scoring**: Rate relevance/importance for different contexts (per Topic 4.4)
  5. **Consolidation**: Write summary nodes, update edges
  6. **Cleanup**: Archive original parts if score is low, never delete

- **Background tasks as async Rust tasks** (tokio) with priority queues. Batch API calls for non-urgent processing (50% cost reduction). Configurable aggressiveness: idle time = aggressive processing; active coding = minimal background work.

**Green Team:**
- Developers get fast writes (low-latency interaction) while the graph self-optimizes.
- Background processing amortizes the cost of context engineering — developers do not wait for compaction.
- Cost-efficient: batch APIs and local models make continuous processing affordable ($4-45/month).
- The MergeTree analogy is intuitive for systems-oriented developers.
- Extends effective horizon: can handle multi-year projects without graph explosion.
- Enables time-travel: if you keep old parts around, you can replay decisions.

**Red Team:**
- Background mutations to the graph while the developer is working could cause confusion ("my message changed").
- Priority and scheduling of background tasks adds significant complexity.
- Resource contention: background LLM calls compete with foreground interactions for API rate limits.
- Stale compactions: summary might be based on incomplete data; later messages contradict it.
- MergeTree data is deterministic; LLM summaries are probabilistic — harder to version/rollback.
- Difficult to test — non-deterministic background processing complicates integration tests.

**Conclusion:** Essential for the vision to work at scale. Mitigate confusion by never mutating original nodes — compaction creates new nodes linked via edges. Use a clear visual indicator for "processing" status. Implement rate limiting to prevent background tasks from starving foreground requests.

---

### 4.4 Multi-Rater Relevance System

**Description.** Messages and nodes are rated for relevance to topics and work items by multiple sources: the developer (explicit), the primary LLM (during conversation), and background LLM evaluators (asynchronous). Ratings are themselves graph nodes, enabling meta-analysis of rating agreement and drift.

**Prior Art.**
- **LLM-as-a-Judge**: Production-proven framework. Prompt an LLM with input + output + scoring rubric. Modes: single output scoring, pairwise comparison, multi-judge aggregation. GPT-4 achieves 80%+ agreement with humans; humans among themselves ~80%.
- **Cross-Encoders**: BERT-based models that jointly encode query + document for precise relevance. ~0.5-2ms per pair on GPU. Cheaper than LLM judges but less flexible.
- **RankRAG**: Unified framework where LLM reranks retrieved contexts while generating answer. Outperforms naive retrieval + generation.
- **Cascade evaluation**: Cheap model first, escalate if uncertain. ~20-30% escalation rate, 70% cost savings vs. all-Sonnet.

**Calibration challenges:**
- Domain expertise gaps: LLM judges struggle in specialized domains — accuracy drops significantly.
- Surface-level fluency bias: LLMs overvalue surface quality and miss subtle errors.
- Solutions: anchor examples (pre-graded samples), few-shot learning, detailed rubrics, logprobs for confidence weighting, human validation sets (30-50 examples).

**Approaches.**
- **Developer ratings**: Thumbs up/down, explicit "this is important for X." One-click, optional, never blocking.
- **Primary model ratings**: After each response, rate which input nodes were most useful (a few extra tokens).
- **Background raters**: Periodically re-evaluate node relevance using cheaper models (DeepSeek, local Qwen).
- **Cascade evaluation**: Embedding similarity as fast first pass. If score > 0.9 or < 0.2, use it. If 0.2-0.9, escalate to LLM judge. Result: 70% of nodes evaluated for $0.001, 30% for $0.10.
- **Perspective-based judges**: "Is this relevant for SECURITY review?" / "PERFORMANCE review?" / "DESIGN review?" — separate scores per perspective.
- **Aggregate**: Weighted combination (developer > primary model > background raters).

**Green Team:**
- Multi-rater reduces single-point-of-failure in relevance assessment.
- Developer ratings capture tacit knowledge no model can infer.
- Enables data-driven compaction decisions — compact what's rated low, preserve what's rated high.
- Ratings over time reveal context drift — useful for long-running projects.
- Explainability: "Sonnet rated this 0.8 relevant to your query" is actionable feedback.

**Red Team:**
- Rating fatigue: developers will stop rating if it's not effortless.
- Model self-assessment of relevance is known to be unreliable.
- Storage overhead: ratings for every node x every topic x every rater scales combinatorially.
- Disagreement between raters requires a resolution strategy that may itself be wrong.
- Expensive to scale: 10K nodes x 3 judges = 30K LLM calls at $0.01 each = $300/batch.
- Feedback loops: judge learns from system outputs, circular reasoning.

**Conclusion:** Developer ratings as primary signal (one-click, optional). Model self-rating as cheap secondary. Background re-evaluation only for nodes that contribute to context construction (not the entire graph). Sparse representation — only store non-default ratings.

---

### 4.5 Non-Linear Developer Interface (Cell Model)

**Description.** Replace the linear chat interface with a Jupyter-cell-like model. Developers create, reorder, branch, and group cells. Each cell is a graph node. Cells can reference other cells, creating explicit dependency edges.

**Prior Art.**
- **Jupyter**: The canonical cell-based interface. Well-understood interaction model.
- **Mindalogue** (arXiv 2410.10570): Node + canvas mindmap for LLM interaction. Shows developers understand logical relationships through non-linear interaction. Documents that "linear interaction in current LLMs does not allow for flexible exploration."
- **Google NotebookLM**: Sources/Chat/Studio panels — document-centric AI.
- **Jupyter AI**: `%%ai` magic commands for LLM integration within notebooks.
- **Runcell**: AI-powered Jupyter notebook assistant that understands notebook structure.
- **Wolfram Notebook Assistant**: Conversational input → precise computational language.

**Approaches.**
- Cells as graph nodes with `responds_to`, `references`, and `follows` edges.
- Cell types: prompt, response, note, code, tool-output, system-directive.
- Layouts: linear (default, familiar), branching (for exploration), grouped (by work item).
- Timeline with branching: fork conversations, explore paths, merge learnings back.
- TUI: tabbed panes with cell list, detail view, and graph minimap.

**Green Team:**
- Solves a real problem: linear chat is terrible for multi-threaded exploration.
- Cell reordering lets developers curate the narrative the model sees.
- Grouping by work item naturally connects the interface to the graph structure.
- Familiar to anyone who has used Jupyter.
- Developers currently maintain *outside* context (Notion, Obsidian, CLAUDE.md). Pulling this into the coding loop eliminates context switching.

**Red Team:**
- Cognitive overload from non-linear layout. Research shows users struggle with graph visualization at scale. Cluttered screens exceed working memory capacity.
- Context ordering still matters for LLMs — cells must be linearized for the prompt, and the ordering algorithm is non-trivial.
- Adoption friction: most developers are trained on linear chat.
- Graph visualization is fundamentally hard in a TUI. No existing Rust TUI library handles this.
- Decision paralysis: "where should I put this cell?" adds friction that linear chat avoids.
- Search becomes critical — users will hate it if they can't quickly find what they said.

**Conclusion:** Start with a linear-by-default view that supports branching as an opt-in power feature. Do not force the graph on users. Use the graph internally for context construction, expose it gradually through features like "show related messages" and "branch from here." TUI graph visualization is a stretch goal — focus on cell list + detail view first.

---

### 4.6 Work Management Integration

**Description.** Work items (tasks, bugs, epics) live in the graph as first-class nodes, connected to requirements, conversations, and artifacts. This is not a project management tool — it is the graph's way of organizing context around purpose.

**Prior Art.**
- **Linear**: Local-first architecture, <50ms operations vs. Jira's 800-3000ms. Key insight: speed changes how people use the tool.
- **Dart**: "AI-native PM tool" (YC) — chat as UI, AI agents as collaborators.
- **Jira/GitHub Issues**: Rich but slow and disconnected from development context.

**Approaches.**
- Minimal work item model: title, status (todo/active/done), parent, description.
- Work items as graph anchors — "show me everything relevant to this task."
- Import from external systems (Jira, Linear, GitHub Issues) via API, creating graph nodes with `synced_from` edges.
- Do NOT build a full PM tool. No sprints, no story points, no burndown charts.

**Green Team:**
- Work items give the graph purpose — "what was I building this for?" is always answerable.
- Context construction anchored to work items is more focused than free-form conversation.
- Import-from-external avoids forcing developers to change their PM workflow.

**Red Team:**
- Scope creep is the existential risk. Building PM features is a black hole.
- Sync with external systems is fragile and maintenance-heavy.
- If work items are too lightweight, developers won't use them. If too heavy, they become a second Jira.
- Better as a Linear/Jira plugin than a standalone feature.

**Conclusion:** Work items are graph anchors, not a PM system. Keep the model minimal (title, status, parent). Prioritize import over native creation. The value is in connecting work items to context, not in managing work items. Resist every temptation to add PM features.

---

### 4.7 Developer Pinning & System Prompt Construction

**Description.** Developers explicitly mark nodes as important. Pinned nodes are always (or conditionally) included in context construction. The system prompt is not a static file — it is dynamically assembled from pinned nodes, ordered by priority and relevance.

**Prior Art.**
- **CLAUDE.md, .cursorrules, AGENTS.md**: Static files manually curated by developers. Crude but effective.
- **Windsurf**: User-generated + auto-generated memories with auto-cleanup.
- Research shows system prompts fade over long conversations — critical instructions must be re-injected dynamically.
- "Dynamically re-inject core objectives into each turn. Track key facts and current state, reinject into each turn like a mini knowledge base."

**Approaches.**
- **Pinning tiers**: System (always included), Conversation (included by default, can be overridden), Temporary (auto-unpins after N turns or time window).
- **Token budget tracking**: Display how much context window is consumed by pins. Warning at >20% — research indicates beyond this threshold, pinned content becomes noise.
- **Auto-cleanup**: Suggest unpinning nodes that haven't influenced model output in N interactions. Never auto-remove without confirmation.
- Pin from any node type: messages, tool results, code snippets, requirements.
- Export pins to CLAUDE.md format for interoperability with existing tools.

**Green Team:**
- Gives developers direct control over what the model "remembers."
- Dynamic assembly beats static files — pins can be scoped to work items, not global.
- Tiered pinning prevents the "everything is important" anti-pattern.
- Token budget visibility is a simple but powerful UX innovation.

**Red Team:**
- Pin management becomes a chore if the developer has many pins.
- Auto-cleanup may remove something the developer intended to keep.
- Dynamic system prompts are harder to debug than static CLAUDE.md files.
- Ordering pinned content for maximum model comprehension is an unsolved problem.

**Conclusion:** Three pinning tiers with clear token budget visibility. Auto-cleanup suggestions (never auto-removal without confirmation). Export to CLAUDE.md for interop. Start with manual ordering; explore model-assisted ordering later.

---

### 4.8 Tool Calls as First-Class Graph Citizens

**Description.** Every tool invocation — file reads, shell commands, API calls, code generation — is recorded as a `ToolCall` node with `ToolResult` child nodes. Arguments are stored as properties. This creates a complete provenance chain.

**Prior Art.**
- **PROV-AGENT**: W3C PROV data model. Tool invocations as `prov:used` (arguments) and `prov:generated` (results). Tool executions linked to LLM interactions via `prov:wasInformedBy`.
- **MCP (Model Context Protocol)**: 97M+ monthly SDK downloads. Universal standard for tool integration governed by the Agentic AI Foundation.
- **LangChain/LangGraph**: Explicit state management with tool calling, memory, human-in-the-loop.
- No current tool exposes tool call provenance as a queryable graph.

**Approaches.**
- Record tool calls with: name, arguments, timestamp, duration, token cost, result summary.
- Edge from triggering message to ToolCall (`invoked`), from ToolCall to ToolResult (`produced`).
- Compaction of tool results: full file contents compress to "read src/main.rs (245 lines, Rust, defines main() and Config struct)."
- MCP as the tool protocol — standardized tool discovery and invocation.
- Cost tracking per tool call enables optimization ("this file read costs 500 tokens every time — cache it").

**Green Team:**
- Complete audit trail: "why did the model read that file?" is always answerable.
- Enables context optimization: stale tool results can be refreshed or dropped.
- Cost analysis per tool call informs budgeting and optimization.
- MCP compatibility ensures broad tool ecosystem access.

**Red Team:**
- Tool call frequency causes node explosion — a typical coding session can invoke hundreds of tool calls.
- Storing full tool results (file contents, command outputs) is expensive in both storage and tokens.
- Provenance tracking overhead adds latency to every tool call.

**Conclusion:** Record all tool calls but apply aggressive result compaction. Store full results for a short window (configurable, default 1 hour), then compact to summaries. Index tool calls by file path and command for deduplication. MCP is the integration protocol — do not build custom tool adapters.

---

## 5. Technical Architecture

### 5.1 Language: Rust

Rust is chosen for:
- Performance: graph traversal and context construction must be sub-millisecond for interactive use.
- Memory safety: long-running background processes must not leak.
- Ecosystem: strong TUI libraries, embedded databases, async runtime.
- Single binary distribution: no runtime dependencies for end users.

### 5.2 Storage Stack

The storage layer is abstracted behind a trait boundary. The graph engine does not know whether its backing store is an in-memory structure, a local embedded database, or a remote service. This separation is a first-class architectural concern.

```
┌──────────────────────────────┐
│     Graph Engine (petgraph)  │  ← in-process, hot working set
├──────────────────────────────┤
│     Storage Trait            │  ← abstract interface
├──────┬───────────┬───────────┤
│ Local│  Local    │  Remote   │
│Embed │  File     │  Service  │
│(Cozo)│  (sled)   │  (gRPC/   │
│      │           │   HTTP)   │
└──────┴───────────┴───────────┘
```

| Layer | Technology | Purpose |
|-------|-----------|---------|
| Hot graph | `petgraph` | In-memory graph for active traversal and context construction |
| Persistent store (local) | `Cozo` | Embedded graph DB with Datalog queries for complex analysis |
| Snapshots (local) | `sled` or filesystem | Point-in-time graph state for versioning and undo |
| Persistent store (remote) | gRPC/HTTP client | Same trait, backed by a network service |
| Blob storage | Filesystem or object store | Large tool results, file contents, artifacts |

The single-user assumption simplifies this: no concurrent writers, no conflict resolution. But the trait boundary ensures the storage backend is swappable without touching the graph engine, context construction, or TUI layers.

### 5.3 TUI Framework

**Primary:** `ratatui` — the dominant Rust TUI framework, forked from tui-rs in 2023, actively maintained.

**State management:** `tui-realm` (Elm/React-inspired architecture) or custom, depending on complexity.

**Layout:**
- Left pane: work item tree / cell list
- Center pane: active cell / conversation
- Right pane: context inspector (what the model will see, token budget)
- Bottom: command palette / status bar

**Graph visualization:** Deferred. Text-mode graph rendering is an unsolved problem at scale. Start with list views; plan for optional web-based graph visualization via `xterm.js` or a lightweight local web UI as an escape hatch.

**Animation/polish:** `tachyonfx` (effects library for ratatui, 2026) for subtle feedback.

### 5.4 LLM Integration

| Use Case | Model | Cost | Latency |
|----------|-------|------|---------|
| Primary conversation | Claude Sonnet 4.5 / user choice | $3/$15 per M tokens | Real-time |
| Background compaction | DeepSeek V3.2 (batch) | $0.07/$0.14 per M tokens | Minutes |
| Local summarization | Qwen 14B (local) | $0 (compute only) | Seconds |
| Rating / classification | DeepSeek V3.2 or local | $0.07/$0.14 per M tokens | Background |

**Protocol:** MCP for tool integration. Direct API calls for LLM providers (Anthropic, OpenAI, DeepSeek, Ollama for local).

**Prompt caching:** Use Anthropic's prompt caching (90% cost reduction on cache hits) aggressively. The deterministic context construction makes cache hit rates predictable.

### 5.5 Cost Model

Monthly budget estimates for a single active developer:

| Activity | Volume | Model | Monthly Cost |
|----------|--------|-------|-------------|
| Primary conversation | 50M tokens/month | Sonnet 4.5 (cached) | ~$9 |
| Background compaction | 100M tokens/month | DeepSeek batch | ~$10 |
| Background rating | 50M tokens/month | DeepSeek batch | ~$5 |
| Local summarization | 200M tokens/month | Qwen 14B local | $0 |
| **Total** | | | **~$24/month** |

With aggressive prompt caching and batch processing, the system is cheaper than a single Cursor subscription ($20/month) while providing orders of magnitude more context intelligence.

---

## 6. Bootstrapping & Cold Start

Day one, the graph is empty. The system needs strategies to populate itself from existing project artifacts.

### 6.1 Git History Crawl

- Walk the commit log. Each commit becomes a node with metadata (author, date, message, files changed).
- Group commits by PR/merge request. Each PR becomes a WorkItem node.
- Extract file change patterns to build `relevant_to` edges between commits and files.
- Depth-configurable: last N commits, last N months, or full history.

### 6.2 Issue Import

- Import from Jira, Linear, or GitHub Issues via API.
- Each issue becomes a WorkItem node with status, description, labels.
- Link issues to PRs/commits via cross-references already in the issue tracker.
- Build `requires` and `subtask_of` edges from issue hierarchies.

### 6.3 PR/MR Summarization

- For each imported PR, use a background LLM call to generate a summary.
- Store as CompactedMessage nodes linked to the PR WorkItem.
- Extract key decisions, trade-offs, and unresolved questions.
- This is a batch operation — use batch API pricing ($1.50/M tokens with Sonnet).

### 6.4 Documentation Ingestion

- Scan `docs/`, `README.md`, `CLAUDE.md`, `.cursorrules`, `AGENTS.md`.
- Each document becomes a node. Sections become child nodes.
- Auto-pin project-level documentation as SystemDirective nodes.

### 6.5 Codebase Structure

- Walk the file tree. Create lightweight nodes for directories and key files.
- Use tree-sitter or LSP to extract module/class/function structure.
- Build `contains` and `depends_on` edges from imports and references.
- This is deterministic (no LLM needed) and fast.

### 6.6 Progressive Bootstrap

Not everything needs to happen at once:
1. **Instant** (no LLM): File tree, Git history metadata, issue metadata, documentation files.
2. **Minutes** (background LLM): PR summaries, issue summarization, documentation chunking.
3. **Hours** (background batch): Deep code analysis, cross-reference detection, relevance scoring.

The system is usable after step 1. Steps 2 and 3 improve context quality over time via the same MergeTree-inspired background processing used during normal operation.

---

## 7. Competitive Landscape

### 7.1 Existing Tools

| Tool | Strengths | Gaps |
|------|-----------|------|
| **Claude Code** | Deep model integration, 200K context, CLAUDE.md | Linear conversation, no graph, no compaction |
| **Cursor** | IDE integration, local RAG | Fragments at scale, opaque retrieval, no versioning |
| **Aider** | Git-native, CLI-first | No persistent context model, no background processing |
| **Continue** | Open source, multi-model | Early stage, no context engineering |
| **Windsurf** | Auto-generated memories | Proprietary, limited control, no provenance |

### 7.2 Market Gaps

No existing tool provides:
1. Graph-based context provenance
2. Multi-perspective message compaction
3. Observable, debuggable context construction
4. Context versioning and reproducibility
5. Cost-optimized background processing
6. Cross-tool context sharing via standard formats

### 7.3 Positioning

Context Manager is **infrastructure**, not an IDE or chat client. It is the engine that any LLM interaction can be built on top of. Initial form factor is a standalone TUI, but the graph engine should be usable as a library.

The wedge: developers who have hit the limits of linear chat and want control over what their models see. Power users first. Simplify for broader adoption later.

---

## 8. Risks & Open Questions

### 8.1 Technical Risks

| Risk | Severity | Mitigation |
|------|----------|------------|
| Graph performance at scale (>100K nodes) | High | Benchmark early with synthetic graphs; partition by work item |
| Compaction quality degrades model output | High | Validation loop: compare outputs with compacted vs. full context |
| Background processing races with foreground | Medium | Immutable nodes — compaction creates new nodes, never mutates |
| TUI graph visualization is unusable | Medium | Defer to web escape hatch; focus on list/tree views first |
| Storage backend abstraction leaks | Medium | Define trait early, test with both local and mock-remote backends |
| MCP protocol evolves incompatibly | Low | Thin adapter layer; MCP is stabilizing with 97M+ monthly downloads |

### 8.2 Product Risks

| Risk | Severity | Mitigation |
|------|----------|------------|
| Scope creep into PM territory | High | Hard constraint: no sprints, no story points, no burndown charts |
| Adoption friction from graph mental model | High | Linear-by-default interface; graph is internal, exposed gradually |
| Developer rating fatigue | Medium | One-click rating, never blocking, mostly model-driven |
| Competition from Claude Code adding similar features | Medium | Open source; focus on composability and extensibility |

### 8.3 Open Questions

1. **Graph schema versioning.** How do we migrate the graph when node/edge types change? Cozo's Datalog may help, but this needs a strategy before v1.
2. **Compaction validation.** What is the ground truth for "good compaction"? Downstream task success rate? Human evaluation? Both?
3. **Multi-model context differences.** Different models respond differently to the same context structure. Should context construction be model-aware?
4. **Privacy and security.** The graph contains complete development history including tool outputs. What is the threat model for local storage? For remote storage?
5. **Linearization algorithm.** When converting a subgraph to a prompt, what ordering maximizes model comprehension? Topological sort? Recency? Relevance score? Likely empirical.
6. **Token counting accuracy.** Different models use different tokenizers. Context budget calculations must be model-specific.
7. **When to compact.** Eager compaction wastes resources on messages that may never be retrieved. Lazy compaction risks slow context construction. The MergeTree analogy suggests lazy with background optimization — but the thresholds need tuning.
8. **Query formulation.** How does a developer ask the system questions? Keyword search + graph traversal? Natural language? Structured queries?

---

## 9. Sources

### Context Engineering
- [Context Engineering: The Definitive Guide (FlowHunt)](https://www.flowhunt.io/blog/context-engineering/)
- [Effective context engineering for AI agents (Anthropic)](https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents)
- [Context Rot: Increasing Input Tokens Impacts LLM Performance (Chroma)](https://research.trychroma.com/context-rot)
- [LLM Context Management Guide (16x Engineer)](https://eval.16x.engineer/blog/llm-context-management-guide)
- [Context Engineering in LLM-Based Agents](https://jtanruan.medium.com/context-engineering-in-llm-based-agents-d670d6b439bc)

### Graph & RAG
- [Microsoft GraphRAG](https://microsoft.github.io/graphrag/)
- [GraphRAG: Unlocking LLM discovery (Microsoft Research)](https://www.microsoft.com/en-us/research/blog/graphrag-unlocking-llm-discovery-on-narrative-private-data/)
- [Context Graphs: Practical Guide](https://medium.com/@adnanmasood/context-graphs-a-practical-guide-to-governed-context-for-llms-agents-and-knowledge-systems-c49610c8ff27)
- [Neo4j Knowledge Graph & LLM Multi-Hop Reasoning](https://neo4j.com/blog/genai/knowledge-graph-llm-multi-hop-reasoning/)
- [LangGraph: Multi-Agent Workflows](https://blog.langchain.com/langgraph-multi-agent-workflows/)
- [Glean: Knowledge Graphs in Agentic Engines](https://www.glean.com/blog/knowledge-graph-agentic-engine)

### Compaction & Summarization
- [Claude API: Compaction Docs](https://platform.claude.com/docs/en/build-with-claude/compaction)
- [Compaction vs Summarization (Morph)](https://www.morphllm.com/compaction-vs-summarization)
- [Predictable Compression Failures: Why Language Models Hallucinate](https://arxiv.org/abs/2509.11208)
- [Personalized Abstractive Multi-Source Meeting Summarization](https://arxiv.org/html/2410.14545v1)
- [LLMLingua: Prompt Compression (FreeCodeCamp)](https://www.freecodecamp.org/news/how-to-compress-your-prompts-and-reduce-llm-costs/)

### Background Processing
- [ClickHouse MergeTree Engine](https://clickhouse.com/docs/engines/table-engines/mergetree-family/mergetree)
- [Continual Learning in Token Space (Letta)](https://www.letta.com/blog/continual-learning)
- [Continuous Batching (HuggingFace)](https://huggingface.co/blog/continuous_batching)
- [LLM Deployment Cost-Benefit Analysis](https://arxiv.org/html/2509.18101v1)

### Relevance & Evaluation
- [LLM-as-a-Judge: Complete Guide (Evidently AI)](https://www.evidentlyai.com/llm-guide/llm-as-a-judge)
- [Evaluating LLM-Evaluators (Eugene Yan)](https://eugeneyan.com/writing/llm-evaluators/)
- [LLM-as-Judge Done Right (Kinde)](https://www.kinde.com/learn/ai-for-software-engineering/best-practice/llm-as-a-judge-done-right-calibrating-guarding-debiasing-your-evaluators/)
- [RankRAG: Unifying Context Ranking with RAG](https://arxiv.org/html/2407.02485v1)

### Developer Tools & UX
- [Mindalogue: Nonlinear LLM Interaction (arXiv)](https://arxiv.org/html/2410.10570v1)
- [ratatui: Rust TUI framework](https://github.com/ratatui/ratatui)
- [tui-realm: Stateful TUI framework](https://github.com/veeso/tui-realm)
- [Beads Viewer: Graph-aware TUI](https://github.com/Dicklesworthstone/beads_viewer)
- [CLAUDE.md Complete Guide](https://medium.com/data-science-collective/the-complete-guide-to-ai-agent-memory-files-claude-md-agents-md-and-beyond-49ea0df5c5a9)

### Provenance
- [PROV-AGENT: Unified Provenance (arXiv)](https://arxiv.org/html/2508.02866v1)
- [LLM Agents for Workflow Provenance (arXiv)](https://arxiv.org/html/2509.13978v2)

### Storage
- [petgraph: Rust graph library](https://github.com/petgraph/petgraph)
- [Cozo: Embedded graph DB with Datalog](https://lobste.rs/s/gcepzn/cozo_new_graph_db_with_datalog_embedded)
- [IndraDB: Rust graph database](https://github.com/indradb/indradb)

### Competitive Landscape
- [Claude Code vs Cursor (Qodo)](https://www.qodo.ai/blog/claude-code-vs-cursor/)
- [Best AI Coding Agents 2026 (Faros)](https://www.faros.ai/blog/best-ai-coding-agents-2026/)
- [LLM API Pricing 2026 (TLDL)](https://www.tldl.io/resources/llm-api-pricing-2026)
- [LLM Cost Optimization (Morph)](https://www.morphllm.com/llm-cost-optimization)

### Collaboration
- [CRDTs: What are they (Loro)](https://loro.dev/docs/concepts/crdt)
- [Conflict-Free Replicated JSON Datatype (arXiv)](https://arxiv.org/pdf/1608.03960)
- [Graph Data Projects: Why They Fail (Gemini Data)](https://www.geminidata.com/5-reasons-graph-data-projects-fail/)
