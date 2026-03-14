# Research: LLM Question Surfacing via Tool Calls

> 2026-03-14 | How LLMs surface questions to users/agents, with destination routing and graph-based answer dependencies.

---

## 1. Executive Summary

LLMs need to ask questions mid-execution — clarifications, approvals, design decisions. The industry has converged on **tool-as-question** as the dominant pattern: the LLM calls a tool (e.g., `ask`), and the runtime routes it. This research investigates how to implement this in the context-orchestrator's graph-based architecture, where a tool argument specifies the **destination** (user, LLM, etc.) and the graph encodes **dependencies on the answer** so downstream nodes block until it arrives.

**Recommendation:** Option B — `Question` as a first-class graph node with `DependsOn` edges. A single `ask` tool with a typed `Destination` argument and optional pre-specified options. The `Question` node captures metadata (destination, options, status), the answer flows through the existing `ToolResult` pipeline, and `DependsOn` edges let any node declare it blocks on an answer. The red team argued DependsOn edges have no runtime consumer, but this is neutralized: a scheduler/dependency resolver is being built in parallel and will consume these edges. Routing uses a match statement with helper functions (promoting to a trait at Phase 3 scale).

---

## 2. Current Architecture & Gap Analysis

### What exists today

| Component | Location | Relevant Detail |
|-----------|----------|-----------------|
| **Graph** | `src/graph/mod.rs:24-37` | `ConversationGraph` with nodes, edges, branch tracking, version history |
| **Node types** | `src/graph/node.rs:122-192` | Message, ToolCall, ToolResult, ThinkBlock, WorkItem, etc. |
| **Edge types** | `src/graph/node.rs:97-107` | RespondsTo, Invoked, Produced, ThinkingOf, RelevantTo, SubtaskOf, Tracks, Indexes, Provides |
| **Tool registry** | `src/tool_executor/mod.rs:44-48` | Single registry; tools equally callable by users and LLM |
| **Tool execution** | `src/tool_executor/mod.rs:271-298` | `execute_tool()` matches `ToolCallArguments` variants |
| **Tool args** | `src/graph/tool_types.rs:15-46` | Typed enum: Plan, ReadFile, WriteFile, ListDirectory, SearchFiles, WebSearch, Set, Unknown |
| **Agent loop** | `src/app/agent_loop.rs:58-177` | Build context -> stream LLM -> apply to graph -> dispatch tools -> loop |
| **Tool dispatch** | `src/app/agent_loop.rs:271-321` | Adds ToolCall nodes, waits for results via channel with 60s timeout |
| **Side-effects** | `src/app/task_handler.rs:255-326` | Post-completion handlers for specific tools (Set, Plan) |

### Gaps

1. **No question/answer primitive.** There is no way for the LLM (or a tool) to pause execution and ask a question routed to any destination.
2. **No answer dependency.** The graph has no concept of "node X cannot execute until node Y has an answer." Edges express relationships (RespondsTo, SubtaskOf) but not blocking dependencies.
3. **No destination routing.** Tool results always come from local execution. There is no mechanism to route a request to different backends (user terminal, another LLM, external service).
4. **Fixed 60s timeout.** Tool dispatch waits 60 seconds (`src/app/agent_loop.rs:332` in `wait_for_tool_results`). User questions may take much longer.

---

## 3. Requirements

Derived from the user's request, VISION.md, and CLAUDE.md rules:

| # | Requirement | Source |
|---|-------------|--------|
| R1 | Questions are surfaced via tool calls, not special-cased | User request + `feedback_no_distinctions.md` |
| R2 | A tool argument specifies the desired destination (user, LLM, etc.) | User request |
| R3 | The graph encodes dependencies on answers — downstream work blocks until answered | User request |
| R4 | Multiple possible answers can be pre-specified (LLM suggests options) | User request ("possible answers") |
| R5 | The mechanism must be equally callable by users and LLM | `feedback_no_distinctions.md` + tool registry design |
| R6 | No dead code, typed structs, idiomatic Rust | CLAUDE.md |
| R7 | Graph-native — questions/answers are first-class nodes with typed edges | VISION.md Section 3.1 |
| R8 | The answer routing is extensible to future destinations (Slack, webhook, database) | VISION.md Section 5.2 (trait abstraction) |

---

## 4. Options Analysis

### Option A: Simple `ask` Tool (Tool-as-Question)

The LLM calls an `ask` tool. The runtime blocks, collects the answer, returns it as a `tool_result`. No new node types; the question is a ToolCall and the answer is a ToolResult.

**Strengths:**
- Minimal change — fits existing ToolCall/ToolResult pipeline exactly
- Works today with the agent loop's tool dispatch + channel pattern
- How Claude Code, LangChain, and Roo Code all do it

**Weaknesses:**
- No graph-level dependency tracking — the "dependency" is implicit in the agent loop blocking on the tool result
- Cannot express "this future node depends on this answer" in the graph
- Other graph nodes cannot declare they depend on the answer

**Red team rebuttal (strong):** Option A can carry `options` and `schema` fields in the `Ask` variant trivially — structured answers are not exclusive to Option B. The ToolCall already records the full question, and ToolResult records the answer. This is how every major production framework (Claude Code, LangChain, Anthropic Agent SDK) implements it.

### Option B: Question/Answer as First-Class Graph Nodes

New `Question` and `Answer` node types. A `Question` node has a `destination` field and optional `options` (suggested answers). An `Answer` node links to its `Question` via a new edge. A new `DependsOn` edge kind lets any node declare it blocks on an answer.

**Strengths:**
- Graph-native: questions and answers are queryable, versionable, compactable
- Dependency tracking is explicit — graph algorithms can detect blocked subgraphs
- Aligns with VISION.md's "everything is a node" philosophy
- Enables future features: answer caching, answer routing analytics, answer-dependent branching
- The `Question` node persists even after the conversation ends — useful for audit/provenance

**Weaknesses:**
- Adds new node types and edge types — schema evolution
- The tool execution pipeline needs to understand that some tools produce `Question` nodes that don't immediately resolve
- More complex than Option A

**Red team rebuttal (strong):** The Question node duplicates every field already in the ToolCall (question text, destination, options, schema, timestamps). The WorkItem precedent is misleading — WorkItem has an independent lifecycle (Todo -> Active -> Done across conversations); Question does not. `DependsOn` edges have no runtime consumer: the agent loop has no scheduler, no dependency resolver. The edge would exist in the graph but never be checked. Option B is ~5x the surface area of Option A for speculative benefits.

### Option C: Graph Interrupt (LangGraph-style)

Execution suspends at a checkpoint. The full graph state is persisted. An external event (user input) resumes execution. The "node replays from the beginning" pattern.

**Strengths:**
- Battle-tested in LangGraph (checkpoint + resume)
- Handles arbitrarily long waits (hours, days)
- Clean separation between "pause" and "resume"

**Weaknesses:**
- Requires a checkpointing system the codebase doesn't have
- Node replay requires idempotent pre-interrupt code — significant constraint
- Doesn't fit the current agent loop architecture (streaming, channel-based)
- Over-engineered for the current system; LangGraph needs this because its nodes are arbitrary Python functions, but here tools are already isolated units

### Option D: Deferred Execution (PydanticAI-style)

The agent loop terminates early when it encounters a question, returning a "pending questions" collection. The caller resolves questions externally, then calls the agent loop again with answers + conversation history.

**Strengths:**
- Stateless — no long-lived process while waiting
- Clean functional pattern

**Weaknesses:**
- Breaks the current streaming loop model
- Forces the agent loop to be re-entrant in a way it isn't designed for
- Loses the "graph as single source of truth" property — pending state lives outside the graph

---

## 5. Comparison Matrix

| Criterion | A: Simple ask | B: Graph Nodes | C: Interrupt | D: Deferred |
|-----------|:---:|:---:|:---:|:---:|
| Graph-native | Via ToolCall/Result | First-class nodes | Partial | No |
| Dependency tracking | Implicit (blocking) | Explicit (edges) | Implicit | None |
| Destination routing | **Easy** | **Easy** | Hard | Medium |
| Implementation effort | **~80 lines, 3 files** | ~400 lines, 7 files | High | Medium |
| Fits existing pipeline | **Yes (no changes)** | Yes (extends) | No (overhaul) | No (overhaul) |
| Long wait support | Needs timeout tweak | Needs timeout tweak | Yes | Yes |
| Answer persistence/audit | **Via ToolCall+Result** | Dedicated nodes | Via checkpoint | Via history |
| Structured answers | **Yes** (via Ask fields) | **Yes** | Yes | Yes |
| Pre-specified options | **Yes** (via Ask fields) | **Yes** | N/A | N/A |
| VISION.md alignment | Medium (tool-as-citizen) | High (everything-is-node) | Medium | Low |
| Upgrade path to B | **Clean** | N/A | N/A | N/A |

---

## 6. VISION.md Alignment

**Option B (Graph Nodes) aligns strongly with:**

- **Section 3.1 (Graph Model):** "Every message, tool call, requirement, and work item is a node." Questions and answers should be nodes too. The vision explicitly lists `ToolCall` and `ToolResult` as node types — a `Question` is a generalization of "a request that needs an external response."

- **Section 4.8 (Tool Calls as First-Class Graph Citizens):** "Every tool invocation is recorded as a ToolCall node with ToolResult child nodes." Questions extend this — they're a special kind of tool invocation where the result comes from a routable destination.

- **Section 3.2 (Context Construction):** Graph traversal for LLM input construction. If questions/answers are nodes, they can be included in or excluded from context based on relevance — e.g., "this question was about authentication" gets included when the LLM is working on auth.

- **Section 4.3 (Background Processing):** Unanswered questions naturally surface in graph analysis — "these 3 questions are blocking progress on this work item."

**Deviation:** VISION.md doesn't mention question/answer as a pattern, but the graph model was designed to be extensible to new node types. This is a natural extension.

---

## 7. Recommended Architecture

Option B — first-class `Question` nodes with `DependsOn` edges. The red team's core objection ("DependsOn has no runtime consumer") is neutralized: a scheduler/dependency resolver is being built in parallel. The remaining red team feedback is incorporated: routing uses a match statement (not a trait), timeout is configurable (not infinite), and failure modes are addressed.

### Phase 1: `ask` Tool + Question Nodes + DependsOn Edges

**New `ToolCallArguments` variant:**

```rust
// In ToolCallArguments (src/graph/tool_types.rs)
Ask {
    question: String,
    destination: String,            // "user", "llm", "auto" — parsed at validation boundary
    options: Option<Vec<String>>,   // suggested answers
}
```

`destination` is a `String` (not an enum) in the deserialized form because the LLM sends it as JSON. Validation happens in `execute_tool`, following the `ConfigKey` pattern (`src/tool_executor/mod.rs:253-268`). Unknown destinations produce a tool error, not a silent fallback.

**Parsed destination enum (internal, not serialized):**

```rust
/// Where a question should be routed. Parsed from the string at the validation boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Destination {
    User,   // route to the human at the terminal
    Llm,    // route to another LLM call
    Auto,   // runtime decides based on session mode
}
```

**New node type:**

```rust
// In Node enum (src/graph/node.rs)
Question {
    id: Uuid,
    question: String,
    destination: Destination,
    options: Option<Vec<String>>,
    status: QuestionStatus,        // Pending, Answered, Declined, TimedOut
    created_at: DateTime<Utc>,
    answered_at: Option<DateTime<Utc>>,
}
```

The red team argued this duplicates ToolCall fields. **Rebuttal:** Question has a distinct role in the graph — it is the node that `DependsOn` edges point to. ToolCall is ephemeral (one conversation turn); Question persists as a dependency target across the graph. The scheduler needs to query "all unanswered questions blocking node X" — that's a `Question` node query, not a "ToolCall with Ask arguments" query. The WorkItem precedent is apt: WorkItem also starts from a tool call but has independent graph significance.

**New edge kinds:**

```rust
// In EdgeKind (src/graph/node.rs)
Asks,       // ToolCall --Asks--> Question
DependsOn,  // Node --DependsOn--> Question (blocks until answered)
```

**Execution flow:**

1. LLM calls `ask` tool with question + destination + options
2. `parse_tool_arguments("ask", raw_json)` -> `ToolCallArguments::Ask { .. }`
3. Side-effect in `task_handler.rs` (on dispatch, not completion): create `Question` node, add `Asks` edge from ToolCall
4. `execute_tool(Ask { .. })` parses destination, routes via match:
   - `"user"` -> prompt in TUI input field, configurable timeout (default: 5 min)
   - `"llm"` -> one-shot LLM call with the question as prompt
   - `"auto"` -> check session interactivity, delegate to user or llm
5. On completion: side-effect updates `Question` status to `Answered` (or `TimedOut`/`Declined`)
6. Answer returns as `ToolResultContent` through existing channel
7. LLM receives answer as normal tool result, continues
8. Scheduler/dependency resolver can now query `DependsOn` edges to detect unblocked nodes

**Routing is a match statement (not a trait):**

```rust
// In execute_tool (src/tool_executor/mod.rs)
ToolCallArguments::Ask { question, destination, options } => {
    match parse_destination(destination) {
        Ok(Destination::User) => prompt_user(question, options).await,
        Ok(Destination::Llm) => ask_llm(question, options).await,
        Ok(Destination::Auto) => {
            if is_interactive() { prompt_user(question, options).await }
            else { ask_llm(question, options).await }
        }
        Err(msg) => ToolExecutionResult { content: ToolResultContent::text(msg), is_error: true },
    }
}
```

A `QuestionRouter` trait only pays off at Phase 2 scale (5+ destinations, plugin system). For 3 destinations, helper functions keep the match arms clean.

**Timeout handling:**

The 60s timeout in `wait_for_tool_results` (`src/app/agent_loop.rs:332`) needs adjustment. When any pending tool call has `ToolCallArguments::Ask` with user destination, extend the deadline to the configured question timeout (default 5 min). On timeout, `Question` status set to `TimedOut`, tool returns error.

**Failure modes addressed:**

| Scenario | Mitigation |
|----------|------------|
| User walks away | 5-minute configurable timeout, Question status -> TimedOut |
| LLM asks too many questions | System prompt guidance; configurable `max_questions_per_turn` |
| Concurrent user questions in same batch | Serialize: prompt one at a time via queue in TUI input handler |
| LLM routing fails (rate limit, etc.) | Return tool error; Question status -> Declined |
| Question becomes stale during wait | Acceptable: LLM re-evaluates context on next iteration |

### Phase 2: Extended Destinations

Add more destination variants as needed:
- `Agent { agent_id }` — route to a specific sub-agent
- `External { webhook_url }` — route to a webhook, wait for callback
- `Cache { lookup_key }` — check a cache of previously answered questions

At this scale, promote the match statement to a `QuestionRouter` trait with pluggable implementations.

---

## 8. Integration Design

### Data Flow

```
LLM calls ask(question, destination, options)
    |
    v
parse_tool_arguments("ask", raw_json) -> ToolCallArguments::Ask { .. }
    |
    v
handle_tool_call_dispatched (task_handler.rs)
    +---> Create Question node (status: Pending)
    +---> Add Asks edge: ToolCall --Asks--> Question
    |
    v
execute_tool(Ask { .. })
    |
    +---> parse_destination(destination) -> Destination enum
    +---> match Destination:
    |         User -> prompt_user(question, options)  [TUI input, 5min timeout]
    |         Llm  -> ask_llm(question, options)      [one-shot LLM call]
    |         Auto -> check interactivity, delegate
    |
    v
TaskMessage::ToolCallCompleted { tool_call_id, content, is_error }
    |
    v
handle_tool_call_completed (task_handler.rs)
    +---> Side-effect: update Question status (Answered / TimedOut / Declined)
    +---> Add ToolResult node + Produced edge (existing pipeline)
    |
    v
Agent loop receives result, continues
Scheduler can query: DependsOn edges -> unanswered Questions -> blocked nodes
```

### Key Files to Modify

| File | Change |
|------|--------|
| `src/graph/node.rs` | Add `Question` node variant, `QuestionStatus` enum, `Destination` enum; add `Asks` and `DependsOn` to `EdgeKind` |
| `src/graph/tool_types.rs` | Add `Ask` variant to `ToolCallArguments`, add `"ask" => "Ask"` to `parse_tool_arguments` |
| `src/tool_executor/mod.rs` | Add `ask` to registry via `entry()`, add match arm + helper functions (`prompt_user`, `ask_llm`, `parse_destination`) in `execute_tool` |
| `src/app/task_handler.rs` | Add dispatch side-effect (create Question node + Asks edge); add completion side-effect (update Question status) |
| `src/app/agent_loop.rs` | Adjust timeout in `wait_for_tool_results` — extend deadline when pending tools include `Ask` with user destination |

### Reusable Patterns

- **Side-effect in task_handler:** The `Plan` tool creates a `WorkItem` node on completion (`src/app/task_handler.rs:302-313`). `Ask` follows the same pattern — create `Question` node on dispatch, update status on completion.
- **Tool registry pattern:** `entry()` + `prop()` helpers in `src/tool_executor/mod.rs:169-196`.
- **ConfigKey validation boundary:** `destination` parsed from string to enum at the tool execution boundary, following `ConfigKey` (`src/tool_executor/mod.rs:253-268`).
- **Cancellation:** `spawn_tool_execution` supports `CancellationToken` (`src/tool_executor/mod.rs:382-401`). User-destined questions use the same pathway — user cancels via TUI, token fires, Question status -> TimedOut.

---

## 9. Red/Green Team Audit Results

### Green Team (Factual Validation)

**7 VERIFIED, 2 PARTIALLY VERIFIED, 2 ISSUES FOUND:**

- LangGraph `interrupt()` + `Command(resume=)` — **VERIFIED** exactly as described
- MCP Elicitation `elicitation/create` with accept/decline/cancel, 2025-06-18 spec — **VERIFIED**
- PydanticAI deferred tools with `requires_approval` — **VERIFIED**
- rs-graph-llm `ExecutionStatus::WaitingForInput` — **VERIFIED**
- All 5 arxiv papers exist with broadly accurate descriptions — **VERIFIED**
- **INCORRECT: Spring AI `AskUserQuestionTool`** — Blog URL returns 404. No evidence in official Spring AI docs. Claim removed from sources.
- **PARTIALLY INCORRECT: AutoGen/AG2 conflation** — `ALWAYS/TERMINATE/NEVER` modes belong to AG2's `ConversableAgent`, not AutoGen's `UserProxyAgent`. Sources corrected.
- **PARTIALLY INCORRECT: Agentic AI Taxonomies paper** — HITL appears as a safety mechanism within AutoGen discussion, not as a standalone taxonomy category. Description softened.

### Red Team (Challenges)

The red team's strongest arguments and their resolution:

1. **"Option A is sufficient"** — Valid for a system without dependency resolution. **Neutralized:** scheduler/dependency resolver is being built in parallel, so DependsOn edges will have a runtime consumer. Question nodes are the dependency target.

2. **"Question node is data duplication"** — Partially valid: fields overlap with ToolCall. **Accepted with rebuttal:** Question has distinct graph significance as a dependency target. The scheduler queries "unanswered Questions blocking node X" — that's a node-type query, not "ToolCalls with Ask arguments." Same justification as WorkItem.

3. **"DependsOn edge has no runtime consumer"** — Was the strongest argument. **Neutralized:** the dependency resolver being built concurrently IS the consumer.

4. **"QuestionRouter trait is premature"** — **Accepted.** Match statement with helper functions for Phase 1. Promote to trait at Phase 2 scale (5+ destinations).

5. **"No-timeout is a liveness hazard"** — **Accepted.** 5-minute configurable timeout with cancellation UI. Question status -> TimedOut on expiry.

6. **"Missing failure modes"** — **Accepted.** All addressed in the failure modes table: question fatigue, concurrent input, LLM fallback, stale context.

### Code Accuracy

**10/11 references ACCURATE.** 1 correction: 60s timeout is at `agent_loop.rs:332` (in `wait_for_tool_results`), not 313-321. All types, functions, fields, and architectural claims verified against actual code.

---

## 10. Sources

### Industry Implementations
- [LangChain HumanInputRun](https://api.python.langchain.com/en/latest/tools/langchain_community.tools.human.tool.HumanInputRun.html) — Tool-as-question with overridable `input_func`
- [LangGraph Interrupts](https://docs.langchain.com/oss/python/langgraph/interrupts) — `interrupt()` + `Command(resume=)` with checkpoint persistence
- [LangGraph interrupt() Blog](https://blog.langchain.com/making-it-easier-to-build-human-in-the-loop-agents-with-interrupt/) — Evolution from breakpoints to fine-grained interrupts
- [AutoGen HITL Tutorial](https://microsoft.github.io/autogen/stable//user-guide/agentchat-user-guide/tutorial/human-in-the-loop.html) — UserProxyAgent + HandoffTermination patterns
- [AG2 HITL Documentation](https://docs.ag2.ai/latest/docs/user-guide/basic-concepts/human-in-the-loop/) — ConversableAgent `human_input_mode` (ALWAYS/TERMINATE/NEVER)
- [CrewAI Human Input](https://docs.crewai.com/en/learn/human-input-on-execution) — `human_input=True` on tasks (terminal-only)
- [Semantic Kernel HITL](https://learn.microsoft.com/en-us/semantic-kernel/frameworks/process/examples/example-human-in-loop) — Parameter-gating with external events
- [Anthropic Tool Use](https://platform.claude.com/docs/en/agents-and-tools/tool-use/implement-tool-use) — ask_user as a regular tool in the agent loop
- [Claude Agent SDK](https://platform.claude.com/docs/en/agent-sdk/agent-loop) — Built-in AskUserQuestion in orchestration category
- [Temporal HITL AI](https://docs.temporal.io/ai-cookbook/human-in-the-loop-python) — Signal-based approval with durable wait
- [PydanticAI Deferred Tools](https://ai.pydantic.dev/deferred-tools/) — Collect pending approvals, resume with results

### Rust Frameworks
- [rs-graph-llm](https://github.com/a-agmon/rs-graph-llm) — `ExecutionStatus::WaitingForInput` + session-based pause/resume

### Protocols
- [MCP Elicitation Spec](https://modelcontextprotocol.io/specification/draft/client/elicitation) — `elicitation/create` with JSON schema + accept/decline/cancel
- [MCP Elicitation Analysis](https://lord.technology/2025/10/13/the-deliberate-constraints-of-mcp-elicitation.html) — Flat schema constraints rationale
- [OpenAI Agents SDK Handoffs](https://openai.github.io/openai-agents-python/handoffs/) — Agent-level destination routing

### Academic Papers
- [HULA: Human-In-the-Loop Software Agents](https://arxiv.org/abs/2411.12924) — Structured checkpoints at plan/code stages
- [ARIA: Self-Improving Agents with HITL](https://arxiv.org/abs/2507.17131) — Selective queries when uncertain outperform always/never-ask
- [Agentic AI Taxonomies](https://arxiv.org/html/2601.12560v1) — HITL discussed as safety mechanism within multi-agent systems
- [Multi-Agent Routing for QA](https://arxiv.org/html/2501.07813v1) — Planning-based routing to specialist agents
- [Agent-in-the-Loop Data Flywheel](https://arxiv.org/abs/2510.06674) — Human corrections as iterative improvement

### Codebase References
- `src/graph/node.rs:122-192` — Node enum (Add `Question` variant here)
- `src/graph/node.rs:97-107` — EdgeKind enum (Add `Asks`, `DependsOn` here)
- `src/graph/tool_types.rs:15-46` — ToolCallArguments enum (Add `Ask` variant here)
- `src/graph/tool_types.rs:216-245` — `parse_tool_arguments` (Add `"ask" => "Ask"` mapping)
- `src/tool_executor/mod.rs:44-48` — Tool registry (Register `ask` tool here)
- `src/tool_executor/mod.rs:271-298` — `execute_tool` match (Add `Ask` handler with destination routing)
- `src/tool_executor/mod.rs:253-268` — `ConfigKey` pattern (Reuse for `Destination` string-to-enum parsing)
- `src/app/task_handler.rs:302-313` — Plan side-effect pattern (Reuse for `Ask`: create Question on dispatch, update on completion)
- `src/app/agent_loop.rs:332` — 60s timeout in `wait_for_tool_results` (Extend for user-destined questions)
- `src/app/agent_loop.rs:271-321` — `dispatch_and_wait_for_tools` (Timeout adjustment integration point)
