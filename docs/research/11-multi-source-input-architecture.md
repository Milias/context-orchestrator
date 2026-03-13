# Multi-Source Input Architecture

> **Date:** 2026-03-13 | Research into how a shared knowledge graph can accept concurrent input from multiple equally-valid sources — agents, users, and external processes — without imposing conversation semantics.

---

## 1. Executive Summary

Most LLM-based systems model interaction as a conversation: one user speaks, one assistant responds, turn by turn. This is the wrong abstraction for a system where agents, users, background processes, and external services all contribute information to a shared knowledge graph simultaneously.

The fundamental shift is from **conversation** (sequential, turn-based, single-source) to **knowledge accumulation** (concurrent, multi-source, append-heavy). This document surveys seven architectural patterns for multi-source graph input, examines five real-world systems that implement variants of these patterns, catalogs known failure modes, and identifies the key design decisions any such system must make.

---

## 2. The Problem Space

### Why Conversation is the Wrong Abstraction

A conversation assumes:
- **Turn-taking**: One participant speaks at a time
- **Sequential ordering**: Message N depends on messages 1..N-1
- **Two parties**: A user and an assistant (or a small fixed group)
- **Ephemeral context**: The conversation is the context; when it ends, the context resets

None of these hold when the system has:
- A background git watcher continuously indexing file changes
- An LLM agent analyzing conversation history and suggesting connections
- A user typing messages in the TUI
- An external tool submitting structured results
- A classifier labeling nodes with metadata
- A sync process pulling context from another system

These sources operate independently, at different cadences, with different priorities. Forcing them into a conversation model creates artificial serialization, hides provenance (who said what?), and makes the system's behavior dependent on the order sources happen to write — an order that has no semantic meaning.

### What Changes With Multi-Source

When all sources are equal contributors to shared state:

1. **Identity matters**: Every mutation must carry its source identity. "A node was added" is insufficient; "the git watcher added a GitFile node" is necessary for debugging, auditing, and undo.

2. **Ordering becomes partial**: Two sources writing simultaneously have no inherent order. The system must either impose a total order (coordinator), accept partial order (causal consistency), or declare order irrelevant (commutative operations).

3. **The graph is alive**: It's not a snapshot taken at the end of a conversation. It's a continuously-mutated shared workspace where the current state is the accumulation of all sources' contributions.

4. **Reads and writes interleave**: A source may read the graph to decide what to write. Between its read and write, another source may have mutated the graph. This is the classic read-write hazard, and the system must have a position on it.

---

## 3. Requirements

A multi-source knowledge graph input system needs:

| Requirement | Description |
|------------|-------------|
| **Source provenance** | Every mutation records which source created it |
| **Concurrent structural writes** | Multiple sources can add nodes and edges simultaneously |
| **Node-internal single writer** | Modifying a node's content assumes one writer at a time (no intra-node conflicts) |
| **Extensible source types** | Adding a new source type (e.g., a webhook listener) should not require modifying existing sources |
| **Consistency model** | Clear guarantees about what a reader sees after a write |
| **Failure isolation** | One source crashing does not corrupt the graph or block other sources |
| **Provenance-based operations** | Filter, replay, or undo mutations by source |
| **No conversation semantics** | No turn-taking, no assumption of sequential message flow |

---

## 4. Architectural Patterns

### 4.1 Blackboard Architecture

**How it works**: A shared data structure (the "blackboard") is accessible to all "knowledge sources" (KS). Each KS independently reads the blackboard, decides whether it can contribute, and writes its contribution. In the classical model, a "control unit" schedules which KS runs next based on blackboard state. In modern reactive variants, KS self-trigger when blackboard entries matching their interest patterns appear — no central scheduler needed.

**Origin**: HEARSAY-II speech recognition system (1971-1976, published 1980). Multiple analyzers at different linguistic levels wrote hypotheses to a shared blackboard. No analyzer knew about the others — they only knew the blackboard.

**Real-world use**: Military command-and-control systems, real-time monitoring, expert systems. Recent revival in LLM multi-agent systems (arXiv:2507.01701).

**Strengths**:
- Knowledge sources are fully decoupled — they only interact through the shared state
- New sources can be added without modifying existing ones
- Emergent behavior: the combined output exceeds what any single source could produce
- Natural fit for heterogeneous sources (human, ML, rule-based, sensor)
- Reactive variants eliminate the central scheduler, allowing fully concurrent KS execution

**Weaknesses**:
- Classical model's control unit is a design challenge: how does it decide which KS runs next?
- No built-in ordering guarantee — the blackboard reflects whatever was written last
- Debugging is hard when behavior emerges from uncoordinated writes
- Reactive variants trade scheduling complexity for pattern-matching complexity

**When it fails**: When sources have strong dependencies on each other's output order. If KS-A must write before KS-B reads, but there's no explicit coordination, race conditions emerge.

### 4.2 Event Sourcing / Event Log

**How it works**: Instead of mutating state directly, every change is recorded as an immutable event in an append-only log. The current state is derived by replaying all events from the beginning (or from a snapshot). The log is the source of truth, not the derived state.

**Origin**: Accounting (double-entry bookkeeping is event sourcing). Formalized in software by Greg Young (~2010). Widely adopted via Apache Kafka, Datomic, EventStoreDB.

**Real-world use**: Banking (every transaction is an event), Kafka-based microservices, Datomic (immutable database), blockchain (extreme event sourcing).

**Strengths**:
- Complete provenance: every mutation, by whom, when, in what order
- Time-travel debugging: reconstruct state at any point in history
- Undo/replay: remove events from source X, recompute state
- Natural multi-source support: each event carries its source identity
- Append-only writes have no conflicts — two sources can append simultaneously

**Weaknesses**:
- Log growth: unbounded unless compacted or snapshotted
- Replay cost: reconstructing state from scratch is O(n) in event count
- Schema evolution: changing event format requires upcasting old events
- Eventual consistency if log is distributed (single-machine avoids this)

**When it fails**: When the event log grows faster than it can be compacted. When events are fine-grained (character-level edits in a document) and replay becomes expensive. When strong consistency is required across readers (two readers may see different states if one hasn't replayed recent events).

### 4.3 Actor Model

**How it works**: Every entity in the system is an "actor" — an isolated unit with private state and a mailbox. Actors communicate through asynchronous messages, and in the pure model share no mutable state (though production systems like Akka permit shared references to immutable data). An actor processes one message at a time from its mailbox, which gives it sequential consistency internally. Some frameworks (Akka Streams) extend this with batch-processing variants for throughput.

**Origin**: Carl Hewitt (1973). Made famous by Erlang/OTP (Ericsson telecom switches), later Akka (JVM), Microsoft Orleans (.NET).

**Real-world use**: Telecom switches (Erlang), game servers (Orleans), distributed systems (Akka Cluster). In the Rust ecosystem: Ractor, Actix, Kameo.

**Strengths**:
- No shared mutable state eliminates data races by construction
- Supervision trees: parent actors restart failed children automatically
- Location transparency: actors don't care if peers are local or remote
- Natural backpressure: mailbox fills up → sender blocks or drops

**Weaknesses**:
- Message ordering is only guaranteed between a specific sender-receiver pair, not globally
- Actor discovery: how does a new actor find existing actors?
- Requires rethinking data ownership: the graph can't be a shared struct, it must be an actor that receives mutation messages
- Debugging message flows across many actors is notoriously difficult

**When it fails**: When you need global ordering across all actors (actors only order per-pair). When the "graph actor" becomes a bottleneck because all mutations funnel through it (this is the single-writer coordinator problem). When the message-passing overhead exceeds the benefit of isolation (very small, fast operations).

### 4.4 Publish-Subscribe (Topic-Based)

**How it works**: Sources publish messages to named topics. Consumers subscribe to topics of interest. A broker (or middleware layer) routes messages from publishers to subscribers. Publishers and subscribers don't know about each other.

**Origin**: Information bus architectures (1980s). Formalized in CORBA Event Service, JMS. Modern: Apache Kafka, MQTT, ROS 2 DDS, NATS.

**Real-world use**: IoT sensor networks (MQTT), robotics (ROS 2), microservices (Kafka), financial data feeds.

**Strengths**:
- Complete decoupling: publishers don't know who subscribes
- Scalable: adding subscribers doesn't affect publishers
- Topic-level filtering: consumers only see what they care about
- QoS policies (ROS 2/DDS): RELIABLE, BEST_EFFORT, ORDERED_ACCESS per-topic

**Weaknesses**:
- No request-response: pub-sub is fire-and-forget
- Ordering across topics is undefined — if source A publishes to topic X and source B to topic Y, no ordering guarantee
- Requires a broker or middleware (adds infrastructure complexity)
- Topic explosion: as the system grows, managing topics becomes its own problem

**When it fails**: When you need request-response semantics (query the graph, get a result). When cross-topic ordering matters. When the broker becomes a single point of failure.

### 4.5 Shared Mutable State (Concurrent Data Structures)

**How it works**: The graph is a shared data structure protected by locks or built with lock-free algorithms. All sources acquire a lock (or use atomic operations), mutate the data, and release. Readers can use read-locks for concurrent reads, write-locks for exclusive writes.

**Real-world use**: Virtually all databases internally. Concurrent hash maps (Java ConcurrentHashMap, Rust DashMap). Lock-free queues and stacks.

**Strengths**:
- Simple mental model: "the graph is right there, just write to it"
- Immediate consistency when properly synchronized: after a write completes and the lock is released, all readers acquiring the lock see the new state (requires correct use of memory barriers / acquire-release semantics)
- Mature tooling: lock analysis, deadlock detection, memory ordering
- Granular locking possible: per-node locks instead of whole-graph locks

**Weaknesses**:
- Lock contention: many writers competing for the same lock degrades throughput
- Deadlock risk with multiple fine-grained locks (lock ordering discipline required)
- Granularity trade-off: coarse locks are simple but contend; fine locks are complex but fast
- No built-in provenance: you know the current state but not who changed it or when
- Priority inversion: a low-priority source holding a lock blocks a high-priority source

**When it fails**: When writer count grows and contention becomes the bottleneck. When lock granularity decisions lead to deadlocks. When you need provenance (locks don't record who acquired them).

### 4.6 CRDTs (Conflict-Free Replicated Data Types)

**How it works**: Data structures mathematically designed so that concurrent updates always converge to the same state, regardless of the order updates are applied. No coordination needed. Each replica applies updates locally and periodically syncs with others.

**Origin**: Shapiro et al. (2011), INRIA. Adopted by Basho (Riak), Redis (distributed counters/sets), and collaborative editing tools (Yjs, Automerge). Note: Figma uses CRDT-inspired techniques but relies on a centralized server for conflict resolution, not true distributed CRDTs.

**Real-world use**: Redis (distributed counters/sets), Riak (distributed key-value), Yjs/Automerge (collaborative text editing). Figma uses a centralized hybrid inspired by CRDT concepts.

**Strengths**:
- No coordinator needed — updates commute by construction
- Works offline: each source applies updates locally, merges later
- Mathematically proven convergence
- No write conflicts possible

**Weaknesses**:
- Limited operation set: not all data structures have CRDT variants
- Metadata overhead: version vectors, tombstones for deletes, causal histories
- Complexity: understanding CRDT semantics requires mathematical background
- "Semilattice join" semantics may not match application intent (e.g., two sources adding different edges — CRDT says keep both, but maybe one is wrong)
- Garbage collection of tombstones is an unsolved problem at scale

**When it fails**: When the application needs to enforce constraints (e.g., "a node can have at most 5 edges of type X"). CRDTs cannot enforce global invariants without coordination. When tombstone accumulation degrades performance. When the semantic merge behavior (keep both sides) doesn't match the desired behavior (keep the correct one).

### 4.7 Tuple Spaces (Associative Memory)

**How it works**: A shared associative memory where sources write immutable tuples and read/take tuples by pattern matching. Three operations: `out` (write a tuple), `in` (destructively read a matching tuple — blocks if none match), and `rd` (non-destructively read a matching tuple). Sources don't address each other — they address the space by data shape.

**Origin**: Linda coordination language (Gelernter, 1985, Yale). Later: JavaSpaces (Sun/Jini), GigaSpaces, IBM TSpaces. Military: Cougaar multi-agent system.

**Real-world use**: Jini distributed computing, GigaSpaces in-memory data grids, parallel computing (tuple spaces as coordination primitive), multi-agent systems (Cougaar).

**Strengths**:
- Temporally decoupled: writer and reader don't need to be active simultaneously
- Spatially decoupled: neither knows the other's identity
- Pattern matching enables flexible routing without explicit topics or addresses
- Blocking `in` provides synchronization without explicit coordination primitives

**Weaknesses**:
- Destructive reads (`in`) create ordering dependencies and potential starvation
- No built-in provenance — tuples are anonymous
- Pattern matching can be expensive for large spaces
- Limited expressivity compared to a full query language

**When it fails**: When you need ordering guarantees (tuple spaces are unordered by design). When destructive reads create contention (multiple sources competing for the same tuple). When provenance matters — tuples carry no source identity unless explicitly added to the tuple structure.

---

## 5. Comparison Matrix

| Criterion | Blackboard | Event Sourcing | Actor Model | Pub-Sub | Shared State | CRDTs | Tuple Spaces |
|-----------|-----------|---------------|-------------|---------|-------------|-------|-------------|
| **Provenance** | Optional (add metadata) | Built-in (event log) | Per-message (mailbox) | Per-message | None (add manually) | Per-operation (version vectors) | None (tuples anonymous) |
| **Ordering** | None (scheduler-dependent) | Total (log order) | Per-pair only | Per-topic only | Immediate (lock order) | None (commutative) | None (unordered) |
| **Concurrency model** | Scheduler or reactive | Append-only (no conflicts) | Message-passing | Async fire-and-forget | Lock-based | Coordination-free | Pattern-matched |
| **Consistency** | Eventual (read-after-write races) | Sequential (single log) / Eventual (distributed) | Sequential per actor | Eventual | Strong (when properly synchronized) | Strong eventual | Linearizable per-tuple |
| **Complexity** | Medium (scheduler) | Low-Medium (log + replay) | Medium-High (actor lifecycle) | Medium (broker) | Low (just locks) | High (CRDT theory) | Low-Medium (pattern matching) |
| **Failure isolation** | Weak (bad write corrupts blackboard) | Strong (immutable log) | Strong (supervision trees) | Medium (broker failure) | Weak (lock corruption) | Strong (local-first) | Medium (blocking reads can deadlock) |
| **Best for** | Heterogeneous AI sources | Audit-heavy, temporal queries | Isolated agents with supervision | Decoupled event-driven systems | Simple concurrent access | Offline-first, multi-device | Loosely coupled parallel processes |

Note: "real-world scale" figures (Kafka millions/sec, Erlang millions of actors) reflect the pattern's ceiling in distributed deployments. For a single-machine knowledge graph with 5-30 sources, all patterns have more than sufficient throughput.

---

## 6. Prior Art Deep Dives

### 6.1 RDF Named Graphs & PROV-O

The Semantic Web community spent two decades solving multi-source provenance for knowledge graphs. Their solution: **named graphs**.

In RDF, every triple (subject, predicate, object) can belong to a named graph — a URI that identifies the context of that triple. Different sources write to different named graphs:

- `<graph://user-input>` contains triples the user asserted
- `<graph://tool-extraction>` contains triples a tool inferred
- `<graph://background-sync>` contains triples pulled from external systems

The **PROV-O** (Provenance Ontology, W3C Recommendation 2013) provides a standard vocabulary: every entity was `wasGeneratedBy` an activity, which `wasAssociatedWith` an agent. This gives you full provenance chains: "this node was created by the git watcher, which was triggered by a file change event, which was caused by the user saving a file."

**RGPROV** extends this to track provenance at individual triple granularity, not just per graph.

**Lesson**: Source provenance is not a new problem. The Semantic Web's mistake was making it too complex (RDF, OWL, SPARQL). The concept — tag every piece of data with its source — is exactly right.

### 6.2 Gas Town (Steve Yegge, 2024-2025)

Gas Town is a production multi-agent coding system running 20-30 agents. Its key architectural insight: **separate durable and ephemeral communication**.

- **Durable messages** (called "beads"): Tracked in Dolt (git-like database). Used for protocol messages (POLECAT_DONE, MERGE_READY). Expensive — each message is a commit.
- **Ephemeral messages** (called "nudges"): In-memory only. Used for heartbeats, wake signals, liveness checks. Zero persistence cost.

When Gas Town moved patrol/heartbeat traffic from durable to ephemeral, Dolt commit volume dropped ~80%. This validated that most inter-agent communication is transient and should not be persisted.

**ZFC Principle** (Gas Town internal terminology): "Go provides transport. Agents provide cognition." The infrastructure handles plumbing; agents handle decisions. Don't over-engineer coordination logic.

**Lesson**: Not all mutations are equal. Distinguishing between durable (graph-structural) and ephemeral (coordination signals) prevents the persistence layer from becoming a bottleneck.

### 6.3 ROS 2 / DDS (Robotics)

ROS 2 (Robot Operating System) handles one of the hardest multi-source problems: a robot where cameras, LIDAR, IMUs, motor controllers, path planners, and human operators all publish data concurrently to build a shared world model.

ROS 2 uses **DDS (Data Distribution Service)** as its middleware — a publish-subscribe system with QoS policies:

- **RELIABLE**: Guaranteed delivery (for safety-critical commands)
- **BEST_EFFORT**: Fire-and-forget (for high-frequency sensor data where the latest value matters)
- **ORDERED_ACCESS**: Maintain publication order within a topic
- **HISTORY depth**: Keep last N samples (for late subscribers)

Each publisher has a **domain** and **topic**. Subscribers can request different QoS per-subscription. There is no global coordinator — DDS uses peer-to-peer discovery.

**Key insight**: ROS 2 doesn't try to order everything globally. Each topic has its own ordering. Cross-topic ordering is explicitly not guaranteed. This is acceptable because a LIDAR reading and a camera frame don't need to be globally ordered — they just need to be timestamped.

**Lesson**: Per-source ordering (not global ordering) is sufficient when sources are independent. Timestamp every mutation and let consumers reason about ordering.

### 6.4 Multi-Agent Frameworks

Modern LLM orchestration frameworks each take a different approach to shared context:

**LangGraph** (LangChain): Shared state dict passed between nodes in a directed graph. Sequential — each node runs to completion before the next starts. No concurrent writes. Checkpointing for rollback. Simple but fundamentally single-source at each step.

**AutoGen** (Microsoft): Speaker selection — agents take turns. Token budgeting per agent. Termination conditions. Still conversation-shaped: one agent speaks at a time, others listen.

**CrewAI**: Task-based delegation. Agents have roles and are assigned tasks. Shared memory via a "crew memory" module. No concurrent writes — tasks execute sequentially or in parallel but write to separate outputs.

**MetaGPT**: Message pool with subscription routing. Agents publish typed messages; other agents subscribe by message type. Closest to pub-sub, but still fundamentally turn-based (agents act in "rounds").

**What's missing in all of them**: True concurrent writes to shared state. Every framework serializes access — either through turn-taking, sequential task execution, or a coordinator. None treat all sources as equal concurrent writers.

### 6.5 Knowledge Graph Databases

**Neo4j**: ACID transactions with read-committed isolation. Multiple clients can write concurrently; the database handles locking internally. Provenance is not built-in — you add it as node/relationship properties. Transactions can conflict (deadlock) and are retried.

**TypeDB**: Strong typing with a schema. Concurrent writes with optimistic concurrency control — transactions validate against a snapshot and fail if the snapshot is stale. Rule-based inference runs as a read-only layer over committed data.

**Dgraph**: Distributed graph database using Raft consensus for write ordering. Every mutation gets a logical timestamp. Concurrent writes from different clients are serialized by the Raft leader. Strong consistency but write throughput bounded by consensus latency.

**Lesson**: Production graph databases use transactions (optimistic or pessimistic) for concurrent writes. None are lock-free. The trade-off is always between consistency strength and write throughput.

---

## 7. Failure Modes & Anti-Patterns

### 7.1 The Conversation Trap

The most common failure: defaulting to conversation semantics because that's how LLM APIs work. The system forces turn-taking, serializes input, and treats the user's message as the only trigger for action.

**Symptom**: Background processes queue their output until the user's next message. The system appears unresponsive between user interactions.

**Fix**: Decouple source input from conversation turns. Any source can write at any time. The UI reflects the graph's current state, not a message history.

### 7.2 Provenance as Afterthought

Systems that add source tracking after the fact (MemGPT, Logseq) find it fragile. Retroactive provenance requires migrating all existing data, and there are always edge cases where the source is unknown.

**Symptom**: "Unknown source" entries proliferate. Provenance data is inconsistent across node types.

**Fix**: Design provenance in from the start. Every mutation carries its source identity as a required field, not an optional annotation.

### 7.3 Last-Write-Wins Without Audit Trail

Logseq's sync model: when two devices edit the same note before syncing, the later write overwrites the earlier one. No merge, no conflict marker, no record of what was lost.

**Symptom**: Data silently disappears. Users don't know what was lost because there's no record.

**Fix**: Event log or audit trail. Even if last-write-wins is the policy, record what was overwritten so it can be recovered.

### 7.4 The Coordinator Bottleneck

Actor-model and single-writer systems funnel all writes through one entity. This works at low write rates (graph operations are microsecond-scale), but becomes the ceiling when write rate scales with agent count.

**Symptom**: Latency increases linearly with agent count. The coordinator's message queue grows unbounded.

**Fix**: Partition writes by independence. If two sources write to unrelated parts of the graph, they shouldn't contend. Fine-grained locking or sharding.

### 7.5 Error Propagation Feedback Loops

When one source's output becomes another source's input, errors compound. A classifier mislabels a node → the mislabel becomes a feature for a link predictor → the predictor creates a wrong edge → the edge becomes structural input for the classifier.

**Symptom**: Prediction quality degrades over time. Errors are self-reinforcing.

**Fix**: Provenance-based isolation. ML-predicted metadata is stored separately from user-asserted or tool-derived data. Predictions never auto-create durable graph structure.

### 7.6 Lock Granularity Mismatch

Choosing the wrong lock granularity creates either contention (too coarse) or deadlocks (too fine):

- **Too coarse**: A single write lock on the entire graph means only one source writes at a time. With 30 sources, this is effectively a single-writer coordinator.
- **Too fine**: Per-node read-write locks enable concurrent writes to different nodes, but acquiring multiple node locks (e.g., adding an edge between two nodes) requires lock ordering discipline to prevent deadlocks.

**Symptom**: Either high contention (coarse) or intermittent deadlocks (fine).

**Fix**: Match granularity to mutation patterns. If most mutations are single-node adds (not multi-node operations), node-level locking works. If mutations frequently span multiple nodes, coarser locking or transactions are safer.

### 7.7 Schema Evolution Breaks Event Replay

Event sourcing promises "replay from any point in history." But when the graph schema changes (e.g., splitting `Message` into `UserMessage` and `AgentMessage`), old events become incompatible with the current data model.

**Symptom**: Replay produces nodes with outdated types. Derived state after replay doesn't match expectations. Upcasting logic grows in complexity with each schema change.

**Fix**: Version events explicitly. Each event carries a schema version. The replay engine applies upcasting transformations (version N → version N+1) during replay. Alternatively, snapshot frequently and only replay events since the last snapshot, limiting the schema versions that must be supported.

### 7.8 Event Log Bloat and Tombstone Accumulation

Over time, an event log accumulates create/delete pairs for nodes that no longer exist. Replaying the full log becomes expensive, and "time-travel debugging" across millions of events is impractical without indexing.

**Symptom**: Startup time grows linearly with history length. "Reconstruct state at month 6" requires replaying through months of irrelevant events.

**Fix**: Periodic snapshots with log truncation. Keep only events since the last snapshot for fast replay. Archive older events for forensic use, but don't require replaying them for normal operation. Accept that time-travel has a horizon (e.g., last 30 days of fine-grained events, snapshots only beyond that).

### 7.9 Uncontrolled Source Trust

If all sources are equally trusted, a broken or malicious source can corrupt the entire graph. A malfunctioning classifier that bulk-writes incorrect labels, or an external webhook that floods the graph with garbage nodes, can damage the system's usefulness.

**Symptom**: Graph quality degrades after a new source is added. No mechanism to selectively roll back one source's contributions without affecting others.

**Fix**: Source-scoped permissions (read-only, append-only, full read-write). Provenance enables selective rollback — undo all mutations from source X. Rate limiting per source prevents flood damage. ML-derived data should be stored as suggestions, not committed graph structure, until confirmed.

### 7.10 Write Bursts and Source Starvation

When one source produces a burst of writes (e.g., git watcher indexing an entire repository), other sources may experience increased latency if they contend for the same write path.

**Symptom**: User-perceived latency spikes when background tasks are active. Interactive sources starve behind batch sources.

**Fix**: Per-source write budgets or scheduling priority. Batch sources should yield periodically. Interactive sources (user input) get priority access. This is orthogonal to consistency — all mutations are eventually applied, but the order of application respects source priority.

---

## 8. Key Design Decisions

Any multi-source graph input system must make these five decisions. There is no universally correct answer — each depends on the system's constraints.

### 8.1 Ordering: Total vs Partial vs Unordered

| Model | Guarantee | Cost | Appropriate when |
|-------|-----------|------|-----------------|
| **Total order** | All sources see mutations in the same sequence | Coordinator or consensus protocol | Debugging requires replaying exact sequences |
| **Partial order** | Causally related mutations are ordered; independent ones are not | Timestamps + causal edges | Sources are mostly independent |
| **Unordered** | No ordering guarantee; commutative operations only | None (cheapest) | All mutations are independent (pure append) |

For a knowledge graph where most mutations are independent node/edge additions, **partial order** (timestamps + causal edges) is the natural fit. Total order is only needed for debugging replay, and can be achieved by logging to a sequential event log even if the live graph doesn't enforce it.

### 8.2 Consistency: Strong vs Eventual vs Causal

| Model | Guarantee | Cost | Appropriate when |
|-------|-----------|------|-----------------|
| **Strong** | After write returns, all readers see the new state | Locks or coordination | Correctness requires read-after-write visibility |
| **Eventual** | All readers will eventually see the new state | None (cheapest) | Sources don't read each other's writes |
| **Causal** | If source A's write causes source B's write, B's write is visible to anyone who saw A's | Vector clocks or causal metadata | Sources have dependencies but not full ordering |

For a single-machine system where sources share memory, **strong consistency with fine-grained locking** is practical. The overhead of locks on a single machine is microseconds, far below the millisecond-scale of LLM calls or disk I/O.

### 8.3 Provenance Granularity

| Level | What's tracked | Storage cost | Appropriate when |
|-------|---------------|-------------|-----------------|
| **Per-mutation** | Every individual add/remove/update records its source | High (one record per mutation) | Full auditability, undo by source |
| **Per-session** | All mutations in a session share one source tag | Medium | Sessions are meaningful units of work |
| **Per-source** | Each source has an identity, but individual mutations aren't logged | Low | Only need "who contributed" not "exactly what" |

**Per-mutation provenance with an event log** is the gold standard. The storage cost is proportional to write rate, which for a local knowledge graph is modest (hundreds to low thousands of mutations per session).

### 8.4 Conflict Resolution

| Strategy | Behavior | Appropriate when |
|----------|----------|-----------------|
| **Prevent** | Structural design ensures conflicts can't occur (e.g., each source writes to its own namespace) | Sources are truly independent |
| **Detect and merge** | Detect conflicting writes, apply merge function | Conflicts are rare and have meaningful merge semantics |
| **Last-write-wins** | Later timestamp overwrites earlier | Conflicts are acceptable losses |
| **Commutative operations** | Operations designed to commute (CRDTs) | All operations are associative + commutative |

For a graph where mutations are primarily **additive** (add node, add edge) rather than **conflicting** (modify same field on same node), conflict resolution is largely a non-issue. Two sources adding different nodes simultaneously is not a conflict — it's normal operation. The only real conflict is two sources modifying the same node's content simultaneously, which the single-writer-per-node assumption prevents.

### 8.5 Source Hierarchy

| Model | Behavior | Appropriate when |
|-------|----------|-----------------|
| **All equal** | No source has priority; all mutations are treated identically | Sources are truly symmetric |
| **Prioritized** | Some sources' writes take precedence (e.g., user > agent > background) | Interactive response time matters |
| **Role-based** | Sources have different capabilities (read-only, read-write, admin) | Security or trust boundaries exist |

For a system where the user must remain responsive, **prioritized** is practical: user writes should never block behind a queue of background agent writes. This is a scheduling concern, not a consistency concern — all mutations are applied, but user mutations are processed first.

### 8.6 Visibility: When Do Other Sources See a Write?

| Model | Behavior | Appropriate when |
|-------|----------|-----------------|
| **Immediate** | Write is visible to all readers as soon as it completes | Writes are atomic (single node/edge add) |
| **Batch-atomic** | A group of writes becomes visible all at once or not at all | Multi-node operations must appear atomic |
| **Lazy** | Write is visible after explicit flush or sync | Sources batch writes for performance |

For additive graph mutations (single node or edge adds), **immediate visibility** is simplest and sufficient. If a source needs to add 10 related nodes atomically, batch-atomic visibility prevents other sources from seeing a partially-constructed subgraph.

### 8.7 Rollback: Undoing a Source's Contributions

Rolling back all mutations from source X is straightforward with an event log: filter out X's events, replay the rest. But cascading effects complicate this:

- If source A adds node N and source B adds an edge to N, rolling back A's contribution orphans B's edge
- Options: cascade-delete dependent structures, leave orphans marked for review, or prevent rollback when cross-source dependencies exist

The safest approach: rollback marks contributions as "retracted" rather than physically deleting them, letting dependent sources decide how to respond.

### 8.8 Access Control: What Can Each Source Write?

| Level | Description | Appropriate when |
|-------|-------------|-----------------|
| **Unrestricted** | Any source writes anything | All sources are trusted (single-user local app) |
| **Append-only** | Sources can add but not modify or delete | External/untrusted sources |
| **Scoped** | Sources can only write specific node/edge types | Different sources have different domains |

For a single-user local system, **unrestricted with provenance** is sufficient — trust all sources but record everything for selective undo. Access control becomes important when external or untrusted sources are added.

---

## 9. Red/Green Team

### Green Team (Factual Verification)

22 of 24 factual claims verified. Key corrections:

- **Figma and CRDTs**: Figma does NOT use pure distributed CRDTs. It uses a centralized server for conflict resolution with CRDT-inspired techniques. Corrected in §4.6.
- **Greg Young's event sourcing formalization**: Public sources date to ~2010, not 2005. The 2005 date could not be verified. Corrected in §4.2.
- **ZFC Principle attribution**: "Zeno's Fourth Commandment" origin could not be verified in public sources. Likely Gas Town internal terminology. Corrected in §6.2.

All other claims (HEARSAY-II, PROV-O, CRDTs/Shapiro, Hewitt/actors, ROS 2 DDS QoS, Gas Town beads/nudges, Neo4j ACID, TypeDB optimistic CC, Dgraph Raft, LangGraph/AutoGen/MetaGPT patterns) verified accurate.

### Red Team (Challenges)

**Missing pattern (now addressed)**: Tuple spaces / Linda model was absent from the original analysis. Added as §4.7. Tuple spaces offer temporal and spatial decoupling but lack provenance and ordering — important limitations for a knowledge graph.

**Recommendation was premature**: The original executive summary declared a winner before the analysis was complete. Revised to be descriptive rather than prescriptive.

**Unaddressed failure modes (now addressed)**: Schema evolution breaking event replay (§7.7), event log bloat (§7.8), source trust/authentication (§7.9), and write-burst starvation (§7.10) were all absent. Added.

**Missing design decisions (now addressed)**: Visibility semantics (§8.6), rollback cascading effects (§8.7), and access control (§8.8) were not discussed. Added.

**"Conversation trap" is overstated**: Many successful systems use sequential processing (git commits, database transactions, Slack messages). The anti-pattern is not sequential processing itself, but forcing conversation semantics onto inherently concurrent multi-source input. The distinction matters — sequential processing by choice (for simplicity) vs. by assumption (because "that's how LLM APIs work") are different problems.

**Provenance overhead unquantified**: Per-mutation provenance at ~100 bytes/event × 1,000 mutations/session × 365 days ≈ 36 MB/year. Modest for desktop, potentially significant for resource-constrained environments. The right granularity depends on the deployment target.

### Conceptual Accuracy

All seven pattern descriptions verified as fundamentally sound. Three imprecisions corrected:

- **Blackboard**: Now mentions reactive/event-driven self-triggering variants alongside classical scheduler model (§4.1)
- **Actor model**: "No shared memory" qualified — pure model forbids it, production systems (Akka) allow shared immutable references. Batch-processing variants mentioned (§4.3)
- **Shared mutable state**: "Immediate consistency" qualified — requires proper synchronization (acquire-release semantics) to guarantee visibility across cores (§4.5)

---

## 10. Sources

### Academic & Foundational
- Erman, L.D. et al. (1980). "The Hearsay-II Speech-Understanding System: Integrating Knowledge to Resolve Uncertainty." *Computing Surveys*.
- Gelernter, D. (1985). "Generative Communication in Linda." *ACM TOPLAS*.
- Hewitt, C. (1973). "A Universal Modular ACTOR Formalism for Artificial Intelligence." *IJCAI*.
- Shapiro, M. et al. (2011). "Conflict-free Replicated Data Types." *SSS 2011*.
- Young, G. (~2010). "CQRS Documents." https://cqrs.files.wordpress.com/2010/11/cqrs_documents.pdf

### Standards & Specifications
- W3C PROV-O: The PROV Ontology (2013). https://www.w3.org/TR/prov-o/
- OMG DDS (Data Distribution Service) Specification. https://www.omg.org/spec/DDS/
- RDF 1.1: Named Graphs. https://www.w3.org/TR/rdf11-concepts/#section-dataset

### Systems & Frameworks
- Gas Town (Yegge, 2024-2025): Multi-agent coding system, 20-30 agents, durable/ephemeral split
- ROS 2 DDS: https://design.ros2.org/articles/ros_on_dds.html
- Apache Kafka: https://kafka.apache.org/ — event sourcing at scale
- Datomic: https://www.datomic.com/ — immutable database, event sourcing
- EventStoreDB: https://www.eventstore.com/ — purpose-built event store

### Multi-Agent Frameworks
- LangGraph: https://langchain-ai.github.io/langgraph/
- AutoGen (Microsoft): https://microsoft.github.io/autogen/
- CrewAI: https://www.crewai.com/
- MetaGPT: https://github.com/geekan/MetaGPT

### Graph Databases
- Neo4j Transaction Management: https://neo4j.com/docs/operations-manual/current/database-internals/transaction-management/
- TypeDB: https://typedb.com/
- Dgraph: https://dgraph.io/

### CRDTs
- Automerge: https://automerge.org/
- Yjs: https://yjs.dev/
- rust-crdt: https://github.com/rust-crdt/rust-crdt

### LLM Blackboard Systems
- arXiv:2507.01701 — "LLM Blackboard Systems" (2025)
- Confluent: Event-Driven Multi-Agent Systems. https://www.confluent.io/blog/event-driven-multi-agent-systems/
