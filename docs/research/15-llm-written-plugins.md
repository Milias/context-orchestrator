# LLM-Written Dynamic Plugins

> **2026-03-13** — Research on integrating LLM-generated tools into the context-orchestrator graph, covering plugin architectures across 30+ real-world systems, sandboxing models, and a phased approach from embedded scripting to WASM to remote execution.

---

## 1. Executive Summary

Agents in context-orchestrator should be able to **write new tools at runtime** — generate code, register it as a callable tool, and have other agents invoke it through the graph. This requires solving language support, sandboxed execution, dependency management, and remote execution. After surveying 30+ plugin systems (editors, games, browsers, infrastructure, AI frameworks), we recommend a **three-phase approach**: Phase 1 uses Rhai (Rust-native embedded scripting) for immediate "LLM writes tool → tool runs" capability; Phase 2 adds WebAssembly via Extism for multi-language support with strong sandboxing; Phase 3 enables remote execution via MCP servers and containers for full language/dependency freedom. The key trade-off is isolation vs. latency: embedded scripting is fastest but single-language; WASM is multi-language with ~10-50% overhead; containers provide full isolation but add boot latency.

---

## 2. Current Architecture & Gap Analysis

### What Exists

Tools are statically defined in `src/tool_executor/mod.rs:32-109` via `registered_tool_definitions()`, returning a `Vec<ToolDefinition>`. Each tool has a name, description, and JSON schema (`src/llm/tool_types.rs:6-16`).

Tool arguments are parsed into a closed enum `ToolCallArguments` (`src/graph/tool_types.rs:15-42`) with an `Unknown { tool_name, raw_json }` catch-all variant. Execution dispatches via `execute_tool()` (`src/tool_executor/mod.rs:112-139`), a match on this enum.

The agent loop (`src/app/agent_loop.rs:53-124`) streams LLM responses, creates `ToolCall` nodes, dispatches execution via `spawn_tool_execution()` (`src/tool_executor/mod.rs:142-155`), waits for results with a 60-second timeout, and creates `ToolResult` nodes linked via `Produced` edges.

Graph provenance is fully tracked: `Message →[Invoked]→ ToolCall →[Produced]→ ToolResult`.

### What's Missing

| Gap | Description |
|-----|-------------|
| **Dynamic registration** | Tools are hardcoded; no way to register new tools at runtime |
| **Plugin execution** | `execute_tool()` only handles known enum variants; `Unknown` returns an error |
| **Schema generation** | No mechanism for an LLM to produce a `ToolDefinition` for a new tool |
| **Sandboxing** | Tool executors run with full process permissions (file I/O, network) |
| **Lifecycle management** | No concept of tool installation, versioning, or unregistration |
| **Dependency resolution** | No mechanism for tools that need external libraries or system packages |
| **Remote dispatch** | All execution is local and in-process |

---

## 3. Requirements

Derived from VISION.md and the user's request:

1. **LLM-authored tools**: An agent must be able to write code that becomes a callable tool
2. **Graph-native**: Plugin nodes, edges, and provenance must live in the conversation graph
3. **Multi-language** (Phase 2+): Support tools written in languages beyond Rust
4. **Sandboxed execution**: Untrusted code must not access the host filesystem, network, or process memory arbitrarily
5. **Dynamic registration**: Tools registered at runtime, discoverable by other agents in the same session
6. **Dependency management**: Mechanism for tools that need external packages
7. **Remote execution** (Phase 3): Tools that run on remote infrastructure for heavy workloads
8. **Hot-reload**: Update a tool's implementation without restarting the orchestrator
9. **No `serde_json::Value` in struct fields**: Per project rules, all schemas must be typed

---

## 4. Real-World Plugin System Survey

### Architectural Patterns

We surveyed 30+ systems across six categories. Every plugin system makes a fundamental choice along two axes: **isolation model** and **language coupling**.

| Pattern | Examples | Isolation | Overhead | Multi-lang | Hot-reload |
|---------|----------|-----------|----------|------------|------------|
| Embedded interpreter | Factorio (Lua), Neovim (Lua), Nginx/OpenResty (LuaJIT), Rhai | Moderate | Low (<2x) | Single | Yes |
| Process-per-plugin | Terraform (gRPC), HashiCorp go-plugin | Strong | Medium (IPC) | Yes | No |
| WASM sandbox | Envoy (proxy-wasm), Traefik, Extism | Strong | 10-50% | Yes | Yes |
| Container | GitHub Actions, Dagger, E2B (Firecracker) | Strongest | High (boot) | Yes | No |
| Classloader/module | IntelliJ (Java), Minecraft/Paper, Redis | Weak | Minimal | Single | Partial |
| Protocol server | MCP, GPT Actions (OpenAPI), Jupyter (ZeroMQ) | Strong | Medium (net) | Yes | Yes |

### Deep-Dive: Key Systems

**Factorio (embedded Lua, strong sandbox):** Each mod's `control.lua` runs in its own Lua interpreter. State does not carry between lifecycle stages (Settings → Data → Control). Cross-mod communication is only via explicit `remote` interfaces. This is the gold standard for embedded scripting isolation — per-interpreter sandboxing with staged APIs that limit what each phase can access.

**VS Code (process-isolated Extension Host):** Extensions run in a separate Node.js process, communicating with the main renderer via bidirectional RPC. A curated `vscode.*` API surface controls what extensions can access. Activation events enable lazy-loading. This pattern — process isolation + curated API + lazy activation — is the most successful editor plugin architecture.

**Terraform (process-per-plugin, gRPC):** Each provider is a separate binary. Terraform Core launches it as a subprocess and communicates over gRPC on loopback. Protocol Buffers define the contract. The `go-plugin` framework (used by all HashiCorp products for 4+ years) handles subprocess lifecycle, health checks, and protocol negotiation. Multiple protocol versions can coexist.

**Envoy (WASM filters):** WebAssembly modules loaded at config time via the proxy-wasm ABI. Each module gets a separate WASM execution instance cloned to each worker thread. CPU-bound workloads see <2x slowdown vs. native. Headers/body data are copied between Envoy memory and WASM linear memory. WASI 0.2 (stable since January 2024) standardizes filesystem, networking, and clock access.

**MCP (protocol server, industry standard):** Client-server architecture where the host discovers tools via `tools/list` and invokes them via `tools/call`. JSON-RPC over stdio (local) or HTTP (remote). Adopted by OpenAI, Google DeepMind, Microsoft, 97M+ monthly SDK downloads. Donated to Agentic AI Foundation (Linux Foundation) in December 2025.

**Grafana (frontend sandbox):** Since v11.5, plugins run in a separate JavaScript context that prevents modification of Grafana's UI or global browser objects. Backend plugins are separate Go binaries. This dual model — script sandbox for UI, process isolation for compute — is worth noting.

**Redis (dynamic loading, hot-reload):** Modules are shared libraries with a `RedisModule_Init()` entry point. Commands registered via `RedisModule_CreateCommand()`. Can be loaded at runtime via `MODULE LOAD` without restart. Same-process execution but with a well-defined API boundary.

### Lessons Learned

1. **Factorio's staged APIs** are ideal for LLM plugins: different capabilities available during definition (schema) vs. execution (runtime)
2. **VS Code's activation events** solve the "too many tools" problem: tools load on-demand based on context
3. **Terraform's protocol versioning** prevents breaking changes: plugins declare which protocol version they support
4. **Envoy proves WASM overhead is acceptable** for network-bound workloads (~10-20% for proxy filters)
5. **MCP is becoming the universal tool protocol**: all major LLM providers now support it
6. **Inner Platform Effect is the biggest risk**: over-abstracting the plugin system into a poor reimplementation of an OS

---

## 5. Options Analysis

### Option A: Embedded Scripting (Rhai)

**Description:** Embed the Rhai scripting engine. LLMs generate Rhai scripts that define tool behavior. Scripts are registered with a JSON schema and executed in-process.

**Strengths:**
- Pure Rust, no external dependencies, compiles with `cargo build`
- Sandboxed by default: cannot access filesystem, network, or host memory unless explicitly exposed
- Protected against stack overflow and runaway scripts (configurable limits)
- Fastest path to "LLM writes tool → tool runs" — no compilation step
- Syntax is Rust-like with JavaScript elements, familiar to LLMs
- ~80KB binary overhead

**Weaknesses:**
- Single language (Rhai only)
- ~2x slower than Python for compute-heavy tasks
- No external dependency support (no `import`, no package manager)
- Limited stdlib (math, string manipulation, basic collections)

**Crate:** `rhai` (latest stable, actively maintained, ~5,000 GitHub stars)

### Option B: WebAssembly via Extism

**Description:** Use Extism (built on Wasmtime) as a WASM plugin runtime. Tools compiled to WASM from any supported language, loaded and executed with strong sandboxing.

**Strengths:**
- Multi-language: Rust, Go, C/C++, JavaScript, Python, Ruby, Zig, Haskell, and more
- Strong sandboxing via WASM linear memory model (Software Fault Isolation)
- Hot-reloadable: swap modules without restart
- Extism handles host↔plugin communication, memory management, and ABI complexity
- WASI 0.2 stable (January 2024), WASI 0.3 adds native async
- Industry-proven: Envoy, Traefik, Shopify, Fermyon all use WASM plugins in production

**Weaknesses:**
- 10-50% performance overhead vs. native
- Compilation step required (LLM writes source → compile to .wasm → load)
- WASM ecosystem still maturing (debugging tools, error messages)
- Memory copying between host and guest for data exchange
- Dependencies must be compiled into the WASM module (no runtime package manager)

**Crates:** `extism` (1.4+), `wasmtime` (stable), `wasm-bridge` (for portability)

### Option C: Process-per-Plugin (gRPC)

**Description:** Each plugin runs as a separate process, communicating with the orchestrator over gRPC (following HashiCorp's go-plugin pattern).

**Strengths:**
- Strongest isolation without hardware virtualization
- Full language freedom (anything that speaks gRPC)
- Crash isolation: plugin crash doesn't affect host
- Mature pattern (HashiCorp uses it across all products for 4+ years)
- Full dependency freedom per-process

**Weaknesses:**
- High complexity: process lifecycle management, health checks, reconnection
- IPC latency for every tool call
- No hot-reload (must restart subprocess)
- Heavy for lightweight tools (process-per-tool overhead)
- Requires gRPC codegen infrastructure

**Crates:** `tonic` (gRPC), `prost` (protobuf)

### Option D: MCP Servers (Protocol Standard)

**Description:** Plugins are MCP servers — local (stdio) or remote (HTTP). The orchestrator acts as an MCP client, discovering and invoking tools via the standard protocol.

**Strengths:**
- Industry standard (OpenAI, Google, Microsoft, Anthropic all support it)
- 10,000+ existing MCP servers available
- Remote execution built-in (HTTP transport)
- Full language freedom
- Tool discovery via `tools/list` is standardized
- Anthropic already publishes MCP SDKs for TypeScript and Python

**Weaknesses:**
- Heavier than in-process execution for simple tools
- No built-in sandboxing (server is trusted or must be containerized separately)
- JSON-RPC overhead for high-frequency calls
- Stdio transport requires subprocess management (similar to go-plugin)

**Crates:** `rmcp` (Rust MCP SDK, early stage), or raw JSON-RPC via `serde_json` + `tokio`

### Option E: Container Execution (Firecracker/Docker)

**Description:** Each tool runs in an isolated container or microVM. Strongest isolation for untrusted code.

**Strengths:**
- Hardware-enforced isolation (Firecracker microVMs via KVM)
- Full OS environment: any language, any dependency, any system package
- Battle-tested at scale (AWS Lambda runs on Firecracker, trillions of invocations)
- E2B provides turnkey solution for LLM code execution

**Weaknesses:**
- Highest latency: ~125ms boot for Firecracker, seconds for Docker
- Infrastructure complexity: requires container runtime or KVM support
- Overkill for simple tools (string formatting, math, data transformation)
- Cost at scale (memory per container)

**Tools:** Firecracker, Docker, E2B SDK, Cloudflare Workers (V8 isolates, <5ms cold start)

---

## 6. Comparison Matrix

| Criterion | Rhai | WASM/Extism | gRPC Process | MCP Server | Container |
|-----------|------|-------------|--------------|------------|-----------|
| **Latency** | <1ms | 1-5ms | 5-20ms | 5-50ms | 125ms-5s |
| **Isolation** | Moderate | Strong | Strong | Varies | Strongest |
| **Multi-language** | No | Yes (compile) | Yes | Yes | Yes |
| **Dependencies** | None | Bundled in WASM | Full freedom | Full freedom | Full freedom |
| **Hot-reload** | Yes | Yes | No | Yes | No |
| **LLM can write** | Directly | Needs compile step | Needs compile+deploy | Needs server setup | Needs image build |
| **Complexity** | Low | Medium | High | Medium | High |
| **Binary size impact** | ~80KB | ~5MB (wasmtime) | ~2MB (tonic) | ~500KB | External |
| **Ecosystem maturity** | Good | Growing | Mature | Rapidly growing | Mature |

---

## 7. VISION.md Alignment

| Vision Concept | Plugin Impact |
|----------------|---------------|
| **Graph-native context** | Plugin definitions, invocations, and results are all graph nodes |
| **Tool calls as first-class citizens** | Dynamic tools get same `ToolCall →[Produced]→ ToolResult` provenance chain |
| **Background graph processing** | Plugin compilation/loading can be background tasks |
| **MCP for tool integration** | Phase 3 directly implements the vision's MCP strategy |
| **Dynamic system prompt construction** | Available plugins contribute to system prompt tool listing |
| **Multi-perspective compaction** | Plugin code/results can be compacted differently per context |

The vision explicitly mentions MCP as the tool integration protocol. Our Phase 3 (MCP servers) directly fulfills this. Phases 1-2 (Rhai, WASM) provide the local execution layer that MCP alone doesn't address — when an agent needs to create and run a tool in the same conversation turn, without deploying an external server.

---

## 8. Recommended Architecture

### Phase 1: Rhai Embedded Scripting (Local, Immediate)

**Goal:** An agent can write a Rhai script, register it as a tool, and other agents can call it — all within one session.

**Design:**
1. LLM generates a `PluginDefinition`: name, description, parameter schema, and Rhai source code
2. Orchestrator validates the schema and compiles the Rhai script via `rhai::Engine`
3. A new `Node::Plugin` is added to the graph (name, schema, source, status)
4. `ToolDefinition` is generated from the schema and added to `registered_tool_definitions()`
5. When invoked, `execute_tool()` matches the plugin name, deserializes arguments, and runs the Rhai script
6. Rhai engine configured with: max operations limit, max call stack depth, no filesystem/network access

**Sandboxing controls:**
- `Engine::set_max_operations(10_000)` — prevents infinite loops
- `Engine::set_max_call_levels(32)` — prevents stack overflow
- No registered filesystem or network functions — Rhai has none by default
- Host functions explicitly registered for controlled I/O (e.g., `read_file` wrapper with path validation)

### Phase 2: WASM Plugin Runtime (Multi-Language)

**Goal:** Tools written in any language that compiles to WASM, with strong sandboxing and hot-reload.

**Design:**
1. LLM generates source code in a supported language + a manifest (name, schema, language)
2. Background task compiles source to `.wasm` module (language-specific toolchain)
3. Extism loads the module with configured WASI capabilities
4. Plugin exposes a `call(input: JSON) -> JSON` function
5. Host provides controlled functions via Extism host functions (file access, HTTP, graph queries)
6. Modules cached on disk for reload across sessions

**WASM capability model (following Envoy's pattern):**
- Default: no filesystem, no network, no clock
- Explicit grants: `--allow-read=/workspace`, `--allow-net=api.example.com`
- Capabilities stored in the `Node::Plugin` graph node

### Phase 3: Remote Execution (MCP + Containers)

**Goal:** Tools that run on remote infrastructure, enabling full dependency freedom and heavy computation.

**Design:**
1. LLM generates a Dockerfile or MCP server spec
2. Background task builds and deploys (local Docker, Cloudflare Worker, or AWS Lambda)
3. Orchestrator connects as MCP client to the deployed server
4. Tools discovered via `tools/list`, invoked via `tools/call`
5. Results flow back through standard `ToolCall → ToolResult` graph provenance
6. Lifecycle managed via `Node::BackgroundTask` (deploy, health check, teardown)

---

## 9. Integration Design

### New Graph Nodes

```
Node::Plugin {
    id: Uuid,
    name: String,
    description: String,
    schema: ToolInputSchema,        // reuse existing type
    source: PluginSource,           // enum: Rhai { code }, Wasm { module_path }, Mcp { server_uri }
    status: PluginStatus,           // enum: Compiling, Ready, Failed, Disabled
    created_by: Uuid,               // agent/message that created it
    created_at: DateTime<Utc>,
}
```

### New Edge Kinds

```
EdgeKind::Created     // Message →[Created]→ Plugin (agent that wrote the tool)
EdgeKind::Implements  // Plugin →[Implements]→ Tool (plugin provides this tool)
```

### Plugin Registry Trait

```rust
trait PluginExecutor: Send + Sync {
    /// Execute the plugin with the given JSON input, return JSON output.
    async fn execute(&self, input: &str) -> Result<ToolResultContent, PluginError>;

    /// Validate that the plugin is ready to execute.
    fn is_ready(&self) -> bool;
}
```

Implementations: `RhaiExecutor`, `WasmExecutor`, `McpExecutor`.

### Dynamic Tool Registration

```rust
struct DynamicToolRegistry {
    /// Static tools (read_file, write_file, etc.)
    static_tools: Vec<ToolDefinition>,
    /// Runtime-registered plugins
    plugins: HashMap<String, (ToolDefinition, Box<dyn PluginExecutor>)>,
}

impl DynamicToolRegistry {
    fn register(&mut self, name: String, def: ToolDefinition, exec: Box<dyn PluginExecutor>);
    fn unregister(&mut self, name: &str);
    fn all_definitions(&self) -> Vec<ToolDefinition>;
    async fn execute(&self, name: &str, input: &str) -> Result<ToolResultContent, PluginError>;
}
```

### Modified Execution Flow

```
Agent writes tool code
    ↓
TaskMessage::PluginRegistered { plugin_id, definition }
    ↓
Main loop adds Plugin node to graph
    ↓
DynamicToolRegistry.register(name, def, executor)
    ↓
Next agent turn: tool list includes new plugin
    ↓
LLM invokes plugin → ToolCall node → Plugin executes → ToolResult node
    ↓
Full provenance: Message →[Created]→ Plugin →[Implements]→ Tool
                  Message →[Invoked]→ ToolCall →[Produced]→ ToolResult
```

### `execute_tool()` Changes

The current `match` on `ToolCallArguments` stays for static tools. The `Unknown` arm changes from returning an error to checking the `DynamicToolRegistry`:

```rust
ToolCallArguments::Unknown { tool_name, raw_json } => {
    if let Some(result) = registry.execute(tool_name, raw_json).await {
        result
    } else {
        ToolExecutionResult { content: ToolResultContent::text("Unknown tool"), is_error: true }
    }
}
```

---

## 10. Red/Green Team Audit

### Green Team (Factual Verification)

30+ claims verified. All major factual claims confirmed against authoritative sources. Two corrections applied:

1. **Rhai GitHub stars**: corrected from "3.5k+" to "~5,000"
2. **Rhai API method**: corrected `set_max_call_stack_depth()` to `set_max_call_levels()` (the actual Rhai API name)

**Verified claims (sample):**
- Rhai: sandboxed by default, ~2x slower than Python 3 — confirmed by official docs
- WASI 0.2 stable January 2024 — confirmed (Bytecode Alliance vote January 25, 2024)
- WASI 0.3 adds native async — confirmed (future/stream types, async function signatures)
- Envoy WASM: <2x CPU-bound, 10-20% network filters — confirmed by Solo.io benchmarks
- Extism built on Wasmtime, supports listed languages — confirmed
- MCP: 97M+ monthly SDK downloads, 10,000+ servers, Linux Foundation December 2025 — all confirmed
- Firecracker: ~125ms boot, powers Lambda/Fargate — confirmed
- Cloudflare Workers: <5ms cold start, V8 isolates — confirmed
- HashiCorp go-plugin: 4+ years, created for Packer — confirmed

### Red Team (Challenging Recommendations)

**Critical issues identified, prioritized by severity:**

**C1: Rhai may be the wrong language for LLM code generation.**
LLMs have far more training data on Lua, JavaScript, and Python than Rhai. Starting with Rhai means accepting significantly more failed tool generations. Alternatives not fully evaluated: **Lua** (via `mlua` — used by Factorio, Neovim, Nginx; LLMs write better Lua), **JavaScript** (via `boa_engine` — pure Rust, ~100KB, ES6+; or QuickJS), **Python** (via RustPython — pure Rust, most popular LLM target language). **Recommendation:** Evaluate LLM code quality across Rhai, Lua, and JavaScript before committing to Phase 1 language.

**C2: Resource exhaustion sandboxing is incomplete — no memory limits.**
`set_max_operations()` prevents infinite loops but not memory exhaustion. A script like `let a = []; loop { a.push(range(0, 1000000)); }` allocates unbounded memory. Missing: per-engine heap limits, OOM recovery strategy. WASM Phase 2 has bounded linear memory (default 1GB) but no mitigation for exhausting it. **Recommendation:** Implement process-level rlimits or a custom allocator with hard limits for Rhai; configure WASM memory limits via Wasmtime.

**C3: Three-phase approach may be over-engineered — MCP-only could work from day 1.**
If MCP servers run locally via stdio, latency is ~1-5ms (not the 50ms cited for network MCP). This eliminates the primary advantage of embedded scripting. One execution model (MCP) is simpler to maintain than three (Rhai + WASM + MCP). **Counter:** MCP servers require packaging and subprocess management even for trivial tools; embedded scripting is genuinely simpler for "write and run in the same turn." **Recommendation:** Prototype both approaches and compare developer experience.

**C4: Host function callback security is underspecified.**
Rhai scripts call host-registered functions like `read_file(path)`, but nothing prevents `read_file("/etc/passwd")` or path traversal via `../../`. Missing: capability-based access controls (whitelist of allowed paths/domains), per-plugin permission grants, escalation prevention. **Recommendation:** Adopt Factorio's staged API model — plugins declare required capabilities at registration, orchestrator validates and grants specific permissions.

**C5: Plugin persistence and cross-session behavior is unaddressed.**
Where are compiled WASM modules cached? How are name collisions across sessions handled? If Agent A creates `summarize()` in session 1, does session 2 see it? What's the lifecycle (auto-expire? manual delete?). **Recommendation:** Define a plugin lifecycle state machine (Writing → Validating → Compiling → Ready → Disabled → Deleted) with clear session scoping rules.

**C6: No plugin testing or validation strategy.**
An LLM generates code that compiles but produces wrong output. No mechanism for test execution, schema validation against actual behavior, or automatic rollback on failure. **Recommendation:** Require plugins to include at least one test case (input → expected output) that runs before registration.

**C7: Plugin composition (tool-calling-tool) is not addressed.**
Can a Rhai plugin call another plugin? Can a WASM tool invoke an MCP server? Circular dependency detection? Error propagation in chains? **Recommendation:** Start with no cross-plugin calls (Phase 1), add explicit dependency declaration (Phase 2).

**C8: Missing alternatives — Deno V8 isolates, Starlark, WASIX.**
Deno provides built-in sandboxing + excellent LLM code quality (JavaScript/TypeScript). Starlark (Python-like, used by Bazel/Buck) is safe by design. WASIX adds POSIX compatibility to WASM, making it more practical than strict WASI. These aren't evaluated in the options analysis.

**C9: Rate limiting and quota management absent.**
Nothing prevents a plugin from being called 10,000 times/second or consuming unbounded compute. Missing: per-plugin rate limits, per-agent quotas, global resource budgets, cost tracking for remote execution.

**C10: Observability gaps.**
No execution timing, error attribution, resource usage tracking, or audit trail beyond graph provenance. `Node::Plugin` needs metrics fields.

### Code Accuracy (File References)

All 12 references verified against actual source files — **100% accurate**:

- `src/tool_executor/mod.rs:32-109` — `registered_tool_definitions()`: 4 tools, confirmed
- `src/tool_executor/mod.rs:112-139` — `execute_tool()`: match on `ToolCallArguments`, confirmed
- `src/tool_executor/mod.rs:142-155` — `spawn_tool_execution()`: tokio task, confirmed
- `src/graph/tool_types.rs:15-42` — `ToolCallArguments`: 7 variants including `Unknown`, confirmed
- `src/llm/tool_types.rs:6-16` — `ToolDefinition` and `ToolInputSchema`, confirmed
- `src/app/agent_loop.rs:53-124` — `run_agent_loop()`: iterates `max_tool_loop_iterations`, confirmed
- `src/graph/mod.rs:83` — `Node` enum start, confirmed
- `EdgeKind::Invoked` and `EdgeKind::Produced` at `src/graph/mod.rs:66-67`, confirmed
- `ToolResultContent` at `src/graph/tool_types.rs:150-155`, confirmed

---

## 11. Sources

### Real-World Plugin Systems
- [Neovim Lua Plugin Docs](https://neovim.io/doc/user/lua-plugin.html)
- [VS Code Extension Host Architecture](https://code.visualstudio.com/api/advanced-topics/extension-host)
- [IntelliJ Plugin Extensions & ClassLoaders](https://plugins.jetbrains.com/docs/intellij/plugin-class-loaders.html)
- [Factorio Data Lifecycle & Lua API](https://lua-api.factorio.com/latest/auxiliary/data-lifecycle.html)
- [Paper (Minecraft) Plugin Docs](https://docs.papermc.io/paper/dev/getting-started/paper-plugins/)
- [Godot GDExtension System](https://docs.godotengine.org/en/stable/tutorials/scripting/gdextension/index.html)
- [Chrome Manifest V3 Extensions](https://developer.chrome.com/docs/extensions/reference/manifest/sandbox)

### Infrastructure Plugin Systems
- [Terraform Plugin Protocol](https://developer.hashicorp.com/terraform/plugin/terraform-plugin-protocol)
- [HashiCorp go-plugin](https://github.com/hashicorp/go-plugin)
- [Envoy WASM Architecture](https://www.envoyproxy.io/docs/envoy/latest/intro/arch_overview/advanced/wasm)
- [Envoy WASM Performance](https://www.solo.io/blog/the-state-of-webassembly-in-envoy-proxy/)
- [Grafana Plugin Frontend Sandbox](https://grafana.com/docs/grafana/latest/administration/plugin-management/plugin-frontend-sandbox/)
- [Redis Module System](https://redis.io/docs/latest/develop/reference/modules/)
- [Traefik Plugin Development](https://plugins.traefik.io/create)
- [Nginx lua-nginx-module (OpenResty)](https://github.com/openresty/lua-nginx-module)

### WASM Runtimes & Plugin Frameworks
- [Wasmtime vs Wasmer Comparison](https://wasmruntime.com/en/blog/wasmtime-vs-wasmer-2026)
- [Extism WASM Plugin Framework](https://lib.rs/crates/extism)
- [WASM Component Model](https://blog.nginx.org/blog/wasm-component-model-part-1)
- [Wasmtime Security Model](https://docs.wasmtime.dev/security.html)
- [Proxy-Wasm Spec](https://github.com/proxy-wasm/spec/blob/main/docs/WebAssembly-in-Envoy.md)

### Embedded Scripting
- [Rhai Embedded Scripting for Rust](https://rhai.rs/)
- [Embedded Scripting Language Comparison](https://caiorss.github.io/C-Cpp-Notes/embedded_scripting_languages.html)
- [Microsecond Transforms: Fast Sandboxes for User Code](https://blog.sequinstream.com/why-we-built-mini-elixir/)

### AI Agent Tool Systems
- [MCP Architecture](https://modelcontextprotocol.io/docs/learn/architecture)
- [A Year of MCP: From Internal Experiment to Industry Standard](https://www.pento.ai/blog/a-year-of-mcp-2025-review)
- [MCP-Zero: Active Tool Discovery](https://arxiv.org/abs/2506.01056)
- [OpenAI Function Calling](https://platform.openai.com/docs/guides/function-calling)
- [Semantic Kernel Plugins](https://learn.microsoft.com/en-us/semantic-kernel/concepts/plugins/)
- [Code Execution with MCP (Anthropic)](https://www.anthropic.com/engineering/code-execution-with-mcp)

### Automation Platforms
- [n8n Custom Node Development](https://docs.n8n.io/integrations/creating-nodes/overview/)
- [Dagger SDK Architecture](https://docs.dagger.io/)
- [GitHub Actions Custom Actions](https://docs.github.com/actions/creating-actions/about-custom-actions)
- [Jupyter Kernel Architecture](https://docs.jupyter.org/en/latest/projects/kernels.html)

### Container Isolation
- [Firecracker vs gVisor](https://northflank.com/blog/firecracker-vs-gvisor)
- [E2B Enterprise AI Agent Cloud](https://e2b.dev/)
- [Cloudflare Workers Performance](https://blog.cloudflare.com/serverless-performance-comparison-workers-lambda/)
- [AWS Serverless MCP Server](https://aws.amazon.com/blogs/compute/introducing-aws-serverless-mcp-server-ai-powered-development-for-modern-applications/)

### Rust Plugin Ecosystem
- [Plugins in Rust: Dynamic Loading](https://nullderef.com/blog/plugin-dynload/)
- [Plugins in Rust: ABI Stable](https://nullderef.com/blog/plugin-abi-stable/)

### Versioning & Schemas
- [Towards the Versioning of LLM-Agent-Based Software](https://dl.acm.org/doi/pdf/10.1145/3696630.3728714)
- [ScaleMCP: Dynamic Tool Selection](https://arxiv.org/abs/2505.06416)
- [ToolMaker: LLM Agents Making Agent Tools](https://arxiv.org/abs/2502.11705)
