# Runtime Storage Options

> Research conducted 2026-03-12. Analysis of storage backends, serialization formats,
> and persistence architectures for Context Manager's property graph, with specific
> recommendations for local-first and future remote storage.

---

## 1. Executive Summary

Context Manager persists its property graph as JSON files — the entire `ConversationGraph` serialized via `serde_json`, written atomically to `~/.context-manager/conversations/{id}/graph.json`. This works for early development but does not scale: every load deserializes the full graph, there are no partial queries, no full-text search, no transactions, and `nodes_by()` is an O(n) linear scan.

VISION.md §5.2 plans a three-layer stack: petgraph (hot) + CozoDB (Datalog queries) + sled (snapshots) + remote gRPC. This architecture is sound but over-engineers Phase 1. CozoDB adds a Datalog learning curve for queries that SQLite recursive CTEs handle adequately at current scale.

**Recommendation:** Replace JSON files with **SQLite** (via `rusqlite`) behind a new `ConversationStore` trait. SQLite provides zero-config ACID storage, recursive CTEs for graph traversal, JSON1 for flexible node storage, FTS5 for content search, and WAL mode for concurrent readers. The trait abstraction enables swapping in Turso (SQLite-compatible remote sync) or SurrealDB (native graph syntax) for Phase 2 without changing application code. Keep petgraph as the in-memory hot working set — unchanged.

---

## 2. Current Architecture

### 2.1 Persistence Flow

Save path (`persistence.rs:23-43`):
1. Serialize `ConversationGraph` → `migration::to_versioned_json()` wraps in `VersionedGraph::V2` envelope
2. Write to `graph.json.tmp`
3. Atomic rename to `graph.json`
4. Separately serialize `ConversationMetadata` to `metadata.json` (same tmp+rename pattern)

Load path (`persistence.rs:45-58`):
1. Read `graph.json` as string
2. `migration::load_and_migrate()` detects version, migrates if needed (V1→V2), backs up original
3. Deserialize into `ConversationGraph` via intermediate `GraphRaw` → `From<ConversationGraphRaw>`

### 2.2 Schema Migration

`migration.rs` implements versioned persistence:
- `VersionedGraph` tagged enum (`migration.rs:60-66`): `#[serde(tag = "version")]` with `V2` variant
- `detect_version()` (`migration.rs:71-76`): tries `VersionedGraph` deser, falls back to V1
- V1→V2 migration (`migration.rs:111-132`): converts `HashMap<Uuid, Uuid>` edges to typed `Vec<Edge>`
- Backup before migration: `graph.v1.json.bak`

### 2.3 In-Memory Graph

`ConversationGraph` (`graph/mod.rs:196-207`):
- `nodes: HashMap<Uuid, Node>` — 9 node types (Message, SystemDirective, WorkItem, GitFile, Tool, BackgroundTask, ThinkBlock, ToolCall, ToolResult)
- `edges: Vec<Edge>` — 9 edge kinds (RespondsTo, SubtaskOf, RelevantTo, Tracks, Indexes, Provides, ThinkingOf, Invoked, Produced)
- `branches: HashMap<String, Uuid>` — branch name → leaf node
- `responds_to: HashMap<Uuid, Uuid>` — runtime index (not serialized), rebuilt on load
- `invoked_by: HashMap<Uuid, Uuid>` — second runtime index for tool call chains
- `nodes_by()` (`graph/mod.rs:398-400`): linear scan, O(n)

### 2.4 Strengths and Weaknesses

| Strength | Weakness |
|----------|----------|
| Simple, zero dependencies | Full deserialize on every load |
| Human-readable JSON | No partial queries (must load entire graph) |
| Atomic writes (tmp+rename) | No full-text search |
| Versioned migration | O(n) node filtering |
| Easy to debug | No concurrent access |
| | No transactions (partial writes possible across files) |
| | No indexes beyond runtime `responds_to` and `invoked_by` |

---

## 3. Requirements

Derived from VISION.md and current architecture:

| # | Requirement | Priority | Status |
|---|-------------|----------|--------|
| 1 | Local-first single-user storage | Must | Current (JSON) |
| 2 | Remote/networked storage | Future | VISION.md §5.2 |
| 3 | Property graph: 9 node types, 9 edge kinds | Must | Current |
| 4 | Schema migration across versions | Must | Current (V1→V2) |
| 5 | Graph traversal (ancestor walking) | Must | Current (responds_to index) |
| 6 | Node filtering by type | Must | Current (O(n) scan) |
| 7 | Full-text search on message content | Should | Not implemented |
| 8 | Concurrent reads (background tasks, multi-agent) | Should | Not supported |
| 9 | Trait abstraction for swappable backends | Must | Not implemented |

---

## 4. Storage Options

### 4.1 Embedded Key-Value Stores

| Store | Crate | Version | Rust-Native | Graph Fit | Notes |
|-------|-------|---------|-------------|-----------|-------|
| sled | `sled` | 0.34.7 | Yes | Poor | Pure Rust Bw-tree (lock-free B-link tree variant). ACID transactions. No query language — all graph traversal is manual application code. Same serialization burden as JSON. |
| redb | `redb` | 2.4.0 | Yes | Moderate | Typed tables (e.g., `Table<&[u8], &[u8]>`). ACID. Better multi-table than sled but still requires manual graph walking. Stable (past 1.0). |
| RocksDB | `rust-rocksdb` | 0.22.0 | No (C++ FFI) | Poor | Battle-tested LSM engine. Overkill for single-user. Adds C++ build dependency. |
| fjall | `fjall` | 2.9.0 | Yes | Poor | Modern LSM alternative to sled. Actively developed but small ecosystem. |
| LMDB | `heed` | 0.20.5 | No (C FFI) | Poor | Ultra-fast mmap reads. Single-writer constraint. Low-level API. |

Also notable but not deeply evaluated: `native_db` (pure Rust, automatic secondary indexes via derive macros — better than raw KV for typed queries but still no graph traversal), `jammdb` (pure Rust LMDB clone — eliminates C FFI but same limitations as heed), `persy` (pure Rust transactional storage).

**Verdict:** Key-value stores solve durability but not queryability. They require the same manual serialization and graph traversal as JSON files. The only gain is crash safety (ACID transactions), which SQLite also provides with far more query capability.

### 4.2 Embedded SQL

#### SQLite — rusqlite v0.33+

The strongest candidate for Phase 1. Zero-config, single-file, ACID.

**Schema design:**
```sql
CREATE TABLE nodes (
    id          TEXT PRIMARY KEY,
    type        TEXT NOT NULL,
    data        TEXT NOT NULL,  -- JSON (Node enum via serde_json)
    created_at  TEXT NOT NULL
);
CREATE TABLE edges (
    from_id  TEXT NOT NULL REFERENCES nodes(id),
    to_id    TEXT NOT NULL REFERENCES nodes(id),
    kind     TEXT NOT NULL
);
CREATE INDEX idx_nodes_type ON nodes(type);
CREATE INDEX idx_edges_from ON edges(from_id);
CREATE INDEX idx_edges_to   ON edges(to_id);
CREATE INDEX idx_edges_kind ON edges(kind);
```

**Graph traversal** — recursive CTE for branch history:
```sql
WITH RECURSIVE history(id, data, depth) AS (
    SELECT id, data, 0 FROM nodes WHERE id = ?1
    UNION ALL
    SELECT n.id, n.data, h.depth + 1
    FROM history h
    JOIN edges e ON e.from_id = h.id AND e.kind = 'responds_to'
    JOIN nodes n ON n.id = e.to_id
)
SELECT * FROM history ORDER BY depth DESC;
```

**Key capabilities:**

| Feature | Support | Notes |
|---------|---------|-------|
| ACID transactions | Yes | Commit/rollback across nodes+edges |
| JSON columns | Yes | JSON1 extension: `json_extract(data, '$.content')` |
| Full-text search | Yes | FTS5 virtual table on message content |
| Schema migration | Yes | `PRAGMA user_version` + migration scripts |
| Concurrent readers | Yes | WAL mode allows many readers + one writer |
| Node lookup by ID | <1ms | Primary key index |
| Node filter by type | <1ms | `idx_nodes_type` index |
| Recursive CTE depth=10 | ~10ms | Adequate for conversation chains |
| Recursive CTE depth=100 | ~100ms | May need optimization at scale |

**Limitations:**
- Single writer at a time (even in WAL mode) — adequate for single-user, constraining for multi-agent concurrent writes
- WAL file can grow unbounded if long-running read transactions prevent checkpointing (relevant for background tasks holding reads open)
- Recursive CTEs are verbose compared to Datalog — a 3-hop traversal is ~15 lines vs. 3 in Datalog
- No native graph indexes (adjacency is via JOIN, not pointer traversal)
- `json_extract()` queries are slower than typed columns — consider hybrid schema (typed columns for frequently-queried fields like `type`, `created_at`; JSON for the rest)
- C dependency: `rusqlite` with `bundled` feature compiles SQLite from C source. Requires a C compiler at build time, which may complicate cross-compilation to musl/Alpine targets. VISION.md §5.1 specifies "single binary distribution" — the bundled feature satisfies this at runtime but adds build complexity.
- **JSON column schema evolution**: When `Node` enum variants change (new fields, renamed fields), `ALTER TABLE` cannot help — the JSON blob must be migrated row-by-row. Options: (a) make all fields `Option<T>` for backward compatibility, (b) write JSON-level migration that updates each row, (c) accept heterogeneous JSON shapes with version tags per row. This is the same problem `migration.rs` solves today but spread across individual rows instead of a single file.

#### DuckDB — duckdb-rs v1.1+

OLAP-optimized analytical engine. Excellent for aggregation queries (token usage by date, message type distributions) but optimized for batch reads, not transactional OLTP. Overkill unless heavy analytics are needed.

#### libSQL / Turso

SQLite-compatible fork with built-in remote sync. Local mode is a drop-in SQLite replacement; remote mode syncs to Turso cloud or self-hosted libSQL server. Good Phase 2 migration path — same SQL, same schema, add sync.

**Crate:** `libsql` (evolving API, less mature than rusqlite)

### 4.3 Embedded Graph Databases

#### CozoDB — cozo v0.7.6

Rust query engine with Datalog. VISION.md's planned choice. Note: the query engine is pure Rust, but the default storage backend is SQLite (C dependency). Pure Rust is possible with sled or in-memory backends only.

**Strengths:**
- Purpose-built for property graphs — relations and rules are first-class
- Datalog naturally expresses multi-hop traversal without verbose CTEs
- Embedded mode (in-process) and standalone server mode (future remote)
- Transactions, rollback, built-in versioning

**Example query** — branch history:
```datalog
ancestors[node] <- edges{from: node, to: parent, kind: 'responds_to'}, ancestors[parent]
ancestors[node] <- edges{from: node, to: parent, kind: 'responds_to'}
?[id, data] := ancestors[?leaf_id], nodes{id, data}
```

**Weaknesses:**
- Datalog learning curve — unfamiliar to most Rust developers
- Smaller ecosystem: fewer tutorials, Stack Overflow answers, production deployments
- Documentation gaps compared to SQLite
- FTS available via `FtsSearch` relation type, but less mature than SQLite FTS5

**When to adopt:** When query complexity outgrows recursive CTEs — specifically when multi-hop traversals with variable depth, graph algorithms (community detection, PageRank, transitive closure), or complex pattern matching become frequent. The trigger is query complexity and edge density, not raw node count — a sparse 50K-node graph may work fine with CTEs while a dense 1K-node graph with avg degree >20 may not.

#### indradb v4.0

Property graph model with pluggable backends (in-memory, sled, RocksDB). Development has slowed significantly since 2022. Not recommended for new projects.

#### oxigraph v0.4

RDF/SPARQL engine. Wrong abstraction — RDF triples are a semantic web model, not a natural fit for typed application nodes and edges.

### 4.4 Document Stores

#### SurrealDB — surrealdb v2.x+ (3.x current)

Multi-model database: document, graph, relational. JSON-first with SurrealQL.

**Key differentiator:** Native graph syntax in queries:
```surql
SELECT * FROM $leaf<-responds_to<-*;
```

**Modes:**
- Embedded (RocksDB backend) — local-first
- Client-server — remote, multi-user, built-in auth/RBAC

**Strengths:**
- Graph-aware queries without CTEs
- Same API local and remote (best Phase 2 story)
- Flexible schema, natural fit for tagged enum nodes
- Active development, growing ecosystem

**Weaknesses:**
- Younger than SQLite — less production history
- RocksDB dependency in embedded mode (C++ FFI)
- SurrealQL is still evolving (breaking changes possible)
- Heavier resource footprint than SQLite

**Best for:** Phase 2 if native graph query syntax is valued over SQLite's maturity.

#### PoloDB — polodb v4.0

MongoDB-like embedded document store, pure Rust. No graph query features. Moderate fit — stores documents well but requires manual graph traversal.

### 4.5 Serialization Formats

Independent of storage backend — these affect how node/edge data is encoded.

| Format | Crate | Size vs JSON | Ser/Deser Speed | Schema Evolution | Human-Readable |
|--------|-------|-------------|-----------------|-----------------|----------------|
| JSON | `serde_json` 1.0 | Baseline | Baseline | Good (additive) | Yes |
| MessagePack | `rmp-serde` 1.3.0 | -30-40% | ~2x faster | Good | No |
| CBOR | `ciborium` 0.2.2 | -30% | ~1.5x faster | Good (RFC 8949) | No |
| bincode | `bincode` 2.0 | -60%+ | ~5x faster | Poor (brittle, crate abandoned) | No |
| postcard | `postcard` 1.1.1 | -50%+ | ~4x faster | Moderate | No |
| FlatBuffers | `flatbuffers` 24.12 | -50%+ | Zero-copy reads | Excellent | No |

**Verdict:** Keep JSON for persistence (human-readable, debuggable, adequate performance). Switch to MessagePack only if file size becomes a problem. Avoid bincode for durable storage — schema changes break compatibility. FlatBuffers/Cap'n Proto are overkill at this scale.

If using SQLite: node data is stored as JSON text in a `TEXT` column, queryable via `json_extract()`. The serialization format question becomes less important — SQLite handles the durability.

**FTS alternatives:** SQLite FTS5 is adequate for keyword search but lacks BM25 ranking customization and faceted search. **Tantivy** (pure Rust, Lucene-inspired) provides richer full-text search with BM25, facets, and no C dependency. For future embedding-based retrieval (VISION.md's relevance scoring via cascade evaluation), **sqlite-vec** provides vector similarity search as a SQLite extension — enabling hybrid keyword+semantic search within a single SQLite database.

---

## 5. Remote-Capable Options

For VISION.md's requirement of eventual remote storage:

| Option | Local Mode | Remote Mode | Migration from SQLite | Maturity |
|--------|-----------|-------------|----------------------|----------|
| **Turso/libSQL** | SQLite file | Cloud or self-hosted sync | Minimal — same SQL | Growing |
| **SurrealDB** | RocksDB embedded | Client-server | New query layer (SurrealQL) | Stable |
| **CozoDB** | Embedded | Standalone server | New query layer (Datalog) | Niche |
| **PostgreSQL + AGE** | N/A (server only) | Excellent | New backend, Cypher queries | Mature |
| **Neo4j** | N/A (server only) | Excellent | New backend, Cypher queries | Mature |

**Recommended path:** Start SQLite locally → Turso for remote sync (minimal migration) OR SurrealDB for richer graph queries (larger migration but better long-term).

---

## 6. Comparison Matrix

| Criterion | JSON (current) | SQLite | SurrealDB | CozoDB | sled/redb |
|-----------|---------------|--------|-----------|--------|-----------|
| **Local-first** | Yes | Yes | Yes | Yes | Yes |
| **Graph traversal** | App code only | Recursive CTEs | Native `<-` syntax | Datalog rules | App code only |
| **Schema migration** | Tagged enum | `PRAGMA user_version` | Schema rules | Datalog rules | Manual |
| **Remote support** | No | Via Turso | Built-in | Server mode | No |
| **ACID transactions** | No | Yes | Yes | Yes | Yes |
| **Full-text search** | No | FTS5 | Built-in | FtsSearch (basic) | No |
| **Node filter by type** | O(n) scan | Indexed, <1ms | Indexed | Indexed | Manual |
| **Concurrent readers** | No | WAL mode | Yes | Yes | Yes |
| **Maturity** | N/A | Decades | 3 years | 3 years | 5+ years |
| **Learning curve** | None | SQL (low) | SurrealQL (medium) | Datalog (high) | Rust API (low) |
| **Rust integration** | serde_json | rusqlite (excellent) | surrealdb crate | cozo crate | Native |
| **Human debuggable** | Yes (text) | Yes (sqlite3 CLI) | Yes (surreal CLI) | Moderate | No |
| **Dependencies** | None | C (SQLite3) | C++ (RocksDB) | C (SQLite default) or None (sled backend) | None |

---

## 7. VISION.md Evaluation

VISION.md §5.2 specifies:

```
┌──────────────────────────────┐
│     Graph Engine (petgraph)  │  ← in-process, hot working set
├──────────────────────────────┤
│     Storage Trait            │  ← abstract interface
├──────┬───────────┬───────────┤
│ Cozo │  sled     │  Remote   │
│      │           │  (gRPC)   │
└──────┴───────────┴───────────┘
```

**Assessment:**

| Component | Verdict | Reasoning |
|-----------|---------|-----------|
| petgraph (hot) | **Keep** | In-memory graph for active traversal is correct |
| Storage Trait | **Keep** | Abstraction enables backend swapping — essential |
| CozoDB | **Defer** | Powerful but premature. SQLite CTEs handle current query patterns. Adopt when query complexity (multi-hop with variable depth, graph algorithms, pattern matching) outgrows CTEs — driven by edge density and query type, not raw node count. |
| sled (snapshots) | **Replace with SQLite** | SQLite provides snapshots via transactions plus queries, FTS, and migration tooling. sled adds a second backend with no query capability. |
| Remote (gRPC) | **Revise to Turso or SurrealDB** | Turso provides SQLite-compatible remote sync. SurrealDB provides graph-native remote. Either is simpler than custom gRPC. |

**Revised architecture:**
```
┌──────────────────────────────────────┐
│  Graph Engine (petgraph, in-memory)  │  ← hot working set (unchanged)
├──────────────────────────────────────┤
│  ConversationStore trait             │  ← abstract interface
├──────────┬───────────────────────────┤
│  SQLite  │  Remote (Turso/SurrealDB) │  ← Phase 1 / Phase 2
└──────────┴───────────────────────────┘
```

---

## 8. Migration Path

### Phase 1: Trait Abstraction
1. Define `ConversationStore` trait. The trait should cover node/edge CRUD and graph traversal — not just whole-graph save/load — to enable backends that support partial queries:
   - Graph-level: `save_graph`, `load_graph`, `list_conversations`
   - Node CRUD: `add_node`, `get_node`, `update_node`, `remove_node`, `query_nodes_by_type`
   - Edge CRUD: `add_edge`, `remove_edge`, `get_edges_from`, `get_edges_to`
   - Traversal: `get_ancestors` (via responds_to chain), `get_descendants`
   - Search: `search_content` (FTS — backends without FTS return empty results)
   - Transaction: `begin`, `commit`, `rollback` (no-ops for backends without transaction support)
2. Implement `JsonStore` wrapping current `persistence.rs` functions — backward compatible. Node/edge CRUD methods operate on in-memory graph and flush on `save_graph`.
3. Wire `App` to use `dyn ConversationStore` instead of direct function calls

### Phase 2: SQLite Backend
1. Implement `SqliteStore` with `nodes` + `edges` tables, JSON1 for node data
2. Add FTS5 virtual table for message content search
3. Use `PRAGMA user_version` for schema versioning
4. One-time migration tool: load JSON conversations → insert into SQLite
5. Default new conversations to SQLite; keep `JsonStore` as fallback

### Phase 3: Remote Backend (future)
1. Implement `TursoStore` or `SurrealStore` behind same trait
2. Configuration selects backend: `storage = "sqlite"` / `"turso"` / `"surreal"`

---

## 9. Event Sourcing Consideration

An alternative to snapshot-based persistence is event sourcing: store an append-only log of graph mutations (`AddNode`, `AddEdge`, `UpdateNode`, etc.) and replay to reconstruct state.

**Advantages:**
- Complete audit trail — every mutation is recorded
- Natural versioning — any historical state is replayable
- Aligns with VISION.md's MergeTree analogy (append-first, optimize later)

**Disadvantages:**
- Replay cost grows with history — need periodic snapshots (checkpoints) anyway
- Branching creates divergent event logs — complex to merge
- Adds significant complexity for marginal benefit at current scale
- Not proven for property graph persistence in production

**Alignment with VISION.md:** Event sourcing aligns well with two VISION.md design principles: (1) the MergeTree analogy (§4.3) — append-only writes with background optimization is the core event sourcing pattern, and (2) the immutable node design — "compaction creates new nodes, never mutates" (§4.3 line 229) is event sourcing semantics. The `GraphEvent` broadcast from doc 07's inter-agent communication design is already an in-memory event stream.

**Recommended hybrid:** Use SQLite as the snapshot store (materialized current state) with an optional `events` table for audit/replay. This combines the query power of SQLite with the audit trail of event sourcing. The `events` table is append-only, cheap to maintain, and can be truncated after checkpointing. This is not pure event sourcing (the snapshot is primary, not derived) but captures the practical benefits without the replay complexity.

```sql
CREATE TABLE events (
    seq       INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp TEXT NOT NULL,
    kind      TEXT NOT NULL,  -- AddNode, AddEdge, UpdateNode, etc.
    data      TEXT NOT NULL   -- JSON payload
);
```

---

## 10. Red Team / Green Team

### Green Team — Arguments for SQLite

1. **Maturity**: SQLite is the most deployed database engine in the world. Zero risk of abandonment.
2. **Zero-config**: Single file, no server, no setup. Same UX as JSON files but with ACID.
3. **Recursive CTEs**: Handle ancestor walking, descendant queries, and transitive closure — covering all current graph traversal patterns.
4. **FTS5**: Full-text search on message content enables "find messages mentioning X" without loading the entire graph.
5. **WAL mode**: Multiple concurrent readers (background tasks, TUI) with one writer. Sufficient for single-user + background agents.
6. **JSON1 extension**: Store `Node` enum as JSON text, query properties with `json_extract()`. No need to fully normalize the schema.
7. **Trait abstraction**: `ConversationStore` trait makes the SQLite choice reversible. If CozoDB or SurrealDB proves better later, swap without touching application code.
8. **Gas Town precedent**: Gas Town uses SQL (Dolt, which is MySQL-compatible) for agent state persistence, validating SQL as adequate for graph-like agent data.

### Red Team — Arguments against SQLite

1. **Graph queries are verbose**: A 3-hop traversal in Datalog is 3 lines; in SQL with recursive CTEs it's 15+ lines. As query complexity grows, this becomes painful.
2. **No native graph indexes**: SQLite uses B-tree indexes on foreign keys. Graph databases use adjacency lists or edge-pointer structures for O(1) neighbor traversal. At >100K edges, the JOIN-based approach may become noticeably slower.
3. **Single writer**: WAL mode allows concurrent readers but still serializes writes. With 10+ agents writing concurrently (per doc 07's multi-agent vision), this becomes a bottleneck. The GraphCoordinator pattern (single writer) mitigates this but constrains throughput.
4. **VISION.md divergence**: The planned architecture specifies CozoDB. Choosing SQLite instead delays the Datalog investment and may require a second migration later when complex graph queries become necessary.
5. **JSON1 is slower than typed columns**: `json_extract()` on every query is slower than native typed columns. At scale, this argues for either full normalization (more schema migration burden) or a purpose-built document/graph store.
6. **CTE performance is unquantified**: The document estimates CTE performance but provides no benchmarks. CTE degradation depends on edge density and fan-out, not just depth — a graph with high average degree will stress CTEs much earlier than a sparse linear chain.
7. **"Covers most of Cozo's value" is unquantified**: This claim is based on current query patterns. Future requirements (community detection, relevance scoring via graph algorithms, multi-perspective traversal) may demand capabilities that CTEs cannot efficiently express.

---

## 11. Sources

### Embedded Databases
- [rusqlite documentation](https://docs.rs/rusqlite/latest/rusqlite/)
- [SQLite JSON1 Extension](https://www.sqlite.org/json1.html)
- [SQLite FTS5](https://www.sqlite.org/fts5.html)
- [SQLite WAL Mode](https://www.sqlite.org/wal.html)
- [SQLite Recursive CTEs](https://www.sqlite.org/lang_with.html)
- [CozoDB Documentation](https://docs.cozodb.org/)
- [CozoDB GitHub](https://github.com/cozodb/cozo)
- [SurrealDB Documentation](https://surrealdb.com/docs)
- [SurrealDB Rust SDK](https://docs.rs/surrealdb/latest/surrealdb/)
- [DuckDB Rust Bindings](https://docs.rs/duckdb/latest/duckdb/)
- [libSQL / Turso](https://github.com/tursodatabase/libsql)

### Key-Value Stores
- [sled documentation](https://docs.rs/sled/latest/sled/)
- [redb documentation](https://docs.rs/redb/latest/redb/)
- [fjall documentation](https://docs.rs/fjall/latest/fjall/)
- [heed (LMDB bindings)](https://docs.rs/heed/latest/heed/)
- [rust-rocksdb](https://docs.rs/rocksdb/latest/rocksdb/)

### Serialization
- [rmp-serde (MessagePack)](https://docs.rs/rmp-serde/latest/rmp_serde/)
- [ciborium (CBOR)](https://docs.rs/ciborium/latest/ciborium/)
- [bincode](https://docs.rs/bincode/latest/bincode/)
- [postcard](https://docs.rs/postcard/latest/postcard/)
- [FlatBuffers Rust](https://docs.rs/flatbuffers/latest/flatbuffers/)

### Search Extensions
- [Tantivy — Rust full-text search](https://github.com/quickwit-oss/tantivy)
- [sqlite-vec — Vector search for SQLite](https://github.com/asg017/sqlite-vec)
- [native_db — Rust-native with secondary indexes](https://github.com/vincent-herleworx/native_db)

### Architecture References
- Context Manager VISION.md §5.2 — Storage Stack
- Gas Town / Dolt persistence patterns — `docs/research/05-gastown-multi-agent-orchestration.md`
- GraphCoordinator architecture — `docs/research/07-inter-agent-communication.md`
- [SQLite as Application File Format](https://www.sqlite.org/appfileformat.html)
- [LangGraph Persistence](https://langchain-ai.github.io/langgraph/)
