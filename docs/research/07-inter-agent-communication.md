# Inter-Agent Communication Methods

> Research conducted 2026-03-12. Analysis of communication patterns, protocols, and
> architectures for multi-agent AI systems, with specific recommendations for Context
> Manager's graph-native architecture.

---

## 1. Executive Summary

Context Manager currently operates as a single-agent system where background tasks communicate with the main event loop through one `tokio::sync::mpsc` channel. As we scale to 2-30 concurrent agents sharing and mutating the conversation graph, this single-channel, single-owner model becomes insufficient. We need a principled inter-agent communication architecture that preserves the graph as the single source of truth while enabling concurrent, fault-tolerant agent coordination.

This document surveys five categories of inter-agent communication patterns — message-passing, shared-state, event-driven, protocol-based, and blackboard — across nine major multi-agent frameworks (LangGraph, AutoGen, CrewAI, OpenAI Swarm, Google ADK, Gas Town, A2A, MCP, ACP). The analysis reveals that Context Manager's property graph already functions as a proto-blackboard, and the natural evolution is a **three-layer architecture**:

| Layer | Medium | Purpose |
|-------|--------|---------|
| **Graph State** | `ConversationGraph` | Durable, queryable truth |
| **Event Log** | `Vec<GraphEvent>` + broadcast | History, notifications, replay |
| **Ephemeral Signals** | tokio channels (mpsc, broadcast, watch) | Heartbeats, backpressure, liveness |

This hybrid is more expressive than any single framework's communication model. It combines the blackboard's shared state, tuple space's content-addressable access, LangGraph's persisted state, and Gas Town's durable/ephemeral split — unified in a typed property graph with out-of-band notification channels.

**Key finding:** The graph-as-blackboard pattern is validated by recent research (arXiv:2507.01701) showing 13-57% improvements over traditional multi-agent communication, with fewer tokens consumed. Gas Town's production experience validates the durable+ephemeral split — switching protocol messages from persistent beads to ephemeral nudges reduced Dolt commit volume by 80%.

---

## 2. Problem Statement

### 2.1 Current Architecture

The current communication topology is minimal and sound for single-agent operation:

```
GitWatcher  --|
ToolDiscovery--|--[mpsc::UnboundedSender<TaskMessage>]--> App (owns graph)
Summarizer  --|                                            |
ToolExtract --|                                          select! loop
```

- **Single channel**: All background tasks share one `mpsc::UnboundedSender<TaskMessage>` (`src/tasks.rs:36-49`)
- **Single owner**: `App` owns `ConversationGraph` exclusively with `&mut self` access (`src/app/mod.rs:21-30`)
- **Fire-and-forget**: Tasks produce messages, App consumes — no request-response
- **Snapshot isolation**: Background LLM tasks receive `ContextSnapshot` clones, not shared references (`src/tasks.rs:23-27`)

The `TaskMessage` enum has four variants: `GitFilesUpdated`, `ToolsDiscovered`, `TaskStatusChanged`, and `ToolExtractionComplete`. The `App::handle_task_message` method (`src/app/task_handler.rs:10-78`) processes each variant by mutating the graph directly.

**Note**: `ToolDiscovery` and `Summarizer` are currently stubs — tool discovery returns a hardcoded list, and context summarization is a no-op. The architecture is wired but the implementations are placeholders for future development.

### 2.2 What Changes with Multi-Agent

| Concern | Single-Agent (Current) | Multi-Agent (Target) |
|---------|----------------------|---------------------|
| Graph access | Exclusive `&mut self` | Concurrent read/write from N agents |
| Communication | Unidirectional (task → app) | Bidirectional, inter-agent |
| Failure | Background task crash = lost update | Agent crash requires recovery + restart |
| Backpressure | None (unbounded channel) | Essential — 30 agents can overwhelm |
| Coordination | Sequential processing | Priority, ordering, conflict resolution |

---

## 3. Communication Pattern Taxonomy

### 3.1 Message-Passing Patterns

#### Tokio Channel Types

| Channel | Topology | Backpressure | Delivery | Use Case |
|---------|----------|-------------|----------|----------|
| `mpsc` (bounded) | N:1 | Yes (send blocks) | At-most-once | Agent → coordinator mutations |
| `mpsc` (unbounded) | N:1 | None (OOM risk) | At-most-once | Current TaskMessage channel |
| `broadcast` | N:M | Lagged receivers | Best-effort | Graph change notifications |
| `watch` | 1:M | Latest-value-wins | Last-writer | Graph version, agent status |
| `oneshot` | 1:1 | N/A (single use) | Exactly-once | Request-response queries |

The **"send the sender" pattern** combines `mpsc` with `oneshot` for request-response semantics: the request message embeds a `oneshot::Sender<Response>`, and the handler sends the response back through it. This is how the coordinator would serve graph queries without shared mutable state.

#### Actor Model

Actors encapsulate state and process messages sequentially from a mailbox, eliminating shared-mutable-state bugs by construction. Five Rust actor frameworks are relevant:

| Framework | Runtime | Supervision | Spawn 10K | Distribution |
|-----------|---------|-------------|-----------|-------------|
| **Ractor** | Tokio | Erlang-style | ~5ms | ractor_cluster |
| **Kameo** | Tokio | Yes | ~68ms | Built-in |
| **Actix** | Actix RT | Limited | Fastest | No |
| **Coerce** | Tokio | Yes | Slowest | Built-in |
| **Xtra** | Multi-RT | No | Good | No |

**Ractor** is the strongest candidate for Context Manager: Erlang-style supervision trees, Tokio-native, production-validated at Meta, process groups for pub/sub, and 4-level priority messaging (Signal > Stop > Supervision > Regular). For 2-5 agents, raw tokio channels suffice. For 6+ agents, Ractor's supervision alone justifies the dependency.

Sources: [Ractor GitHub](https://github.com/slawlor/ractor), [Actor Benchmarks](https://github.com/tqwewe/actor-benchmarks), [Comparing Rust Actor Libraries (2025)](https://tqwewe.com/blog/comparing-rust-actor-libraries/)

### 3.2 Shared-State Patterns

#### Single-Owner Graph (Current)

The `App` struct owns the `ConversationGraph` and processes mutations sequentially in its `select!` loop. No concurrency bugs are possible. The limitation: only one entity can read or write the graph at a time, and background tasks work on stale snapshots.

#### Arc\<RwLock\<ConversationGraph\>\>

Wrap the graph in `Arc<RwLock<...>>`. Multiple agents acquire read locks concurrently; writers get exclusive access. **Acceptable for 2-3 agents, unacceptable for 10+.** Write-lock contention under 30 agents kills throughput. Tokio's `RwLock` does not support lock upgrades, creating deadlock traps.

#### DashMap: Wrong Abstraction

`DashMap` provides fine-grained concurrent key-value access, but **graph operations are multi-step transactions**. `add_message` inserts a node, pushes an edge, and updates the branch pointer — three operations that must be atomic. DashMap cannot enforce this. An agent observing the graph mid-mutation sees an inconsistent state (node without edge, edge pointing to nonexistent node).

#### Actor-Owned Graph (Recommended)

A `GraphCoordinator` task owns the graph exclusively. All mutations go through its bounded `mpsc` inbox as typed commands. Reads are served as copy-on-write snapshots via `oneshot` response channels. This preserves single-writer semantics while enabling concurrent agents:

- **No locks, no races, no inconsistent states** — the graph is never accessed from multiple threads
- **Scales to 30+ agents** — agents run concurrently, coordinator processes mutations sequentially
- **Not a bottleneck** — graph operations are microsecond-scale; LLM latency (seconds) dominates
- **Architecturally similar to the current design** — the `App` struct's `select!` loop with `task_rx` is already a hand-rolled actor pattern, though extracting a full GraphCoordinator requires bounded channels, oneshot queries, broadcast events, and converting all background tasks to the new protocol

#### CRDTs

Conflict-Free Replicated Data Types enable concurrent writes without coordination. Relevant Rust crates: `crdts` (G-Counter, OR-Set, LWW-Register), `automerge` (JSON CRDT), `loro` (rich text + movable tree). **Future consideration only** — CRDTs are designed for distributed systems with network partitions, which Context Manager does not have. Metadata overhead (tombstones, version vectors) grows with concurrent writers.

#### Event Sourcing

Record every graph mutation as an immutable event. The graph is a materialized projection of the event log. Benefits: complete audit trail, time-travel debugging, agent replay/catch-up, undo/redo. Rust crates: `cqrs-es`, `esrs` (Prima, production-used). **Recommended as a lightweight variant**: the coordinator maintains an ordered `Vec<GraphEvent>` alongside the live graph, providing debugging capability without full CQRS infrastructure.

Sources: [Event Sourcing: Backbone of Agentic AI (Akka)](https://akka.io/blog/event-sourcing-the-backbone-of-agentic-ai), [CQRS in Rust](https://doc.rust-cqrs.org/)

### 3.3 Event-Driven Patterns

#### Pub/Sub via Broadcast

After the coordinator applies a graph mutation, it broadcasts a `GraphEvent` (e.g., `NodeAdded`, `EdgeAdded`, `NodeRemoved`) via `tokio::sync::broadcast`. Each agent subscribes and filters for events it cares about. This is simple, scales well with receivers, and degrades gracefully (slow agents receive `Lagged` errors and can request a fresh snapshot).

#### Confluent's Four Multi-Agent Patterns

A taxonomy from Confluent (January 2025):

1. **Orchestrator-Worker**: Central coordinator dispatches to worker agents. Our GraphCoordinator + agent pool.
2. **Hierarchical Agent**: Tree of agents with delegation. Parent breaks down tasks, children execute.
3. **Blackboard**: Shared event log as communication medium. Our graph + event log.
4. **Market-Based**: Agents bid on tasks via auction. Relevant for heterogeneous agent pools.

Our architecture combines patterns 1 (coordinator) and 3 (blackboard), which Confluent identifies as the most common pairing in production systems.

Source: [Confluent - Event-Driven Multi-Agent Systems](https://www.confluent.io/blog/event-driven-multi-agent-systems/)

### 3.4 Protocol-Based Patterns

#### Google A2A (Agent-to-Agent)

Launched April 2025, now v0.3 under Linux Foundation. Client-server model over HTTPS + JSON-RPC 2.0. Agents are **opaque** — they don't share memory, tools, or logic. Communication happens through Tasks with lifecycle states (submitted → working → completed/failed). **Agent Cards** advertise capabilities as JSON at `.well-known/agent-card.json`.

**Relevance**: A2A is designed for inter-organization agent communication. Context Manager's agents are intra-system — they share the same process and graph. A2A's opacity model is the wrong fit. However, the Agent Card concept is valuable: each agent could have a capability-advertising node in the graph.

Sources: [A2A Protocol](https://a2a-protocol.org/latest/), [IBM - What is A2A](https://www.ibm.com/think/topics/agent2agent-protocol)

#### Anthropic MCP (Model Context Protocol)

Widely adopted standard for agent-to-tool communication via JSON-RPC 2.0. The 2026 roadmap adds a Tasks primitive and session migration, converging toward agent-to-agent communication. MCP's host-as-orchestrator pattern mirrors Context Manager's coordinator architecture.

Source: [MCP 2026 Roadmap](http://blog.modelcontextprotocol.io/posts/2026-mcp-roadmap/)

#### OpenAI Swarm / Agents SDK

Two primitives: **Routines** (system prompt + tools) and **Handoffs** (agent-to-agent transfer via function calls). Stateless — every handoff includes all context. The handoff pattern maps naturally to graph edge creation: Agent A produces a node, creates a `DelegateTo` edge to Agent B.

Source: [Orchestrating Agents: Routines and Handoffs](https://developers.openai.com/cookbook/examples/orchestrating_agents/)

#### Google ADK (Agent Development Kit)

Event-driven ask-yield runtime. 8 documented multi-agent patterns: sequential pipeline, parallel scatter-gather, dynamic delegation, loop agent, hierarchical teams, competitive, human-in-the-loop, pipeline parallelism. Communication via shared session state with `output_key` auto-save. Three-layer architecture: ADK (orchestration) + A2A (agent-to-agent) + MCP (agent-to-tool).

Source: [ADK Multi-Agent Patterns](https://developers.googleblog.com/developers-guide-to-multi-agent-patterns-in-adk/)

#### Gas Town

The most sophisticated open-source multi-agent orchestrator (20-30 agents). Yegge reports API costs of $100-200/hr at full scale across multiple Claude Pro Max accounts. Three communication tiers:

| Mechanism | Persistence | Cost | Use Case |
|-----------|------------|------|----------|
| **Mail** | Permanent bead + Dolt commit | High | Protocol messages (POLECAT_DONE, MERGE_READY) |
| **Nudge** | Ephemeral, session-scoped | Zero | Health checks, wake signals |
| **Handoff** | Mail + new session | Medium | Context transfer between sessions |

GUPP ("If there is work on your Hook, YOU MUST RUN IT") ensures agents proceed without waiting for confirmation. ZFC ("Go provides transport. Agents provide cognition.") separates infrastructure from reasoning.

**Key lesson**: Switching protocol messages from durable mail to ephemeral nudges reduced Dolt commit volume by ~80%. This directly validates our three-layer split.

Sources: [Gas Town GitHub](https://github.com/steveyegge/gastown), [Maggie Appleton - Gas Town Patterns](https://maggieappleton.com/gastown)

### 3.5 Blackboard Patterns

#### Classic Architecture (Hearsay-II, BB1)

The blackboard pattern originated at CMU (1970s) with three components: a **blackboard** (shared memory), **knowledge sources** (independent specialists), and a **control component** (activation scheduler). Knowledge sources are triggered by blackboard state changes — opportunity-driven activation, not direct invocation.

BB1 (Stanford, 1980s) introduced a **control blackboard** for meta-reasoning about which agent to activate next. This maps directly to a supervisor agent watching graph mutations and making scheduling decisions.

#### Modern LLM Blackboard (arXiv:2507.01701)

A 2025 paper integrating blackboard architecture into LLM multi-agent systems found: competitive with SOTA static and dynamic MAS, best average performance, fewer tokens consumed, 13-57% relative improvements in end-to-end success on data science tasks. The key: agents communicate **solely** through the blackboard, no direct contact.

Source: [arXiv:2507.01701](https://arxiv.org/html/2507.01701v1)

#### Context Manager's Graph as Blackboard

The `ConversationGraph` already functions as a proto-blackboard:

| Blackboard Concept | Graph Mapping |
|-------------------|---------------|
| Levels | Node types (Message, WorkItem, GitFile, Tool) |
| Hypotheses | Nodes with properties (status, confidence) |
| KS triggers | Graph pattern queries (`nodes_by()`) |
| Control component | App event loop + `select!` |
| Multi-level reasoning | Edge traversal across node types |

**Evolution path**: (1) Current — unidirectional task→app. (2) Bidirectional — agents observe graph changes via broadcast and react. (3) Full blackboard — agents register condition-action rules, control component evaluates and activates.

---

## 4. The Graph as Communication Medium

### 4.1 Why It Works

The property graph is strictly more expressive than any other communication medium analyzed:

- **Durable**: nodes persist, surviving agent crashes
- **Queryable**: `get_branch_history()`, `nodes_by()`, edge traversal
- **Relationship-aware**: typed edges encode *why* two items are related
- **Context-native**: the graph IS the LLM context — agents read the relevant subgraph
- **Composable**: complex behaviors emerge from simple node+edge patterns

LangGraph demonstrates this model at scale — shared state IS the communication channel, with production adoption by enterprise users.

### 4.2 Lessons from Tuple Spaces

Linda's associative memory (Yale, 1980s) provides design patterns for graph-based communication:

| Linda Primitive | Graph Equivalent |
|----------------|-----------------|
| `out(tuple)` — insert | `add_node()` + `add_edge()` |
| `in(pattern)` — destructive read | Claim semantics: mark WorkItem as ClaimedBy agent |
| `rd(pattern)` — non-destructive read | `nodes_by(predicate)` |
| `eval(tuple)` — spawn process | Create BackgroundTask node |
| Blocking read | Graph subscription via broadcast channel |

Key insight: agents should query by pattern, not by node ID. The `nodes_by()` method already supports this — "give me any WorkItem with status=Todo" is content-addressable lookup.

### 4.3 When NOT to Use the Graph

**Decision framework**: default to graph nodes. Use channels only when:

1. **High-frequency signals** (>1/sec): heartbeats at 10 agents = 600 nodes/min. Graph explodes.
2. **Sub-millisecond coordination**: backpressure signals need immediate delivery.
3. **No semantic content**: "I'm alive" carries no queryable information.
4. **Privacy**: some agent-internal state shouldn't be visible to all agents.
5. **Binary data**: large payloads bloat the graph.

**Everything else should be a node.** If any agent will reference it later, if it has meaningful relationships, or if it describes a domain entity — it's a node.

---

## 5. Recommended Architecture: Three Layers

```
┌─────────────────────────────────────────────────────┐
│  Layer 3: Ephemeral Signals                         │
│  tokio channels: heartbeats, backpressure, shutdown │
├─────────────────────────────────────────────────────┤
│  Layer 2: Event Log                                 │
│  append-only Vec<GraphEvent>, broadcast to agents   │
├─────────────────────────────────────────────────────┤
│  Layer 1: Graph State                               │
│  ConversationGraph: materialized, queryable truth   │
└─────────────────────────────────────────────────────┘
```

### 5.1 Channel Topology

```
Agent 1 --[bounded mpsc]--> GraphCoordinator --[broadcast]--> Agent 1
Agent 2 --[bounded mpsc]--> GraphCoordinator --[broadcast]--> Agent 2
  ...                            |                              ...
Agent N --[bounded mpsc]--> GraphCoordinator --[broadcast]--> Agent N
                                 |
                            [watch: graph version]
                            [watch: active branch]
                                 |
                            TUI event loop
```

The GraphCoordinator is the sole graph owner. Agents send `GraphCommand` variants (AddMessage, AddNode, GetSnapshot, etc.) via bounded `mpsc`. The coordinator processes mutations sequentially, broadcasts `GraphEvent` notifications, and serves reads as copy-on-write `Arc<ConversationGraph>` snapshots via `oneshot` response channels.

### 5.2 Priority Processing

```rust
select! {
    biased;
    msg = critical_rx.recv() => handle_critical(msg),  // user actions
    msg = normal_rx.recv()   => handle_normal(msg),    // agent mutations
    msg = bulk_rx.recv()     => handle_bulk(msg),      // git file updates
}
```

Per-agent bounded channel capacities tuned by agent type: git watcher (4, latest-state-wins), tool discovery (8), summarizer (16), user-facing LLM agent (32).

### 5.3 New Graph Types for Multi-Agent

New node types needed: `AgentIdentity` (name, capabilities, status) and `Claim` (agent claims work item, with TTL-based expiry). New edge types: `ClaimedBy`, `ProducedBy`, `DelegateTo`. These enable capability-based routing, conflict resolution via claims, and attribution of all graph mutations to specific agents.

### 5.4 Backpressure and Failure Handling

**Backpressure**: Bounded channels with per-agent capacity. When full, agent's `.send().await` blocks — natural flow control. Burst producers (git watcher) coalesce pending updates before retry. Circuit breaker on LLM provider via `tower-circuitbreaker`.

**Failure recovery**: With raw channels, a spawner loop restarts crashed agents with exponential backoff. With Ractor, the supervision tree handles restart policy automatically. The actor-owned graph eliminates partial-mutation cleanup — if an agent crashes, its command was either fully received by the coordinator or not at all.

**Stuck agent detection**: Periodic heartbeats via `watch` channel. Coordinator maintains last-seen timestamps and flags agents exceeding 2x expected interval.

---

## 6. Comparison with Existing Systems

| Framework | Communication | State Model | Graph-Aware | Durable | Concurrent Agents |
|-----------|--------------|-------------|-------------|---------|-------------------|
| **LangGraph** | Shared state dict | Typed schema, reducers | No (flat dict) | Checkpointed | Via super-steps |
| **AutoGen** | Shared message thread | Append-only chat | No (linear chain) | In-memory | Speaker selection |
| **CrewAI** | Task pipeline | Sequential output | No | Per-run | Sequential |
| **OpenAI Swarm** | Handoffs | Stateless | No | None | Transfer-only |
| **Google ADK** | Session state | Key-value | No | Checkpointed | 8 patterns |
| **Gas Town** | Mail + nudges | Dolt SQL + Git | Bead refs | Git-backed | 20-30 agents |
| **MetaGPT** | Shared memory pool | Message queue + env | No | Per-run | Role-based |
| **A2A** | JSON-RPC tasks | Opaque | No | Server-dep | Unlimited (dist) |
| **Context Manager** | **Graph + events + channels** | **Property graph** | **Yes (typed edges)** | **JSON + events** | **2-30 (planned)** |

**Where Context Manager is weak**: ecosystem maturity (pre-release vs. years of production use for LangGraph/AutoGen), community size (single developer vs. thousands of contributors), language support (Rust-only vs. Python ecosystems), and production validation (none vs. enterprise deployments).

**Where Context Manager is strong**: relationship-aware state (typed property graph), durable communication (nodes + event log), ephemeral coordination (channels), graph-native agent discovery (capability nodes), and observable context construction. No other framework unifies all of these.

**MetaGPT** deserves special mention as the closest domain competitor — it targets multi-agent software development with structured outputs (PRD, design docs, code). Its communication uses a shared message pool with subscription-based routing, where agents publish `Message` objects and subscribers filter by `cause_by` (the action type that produced the message). This is conceptually similar to our broadcast + filter pattern, though MetaGPT uses role-based routing where Context Manager would use graph-pattern-based activation.

---

## 7. Migration Path

| Phase | Changes | Agent Scale | Risk |
|-------|---------|-------------|------|
| **1: GraphCoordinator** | Extract graph into own task, bounded mpsc, oneshot queries, broadcast events | 2-5 | Low — refactor of existing `select!` loop |
| **2: Agent Trait** | Define Agent with start/stop/handle_graph_event, convert background tasks | 5-10 | Medium — API design |
| **3: Ractor** | Adopt actor framework, supervision trees, process groups | 6-30 | Medium — new dependency |
| **4: Advanced** | Priority channels, circuit breaker, event log, dynamic agent pool | 15-30 | High — production tuning |

Phase 1 is the natural starting point. The current `App` with its `select!` loop over `task_rx` already follows the actor pattern structurally, though extracting a full GraphCoordinator involves non-trivial work: new command/event enums, bounded channel migration, oneshot query protocol, and converting all background tasks.

---

## 8. Red Team / Green Team

### Green Team (Validates Approach)

- **Graph-as-blackboard is validated** by arXiv:2507.01701 — 13-57% improvements over traditional multi-agent communication, with fewer tokens consumed.
- **LangGraph demonstrates shared-state communication at scale** — enterprise production adoption validates the pattern.
- **Gas Town validates durable+ephemeral split** — 80% commit volume reduction when switching protocol messages from persistent to ephemeral.
- **Actor-owned graph eliminates all shared-mutable-state bugs** by construction. Single writer, sequential processing, no locks.
- **Three-layer architecture aligns with every successful system reviewed** — Gas Town (beads/nudges), LangGraph (state/messages), Google ADK (session/events).
- **Current architecture follows the target pattern** — the `App` struct's `select!` loop is structurally similar to a GraphCoordinator actor.
- **The graph's typed edges make it strictly more expressive** than flat state dicts (LangGraph), append-only logs (AutoGen), or opaque task protocols (A2A).

### Red Team (Challenges and Risks)

- **Single GraphCoordinator is a throughput bottleneck.** Mitigated: graph operations are microsecond-scale; 300 mutations/sec from 30 agents is trivial. LLM latency (seconds) dominates end-to-end time.
- **Blackboard activation patterns add control complexity.** Deciding which agent to activate based on graph state requires careful rule design. Risk of activation storms or starvation.
- **Event sourcing adds significant complexity** for marginal debugging gains at current scale. A lightweight `Vec<GraphEvent>` log is sufficient until event replay becomes operationally necessary.
- **Ractor adds dependency weight and learning curve.** For <6 agents, raw channels with hand-rolled supervision are simpler and equally correct.
- **No production validation of graph-as-blackboard for LLM agent systems at our planned scale.** The arXiv paper validates the concept; production proof is missing.
- **Graph size management under multi-agent.** 30 agents creating nodes aggressively can cause unbounded growth. Requires garbage collection, archival, or summarization-based compaction.
- **ZFC principle caution (Gas Town).** Over-encoding agent coordination logic in Rust (transport) vs. leaving decisions to agents (cognition) risks building fragile heuristics. The three-layer architecture should expose data to agents, not make decisions for them.
- **Debugging multi-agent interactions is hard.** Reproducing a bug requires replaying all agent interactions in order. No existing observability tooling handles graph-based multi-agent debugging well.
- **Testing is expensive and non-deterministic.** Unit testing individual agents is insufficient; integration testing with LLM calls is costly and results vary. Mock-based testing misses emergent interaction bugs.
- **API cost scales linearly with agent count.** 30 agents making concurrent LLM calls can burn $100+/hr (Gas Town's experience). Context Manager targets $24/month per developer for single-agent; multi-agent cost projection needs careful modeling.
- **Graph query performance degrades.** `nodes_by()` does a linear scan of all nodes (`src/graph/mod.rs:331-333`). At 10K+ nodes from 30 aggressive agents, this becomes a coordinator bottleneck requiring index structures.
- **Schema migration burden.** Adding `AgentIdentity` and `Claim` nodes requires a V3 graph schema version with a migration path from V2. Each new node/edge type has a serialization cost.
- **Unbounded event log.** The recommended `Vec<GraphEvent>` grows without bound. Needs compaction, rotation, or memory caps — adding complexity that partially negates the "lightweight variant" framing.

---

## 9. Synthesis

The survey of 10+ multi-agent frameworks and 5 communication pattern categories reveals a convergent design: successful systems separate durable state from ephemeral coordination. Gas Town uses beads+nudges. LangGraph uses state+messages. Google ADK uses session+events. The pattern is universal.

Context Manager's property graph gives us a unique advantage: our durable layer is not just a flat state dict or an append-only log, but a typed, relationship-aware graph that supports multi-hop queries, subgraph extraction, and content-addressable lookup. This makes the graph a natural blackboard — validated by recent research showing 13-57% improvements for blackboard-based multi-agent LLM systems.

The recommended path forward is incremental: extract a `GraphCoordinator` from the existing `App` (Phase 1), formalize agents as a trait (Phase 2), adopt Ractor for supervision when agent count warrants it (Phase 3), and add advanced patterns as needed (Phase 4). Each phase delivers value independently and does not require subsequent phases.

The primary risk is not technical but organizational: the temptation to over-engineer the coordination layer. Gas Town's ZFC principle is instructive — Rust should provide transport (channels, graph operations, serialization), while agents provide cognition (what to do, when to act, how to coordinate). The three-layer architecture enables this separation cleanly.

---

## 10. Sources

### Multi-Agent Frameworks
- [LangGraph Documentation](https://docs.langchain.com/oss/python/langgraph/overview)
- [AutoGen Group Chat](https://microsoft.github.io/autogen/stable//user-guide/core-user-guide/design-patterns/group-chat.html)
- [AutoGen Paper (arXiv:2308.08155)](https://arxiv.org/abs/2308.08155)
- [CrewAI Documentation](https://docs.crewai.com/en/concepts/agents)
- [OpenAI Swarm GitHub](https://github.com/openai/swarm)
- [Orchestrating Agents: Routines and Handoffs](https://developers.openai.com/cookbook/examples/orchestrating_agents/)
- [Google ADK Documentation](https://google.github.io/adk-docs/)
- [ADK Multi-Agent Patterns](https://developers.googleblog.com/developers-guide-to-multi-agent-patterns-in-adk/)
- [Gas Town GitHub](https://github.com/steveyegge/gastown)
- [Maggie Appleton - Gas Town Patterns](https://maggieappleton.com/gastown)
- [DoltHub - A Day in Gas Town](https://www.dolthub.com/blog/2026-01-15-a-day-in-gas-town/)

- [MetaGPT GitHub](https://github.com/geekan/MetaGPT)
- [MetaGPT Paper (arXiv:2308.00352)](https://arxiv.org/abs/2308.00352)

### Protocols
- [Google A2A Protocol](https://a2a-protocol.org/latest/)
- [IBM - What is A2A](https://www.ibm.com/think/topics/agent2agent-protocol)
- [Linux Foundation A2A Launch](https://www.linuxfoundation.org/press/linux-foundation-launches-the-agent2agent-protocol-project)
- [MCP Specification 2025-11-25](https://modelcontextprotocol.io/specification/2025-11-25)
- [MCP 2026 Roadmap](http://blog.modelcontextprotocol.io/posts/2026-mcp-roadmap/)
- [IBM ACP Overview](https://www.ibm.com/think/topics/agent-communication-protocol)
- [Survey of Agent Interoperability Protocols (arXiv:2505.02279)](https://arxiv.org/abs/2505.02279)

### Rust Concurrency
- [Ractor GitHub](https://github.com/slawlor/ractor)
- [Kameo GitHub](https://github.com/tqwewe/kameo)
- [Comparing Rust Actor Libraries (2025)](https://tqwewe.com/blog/comparing-rust-actor-libraries/)
- [Actor Benchmarks](https://github.com/tqwewe/actor-benchmarks)
- [Tokio Channels Tutorial](https://tokio.rs/tokio/tutorial/channels)
- [Async Channels in Rust](https://medium.com/@adamszpilewicz/async-channels-in-rust-mpsc-broadcast-watch-which-one-fits-your-app-0ceaf566a092)
- [Avoiding Over-Reliance on mpsc](https://blog.digital-horror.com/blog/how-to-avoid-over-reliance-on-mpsc/)
- [Event Bus in Tokio](https://blog.digital-horror.com/blog/event-bus-in-tokio/)
- [Rust Async: Practical Patterns (Feb 2026)](https://dasroot.net/posts/2026/02/rust-async-practical-patterns-high-performance-tools/)

### Blackboard and Shared-State
- [Nii (1986) - Blackboard Systems (Stanford CS-TR-86-1123)](http://i.stanford.edu/pub/cstr/reports/cs/tr/86/1123/CS-TR-86-1123.pdf)
- [Corkill - Blackboard Systems (AI Expert)](http://mas.cs.umass.edu/Documents/Corkill/ai-expert.pdf)
- [arXiv:2507.01701 - LLM Multi-Agent Blackboard Systems](https://arxiv.org/html/2507.01701v1)
- [Blackboard Pattern with MCPs (Medium)](https://medium.com/@dp2580/building-intelligent-multi-agent-systems-with-mcps-and-the-blackboard-pattern-to-build-systems-a454705d5672)
- [Wikipedia - Tuple Space](https://en.wikipedia.org/wiki/Tuple_space)

### Event-Driven and Event Sourcing
- [Confluent - Event-Driven Multi-Agent Systems](https://www.confluent.io/blog/event-driven-multi-agent-systems/)
- [Event Sourcing: Backbone of Agentic AI (Akka)](https://akka.io/blog/event-sourcing-the-backbone-of-agentic-ai)
- [CQRS and Event Sourcing in Rust](https://doc.rust-cqrs.org/)
- [Circuit Breaker Pattern in Rust](https://sdpr.rantai.dev/docs/part-vi/chapter-41/)

### CRDTs
- [rust-crdt GitHub](https://github.com/rust-crdt/rust-crdt)
- [Automerge](https://automerge.org/)
- [Loro](https://loro.dev/)
- [CRDT Field Guide 2025](https://www.iankduncan.com/engineering/2025-11-27-crdt-dictionary/)
