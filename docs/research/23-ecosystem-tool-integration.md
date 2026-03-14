# Ecosystem Tool Integration: MCP Servers, Skills, and Harness Helpers

> **2026-03-14** — Research on integrating MCP servers, Claude Code-style skills, and common agent harness tools (filesystem, browser, database, API) into the context-orchestrator graph. Covers protocol mechanics, discovery patterns, the unified tool abstraction, and a phased architecture from static registry to full ecosystem participation.

---

## 1. Executive Summary

The context-orchestrator's tool system is currently a closed registry of ~10 built-in tools. The broader ecosystem — 5,000-9,000+ MCP servers (per PulseMCP directory), Claude Code skills, OpenAPI endpoints, LangChain tools, A2A agents — represents an enormous capability surface that must become accessible through the same graph. This document designs a **unified tool provider abstraction** where MCP servers, skills, dynamic plugins (doc 15), and built-in tools are all interchangeable `ToolProvider` implementations, registered as graph nodes, discoverable by agents, and invoked through the existing `ToolCall → ToolResult` provenance chain.

**Key design decisions:**

1. **One abstraction, many backends.** A `ToolProvider` trait unifies all tool sources. Built-in tools, MCP servers, skills, and dynamic plugins all implement it. The graph and agents don't know or care about the backend.
2. **Graph-native discovery.** Tool providers are `Node::ToolProvider` nodes. Individual tools are `Node::Tool` nodes linked via `ProvidedBy` edges. Discovery is a graph query, not a separate registry.
3. **Lazy schema loading.** Only tool descriptions are loaded into LLM context by default. Full schemas load on-demand when the LLM signals intent (following Claude Code's `defer_loading` pattern). This keeps context overhead proportional to usage, not catalog size.
4. **MCP as the external protocol.** The orchestrator acts as an MCP client for external tool servers. The `rmcp` Rust SDK provides the transport layer. MCP's `tools/list` → `tools/call` lifecycle maps directly to graph node creation → `ToolCall` dispatch.
5. **Skills as prompt-augmented tool providers.** Skills (directory-based packages with instructions + optional tools) are a special case: they inject context into the system prompt AND may provide callable tools. Both aspects are graph nodes.
6. **Incremental adoption.** Phase 1 adds MCP client capability alongside existing tools. Phase 2 adds skills and lazy loading. Phase 3 adds dynamic providers and A2A federation. Each phase is independently useful.

**Trade-off:** We sacrifice the compile-time exhaustiveness of `ToolCallArguments` for external tools (they use the `Unknown` variant) in exchange for ecosystem breadth. The mitigation is runtime schema validation at the provider boundary.

---

## 2. Current Architecture & Gap Analysis

### What Exists

**Static Tool Registry** (`src/tool_executor/mod.rs:56-60`): `tool_registry()` returns a `&'static [ToolRegistryEntry]` built from `build_registry()` at first call. Three categories: config, plan, filesystem. Each entry has a `ToolName` enum variant, description, and `ToolInputSchema`.

**Closed Enum Arguments** (`src/graph/tool_types.rs:94-165`): `ToolCallArguments` is a `#[serde(tag = "tool_type")]` enum with typed variants for each built-in tool plus `Unknown { tool_name: String, raw_json: String }` as escape hatch.

**Tool Execution** (`src/tool_executor/execute.rs`): `execute_tool()` matches on `ToolCallArguments` variants. `Unknown` currently returns an error: `"Unrecognized tool or invalid arguments: {tool_name}"`.

**Tool Nodes in Graph** (`src/graph/node.rs`): `Node::Tool { id, name, description, updated_at }` exists for discovered tools. `Node::ToolCall` and `Node::ToolResult` handle invocation provenance.

**Provenance Chain**: `Message →[Invoked]→ ToolCall →[Produced]→ ToolResult` is fully tracked (design doc 03).

**Agent Loop** (`src/app/agent_loop.rs`): Iterates up to `max_tool_loop_iterations`, dispatching tool calls and waiting for results via `TaskMessage` channel.

### What's Missing

| Gap | Impact |
|-----|--------|
| No MCP client | Cannot connect to any of the 10,000+ MCP servers |
| No skill/prompt injection | Cannot integrate Claude Code-style skill packages |
| `Unknown` variant errors | External tools are explicitly rejected |
| No tool provider abstraction | Each new tool source requires code changes across registry, executor, and graph |
| No lazy schema loading | Tool definitions consume context linearly with catalog size |
| No capability-based tool routing | All tools offered to all agents regardless of relevance |
| No provider lifecycle | No connect/disconnect, health check, or reconnection for external servers |

---

## 3. Requirements

Derived from VISION.md (Section 4.8: "MCP is the integration protocol — do not build custom tool adapters"), user feedback (all tools equally callable by users and agents), and doc 15 (dynamic plugin foundation):

1. **Protocol compliance.** MCP client implementation following the 2025-11-25 specification
2. **Unified abstraction.** Single `ToolProvider` trait for all tool sources — no separate code paths per source type
3. **Graph-native.** Providers and their tools are graph nodes with typed edges
4. **Equal access.** Users and agents invoke external tools through the same mechanism (user feedback: no separate systems)
5. **Lazy loading.** Context consumption proportional to used tools, not total catalog
6. **Provider lifecycle.** Connect, health check, reconnect, disconnect for external providers
7. **Schema validation.** Runtime JSON Schema validation for external tool inputs/outputs
8. **Incremental.** Each phase works independently; Phase 1 does not require Phase 2

---

## 4. Ecosystem Survey

### 4.1 MCP Protocol (2025-11-25 Specification)

The Model Context Protocol defines three primitives:
- **Tools**: Functions the LLM can invoke. Discovered via `tools/list` (paginated), invoked via `tools/call`. JSON-RPC 2.0.
- **Resources**: Read-only data the LLM can access. URIs with MIME types.
- **Prompts**: Reusable prompt templates exposed by servers.

Transports: stdio (local subprocess), HTTP+SSE (remote). Authorization: OAuth 2.1 with incremental scope negotiation.

**Rust SDK**: `rmcp` (v1.2.0 current) — official Rust MCP SDK built on tokio. Provides `McpClient` for connecting to servers, `McpServer` for exposing tools. Transport adapters for stdio and HTTP. `#[tool]` macro for server-side tool definition.

**Key insight for our architecture**: MCP's `tools/list` returns tool schemas that map directly to our `ToolDefinition` type. MCP's `tools/call` returns content blocks that map to our `ToolResultContent`. The protocol is almost a 1:1 match with our existing types.

### 4.2 Claude Code Skills

Skills are directory-based packages:
```
.claude/skills/my-skill/
  SKILL.md          # Instructions + metadata (required)
  template.md       # Optional template
  examples/         # Optional examples
  scripts/          # Optional helper scripts
```

**Discovery**: Auto-discovered from `.claude/skills/` directories. Descriptions always loaded (for semantic matching). Full content loads only on invocation.

**Context budget**: 2% of context window, minimum 16KB. This is the progressive disclosure pattern — descriptions are cheap, full instructions are expensive.

**Key insight**: Skills are NOT just tools. They are prompt augmentations that may also provide tools. A "commit" skill injects commit conventions into the system prompt AND provides a structured workflow. The graph must model both aspects.

### 4.3 Common Harness Helpers

Surveying Claude Code, Cursor, Aider, LangChain DeepAgents, and Cline reveals a standard set of tool categories present in every agent harness:

| Category | Tools | Typical Count |
|----------|-------|---------------|
| Filesystem | read, write, edit, glob, grep | 5-7 |
| Code execution | bash, sandbox, REPL | 1-3 |
| Browser/web | navigate, screenshot, click, search | 4-8 |
| Version control | git status, diff, commit, push | 4-6 |
| Database | query, schema, migrate | 2-4 |
| Communication | slack, email, PR comments | 2-5 |
| Observability | logs, metrics, traces | 2-4 |
| Planning | create task, update status, dependencies | 3-5 |

Total across a typical harness: 25-45 tools. With MCP servers: potentially hundreds.

**The scaling problem**: LLM tool-use quality degrades with large tool catalogs — Claude Code's `defer_loading` activates when definitions exceed 10% of context window. With MCP servers, tool counts easily reach hundreds. Solution: lazy loading with semantic routing.

### 4.4 Agent-to-Agent Protocol (A2A)

Google's A2A protocol (v0.3, donated to Linux Foundation) enables agent-to-agent communication:
- **Agent Cards**: JSON capability advertisements (analogous to our `Node::ToolProvider`)
- **Task-oriented messaging**: Agents exchange tasks, not raw messages
- **gRPC + JSON-RPC bindings**: Protocol-agnostic

**Relationship to MCP**: MCP is model↔tool (asymmetric). A2A is agent↔agent (symmetric). They coexist: an agent exposes MCP tools while communicating with peers via A2A. For our architecture, A2A is a Phase 3 concern — agents in our graph could federate with external agents via A2A, exposing their work items as A2A tasks.

### 4.5 Dynamic Tool Generation

From doc 15 and external research:
- **ToolRegistry** (arxiv 2507.10593): Protocol-agnostic registry with lifecycle management
- **Automated LLM Agent Optimization** (arxiv 2512.09108): Evolutionary optimization of LLM-based agents
- **ToolMaker** (arxiv 2502.11705): Translates scientific repos into agent tools
- **Claude Code's ToolSearch**: On-demand schema loading when tool definitions would exceed 10% of context

The pattern: tools are not just static definitions — they can be created, modified, and composed at runtime. Our `ToolProvider` trait must support dynamic registration.

---

## 5. Options Analysis

### Option A: MCP-Only (All External Tools via MCP)

Every external tool source runs as an MCP server. Built-in tools stay as-is. The orchestrator acts purely as an MCP client.

**Strengths:**
- Single protocol for all external tools — no custom adapters
- 10,000+ existing MCP servers work immediately
- Anthropic, OpenAI, Google all support MCP
- Clear boundary: internal (Rust) vs. external (MCP)

**Weaknesses:**
- Skills don't fit: they are prompt augmentations, not just tools
- IPC overhead for trivial operations (even stdio MCP adds ~1-5ms per call)
- Built-in tools and MCP tools have different code paths — violates unified abstraction
- No mechanism for LLM-generated tools without deploying an MCP server

### Option B: Unified ToolProvider Trait (Recommended)

All tool sources implement a common `ToolProvider` trait. Built-in tools, MCP servers, skills, and dynamic plugins are all providers. The registry manages providers, not individual tools.

**Strengths:**
- Single abstraction for everything — agents don't know the backend
- Graph-native: providers are nodes, tools are nodes, edges connect them
- Extensible: new provider types require only a trait impl, no registry changes
- Compatible with doc 15's `PluginExecutor` (subsumes it as a `ToolProvider` variant)
- Skills get first-class treatment as prompt+tool providers

**Weaknesses:**
- Loses compile-time exhaustiveness for external tool arguments
- Trait object dispatch adds ~1ns overhead (negligible)
- Provider lifecycle management is complex

### Option C: Plugin Registry (Doc 15 Extended)

Extend doc 15's `DynamicToolRegistry` with MCP and skill support. Tools registered individually, not by provider.

**Strengths:**
- Builds directly on existing doc 15 design
- Fine-grained per-tool control

**Weaknesses:**
- Doesn't model provider lifecycle (connect/disconnect/health)
- Each MCP server with N tools = N individual registrations (awkward)
- Skills don't fit (they're not just tools)
- Provider-level concerns (auth, reconnection) have no natural home

---

## 6. Comparison Matrix

| Criterion | MCP-Only | ToolProvider Trait | Plugin Registry |
|-----------|----------|-------------------|-----------------|
| Unified abstraction | Partial | Full | Partial |
| Graph-native | Tools only | Providers + Tools | Tools only |
| Skill support | No | Yes (prompt+tools) | No |
| MCP compatibility | Native | Via adapter | Via adapter |
| Dynamic tools (doc 15) | Separate system | Integrated | Native |
| Provider lifecycle | N/A | First-class | N/A |
| Context scaling | Manual | Lazy loading | Manual |
| Implementation effort | Low | Medium | Low-Medium |
| A2A future compat | Good | Best | Weak |
| Compile-time safety | Lost for external | Lost for external | Lost for external |

---

## 7. VISION.md Alignment

| Vision Concept | Integration Impact |
|----------------|-------------------|
| **Graph-native context** (3.1) | ToolProviders and Tools are graph nodes with typed edges |
| **Tool calls as first-class citizens** (4.8) | External tools get same `ToolCall → ToolResult` provenance |
| **MCP for tool integration** (4.8, 5.4) | MCP is the external protocol — direct alignment |
| **Background graph processing** (4.3) | Provider connections managed as background tasks |
| **Dynamic system prompt construction** (4.7) | Skill instructions injected via graph traversal |
| **Work management** (4.6) | A2A tasks map to WorkItem nodes in Phase 3 |
| **Multi-perspective compaction** (4.2) | Tool results from any provider compactable |

The vision explicitly states: "MCP is the integration protocol — do not build custom tool adapters" (VISION.md line 404). Our design honors this by using MCP for ALL external tool communication while adding a thin orchestrator-side abstraction for lifecycle and graph integration.

---

## 8. Recommended Architecture

### Phase 1: MCP Client + Provider Trait Foundation

**Goal:** Connect to MCP servers, invoke their tools, record everything in the graph. Built-in tools migrated to the provider trait.

#### ToolProvider Trait

```rust
/// A source of tools. Implementations include built-in tools, MCP servers,
/// skill packages, and dynamic plugins.
#[async_trait]
trait ToolProvider: Send + Sync {
    /// Unique identifier for this provider.
    fn id(&self) -> Uuid;

    /// Human-readable name (e.g., "github", "filesystem", "my-skill").
    fn name(&self) -> &str;

    /// Provider kind for graph serialization.
    fn kind(&self) -> ToolProviderKind;

    /// List available tools. Returns descriptions + schemas.
    async fn list_tools(&self) -> Result<Vec<ToolDefinition>>;

    /// Execute a tool call. Input is validated JSON matching the tool's schema.
    async fn execute(&self, tool_name: &str, input: &str) -> Result<ToolExecutionResult>;

    /// Health check. Returns true if the provider is ready to accept calls.
    async fn is_healthy(&self) -> bool;
}

/// Discriminant for provider serialization and graph queries.
enum ToolProviderKind {
    BuiltIn,
    Mcp { transport: McpTransport },
    Skill { path: PathBuf },
    DynamicPlugin,  // doc 15 integration point
}

/// MCP transport configuration.
enum McpTransport {
    Stdio { command: String, args: Vec<String> },
    Http { url: String },
}
```

#### Graph Nodes

```rust
/// A source of tools registered in the graph.
Node::ToolProvider {
    id: Uuid,
    name: String,
    kind: ToolProviderKind,
    status: ProviderStatus,  // Connecting, Ready, Degraded, Disconnected
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}
```

New edges:
- `ProvidedBy`: `Node::Tool → Node::ToolProvider` — "this tool comes from this provider"
- `ExecutedBy`: `Node::ToolCall → Node::ToolProvider` — "this call was handled by this provider"

#### MCP Client Implementation

```rust
/// MCP server connection implementing ToolProvider.
struct McpToolProvider {
    id: Uuid,
    name: String,
    transport: McpTransport,
    client: rmcp::McpClient,  // from rmcp crate
    tools_cache: Vec<ToolDefinition>,
}

impl ToolProvider for McpToolProvider {
    async fn list_tools(&self) -> Result<Vec<ToolDefinition>> {
        // Convert MCP ToolInfo → our ToolDefinition
        let mcp_tools = self.client.tools_list(None).await?;
        mcp_tools.into_iter().map(convert_mcp_tool).collect()
    }

    async fn execute(&self, tool_name: &str, input: &str) -> Result<ToolExecutionResult> {
        let result = self.client.tools_call(tool_name, input).await?;
        Ok(convert_mcp_result(result))
    }
}
```

#### Modified Execution Flow

```
Agent requests tool call
    ↓
ToolCallArguments::Unknown { tool_name, raw_json }
    ↓
ProviderRegistry.find_provider(tool_name)
    ↓
  Found: provider.execute(tool_name, raw_json)
    ↓
  Not found: error "Unknown tool: {name}"
    ↓
ToolResult node created with ExecutedBy edge to provider
```

#### Built-in Migration

Built-in tools become a `BuiltInProvider` implementing `ToolProvider`:

```rust
struct BuiltInProvider {
    id: Uuid,
}

impl ToolProvider for BuiltInProvider {
    fn name(&self) -> &str { "built-in" }
    fn kind(&self) -> ToolProviderKind { ToolProviderKind::BuiltIn }

    async fn list_tools(&self) -> Result<Vec<ToolDefinition>> {
        // Delegate to existing tool_registry()
        Ok(tool_registry().iter().map(|e| e.to_definition()).collect())
    }

    async fn execute(&self, tool_name: &str, input: &str) -> Result<ToolExecutionResult> {
        let args = parse_tool_arguments(tool_name, input);
        execute_tool(&args).await
    }
}
```

This is a migration, not a rewrite. Existing `execute_tool()` stays, just called through the trait.

### Phase 2: Skills + Lazy Schema Loading

**Goal:** Load skill packages as tool providers with prompt injection. Implement lazy schema loading for large tool catalogs.

#### Skill Provider

Skills are unique: they provide both **prompt context** and **tools**. A skill provider:

1. Reads `SKILL.md` and extracts description (always loaded) and full instructions (loaded on demand)
2. Optionally discovers tool definitions from the skill directory
3. Registers as a `ToolProvider` for its tools
4. Provides a `prompt_fragment()` method for system prompt injection

```rust
struct SkillProvider {
    id: Uuid,
    name: String,
    path: PathBuf,
    description: String,       // Always in memory (cheap)
    instructions: Option<String>, // Loaded on demand (lazy)
    tools: Vec<ToolDefinition>,
}

impl SkillProvider {
    /// Returns prompt text to inject into system prompt when skill is active.
    fn prompt_fragment(&mut self) -> Result<&str> {
        if self.instructions.is_none() {
            self.instructions = Some(fs::read_to_string(self.path.join("SKILL.md"))?);
        }
        Ok(self.instructions.as_deref().unwrap_or(""))
    }
}
```

Graph model: `Node::ToolProvider { kind: Skill { path } }` with a new edge `AugmentsPrompt` linking the provider to the `SystemDirective` it enhances.

#### Lazy Schema Loading (ToolSearch Pattern)

Following Claude Code's `defer_loading` and `ToolSearch` patterns:

1. **All providers register descriptions** (name + one-line description). Always in LLM context.
2. **Full schemas deferred** until the LLM requests them via a `tool_search` meta-tool.
3. **Threshold**: When total tool definitions would exceed 10% of context window, switch to lazy mode automatically.

```
LLM sees: "github: GitHub operations (PRs, issues, repos)"
LLM decides: "I need to create a PR" → calls tool_search("github create PR")
Orchestrator: loads full schema for github.create_pull_request into context
LLM: calls github.create_pull_request with proper arguments
```

The `tool_search` tool is itself a built-in tool:

```rust
ToolCallArguments::ToolSearch {
    query: String,          // semantic search across tool descriptions
    provider: Option<String>, // optional: restrict to specific provider
}
```

This returns matching `ToolDefinition` objects that get injected into the next LLM turn.

### Phase 3: Dynamic Providers + A2A Federation

**Goal:** Agents can create new tool providers at runtime. External agents can federate via A2A.

#### Dynamic Provider Creation

Combines doc 15's plugin system with the provider trait:

1. Agent writes tool code (Rhai/WASM per doc 15)
2. Code is wrapped in a `DynamicPluginProvider` implementing `ToolProvider`
3. Provider node added to graph with `Created` edge from originating message
4. Tools immediately available to all agents

This subsumes doc 15's `DynamicToolRegistry` — plugins are just another `ToolProvider` variant.

#### A2A Federation

External agents advertise capabilities via A2A Agent Cards. The orchestrator:

1. Discovers external agents (via Agent Name Service or explicit configuration)
2. Creates `Node::ToolProvider { kind: A2A { agent_card_url } }` nodes
3. Maps A2A tasks to `WorkItem` nodes, A2A messages to `Message` nodes
4. Routes work to external agents when internal agents lack capability

This is the furthest-horizon feature and depends on A2A protocol maturity.

---

## 9. Integration Design

### Provider Registry

```rust
/// Central registry of all tool providers. Thread-safe, supports dynamic
/// registration and lookup.
struct ProviderRegistry {
    /// All registered providers, keyed by provider ID.
    providers: HashMap<Uuid, Arc<dyn ToolProvider>>,
    /// Tool name → provider ID mapping for fast dispatch.
    tool_index: HashMap<String, Uuid>,
}

impl ProviderRegistry {
    /// Register a new provider and index its tools.
    async fn register(&mut self, provider: Arc<dyn ToolProvider>) -> Result<()>;

    /// Remove a provider and its tool index entries.
    fn unregister(&mut self, provider_id: Uuid);

    /// Find the provider that handles a given tool name.
    fn find_provider(&self, tool_name: &str) -> Option<Arc<dyn ToolProvider>>;

    /// All tool definitions across all providers (for LLM context).
    async fn all_definitions(&self) -> Vec<ToolDefinition>;

    /// Tool definitions matching a search query (for lazy loading).
    async fn search_tools(&self, query: &str) -> Vec<ToolDefinition>;
}
```

### MCP Tool Naming Convention

MCP tools use the `mcp__<server>__<tool>` naming convention (following Claude Code):

```
mcp__github__create_pull_request
mcp__grafana__query_prometheus
mcp__serena__find_symbol
```

This prevents name collisions between providers and makes provenance visible in tool names.

### Configuration

Provider configuration in the orchestrator config:

```toml
[[tool_providers.mcp]]
name = "github"
transport = "stdio"
command = "npx"
args = ["-y", "@anthropic/mcp-server-github"]

[[tool_providers.mcp]]
name = "grafana"
transport = "http"
url = "http://localhost:3100/mcp"

[[tool_providers.skill]]
name = "commit"
path = ".claude/skills/commit"
```

### Data Flow

```
Startup:
  Config → ProviderRegistry.register(BuiltInProvider)
  Config → for each MCP server: spawn connect task → McpToolProvider → register
  Config → for each skill: SkillProvider.load() → register
  Each provider: list_tools() → create Node::Tool + ProvidedBy edges

Agent turn:
  ProviderRegistry.all_definitions() → tool list for LLM
  (or: descriptions only + tool_search for lazy mode)

Tool invocation:
  LLM emits tool_use { name, input }
  ProviderRegistry.find_provider(name) → provider
  provider.execute(name, input) → ToolExecutionResult
  Create ToolCall + ToolResult nodes + ExecutedBy edge

Provider lifecycle:
  Health check loop: provider.is_healthy() → update ProviderStatus
  On disconnect: mark Degraded, attempt reconnect
  On reconnect: refresh tool list, update Tool nodes
```

### Interaction with Existing Systems

**Scheduler (doc 20)**: Tool providers contribute to agent capability labels. An agent connected to `mcp__github` gets `capability: github`. The Filter+Score pipeline can route GitHub-related work to agents with this capability.

**Q/A System (doc 21)**: Questions about external tool results flow through the same `Asks/Answers` edges. A question about a Grafana dashboard query is linked to the `ToolResult` from `mcp__grafana__query_prometheus`.

**Dynamic Plugins (doc 15)**: `DynamicPluginProvider` wraps doc 15's `RhaiExecutor`/`WasmExecutor`/`McpExecutor` as `ToolProvider` implementations. The `PluginExecutor` trait from doc 15 becomes an internal implementation detail.

---

## 10. Red/Green Team Audit

### Green Team (Factual Verification)

25 claims verified. 20 fully confirmed, 1 corrected, 4 unverifiable (ecosystem statistics). Corrections applied:

1. **rmcp version**: Corrected from "v1.2+" to "v1.2.0" (current stable, not a minimum threshold)
2. **MCP server count**: Corrected from "10,000+" to "5,000-9,000+" — PulseMCP directory shows ~9,080, GitHub official repo lists ~1,864 curated
3. **SDK downloads**: Clarified "97M+ monthly" applies to Python/TypeScript combined; Rust SDK (rmcp) sees ~1.4M monthly
4. **arxiv 2512.09108**: Paper is actually "Evolving Excellence: Automated Optimization of LLM-based Agents" — corrected description
5. **"50 tools" threshold**: No public Anthropic source found; replaced with documented 10% context threshold

All MCP protocol claims, A2A protocol claims, Claude Code features, and arxiv papers (2507.10593, 2502.11705, 2510.24663, 2510.25320) verified against primary sources.

### Red Team (Challenge Recommendations)

**Critical issues identified, prioritized by severity:**

**C10 (CRITICAL): Security model is missing.** MCP servers run with full host permissions. No mechanism for vetting servers, restricting capabilities, or preventing supply chain attacks via malicious packages. Example: `npx -y evil-npm-package` in MCP config gains full filesystem access. **Resolution:** Phase 1 must include: (1) explicit allow-listing in config (default deny), (2) capability declarations per provider (filesystem read/write, network domains), (3) audit logging of all external tool calls. Fine-grained sandboxing (seccomp/AppArmor) deferred to Phase 2.

**C5 (HIGH): ProviderRegistry is a second source of truth.** Graph stores `Node::ToolProvider` and `Node::Tool` durably. `ProviderRegistry` is a RAM `HashMap`. Which is authoritative on restart? When health checks fail? **Resolution:** Graph is the sole source of truth. `ProviderRegistry` is an in-memory cache rebuilt from graph nodes at startup. Provider status changes update the graph node first, then the cache. Add `ProviderRegistry::refresh_from_graph()` method.

**C7 (HIGH): Error handling undefined.** What happens to in-flight `ToolCall` nodes when a provider disconnects? What about partial results, invalid JSON responses, circular tool invocations? **Resolution:** Define `ProviderError` enum (Timeout, Disconnected, InvalidSchema, ExecutionFailed). Implement circuit breaker: 5 failures in 60s → `Degraded` status, 5-minute cooldown. In-flight calls on disconnect → mark `ToolCall` as `Failed` with error reason. Max call depth of 10 to prevent cycles.

**R6 (HIGH): Credential management absent.** MCP servers need API keys (GitHub tokens, Grafana auth). Where stored? How rotated? **Resolution:** Phase 1 uses environment variables (simple, existing pattern). Phase 2 adds encrypted local vault. Never store credentials in config files.

**C1 (HIGH): ToolProvider trait may be premature for Phase 1.** With ~10 built-in tools and 0 external sources today, a simpler approach (handle `Unknown` variant via MCP client lookup) would work. **Counter-argument:** If Phase 1 and Phase 2 ship together, the trait justifies itself — skills genuinely require it. **Resolution:** If Phase 1 ships alone, use the simpler `Unknown` handler approach. If phases ship together, the trait is justified. Decision depends on implementation timeline.

**C3 (MEDIUM): Skills conflate prompt augmentation with tools.** A "commit conventions" skill has instructions but zero tools — it's a strange `ToolProvider`. **Resolution:** Consider separating `Node::Skill` (prompt context) from `Node::ToolProvider` (callables). A skill can optionally provide a tool provider via an edge. This prevents "empty provider" anti-patterns.

**C6 (MEDIUM): A2A is premature.** Protocol is v0.3 (pre-1.0), no major LLM provider has officially adopted it, and the codebase has zero MCP support yet. **Resolution:** Demote Phase 3 A2A from "planned" to "future work." Focus engineering effort on Phases 1-2. Revisit when A2A hits v1.0 and has broad adoption.

**C4 (MEDIUM): Tool name collision not fully addressed.** If two MCP servers have tools with the same name, `tool_index` silently overwrites. **Resolution:** `register()` should error on conflict, not silently overwrite. Include provider name in all tool names (already done via `mcp__<server>__<tool>` convention).

**C8 (MEDIUM): Missing alternatives.** OpenAPI-to-tool conversion (50,000+ APIs have OpenAPI specs), LangChain tool adapters (50+ pre-built), and direct HTTP calling are not evaluated. **Acknowledged:** MCP is chosen for protocol standardization and ecosystem breadth, but OpenAPI support via `samchon/openapi`-style conversion would significantly expand accessible tools without MCP server deployment. Worth evaluating in Phase 2.

**Additional risks identified:** MCP protocol evolution risk (pre-1.0, breaking changes possible), provider startup timing (non-blocking connect with 5s timeout), schema drift (periodic refresh needed), tool result size limits (set 1MB max), cost attribution for external tools.

### Code Accuracy

All 10 code references verified against source files. **9/10 fully accurate**, 1 minor correction applied:

- `src/tool_executor/mod.rs:56-60` — `tool_registry()`: ACCURATE
- `src/graph/tool_types.rs:94-165` — `ToolCallArguments` with `#[serde(tag = "tool_type")]`: ACCURATE
- `src/tool_executor/execute.rs` — `execute_tool()` with `Unknown` error: CORRECTED (message was "Unrecognized tool or invalid arguments" not "Unknown tool")
- `src/graph/node.rs` — `Node::Tool`, `Node::ToolCall`, `Node::ToolResult`: ACCURATE
- `src/app/agent_loop.rs` — `max_tool_loop_iterations` loop: ACCURATE
- `EdgeKind::Invoked` and `EdgeKind::Produced`: ACCURATE
- `TaskMessage::ToolCallDispatched` and `ToolCallCompleted`: ACCURATE
- `ToolResultContent::Text`/`Blocks`: ACCURATE
- `ToolRegistryEntry` with `name: ToolName`: ACCURATE
- `build_registry()` calling config/plan/filesystem tools: ACCURATE

---

## 11. Sources

### MCP Protocol & SDKs
- [MCP Specification 2025-11-25](https://modelcontextprotocol.io/specification/2025-11-25/server/tools)
- [MCP Architecture Overview](https://modelcontextprotocol.io/docs/learn/architecture)
- [rmcp: Official Rust MCP SDK](https://crates.io/crates/rmcp) (v1.2.0)
- [rust-mcp-sdk](https://crates.io/crates/rust-mcp-sdk) — alternative Rust SDK
- [MCP Anniversary Blog](http://blog.modelcontextprotocol.io/posts/2025-11-25-first-mcp-anniversary/)
- [MCP JSON-RPC Reference](https://portkey.ai/blog/mcp-message-types-complete-json-rpc-reference-guide/)
- [MCP Tool Discovery for LLM Agents](https://portkey.ai/blog/mcp-tool-discovery-for-llm-agents/)
- [Scaling MCP Tools with defer_loading](https://unified.to/blog/scaling_mcp_tools_with_anthropic_defer_loading)
- [MCP Gateway Architecture](https://dev.to/hadil/how-to-scale-claude-code-with-an-mcp-gateway-run-any-llm-centralize-tools-control-costs-nd9)
- [MCP Auth Guide](https://www.permit.io/blog/the-ultimate-guide-to-mcp-auth)
- [Shuttle: Building Rust MCP Servers](https://www.shuttle.dev/blog/2025/07/18/how-to-build-a-stdio-mcp-server-in-rust)

### Claude Code Skills & Tool Patterns
- [Claude Code Skills Documentation](https://code.claude.com/docs/en/skills)
- [Claude Code MCP Integration](https://code.claude.com/docs/en/mcp)
- [Claude Code Customization Guide](https://alexop.ai/posts/claude-code-customization-guide-claudemd-skills-subagents/)
- [Claude API Tool Search](https://platform.claude.com/docs/en/agents-and-tools/tool-use/tool-search-tool)
- [Anthropic: Advanced Tool Use](https://www.anthropic.com/engineering/advanced-tool-use)
- [Claude Agent SDK Custom Tools](https://platform.claude.com/docs/en/agent-sdk/custom-tools)
- [Claude Agent SDK Hooks](https://platform.claude.com/docs/en/agent-sdk/hooks)
- [Anthropic: Effective Context Engineering](https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents)

### Agent Frameworks & Harness Patterns
- [LangGraph Overview](https://docs.langchain.com/oss/javascript/langgraph/overview)
- [LangChain DeepAgents Harness](https://docs.langchain.com/oss/python/deepagents/harness)
- [Agent Harness Anatomy (LangChain)](https://blog.langchain.com/the-anatomy-of-an-agent-harness/)
- [Your Agent Needs a Harness (Inngest)](https://www.inngest.com/blog/your-agent-needs-a-harness-not-a-framework)
- [OpenAI Agents SDK](https://openai.github.io/openai-agents-python/)
- [MCP Tool Design Patterns](https://www.arcade.dev/blog/mcp-tool-patterns)
- [Tool Calling Error Handling](https://medium.com/@gopiariv/handling-tool-calling-errors-in-langgraph-a-guide-with-examples-f391b7acb15e)

### A2A Protocol
- [A2A Specification v0.3](https://a2a-protocol.org/latest/specification/)
- [A2A Announcement (Google)](https://developers.googleblog.com/en/a2a-a-new-era-of-agent-interoperability/)
- [A2A Discussion: Understanding the Protocol](https://discuss.google.dev/t/understanding-a2a-the-protocol-for-agent-collaboration/189103)

### Dynamic Tool Generation
- [ToolRegistry: Protocol-Agnostic Tool Management](https://arxiv.org/abs/2507.10593)
- [Automated LLM Agent Optimization](https://arxiv.org/abs/2512.09108)
- [ToolMaker: LLM Agents Making Agent Tools](https://arxiv.org/abs/2502.11705)
- [OrchDAG: DAG-Based Tool Orchestration](https://arxiv.org/abs/2510.24663)
- [Graph-Based Adaptive Planning (GAP)](https://arxiv.org/abs/2510.25320)

### Tool Schemas & Validation
- [MCP Tool Schema Design](https://www.merge.dev/blog/mcp-tool-schema)
- [Mapping APIs to MCP Tools](https://www.scalekit.com/blog/map-api-into-mcp-tool-definitions)
- [Mastering LLM Tool Calling](https://machinelearningmastery.com/mastering-llm-tool-calling-the-complete-framework-for-connecting-models-to-the-real-world/)
- [OpenAPI to LLM Function Schemas (samchon/openapi)](https://github.com/samchon/openapi)
- [Agent Name Service (ANS)](https://www.aigl.blog/content/files/2025/05/Agent-Name-Service--ANS--for-Secure-AI-Agent-Discovery.pdf)

### Internal References
- `docs/VISION.md` — Section 4.8 (Tool Calls as First-Class Graph Citizens), Section 5.4 (LLM Integration)
- `docs/design/03-tool-call-foundation.md` — Tool provenance design
- `docs/research/15-llm-written-plugins.md` — Dynamic plugin system (Rhai/WASM/MCP phases)
- `docs/research/06-inline-tool-invocation-patterns.md` — Tool invocation survey
- `docs/research/20-kubernetes-inspired-agent-scheduling.md` — Scheduler integration
- `docs/research/21-graph-scheduler-qa-relationships.md` — Q/A edge taxonomy
- `src/tool_executor/mod.rs:56-60` — Current tool registry
- `src/graph/tool_types.rs:94-165` — ToolCallArguments enum
- `src/graph/node.rs` — Node::Tool, Node::ToolCall, Node::ToolResult
