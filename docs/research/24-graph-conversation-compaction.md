# Graph Conversation Compaction Strategies

> Research conducted 2026-03-14. Investigates how to compact graph nodes into
> compressed representations that preserve semantic content, enabling conversation
> rebuild from compacted state with on-demand detail expansion via edge traversal.

---

## 1. Executive Summary

The context-orchestrator currently handles token pressure by truncating oldest
messages from the front of the conversation (`src/app/context/sanitize.rs:35-57`).
This approach is **destructive** (information is permanently lost from the context
window) and **indiscriminate** (no relevance signal drives what gets dropped). A
2000-token design discussion about the authentication architecture is as likely to be
dropped as a 50-token acknowledgment message.

**The core idea:** Replace nodes in the conversation graph with compacted nodes
that preserve semantic content at reduced token cost. Original nodes remain in
the graph; compacted nodes link to them via `CompactedFrom` edges. When rebuilding
the conversation for an LLM call, the Render stage selects between full and
compacted representations based on relevance scores and token budget. Agents can
"zoom in" by following `CompactedFrom` edges back to originals for full detail.

This is not text summarization bolted onto a chat log. The graph structure makes
compaction a **first-class graph operation**: compacted nodes participate in
traversal, scoring, and rendering just like any other node. Multiple compaction
levels coexist for the same source. The `ContextPolicy` trait (already defined at
`src/app/context/policy.rs:62-90`) provides the `DetailLevel` enum (`Full`,
`Summary`, `OneLine`) that drives selection.

**Recommendation:** A new `CompactedMessage` node variant, a new `CompactedFrom`
edge kind, and a three-phase rollout:
1. **Phase 1:** Reactive single-summary compaction triggered by token budget pressure
2. **Phase 2:** Background MergeTree-style compaction with batch API processing
3. **Phase 3:** Multi-perspective compaction with tiered storage

**Key external findings:**
- Claude Code auto-compacts at ~95% capacity with structured summarization
- MemGPT/Letta uses two-tier memory with LLM-driven paging between tiers
- LLMLingua achieves up to 20x compression with minimal accuracy loss
- LongLLMLingua: 94% cost reduction on LooGLE benchmark; 21.4% performance boost
  with 4x fewer tokens on NaturalQuestions
- Proactive compaction (before emergency threshold) outperforms reactive-only

---

## 2. Current Architecture & Gap Analysis

### 2.1 What Exists Today

Context construction is a 3-step synchronous-then-async pipeline. The module was
recently refactored from a monolithic `context.rs` into a directory:

1. **`build_messages()`** (`src/app/context/policies/conversational.rs:82-129`):
   Walks `RespondsTo` edges from branch leaf to root. Collects only `Message` and
   `SystemDirective` nodes (all 9 other node types are skipped). Pairs
   ToolCall/ToolResult blocks via `build_assistant_message_with_tools()`.

2. **`finalize_context()`** (`src/app/context/sanitize.rs:12-32`): Async token
   counting via the LLM provider API. If `token_count > max_context_tokens`, calls
   `truncate_messages()` which computes a character ratio and `drain(0..remove_count)`
   from the front — removing entire messages from oldest first.

3. **`sanitize_message_boundaries()`** (`src/app/context/sanitize.rs:63-93`):
   Post-hoc repair — drops orphaned ToolResult messages, leading assistant messages,
   and trailing assistant messages with unpaired ToolUse blocks. This cleanup is
   necessary because truncation is structurally unaware of ToolCall/ToolResult pairing.

### 2.2 Gaps

| Capability | Current State | Gap |
|-----------|--------------|-----|
| Compaction nodes | Not in `Node` enum | VISION.md section 3.1 lists `CompactedMessage` but it does not exist in `src/graph/node.rs` |
| `compacted_from` edge | Not in `EdgeKind` | VISION.md section 3.1 lists it but `EdgeKind` has no equivalent |
| Truncation granularity | All-or-nothing per message | A 2000-token message is fully included or fully dropped; no intermediate |
| Tool result compaction | Full verbatim serialization | `build_assistant_message_with_tools()` renders ToolResult content as-is every time |
| Background compaction | No-op stub | `spawn_context_summarization` at `src/tasks.rs:263-285` creates task node but does zero work |
| Re-expansion | Impossible | Once truncated, content is gone for the current request — no mechanism to retrieve it |
| Compaction selection | Not applicable | No `DetailLevel`-driven rendering; `ContextPolicy` trait exists but Render stage is not integrated |

### 2.3 Existing Infrastructure to Reuse

- **`NodeSnapshot` / `mutate_node`** (`src/graph/mutation.rs:12-28`): Captures
  pre-mutation state with version history. Same immutable-original pattern applies.
- **`Supersedes` edge** (`src/graph/node.rs:154`): Used for Answer versioning.
  Similar structural pattern but different semantics (see section 6.1).
- **`BackgroundTaskKind::ContextSummarize`** (`src/graph/node.rs:80`): The enum
  variant already exists; wired into task spawning infrastructure.
- **`ContextPolicy` trait** (`src/app/context/policy.rs:62-90`): Defines the
  6-stage pipeline (Anchor, Expand, Score, Budget, Render, Sanitize) with
  `DetailLevel` enum — the compaction level selector.
- **`ConversationalPolicy`** (`src/app/context/policies/conversational.rs:18-78`):
  Working implementation of the trait — pattern for additional policies.

---

## 3. Requirements

Derived from VISION.md, the `ContextPolicy` design, and the user's framing:

1. **Non-destructive.** Original nodes are never mutated or deleted. Compacted nodes
   are new nodes linked via edges. (VISION.md section 8.1: "Immutable nodes —
   compaction creates new nodes, never mutates.")
2. **Graph-native.** Compacted nodes are first-class graph citizens. They participate
   in traversal, scoring, and rendering like any other node.
3. **Deterministic selection.** Same graph state + same anchor + same policy = same
   compaction level chosen for each node. (VISION.md section 3.2.)
4. **Re-expandable.** Agents follow `CompactedFrom` edges to retrieve full original
   content on demand — the "query upstream nodes for more detail" requirement.
5. **Pair-aware.** ToolCall + ToolResult pairs must be compacted as a unit. Compacting
   one without the other breaks the API's tool_use/tool_result contract.
6. **Background-capable.** Compaction runs as a background task via existing
   `BackgroundTaskKind` / `TaskMessage` infrastructure, following MergeTree's
   "write fast, compact later" pattern.
7. **Cache-compatible.** Compacted representations for a given node must be
   deterministic (same prompt → same output via temperature=0) so that prompt prefix
   stability is maintained across turns. Note: the codebase has no prompt caching
   integration yet — this is a forward-looking constraint.
8. **Multi-perspective ready.** Phase 1 produces one compaction per node. The
   architecture must support Phase 3's multi-perspective compaction (same source,
   different summaries per topic) without structural changes.

---

## 4. Options Analysis

### Option A: New `CompactedMessage` Node Variant

The VISION.md approach. A 12th variant alongside the existing 11:

```rust
pub enum CompactionLevel {
    /// ~25% of original tokens. Key decisions and outcomes preserved.
    Summary,
    /// ~10% of original tokens. One-paragraph essence.
    Aggressive,
    /// <50 tokens. Node type, timestamp, one-line description.
    MetadataOnly,
}

Node::CompactedMessage {
    id: Uuid,
    content: String,
    compaction_level: CompactionLevel,
    /// None for Phase 1 (generic). Some("security") for Phase 3 multi-perspective.
    perspective: Option<String>,
    /// original_tokens / compacted_tokens — for monitoring compression quality.
    compression_ratio: f32,
    original_token_count: u32,
    created_at: DateTime<Utc>,
}
```

New edge: `EdgeKind::CompactedFrom` — from `CompactedMessage` to original node(s).

**Strengths:** Clean type separation — `match` arms cannot confuse compacted and
original nodes. Multiple compactions per source (different levels, different
perspectives) are distinct nodes with distinct edges. Can compact any node type
(ToolResult, SystemDirective, not just Message). Exact match with VISION.md section 3.1.

**Weaknesses:** Every `match node` in the codebase needs a new arm. Current audit
found ~13 match statements across 9 files, including 10 methods on `Node` itself
(`id()`, `content()`, `created_at()`, etc.) plus TUI rendering in `message_style.rs`
and `context_panel.rs`. This is ~15 locations total — mechanical but non-trivial.

**Semantic concern:** Question/Answer are distinct *concepts*; `CompactedMessage` is
a *representation* of another node. A trait-based approach (see Option E) may be more
semantically honest, but adds architectural complexity that VISION.md did not
anticipate.

### Option B: Compaction as a Field on Existing `Message`

Add `compacted_content: Option<String>` and `compaction_level: Option<CompactionLevel>`
to the `Message` variant.

**Strengths:** Zero new match arms. The Render stage checks the field and uses
`compacted_content` when present.

**Weaknesses:** Bloats every `Message` with two fields that are `None` for >90% of
messages. Violates the codebase's pattern of semantic distinction via variants
(Question is a variant, not a `Message` with `is_question: bool`). Cannot represent
multiple compaction levels for the same message. Cannot compact non-Message nodes
(ToolCall, ToolResult, SystemDirective).

### Option C: Compaction as Edge Properties

Store compacted content as properties on a new `CompactedTo` edge from the original.

**Strengths:** No new Node variant.

**Weaknesses:** The current `Edge` struct has only `{from, to, kind}` — adding
properties is a structural change to `ConversationGraph`. Edges cannot be rendered
by the context pipeline — everything renderable is a node. Breaks the fundamental
pattern.

### Option D: Hybrid — Subgraph Variant + Field

New `CompactedSubgraph` variant for multi-node compaction (20 messages into 1
summary), field on `Message` for single-node.

**Strengths:** Subgraph compaction is the bigger win (compress 20 messages into 1).

**Weaknesses:** Two different mechanisms adds complexity. The field approach inherits
Option B's weaknesses. Inconsistent patterns confuse contributors.

### Option E: Trait-Based Renderable (Not a Node Variant)

CompactedMessage as a separate struct implementing a `Renderable` trait, stored
alongside the graph rather than inside it.

**Strengths:** Zero match arm changes on `Node`. Clean separation between "graph
data" and "rendering cache." Semantically honest — compaction is a view, not data.

**Weaknesses:** Compacted representations no longer participate in graph traversal
(cannot be scored, expanded, or linked via edges). Requires a separate storage
mechanism. Breaks VISION.md's architecture where "everything is a node." Cannot
support multi-perspective compaction via graph queries. Edge-based provenance
(`CompactedFrom`) becomes impossible without a node to anchor it.

---

## 5. Comparison Matrix

| Criterion | A: New Variant | B: Field | C: Edge Props | D: Hybrid | E: Trait |
|-----------|:-:|:-:|:-:|:-:|:-:|
| Match arm cost | High (~15 locs) | None | None | Medium | None |
| Multi-perspective | Yes | No | Yes | Partial | No |
| Non-Message compaction | Yes | No | Yes | Partial | Yes |
| Subgraph compaction | Yes (N:1 edges) | No | No | Yes | No |
| VISION.md alignment | Exact | Deviation | Major deviation | Partial | Major deviation |
| Edge struct change | No | No | Yes | No | No |
| Graph-native traversal | Yes | Partial | No | Partial | No |
| Pattern consistency | Follows variant pattern | Breaks it | Breaks renderable | Inconsistent | New pattern |

**Recommendation: Option A.** The ~15-location match arm cost is real but bounded and
mechanical. It is the only option that exactly matches VISION.md's architecture,
supports multi-perspective compaction, and keeps compacted nodes as first-class graph
citizens traversable by the existing `ContextPolicy` pipeline. Option E is
semantically appealing but sacrifices graph-native behavior.

---

## 6. Key Design Decisions

### 6.1 `CompactedFrom` vs Reusing `Supersedes`

**New `EdgeKind::CompactedFrom`.** `Supersedes` means "this node replaces the other
in all contexts" — a superseded Answer should never appear in context. `CompactedFrom`
means "this node is a lossy representation; both coexist and the Render stage selects
between them." Fundamentally different semantics: coexistence vs replacement.

### 6.2 How the Render Stage Chooses

The `ContextPolicy::render()` method receives a `DetailLevel` (`Full`, `Summary`,
`OneLine`). The Score stage assigns scores; the Budget stage maps scores to detail
levels. When `DetailLevel::Summary` is requested, the Render stage looks up whether
a `CompactedMessage` with `compaction_level: Summary` exists via:

```rust
graph.sources_by_edge(node_id, EdgeKind::CompactedFrom)
    .iter()
    .find(|&cid| matches!(graph.node(*cid),
        Some(Node::CompactedMessage { compaction_level: CompactionLevel::Summary, .. })))
```

If found, render the compacted content. If not, fall back to full node.

**Performance note:** `sources_by_edge` is O(|edges|) per call. For large graphs
this lookup in the Render stage (called per selected node) may need an index.
Mitigation: a `HashMap<(Uuid, EdgeKind), Vec<Uuid>>` runtime index, built on
deserialization like `responds_to` and `invoked_by` already are.

### 6.3 Agent "Zoom In" (Re-expansion)

When an agent receives a compacted summary and needs more detail, it issues a tool
call (e.g., `expand_context`). The executor follows `CompactedFrom` edges from the
compacted node to its sources, retrieves the full content, and returns it. Analogous
to MemGPT/Letta's "paging in" from archival memory.

**Agent awareness:** Compacted content in the rendered context must be visually
marked (e.g., `[compacted — use expand_context for full detail]`) so the agent knows
expansion is available. Without this marker, the agent may assume it has full
information and make decisions based on incomplete summaries.

### 6.4 Compaction Triggers — Event-Driven Model

Compaction is **mandatory**, not a fallback. When context is full, nodes must be
compacted — we cannot simply truncate and lose information. This requires an
event-driven architecture that avoids holding the graph lock during async LLM calls.

**The flow:**

1. **Detect:** The Budget stage (under read lock) determines that selected nodes
   exceed the token budget and identifies which nodes need compaction.

2. **Request:** The pipeline emits a graph event marking those nodes for compaction.
   This is a lightweight graph mutation — adds a `BackgroundTask` node (kind:
   `ContextSummarize`) with `DependsOn` edges to the nodes that need compacting.
   The read lock is released.

3. **Compact:** An async compaction worker (subscribed to graph events via the
   `EventBus`) picks up the task. It acquires a write lock, reads the target nodes,
   releases the lock, calls the LLM to generate summaries, then re-acquires the
   write lock to insert `CompactedMessage` nodes with `CompactedFrom` edges.

4. **Retry:** The agent loop retries context construction. The pipeline now finds
   compacted versions available and renders them at the appropriate `DetailLevel`.

**Two strategies for the retry:**

- **Fail-and-retry:** The pipeline returns a "compaction pending" result. The agent
  loop waits for the compaction event (via `EventBus` subscription) and re-runs
  the pipeline. Simple, but adds latency to the first occurrence.

- **Dependency-based:** The pipeline inserts a placeholder node that `DependsOn`
  the compaction task. The scheduler (from Design 04's `ready_unclaimed_nodes()`)
  holds the agent until all dependencies resolve. More aligned with the existing
  graph-as-work-queue architecture.

**Proactive (background):** The `ContextSummarize` stub gets replaced with a real
compaction worker that runs proactively on idle (no user input for N seconds),
processing nodes older than a configurable age threshold. This ensures that by the
time context pressure occurs, most nodes already have compacted versions — the
reactive path is the exception, not the rule.

### 6.5 Tool Call/Result Compaction

ToolCall + ToolResult are a semantic unit linked by `Invoked` and `Produced` edges.
A single `CompactedMessage` summarizes both together:

```
Original: ToolCall(read_file, path="src/auth.rs") + ToolResult(500 lines of Rust)
Compacted: "Read src/auth.rs (500 lines): JwtConfig struct, validate_token()
            with RS256 verification, token refresh flow"
```

The compacted node has two `CompactedFrom` edges — one to the `ToolCall`, one to the
`ToolResult`. This is the **highest-value compaction target**: file read results
dominate token usage in coding conversations.

**Error preservation:** When compacting ToolResults with `is_error: true`, the
compaction must preserve error codes, affected resources, and actionable remediation
info. A compacted "database connection failed" that drops the port number and timeout
duration loses debugging context. Compaction prompts for error results should
explicitly instruct: "preserve all error codes, file paths, and numeric values."

### 6.6 ThinkBlock Compaction

ThinkBlock nodes (`src/graph/node.rs:221-226`) are child nodes of Messages linked via
`ThinkingOf` edges. Three options:

1. **Fold into parent:** When compacting a Message, include a summary of its
   ThinkBlock reasoning in the compacted content. Pro: simpler graph. Con: inflates
   the compacted message.
2. **Compact separately:** Create a CompactedMessage for the ThinkBlock independently.
   Pro: fine-grained control. Con: ThinkBlocks are not typically rendered in context.
3. **Drop on compaction:** ThinkBlocks are internal reasoning artifacts; drop them
   when the parent Message is compacted. Pro: maximum compression. Con: loses
   reasoning provenance.

**Recommendation:** Option 3 for Phase 1 (ThinkBlocks are not rendered in the current
context pipeline). Phase 2 can revisit with Option 1 if agents benefit from
understanding prior reasoning chains.

### 6.7 Edge Propagation

When a Message node has outgoing edges (`Invoked` → ToolCall, `About` → Question,
`RelevantTo` → WorkItem), the CompactedMessage does NOT inherit these edges. The
`CompactedFrom` edge provides the link back to the original, and the original retains
all its edges. The Render stage, when using a compacted version, can still traverse
from the original if needed.

This avoids edge duplication and keeps the compacted node lightweight.

### 6.8 Compaction Quality and Rollback

**Quality metrics:**
- `compression_ratio` (stored on node): target >4x for Summary, >10x for Aggressive
- Size guard: reject compactions longer than 50% of the original (Beads pattern)
- Spot-check: background task can compare agent performance with/without compaction

**Rollback:** If a compacted node is discovered to be inaccurate, remove it from
the graph and delete its `CompactedFrom` edges. The Render stage falls back to the
full original automatically. No `Supersedes` chain needed — compacted nodes are
cache entries, not authoritative data.

### 6.9 Concurrency

Multiple agents may attempt to compact the same node concurrently. Mitigation:
before issuing a compaction request, check if a `CompactedFrom` edge already exists
for the target node at the desired level. This check happens under the write lock
when inserting the new CompactedMessage node. If a compaction was created between
the check and the insert, the duplicate is simply not added (idempotent).

### 6.10 Graph Size Growth

Compacted nodes add to graph size. For a 1000-message conversation with one
Summary-level compaction each, that's ~1000 additional nodes + ~1000 edges. Phase 1
has no eviction. Phase 2 introduces TTL-based cleanup for compactions older than a
configurable threshold. Phase 3's tiered storage moves content out of the graph
entirely.

---

## 7. VISION.md Alignment

| VISION.md Concept | Section | Proposed Design | Status |
|---|---|---|---|
| `CompactedMessage` node type | 3.1 | New Node variant with `CompactionLevel` + `perspective` | Exact match |
| `compacted_from` edge type | 3.1 | New `EdgeKind::CompactedFrom` | Exact match |
| Multi-perspective compaction | 4.2 | `perspective: Option<String>`, Phase 3 | Deferred, structurally supported |
| MergeTree background processing | 4.3 | Background worker replacing `ContextSummarize` stub | Exact match |
| Tiered storage | 4.3 | `CompactionLevel` maps to Hot/Warm/Cold | Structural match |
| Select compaction level | 3.2 | `DetailLevel` in `ContextPolicy` Render stage | Integration with existing trait |
| Deterministic construction | 3.2 | Same graph + anchor + policy = same selection | Preserved |
| Never mutate originals | 8.1 | `CompactedFrom` edges; originals untouched | Exact match |

---

## 8. Recommended Architecture

### Phase 1: Event-Driven Compaction

**New types (proposed, not yet implemented):** `CompactionLevel` enum (`Summary`,
`Aggressive`, `MetadataOnly`), `CompactedMessage` node variant,
`EdgeKind::CompactedFrom`.

**Behavior:** Budget stage detects token pressure → emits compaction request as a
graph event → async worker compacts target nodes → agent retries with compacted
versions available. Compaction is mandatory — the pipeline never silently drops
nodes. If no compacted version exists and budget is exceeded, the pipeline signals
"compaction needed" and the agent loop waits for the compaction worker to complete.

**Compaction prompt:** Structured — requests Summary/Key-Decisions/Resolution
format. Size guard rejects compactions longer than 50% of originals.

**Graph migration:** Additive — new Node variant and EdgeKind. No data transformation
needed. Follows the same V-bump pattern as existing migrations.

**TUI:** Compacted nodes render with a dim style and `[compacted]` label.

### Phase 2: Background MergeTree Compaction

**Replace stub:** `spawn_context_summarization` at `src/tasks.rs:263` becomes a real
compaction worker.

**Trigger:** Idle detection (no user input for configurable N seconds) + age-based
eligibility (nodes older than threshold without existing compaction).

**Priority queue:** ToolResult nodes first (highest token savings per compaction),
then long Messages (>500 tokens), then everything else.

**Batch API:** Use batch/background API pricing for cost optimization. Local model
(Qwen 14B) is zero-cost; cloud batch APIs offer significant discounts.

**Metadata:** `compression_ratio` and `original_token_count` on each `CompactedMessage`
enable quality monitoring and threshold tuning.

**`expand_context` tool:** Registered as a tool available to agents. Follows
`CompactedFrom` edges to retrieve full content on demand.

### Phase 3: Multi-Perspective + Tiered Storage

**Multi-perspective:** `perspective` field becomes `Some("security")`,
`Some("performance")`, etc. Multiple `CompactedMessage` nodes per source, each with
a different perspective. Topic detection drives perspective selection.

**Conversation arc compaction:** Group related messages by topic (using edge clusters
or LLM-based segmentation) and compact entire arcs into single nodes with N:1
`CompactedFrom` edges. This is the highest-compression strategy — 20 messages into
1 arc summary.

**Tiered storage:**
- **Hot:** Full nodes in graph (active conversation)
- **Warm:** Compacted in graph (recent but not active)
- **Cold:** Metadata-only in graph, content in blob store
- **Archive:** Content evicted, metadata + embeddings only

---

## 9. Integration Design

### Proposed Types

```rust
/// How aggressively a node has been compacted.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CompactionLevel {
    Summary,
    Aggressive,
    MetadataOnly,
}

/// Compressed representation of one or more source nodes.
/// Linked to originals via `CompactedFrom` edges.
Node::CompactedMessage {
    id: Uuid,
    content: String,
    compaction_level: CompactionLevel,
    perspective: Option<String>,
    compression_ratio: f32,
    original_token_count: u32,
    created_at: DateTime<Utc>,
}

/// New edge kind for compaction provenance.
EdgeKind::CompactedFrom
```

### Pipeline Data Flow

```
Anchor -> Expand -> Score -> Budget -> Render -> Sanitize
                              |
                     tokens fit budget?
                     /                \
                   Yes                 No
                    |                   |
              Render(Full)     Downgrade DetailLevel for lowest-scored:
                                 CompactedFrom edge exists?
                                 /                    \
                               Yes                     No
                                |                       |
                         Render(Summary)         Emit compaction event
                                                  -> BackgroundTask node
                                                  -> Async worker compacts
                                                  -> Agent retries pipeline
                                                  -> Finds compacted versions
```

### Re-expansion Flow

```
Agent receives compacted context (marked with [compacted] indicator)
  -> Needs more detail on node X
  -> Issues expand_context(node_id=X)
  -> Executor: graph.sources_by_edge(X, CompactedFrom)
  -> Retrieves original node(s) via edge targets
  -> Returns full content as tool result
  -> Agent continues with expanded context
```

---

## 10. Red/Green Team

### Green Team Findings (Factual Verification)

| Claim | Status | Correction |
|-------|--------|------------|
| Claude Code auto-compacts at ~95% capacity | Verified | Confirmed via morphllm.com analysis |
| MemGPT two-tier memory with LLM-driven paging | Verified | Matches arxiv.org/abs/2310.08560 |
| LongLLMLingua: 21.4% boost, 4x fewer tokens | Verified | Matches ACL 2024 paper |
| RCC: 32x compression, BLEU4 ~0.95 | Verified | Matches OpenReview paper |
| ACON: 26-54% memory reduction | Verified | Matches arxiv.org/abs/2510.00615 |

**Corrections applied:**
- Separated LLMLingua (up to 20x compression) from LongLLMLingua (94% cost reduction
  on LooGLE, 21.4% boost on NaturalQuestions) — these are different methods
- Removed unverifiable "12.5% lossy zone" claim — cited paper (arxiv 2509.11208) does
  not contain this specific threshold
- Removed unverifiable JetBrains research citation — URL does not exist; replaced with
  general principle that proactive compaction outperforms reactive-only
- Updated DeepSeek pricing reference to note that specific batch pricing varies; use
  local models as zero-cost baseline

### Red Team Findings (Challenges)

**Critical — JIT compaction lock contention (resolved):**
The original design proposed just-in-time async LLM calls during the Budget stage.
This is incompatible with the `SharedGraph` (`Arc<RwLock<ConversationGraph>>`) read
lock held during context pipeline execution. Resolution: event-driven model — the
pipeline emits a compaction request as a graph event, an async worker performs the
compaction outside any lock, and the agent retries the pipeline after compaction
completes. Compaction is mandatory; the pipeline never silently drops nodes.

**High — Compaction hallucination risk:**
An LLM-generated compaction may introduce inaccuracies that become authoritative in
the graph. Mitigation: compacted nodes are never authoritative — they are rendering
cache that the Render stage can bypass. Size guard + structured prompt + temperature=0
reduce hallucination probability. Rollback: delete the CompactedMessage node.

**High — Agent ignorance of compaction:**
Agents may not realize context is compacted and may reason on incomplete information.
Resolution: rendered compacted content includes a `[compacted]` marker and the
`expand_context` tool is documented in the system prompt.

**Medium — Missing: conversation arc compaction:**
Compacting individual messages misses the 5-10x compression available from grouping
related messages into topic arcs. Added to Phase 3 recommendation.

**Medium — Missing: ThinkBlock handling:**
ThinkBlocks have parent-child relationship with Messages and need explicit policy.
Added section 6.6: drop on compaction in Phase 1, revisit in Phase 2.

**Medium — Missing: edge propagation semantics:**
Original node edges are NOT copied to CompactedMessage. Added section 6.7.

**Medium — Graph size growth:**
Compacted nodes increase graph size monotonically in Phase 1. Added section 6.10
with Phase 2 TTL-based cleanup and Phase 3 tiered storage.

**Low — Match arm cost understated:**
Original claim of "bounded and mechanical" was correct but unquantified. Audit found
~15 locations. Updated Option A weakness with actual count.

### Code Accuracy Findings

**Critical — File references updated:**
The monolithic `src/app/context.rs` was refactored into `src/app/context/` module
directory. All file:line references updated to reflect the new structure:
- `build_messages()` → `src/app/context/policies/conversational.rs:82-129`
- `finalize_context()` → `src/app/context/sanitize.rs:12-32`
- `truncate_messages()` → `src/app/context/sanitize.rs:35-57`
- `sanitize_message_boundaries()` → `src/app/context/sanitize.rs:63-93`
- `spawn_context_summarization` → `src/tasks.rs:263-285` (was 263-284)

**All other code references verified accurate:** `Node` enum (11 variants),
`EdgeKind` enum, `Edge` struct (`{from, to, kind}` only), `ContextPolicy` trait
(6 methods), `DetailLevel` enum, `sources_by_edge` method signature,
`ContextSummarize` stub behavior, `Supersedes` edge at `node.rs:154`.

---

## 11. Sources

### Compaction & Compression
- [Claude API: Compaction Docs](https://platform.claude.com/docs/en/build-with-claude/compaction)
- [Compaction vs Summarization (Morph)](https://www.morphllm.com/compaction-vs-summarization)
- [LLMLingua: Prompt Compression](https://github.com/microsoft/LLMLingua) — up to 20x compression
- [LongLLMLingua](https://aclanthology.org/2024.acl-long.91/) — ACL 2024; 94% cost reduction, 21.4% boost
- [Recurrent Context Compression](https://openreview.net/forum?id=GYk0thSY1M) — 32x compression, BLEU4 ~0.95
- [Selective_Context](https://github.com/liyucheng09/Selective_Context) — self-information-based pruning

### Memory & Agent Architecture
- [MemGPT: LLMs as Operating Systems](https://arxiv.org/abs/2310.08560) — tiered memory with LLM-driven paging
- [Letta Documentation](https://docs.letta.com/concepts/memgpt/) — in-context + archival memory blocks
- [ACON: Context Compression for Agents](https://arxiv.org/abs/2510.00615) — 26-54% memory reduction

### Background Processing Analogies
- [ClickHouse MergeTree](https://clickhouse.com/docs/engines/table-engines/mergetree-family/mergetree) — write-optimized + async compaction
- [LSM Compaction Design Space](https://vldb.org/pvldb/vol14/p2216-sarkar.pdf) — leveling vs tiering trade-offs

### Industry Practice
- [Claude Code Auto-Compact Analysis](https://www.morphllm.com/claude-code-auto-compact) — token savings 60-80%

### Internal References
- `docs/VISION.md` sections 3.1, 4.2, 4.3 — CompactedMessage, multi-perspective, MergeTree
- `docs/research/22-graph-context-building-strategies.md` — 5-stage context pipeline
- `docs/design/04-graph-scheduler-qa-relationships.md` — ContextPolicy trait, DetailLevel
- `src/app/context/policy.rs:62-90` — ContextPolicy trait definition
- `src/app/context/policies/conversational.rs:18-78` — ConversationalPolicy implementation
- `src/tasks.rs:263-285` — ContextSummarize stub
