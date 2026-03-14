# LLM Memory Management for Graph-Based Context Orchestration

> Research conducted 2026-03-14. Investigates how existing systems manage LLM
> memories (episodic, semantic, procedural), and designs how memory nodes would
> integrate into the context-orchestrator's directed property graph.

---

## 1. Executive Summary

The context-orchestrator needs persistent memory — knowledge that survives across
conversation sessions and informs future context construction. The industry has
converged on three memory architectures: **layered storage** (Mem0, Letta),
**temporal knowledge graphs** (Zep/Graphiti), and **file-based memories** (Claude
Code, Cursor). None of these operate natively inside a directed property graph.

**Recommendation:** Add a single `Memory` node variant with a `MemoryKind` enum
(Episodic, Semantic, Procedural) and three new edge types (`ExtractedFrom`,
`Supersedes`, `ContextOf`). Memories are injected into the system prompt during
context construction, gated by a configurable token budget. Phase 1 is
user-driven (no LLM extraction cost); Phase 2 adds background extraction during
idle time following Letta's "sleep-time compute" pattern; Phase 3 adds
multi-perspective memories aligned with VISION.md's compaction model.

**Key trade-off:** Graph-native memories (our approach) sacrifice the retrieval
speed of dedicated vector DBs but gain provenance tracking, version history, and
unified graph traversal — the core differentiators of this project.

---

## 2. Current Architecture & Gap Analysis

### What Exists Today

The graph stores 9 node types and 9 edge types (`src/graph/node.rs:97-192`).
Context construction in `src/app/context.rs:7-53` walks the `RespondsTo` chain,
collecting only `Message` and `SystemDirective` nodes. All other node types
(WorkItem, GitFile, Tool, BackgroundTask, ThinkBlock, ToolCall, ToolResult) are
side-attached via non-`RespondsTo` edges and skipped during context assembly.

The `SystemDirective` node (`src/graph/node.rs:136-140`) serves as the root
system prompt — the only persistent "memory" today. There is no mechanism to:

- Remember facts learned during conversations across sessions
- Inject learned preferences into future prompts
- Track how memories evolve, contradict, or decay over time
- Scope memories to specific work items or topics

### Gap

| Capability | Current State | Gap |
|-----------|--------------|-----|
| Cross-session memory | None — each conversation starts fresh | No persistent knowledge store |
| Memory provenance | N/A | Cannot trace why the system "knows" something |
| Memory decay | N/A | No mechanism to age out stale knowledge |
| Scoped retrieval | N/A | Cannot filter memories by work item or topic |
| User memory control | N/A | No way to explicitly teach or correct the system |

### Existing Infrastructure to Reuse

- **Background tasks** (`src/tasks.rs:263-284`): `spawn_context_summarization` is
  an explicit no-op stub waiting for this work. Infrastructure for
  `BackgroundTaskKind`, `TaskMessage`, and `TaskStatusChanged` is fully wired.
- **Node mutation with history** (`src/graph/mutation.rs`): `mutate_node` captures
  `NodeSnapshot` before changes — gives free version history for memory updates.
- **Tool dispatch pipeline** (`src/app/task_handler.rs`, `src/tool_executor/mod.rs`):
  `ToolCallArguments` enum dispatch pattern works for adding `/remember` and
  `/forget` commands.
- **Migration system** (`src/migration.rs`): V1->V2->V3 chain with backup, version
  detection, and `VersionedGraph` tagged union — ready for V4.

---

## 3. Requirements

Derived from VISION.md and current architecture:

1. **Graph-native.** Memories must be graph nodes with typed edges, not a separate
   store. Provenance, versioning, and traversal must work identically to other nodes.
2. **Deterministic context injection.** Given the same graph state, the same
   memories must appear in the same prompt — matching VISION.md's deterministic
   construction principle.
3. **User control.** Developers must be able to explicitly create, view, update,
   and delete memories. Never auto-delete without confirmation (VISION.md §4.7).
4. **Scoped retrieval.** Memories must be filterable by work item, topic, or
   conversation — enabling the "show me everything relevant to this task" query.
5. **Background processing.** Memory extraction must run asynchronously during
   idle time, following the MergeTree "write fast, optimize later" principle.
6. **Token budget.** Memory injection must respect a configurable token cap to
   prevent context pollution. Research shows >20% of context consumed by
   persistent instructions becomes noise (VISION.md §4.7).
7. **Decay.** Memories must have lifecycle management — time-based expiry,
   access-frequency tracking, and supersession chains.

---

## 4. Options Analysis

### Option A: Layered Memory Store (Mem0/Letta Model)

**Description.** Three separate memory tiers — episodic (session summaries),
semantic (facts/preferences), procedural (workflows) — each with distinct storage
and retrieval characteristics. Memories are stored in a vector DB with embeddings
for semantic search.

**How Mem0 works.** Messages enter the conversation layer, then relevant details
are promoted to session or user memory based on metadata. Retrieval pulls from all
layers with rank prioritization (user > session > history). Integrates with
ElastiCache + Neptune Analytics at production scale. Memory types: episodic
(past interactions), semantic (concept relationships), procedural (how-to
knowledge). See [Mem0 paper](https://arxiv.org/abs/2504.19413).

**How Letta works.** Evolved from MemGPT. Organizes memory into Core (in-context
blocks manageable by the agent), Archival (externally stored persistent context),
and Recall (rapidly accessible long-term data). Their "sleep-time compute"
innovation runs a background agent that reorganizes memory during idle periods,
improving accuracy by up to 18% on specific benchmarks (Stateful AIME math).
Memory stored as git-backed markdown
files (MemFS). Sleep-time agents use git worktrees for conflict-free parallel
edits.

| Strength | Weakness |
|----------|----------|
| Proven at scale (Mem0 production deployments) | Requires external vector DB (not graph-native) |
| Natural tier-based decay | Separate retrieval pipeline from graph traversal |
| Sleep-time consolidation reduces response latency | Two storage systems to maintain |
| Episodic/semantic/procedural taxonomy is well-understood | Embedding generation adds cost per memory |

### Option B: Temporal Knowledge Graph (Zep/Graphiti Model)

**Description.** Build a knowledge graph from conversations via entity extraction.
Entities become nodes, relationships become edges with temporal validity intervals.
Retrieval combines semantic similarity, BM25, and graph traversal.

**How Graphiti works.** Three-tier subgraph: EpisodicNodes (raw messages),
EntityNodes (extracted entities with embeddings), CommunityNodes (cluster
summaries). Each edge carries a bi-temporal model: `valid_at`/`invalid_at`
(reality timeline) and `created_at`/`expired_at` (system timeline). Ingestion
requires multiple LLM calls per episode (~5+ pipeline stages): entity extraction
with reflexion, parallel entity resolution (full-text + cosine search, then LLM
disambiguation), fact extraction as hyper-edges,
temporal date resolution, contradiction detection via edge invalidation. Retrieval
is LLM-free: three parallel searches (cosine, BM25, BFS) fused via Reciprocal
Rank Fusion. P95 ~300ms. Accuracy 71.2% vs 60.2% full-context baseline, at 1.6k
tokens instead of 115k. Stored in Neo4j.
See [Graphiti paper](https://arxiv.org/abs/2501.13956).

| Strength | Weakness |
|----------|----------|
| Bi-temporal model handles contradictions elegantly | Multiple LLM calls per episode is expensive |
| LLM-free retrieval (fast) | Requires Neo4j (heavy external dependency) |
| Entity resolution prevents duplicates | Complex extraction pipeline |
| Graph-native (aligns with our model) | Entity extraction quality varies |

### Option C: File-Based Memories (Claude Code / Cursor Model)

**Description.** Memories stored as plain markdown files on disk. Loaded at
session start. User edits them directly.

**How Claude Code works.** Project memory in `~/.claude/projects/<project>/memory/`.
`MEMORY.md` is an index (first 200 lines loaded per session). Topic files contain
actual memories with frontmatter (name, description, type). Four memory types:
user (role/preferences), feedback (corrections/guidance), project (ongoing work),
reference (pointers to external systems). Memories are created both explicitly
(user says "remember this") and implicitly (system observes patterns).

**How Cursor works.** `.cursor/rules/*.mdc` files with four trigger types: always,
auto, agent-requested, manual. No native memory system — the community "Memory
Bank" pattern uses `.memory/` directory with structured markdown. Codebase index
uses tree-sitter chunks with 1536-dim embeddings.

| Strength | Weakness |
|----------|----------|
| Zero infrastructure (plain files) | No provenance tracking |
| User can edit with any text editor | No semantic search — relies on filename/description matching |
| Git-friendly (version controlled) | 200-line MEMORY.md limit forces aggressive curation |
| Simple mental model | No decay mechanism |

### Option D: Graph-Native Memory Nodes (Our Approach)

**Description.** Add `Memory` as a new node variant in the existing graph. Memories
link to their sources via `ExtractedFrom` edges, to work items via `ContextOf`
edges, and to each other via `Supersedes` edges. Retrieved during context
construction and injected into the system prompt.

| Strength | Weakness |
|----------|----------|
| Unified graph — one traversal engine for everything | No semantic search (substring/exact match only in Phase 1) |
| Full provenance via `ExtractedFrom` edges | Linear scan of Memory nodes for retrieval |
| Version history via existing `mutate_node` + snapshots | No embedding-based similarity |
| Deterministic injection (same graph state = same prompt) | Scale concern: 1000+ memories = slow scan |
| No external dependencies | Must build extraction pipeline from scratch |

### Option E: Hybrid (Graph Nodes + Embedding Index)

**Description.** Memory nodes live in the graph (provenance, lifecycle, edges).
A sidecar embedding index provides semantic search for retrieval. The graph is
the source of truth; the index is derived and rebuildable.

| Strength | Weakness |
|----------|----------|
| Best of both: provenance + semantic search | Two systems to keep in sync |
| Fast retrieval via embeddings | Embedding model dependency |
| Graph remains authoritative | Index rebuild on corruption adds complexity |
| Scales to 100K+ memories | Storage overhead (graph + vectors) |

---

## 5. Comparison Matrix

| Criterion | A: Layered | B: Temporal KG | C: Files | D: Graph-Native | E: Hybrid |
|-----------|-----------|---------------|----------|----------------|-----------|
| Graph-native | No | Partially | No | **Yes** | Yes |
| Provenance | Weak | Strong | None | **Strong** | Strong |
| Retrieval speed | Fast (vector) | Fast (hybrid) | Slow (file scan) | Slow (linear) | **Fast** |
| External deps | Vector DB | Neo4j | None | **None** | Embedding model |
| LLM cost to maintain | Medium | **High** (5+ calls/episode) | None | Low (Phase 1: zero) | Medium |
| Contradiction handling | Merge heuristics | **Bi-temporal** | Manual | Supersedes edges | Supersedes + temporal |
| Decay mechanism | Layered | Temporal invalidation | None | **Time + access** | Time + access |
| Implementation complexity | High | Very high | **Low** | Medium | High |
| Aligns with VISION.md | Partially | Partially | No | **Yes** | Yes |
| Deterministic injection | No (embedding ranking varies) | No | Yes (static) | **Yes** | No |

---

## 6. VISION.md Alignment

| VISION.md Principle | How Memory Design Aligns |
|--------------------|------------------------|
| Graph-native context (§3.1) | Memories are graph nodes with typed edges — same traversal as all other nodes |
| Deterministic input construction (§3.2) | Sorted by confidence -> access_count -> recency; same graph = same memories injected |
| Background processing / MergeTree (§4.3) | Phase 2 extraction runs during idle via existing `spawn_context_summarization` stub |
| Multi-perspective compaction (§4.2) | Phase 3: same source conversation produces different memories per topic |
| Multi-rater relevance (§4.4) | `confidence` field starts at extraction confidence, adjustable by user or background raters |
| Developer pinning (§4.7) | User-created memories with `confidence: 1.0` are effectively pinned |
| Tool calls as graph citizens (§4.8) | `/remember` and `/forget` are tools with full ToolCall/ToolResult provenance |
| Tiered architecture (§4.3) | Active -> Superseded -> Archived -> Deleted lifecycle maps to Hot -> Warm -> Cold -> Archive |

**Deviation:** VISION.md envisions `Rating` as a separate node type for multi-rater
scoring. Phase 1 uses a scalar `confidence` field on the Memory node instead.
This is a deliberate simplification — when the full Rating system arrives,
`confidence` becomes derived rather than stored. The deviation is temporary and
forward-compatible.

---

## 7. Recommended Architecture

### Phase 1: User-Driven Memory (no LLM cost)

**New types in `src/graph/node.rs`:**

```rust
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryKind {
    /// What happened — session summaries, event records
    Episodic,
    /// Facts, preferences, decisions
    Semantic,
    /// Workflows, patterns, how-tos
    Procedural,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryStatus {
    Active,     // eligible for context injection
    Superseded, // replaced by newer memory, kept for provenance
    Archived,   // decayed below threshold, not injected
    Deleted,    // user-deleted tombstone
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemorySource {
    UserExplicit,         // /remember command
    BackgroundExtraction, // Phase 2: idle-time LLM extraction
    SessionSummary,       // Phase 2: end-of-session episodic
    Imported,             // loaded from external source
}
```

**New `Node` variant:**

```rust
Node::Memory {
    id: Uuid,
    kind: MemoryKind,
    status: MemoryStatus,
    content: String,
    source: MemorySource,
    confidence: f32,           // 0.0-1.0
    access_count: u32,         // incremented on each context injection
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    expires_at: Option<DateTime<Utc>>,
}
```

**New edge kinds:**

```rust
EdgeKind::ExtractedFrom  // Memory -> source node (Message, ToolResult, WorkItem)
EdgeKind::Supersedes     // NewMemory -> OldMemory
EdgeKind::ContextOf      // Memory -> WorkItem (scoping)
```

**Context injection in `src/app/context.rs`:**

After `extract_messages` assembles the `RespondsTo` chain, a new
`collect_active_memories` step:
1. Filters Memory nodes where `status == Active`
2. If a current WorkItem exists, prefers memories with `ContextOf` edges to it
3. Sorts by: confidence desc -> access_count desc -> created_at desc
4. Accumulates until `max_memory_tokens` budget (default 2000) is spent
5. Increments `access_count` on injected memories
6. Formats as `<memories>` XML block appended to system prompt

**User tools:**

- `remember` tool: creates Memory node with `MemorySource::UserExplicit`,
  `confidence: 1.0`, `ExtractedFrom` edge to current conversation leaf
- `forget` tool: sets Memory status to `Deleted` (tombstone, not removed)
- Both follow existing `ToolCallArguments` enum dispatch in
  `src/tool_executor/mod.rs`

**TUI:** New `ContextTab::Memories` in context panel, rendering each memory with
kind, confidence, and truncated content.

**Migration:** V3 -> V4 (additive — new node variant, no data transformation).

### Phase 2: Background Extraction + Sleep-Time Compute

- New `BackgroundTaskKind::MemoryExtraction` replaces the
  `ContextSummarize` stub
- Triggers on idle (no user input for configurable N seconds) or session end
- Extraction prompt sends recent conversation turns, asks LLM to extract
  semantic and procedural memories as structured JSON
- Deduplication: before creating, check for semantic overlap with existing
  memories (substring first, LLM similarity if ambiguous)
- `Reinforces` and `Contradicts` edge kinds added for inter-memory links
- Access-based decay: background task proposes archival for memories with
  `access_count == 0` after 14 days. User confirms; never auto-delete.
- `SearchMemory` tool for agent-initiated memory queries

### Phase 3: Multi-Perspective + Tiered Storage

- Multi-perspective memories: same source conversation creates different
  memories per topic (aligns with VISION.md §4.2)
- Tiered storage: Hot (in-graph, active) -> Warm (SQLite, infrequent) ->
  Cold (metadata-only in graph, content in blob)
- Cross-conversation memory sharing via shared memory store
- Memory consolidation during idle: merge related episodic memories into
  single semantic memories (Letta sleep-time pattern)
- Batch API integration for cost-optimized background extraction

---

## 8. Integration Design

### Graph Topology with Memories

```
SystemDirective("You are helpful...")
    | RespondsTo
    v
Message(User: "Help me design auth")
    | RespondsTo
    v
Message(Asst: "I recommend JWT with RS256...")  <-- ThinkingOf -- ThinkBlock
    |                        |
    | RespondsTo             | ExtractedFrom
    v                        v
Message(User: "Use refresh   Memory(Semantic: "Project uses
 tokens too")                 JWT RS256 for auth")
    |                            | ContextOf
    | RespondsTo                 v
    v                        WorkItem("Auth middleware")
Message(Asst: "Here's the
 refresh token plan...")
    | ExtractedFrom
    v
Memory(Episodic: "Auth design session:
 JWT RS256 + refresh tokens decided")
```

### Context Construction Flow

```
                    +------------------+
                    | get_branch_history|
                    | (RespondsTo walk) |
                    +--------+---------+
                             |
                             v
                    +------------------+
                    | extract_messages  |
                    | (Message + Sys)   |
                    +--------+---------+
                             |
                             v
                    +------------------+     +-----------------+
                    |collect_active_   |---->| Memory nodes     |
                    |memories          |     | (filter, sort,   |
                    |                  |<----| budget)          |
                    +--------+---------+     +-----------------+
                             |
                             v
                    +------------------+
                    | Inject <memories>|
                    | into system      |
                    | prompt           |
                    +--------+---------+
                             |
                             v
                    +------------------+
                    | finalize_context |
                    | (token count +   |
                    | truncation)      |
                    +------------------+
```

### Memory Lifecycle State Machine

```
                  +----------+
   /remember --->|  Active   |<-- background extraction
   import ------>|           |
                  +----+-----+
                       |
              +--------+--------+
              v        v        v
        +----------+ +-------+ +---------+
        |Superseded| |Archived| | Deleted |
        |(new ver) | |(decay) | |(/forget)|
        +----------+ +-------+ +---------+
```

### Key Data Flow: `/remember` Command

1. User types `/remember Project uses JWT RS256 for auth`
2. Tool dispatch creates `ToolCallArguments::Remember { content, kind }`
3. Executor creates `Node::Memory` with `MemorySource::UserExplicit`
4. Adds `ExtractedFrom` edge to current conversation leaf node
5. If a WorkItem is active, adds `ContextOf` edge to it
6. Returns `ToolResultContent::Text("Memory saved: ...")`
7. On next LLM call, `collect_active_memories` picks it up

### Files to Modify

| File | Change |
|------|--------|
| `src/graph/node.rs` | Add `MemoryKind`, `MemoryStatus`, `MemorySource` enums; `Node::Memory` variant; `ExtractedFrom`, `Supersedes`, `ContextOf` edge kinds; extend `Node::id/content/created_at` match arms |
| `src/graph/mod.rs` | Re-export new types |
| `src/graph/tool_types.rs` | Add `ToolCallArguments::Remember` and `ToolCallArguments::Forget` variants |
| `src/graph/mutation.rs` | `update_memory_status`, `increment_access_count` methods |
| `src/app/context.rs` | Add `collect_active_memories` function; modify `extract_messages` to inject memories into system prompt |
| `src/tools.rs` | Register `remember` and `forget` tools in `tool_registry()` |
| `src/tool_executor/mod.rs` | `execute_remember`, `execute_forget` implementations |
| `src/tui/mod.rs` | Add `ContextTab::Memories` variant |
| `src/tui/widgets/context_panel.rs` | Render memories tab |
| `src/config.rs` | Add `max_memory_tokens` field |
| `src/migration.rs` | V4 migration (additive — version bump, V4Graph struct) |
| `src/tasks.rs` | Wire expiry checking into `spawn_context_summarization` |

---

## 9. Red/Green Team

### Green Team (Validations)

**All codebase references verified.** Nine file:line references audited against
source — all correct. The Node enum has exactly 9 variants, EdgeKind has exactly
9 variants, `extract_messages` behaves as described, `spawn_context_summarization`
is confirmed as a no-op stub, and the migration system is V1->V2->V3 as stated.

**Core factual claims verified.** Mem0 episodic/semantic/procedural types,
Mem0 paper (arXiv 2504.19413), Graphiti paper (arXiv 2501.13956), Graphiti
accuracy (71.2% vs 60.2% at 1.6k tokens), bi-temporal edge model, Letta MemFS
with git-backed markdown, Letta worktrees for parallel edits, Claude Code 200-line
MEMORY.md limit, CrewAI composite scoring weights, Cursor rules trigger types,
and Motorhead in Rust — all confirmed against primary sources.

**Corrections applied.** Letta's "18% improvement" is specifically on the Stateful
AIME math benchmark, not general response quality. Graphiti entity resolution uses
parallel (full-text + cosine) search followed by LLM disambiguation, not
sequential tiers. CrewAI v1.10+ unified separate memory types into a single
`Memory` class. Motorhead is deprecated (no longer maintained). Two academic paper
descriptions (arXiv 2512.12856, arXiv 2510.27246) had swapped summaries — fixed.

### Red Team (Challenges)

**Challenge 1: Single `Memory` variant risks becoming a God object.**

The document criticizes Beads' single `Issue` struct as a "God object" (§10
sources) then proposes a single `Memory` variant with 10 fields. When Phase 2
arrives and episodic memories need session boundaries, or semantic memories need
entity triples, or procedural memories need step sequences — all must be encoded
as `Option` fields or `MemoryKind`-specific branching.

*Counterargument:* The existing codebase has 15+ `match` arms on `Node`. Adding 3
new top-level variants would expand every match. The `BackgroundTaskKind` and
`ToolCallArguments` patterns already use enum-on-variant successfully. The `Memory`
variant's fields are genuinely shared across kinds (content, confidence, lifecycle
status). Kind-specific fields can be added as `Option`s without structural damage.
If Phase 2 proves this wrong, refactoring one variant into three is a smaller
change than consolidating three into one.

*Decision:* Keep single variant for Phase 1. Revisit at Phase 2 boundary.

**Challenge 2: System prompt injection may not be the right default for all kinds.**

System prompt content carries "constitutional" weight — the LLM treats it as
ground truth. Injecting volatile episodic memories ("last session we discussed
auth") alongside semantic facts ("project uses Rust") grants episodes an authority
level they haven't earned. Letta explicitly separates Core memory (system prompt,
small) from Archival memory (retrieved on demand, large).

*Decision:* Valid concern. Phase 1 should inject semantic memories into the system
prompt and episodic memories as a synthetic `[context]` block in the first user
message position. This matches how humans recall: facts are background knowledge,
episodes are "previously on...". The implementation design (section 7) should
document this distinction.

**Challenge 3: `access_count` creates a positive feedback loop.**

Memories injected into context get their `access_count` incremented, which
increases their rank, which ensures they keep being injected. High-access memories
become immortal — they can never reach `access_count == 0` for archival.

*Decision:* Use a rolling-window access rate (accesses in last 14 days) instead
of a monotonic counter. A memory accessed 100 times last month but 0 times this
week should lose rank. Track `last_accessed_at` timestamp rather than (or in
addition to) lifetime count.

**Challenge 4: No relevance filtering without embeddings is a critical gap.**

With 50 active memories about 10 topics, the user working on "auth middleware"
will have their token budget filled with memories about unrelated topics that
happen to rank higher. Without even basic relevance filtering, the system injects
noise.

*Decision:* Add a `tags: Vec<String>` field to Memory nodes, populated from
content keywords at creation time (user can override). During
`collect_active_memories`, compute tag overlap with recent messages and use it as a
filter gate before sorting. This is minimal code and transforms retrieval from
"dump everything" to "dump relevant things."

**Challenge 5: Cross-conversation persistence must be Phase 1, not Phase 3.**

Memories stored as nodes within a single `ConversationGraph` die with their
conversation. The document's gap analysis identifies cross-session memory as the
core problem, but the proposed architecture stores memories per-conversation.
Phase 3's "shared memory store" is the foundational requirement.

*Decision:* This is the most critical challenge. Phase 1 must include a
conversation-independent memory store. Options: (a) a shared `MemoryGraph`
alongside per-conversation graphs, or (b) memories stored in the analytics SQLite
DB (`src/storage/analytics.rs`), or (c) a dedicated `~/.context-manager/memories/`
JSON file. Option (c) is simplest and mirrors Claude Code's approach. The
conversation graph stores `ExtractedFrom` edges referencing memory IDs, but
memories themselves live in the shared store.

**Challenge 6: Privacy implications are completely absent.**

Persistent memory means the system remembers API keys, passwords, and PII
mentioned in conversations. The Windsurf cautionary tale is cited but its lessons
are not incorporated.

*Decision:* Add a `sensitive: bool` field. The `/remember` tool should check
content against common secret patterns (API keys, tokens, passwords) and warn
before saving. The `/forget` tool should support bulk deletion. Memory export
format should be documented for GDPR-style right-to-deletion.

**Challenge 7: User-created memories at confidence 1.0 are dogmatic.**

A user who types `/remember always use JWT` then switches to Paseto has a
confidence=1.0 memory that actively misleads until manually superseded.

*Decision:* Start user memories at 0.9 instead of 1.0. Reserve 1.0 for memories
confirmed by multiple sources (Phase 2). This gives background extraction a
chance to flag contradictions.

### Summary of Changes from Audit

| Finding | Severity | Resolution |
|---------|----------|------------|
| Letta 18% is benchmark-specific | Low | Corrected in text |
| Graphiti resolution is parallel | Low | Corrected in text |
| Paper descriptions swapped | Low | Corrected in sources |
| CrewAI unified memory in v1.10+ | Low | Noted in sources |
| Motorhead deprecated | Low | Noted in sources |
| Cross-conversation persistence | Critical | Must be Phase 1 (shared memory store) |
| No relevance filtering | High | Add `tags` field + tag-overlap gate |
| access_count feedback loop | High | Use rolling-window rate, not monotonic counter |
| System prompt injection for all kinds | Medium | Inject episodic as context block, not system prompt |
| User confidence at 1.0 | Medium | Start at 0.9, reserve 1.0 for confirmed |
| Privacy absent | Medium | Add `sensitive` flag + secret detection |
| Single variant vs. three | Medium | Keep single for Phase 1, revisit at Phase 2 |

---

## 10. Sources

### Memory Systems

- [Mem0: Memory Layer for AI Agents](https://mem0.ai/) — layered memory with
  episodic/semantic/procedural types
- [Mem0 Paper (arXiv 2504.19413)](https://arxiv.org/abs/2504.19413) — technical
  architecture details
- [Mem0 + AWS Integration](https://aws.amazon.com/blogs/database/build-persistent-memory-for-agentic-ai-applications-with-mem0-open-source-amazon-elasticache-for-valkey-and-amazon-neptune-analytics/) —
  production deployment with ElastiCache + Neptune
- [Letta (MemGPT successor)](https://www.letta.com/blog/memgpt-and-letta) —
  stateful AI agents with advanced memory
- [Letta Agent Memory](https://www.letta.com/blog/agent-memory) — core/archival/recall
  memory tiers
- [Letta Sleep-Time Compute](https://www.letta.com/blog/sleep-time-compute) — async
  memory consolidation during idle, up to 18% accuracy gain on math benchmarks
- [Letta Continual Learning](https://www.letta.com/blog/continual-learning) —
  tokens-to-weights distillation vision

### Knowledge Graphs for Memory

- [Graphiti (Zep) Paper (arXiv 2501.13956)](https://arxiv.org/abs/2501.13956) —
  temporal knowledge graph with bi-temporal edges, hybrid retrieval
- [Graphiti: Knowledge Graph Memory (Neo4j)](https://neo4j.com/blog/developer/graphiti-knowledge-graph-memory/) —
  P95 300ms retrieval, hybrid search architecture
- [Cognee: Memory Engine for AI Agents](https://www.cognee.ai/) — graph + vector
  hybrid with MCP support
- [Cognee Graph Memory Pipeline](https://memgraph.com/blog/from-rag-to-graphs-cognee-ai-memory/) —
  ingestion, enrichment, retrieval combining time/graph/vector

### File-Based Memory

- [Claude Code Memory System](https://code.claude.com/docs/en/memory) — CLAUDE.md +
  auto-memory with user/feedback/project/reference types
- [Claude Code Auto-Memory Analysis](https://medium.com/@joe.njenga/anthropic-just-added-auto-memory-to-claude-code-memory-md-i-tested-it-0ab8422754d2) —
  implicit knowledge accumulation across sessions
- [Claude Code Memory Architecture](https://institute.sfeir.com/en/claude-code/claude-code-memory-system-claude-md/) —
  200-line MEMORY.md limit, project-scoped persistence

### Agent Memory Frameworks

- [CrewAI Memory Types](https://docs.crewai.com/concepts/memory) — unified Memory
  class (replaced separate short/long/entity types in v1.10+) with composite
  scoring (semantic 0.5 + recency 0.3 + importance 0.2)
- [LangGraph Memory](https://docs.langchain.com/oss/python/langgraph/memory) —
  thread-scoped short-term + cross-session long-term via namespaced stores
- [LangMem SDK](https://blog.langchain.com/langmem-sdk-launch/) — episodic memory
  preserving successful interactions as learning examples
- [LangGraph Semantic Search](https://blog.langchain.com/semantic-search-for-langgraph-memory/) —
  vector similarity across PostgresStore, InMemoryStore
- [ChatGPT Memory](https://openai.com/index/memory-and-new-controls-for-chatgpt/) —
  implicit memory creation from conversations, user control via settings

### Academic / Research

- [Memory-Augmented Neural Networks (arXiv 2209.10818)](https://arxiv.org/pdf/2209.10818) —
  parametric vs ephemeral vs non-parametric memory taxonomy
- [Memory Consolidation in AI (arXiv 2504.15965)](https://arxiv.org/html/2504.15965v1) —
  append-then-summarize, incremental consolidation, sleep-inspired patterns
- [Privacy-Aware Memory and MaRS (arXiv 2512.12856)](https://arxiv.org/pdf/2512.12856) —
  forgetting policies (FIFO, LRU, Priority Decay) for privacy-aware agents,
  Memory-Aware Retention Schema
- [Adaptive RAG with Decay (arXiv 2601.02428)](https://arxiv.org/abs/2601.02428) — selective
  remembrance with frequency-based promotion/decay
- [ACT-R Dialogue Memory (ACM)](https://dl.acm.org/doi/10.1145/3765766.3765803) — vector-based
  activation with temporal decay + semantic similarity + noise
- [Long-Term Memory Benchmarks (arXiv 2510.27246)](https://arxiv.org/html/2510.27246) —
  benchmarking and enhancing long-term memory in LLMs beyond a million tokens
- [IBM Memory-Augmented LLMs](https://research.ibm.com/blog/memory-augmented-LLMs) —
  hippocampus (short-term) + neocortex (long-term) analogy

### Prior Art in This Project

- `docs/research/04-beads-agent-memory-system.md` — Beads tiered compaction (30/90 day),
  hash-based IDs, 19 dependency types, single Issue struct as God object
- `docs/VISION.md` §4.2 — multi-perspective compaction
- `docs/VISION.md` §4.3 — MergeTree-inspired background processing
- `docs/VISION.md` §4.4 — multi-rater relevance scoring
- `docs/VISION.md` §4.7 — developer pinning, token budget tracking
- `docs/design/01-graph-extensions-and-context-panel.md` — typed edges, new node
  types, background task infrastructure

### Tools & Infrastructure

- [Windsurf Cascade Memories](https://docs.windsurf.com/windsurf/cascade/memories) —
  auto-generated protobuf memories, stale memory problem
- [Windsurf Hidden Memory Bug](https://itstrategists.com/fixing-windsurf-auto-memory-how-hidden-memories-broke-my-projects-and-how-i-recovered) —
  cautionary tale of opaque memory formats
- [Motorhead: Rust Memory Server](https://github.com/getmetal/motorhead) — open-source
  Rust memory server with incremental summarization (deprecated, no longer maintained)
- [Cursor Rules System](https://docs.cursor.com/context/rules-for-ai) —
  `.cursor/rules/*.mdc` with four trigger types, no native memory
