# 17 — LiteLLM Provider Integration

> **Date:** 2026-03-13
> **Summary:** Comprehensive analysis of LiteLLM capabilities for multi-provider LLM access, cost tracking, caching, guardrails, and more — evaluated against the context-orchestrator's current Anthropic-only architecture.

---

## 1. Executive Summary

The context-orchestrator currently supports only Anthropic's Messages API via a custom SSE parser in `src/llm/anthropic.rs`. LiteLLM is an open-source AI gateway (100+ providers, 8ms P95 overhead at 1,170 RPS) that exposes an OpenAI-compatible proxy, enabling unified access to Anthropic, OpenAI, DeepSeek, Ollama, Bedrock, Vertex, and dozens more — with built-in cost tracking, caching, rate limiting, guardrails, and observability. The recommended approach is a native `OpenAiCompatibleProvider` in `src/llm/openai_compat.rs` using reqwest + manual SSE parsing (~250 lines, zero new dependencies), which can point directly at DeepSeek, Ollama, or any OpenAI-compatible endpoint — including a LiteLLM proxy if the user chooses to run one. This preserves the direct Anthropic provider for latency-sensitive foreground use while routing background tasks (compaction, rating, classification) to cheaper models. LiteLLM remains a powerful optional upgrade for users who want centralized cost tracking, caching, and multi-provider routing, but is not required for basic multi-provider support.

---

## 2. Current Architecture & Gap Analysis

### What Exists

| Component | Location | Description |
|-----------|----------|-------------|
| Provider trait | `src/llm/mod.rs:61-75` | `LlmProvider` with `chat()` (streaming) and `count_tokens()` |
| Anthropic impl | `src/llm/anthropic.rs` (375 lines) | Custom SSE parser, tool_use accumulation, retry logic |
| Config | `src/config.rs` (80 lines) | All fields `anthropic_*`, no provider abstraction |
| Tool types | `src/llm/tool_types.rs` | Provider-agnostic `ToolDefinition`, `ChatContent`, `ContentBlock` |
| Error handling | `src/llm/error.rs` | `ApiError` enum with retryable/auth/bad_request variants |
| Agent loop | `src/app/agent_loop.rs` (285 lines) | Provider-agnostic; takes `Arc<dyn LlmProvider>` |
| Background LLM | `src/llm/mod.rs:108-136` | `background_llm_call()` with semaphore, non-streaming |
| Streaming | `src/app/agent_streaming.rs` | Retry wrapper, think splitter, throttled UI updates |

### What's Good (Reusable)

- `LlmProvider` trait is clean and protocol-agnostic — `StreamChunk` hides API specifics
- `ChatMessage`, `ChatContent`, `ToolDefinition` types are provider-neutral
- Agent loop and background call infrastructure don't depend on Anthropic internals
- `RetryConfig` and `with_retry()` logic are generic

### Gaps

| Gap | Impact | VISION.md Ref |
|-----|--------|---------------|
| Only Anthropic provider | Cannot use DeepSeek/Ollama for background processing | §5.4 |
| Config hardcoded to `anthropic_*` | No way to configure alternative providers | §5.4 |
| No cost tracking | Cannot optimize spend across models | §5.5, §2.3 |
| No provider fallbacks | Single point of failure | §8.1 |
| No caching layer | Redundant API calls for repeated context | §5.4 |
| No rate limiting awareness | Background tasks can starve foreground | §4.3 |
| No multi-model routing | Can't route by cost/latency/capability | §5.4 |
| SSE parser is Anthropic-specific | OpenAI SSE format uses different structure | — |
| Tool schema format is Anthropic-wire | OpenAI uses `parameters` not `input_schema` | — |

---

## 3. Requirements

Derived from VISION.md §5.4, §5.5, §4.3, and the current architecture:

### Must Have
1. **Multi-provider support** — at minimum Anthropic (direct) + OpenAI-compatible (direct or via proxy)
2. **Streaming** — SSE streaming for both foreground and background calls
3. **Tool use** — function calling through the proxy for agent loops
4. **Provider selection in config** — choose provider per use case (foreground vs background)
5. **Preserve direct Anthropic** — keep the existing provider for latency-sensitive paths

### Should Have
6. **Cost tracking** — per-call cost reporting, aggregated by model/task type
7. **Provider fallbacks** — if primary fails, try secondary
8. **Prompt caching passthrough** — Anthropic cache_control through LiteLLM
9. **Token counting** — model-specific token counts

### Nice to Have
10. **Caching** — response caching for repeated queries (via LiteLLM Redis/semantic cache)
11. **Rate limiting awareness** — respect provider rate limits, prioritize foreground
12. **Observability callbacks** — Langfuse/OTEL integration via LiteLLM
13. **Guardrails** — PII masking, content filtering via LiteLLM middleware
14. **Batch API** — 50% cost reduction for non-urgent background tasks
15. **MCP gateway** — unified MCP tool access through LiteLLM

---

## 4. Options Analysis

### Option A: async-openai Crate + LiteLLM Proxy

**Description:** Create `OpenAiCompatibleProvider` using the `async-openai` crate (v0.33.1, ~400 dependents). Point it at a LiteLLM proxy running locally or remotely. The crate supports custom base URLs via `OpenAIConfig::new().with_api_base()`.

**Strengths:**
- Battle-tested crate with full streaming, tool calling, embeddings support
- Works with LiteLLM, vLLM, Ollama, or any OpenAI-compatible server
- Single implementation covers 100+ providers through the proxy
- LiteLLM provides built-in cost tracking, caching, rate limiting

**Weaknesses:**
- Requires running a LiteLLM proxy server (Docker + PostgreSQL + Redis for full features)
- `async-openai` pulls in `reqwest-eventsource`, `tokio-tungstenite`, `derive_builder` — heavy for what we need
- Adds large type surface area (Assistants, Threads, Audio, Images, Fine-tuning — all unused)
- LiteLLM has documented memory leaks in production, recommends `--max_requests_before_restart`
- LiteLLM ships multiple releases/day with occasional breaking changes (800+ open issues)
- LiteLLM stores all provider API keys in plaintext config, with documented key leak bugs
- Extra network hop adds ~8ms P95 / 13ms P99 overhead; cold start is 3-10 seconds

### Option B: Native OpenAI-Compatible Provider (No Proxy, No New Deps)

**Description:** Build a single `OpenAiCompatibleProvider` using the existing `reqwest` + manual SSE parsing (~250 lines). DeepSeek, Ollama, and any OpenAI-compatible endpoint work directly — just change the base URL. No proxy required.

**Strengths:**
- Zero new dependencies — uses existing `reqwest`, `serde`, `futures`
- Zero operational infrastructure — no Docker, no PostgreSQL, no Redis, no Python
- OpenAI SSE format is *simpler* than Anthropic's (data-only lines, no typed events)
- Covers DeepSeek (direct, `https://api.deepseek.com/v1`), Ollama (direct, `localhost:11434/v1`), OpenAI
- ~250 lines: request struct (~30), SSE parsing (~100), provider glue (~40), response types (~40), tool format (~30)
- Same base URL trick works with LiteLLM/vLLM/OpenRouter if user wants a proxy later
- Full control, no translation fidelity issues
- Follows the same pattern as the existing Anthropic provider (proven architecture)

**Weaknesses:**
- Must maintain SSE parser for OpenAI format (though simpler than existing Anthropic parser)
- No built-in cost tracking, caching, or rate limiting — must use response `usage` fields
- Fallback routing is manual (~20 lines: try A, on error try B)
- Adding truly exotic providers (Bedrock, Vertex) would require per-provider work

### Option C: `genai` Crate (Native Multi-Provider)

**Description:** Use the `genai` Rust crate (v0.5.0) which provides native multi-provider support for OpenAI, Anthropic, Gemini, Groq, Cohere, etc. with a unified API.

**Strengths:**
- Pure Rust, no external proxy
- Single crate covers multiple providers natively
- Supports chat completions and tool calling
- Active development (maintained by Jeremy Chone)

**Weaknesses:**
- Less mature than `async-openai` (fewer dependents, smaller community)
- The maintainer himself recommends `async-openai` for comprehensive OpenAI API needs
- Doesn't cover cost tracking, caching, or rate limiting
- Would still need LiteLLM for background processing cost optimization
- Replaces the existing Anthropic provider rather than complementing it

### Option D: LiteLLM for Everything (Replace Anthropic Provider)

**Description:** Route ALL calls through LiteLLM, including foreground conversation. Remove the direct Anthropic provider entirely.

**Strengths:**
- Single provider implementation to maintain
- Unified cost tracking, caching, rate limiting for all calls
- Simplest codebase — one provider, one format
- LiteLLM handles Anthropic prompt caching passthrough

**Weaknesses:**
- Adds latency to every foreground call (~8ms P95, worse on cold starts)
- Single point of failure — LiteLLM proxy down = entire app down
- Loses Anthropic-specific SSE features (extended thinking blocks, `anthropic_thinking` tags)
- Forces users to run a separate service even for basic Anthropic-only usage
- Current Anthropic provider is working and battle-tested — replacing it adds risk

### Option E: Native Provider + Optional LiteLLM (Recommended)

**Description:** Build a native `OpenAiCompatibleProvider` (Option B) as the immediate step. LiteLLM becomes an optional infrastructure choice — users who want centralized cost tracking, caching, guardrails, and observability can run LiteLLM and point the same provider at it. Users who just want DeepSeek/Ollama for background tasks connect directly.

**Strengths:**
- Works out of the box with zero infrastructure (direct to DeepSeek/Ollama)
- LiteLLM is opt-in, not required — respects VISION.md's single-developer target
- Same `OpenAiCompatibleProvider` code works for both direct and proxy paths
- No new crate dependencies
- Graduated complexity: start simple, add LiteLLM when enterprise features are needed
- Avoids Python/Docker dependency for basic multi-provider support

**Weaknesses:**
- Users who want LiteLLM features must set it up themselves
- No built-in cost dashboard (but response `usage` fields + static pricing table covers basics)
- Fallback routing requires Rust code, not proxy config

---

## 5. Comparison Matrix

| Criteria | A: async-openai + LiteLLM | B: Native OpenAI-compat | C: genai Crate | D: LiteLLM Only | **E: Native + Optional LiteLLM** |
|----------|--------------------------|------------------------|----------------|-----------------|--------------------------------|
| **Implementation effort** | Low (~200 lines + heavy dep) | Low (~250 lines, no new deps) | Medium (refactor) | Low (replace) | **Low (~250 lines, no new deps)** |
| **Provider coverage** | 100+ via proxy | 3-4 directly | ~10 natively | 100+ via proxy | **3-4 direct, 100+ via optional proxy** |
| **Foreground latency** | Direct Anthropic (0ms) | Direct (0ms) | Direct (0ms) | +8ms P95 | **Direct (0ms)** |
| **Background cost optimization** | Yes | Yes | Yes | Yes | **Yes** |
| **Cost tracking** | Built-in via LiteLLM | Response `usage` fields | Must build | Built-in | **Response `usage` + optional LiteLLM** |
| **Caching** | Built-in (Redis/semantic) | Must build | Must build | Built-in | **Optional via LiteLLM** |
| **Rate limiting** | Built-in | Semaphore (exists) | Must build | Built-in | **Semaphore + optional LiteLLM** |
| **Fallback routing** | Built-in | ~20 lines Rust | Partial | Built-in | **~20 lines Rust + optional proxy** |
| **External dependency** | LiteLLM + PostgreSQL + Redis | None | None | LiteLLM stack | **None (LiteLLM optional)** |
| **Anthropic feature fidelity** | Full (direct kept) | Full | Partial | Reduced | **Full** |
| **Operational burden** | High (proxy infra) | None | None | High | **None (optional high)** |
| **New Rust dependencies** | async-openai + transitive | None | genai | async-openai | **None** |

---

## 6. VISION.md Alignment

| VISION.md Goal | How LiteLLM Helps |
|---------------|-------------------|
| §5.4: DeepSeek V3.2 for background compaction at $0.07/$0.14 per M tokens | Route through LiteLLM with `deepseek/deepseek-chat` model prefix |
| §5.4: Ollama for local summarization at $0 | Route through LiteLLM with `ollama/qwen2.5` model prefix |
| §5.4: Multiple LLM providers (Anthropic, OpenAI, DeepSeek, Ollama) | LiteLLM supports all four plus 100+ more |
| §5.5: $24/month total cost budget | LiteLLM's cost tracking + budget controls per key/team/model |
| §5.4: Prompt caching (90% cost reduction) | LiteLLM passes through Anthropic `cache_control` annotations |
| §5.4: Batch API (50% cost reduction) | LiteLLM supports batch for OpenAI, Azure, Vertex, Bedrock |
| §4.3: Background processing with rate limiting | LiteLLM's TPM/RPM limits + priority routing |
| §4.3: Batch API for non-urgent processing | LiteLLM `/batches` endpoint |
| §4.4: Cascade evaluation (cheap model first) | LiteLLM model groups with fallback chains |
| §4.8: MCP as the tool protocol | LiteLLM MCP gateway with access control |
| §8.1: Background races with foreground | LiteLLM rate limiting separates foreground/background |

**Deviation:** VISION.md assumes direct API calls per provider. Using a proxy adds an architectural component but reduces per-provider implementation work dramatically.

---

## 7. Recommended Architecture (Option E)

### Phase 1: Native OpenAI-Compatible Provider (Immediate)

Add `OpenAiCompatibleProvider` in `src/llm/openai_compat.rs` (~250 lines, zero new deps):
- Implements `LlmProvider` trait using existing `reqwest` + manual SSE parsing
- Configurable base URL — defaults vary by target:
  - DeepSeek: `https://api.deepseek.com/v1`
  - Ollama: `http://localhost:11434/v1`
  - LiteLLM (optional): `http://localhost:4000`
- OpenAI SSE format is simpler than Anthropic's: `data:` lines only, `data: [DONE]` termination
- Tool use via OpenAI function calling format (`parameters`, `tool_calls`, role `"tool"`)
- Token counting via response `usage` fields (prompt_tokens, completion_tokens)

Refactor `AppConfig` to support provider selection:
```rust
pub enum ProviderKind { Anthropic, OpenAiCompatible }

pub struct ProviderConfig {
    pub kind: ProviderKind,
    pub base_url: String,
    pub api_key: Option<String>,
    pub model: String,
    pub max_tokens: u32,
}

pub struct AppConfig {
    pub foreground: ProviderConfig,  // default: Anthropic direct
    pub background: ProviderConfig,  // default: OpenAI-compatible (DeepSeek direct)
    // ... rest unchanged
}
```

### Phase 2: Cost Tracking (Near-term)

- Parse `usage` fields from all provider responses (both Anthropic and OpenAI-compat return these)
- Maintain a static pricing table per model (sourced from LiteLLM's open-source pricing DB)
- Store cost per `ToolCall` and `Message` node in the graph (new field)
- Aggregate cost per conversation, per model, per task type
- Display in TUI status bar or context panel
- Optionally parse `x-litellm-response-cost` header if user runs LiteLLM proxy

### Phase 3: LiteLLM as Optional Upgrade (Future)

For users who need enterprise features, document how to run LiteLLM and point the existing provider at it:
- **Centralized cost dashboard**: LiteLLM's `/global/spend/report` endpoint
- **Redis caching**: semantic cache for repeated background queries
- **Fallback routing**: model groups with automatic failover (Claude → GPT-4 → DeepSeek)
- **Observability**: Langfuse/OTEL callbacks for debugging context construction quality
- **Guardrails**: Presidio PII masking for sensitive codebases
- **Batch API**: `/batches` endpoint for 50% cost reduction on background compaction
- **Rate limiting**: TPM/RPM per foreground vs background to prevent starvation
- **MCP gateway**: unified MCP tool access with per-key access control

**Key insight:** The same `OpenAiCompatibleProvider` code works for both direct and LiteLLM paths. The only difference is the base URL.

---

## 8. Integration Design

### Provider Instantiation Flow

```
main.rs
  ├─ load AppConfig
  ├─ match foreground.kind
  │   ├─ Anthropic → AnthropicProvider::new(base_url, api_key)
  │   └─ OpenAiCompatible → OpenAiCompatibleProvider::new(base_url, api_key)
  ├─ match background.kind
  │   ├─ Anthropic → AnthropicProvider::new(...)
  │   └─ OpenAiCompatible → OpenAiCompatibleProvider::new(...)
  └─ App::new(foreground_provider, background_provider, ...)
```

### Data Flow

**Direct connection (default, no proxy):**
```
context-orchestrator                    DeepSeek / Ollama / OpenAI
       │                                         │
       │  POST /v1/chat/completions               │
       │  model: "deepseek-chat"                  │
       │ ──────────────────────────────────────>   │
       │                                          │
       │  <──── SSE stream (OpenAI format) ────── │
       │  usage: { prompt_tokens, completion_tokens } │
```

**With optional LiteLLM proxy (for enterprise features):**
```
context-orchestrator          LiteLLM Proxy              Provider API
       │                          │                          │
       │  POST /chat/completions  │                          │
       │  model: "deepseek/..."   │                          │
       │ ─────────────────────>   │                          │
       │                          │  POST (native format)    │
       │                          │ ─────────────────────>   │
       │                          │                          │
       │                          │  <─── SSE stream ──────  │
       │  <── SSE (OpenAI fmt) ── │                          │
       │  x-litellm-response-cost │                          │
```

### Key Types

```rust
// src/llm/openai_compat.rs
pub struct OpenAiCompatibleProvider {
    client: reqwest::Client,
    base_url: String,
    api_key: Option<String>,
    default_model: String,
}

#[async_trait]
impl LlmProvider for OpenAiCompatibleProvider {
    async fn chat(...) -> Result<Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>>> {
        // Convert ChatMessage → OpenAI request body (serde structs, not Value)
        // Convert ToolDefinition → OpenAI function format (parameters, not input_schema)
        // POST to {base_url}/chat/completions with stream: true
        // Parse SSE: data-only lines, choices[0].delta.content, data: [DONE]
        // Map to StreamChunk::TextDelta / ToolUse / Done / Error
    }

    async fn count_tokens(...) -> Result<u32> {
        // Use response.usage.prompt_tokens from previous call
        // Or estimate locally (4 chars ≈ 1 token heuristic)
    }
}
```

### Config (environment variables + TOML)

```toml
# ~/.context-manager/config.toml

[foreground]
provider = "anthropic"
base_url = "https://api.anthropic.com"
model = "claude-sonnet-4-6"
max_tokens = 4096

[background]
provider = "openai_compatible"
base_url = "https://api.deepseek.com/v1"  # Direct to DeepSeek, no proxy needed
api_key = "sk-deepseek-..."
model = "deepseek-chat"
max_tokens = 1024

# Alternative: local Ollama
# [background]
# provider = "openai_compatible"
# base_url = "http://localhost:11434/v1"
# model = "qwen2.5:14b"
# max_tokens = 1024

# Alternative: LiteLLM proxy (for enterprise features)
# [background]
# provider = "openai_compatible"
# base_url = "http://localhost:4000"
# api_key = "sk-litellm-..."
# model = "deepseek/deepseek-chat"
# max_tokens = 1024
```

### New Dependencies

None. Uses existing `reqwest`, `serde`, `futures`, `async-trait`.

---

## 9. Red/Green Team

### Green Team (Claim Validation)

| # | Claim | Verdict | Notes |
|---|-------|---------|-------|
| 1 | async-openai v0.33.1 exists with streaming + tool calling | CONFIRMED | Features verified on lib.rs |
| 2 | LiteLLM 100+ providers | CONFIRMED | Official branding says "100+", actual list is ~130-140 |
| 3 | LiteLLM 8ms P95 overhead | CONFIRMED | At 1,170 RPS with 4 instances. P99 is 13ms. Total endpoint P95 is 150ms |
| 4 | LiteLLM prompt caching passthrough | CONFIRMED | Passes `cache_control: {"type": "ephemeral"}` to Anthropic, returns `cache_creation_input_tokens` |
| 5 | genai v0.5.0, maintainer recommends async-openai | CONFIRMED | README: "If you require a complete client API, consider async-openai" |
| 6 | tiktoken-rs | CONFIRMED | Latest is v0.9.1 (not v0.6 as originally stated) |
| 7 | LiteLLM MCP gateway | CONFIRMED | Supports SSE, HTTP, stdio transports with per-key access control |
| 8 | LiteLLM Responses API | CONFIRMED | Since v1.63.8+, with automatic bridging for non-native providers |
| 9 | LiteLLM A2A protocol | CONFIRMED | JSON-RPC 2.0 at `/a2a/{agent_name}/message/send` |
| 10 | `x-litellm-response-cost` header | CONFIRMED | Primary per-request cost reporting mechanism |
| 11 | Anthropic OpenAI compat lacks prompt caching | CONFIRMED | Official docs: "Prompt caching is not supported" in compatibility layer |
| 12 | OpenAI SSE: data-only lines | CONFIRMED for /chat/completions | Note: OpenAI Responses API uses `event:` + `data:` lines |

### Red Team (Challenges to Recommendation)

**R1: LiteLLM alternatives not evaluated.** The initial draft failed to consider Helicone (Rust-based, open-source, 8ms P50), OpenRouter (zero-setup SaaS, 5% markup), Portkey (enterprise controls), or Cloudflare AI Gateway (free edge distribution). For a Rust project, Helicone deserved evaluation as the first proxy candidate. **Resolution:** Recommendation changed from "LiteLLM required" to "native direct + LiteLLM optional" (Option E), making the proxy choice a user decision.

**R2: async-openai is unnecessary.** The project already has `reqwest` with streaming, `serde`, and a working SSE parser (Anthropic's, which is *harder* than OpenAI's). Adding async-openai pulls in `reqwest-eventsource`, `tokio-tungstenite`, `derive_builder`, and a massive type surface for Assistants/Threads/Audio/Images/Fine-tuning — all unused. **Resolution:** Recommendation changed to native `reqwest` + manual SSE parsing, zero new dependencies.

**R3: Native adapters were underestimated.** The initial draft claimed "400+ lines/provider" and "Very High effort." In reality: the shared infrastructure (`error.rs`, `retry.rs`, `tool_types.rs`) is reusable. An OpenAI-compat provider needs ~250 lines total. OpenAI SSE is simpler than Anthropic SSE (data-only lines vs typed events). DeepSeek and Ollama speak OpenAI natively — a single provider covers 3+ endpoints. **Resolution:** Option B rewritten to reflect actual effort; Option E recommended.

**R4: LiteLLM operational burden is severe for a single-developer TUI.** Full LiteLLM deployment requires Docker, PostgreSQL (request logs at 100K/day cause slowdowns), Redis (rate limiting/caching), config management, and monitoring. LiteLLM has documented memory leaks (12GB consumption, requires `--max_requests_before_restart`). Multiple releases per day with occasional breaking changes. 800+ open GitHub issues. This is enterprise infrastructure, not a lightweight proxy. **Resolution:** LiteLLM made optional, not required. Native direct connections work out of the box.

**R5: Most LiteLLM "free" features are enterprise-irrelevant.** Team budgets, PII masking, A/B testing, Langfuse integration, multi-key management — none apply to a single developer running a TUI. The only genuinely useful features are fallback routing (~20 lines of Rust) and cost tracking (available from response `usage` fields). **Resolution:** Enterprise features moved to Phase 3 as optional upgrade documentation.

**R6: LiteLLM feature translation is lossy.** Documented bugs: prompt caching + extended thinking fails (GitHub #18950), cache-enabled requests via Vertex always fail (#14293), API keys leak into logs (#15799). LiteLLM model naming (`deepseek/deepseek-chat`) is proprietary — switching proxies requires renaming all models. **Resolution:** Direct connections avoid translation layer entirely. LiteLLM-specific model names only appear in LiteLLM config, not in app code.

**R7: Cold start latency.** The initial draft cited 8ms P95 without noting: this is warm-state, 4-instance, at scale. A single-instance cold start (Docker boot + Python import + first request) is 3-10 seconds. P99 is 13ms, not 8ms. For a TUI that launches on demand, this matters. **Resolution:** Latency discussion expanded; direct connections eliminate this entirely.

### Code Accuracy

| Reference | Status | Fix |
|-----------|--------|-----|
| `src/llm/mod.rs:12-23` for LlmProvider | WRONG | Fixed to `src/llm/mod.rs:61-75` |
| `src/llm/anthropic.rs` (376 lines) | OFF BY 1 | Fixed to 375 lines |
| `src/config.rs` (81 lines) | OFF BY 1 | Fixed to 80 lines |
| `src/app/agent_loop.rs` (286 lines) | OFF BY 1 | Fixed to 285 lines |
| `src/tools.rs` for `background_llm_call()` | WRONG FILE | Fixed to `src/llm/mod.rs:108-136` |
| `src/llm/tool_types.rs` types | CORRECT | `ToolDefinition`, `ChatContent`, `ContentBlock` verified |
| `src/llm/error.rs` ApiError variants | CORRECT | Retryable, Auth, BadRequest, Network, Timeout verified |
| `src/app/agent_streaming.rs` exists | CORRECT | 272 lines |
| `StreamChunk` variants | CORRECT | TextDelta, ToolUse, Done, Error |
| `ToolDefinition::to_api()` | CORRECT | `src/llm/tool_types.rs:72-95` |
| `registered_tool_definitions()` | CORRECT | Registers read_file, write_file, list_directory, search_files |

---

## 10. Sources

### LiteLLM
- [LiteLLM Documentation](https://docs.litellm.ai/)
- [LiteLLM GitHub](https://github.com/BerriAI/litellm) — 100+ providers, 8ms P95 proxy overhead
- [LiteLLM Supported Endpoints](https://docs.litellm.ai/docs/supported_endpoints)
- [LiteLLM Proxy Benchmarks](https://docs.litellm.ai/docs/benchmarks) — 1,170 RPS at 8ms P95 / 13ms P99 (4-instance)
- [LiteLLM Prompt Caching](https://docs.litellm.ai/docs/completion/prompt_caching) — Anthropic cache_control passthrough
- [LiteLLM Cost Tracking](https://docs.litellm.ai/docs/proxy/cost_tracking) — `x-litellm-response-cost` header
- [LiteLLM Caching](https://docs.litellm.ai/docs/caching/all_caches) — 7 backends including semantic cache
- [LiteLLM Guardrails](https://docs.litellm.ai/docs/proxy/guardrails)
- [LiteLLM MCP Gateway](https://docs.litellm.ai/docs/mcp) — per-key access control, SSE/HTTP/stdio
- [LiteLLM Rate Limiting](https://docs.litellm.ai/docs/proxy/rate_limiting)
- [LiteLLM Batch API](https://docs.litellm.ai/docs/batch_completion)
- [LiteLLM Logging/Callbacks](https://docs.litellm.ai/docs/proxy/logging)
- [LiteLLM A2A Protocol](https://docs.litellm.ai/docs/a2a)
- [LiteLLM Responses API](https://docs.litellm.ai/docs/response_api) — since v1.63.8+
- [LiteLLM Memory Issues](https://docs.litellm.ai/docs/troubleshoot/memory_issues) — `--max_requests_before_restart`
- [LiteLLM Production Best Practices](https://docs.litellm.ai/docs/proxy/prod)

### LiteLLM Alternatives (Red Team)
- [Top 5 LLM Gateways 2025 (Helicone)](https://www.helicone.ai/blog/top-llm-gateways-comparison-2025)
- [Top 5 LiteLLM Alternatives](https://dev.to/debmckinney/top-5-litellm-alternatives-in-2025-1pki)
- [LiteLLM vs OpenRouter](https://www.truefoundry.com/blog/litellm-vs-openrouter)
- [The Real Problems With LiteLLM](https://dev.to/debmckinney/the-real-problems-with-litellm-and-what-actually-works-better-227k)
- [LiteLLM Production Issues 2026](https://dev.to/debmckinney/youre-probably-going-to-hit-these-litellm-issues-in-production-59bg)
- [Helicone vs LiteLLM](https://aicostboard.com/comparisons/helicone-vs-litellm)
- [LiteLLM Memory Leak #15128](https://github.com/BerriAI/litellm/issues/15128)
- [LiteLLM API Key Leak #15799](https://github.com/BerriAI/litellm/issues/15799)
- [LiteLLM Prompt Caching + Thinking Bug #18950](https://github.com/BerriAI/litellm/issues/18950)

### Rust Crates
- [async-openai](https://crates.io/crates/async-openai) v0.33.1 — OpenAI-compatible Rust client, ~400 dependents
- [async-openai GitHub](https://github.com/64bit/async-openai)
- [genai](https://github.com/jeremychone/rust-genai) v0.5.0 — Native multi-provider Rust crate
- [tiktoken-rs](https://crates.io/crates/tiktoken-rs) v0.9.1 — Rust tokenizer for OpenAI models
- [litellm-rs](https://github.com/majiayu000/litellm-rs) — Community Rust gateway reimplementation (28 stars, not a client SDK)

### Provider Comparisons
- [Anthropic OpenAI SDK Compatibility](https://platform.claude.com/docs/en/api/openai-sdk) — does NOT support prompt caching
- [OpenAI Function Calling Guide](https://developers.openai.com/api/docs/guides/function-calling/)
- [How Streaming LLM APIs Work (Simon Willison)](https://til.simonwillison.net/llms/streaming-llm-apis)
- [OpenAI API vs Anthropic API Comparison 2026](https://is4.ai/blog/our-blog-1/openai-api-vs-anthropic-api-comparison-2026-252)

### Streaming Format Differences
- OpenAI `/chat/completions`: `data:` lines only, token at `choices[0].delta.content`, terminates with `data: [DONE]`
- OpenAI `/responses` (newer API): uses `event:` + `data:` lines with typed events — not relevant for LiteLLM proxy which exposes `/chat/completions`
- Anthropic: `event:` + `data:` lines, typed events (`message_start`, `content_block_delta`, etc.), token at `delta.text`
- LiteLLM translates Anthropic → OpenAI format automatically through the proxy

### Tool Use Format Differences
- OpenAI: `parameters` field, `tool_calls` array, `arguments` as stringified JSON, role `"tool"` for results
- Anthropic: `input_schema` field, content blocks with `type: "tool_use"`, `input` as parsed JSON, role `"user"` with `tool_result` blocks
- LiteLLM translates between formats bidirectionally

### How Other Tools Handle Multi-Provider
- [Aider Multi-Provider Integration](https://deepwiki.com/Aider-AI/aider/6.3-multi-provider-llm-integration) — uses LiteLLM Python SDK directly
- [Continue.dev LLM Abstraction](https://deepwiki.com/continuedev/continue/4.1-extension-architecture) — built own TypeScript adapter system
- [fast-litellm](https://github.com/neul-labs/fast-litellm) — PyO3 Rust acceleration for LiteLLM proxy

### Performance
- LiteLLM proxy overhead: 2ms median, 8ms P95, 13ms P99 at 1,170 RPS (4-instance)
- Bifrost (Rust gateway): 11μs mean but far fewer features
- Direct API calls: 0ms overhead but no unified format/cost tracking/load balancing

### Internal References
- `VISION.md` §5.4 — LLM provider table (Anthropic, DeepSeek, Ollama)
- `VISION.md` §5.5 — $24/month cost model
- `VISION.md` §4.3 — Background processing with batch API
- `docs/design/02-background-llm-and-tool-invocation.md` — Background LLM call architecture
- `docs/design/03-tool-call-foundation.md` — Tool dispatch and LLM tool_use integration
- `src/llm/mod.rs` — `LlmProvider` trait
- `src/llm/anthropic.rs` — Current provider implementation (375 lines)
- `src/config.rs` — `AppConfig` with `anthropic_*` fields (80 lines)
- `src/app/agent_loop.rs` — Provider-agnostic agent loop (285 lines)

### OpenAI-Compatible Providers (Direct Connection)
- [DeepSeek API](https://api-docs.deepseek.com/) — OpenAI-compatible at `https://api.deepseek.com/v1`
- [Ollama OpenAI Compatibility](https://docs.ollama.com/api/openai-compatibility) — at `http://localhost:11434/v1`
