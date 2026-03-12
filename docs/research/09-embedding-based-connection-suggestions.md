# Embedding-Based Connection Suggestions

> Research conducted 2026-03-12. Analysis of embedding models, vector storage,
> and integration patterns for discovering semantic connections between graph
> nodes in Context Manager.

---

## 1. Executive Summary

Context Manager's property graph has 9 node types connected by 9 explicit edge kinds — but no mechanism to discover implicit semantic relationships. A Message about authentication has no automatic link to a WorkItem titled "implement OAuth" or a ThinkBlock reasoning about session management. These connections exist only if a human or LLM explicitly creates a `RelevantTo` edge.

Embeddings solve this by converting node text content into dense vectors where semantic similarity corresponds to geometric proximity. Given a new node, we compute its embedding, compare against all stored embeddings via cosine similarity, and surface high-scoring pairs as suggested `RelevantTo` edges.

Research doc 01 (§4.4) already describes a **cascade evaluation** pattern: embedding similarity as a fast first pass ($0.001/node), escalating uncertain scores to an LLM judge ($0.10/node). This document designs the implementation.

**Recommendation:**
- **Phase 1:** `fastembed` crate (ONNX Runtime, `all-MiniLM-L6-v2`, 384 dimensions) for local embedding. In-memory `HashMap<Uuid, Vec<f32>>` with brute-force cosine similarity. Persisted alongside the graph. Background worker via existing `TaskMessage` + `mpsc` infrastructure.
- **Phase 2:** Migrate vector storage to `sqlite-vec` when SQLite backend lands (per doc 08). Add Ollama provider for `nomic-embed-text` (768d, 8K context).
- **Phase 3:** ANN index (`usearch` or `hnsw_rs`) if graph exceeds 50K nodes. Remote API providers (Voyage AI, OpenAI) behind trait abstraction.

---

## 2. Current Architecture & Gap Analysis

### 2.1 Embeddable Content

`Node::content()` (`graph/mod.rs:162-174`) normalizes text access across all 9 node types:

| Node Type | Content Source | Embedding Value | Notes |
|-----------|---------------|-----------------|-------|
| Message | `content` field | **High** | Core conversation; topic similarity, cross-session recall |
| ThinkBlock | `content` field | **High** | Reasoning patterns; find recurring analysis |
| WorkItem | `title` (+ `description`) | **High** | Task similarity; find related work across sessions |
| ToolResult | `content` field | **Medium** | Similar outputs reveal related operations |
| Tool | `name` (+ `description`) | **Medium** | Group semantically similar tools |
| SystemDirective | `content` field | **Low** | Mostly static; little discovery value |
| ToolCall | `arguments.tool_name()` | **Low** | Name only; arguments contain more signal |
| GitFile | `path` field | **Low** | File paths; structural, not semantic |
| BackgroundTask | `description` field | **Low** | Internal status; minimal semantic content |

Note: `Node::content()` returns only the primary text field. For WorkItem, this is `title` — the optional `description` field (which often contains richer context) would require a separate accessor or a dedicated embedding text builder.

### 2.2 Existing Edge Infrastructure

`EdgeKind::RelevantTo` (`graph/mod.rs:60`) exists but is never automatically created. It's the natural target for embedding-suggested connections. No new edge kind is needed — `RelevantTo` was designed for this purpose.

### 2.3 Current Query Limitations

`nodes_by()` (`graph/mod.rs:398-400`) performs O(n) linear scan with a closure predicate. There is no semantic filtering, full-text search, or similarity-based retrieval.

### 2.4 Background Task Infrastructure

`tasks.rs:37-60` defines `TaskMessage` enum with 6 variants sent via `mpsc::UnboundedSender`. Background workers (`spawn_git_watcher`, `spawn_tool_discovery`, `spawn_context_summarization`) follow a consistent pattern: spawn task → send status update → do work → send results. Embedding computation fits this pattern exactly — new `TaskMessage` variants for embedding requests and results.

### 2.5 Cascade Evaluation (Doc 01)

Research doc 01, lines 366-370, describes the cascade:
1. Embed node, compute cosine similarity to query (cheap)
2. If score > 0.9 or < 0.2: use that score directly
3. If 0.2-0.9: escalate to LLM judge (slow but accurate)
4. Result: 70% of nodes evaluated cheaply, 30% escalated

VISION.md §4.4 reinforces this: "Embedding similarity as fast first pass" in the multi-rater relevance system.

---

## 3. Embedding Model Options

### 3.1 Local Inference

| Crate | Backend | Models | Dims | Deps | Notes |
|-------|---------|--------|------|------|-------|
| `fastembed` | ONNX Runtime (ort) | all-MiniLM-L6-v2, bge-small-en-v1.5 | 384 | ort (C++) | Recommended. Sync API, no tokio needed. Built for embedding. |
| `candle-transformers` | Candle (HF) | BERT, custom | 768+ | Pure Rust possible | Flexible but more manual. WASM-capable. |
| `rust-bert` | tch-rs (PyTorch) | Sentence transformers | varies | libtorch (C++) | Heavy dep. Good model variety. |
| `ort` (direct) | ONNX Runtime | Any ONNX model | varies | ort (C++) | Lower-level than fastembed. More control. |
| `llama-cpp-rs` | llama.cpp | GGUF embedding models | varies | llama.cpp (C++) | Quantized models. Less common for embeddings. |

**`fastembed` detail:** Wraps `ort` with a purpose-built API for text embeddings. Handles tokenization, batching, and normalization internally. Pre-configured model list with automatic download. Sync API runs well inside `spawn_blocking`.

**`candle` detail:** Hugging Face's Rust ML framework. More flexible than fastembed (can load arbitrary models) but requires more setup — manual tokenizer initialization, model weight loading, mean pooling. 3-4x slower on GPU than PyTorch but excellent for CPU-only and edge/WASM scenarios.

### 3.2 Remote APIs

| Provider | Model | Dims | Context | Cost | Notes |
|----------|-------|------|---------|------|-------|
| Ollama (local) | nomic-embed-text | 768 | 8,192 tokens | Free | Local REST API. Requires ollama running. |
| OpenAI | text-embedding-3-small | 1536 (or 512) | 8,191 tokens | $0.02/1M tokens | Flexible dimensions via API param. |
| OpenAI | text-embedding-3-large | 3072 | 8,191 tokens | $0.13/1M tokens | Highest quality. |
| Voyage AI | voyage-code-3 | varies | varies | Similar to OpenAI | Best for code+NL mix. Anthropic's recommended provider. |
| Cohere | embed-english-v3.0 | 1024 | 512 tokens | $0.10/1M tokens | Good quality, shorter context. |

Anthropic does not offer its own embeddings API.

### 3.3 Recommendation

**Phase 1: `fastembed` + `all-MiniLM-L6-v2`**
- 384 dimensions — smallest common embedding, fast cosine, minimal memory
- Sync API works with `spawn_blocking` in the existing background task pattern
- No external service dependency — local-first, privacy-preserving
- Quality: strong on MTEB for short English text (messages, titles, descriptions)
- **Caveat:** Trained on sentence pairs (NLI/STS), not code. Performance on mixed code+natural language content (tool results, code snippets in messages) is weaker than code-aware models. If code-heavy conversations dominate, consider `nomic-embed-text` ONNX weights through fastembed/ort directly (no Ollama needed) as an alternative Phase 1 model.
- **Cross-compilation note:** `ort` bundles C++ shared libraries, complicating musl/Alpine builds. If VISION.md's single-binary requirement is a hard constraint, evaluate `candle-transformers` (pure Rust) as the Phase 1 backend instead, accepting slower inference.

**Phase 2: Add Ollama provider**
- `nomic-embed-text` with 8K context handles long messages and full tool results
- Matryoshka dimensions (64-768) allow trading quality for speed
- Still local — no data leaves the machine

**Phase 3: Remote providers**
- `voyage-code-3` for mixed code+natural language content
- Behind `EmbeddingProvider` trait — no application code changes

---

## 4. Vector Storage & Similarity Search

### 4.1 Options

| Store | Type | Rust Integration | Complexity | Best For |
|-------|------|------------------|------------|----------|
| `HashMap<Uuid, Vec<f32>>` | In-memory brute-force | Native | Trivial | <10K nodes, Phase 1 |
| `sqlite-vec` | SQLite extension | rusqlite + loadable ext | Low | Aligns with doc 08 SQLite plan |
| `usearch` | ANN (HNSW) | Rust crate | Medium | >50K nodes, sub-ms queries |
| `hnsw_rs` | ANN (HNSW) | Pure Rust | Medium | >50K nodes, no C deps |
| `hora` | ANN (multi-algo) | Pure Rust | Medium | **Abandoned** (last release 2021). Listed for completeness. |
| Qdrant | Vector DB | gRPC client | High | Multi-user, complex filtering |

### 4.2 Memory Budget

Formula: `nodes × dimensions × 4 bytes` (f32)

| Nodes | 384d | 768d | 1536d |
|-------|------|------|-------|
| 1,000 | 1.5 MB | 3 MB | 6 MB |
| 10,000 | 15 MB | 30 MB | 60 MB |
| 50,000 | 75 MB | 150 MB | 300 MB |
| 100,000 | 150 MB | 300 MB | 600 MB |

At 384 dimensions, even 100K nodes fit comfortably in memory. ANN becomes worthwhile not for memory but for query latency — brute-force cosine over 100K vectors at 384d takes ~10ms (SIMD-optimized), which may be acceptable depending on query frequency.

### 4.3 Brute-Force vs ANN Crossover

Brute-force cosine similarity on normalized vectors reduces to dot product:
- 1K vectors × 384d: ~0.05ms (always brute-force)
- 10K vectors × 384d: ~0.5ms (brute-force fine)
- 50K vectors × 384d: ~2-5ms (borderline — depends on query frequency)
- 100K vectors × 384d: ~5-15ms (consider ANN if querying frequently)

For Context Manager's use case (embedding on node creation, not interactive search), brute-force is sufficient well past 10K nodes.

### 4.4 Recommendation

**Phase 1:** `HashMap<Uuid, Vec<f32>>` serialized via `rmp-serde` (MessagePack) — binary format is 2-3x smaller than JSON for float arrays and ~10x faster to parse. Cosine similarity via manual dot product (10 lines of code).

**Phase 2:** `sqlite-vec` — when SQLite backend lands, store embeddings as a virtual table. Natural integration, persistent, queryable with SQL.

**Phase 3:** `usearch` or `hnsw_rs` — only if graph exceeds 50K nodes AND embedding queries are latency-sensitive (interactive search).

---

## 5. Suggesting Connections

### 5.1 Basic Pipeline

1. Node `N` is created or updated
2. Background worker computes `embed(N.content())` → `Vec<f32>`
3. Compare against all stored embeddings via cosine similarity
4. Filter: score > threshold AND no existing edge between nodes
5. Create `EdgeKind::RelevantTo` edges for high-confidence pairs

### 5.2 Threshold Calibration

Cosine similarity thresholds are model-dependent and content-dependent. all-MiniLM-L6-v2 produces different score distributions for code snippets vs natural language. **Do not hardcode thresholds.**

**Phase 1 calibration approach:**
1. Embed all nodes, compute pairwise similarities, build a histogram
2. Set auto-create threshold at the 99th percentile (only the most similar pairs)
3. Set suggestion threshold at the 95th percentile (surface in TUI for user approval)
4. Log all scores and user accept/reject decisions to refine thresholds over time

**Cascade evaluation** (from doc 01, lines 366-370) applies at **retrieval time** — when selecting context for a query. It does NOT apply at indexing time. At indexing time, use only embedding similarity with top-k (not absolute thresholds) to suggest edges. LLM judge escalation is too expensive to run on every node creation (at 10K nodes, 30% escalation = 3K LLM calls per new node).

### 5.3 Graph-Aware Scoring (Deferred)

Pure embedding similarity misses structural context. A message from 3 months ago with 0.75 similarity may be less relevant than a message from the same session with 0.60 similarity. Microsoft's GraphRAG combines semantic, structural, and temporal signals.

**Phase 1: Pure cosine similarity only.** Graph-aware scoring adds significant complexity (shortest-path computation per query, 3 hyperparameters to tune) with unclear benefit until we have real usage data. Add structural and temporal signals only if false positive rate is high after Phase 1 deployment.

### 5.4 Edge Limits and Decay

Without limits, embedding suggestions generate O(N) `RelevantTo` edges per node, overwhelming the graph. Constraints:

- **Max edges per node:** 3 suggested `RelevantTo` edges per node (top-3 by score)
- **Minimum score:** Only suggest if cosine similarity exceeds the calibrated threshold
- **Global budget:** `RelevantTo` edges capped at 2x the count of structural edges (RespondsTo, Invoked, etc.)
- **Suggested vs confirmed:** New `RelevantTo` edges start as `suggested`. Users can confirm or dismiss in the TUI. Only confirmed edges persist across re-indexing.
- **Confidence decay:** Reduce stored similarity scores by 5% per week. Prune edges that fall below the minimum threshold. This ensures old, weak suggestions don't clutter the graph.

### 5.5 Cross-Session Connections

The highest value of embeddings is connecting nodes across different conversations — something the current explicit-edge model cannot do at all. A WorkItem "implement rate limiting" in conversation A should surface when conversation B discusses API throttling.

This requires an embedding index that spans all conversations, not just the active one. The `EmbeddingIndex` must be conversation-agnostic, keyed by `(conversation_id, node_id)`.

**Data isolation:** When a conversation is deleted, all its embeddings must be removed. The `EmbeddingIndex` needs:
- `remove_by_conversation(conversation_id)` method
- Reconciliation on load: prune embeddings whose node IDs no longer exist in any conversation graph
- Atomic updates: write index only after conversation save succeeds, or accept eventual consistency with periodic reconciliation

---

## 6. Integration Architecture

### 6.1 New Types

```rust
/// New variant for BackgroundTaskKind
BackgroundTaskKind::EmbeddingCompute

/// New TaskMessage variants
TaskMessage::EmbeddingRequested {
    node_id: Uuid,
    content: String,
}
TaskMessage::EmbeddingComputed {
    node_id: Uuid,
    vector: Vec<f32>,
    model_id: String,
    similar_nodes: Vec<(Uuid, f32)>, // (node_id, score)
}
```

### 6.2 EmbeddingProvider Trait

```rust
pub trait EmbeddingProvider: Send + Sync {
    /// Embed a single text. Implementations may batch internally.
    fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>>;

    /// Embed multiple texts in a batch (more efficient for bulk operations).
    fn embed_batch(&self, texts: &[&str]) -> anyhow::Result<Vec<Vec<f32>>>;

    /// Dimensionality of the output vectors.
    fn dimensions(&self) -> usize;

    /// Unique identifier for the model (used for cache invalidation).
    fn model_id(&self) -> &str;
}
```

Implementations: `FastEmbedProvider`, `OllamaProvider`, `OpenAiProvider`.

### 6.3 EmbeddingIndex

```rust
pub struct EmbeddingIndex {
    vectors: HashMap<Uuid, Vec<f32>>,
    conversation_map: HashMap<Uuid, String>, // node_id -> conversation_id
    model_id: String,
}

impl EmbeddingIndex {
    pub fn insert(&mut self, conversation_id: &str, node_id: Uuid, vector: Vec<f32>);
    pub fn remove(&mut self, node_id: Uuid);
    pub fn remove_by_conversation(&mut self, conversation_id: &str);
    pub fn top_k(&self, query: &[f32], k: usize, exclude: &HashSet<Uuid>) -> Vec<(Uuid, f32)>;
    pub fn reconcile(&mut self, valid_node_ids: &HashSet<Uuid>); // prune orphans
    pub fn model_id(&self) -> &str;
    pub fn needs_reindex(&self, current_model: &str) -> bool;
}
```

### 6.4 Background Worker Flow

```
App::handle_node_created(node)
  │
  ├─ tx.send(TaskMessage::EmbeddingRequested { node_id, content })
  │
  ▼
spawn_embedding_worker(rx, embedding_provider, embedding_index)
  │
  ├─ Receive EmbeddingRequested
  ├─ provider.embed(content) → vector
  ├─ index.insert(node_id, vector)
  ├─ index.top_k(vector, 5, already_connected) → similar_nodes
  │
  ├─ tx.send(TaskMessage::EmbeddingComputed { node_id, vector, model_id, similar_nodes })
  │
  ▼
App::handle_embedding_computed(msg)
  │
  ├─ Apply edge limits (max 3 per node, global budget)
  ├─ Create RelevantTo edges with suggested=true
  └─ Surface in TUI for user confirmation
```

---

## 7. Embedding Lifecycle

### 7.1 When to Embed

- **On node creation:** Background, non-blocking. Most nodes are created once.
- **On content change:** Re-embed. Rare — Messages are immutable, WorkItem `title` changes are uncommon.
- **Cold start:** Batch-embed all existing nodes on first run or model change.

### 7.2 Model Versioning

Store `model_id` alongside the embedding index. When the configured model changes:
1. Detect mismatch: `index.model_id() != provider.model_id()`
2. Mark index as stale
3. Background batch re-embeds all nodes
4. Old index remains usable during re-indexing (stale but functional)

### 7.3 Persistence

**Phase 1 (MessagePack):** Serialize `EmbeddingIndex` via `rmp-serde`:

```rust
#[derive(Serialize, Deserialize)]
struct EmbeddingIndexData {
    model_id: String,
    dimensions: usize,
    entries: Vec<EmbeddingEntry>,
}

#[derive(Serialize, Deserialize)]
struct EmbeddingEntry {
    conversation_id: String,
    node_id: Uuid,
    vector: Vec<f32>,
}
```

Written to `~/.context-manager/embeddings/index.msgpack`. Binary format: 10K × 384 floats ≈ 15MB (vs ~40MB as JSON). Separate from conversation data since the index spans conversations. Includes `conversation_id` for cleanup on conversation deletion.

**Phase 2 (SQLite):** `embeddings(node_id TEXT, conversation_id TEXT, vector BLOB, model_id TEXT)` with sqlite-vec virtual table for similarity queries.

### 7.4 Cold Start Strategy

First run with embeddings enabled:
1. Spawn background task: iterate all conversations, all nodes
2. Batch-embed (32 items per batch via `embed_batch`)
3. Progress reported via `TaskMessage::TaskStatusChanged`
4. Suggestions only available after indexing completes
5. Estimated time: 1K nodes at ~50ms/batch of 32 ≈ 1.5 seconds

---

## 8. Prior Art

### 8.1 GraphRAG (Microsoft)

Extracts entities and relationships from text, builds a knowledge graph, generates community summaries, then combines graph traversal with vector similarity for retrieval. Key insight: graph structure adds context that pure vector similarity misses. Their v1.0 (Dec 2024) separated embeddings into dedicated vector stores, reducing storage 43%.

### 8.2 LangGraph Long-Term Memory

Stores memories as JSON documents in vector stores with namespaces. Semantic retrieval via embeddings enables cross-conversation recall. Uses Voyage AI models. MongoDB and PostgreSQL integrations with DiskANN acceleration.

### 8.3 Cursor Codebase Indexing

Custom embedding models generate per-chunk vectors stored in Turbopuffer (serverless vector DB). Privacy model: only embeddings stored in cloud, source code stays local. Combines vector search with full-text search.

### 8.4 CID-GraphRAG

Integrates intent transition graphs with semantic similarity for conversational retrieval. Models both dynamic intent transitions (graph edges) AND semantic associations (embeddings). Addresses the limitation that RAG captures semantics but misses conversation flow.

### 8.5 Doc 01 Cascade Evaluation

This project's own research (doc 01, lines 366-370): embed → cosine → LLM judge escalation. 70% cheap, 30% expensive. Risk: embedding similarity can miss contextual relevance that only an LLM catches.

---

## 9. Comparison Matrix

| Criterion | FastEmbed | Candle | Ollama | OpenAI | Voyage AI |
|-----------|-----------|--------|--------|--------|-----------|
| **Local-only** | Yes | Yes | Yes (local server) | No | No |
| **Inference speed** | Fast (ONNX) | Medium | Slow (HTTP + model load) | Fast (API) | Fast (API) |
| **Quality (MTEB)** | Good (384d) | Good-Excellent | Excellent (768d) | Excellent | Best for code |
| **Dimensions** | 384 | 768+ | 768 (flexible) | 512-3072 | varies |
| **Dependencies** | ort (C++ via bundled) | Pure Rust possible | reqwest only | reqwest only | reqwest only |
| **Cost** | Free | Free | Free | $0.02/1M tok | ~$0.02/1M tok |
| **Code+NL quality** | Good | Good | Good | Good | Excellent |
| **Setup complexity** | Low (auto-download) | Medium (manual) | Medium (install ollama) | Low (API key) | Low (API key) |
| **Privacy** | Full (local) | Full (local) | Full (local) | Data sent to API | Data sent to API |

---

## 10. VISION.md Alignment

**§4.4 Multi-Rater Relevance System:** Describes cascade evaluation with "embedding similarity as fast first pass." Embeddings are the cheap first rater in the multi-rater architecture. This document implements that vision.

**§4.1 Graph-Based Context Management:** References prior art with "vector search within the subgraph." Embeddings enable this — restrict similarity search to nodes within a subgraph for focused retrieval.

**§4.3 Async Compaction Pipeline:** Step 2 is "Analysis: Background LLM extracts structure, detects clusters." Embeddings are a natural clustering mechanism — group similar nodes before LLM summarization.

Embeddings complement, not replace, LLM judges. They're the fast/cheap tier in a cascade where LLMs are the slow/expensive tier.

---

## 11. Red/Green Team

### Green Team (Validated)

- **fastembed crate:** Confirmed on crates.io (v5.12.0), ONNX Runtime via ort, sync API, maintained by Qdrant team. All claims accurate.
- **all-MiniLM-L6-v2:** Confirmed 384 dimensions, ~90MB ONNX model, good (not top) MTEB ranking for its size class.
- **Memory math:** All calculations verified (10K × 384 × 4 = 15MB, etc.).
- **Brute-force timing:** Estimates consistent with SIMD-optimized dot product benchmarks.
- **sqlite-vec:** Confirmed pure C, successor to sqlite-vss, rusqlite loadable extension support.
- **GraphRAG v1.0:** Confirmed Dec 2024 release, 43% storage reduction claim accurate.
- **Ollama, OpenAI, Anthropic/Voyage:** All API claims verified.
- **usearch, hnsw_rs:** Confirmed active and maintained.
- **hora:** **Abandoned** since August 2021 — flagged in options table.

### Red Team (Addressed)

**CRITICAL — Fixed in this document:**
- **Cross-conversation data isolation:** Global index without cleanup = data leaks on conversation deletion. **Fixed:** Added `remove_by_conversation()`, `reconcile()` to EmbeddingIndex, conversation_id tracking in persistence schema (§5.5, §6.3, §7.3).
- **Edge spam:** Unlimited RelevantTo edges overwhelm the graph. At 10K nodes with top-5 suggestions, that's 50K edges — 10x more than structural edges. **Fixed:** Added edge limits, global budget, suggested vs confirmed states, confidence decay (§5.4).

**HIGH — Fixed in this document:**
- **Arbitrary thresholds:** 0.85/0.5 had no empirical basis. **Fixed:** Replaced with calibration methodology — build histogram from real data, set thresholds at percentiles, log accept/reject for refinement (§5.2).
- **Wrong model for code:** all-MiniLM-L6-v2 trained on sentence pairs, not code. **Fixed:** Added caveat and alternative (nomic-embed-text ONNX weights directly through ort, no Ollama needed) in §3.3.
- **LLM judge cost model broken:** Cascade applied at indexing time = 3K LLM calls per new node at 10K nodes. **Fixed:** Clarified cascade is retrieval-time only, not indexing-time. Indexing uses top-k similarity only (§5.2).
- **JSON persistence wasteful:** 40MB JSON vs 15MB binary for 10K embeddings. **Fixed:** Changed Phase 1 to MessagePack (rmp-serde) (§4.4, §7.3).

**MEDIUM — Acknowledged:**
- **ONNX vs pure Rust:** `ort` bundles C++ libraries, complicating cross-compilation. Candle is pure Rust alternative. **Noted:** Added cross-compilation caveat in §3.3. If single-binary is a hard constraint, swap FastEmbed for Candle.
- **Graph-aware scoring premature:** Combined formula adds complexity without data to tune. **Fixed:** Deferred to post-Phase 1. Start with pure cosine (§5.3).

**LOW — Noted:**
- **Rig framework:** Good for remote providers in Phase 3, overkill for Phase 1 custom trait.
- **Content quality for ToolCall:** Single-word embeddings have low discriminative power. Consider building richer embedding text (tool name + arguments) in a future iteration.
- **Model download size:** 90MB first-download latency. Mitigate with lazy download and progress indicator.

---

## 12. Sources

### Embedding Libraries
- fastembed-rs: https://github.com/Anush008/fastembed-rs
- ort (ONNX Runtime): https://github.com/pykeio/ort
- candle: https://github.com/huggingface/candle
- rust-bert: https://github.com/guillaume-be/rust-bert

### Models
- all-MiniLM-L6-v2: https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2
- nomic-embed-text: https://huggingface.co/nomic-ai/nomic-embed-text-v1.5
- OpenAI embeddings: https://platform.openai.com/docs/guides/embeddings
- Voyage AI: https://www.voyageai.com/

### Vector Storage
- sqlite-vec: https://github.com/asg017/sqlite-vec
- usearch: https://github.com/unum-cloud/usearch
- hnsw_rs: https://crates.io/crates/hnsw_rs
- hora: https://github.com/hora-search/hora

### Prior Art
- Microsoft GraphRAG: https://microsoft.github.io/graphrag/
- LangGraph memory: https://blog.langchain.com/launching-long-term-memory-support-in-langgraph/
- CID-GraphRAG: https://arxiv.org/html/2506.19385v1
- Cursor indexing: https://towardsdatascience.com/how-cursor-actually-indexes-your-codebase/

### Project References
- Doc 01 cascade evaluation: `docs/research/01-graph-context-and-prior-art.md:366-370`
- Doc 08 SQLite recommendation: `docs/research/08-runtime-storage-options.md`
- VISION.md §4.4 multi-rater: `docs/VISION.md:233-271`
- Node::content(): `src/graph/mod.rs:162-174`
- EdgeKind::RelevantTo: `src/graph/mod.rs:60`
- TaskMessage: `src/tasks.rs:37-60`
- Background workers: `src/tasks.rs:62-232`
