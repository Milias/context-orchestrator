# Graph Scheduler + Q/A Relationship Model

> Research conducted 2026-03-14. Designs a distributed coordination model where agent loops
> self-schedule via graph state, and maps the full taxonomy of Q/A relationships to graph edges.

---

## 1. Executive Summary

The `ask` tool requires a coordination model where agents self-schedule based on graph state. Research across 8 scheduling frameworks and 3 internal research documents (doc 05 Gas Town, doc 07 Inter-Agent Communication, doc 20 K8s Scheduling) converges on one pattern: **agent loops as scheduler units**. There is no central scheduler loop. Each agent, on completion, queries the graph for ready work, claims it atomically via the GraphCoordinator (doc 07), and starts processing. The graph IS the work queue; events broadcast state changes; claims prevent duplicate execution.

This integrates with doc 20's K8s-inspired routing: when an agent finds ready work, **Filter+Score is the claim selection logic** — which agent is the best fit? For single-agent mode, this is trivial (always self). For multi-agent, it's capability matching.

For Q/A: questions and answers are graph events. Routing a question to a backend (user, LLM) is just another agent claiming work. Answers entering the graph resolve DependsOn edges, making blocked work visible to agents who then claim and process it.

Q/A relationships go beyond DependsOn. Research yielded ~30 edge types; **6 are immediately relevant** (About, DependsOn, Triggers, Supersedes, Asks/Answers), with 3 for Phase 2.

---

## 2. Scheduler Patterns in the Industry

### 2.1 Comparison Matrix

| Framework | Model | Dependency Expression | Trigger Mechanism | Centralized? |
|-----------|-------|----------------------|-------------------|-------------|
| **LangGraph** | Superstep sync | Explicit directed edges | Layer boundaries (all parents done → next layer) | Yes |
| **Temporal** | Event-driven signals | Signals + wait_condition() | State mutation unblocks waiting condition | Distributed |
| **AutoGen** | Conversation-driven | Implicit (message flow) | Speaker selection heuristics | Yes (GroupChat) |
| **CrewAI** | Sequential/hierarchical | Explicit `depends_on` | Task completion → next task | Yes (Crew) |
| **Dagster** | Asset-centric | Upstream/downstream assets | Upstream freshness → auto-materialize downstream | Yes (UI + metadata) |
| **Prefect** | Reactive ("when it matters") | Task dependencies | On-demand + event-driven triggers | Cloud control plane |
| **DynTaskMAS** | Dynamic task graph | DAG with logical deps | Async parallel execution engine | Centralized planner |

### 2.2 Key Patterns

**Pattern A: Superstep Synchronization (LangGraph)**
Execute all ready nodes in parallel, synchronize, check next layer. Deterministic but requires global sync points. Not a good fit — our agents are long-running and shouldn't block on sync barriers.

**Pattern B: Event-Driven Signals (Temporal)**
Workflows block on `wait_condition()` until state meets requirements. Signals asynchronously mutate state. Resume from exact continuation point. Graph mutations are signals, DependsOn resolution is the wait condition.

**Pattern C: Asset Auto-Materialization (Dagster)**
When upstream changes, downstream auto-refreshes. Declarative conditions define when. Interesting for "answer triggers re-evaluation" but over-engineered for our needs.

**Pattern D: Dynamic Task Graphs (DynTaskMAS)**
LLM generates DAG of subtasks, async execution engine maximizes parallelism. Complementary — our agents already generate work items; the coordination layer needs to execute them.

### 2.3 Rust Reactive Primitives

- **`reactive_graph` crate**: Fine-grained signals → effects. Async runtime agnostic.
- **`tokio::sync::watch`**: Single-producer, multi-consumer. Graph state changes broadcast to watchers.
- **`tokio::sync::broadcast`**: Multi-producer, multi-consumer. GraphCoordinator publishes events; agents subscribe.

---

## 3. Q/A Relationship Taxonomy

### 3.1 Full Taxonomy (from research)

Research across knowledge graphs, IBIS, discourse graphs, and elicitation protocols yielded ~30 edge types in 9 categories:

| Category | Edge Types | Source Domain |
|----------|-----------|---------------|
| **Dependency** | DependsOn, Resolves, BlockedBy | Task management, GitHub Issues |
| **Inference** | Infers, InformedBy, Informs | Knowledge graphs, QA systems |
| **Argumentation** | Attacks, Supports, Pro, Con, Contradicts | IBIS, Kialo, Compendium |
| **Versioning** | Supersedes, RefinesQuestion, PartiallyAnswers | Knowledge management |
| **Context** | About, References, Context | Knowledge graphs, discourse |
| **Causal** | Triggers, GeneratesSubquestion, Prompts | Workflow systems, MCP |
| **Evaluation** | AlternativeTo, PreferredOver, Strengthens, Weakens | Decision analysis |
| **Discourse** | ReplyTo, Continuation, CrossReferences, Similar | Forums, threading |
| **Elicitation** | Elicits, ProvidesContext, MissingParameter | MCP protocol |

### 3.2 What's Relevant for This System

#### Immediately Relevant (Phase 1)

| Edge | From → To | Semantics | Why Needed |
|------|-----------|-----------|------------|
| **Asks** | ToolCall → Question | "This tool call created this question" | Provenance: trace which agent asked what |
| **Answers** | Answer → Question | "This answer resolves this question" | Core Q/A pairing |
| **DependsOn** | AnyNode → Question | "This node can't proceed until this question is answered" | Blocking dependencies (already exists) |
| **About** | Question → AnyNode | "This question is about this node" | Context: what is the question referencing? A file, a work item, a message, a tool result |
| **Triggers** | Answer → AnyNode | "This answer caused the creation of this node" | Traceability: the JWT answer triggered the auth implementation work item |
| **Supersedes** | Answer → Answer | "This answer replaces that answer for the same question" | Answer evolution: user changes mind, LLM refines answer |

#### Worth Considering (Phase 2)

| Edge | From → To | Semantics | Use Case |
|------|-----------|-----------|----------|
| **Informs** | Answer → AnyNode | "This answer provides information that shapes this node" | Weaker than Triggers — the answer influenced but didn't create the node |
| **AlternativeTo** | Answer → Answer | "These are competing answers to the same question" | When the LLM suggests multiple options and the user picks one |
| **GeneratesSubquestion** | Answer → Question | "This answer raised a follow-up question" | Multi-hop: "JWT" → "Which JWT library?" |

#### Not Relevant (explicitly excluded)

| Category | Why Not |
|----------|---------|
| **Argumentation** (Pro/Con/Attacks/Supports) | This is an LLM dev tool, not a debate platform. LLMs reason internally. |
| **Semantic Web** (Predicate/Entails/Taxonomic) | Not a knowledge graph. Over-structured for our needs. |
| **Discourse threading** (ReplyTo/Continuation) | Already handled by RespondsTo edges on Messages. |
| **Elicitation** (MissingParameter) | Covered by About + DependsOn composition. |

### 3.3 How `About` Changes the Model

The `About` edge makes Questions contextual. Without it, a Question is just text. With it:

```
Question("Should I refactor this?") --About--> GitFile("src/auth.rs")
Question("JWT or sessions?") --About--> WorkItem("Design auth module")
Question("Is this migration safe?") --About--> ToolResult(read_file output)
```

This lets agents and context builders know *what the question is about*, not just *what it asks*. When constructing context for an agent, the system can include the referenced node alongside the question.

### 3.4 How `Triggers` Enables Reactive Work

When an Answer enters the graph and has `Triggers` edges, those target nodes are created/activated:

```
Answer("JWT") --Triggers--> WorkItem("Implement JWT middleware")
Answer("JWT") --Triggers--> WorkItem("Add token refresh endpoint")
```

Agents see new WorkItems → evaluate their dependencies → claim if ready. This is how answers cause new work without the answering agent explicitly calling `plan`.

**Who creates Triggers edges?** Both automatic and explicit:

1. **Automatic** (default): When an agent creates new nodes while processing an answer, the system adds `Triggers` edges from the Answer to those nodes. Agent loops carry an `answer_context_id: Option<Uuid>` — all nodes created in that context are attributed.
2. **Explicit**: The LLM can declare triggers via an `add_trigger(answer_id, target_id)` tool call for precise attribution.

---

## 4. Current Architecture Gaps

### 4.1 Existing Edge Types (10 total)

| EdgeKind | Usage | Runtime Indexed |
|----------|-------|----------------|
| RespondsTo | Message threading | Yes |
| Invoked | ToolCall → Message | Yes |
| Produced | ToolResult → ToolCall | No (linear scan) |
| SubtaskOf | WorkItem hierarchy | No |
| DependsOn | Plan prerequisites | No |
| RelevantTo | WorkItem → Message | No |
| ThinkingOf | ThinkBlock → Message | No |
| Indexes | GitFile → branch leaf | No |
| Provides | Tool → branch leaf | No |
| Tracks | Unused | N/A |

### 4.2 Missing for Coordination + Q/A

| Gap | What's Needed |
|-----|--------------|
| No readiness query | `ready_unclaimed_nodes()` — nodes where all DependsOn targets are resolved and no ClaimedBy edge |
| No claim mechanism | `try_claim()` — atomic ClaimedBy edge creation via GraphCoordinator |
| No event broadcast | GraphCoordinator broadcasts GraphEvents to agent subscribers |
| No About edge | Questions are contextless — no reference to what they're about |
| No Triggers edge | No provenance for "this answer caused that work" |
| No Supersedes edge | No answer versioning |

### 4.3 VISION.md Alignment

VISION.md doesn't mention scheduling explicitly but supports it:
- **Section 4.3**: Background processing with priority tiers (idle aggressive → active minimal)
- **Section 4.6**: Work items as graph anchors — "show me everything relevant to this task"
- **Section 5.5**: Batch API calls for non-urgent processing
- The graph + background processing model is the natural home for distributed coordination

---

## 5. Proposed Coordination Architecture

### 5.1 No Central Scheduler — Agent Loops Self-Schedule

Three internal research documents converge on the same pattern:

| Document | Key Insight |
|----------|-------------|
| **Doc 05 (Gas Town)** | GUPP: "If there is work on your Hook, YOU MUST RUN IT." Agents self-schedule by polling their queue. |
| **Doc 07 (Inter-Agent)** | GraphCoordinator broadcasts events. Agents claim work atomically. ClaimedBy edge prevents duplicates. |
| **Doc 20 (K8s Scheduling)** | Filter+Score for routing. Red team: "overkill for 2-5 agents." Option D (topo dispatch) + Option F (work-stealing) simpler. |

**The unified model**: Each agent loop, on completion, checks the graph for more work. If ready work exists, the agent claims it (or delegates to a better-fit agent). The GraphCoordinator (doc 07) serializes all mutations, broadcasts events, and ensures claims are atomic.

### 5.2 The Self-Scheduling Loop

```
Agent loop finishes (Answer produced, WorkItem completed, etc.)
  → Graph mutation via GraphCoordinator (add Answer, update status)
  → GraphCoordinator broadcasts GraphEvent (NodeAdded, StatusChanged)
  → All agents receive event via broadcast channel
  → Each agent queries graph: "any ready work matching my capabilities?"
    Ready = DependsOn all resolved + not ClaimedBy any agent
  → Agent claims matching work (ClaimedBy edge, atomic via GraphCoordinator)
  → Agent starts processing claimed work
  → On completion: cycle repeats
```

For Q/A specifically:
```
Question created → GraphEvent::NodeAdded(Question)
  → Question routing is "claiming" by the right backend:
    - UserBackend agent claims user-destined questions → shows in TUI
    - LlmBackend agent claims llm-destined questions → spawns agent loop
  → Answer produced → GraphEvent::NodeAdded(Answer)
  → Agents check: does this answer resolve any of my blocked DependsOn?
  → If so: claim unblocked work, start processing
```

### 5.3 Graph Queries Needed

```rust
impl ConversationGraph {
    /// All Questions with status Pending (not yet claimed/routed).
    fn pending_questions(&self) -> Vec<&Node> { ... }

    /// All nodes whose DependsOn targets are ALL resolved AND not ClaimedBy anyone.
    fn ready_unclaimed_nodes(&self) -> Vec<Uuid> { ... }

    /// Check if a specific node's dependencies are all resolved.
    fn all_deps_resolved(&self, node_id: Uuid) -> bool { ... }

    /// Atomically claim a node for an agent (add ClaimedBy edge).
    /// Returns false if already claimed (prevents double-assignment).
    fn try_claim(&mut self, node_id: Uuid, agent_id: Uuid) -> bool { ... }
}
```

### 5.4 Routing as Claiming

Question routing is not special — it's the same claim mechanism:

| Destination | "Agent" That Claims | What It Does |
|-------------|-------------------|-------------|
| `user` | UserBackend | Claims question, shows in TUI, waits for user input, creates Answer |
| `llm` | LlmBackend | Claims question, spawns full agent loop, agent's response becomes Answer |
| `auto` | AutoRouter | Checks interactivity, delegates claim to User or Llm backend |

All backends produce the same output: `graph.add_answer(question_id, content)` → broadcast → agents react.

### 5.5 Integration with Doc 20 (K8s Routing)

When multiple agent types exist, claiming needs routing logic. Doc 20's Filter+Score pipeline becomes the claim selection function:

```
Agent receives GraphEvent (new ready work)
  → Does this work match my capabilities? (Filter: label match, taint check)
  → Am I the best fit? (Score: context headroom, load balance)
  → If yes: try_claim(work_id, my_agent_id)
  → If claim succeeds: process
  → If claim fails (another agent was faster): ignore
```

For single-agent mode: the agent always claims. No routing needed.
For multi-agent mode: Filter+Score determines which agent attempts the claim.

### 5.6 What Gets Claimed

Any node with DependsOn edges is claimable:
- **WorkItem**: agent claims it and starts working
- **Question**: backend claims it and routes to destination
- **BackgroundTask**: agent claims it and resumes processing
- Future node types get self-scheduling for free via DependsOn + ClaimedBy

---

## 6. Proposed Edge Types for Q/A

### Phase 1 (5 new edges)

```rust
pub enum EdgeKind {
    // ... existing 10 ...
    Asks,        // ToolCall → Question (provenance)
    Answers,     // Answer → Question (resolution)
    About,       // Question → AnyNode (what is this question about?)
    Triggers,    // Answer → AnyNode (what did this answer cause?)
    Supersedes,  // Answer → Answer (answer versioning)
    // DependsOn already exists
    // ClaimedBy added for coordination (not Q/A-specific)
}
```

### Phase 2 (3 more edges)

```rust
    Informs,              // Answer → AnyNode (influenced but didn't create)
    AlternativeTo,        // Answer → Answer (competing answers)
    GeneratesSubquestion, // Answer → Question (multi-hop follow-up)
```

---

## 7. Design Decisions (Resolved)

1. **Answer acceptance**: Configurable per-question. The `ask` tool has an optional `requires_approval: bool` argument (default: false). When true, the LLM's answer enters as `PendingApproval` status — the user must accept before it becomes the real Answer and unblocks dependencies. When false, answers are auto-accepted.

2. **Scheduling order**: All ready nodes scheduled in parallel, up to concurrency limit. **No priority ordering.** Dependencies are the ONLY ordering mechanism. If 5 nodes are ready, all 5 get scheduled simultaneously.

3. **Triggers edge creation**: Both automatic (context tagging) and explicit (`add_trigger` tool). Automatic is the default; explicit overrides when the LLM wants precision.

4. **Coordination persistence**: Re-evaluation from graph state on startup. No separate state — the graph IS the state. On restart, release stale ClaimedBy edges and re-evaluate.

5. **Concurrency limit**: Configurable `max_concurrent_agents` (default TBD). When limit reached, ready nodes queue until a slot opens.

---

## 8. Red/Green Team Audit

### Green Team: All 15 factual claims VERIFIED
- All 8 codebase claims accurate (EdgeKind variants, runtime indexes, DependsOn/has_dependency_path, plan_effects pattern, propagate_status, parking_lot::RwLock, 60s timeout, Tracks unused)
- All 4 framework claims accurate (LangGraph superstep/Pregel, Temporal signals/wait_condition, DynTaskMAS at ICAPS 2025, reactive_graph crate)
- All 3 Q/A model claims accurate (IBIS elements, Kialo structure, MCP Elicitation spec)

### Red Team: 12 Issues Found

#### Critical (2)

**1. Double-routing / double-claiming** — Two agents see the same ready work and both try to process it.
**Resolution: Atomic claims via GraphCoordinator.** `try_claim(node_id, agent_id)` adds a `ClaimedBy` edge atomically. Since all mutations go through the GraphCoordinator (doc 07), claims are serialized — no race by construction.

**2. Notify lost permit** — No longer relevant. There is no Notify. Agents receive GraphEvents via broadcast channel (doc 07). Multiple rapid events → multiple broadcasts → agents process each independently.

#### High (5)

**3. Answer during agent evaluation** — Agent processing an event while another mutation occurs.
**Resolution:** GraphCoordinator provides snapshots. Agents query via snapshot, then submit claim. If state changed, claim fails. Agent re-queries on next event.

**4. 50 concurrent agents** — Already addressed: `max_concurrent_agents` with queue.

**5. Triggers edge causality ambiguity** — Auto-tracing can't distinguish "caused by answer" vs "coincidental."
**Resolution:** Context tagging. Agent loops carry `answer_context_id: Option<Uuid>`. All nodes created in that context get Triggers edges. LLM can override via explicit `add_trigger` tool.

**6. Supersedes re-evaluation** — If Answer B supersedes A, do downstream nodes re-evaluate?
**Resolution: No.** Supersedes is history/provenance only. No automatic re-evaluation. Prevents infinite loops.

**7. Rejection path for requires_approval** — User rejects LLM answer, then what?
**Resolution:** Full state machine: `Pending → Claimed → PendingApproval → Answered | Rejected | TimedOut`. On Rejected: Question returns to `Pending`, re-claimable by backends.

#### Medium (5)

**8. About edge redundancy** — Question text already describes what it's about.
**Resolution:** About is for programmatic discovery ("find all questions about node X") and context inclusion. Optional when the reference is a general concept.

**9. Lock contention under 50 concurrent agents** — parking_lot::RwLock with many writers.
**Resolution:** GraphCoordinator (doc 07) serializes all writes via mpsc channel, eliminating lock contention entirely.

**10. Missing Agent → Question provenance** — Asks goes ToolCall → Question, but Agent isn't a node.
**Resolution:** Low priority. ToolCall → parent_message_id → Message traces back to the agent.

**11. Claimed-but-unfinished work on crash** — ClaimedBy edge exists but agent is dead.
**Resolution:** On startup, scan for stale ClaimedBy edges. Release them. TTL on claims (doc 07) provides automatic expiry.

**12. Question lifecycle** — Can answered questions be re-asked?
**Resolution:** Questions are immutable once Answered. To re-ask, create a new Question.

---

## 9. Open Questions

1. **Concurrency default**: What's a reasonable default for `max_concurrent_agents`? Depends on API rate limits and cost tolerance.

2. **Answer approval UX**: When `requires_approval` is true, how does the user approve? TUI shows proposed answer with accept/reject, queued alongside user-destined questions.

---

## 10. Sources

### Scheduler Frameworks
- [LangGraph Execution Semantics](https://chbussler.medium.com/langgraph-execution-semantics-c7dd89900ed4) — Superstep sync inspired by Google Pregel
- [Temporal Workflow Message Passing](https://docs.temporal.io/encyclopedia/workflow-message-passing) — Signals + wait_condition()
- [Dagster vs Prefect Comparison](https://dagster.io/vs/dagster-vs-prefect) — Asset-centric vs reactive scheduling
- [DynTaskMAS: Dynamic Task Graphs for LLM Agents](https://arxiv.org/html/2503.07675v1) — ICAPS 2025, async parallel execution
- [reactive_graph crate](https://crates.io/crates/reactive_graph) — Async-runtime-agnostic reactive primitives for Rust

### Q/A Relationship Models
- [IBIS Framework](https://en.wikipedia.org/wiki/Issue-based_information_system) — Issues, Positions, Arguments
- [Kialo Debate Platform](https://en.wikipedia.org/wiki/Kialo) — Tree-structured pro/con arguments
- [Knowledge Graph QA - VLDB](https://www.vldb.org/pvldb/vol11/p1373-zheng.pdf) — Subquestion dependency graphs
- [Multi-hop QA with Knowledge Graphs](https://www.wisecube.ai/blog-2/multi-hop-question-answering-with-llms-knowledge-graphs/) — Multi-hop reasoning chains
- [MCP Elicitation Protocol](https://modelcontextprotocol.io/specification/draft/client/elicitation) — accept/decline/cancel response states
- [Discourse Relations Taxonomy](https://www.mdpi.com/2076-3417/13/12/6902) — Elaboration, Background, Continuation
- [GitHub Issue Dependencies](https://docs.github.com/en/issues/tracking-your-work-with-issues/using-issues/creating-issue-dependencies) — DependsOn / BlockedBy
- [Temporal Versioning in Knowledge Graphs](https://arxiv.org/html/2409.04499v1) — Answer supersession semantics

### Internal References
- `docs/research/05-gastown-multi-agent-orchestration.md` — GUPP, imperative dispatch, agent roles
- `docs/research/07-inter-agent-communication.md` — GraphCoordinator, three-layer architecture, ClaimedBy edges
- `docs/research/19-llm-question-surfacing.md` — ask tool research, Option B recommendation
- `docs/research/20-kubernetes-inspired-agent-scheduling.md` — Filter+Score pipeline, Agent nodes, ScheduledTo edges

### Codebase References
- `src/graph/node.rs:106-118` — EdgeKind enum (current 10 variants)
- `src/graph/mod.rs:251-276` — dependencies_of(), has_dependency_path()
- `src/app/task_handler.rs` — Side-effect pattern for tool completion
- `src/app/agent_loop.rs:332` — wait_for_tool_results 60s timeout
- `src/app/plan_effects.rs` — apply_plan, apply_add_task, apply_add_dependency patterns
- `docs/VISION.md:4.3` — Background processing model
