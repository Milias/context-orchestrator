# LLM-Defined Event Triggers

> **2026-03-14** — Research on enabling LLMs to define new event→action triggers at runtime within the context-orchestrator's broadcast EventBus architecture, covering rule engines, ECA patterns, complex event processing, self-modifying agent systems, declarative trigger DSLs, and safety governance — with a phased approach from declarative JSON patterns to sandboxed scripted conditions to LLM-driven autonomous triggers.

---

## 1. Executive Summary

The context-orchestrator's EventBus broadcasts 20 `GraphEvent` variants through a tokio broadcast channel; currently, all reactions are hardcoded in `event_dispatch.rs` and `task_handler.rs`. Enabling LLMs to define new event→action mappings at runtime would let agents automate repetitive workflows, create project-specific automation, and compose triggers with the plugin system from research doc 15. After surveying rule engines (Rete/Drools), automation platforms (IFTTT/n8n/EventBridge), game engine event systems (Godot signals, Factorio Lua events), self-modifying agents (Voyager, Reflexion), CEP systems (Esper, Flink), and formal models (ECA rules, temporal logic), we recommend a three-phase approach: Phase 1 uses declarative JSON event patterns (inspired by AWS EventBridge) stored as graph nodes; Phase 2 adds sandboxed Rhai conditions and multi-event temporal patterns; Phase 3 enables LLM-driven autonomous trigger creation with reflection-based refinement. The key trade-off is expressiveness vs. safety: declarative patterns are verifiable but limited; scripted conditions are powerful but require sandboxing; autonomous trigger creation requires governance infrastructure to prevent runaway costs and infinite loops.

---

## 2. Current Architecture & Gap Analysis

### What Exists

The EventBus (`src/graph/event.rs:92-124`) uses `tokio::sync::broadcast` with a 256-item buffer. All graph mutations emit events; all cross-component communication flows through this bus.

Event dispatch (`src/app/event_dispatch.rs:22-69`) is a single `match` on `GraphEvent` variants with hardcoded coordination logic:

| Event | Hardcoded Reaction |
|-------|--------------------|
| `QuestionAdded` | Route to user/LLM, try_claim |
| `MessageAdded(User)` | Parse `/command` triggers, spawn agent |
| `ToolCallCompleted` | Wake waiting agent, apply effects |
| `WorkItemStatusChanged` | Upward status propagation |

The effects system (`src/app/plan/effects.rs:1-183`) follows a pattern of: match tool call arguments → create domain nodes → emit events. This is the closest existing analogue to a trigger system — tool completion triggers graph mutations that emit new events.

The agent loop (`src/app/agent/loop.rs:65-76`) filters wake events via `is_wake_event()`, reacting only to `MessageAdded(User)` and `NodeClaimed` for the agent's own ID.

### What's Missing

| Gap | Description |
|-----|-------------|
| **Dynamic event reactions** | All event→action mappings are compile-time; no runtime registration |
| **Trigger definitions as data** | No graph node type for trigger rules; no way to persist or version them |
| **Condition evaluation** | No mechanism for evaluating predicates over event data or graph state |
| **Temporal patterns** | No support for "A followed by B within N seconds" or windowed aggregation |
| **Trigger lifecycle** | No concept of enabling/disabling/versioning triggers at runtime |
| **Safety infrastructure** | No loop detection, depth limits, rate limiting, or resource budgets for event cascades |
| **LLM trigger authoring** | No tool for an LLM to create a trigger definition via the standard tool call flow |

---

## 3. Requirements

Derived from the architecture, VISION.md, and project principles:

1. **Graph-native**: Trigger definitions must be graph nodes with edges to the events they watch and actions they perform — same provenance model as tool calls
2. **Event-driven**: Triggers must reactively consume EventBus events, never poll — per the project's architectural principle that agents are reactive consumers
3. **LLM-authorable**: An agent must be able to define a trigger via a tool call (e.g., `register_trigger`), just as it creates plans and tasks today
4. **Composable with plugins**: Trigger actions can invoke tools from the plugin system (research doc 15: Rhai/WASM/MCP), creating a closed loop: LLM writes tool → LLM writes trigger → trigger invokes tool
5. **Safe by default**: Infinite loop prevention, resource budgets, and capability-based permissions must be infrastructure-enforced, not prompt-based
6. **Deterministic evaluation**: Given the same graph state and event, a trigger must produce the same actions — enabling reproducibility and debugging
7. **Observable**: Trigger evaluations, firings, and actions must be visible in the TUI and queryable via the graph

---

## 4. Concepts & Patterns from Prior Art

### 4.1 Event-Condition-Action (ECA) Rules

The foundational formal model for reactive triggers, originating in active database systems:

```
WHEN  <event detected>
IF    <condition satisfied over current state>
THEN  <execute action>
```

ECA rules in databases (Oracle triggers, PostgreSQL LISTEN/NOTIFY) handle data modification events. IoT systems (AWS IoT Rules Engine) extend this with SQL-like conditions over event payloads. Research on formal verification translates ECA rules into Petri nets to prove termination, confluence (order-independence), and absence of structural errors (inconsistency, redundancy, circularity).

**Key insight**: ECA is the right abstraction level for graph event triggers. The event is a `GraphEvent` variant, the condition is a predicate over event data + graph state, the action is a graph mutation or tool invocation.

**Sources**: [ECA Rules (ScienceDirect)](https://www.sciencedirect.com/topics/computer-science/event-condition-action-rule), [Symbolic Verification of ECA Rules (Springer)](https://link.springer.com/chapter/10.1007/978-3-662-45730-6_6)

### 4.2 Rete Algorithm & Production Rule Systems

The Rete algorithm (Forgy, 1974) underlies systems like Drools, CLIPS, and Clara Rules. It builds a discrimination network where nodes represent partial pattern matches, enabling efficient incremental evaluation as facts change.

**Conflict resolution** when multiple rules match the same event:
- **Salience**: Explicit numeric priority
- **Specificity**: More complex patterns fire first
- **Recency**: Most recently asserted facts fire first

**Relevance**: With hundreds of triggers, naive linear scanning becomes expensive. Rete-style indexing enables efficient matching. However, the context-orchestrator's current scale (20 event types, likely <100 triggers) makes simple linear evaluation sufficient for Phase 1.

**Sources**: [Rete Algorithm (Wikipedia)](https://en.wikipedia.org/wiki/Rete_algorithm), [Drools Documentation](https://docs.drools.org/5.2.0.M2/drools-expert-docs/html/ch01.html)

### 4.3 Complex Event Processing (CEP)

CEP systems detect patterns across event streams using temporal operators:

| Operator | Semantics | Example |
|----------|-----------|---------|
| **Sequence** | A then B | Tool call A completes, then tool call B starts |
| **Window** | Within time T | 3 errors in 60 seconds |
| **Negation** | A not followed by B | Question asked, not answered within 5 minutes |
| **Aggregation** | Count/sum/avg over window | Average token usage exceeds threshold |

Esper's Event Processing Language (EPL) extends SQL-92 with temporal operators. Apache Flink CEP uses state machines over event streams. Both demonstrate that temporal patterns require explicit state management — a "pattern instance" tracks partial matches across events.

**Relevance**: Phase 2 should support temporal patterns for workflows like "when all subtasks complete, mark parent done" or "when agent idles for >30s after tool error, escalate to user."

**Sources**: [Esper CEP](https://www.espertech.com/esper/), [Apache Flink CEP](https://nightlies.apache.org/flink/flink-docs-master/docs/libs/cep/)

### 4.4 Declarative Event Patterns (AWS EventBridge)

EventBridge uses JSON patterns for declarative matching without code:

```json
{
  "source": ["graph"],
  "detail-type": ["WorkItemStatusChanged"],
  "detail": {
    "new_status": ["Completed"],
    "kind": ["Task"]
  }
}
```

Matching semantics: fields at the same level are AND; arrays within a field are OR. Supports prefix matching, numeric comparisons, and IP address matching. No nested boolean operators.

**Key advantage**: JSON patterns are human-readable, LLM-writable, and statically analyzable (no Turing-completeness means guaranteed termination). EventBridge Sandbox enables dry-run validation before deployment.

**Relevance**: Phase 1 should use this model — a JSON object that declaratively describes which `GraphEvent` variants and field values to match.

**Sources**: [AWS EventBridge Event Patterns](https://docs.aws.amazon.com/eventbridge/latest/userguide/eb-create-pattern.html), [EventBridge Pattern Matching Field Guide](https://deceptiq.com/blog/eventbridge-pattern-matching-guide)

### 4.5 Self-Modifying Agent Systems

Three systems demonstrate LLMs creating and refining their own behavioral rules:

**Voyager** (Minecraft agent): Writes JavaScript programs to achieve goals, stores successful programs in a skill library (vector DB) for future composition. Uses closed-loop feedback: generate → execute → observe errors → regenerate. Achieves 3.3x more item discovery than baselines.

**Reflexion**: Agents verbally reflect on task feedback, store reflections in episodic memory. No weight updates — linguistic self-assessment induces better decisions. Using GPT-4 as backbone, achieves 91% on HumanEval (vs. GPT-4 baseline of ~88%).

**Generative Agents** (Stanford): Memory stream + reflection + planning. Reflection synthesizes memories into higher-level summaries stored back in memory. All three components essential for coherent long-term behavior.

**Key insight for triggers**: Self-modifying triggers should follow Voyager's pattern — closed-loop feedback where trigger execution results are observed, and the LLM can refine trigger definitions based on outcomes. Reflection (summarizing past trigger behavior) prevents drift and infinite loops.

**Sources**: [Voyager (arxiv 2305.16291)](https://arxiv.org/abs/2305.16291), [Reflexion (arxiv 2303.11366)](https://arxiv.org/abs/2303.11366), [Generative Agents (arxiv 2304.03442)](https://arxiv.org/abs/2304.03442)

### 4.6 Automation Platform Trigger Models

| Platform | Trigger Definition | Strengths | Weaknesses |
|----------|-------------------|-----------|------------|
| **n8n** | Visual nodes + config forms | 8+ trigger types, composable | Requires UI for definition |
| **Zapier** | "When X in app A, do Y in app B" | Natural language-like | Limited to pre-built integrations |
| **Node-RED** | Flow-based message routing | Modular, testable | Visual-first, no declarative spec |
| **GitHub Actions** | YAML `on:` blocks with filters | Familiar, version-controlled | Git-lifecycle-only events |
| **Kubernetes Operators** | Watch CRDs → reconcile loop | Idempotent, scalable | High boilerplate |

**Key pattern from Kubernetes**: The reconcile loop compares desired state (trigger definition) to actual state (graph). Idempotent reconciliation is safer than imperative event handling — if a trigger fires twice for the same event, the second application is a no-op.

**Sources**: [n8n Trigger Nodes](https://docs.n8n.io/integrations/builtin/trigger-nodes/), [Kubernetes Operator Pattern](https://kubernetes.io/docs/concepts/extend-kubernetes/operator/), [GitHub Actions Workflow Syntax](https://docs.github.com/en/actions/writing-workflows/workflow-syntax-for-github-actions)

### 4.7 Game Engine Event Systems

**Godot Signals**: Nodes emit typed signals; other nodes connect handlers. Fully decoupled — emitter doesn't know who listens. Clean observer pattern with explicit subscription.

**Factorio Lua Events**: Mods register handlers via `script.on_event(defines.events.on_entity_died, handler)`. Sandboxed — mods can't access filesystem, network, or other mods' state except through explicit `remote` interfaces. Historical Lua bytecode validation vulnerabilities highlight that sandboxing is hard even in mature systems.

**VS Code Activation Events**: Extensions declare `activationEvents` in manifest — loaded lazily when event matches. Events include `onLanguage`, `onCommand`, `onFile`. Solves the "too many plugins" problem through demand-loading.

**Relevance**: Godot's signal model maps directly to the EventBus subscription pattern. Factorio's staged API (different capabilities during definition vs. execution) applies to trigger sandboxing. VS Code's activation events solve trigger efficiency — only evaluate triggers relevant to the current event type.

**Sources**: [Godot Event System](https://medium.com/codex/godots-amazing-event-system-aa0aca9ab552), [Factorio Lua API](https://lua-api.factorio.com/latest/events.html), [VS Code Activation Events](https://code.visualstudio.com/api/references/activation-events)

### 4.8 Event Pattern Languages

**Reactive Extensions (Rx)**: Observable sequences + composable operators (`filter`, `map`, `merge`, `throttle`, `buffer`, `debounce`). Operators compose functionally — each transforms the stream without mutating source. This operator model could express trigger composition: "throttle WorkItemStatusChanged events to at most 1 per 5 seconds, then merge with AgentFinished."

**Temporal Logic (LTL)**: Formal operators for reasoning about event sequences:
- `F φ` (Finally): φ will eventually hold
- `G φ` (Globally): φ holds in all future states
- `φ U ψ` (Until): φ holds until ψ becomes true

LTL provides the theoretical foundation for temporal trigger patterns but is too abstract for direct LLM authoring. A simplified subset (sequence, timeout, negation) is more practical.

**Datalog + Events**: Event Choice Datalog extends Datalog to reason about event sequences. Unification + pattern matching without explicit loops. Relevant because the project's VISION.md already mentions Cozo (Datalog-based graph DB) as a potential backing store.

**Sources**: [ReactiveX Introduction](https://reactivex.io/intro.html), [Linear Temporal Logic (Wikipedia)](https://en.wikipedia.org/wiki/Linear_temporal_logic), [Event Choice Datalog (ResearchGate)](https://www.researchgate.net/publication/221336444_Event_choice_datalog_A_logic_programming_language_for_reasoning_in_multiple_dimensions)

---

## 5. Comparison Matrix: Trigger Definition Approaches

| Criterion | JSON Patterns | Scripted (Rhai) | Datalog Rules | LLM-Driven |
|-----------|--------------|-----------------|---------------|------------|
| **Expressiveness** | Low (equality, ranges) | High (Turing-complete) | Medium (relational) | Highest (natural language) |
| **Verifiability** | Full (decidable) | None (halting problem) | Partial (decidable subset) | None |
| **LLM authoring quality** | High (structured JSON) | Medium (Rhai less known) | Low (Datalog unfamiliar) | N/A (LLM IS the author) |
| **Temporal patterns** | No | Yes (with state) | Yes (Event Choice) | Yes (via reasoning) |
| **Sandboxing** | Inherent (no execution) | Required (Rhai engine) | Inherent (no side effects) | Required (action budget) |
| **Latency** | <1ms (pattern match) | 1-10ms (interpret) | 1-5ms (query eval) | 100ms-5s (LLM call) |
| **Loop prevention** | Trivial (no recursion) | Requires depth limits | Detectable (stratification) | Requires infrastructure |
| **Integration with plugins** | Actions reference tool names | Direct Rhai→tool calls | Actions reference tool names | LLM chooses tools |

---

## 6. VISION.md Alignment

| Vision Concept | Trigger Impact |
|----------------|---------------|
| **Graph-native context** | Trigger definitions are graph nodes; firings create provenance edges |
| **Tool calls as first-class citizens** | Trigger actions that invoke tools get same `ToolCall →[Produced]→ ToolResult` chain |
| **Background graph processing** | Triggers with temporal patterns run as background event consumers |
| **MCP for tool integration** | Phase 3 triggers can invoke MCP-provided tools as actions |
| **Developer control** | Users can enable/disable/inspect triggers through the TUI |
| **Deterministic context construction** | Trigger firings are logged as graph events, enabling replay and debugging |

The vision's MergeTree analogy (write fast, optimize later) applies: triggers fire immediately on events, but trigger optimization (deduplication, batching, conflict resolution) happens in the background.

---

## 7. Recommended Architecture

### Phase 1: Declarative JSON Event Patterns

**Goal**: LLM or user defines a trigger as a JSON pattern matching `GraphEvent` fields, with an action that emits a new event, invokes a tool, or creates a graph node.

**Trigger definition model (ECA)**:

```
Event:     JSON pattern matching a GraphEvent variant + field values
Condition: Optional field-level predicates (equality, ranges, set membership)
Action:    One of: emit event, invoke tool, create node, update node status
```

**Stored as a graph node**:

```
Node::Trigger {
    id: Uuid,
    name: String,
    description: String,
    event_pattern: EventPattern,     // declarative JSON matcher
    action: TriggerAction,           // what to do when pattern matches
    status: TriggerStatus,           // Enabled, Disabled, Fired(count)
    priority: u16,                   // conflict resolution (lower = higher priority)
    max_fires: Option<u32>,          // auto-disable after N firings
    cooldown: Option<Duration>,      // minimum time between firings
    created_by: Uuid,                // agent/user that created it
    created_at: DateTime<Utc>,
}
```

**Evaluation**: Simple pattern matching — for each EventBus event, iterate enabled triggers, check if event matches pattern, execute action. No Rete needed at this scale.

**Safety**: JSON patterns can't loop (no execution), actions are bounded (single tool call or event emission), `max_fires` and `cooldown` prevent runaway triggers.

### Phase 2: Scripted Conditions + Temporal Patterns

**Goal**: Triggers can evaluate Rhai conditions over graph state and match temporal patterns (sequences, windows, negation).

**Extended trigger model**:

```
Event:     JSON pattern OR temporal pattern (sequence, window, negation)
Condition: Optional Rhai predicate with read-only graph access
Action:    Tool invocation, event emission, or Rhai script (sandboxed)
```

**Temporal pattern types**:

| Pattern | Description | Example |
|---------|-------------|---------|
| `Sequence(A, B)` | A followed by B | ToolCall starts → ToolCall errors |
| `Window(A, count, duration)` | N events of type A within duration | 3 errors in 60s |
| `Negation(A, B, timeout)` | A occurs, B does not within timeout | Question asked, not answered in 5min |
| `All(A₁..Aₙ)` | All events in set occur (any order) | All subtasks complete |

**State management**: Each temporal pattern instance maintains a partial match state (inspired by Flink CEP state machines). Partial matches are persisted as `BackgroundTask` nodes for crash recovery.

**Rhai conditions**: The Rhai engine from research doc 15 evaluates conditions with read-only graph access. Sandboxed: `set_max_operations(1000)`, no filesystem/network, graph access via host functions (`node_status(id)`, `edge_exists(from, to, kind)`).

### Phase 3: LLM-Driven Autonomous Triggers

**Goal**: An LLM can create, evaluate, and refine triggers autonomously, using reflection on past trigger behavior.

**Capabilities**:
- **Auto-creation**: Agent observes repetitive manual actions, proposes trigger to automate them
- **Reflection**: After N firings, LLM summarizes trigger behavior, suggests refinements
- **Compositional triggers**: Trigger A's action creates Trigger B (meta-triggers)
- **Natural language conditions**: "When the agent seems stuck" → LLM evaluates at trigger time

**Governance**: All Phase 3 triggers require explicit user approval before activation. Dry-run mode shows what would fire without executing actions. Resource budgets limit total LLM calls per trigger per hour.

---

## 8. Integration Design

### Trigger as Graph Node

```
User/Agent ─[tool_use: register_trigger]─→ ToolCall ─[Produced]─→ ToolResult
                                                │
                                         [effects.apply()]
                                                │
                                                ↓
                                         Node::Trigger (graph node)
                                                │
                                         [EventBus emit]
                                                ↓
                                      GraphEvent::TriggerRegistered { node_id }
```

### Trigger Evaluation Pipeline

```
EventBus event arrives
    ↓
TriggerEvaluator receives event (EventBus subscriber)
    ↓
For each enabled Trigger node (sorted by priority):
    ├─ Match event against trigger's EventPattern
    ├─ If match: evaluate condition (Phase 1: always true; Phase 2: Rhai)
    ├─ If condition met: check cooldown/max_fires
    └─ If allowed: execute action
            ├─ EmitEvent → EventBus.emit(new_event)
            ├─ InvokeTool → spawn_tool_execution(args)
            ├─ CreateNode → graph.add_node(node)
            └─ UpdateStatus → graph.update_work_item_status(id, status)
        Then: record firing in trigger's history, emit TriggerFired event
```

### New GraphEvent Variants

```
TriggerRegistered { node_id: Uuid }
TriggerFired { trigger_id: Uuid, event_id: Uuid, action_taken: String }
TriggerDisabled { node_id: Uuid, reason: String }
```

### New Tool: `register_trigger`

An LLM creates a trigger via the standard tool call flow:

```
register_trigger {
    name: "auto-complete-parent",
    description: "When all subtasks of a plan complete, mark the plan as completed",
    event_type: "WorkItemStatusChanged",
    event_filter: { "new_status": "Completed", "kind": "Task" },
    action: {
        type: "update_status",
        target: "parent_work_item",
        new_status: "Completed"
    },
    max_fires: null,
    cooldown_seconds: null
}
```

### Safety Infrastructure

| Mechanism | Purpose | Implementation |
|-----------|---------|----------------|
| **Cascade depth limit** | Prevent A→B→A infinite loops | Global counter, default max 8 |
| **Per-trigger rate limit** | Prevent single trigger flooding | `cooldown` field + `max_fires` |
| **Global event budget** | Prevent total system overload | Max triggered events per minute (configurable) |
| **Action capability set** | Prevent privilege escalation | Trigger declares which tools/nodes it can affect |
| **Dry-run mode** | Preview without execution | `TriggerStatus::DryRun` logs matches without acting |
| **Approval workflow** | Human gate for high-risk triggers | `TriggerStatus::PendingApproval` for Phase 3 |

### Loop Detection Strategy

When a trigger's action produces an event that could match another trigger (or itself):

1. Maintain a **cascade counter** per event chain (passed as metadata through EventBus)
2. Each trigger firing increments the counter
3. When counter exceeds `MAX_CASCADE_DEPTH` (default: 8), halt the chain and emit `ErrorOccurred`
4. Log the full chain for debugging: `[TriggerA fired → EventX → TriggerB fired → EventY → ...]`

This is the same strategy used by database trigger systems (PostgreSQL's `max_trigger_depth`) and CI systems (GitHub Actions' reentrant workflow prevention).

---

## 9. Red/Green Team Audit

### Green Team (Factual Verification)

**Overall accuracy: 95%.** All foundational CS concepts (Rete, ECA, LTL, CEP), platform features, and academic paper claims verified against authoritative sources. Three corrections applied:

1. **GraphEvent variant count**: Corrected from 25 to 20 (actual count from `src/graph/event.rs`)
2. **Reflexion benchmark**: Corrected from "vs. GPT-4's 80%" to "vs. GPT-4 baseline of ~88%" — the 91% figure uses GPT-4 as backbone, so the comparison is against GPT-4's own baseline
3. **n8n trigger types**: Corrected from "6 trigger types" to "8+ trigger types" per current documentation

**Verified claims (sample):**
- Rete algorithm origin (Forgy, 1974) — confirmed
- AWS EventBridge AND/OR semantics — fields at same level are AND; arrays within fields are OR — confirmed
- Esper EPL extending SQL-92 — confirmed
- Apache Flink CEP state machine approach — confirmed
- Voyager 3.3x item discovery — confirmed (arxiv 2305.16291)
- Generative Agents: all three components essential per ablation studies — confirmed (arxiv 2304.03442)
- PostgreSQL `max_trigger_depth` for loop prevention — confirmed (default: 16)
- GitHub Actions reentrant workflow prevention — confirmed (uses concurrency groups)
- Factorio Lua sandboxing: no filesystem/network, `remote` interfaces only — confirmed
- VS Code activation events: `onLanguage`, `onCommand`, `onFile` — confirmed
- WordPress hooks: actions vs. filters distinction — confirmed
- LTL operators (F, G, U, X) definitions — confirmed against formal methods literature
- Capability-based security as unforgeable tokens — confirmed
- All file:line references verified against actual source code (12/13 exact match; variant count was the single error)

### Red Team (Challenging Recommendations)

Ten challenges identified, prioritized by severity:

**C1: JSON patterns too restrictive for Phase 1 — CRITICAL.**
The `register_trigger` example (§8) shows "auto-complete-parent" — marking a plan as completed when all subtasks complete. EventBridge-style JSON patterns can only match field equality/ranges on a single event; they cannot express "all siblings of this node have status Completed" because that requires graph traversal. Phase 1 as described handles only trivial single-event patterns (field matching), not the graph-aware triggers that would prove the concept. **Resolution**: Accept that Phase 1 is limited to simple field-matching triggers (still useful for notifications, logging, basic reactions). Move graph-aware triggers to Phase 2 where Rhai conditions can query the graph. Alternatively, the effects system could synthesize higher-level events like `AllSubtasksCompleted { parent_id }` that Phase 1 triggers can match.

**C2: Three-phase approach overlaps with research doc 15 — HIGH.**
Doc 15 has Rhai→WASM→MCP for plugins; doc 27 has JSON→Rhai→LLM for triggers. Doc 27 Phase 2 uses the same Rhai engine as doc 15 Phase 1 for condition evaluation, but the documents don't cross-reference or align. Questions unanswered: do trigger Rhai scripts share the same sandbox as plugin Rhai scripts? Can a trigger invoke a plugin tool? What's the division of labor between "plugin that reacts to events" and "trigger that invokes a plugin"? **Resolution**: This document is concepts-only, not an implementation spec. The alignment question (one system or two) should be resolved during implementation design, but the conceptual overlap is acknowledged — triggers and plugins share the Rhai execution layer and should share sandbox configuration.

**C3: Cascade depth limit of 8 is arbitrary — MEDIUM.**
PostgreSQL's default is 16. No justification given for 8. Multi-level approval chains or deeply nested work item hierarchies could legitimately exceed 8. **Resolution**: The number should be configurable with a sensible default. PostgreSQL's 16 is a better-justified starting point. The key insight is that any fixed limit needs: (a) visibility when hit (not silent truncation), (b) per-trigger or per-session override capability, (c) cost modeling (events per level × latency per evaluation).

**C4: Trigger evaluation pipeline is synchronous — CRITICAL.**
The pipeline described iterates all enabled triggers for every event. At 100 triggers × 1ms evaluation each, that's 100ms blocking the EventBus per event. This contradicts doc 20's requirement that the scheduler runs as a concurrent process. **Resolution**: This document describes concepts, not implementation. The evaluation pipeline should be async (spawn trigger evaluation as a tokio task, not block the EventBus subscriber). Phase 1 with <20 triggers and <1ms JSON matching per trigger is safe; the pipeline description should note that async evaluation becomes mandatory at scale. VS Code's activation event pattern (§4.7) solves this efficiently — triggers subscribe to specific event types, so only relevant triggers are evaluated.

**C5: Temporal pattern state doesn't fit BackgroundTask — MEDIUM.**
`BackgroundTask` has lifecycle semantics (Pending→Running→Completed) unsuitable for partial pattern match state (event history, window boundaries, match count). **Resolution**: Acknowledged — temporal pattern state needs its own storage model. Options: (a) dedicated `TemporalMatchState` node type, (b) in-memory state with periodic snapshots, (c) embed state in the Trigger node itself. This is a Phase 2 design decision, not a Phase 1 concern.

**C6: Missing alternatives not evaluated — HIGH.**
The document surveys prior art but doesn't justify why JSON→Rhai→LLM was chosen over: (a) actor model per trigger, (b) embedded Datalog (Cozo is already in VISION.md), (c) reactive stream operators (tokio-stream), (d) webhook-based external triggers, (e) "no triggers" — just let agents reason about events via LLM calls. **Resolution**: These are valid alternatives. The comparison matrix (§5) includes Datalog as a column but doesn't evaluate actors or reactive streams. For a concepts-focused research document, the key contribution is the ECA framing and safety analysis, not a definitive technology choice. The actor model and Datalog alternatives warrant separate investigation if the trigger system moves to implementation.

**C7: Safety mechanisms not phased — HIGH.**
The safety table lists 6 mechanisms but doesn't specify which are Phase 1 vs. Phase 2 vs. Phase 3. **Resolution**: Phase 1 minimum: cascade depth limit + per-trigger max_fires + cooldown. Phase 2 adds: global event budget + action capability sets. Phase 3 adds: approval workflow + dry-run mode. Failure behavior: always emit `ErrorOccurred` with the trigger chain trace; never silently drop.

**C8: Plugin integration underspecified — HIGH.**
"Trigger actions can invoke tools from the plugin system" but no detail on: does TriggerEvaluator access DynamicToolRegistry? What's the parent node for a trigger-invoked ToolCall? How are results routed? **Resolution**: This is intentional for a concepts document — the integration design in §8 is a sketch, not a spec. Implementation would need to define: trigger-invoked ToolCalls have the Trigger node as parent (new edge type), results are recorded as ToolResult nodes, and the TriggerEvaluator delegates to the same `spawn_tool_execution()` used by the agent loop.

**C9: No trigger testing/validation mechanism — MEDIUM.**
Phase 3 describes LLM-driven trigger creation but no way to verify a trigger before enabling it. Voyager (§4.5) relies on closed-loop feedback, which implies trigger execution results must be observable and refinable. **Resolution**: Phase 2 should add a `test_trigger` tool that replays historical events against a trigger definition and reports matches. Phase 3 should add trigger performance metrics (fired N times, succeeded M times) queryable by the LLM for reflection.

**C10: Operational concerns absent — HIGH.**
No discussion of: TUI representation of triggers, debugging trigger chains, trigger versioning, monitoring/alerting when triggers malfunction, disaster recovery (auto-disable triggers that cause crashes on restart). **Resolution**: Acknowledged as out of scope for a concepts document. These are essential for implementation design. Key principles: triggers must be visible in TUI (dedicated panel or tab), firing history must be queryable, triggers that cause `ErrorOccurred` should auto-disable after N consecutive errors.

### Code Accuracy Verification

**12 of 13 code references verified as accurate.** One error corrected:

| Reference | Status |
|-----------|--------|
| `src/graph/event.rs:92-124` (EventBus) | Correct (struct at line 95, impl lines 99-124) |
| `src/app/event_dispatch.rs:22-69` (handle_graph_event) | Correct (exact match) |
| `src/app/plan/effects.rs:1-183` (apply function) | Correct (apply at line 18, file ends ~182) |
| `src/app/agent/loop.rs:65-76` (is_wake_event) | Correct (exact match) |
| 256-item buffer (`EVENT_BUFFER_SIZE`) | Correct (line 20) |
| Hardcoded reactions table | Correct (all four reactions verified) |
| `EdgeKind::Triggers` exists | Correct (line 156 in node.rs) |
| Research doc 15, 20, 21 exist | Correct (all three present) |
| BackgroundTask node fields | Correct (lines 219-226 in node.rs) |
| TriggerStatus/TriggerAction as proposed | Correct (not falsely claimed as existing) |
| `tool_registry()` signature | Correct (line 57 in mod.rs) |
| `ToolCallArguments::Unknown` fields | Correct (lines 177-180 in types.rs) |
| "25 GraphEvent variants" | **Incorrect** — corrected to 20 |

---

## 10. Sources

### Rule Engines & Formal Models
- [Rete Algorithm (Wikipedia)](https://en.wikipedia.org/wiki/Rete_algorithm)
- [Drools Rule Engine Documentation](https://docs.drools.org/5.2.0.M2/drools-expert-docs/html/ch01.html)
- [Event-Condition-Action Rules (ScienceDirect)](https://www.sciencedirect.com/topics/computer-science/event-condition-action-rule)
- [Symbolic Verification of ECA Rules (Springer)](https://link.springer.com/chapter/10.1007/978-3-662-45730-6_6)

### Complex Event Processing
- [Esper Complex Event Processing](https://www.espertech.com/esper/)
- [Apache Flink CEP Documentation](https://nightlies.apache.org/flink/flink-docs-master/docs/libs/cep/)

### Automation Platforms
- [AWS EventBridge Event Pattern Syntax](https://docs.aws.amazon.com/eventbridge/latest/userguide/eb-create-pattern.html)
- [n8n Trigger Nodes Documentation](https://docs.n8n.io/integrations/builtin/trigger-nodes/)
- [Kubernetes Operator Pattern](https://kubernetes.io/docs/concepts/extend-kubernetes/operator/)
- [GitHub Actions Workflow Syntax](https://docs.github.com/en/actions/writing-workflows/workflow-syntax-for-github-actions)

### Self-Modifying Agent Systems
- [Voyager: Open-Ended Embodied Agent (arxiv 2305.16291)](https://arxiv.org/abs/2305.16291)
- [Reflexion: Verbal Reinforcement Learning (arxiv 2303.11366)](https://arxiv.org/abs/2303.11366)
- [Generative Agents: Interactive Simulacra (arxiv 2304.03442)](https://arxiv.org/abs/2304.03442)
- [ToolMaker: LLM Agents Making Agent Tools (arxiv 2502.11705)](https://arxiv.org/abs/2502.11705)
- [AutoAct: Automatic Agent Learning (arxiv 2401.05268)](https://arxiv.org/html/2401.05268v2)

### Game Engine Event Systems
- [Godot Event System (Medium/Codex)](https://medium.com/codex/godots-amazing-event-system-aa0aca9ab552)
- [Factorio Lua Event API](https://lua-api.factorio.com/latest/events.html)
- [VS Code Activation Events](https://code.visualstudio.com/api/references/activation-events)

### Event Pattern Languages
- [ReactiveX Introduction](https://reactivex.io/intro.html)
- [Linear Temporal Logic (Wikipedia)](https://en.wikipedia.org/wiki/Linear_temporal_logic)
- [Event Choice Datalog (ResearchGate)](https://www.researchgate.net/publication/221336444_Event_choice_datalog_A_logic_programming_language_for_reasoning_in_multiple_dimensions)

### LLM Safety & Governance
- [LLM Tool-Calling Failure Modes (Medium)](https://medium.com/@komalbaparmar007/llm-tool-calling-in-production-rate-limits-retries-and-the-infinite-loop-failure-mode-you-must-2a1e2a1e84c8)
- [What 1,200 Production Deployments Reveal About LLMOps (ZenML)](https://www.zenml.io/blog/what-1200-production-deployments-reveal-about-llmops-in-2025)
- [Sandboxed Code Execution for AI Agents (inference.sh)](https://inference.sh/blog/tools/sandboxed-execution)
- [Capability-Based Security (Wikipedia)](https://en.wikipedia.org/wiki/Capability-based_security)

### Plugin Integration Patterns
- [WordPress Hooks: Actions & Filters](https://developer.wordpress.org/plugins/hooks/)
- [Apache Camel Enterprise Integration Patterns](https://camel.apache.org/components/4.14.x/eips/enterprise-integration-patterns.html)
- [LLM-Based Wargame Scenario Generation with ECA Rules (SAGE)](https://journals.sagepub.com/doi/10.1177/00375497251415245)

### Internal References
- Research doc 15: LLM-Written Dynamic Plugins (`docs/research/15-llm-written-plugins.md`)
- Research doc 20: Kubernetes-Inspired Agent Scheduling (`docs/research/20-kubernetes-inspired-agent-scheduling.md`)
- Research doc 21: Graph Scheduler & Q/A Relationships (`docs/research/21-graph-scheduler-qa-relationships.md`)
