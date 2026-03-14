# Kubernetes-Inspired Agent Scheduling

> Research conducted 2026-03-14. Designs a Kubernetes-inspired scheduling system for
> multi-agent task assignment in the context-orchestrator, mapping K8s primitives to
> graph-native agent orchestration with adapted resource modeling.

---

## 1. Executive Summary

The context-orchestrator currently runs a single agent loop per user message. As we scale to 2-30 concurrent agents (per doc 07's target), we need a principled system for assigning work items to agents. This document maps Kubernetes scheduling concepts to multi-agent LLM orchestration, identifies where the metaphor holds, where it breaks, and proposes adaptations.

**The core mapping:**

| K8s Concept | Context-Orchestrator Mapping |
|---|---|
| Node | `AgentNode` — an agent instance with a model, tools, and context window |
| Pod | `WorkItem` — a plan or task from the graph, with its tool calls |
| Label | Agent capability tags (`model:sonnet-4`, `specialty:code-review`) |
| Taint | Agent limitation flags (`rate-limited:NoSchedule`, `context-full:PreferNoSchedule`) |
| Toleration | WorkItem flags permitting scheduling to tainted agents |
| Extended Resource | Model capabilities (`anthropic.com/sonnet-4: 1`, like `nvidia.com/gpu: 1`) |
| ResourceQuota | Per-agent token budget ceilings per time window |
| Node Condition | `RateLimitPressure`, `ContextPressure` — trigger automatic taints |
| Priority Class | WorkItem priority (`Critical`, `High`, `Normal`, `BestEffort`) |
| Preemption | Cancel `BestEffort` work via `CancellationToken` to run `Critical` work |

**Key finding:** K8s scheduling is bin-packing — maximize utilization of fungible compute. LLM agent scheduling is capability-routing with inverse utilization — match model capabilities and prefer agents with *more* available context, since high context utilization degrades quality. The adaptation retains K8s's Filter+Score pipeline but replaces the bin-packing objective with a capability-match + context-headroom objective.

**Recommendation:** A hybrid architecture combining graph-native topological dispatch (what to schedule) with a K8s-inspired Filter+Score pipeline (where to schedule it). The graph's `DependsOn` edges determine the ready set; the scheduler assigns ready items to agents based on labels, taints, resources, and scoring.

---

## 2. Current Architecture & Gap Analysis

### 2.1 What Exists Today

The single-agent architecture in `src/app/mod.rs:212-266` spawns exactly one `agent_loop::spawn_agent_loop` per user message. The `AgentLoopConfig` (`src/app/agent_loop.rs:19-25`) carries `model`, `max_tokens`, `max_context_tokens`, `max_tool_loop_iterations`, and `tools` — these are proto-resource specifications hardcoded from `AppConfig`.

Existing graph primitives relevant to scheduling:

- **WorkItem nodes** (`src/graph/node.rs:152-161`) with `WorkItemKind` (Plan/Task) and `WorkItemStatus` (Todo/Active/Done) — the "pod" abstraction already exists
- **DependsOn edges** (`src/graph/mod.rs:251-276`) with `dependencies_of()`, `has_dependency_path()` — dependency-aware scheduling is already expressible
- **SubtaskOf edges** with `children_of()`, `parent_of()` — hierarchical task decomposition
- **Status propagation** (`src/graph/mutation.rs:133-199`) — `propagate_status()` auto-transitions parents when children complete
- **CancellationToken hierarchy** (`src/app/mod.rs:46-49`) — preemption mechanism exists
- **BackgroundTask lifecycle** (`src/graph/node.rs:94-102`) — Pending/Running/Completed/Failed/Stopped

### 2.2 Gaps

| Capability | Current State | Gap |
|---|---|---|
| Agent identity | None — single anonymous agent | No `AgentNode` in graph, no capability tracking |
| Task assignment | Implicit — user message triggers one agent | No scheduling decision, no queue, no routing |
| Resource tracking | Hardcoded from AppConfig | No token budget accounting, no model capability registry |
| Priority | FIFO by user message order | No priority classes, no preemption policy |
| Load balancing | N/A — single agent | No topology spread, no multi-model distribution |
| Agent health | None | No heartbeat, no condition-based taints |

### 2.3 Existing Infrastructure to Reuse

- `WorkItem` + `DependsOn` + `SubtaskOf` edges → ready-set computation is a graph query
- `CancellationToken` hierarchy → preemption is already wired
- `mutate_node` with `NodeSnapshot` history → scheduling state changes get free version tracking
- Doc 07's three-layer architecture (Graph State, Event Log, Ephemeral Signals) → scheduling event bus
- Doc 07's `GraphCoordinator` actor → scheduler as a coordinator client

---

## 3. Requirements

Derived from VISION.md, CLAUDE.md, and documented user preferences:

1. **Graph-native.** Scheduling decisions are graph nodes/edges. Agent capabilities are queryable via graph traversal. No separate scheduling state store.
2. **Async from the start.** The scheduler runs as a concurrent process, not blocking the App event loop.
3. **Tools equally callable.** Scheduling configuration (labels, taints, priorities) is expressible as tool calls, callable by both users and agents (per feedback: no separate systems).
4. **Two-phase scheduling.** Filter (hard constraints) then Score (soft preferences).
5. **Resource-aware.** Token budget quotas per time window, model capabilities as extended resources, context headroom as a scheduling signal.
6. **Priority with preemption.** Higher-priority work can preempt lower-priority work via `CancellationToken`.
7. **Observable.** "Why was this task assigned to this agent?" is answered by a graph traversal.
8. **Incremental.** Phase 1 works with a single agent as a no-op pass-through.
9. **Reconciled.** Must integrate with doc 07's GraphCoordinator and doc 05's Gas Town patterns.

---

## 4. The K8s Mapping — What Works, What Adapts, What Doesn't

### 4.1 Concepts That Map Directly

**Labels and Selectors.** Agent capabilities map naturally to K8s labels. An agent running Sonnet 4 has `model: sonnet-4`, `provider: anthropic`, `specialty: code-generation`. A work item requiring code generation specifies `nodeSelector: { specialty: code-generation }`. Label selectors (equality-based and set-based) work unchanged.

**Taints and Tolerations.** Agent degradation maps to taints. When an agent hits its API rate limit, it receives `rate-limited: NoSchedule`. When its context window exceeds 80% utilization, it receives `context-pressure: PreferNoSchedule`. A background compaction task tolerates `context-pressure` because it adds minimal context; an interactive user task does not.

**Priority and Preemption.** K8s PriorityClasses map directly to work item priorities. The existing `CancellationToken` hierarchy provides the preemption mechanism — cancel a `BestEffort` background task to free an agent for `Critical` interactive work.

**Filter+Score Pipeline.** The two-phase scheduling model translates cleanly:
- **Filter:** Eliminate agents lacking required labels, bearing blocking taints, or missing required model capabilities.
- **Score:** Rank remaining agents by affinity match, context headroom (inverse utilization), budget remaining, and load balance.

### 4.2 Resource Model — Adapted, Not Abandoned

K8s pod-level resource requests (`requests: { cpu: 2, memory: 4Gi }`) do not map to LLM scheduling because tokens are consumed over a conversation's lifetime, not allocated and returned per task. However, resources DO map when reframed:

**Model capabilities as Extended Resources.** Like `nvidia.com/gpu: 1`, model capabilities are the primary scheduling signal:

```yaml
# Agent "alpha" exposes:
allocatable:
  anthropic.com/sonnet-4: 1
  context-tokens: 200000

# WorkItem requests:
resources:
  requests:
    anthropic.com/sonnet-4: 1  # must be Sonnet 4
    context-tokens: 50000       # estimated context needed
```

This is the most important adaptation. In K8s, `nvidia.com/gpu: 1` ensures GPU workloads land on GPU nodes. In our system, `anthropic.com/sonnet-4: 1` ensures code-generation tasks land on Sonnet agents, not Haiku summarization agents.

**Token budgets as ResourceQuota.** Per-agent, per-time-window ceilings — not per-task allocations. An agent has a daily budget of 1M input tokens. The scheduler checks "does this agent have budget remaining?" before assignment. This prevents runaway cost when parallelism increases.

**Context window as Allocatable with inverse scoring.** Context window capacity is like node memory: `allocatable: { context-tokens: 200000 }`. But unlike CPU utilization, high context utilization degrades LLM performance (context rot — VISION.md section 2.1 cites "a focused 300-token context often outperforms an unfocused 113,000-token context"). The scheduler must **prefer agents with more available context** — the opposite of K8s bin-packing.

**Rate limits as Node Conditions.** Like `MemoryPressure` and `DiskPressure`, rate limit exhaustion is a node condition: `RateLimitPressure`. Conditions trigger automatic taints (`rate-limited: NoSchedule`) that resolve when the rate limit window resets. These are externally imposed, may change unpredictably, and should not be modeled as accountable resources.

### 4.3 Concepts That Do Not Map

| K8s Concept | Why It Doesn't Map |
|---|---|
| Pod Restartability | LLM agents are stateful — the context window is accumulated conversation state. "Restarting" an agent means reconstructing context from the graph, not replaying a container image. |
| Horizontal Pod Autoscaling | Adding agents means adding API cost, not compute. HPA would autoscale into bankruptcy. Agent count is a budget decision, not a load-reactive decision. |
| Service Discovery | Agents share a single process and graph. They don't expose network endpoints. Doc 07's `AgentIdentity` nodes solve identity without DNS/service meshes. |
| Persistent Volumes | The `ConversationGraph` is shared state, not per-agent storage. Per-agent state contradicts the single-source-of-truth architecture. |
| Readiness/Liveness Probes | An agent's "liveness" is the LLM API responding. A local heartbeat doesn't help when the bottleneck is external. Model as a taint condition instead. |

### 4.4 The Core Adaptation

K8s asks: "Which node has free resources?" (bin-packing, maximize utilization).
We ask: "Which agent has the right model and enough context headroom?" (capability-routing, minimize context utilization).

| Signal | K8s Priority | Our Priority |
|---|---|---|
| Resource fit (CPU/memory) | Primary | Tertiary (budget quotas) |
| Label match | Secondary (nodeSelector) | Primary (model capabilities as extended resources) |
| Utilization | Maximize (bin-pack) | Minimize (context headroom — inverse bin-pack) |
| Node conditions | Trigger taints | Same — trigger taints |
| Affinity | Soft/hard preferences | Same — specialty-based |

---

## 5. Options Analysis

### Option A: K8s-Style Filter+Score (Adapted)

Map the K8s scheduling framework directly. A `Scheduler` struct maintains a priority queue of unscheduled work items. Filter eliminates infeasible agents; Score ranks the remainder. Extension points: PreFilter, Filter, PostFilter (preemption), PreScore, Score, Reserve (decrement budget), Bind (create `ScheduledTo` edge).

**Strengths:** Battle-tested at planetary scale, well-understood semantics, extensible plugin model. **Weaknesses:** Potentially over-engineered for 2-5 agents, K8s vocabulary learning curve.

### Option B: Nomad-Style Constraints + Affinity

HashiCorp Nomad uses explicit constraints (hard) plus affinity scoring (soft). No taints/tolerations — constraints serve both purposes. Bin-packing or spread algorithms for placement.

**Strengths:** Simpler mental model, fewer concepts. **Weaknesses:** No taint equivalent for dynamic agent degradation, less composable than K8s extension points.

### Option C: Supervisor Agent (LLM-Based Scheduling)

A supervisor LLM with access to the work item graph decides dispatch via tool calls (`dispatch_to_agent`). This is Gas Town's Mayor pattern — the most successful production multi-agent system uses imperative dispatch, not constraint-based scheduling.

**Strengths:** Flexible (prompt-upgradeable, no code changes), debuggable (reasoning visible as `ThinkBlock` nodes), handles qualitative routing naturally. **Weaknesses:** Costs tokens for scheduling decisions, adds latency (LLM call per dispatch), non-deterministic (same state may produce different assignments).

### Option D: Dependency-Aware Topological Dispatch

No scheduler. Compute the ready set from `DependsOn` edges: work items where all dependencies are `Done` and no agent is currently working on them. Assign to the next available agent round-robin. This is the "graph already solves the core problem" approach.

**Strengths:** Trivial to implement, zero overhead, graph-native. **Weaknesses:** No quality-of-fit optimization, no preemption, no model-aware routing — all agents must be interchangeable.

### Option E: Simple Priority Queue + Capability Filter

`BinaryHeap` ordered by priority class. Dequeue the highest-priority item, filter agents by required capabilities, assign to least-loaded match. ~15 lines of code.

**Strengths:** Easy to implement, low overhead, fully testable. Handles priorities, capability matching, and load balancing. Easily extended to Option A if needed. **Weaknesses:** No soft preferences, no taint mechanism, no topology awareness.

### Option F: Work-Stealing (Agents Pull Tasks)

Agents poll a shared work queue for tasks matching their capabilities. Each agent claims tasks atomically, managing their own load. Natural backpressure: busy agents don't poll.

**Strengths:** No central scheduler, agents self-manage load, simple concurrency model. **Weaknesses:** Polling adds latency (unless using notify/wait), less load-aware globally, requires atomic claim mechanism on `WorkItem` nodes.

---

## 6. Comparison Matrix

| Criterion | A: K8s-Style | B: Nomad | C: Supervisor | D: Topo Dispatch | E: Queue+Filter | F: Work-Stealing |
|---|---|---|---|---|---|---|
| Complexity | High | Medium | Medium | Low | Low | Low |
| Model-aware routing | Extended resources | Constraints | LLM reasoning | None | Capability filter | Capability filter |
| Dynamic degradation | Taints + conditions | Re-express constraints | LLM observes state | None | Manual | Agent self-manages |
| Preemption | PriorityClass native | Limited | LLM decides | None | None | Agent yields |
| Observability | Decision logs + edges | Constraint evaluation | ThinkBlock nodes | Trivial | Trivial | Claim logs |
| Graph-native fit | Excellent (edges) | Good | Excellent (tool calls) | Native | Good | Good |
| Incremental adoption | Phase 1 = no-op | Easy | Needs supervisor agent | Trivial | Trivial | Trivial |
| Cost efficiency | Code-only decisions | Code-only | Token cost per dispatch | Code-only | Code-only | Code-only |
| Qualitative routing | Labels approximate | Constraints approximate | LLM understands | None | Labels approximate | None |
| Prior art maturity | 10+ years (planet-scale) | 8+ years (production) | Gas Town (imperative dispatch) | Common pattern | Common pattern | Tokio/Rayon (proven) |

---

## 7. VISION.md Alignment

| VISION.md Section | How Scheduling Maps |
|---|---|
| 3.2 Context Construction | Each agent gets a subgraph view. Scheduler decides *which* agent; context construction decides *what it sees*. |
| 4.3 Background Processing | Compaction/rating tasks are `BestEffort` priority, scheduled to cheap model agents (`model:deepseek-v3`). |
| 4.4 Multi-Rater Relevance | Rating agents have `specialty:relevance-rating` labels. Cascade evaluation: cheap model first, escalate if uncertain. |
| 5.4 Multi-Model Table | Model table maps to agent labels: `model:sonnet-4` for conversation, `model:deepseek-v3` for batch, `model:qwen-14b-local` for local. |
| 5.5 Cost Model ($24/month) | Token budget quotas enforce the cost model. Scheduler routes background work to cheap agents. |

---

## 8. Recommended Architecture

### 8.1 Approach: Hybrid (Option D foundation + Option A pipeline)

The graph's `DependsOn` edges determine **what** to schedule (the ready set). The K8s-inspired Filter+Score pipeline determines **where** to schedule it (which agent). This separates concerns:

- **Ready-set computation** — graph query, no scheduler needed: `work_items.filter(|wi| wi.status == Todo && wi.dependencies.all(Done))`
- **Agent assignment** — Filter (model capability match, taint check, budget check) then Score (context headroom, affinity, load balance)

For complex orchestration decisions beyond algorithmic capability (e.g., "this task requires understanding of module X, which agent Y has been working on"), a supervisor agent (Option C) can participate as an optional Phase 4 overlay.

### 8.2 Core Types

```rust
/// Agent capability label (key-value pair on Agent nodes).
pub struct Label { pub key: String, pub value: String }

/// Agent limitation, triggering scheduling avoidance.
pub struct Taint {
    pub key: String,
    pub value: String,
    pub effect: TaintEffect,
}

pub enum TaintEffect {
    NoSchedule,          // Hard: do not schedule here
    PreferNoSchedule,    // Soft: avoid if alternatives exist
    NoExecute,           // Evict existing work (severe degradation)
}

/// WorkItem tolerance for agent taints.
pub struct Toleration {
    pub key: String,
    pub operator: TolerationOperator,  // Equal or Exists
    pub value: Option<String>,
    pub effect: Option<TaintEffect>,
}

/// Model capabilities as extended resources (like nvidia.com/gpu: 1).
/// Key format: "provider.com/model-name", value: count (usually 1).
pub type ExtendedResources = HashMap<String, u32>;

/// Agent resource state for scheduling decisions.
pub struct AgentResources {
    pub extended: ExtendedResources,       // e.g. {"anthropic.com/sonnet-4": 1}
    pub context_allocatable: u32,          // Total context window (e.g. 200_000)
    pub context_used: u32,                 // Current context utilization
    pub token_budget: TokenBudget,         // Quota per time window
    pub conditions: Vec<NodeCondition>,    // RateLimitPressure, ContextPressure
}

pub struct TokenBudget {
    pub limit: u64,
    pub used: u64,
    pub window: Duration,
    pub window_start: DateTime<Utc>,
}

pub enum NodeCondition {
    RateLimitPressure,    // Provider rate limit approaching/exceeded
    ContextPressure,      // Context window > 80% utilized
    ProviderDown,         // API endpoint unreachable
}

/// Work item priority class.
pub enum PriorityClass {
    BestEffort = 0,   // Background compaction, rating. Preemptable.
    Normal = 1,       // Standard interactive work.
    High = 2,         // Time-sensitive (user waiting).
    Critical = 3,     // System-critical (error recovery, data integrity).
}

/// Scheduling requirements on a WorkItem.
pub struct SchedulingSpec {
    pub node_selector: Vec<Label>,              // Hard: agent MUST have these
    pub tolerations: Vec<Toleration>,           // Tolerate specific taints
    pub affinity: Option<AgentAffinity>,        // Soft/hard agent preferences
    pub resource_requests: ExtendedResources,   // Required capabilities
    pub priority_class: PriorityClass,
}
```

### 8.3 Scheduler Trait

```rust
/// Plugin for the Filter+Score pipeline.
pub trait SchedulerPlugin: Send + Sync {
    fn filter(&self, spec: &SchedulingSpec, agent: &AgentSnapshot) -> FilterResult;
    fn score(&self, spec: &SchedulingSpec, agent: &AgentSnapshot) -> u32; // 0-100
}

pub enum FilterResult {
    Feasible,
    Infeasible(String),  // Reason for rejection (for observability)
}

/// Immutable snapshot of agent state at scheduling time.
pub struct AgentSnapshot {
    pub id: AgentId,
    pub node_id: Uuid,
    pub labels: Vec<Label>,
    pub taints: Vec<Taint>,
    pub resources: AgentResources,
    pub active_work_items: u32,
}
```

**Built-in plugins:** `LabelMatchPlugin` (filter by nodeSelector, score by affinity), `TaintTolerationPlugin` (filter by taint/toleration compatibility), `ResourceFitPlugin` (filter by extended resource availability, score by context headroom — higher headroom = higher score), `LoadBalancePlugin` (score by inverse active work items).

### 8.4 Graph Integration

New `Node` variant:

```rust
Agent {
    id: Uuid,
    agent_id: AgentId,
    labels: Vec<Label>,
    taints: Vec<Taint>,
    resources: AgentResources,
    status: AgentStatus,    // Ready, Busy, Draining, Offline
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}
```

New `EdgeKind` variant: `ScheduledTo` — `WorkItem --ScheduledTo--> Agent`.

### 8.5 Phased Delivery

**Phase 1: Foundation (single-agent compatible).** Add `src/scheduler/` with core types. Add `PriorityClass` to `WorkItem`. Implement `schedule_one()` that is a pass-through with one agent. Wire into `handle_send_message` before `spawn_agent_loop`.

**Phase 2: Multi-agent pool.** `AgentPool` managing N agent instances with different `AgentLoopConfig`s. Label-based routing: `model:sonnet-4` vs `model:deepseek-v3`. Scheduling loop as a separate tokio task consuming graph events.

**Phase 3: Dynamic scheduling.** Taints for degraded agents (auto-applied from rate limit headers, per doc 17). Preemption via `CancellationToken`. `NodeCondition` monitoring.

**Phase 4: Supervisor overlay.** Optional supervisor agent for qualitative routing decisions beyond what labels/scores can express. Participates via tool calls (`dispatch_to_agent`), not by replacing the algorithmic scheduler.

### 8.6 User Story

A developer has a plan with 6 tasks: 3 independent code implementation tasks, 2 code review tasks (depend on the implementations), 1 documentation task. The system has 4 agents: 2 running Sonnet 4 (`model:sonnet-4`, `specialty:code-generation`), 1 running Haiku (`model:haiku`, `specialty:review`), 1 running DeepSeek (`model:deepseek-v3`, `specialty:documentation`).

1. **Ready-set:** All 3 implementation tasks are ready (no dependencies). Doc task is ready. Review tasks are blocked.
2. **Filter:** Implementation tasks require `model:sonnet-4`. Only 2 agents match. Doc task has no model requirement.
3. **Score:** Two Sonnet agents tie on labels. Agent with more context headroom scores higher. Doc task goes to DeepSeek (label match + only eligible agent).
4. **Assignment:** 2 implementations start on Sonnet agents. Doc task starts on DeepSeek. 3rd implementation queues.
5. **Completion:** First implementation finishes. 3rd implementation dequeues to the freed Sonnet agent. When all 3 are done, review tasks enter ready-set and are dispatched to the Haiku agent (label match on `specialty:review`).
6. **Degradation:** Sonnet rate limit hit → `RateLimitPressure` condition → auto-taint `rate-limited:NoSchedule` → 3rd implementation re-queues, schedules when taint expires.

---

## 9. Integration Design

### 9.1 Data Flow

```
WorkItem created (Todo)
  → DependsOn check: all dependencies Done?
  → If yes: enqueue in scheduler priority queue
  → Scheduler snapshots agent pool
  → Filter: label match, taint check, extended resource fit, budget check
  → Score: context headroom, affinity, load balance
  → Winner selected
  → Graph mutation: add ScheduledTo edge, update WorkItem to Active
  → Agent receives assignment via GraphEvent broadcast
  → Agent loop starts processing
```

### 9.2 Reconciliation with Doc 07 (GraphCoordinator)

The scheduler operates as a client of the `GraphCoordinator` from doc 07:

- **Reads:** Agent snapshots (labels, taints, resources), WorkItem status, DependsOn edges
- **Writes:** `ScheduledTo` edges, WorkItem status transitions (Todo → Active)
- **Events:** Consumes `GraphEvent::NodeAdded(WorkItem)`, produces `GraphEvent::EdgeAdded(ScheduledTo)`

The scheduler does NOT own the graph. It submits mutations to the coordinator, which serializes writes (solving the `Arc<RwLock>` contention concern from the red team).

### 9.3 Reconciliation with Doc 05 (Gas Town)

Gas Town uses GUPP: "If there is work on your Hook, YOU MUST RUN IT." The scheduler fills the hook — it assigns work to agents, and agents execute immediately upon assignment. The scheduler is the dispatch mechanism; GUPP is the execution contract. This is analogous to Gas Town's `gt sling` command, which the Mayor uses to dispatch work to rigs.

### 9.4 Tool Integration

New tools for scheduling configuration, following the existing pattern in `src/tool_executor/mod.rs`:

| Tool | Description | Callable by |
|---|---|---|
| `set_agent_label` | Add/update a label on an agent | Users and agents |
| `remove_agent_label` | Remove a label from an agent | Users and agents |
| `set_agent_taint` | Apply a taint to an agent | Users, agents, auto (conditions) |
| `remove_agent_taint` | Remove a taint from an agent | Users, agents, auto (recovery) |
| `set_work_item_priority` | Set priority class on a work item | Users and agents |

### 9.5 Resource Update Flow

```
LLM response received (with rate limit headers from provider)
  → Parse x-ratelimit-remaining, x-ratelimit-reset (per doc 17)
  → Update AgentResources on Agent node
  → If remaining < threshold: apply NodeCondition::RateLimitPressure
  → Condition triggers auto-taint: rate-limited:NoSchedule
  → Taint auto-expires when rate limit window resets (TTL-based)
```

---

## 10. Red/Green Team

### 10.1 Green Team (Validates)

- **Battle-tested pipeline.** Filter+Score has been proven at planet-scale. The two-phase model handles heterogeneous workloads with composable constraints. All K8s documentation claims in this document verified accurate.
- **Graph-native fit.** Labels as node properties, `ScheduledTo` as edge kind. Adding a new agent capability = adding a label. No scheduler code changes needed.
- **Extended resources for model routing.** `anthropic.com/sonnet-4: 1` follows K8s's `nvidia.com/gpu: 1` pattern — the most successful heterogeneous-compute scheduling mechanism in production.
- **Resource tracking has standalone value.** Token budgets, rate limit monitoring, and context utilization tracking are useful regardless of scheduling approach.
- **Internal references verified.** All file:line references, enum/struct claims, and architectural descriptions match the actual codebase (12 references, 100% accuracy).

### 10.2 Red Team (Challenges)

**Severity: High — Architectural concerns**

- **Filter+Score may be overkill at target scale.** For 2-5 identical agents, a simple `agents.filter(|a| a.model == task.model).min_by_key(|a| a.load)` produces equivalent results in 3 lines vs. a full plugin architecture. K8s scheduling has 10+ plugins because it handles dozens of scenarios; we have 1-2 filter criteria and 1-2 scoring factors today. *Mitigation: Start with Option E (simple queue + capability filter). Graduate to Option A only if you accumulate 5+ filter criteria or 5+ scoring factors. The types (Label, Taint, PriorityClass) are useful regardless of scheduling complexity.*

- **Supervisor agent (Option C) should be considered as Phase 2, not Phase 4.** Gas Town's Mayor uses imperative dispatch at 20-30 agent scale — the most successful production system. A supervisor LLM making 10 dispatch decisions at 500 tokens each costs ~$0.001 per conversation — negligible. It handles qualitative routing naturally ("this task is about module X, which agent Y has context on"). The algorithmic scheduler should be an optimization overlay on top of the supervisor, not the other way around. *Mitigation: The document presents both options. Implementation should evaluate: if most routing decisions are capability-based (model match), start with algorithmic. If most are context-dependent, start with supervisor.*

- **Extended resources are semantically awkward for binary capabilities.** `anthropic.com/sonnet-4: 1` is always 1 — it's a boolean, not a quantity. A `HashSet<Capability>` with set membership checks is cleaner in Rust than `HashMap<String, u32>`. *Mitigation: Implement as typed capabilities (`enum Capability { Model(String), Specialty(String) }`) internally. Map to K8s extended resource semantics only if needed for external compatibility.*

- **Token budgets are per-API-key, not per-agent.** If 5 agents share one Anthropic API key, they share the same rate limit and quota. The "per-agent budget" model assumes isolated API keys. *Mitigation: Track quota at the API key level. Agents inherit their provider's quota. Budget check becomes: `(key.used + estimated_cost) <= key.limit`.*

**Severity: Medium — Implementation concerns**

- **Phase 1 as no-op is dead code** per CLAUDE.md rules ("No dead code, unless you are about to use it"). A `schedule_one()` that always returns the only agent adds no value. *Mitigation: Skip Phase 1. Build Phase 1+2 together when ready to implement multi-agent, validated against a concrete use case.*

- **Context headroom estimation is imprecise and may mislead.** If estimates are +/-50% off, a safety margin of 50% of allocatable is needed, effectively halving usable context. The inverse-utilization assumption ("less loaded = better") lacks empirical validation — at what utilization % does quality actually degrade? *Mitigation: Use headroom as a soft tiebreaker only (never hard filter). Collect telemetry before baking thresholds into scoring.*

- **Observability requires decision logs, not just graph edges.** A `ScheduledTo` edge tells you the assignment but not why agent A was chosen over agent B. *Mitigation: Each scheduling decision must record: `{ task_id, candidates: [(agent_id, filter_pass, score)] }`. Logs are queryable by task or agent.*

- **Preemption via CancellationToken is underspecified.** Missing: how does the scheduler detect a task is freeable? What if cancellation is slow (I/O-bound tool call)? What happens to tokens consumed before cancellation? *Mitigation: Preemption must define: (a) signaling mechanism, (b) grace period, (c) partial work tracking for cost attribution.*

**Severity: Low — Scope concerns**

- **Cost amplification.** VISION.md's $24/month assumes one agent. At 30 concurrent Sonnet agents: $500+/month. *Mitigation: Token budget quotas as first-class signal. Cost dashboard before enabling multi-agent.*

- **Graph bloat from Agent nodes.** *Mitigation: Agent nodes are long-lived (one per agent). O(agents + assignments), not multiplicative. Alternatively, keep agent state outside the graph in a separate `AgentPool` struct.*

- **Solving an anticipated problem.** No evidence that scheduling is a current bottleneck. *Mitigation: Build scheduling when there's a concrete multi-agent scenario. The research document provides the architectural vocabulary; implementation waits for demand.*

### 10.3 Open Questions

1. **Algorithmic vs. supervisor-first?** Should Phase 2 be algorithmic (Filter+Score) or LLM-based (supervisor agent)? Depends on whether most routing decisions are capability-based or context-dependent.
2. **Capability representation:** Typed `HashSet<Capability>` (Rust-idiomatic) or `HashMap<String, u32>` (K8s-compatible)? Trade-off between type safety and extensibility.
3. **Context headroom thresholds:** At what utilization % does quality degrade? Linear, step-function, or threshold? Needs empirical measurement before encoding in scheduler.
4. **Preemption safety:** How to handle partial work during eviction? Record "preempted, X tokens consumed" for cost tracking?
5. **Work-stealing vs. push-based:** Should agents pull tasks (Option F) or receive assignments (Option A)? Pull is simpler; push enables better global optimization.

---

## 11. Sources

### Kubernetes Scheduling
- [Kubernetes Scheduler](https://kubernetes.io/docs/concepts/scheduling-eviction/kube-scheduler/) — two-phase Filter+Score architecture
- [Scheduling Framework](https://kubernetes.io/docs/concepts/scheduling-eviction/scheduling-framework/) — extension points: PreFilter through PostBind
- [Taints and Tolerations](https://kubernetes.io/docs/concepts/scheduling-eviction/taint-and-toleration/) — NoSchedule, PreferNoSchedule, NoExecute
- [Node Affinity](https://kubernetes.io/docs/concepts/scheduling-eviction/assign-pod-node/) — required/preferred scheduling terms
- [Pod Priority and Preemption](https://kubernetes.io/docs/concepts/scheduling-eviction/pod-priority-preemption/) — PriorityClass, preemption policy
- [Extended Resources](https://kubernetes.io/docs/concepts/configuration/manage-resources-containers/#extended-resources) — `nvidia.com/gpu: 1` pattern
- [Resource Quotas](https://kubernetes.io/docs/concepts/policy/resource-quotas/) — per-namespace budget ceilings

### Alternative Schedulers
- [Nomad Scheduling](https://developer.hashicorp.com/nomad/docs/concepts/scheduling/how-scheduling-works) — constraints + affinity + bin-packing/spread
- [Mesos Architecture](https://mesos.apache.org/documentation/latest/architecture/) — two-level scheduling, Dominant Resource Fairness
- [Agent.xpu](https://arxiv.org/html/2506.24045v1/) — dual-queue design (real-time + best-effort) for agentic LLM workloads
- [QLLM](https://arxiv.org/html/2503.09304v1) — priority-aware preemption for MoE inference, per-expert queues

### Multi-Agent Frameworks
- [AutoGen](https://microsoft.github.io/autogen/) — SelectorGroupChat dynamic agent selection
- [CrewAI](https://docs.crewai.com/) — role-based sequential/hierarchical execution
- [LangGraph](https://blog.langchain.com/langgraph-multi-agent-workflows/) — state machine with conditional routing
- No existing LLM multi-agent framework uses K8s-style Filter+Score scheduling; all use role/capability/state routing. (K8s-style multi-agent scheduling exists in edge-cloud contexts, e.g., [KaiS](https://link.springer.com/article/10.1186/s13677-023-00471-1), but not for LLM orchestration.)

### Rust Ecosystem
- [taskflow-rs](https://crates.io/crates/taskflow-rs) — async task orchestration with dependency management
- [Ractor](https://github.com/slawlor/ractor) — actor model (recommended in doc 07)
- [libpcp](https://crates.io/crates/libpcp) — constraint satisfaction problem solver
- [Tokio cooperative scheduling](https://tokio.rs/blog/2020-04-preemption) — budget-based fairness

### Internal References
- `docs/research/05-gastown-multi-agent-orchestration.md` — 8 agent roles, GUPP execution model, imperative dispatch
- `docs/research/07-inter-agent-communication.md` — three-layer architecture, GraphCoordinator, blackboard pattern
- `docs/research/17-litellm-provider-integration.md` — rate limit headers, cost tracking
- `docs/VISION.md` — context construction (3.2), background processing (4.3), multi-model (5.4), cost model (5.5)
