# 17 — LiteLLM Provider Integration

> **Date:** 2026-03-13
> **Summary:** Exhaustive catalog of every API endpoint and feature exposed by an existing LiteLLM deployment, mapped to concrete integration opportunities for the context-orchestrator.

---

## 1. Executive Summary

Given an existing LiteLLM deployment, this document catalogs everything it exposes and identifies what the context-orchestrator can integrate. LiteLLM is far more than a chat proxy — it provides embeddings (for graph node similarity), reranking (for context construction), batch processing (for background compaction at 50% cost), semantic caching (for repeated analyses), token counting (for pre-flight budget validation), per-request cost tracking (via response headers), model health monitoring, guardrails, and an MCP gateway. The highest-value integrations are: (1) `/chat/completions` as a multi-model provider for background tasks, (2) `/embeddings` for graph node similarity and connection suggestions, (3) `/rerank` for context construction ordering, (4) `/batches` for cheap background compaction, (5) `x-litellm-response-cost` header for cost tracking, and (6) semantic caching for repeated background analyses.

---

## 2. Current Architecture

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

---

## 3. LiteLLM API Catalog

### 3.1 LLM Completion Endpoints

| Endpoint | Method | What It Does |
|----------|--------|-------------|
| `/chat/completions` | POST | OpenAI-compatible chat. Streaming, tool_use, parallel_tool_calls, response_format (json_object, json_schema), logprobs, seed, stop sequences. LiteLLM adds: `metadata`, `fallbacks`, `context_window_fallbacks`, `guardrails`, `tags`, `num_retries`. |
| `/completions` | POST | Legacy text completions (prompt-based). |
| `/v1/messages` | POST | **Native Anthropic format** — passes through directly to Anthropic with full feature support including `cache_control`, extended thinking, citations, PDFs. |
| `/v1/messages/count_tokens` | POST | Provider-agnostic token counting. Auto-routes to each provider's native counting API. |
| `/v1/responses` | POST | OpenAI Responses API with persistent objects, `previous_response_id` for multi-turn, built-in tools (web_search, file_search, computer_use), server-side context compaction. Works with ALL providers via bridging. |

**Integration opportunity:** The `/chat/completions` endpoint is the primary integration point — an `OpenAiCompatibleProvider` pointed at LiteLLM gives access to every model LiteLLM has configured (DeepSeek, Ollama, OpenAI, Bedrock, etc.) through a single code path. The `/v1/messages` endpoint means the existing `AnthropicProvider` can also be pointed at LiteLLM with zero code changes. The `/v1/messages/count_tokens` endpoint enables pre-flight token budget validation without making a completion call.

### 3.2 Embeddings

| Endpoint | Method | What It Does |
|----------|--------|-------------|
| `/embeddings` | POST | Generate embeddings from 15+ providers. Supports dimensions control, batch input (array of strings), multimodal input (Gemini, TwelveLabs: text + images + audio + video + PDFs). Base64 encoding option. |

**Integration opportunity — HIGH VALUE:** VISION.md §4.4 describes cascade evaluation with embedding similarity as the first pass. Embeddings enable:
- **Graph node similarity**: embed each message/node, find semantically related nodes without LLM calls
- **Connection suggestions**: `docs/research/09-embedding-based-connection-suggestions.md` already researched this
- **Context construction**: rank candidate nodes by embedding similarity to the current query before LLM reranking
- **Semantic search**: "find messages about authentication" without keyword matching
- **Compaction validation**: compare embeddings before/after compaction to measure information loss

### 3.3 Reranking

| Endpoint | Method | What It Does |
|----------|--------|-------------|
| `/rerank` | POST | Cohere-compatible reranking. Send a query + list of documents, get relevance scores. 14 providers (Cohere, Jina, Together, Bedrock, Vertex, Azure, Infinity, etc.). |

**Integration opportunity — HIGH VALUE:** Context construction (VISION.md §3.2) currently has no relevance ranking. Reranking enables:
- **Context ordering**: after gathering candidate nodes via graph traversal, rerank by relevance to the current query
- **Pruning**: when context exceeds token budget, drop lowest-ranked nodes instead of oldest
- **Multi-perspective**: rerank the same nodes differently for different tasks ("rank for security review" vs "rank for performance review")
- **Cheaper than LLM judge**: reranking costs ~$0.001 per query vs $0.01+ for LLM-as-judge

### 3.4 Batch Processing

| Endpoint | Method | What It Does |
|----------|--------|-------------|
| `/batches` | POST | Create batch jobs (JSONL upload). Supports OpenAI, Azure, Vertex, Bedrock, vLLM. 24h completion window, 50% cost reduction. |
| `/batches/{batch_id}` | GET | Check batch status. |
| `/batches/{batch_id}/cancel` | POST | Cancel a running batch. |
| `/files` | POST/GET/DELETE | Upload/manage JSONL files for batch processing. |

**Integration opportunity — HIGH VALUE:** VISION.md §4.3 explicitly calls for batch processing:
- **Background compaction**: batch-submit old messages for multi-perspective summarization at 50% cost
- **Relevance scoring**: batch-rate all nodes for relevance to active work items
- **Topic classification**: batch-classify messages into topic clusters
- **PR summarization**: batch-summarize imported PRs during bootstrapping (VISION.md §6.3)
- Cost: background Sonnet drops from $3/$15 to $1.50/$7.50 per M tokens

### 3.5 Moderation

| Endpoint | Method | What It Does |
|----------|--------|-------------|
| `/moderations` | POST | OpenAI-compatible content moderation. 10+ categories (harassment, hate, self-harm, sexual, violence, etc.). Text + multimodal input. |

**Integration opportunity — LOW:** Not directly relevant to a dev tool, but could be used as a lightweight guardrail for generated code/messages if working with sensitive content.

### 3.6 Audio

| Endpoint | Method | What It Does |
|----------|--------|-------------|
| `/audio/transcriptions` | POST | Speech-to-text from 9 providers (Whisper, Deepgram, Groq, Vertex, Gemini). |
| `/audio/speech` | POST | Text-to-speech from 8 providers (OpenAI, Azure, AWS Polly, ElevenLabs). |

**Integration opportunity — FUTURE:** Voice input for the TUI (dictate messages). Not a priority.

### 3.7 Image Generation

| Endpoint | Method | What It Does |
|----------|--------|-------------|
| `/images/generations` | POST | Image generation from 9 providers (DALL-E, gpt-image-1, Stable Diffusion via Bedrock, Recraft, etc.). |

**Integration opportunity — NONE** for current scope.

### 3.8 Model Information

| Endpoint | Method | What It Does |
|----------|--------|-------------|
| `/models` | GET | List all available models. |
| `/model/info` | GET | Detailed info per model: max tokens, input/output cost, supports_vision, supports_function_calling, supports_parallel_function_calling, supports_response_schema, mode (chat/embedding/completion). |
| `/model_group/info` | GET | Aggregated info per model group (for load-balanced deployments). |

**Integration opportunity — MEDIUM:** Dynamic model discovery and capability detection:
- **Auto-configure**: on startup, query `/model/info` to know which models support tools, vision, JSON mode
- **Token budget**: use `max_tokens` from model info for context window calculations
- **Cost estimation**: use input/output cost per model for pre-flight cost estimates
- **Capability routing**: automatically route vision requests to models that support it

### 3.9 Cost & Spend Tracking

| Endpoint | Method | What It Does |
|----------|--------|-------------|
| Response header: `x-litellm-response-cost` | — | Per-request cost in the response header. |
| Response header: `x-litellm-key-spend` | — | Cumulative spend for the API key. |
| `/spend/logs` | GET | Detailed spend logs with filters (key, user, model, date range, tags). |
| `/spend/tags` | GET/POST | Spend aggregated by custom tags. |
| `/global/spend/report` | GET | Global spend report by model, team, provider. |
| `/user/daily/activity` | GET | Per-user daily token/cost breakdown. |

**Integration opportunity — HIGH VALUE:**
- **Per-request cost**: parse `x-litellm-response-cost` header from every response, store on graph nodes
- **Budget enforcement**: check `x-litellm-key-spend` against configurable limit, pause background tasks when approaching budget
- **Cost dashboard in TUI**: query `/spend/tags` to show cost by task type (compaction, rating, foreground chat)
- **Tag-based attribution**: send `tags: ["background_compaction"]` or `tags: ["foreground_chat"]` in request metadata to separate costs

### 3.10 Caching

| Endpoint | Method | What It Does |
|----------|--------|-------------|
| `/cache/ping` | GET | Check cache health. |
| `/cache/delete` | POST | Flush cache. |
| Per-request: `cache` body param | — | Control caching per-request: `ttl`, `s-maxage`, `no-cache`, `no-store`, `namespace`. |

**Cache backends:** In-memory, disk, Redis, Redis Semantic, Qdrant Semantic, S3, GCS.

**Modes:** `default_on` (cache everything), `default_off` (opt-in per request).

**Integration opportunity — HIGH VALUE:** Semantic caching is particularly powerful:
- **Repeated analyses**: background tasks that re-analyze the same content (e.g., re-rating nodes after graph changes) hit cache instead of making new API calls
- **Topic classification**: classify the same message for multiple perspectives — second+ calls hit cache
- **Similarity threshold**: tunable (0.0-1.0) — a query semantically similar to a cached one returns the cached response
- **Namespace isolation**: separate cache for foreground vs background, or per-conversation
- **TTL control**: short TTL for volatile data, long TTL for stable analyses

### 3.11 Rate Limiting & Request Metadata

**Response headers on every request:**
| Header | What It Contains |
|--------|-----------------|
| `x-ratelimit-remaining-requests` | Remaining requests in current window |
| `x-ratelimit-remaining-tokens` | Remaining tokens in current window |
| `x-ratelimit-limit-requests` | Request limit |
| `x-ratelimit-limit-tokens` | Token limit |
| `x-ratelimit-reset-requests` | Time until request limit resets |
| `x-ratelimit-reset-tokens` | Time until token limit resets |
| `x-litellm-response-duration-ms` | Total response time |
| `x-litellm-overhead-duration-ms` | LiteLLM proxy overhead |
| `x-litellm-attempted-retries` | Number of retries attempted |
| `x-litellm-attempted-fallbacks` | Number of fallbacks attempted |
| `x-litellm-call-id` | Unique request identifier |
| `x-litellm-model-id` | Actual model deployment used |
| `x-litellm-model-group` | Model group name |
| `x-litellm-cache-key` | Cache key (if cached) |

**Integration opportunity — MEDIUM:**
- **Adaptive throttling**: read `x-ratelimit-remaining-tokens`, slow down background tasks when limits are low
- **Latency tracking**: store `x-litellm-response-duration-ms` on graph nodes for performance analysis
- **Fallback visibility**: show `x-litellm-attempted-fallbacks` in TUI to indicate provider issues
- **Cache hit detection**: check for `x-litellm-cache-key` to show cached vs fresh responses

### 3.12 Health & Monitoring

| Endpoint | Method | What It Does |
|----------|--------|-------------|
| `/health` | GET | Active health check — calls each configured model with max_tokens:1. |
| `/health/readiness` | GET | DB + cache + version check (unauthenticated). |
| `/health/liveliness` | GET | Simple alive check (unauthenticated). |
| `/health/services` | GET | Third-party service health (Datadog, Slack, Langfuse). |

**Integration opportunity — MEDIUM:** Check `/health/readiness` on app startup. If LiteLLM is unreachable, degrade gracefully to Anthropic-only mode. Show health status in TUI status bar.

### 3.13 Guardrails

**Providers:** Aporia, Bedrock, Lakera, Presidio (PII masking), AIM Security, Azure Text Moderation, Guardrails AI, Hide-Secrets.

**Modes:** `pre_call` (before LLM), `post_call` (after LLM), `during_call` (async parallel), `logging_only`.

**Per-request:** Send `guardrails: ["presidio"]` in request body. Presidio supports MASK or BLOCK actions with configurable confidence per entity type (SSN, email, credit card, AWS keys).

**Integration opportunity — LOW-MEDIUM:** Useful if context-orchestrator handles sensitive codebases. Could mask PII in tool results (file contents) before sending to LLM. The Hide-Secrets provider detects API keys, passwords, tokens in content — relevant for code analysis.

### 3.14 MCP Gateway

**27 management endpoints** under `/v1/mcp/*`. Supports SSE, HTTP, stdio transports. OAuth 2.0, AWS SigV4, bearer token auth to upstream MCP servers.

| Feature | Description |
|---------|-------------|
| Tool discovery | List available MCP tools via `/chat/completions` or `/v1/responses` |
| Tool calling | Invoke MCP tools through the chat API |
| Access control | Per-key, per-team, per-org tool permissions |
| Server management | Register, approve, configure MCP servers |
| OpenAPI-to-MCP | Automatic conversion of OpenAPI specs to MCP tools |

**Integration opportunity — MEDIUM:** VISION.md §4.8 specifies MCP as the tool protocol. LiteLLM's MCP gateway could serve as the tool registry — context-orchestrator queries available tools from LiteLLM rather than maintaining its own tool list. Tool execution goes through the gateway with centralized auth.

### 3.15 Prompt Caching Passthrough

LiteLLM passes through provider-native prompt caching:
- **Anthropic**: requires `cache_control: {"type": "ephemeral"}` on message content blocks. Returns `cache_creation_input_tokens` and `cache_read_input_tokens` in usage.
- **OpenAI**: automatic (>= 1024 tokens). No client changes needed.
- **DeepSeek**: automatic. No client changes needed.
- **Auto-injection**: LiteLLM config supports `cache_control_injection_points` to automatically add `cache_control` to system messages without client code changes.

**Integration opportunity — HIGH VALUE:** The context-orchestrator builds deterministic context (VISION.md §3.2). Deterministic = high cache hit rates. Auto-injection means zero code changes — configure it in LiteLLM and system prompts are cached automatically.

### 3.16 Structured Output & JSON Mode

Supported via `response_format`:
- `{"type": "json_object"}` — force JSON output
- `{"type": "json_schema", "json_schema": {...}}` — enforce specific schema

Works with all providers that support it. LiteLLM translates the format for non-native providers.

**Integration opportunity — MEDIUM:** Background tasks (compaction, rating, classification) that need structured output can use `json_schema` response format to guarantee valid JSON without parsing gymnastics. Currently `strip_json_fences()` in `background_llm_call()` is a workaround for this.

### 3.17 Tag-Based Routing & Metadata

- **Tags**: send `tags: ["background", "compaction"]` in request body or `x-litellm-tags` header
- **Metadata**: send `metadata: {"conversation_id": "...", "task_type": "compaction"}` for tracking
- **Tag-based routing**: models in LiteLLM config can be tagged; requests with matching tags route to tagged models

**Integration opportunity — MEDIUM:** Separate foreground and background traffic at the proxy level. Tag background compaction requests differently from foreground chat. Query spend per tag to see cost breakdown by task type.

### 3.18 Fallback & Routing

- **Model fallbacks**: `fallbacks: [{"gpt-4": ["claude-sonnet", "deepseek"]}]` in request body
- **Context window fallbacks**: auto-route to larger-context model if input exceeds model limit
- **Pre-call checks**: `enable_pre_call_checks: true` validates token count before calling, avoiding wasted API calls
- **Cooldowns**: auto-disable models after failure threshold (configurable), auto-re-enable after cooldown
- **6 routing strategies**: simple-shuffle, rate-limit-aware, latency-based, least-busy, cost-based, custom

**Integration opportunity — MEDIUM:** Pre-call token validation is particularly useful — the context-orchestrator can attempt to send full context to a cheap model, and LiteLLM automatically falls back to a more expensive larger-context model only when needed.

### 3.19 Observability

- **Prometheus**: `/metrics` endpoint with 30+ metric families (spend, tokens, latency, deployment health, rate limits)
- **40+ callback integrations**: Langfuse, DataDog, OTEL, S3, GCS, DynamoDB, Sentry, Slack, custom webhooks
- **Spend alerts**: webhook notifications at 85% and 95% of budget, with projected spend analysis
- **Request logging**: all requests logged to PostgreSQL, queryable via `/spend/logs`

**Integration opportunity — LOW-MEDIUM:** Prometheus metrics could feed a Grafana dashboard for long-term cost/performance analysis. Spend alerts prevent surprise bills. Request logs enable debugging "why did that compaction produce bad results?"

### 3.20 Additional Endpoints

| Endpoint | What It Does | Integration Value |
|----------|-------------|-------------------|
| `/v1/responses` + compaction | Server-side context compaction with `previous_response_id` | LOW — we build our own compaction |
| `/rerank` | Document reranking (covered in §3.3) | HIGH |
| `/key/*` (10 endpoints) | Virtual key management, per-key budgets/rate limits | LOW — single user |
| `/fine_tuning/jobs` | Fine-tuning management (Enterprise) | FUTURE — fine-tune compaction model |
| `/a2a/{agent}/message/send` | Agent-to-Agent protocol (Google A2A) | FUTURE — multi-agent coordination |
| Pass-through endpoints | Proxy to any external API with centralized auth | LOW |
| `/vector_stores`, `/rag/*` | Vector store and RAG management | LOW — we build our own graph |

---

## 4. Integration Opportunity Matrix

Ranked by value to context-orchestrator vs implementation effort:

| # | Feature | Value | Effort | VISION.md | Notes |
|---|---------|-------|--------|-----------|-------|
| 1 | `/chat/completions` as multi-model provider | Critical | Medium | §5.4 | Unlocks background processing on cheap models |
| 2 | `x-litellm-response-cost` header | High | Low | §5.5 | Parse one header, store on nodes |
| 3 | `/embeddings` for node similarity | High | Medium | §4.4, research/09 | Enables semantic graph operations |
| 4 | `/rerank` for context construction | High | Medium | §3.2 | Better than token-count-based pruning |
| 5 | `/batches` for background compaction | High | Medium | §4.3 | 50% cost reduction on background work |
| 6 | Semantic caching | High | Low | §4.3 | Config-only in LiteLLM, zero code changes |
| 7 | Prompt caching auto-injection | High | Low | §5.4 | Config-only in LiteLLM, zero code changes |
| 8 | `/model/info` for capabilities | Medium | Low | — | Auto-discover models and limits |
| 9 | Rate limit headers | Medium | Low | §4.3 | Adaptive background task throttling |
| 10 | `/v1/messages/count_tokens` | Medium | Low | §8.3 | Pre-flight budget validation |
| 11 | `response_format: json_schema` | Medium | Low | — | Replaces `strip_json_fences()` hack |
| 12 | Tag-based cost attribution | Medium | Low | §5.5 | Send tags, query spend per tag |
| 13 | Pre-call checks + context window fallbacks | Medium | Low | — | Config-only in LiteLLM |
| 14 | Health check on startup | Medium | Low | — | Graceful degradation |
| 15 | MCP gateway | Medium | Medium | §4.8 | Centralized tool registry |
| 16 | Guardrails (Hide-Secrets) | Low | Low | — | Detect leaked API keys in code |
| 17 | `/moderations` | Low | Low | — | Content safety |
| 18 | Prometheus metrics | Low | Low | — | Long-term dashboards |
| 19 | Audio transcription | Low | Medium | — | Voice input |
| 20 | Fine-tuning | Low | High | — | Custom compaction model |

---

## 5. VISION.md Alignment

| VISION.md Goal | LiteLLM Feature | Integration Path |
|---------------|----------------|------------------|
| §5.4: Multi-model (Anthropic, DeepSeek, Ollama) | `/chat/completions` + model routing | OpenAI-compat provider → LiteLLM |
| §5.4: Prompt caching (90% cost reduction) | Auto-inject `cache_control` | LiteLLM config only |
| §5.5: $24/month cost budget | `x-litellm-response-cost` + `/spend/tags` | Parse header + tag requests |
| §4.3: Background compaction (batch) | `/batches` | JSONL upload, poll for results |
| §4.3: Background processing rate limiting | Rate limit headers + tag-based routing | Adaptive throttling in Rust |
| §4.4: Cascade evaluation (cheap first) | `/embeddings` → `/rerank` → LLM judge | Three-tier relevance pipeline |
| §4.4: Multi-rater relevance | `/rerank` as fast rater | Complements LLM-as-judge |
| §3.2: Context construction ordering | `/rerank` | Rerank candidate nodes by query |
| §4.2: Compaction validation | `/embeddings` similarity | Compare pre/post embeddings |
| §4.8: MCP tool protocol | MCP gateway | Centralized tool registry |
| §6.3: PR summarization (bootstrap) | `/batches` | Batch-summarize imported PRs |
| §8.3: Token counting accuracy | `/v1/messages/count_tokens` | Provider-agnostic counting |

---

## 6. Recommended Integration Roadmap

### Phase 1: Multi-Model Provider + Cost Tracking

- `OpenAiCompatibleProvider` in `src/llm/openai_compat.rs` pointed at LiteLLM
- Parse `x-litellm-response-cost` from response headers, store on graph nodes
- Send `metadata: {conversation_id, task_type}` and `tags` with every request
- Query `/model/info` on startup for available models and capabilities
- Check `/health/readiness` on startup for graceful degradation

### Phase 2: Embeddings + Reranking Pipeline

- New trait: `EmbeddingProvider` with `embed(texts: Vec<String>) -> Vec<Vec<f32>>`
- LiteLLM implementation via `/embeddings` endpoint
- Embed messages on creation (background task), store vectors on nodes
- Use `/rerank` in context construction: gather candidates → rerank → take top-N
- Ties into existing `docs/research/09-embedding-based-connection-suggestions.md`

### Phase 3: Batch Processing + Semantic Cache

- Implement batch job lifecycle: create JSONL → upload to `/files` → submit `/batches` → poll → retrieve results
- Route background compaction and rating through batch API (50% cost savings)
- Enable semantic caching in LiteLLM config for background tasks (`default_off` mode, opt-in per request)
- Use `json_schema` response format for structured background outputs

### Phase 4: Advanced Features

- MCP gateway integration for tool discovery and execution
- Guardrails for sensitive codebases (Hide-Secrets for API key detection)
- Prometheus metrics → Grafana dashboard for cost/performance over time
- Pre-call token validation + context window fallbacks

---

## 7. Integration Design

### New Traits

```rust
/// Embedding generation for graph node similarity.
#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    async fn embed(&self, texts: Vec<String>, model: &str) -> Result<Vec<Vec<f32>>>;
}

/// Document reranking for context construction.
#[async_trait]
pub trait RerankProvider: Send + Sync {
    async fn rerank(&self, query: &str, documents: Vec<String>, model: &str, top_n: usize) -> Result<Vec<RerankResult>>;
}

pub struct RerankResult {
    pub index: usize,
    pub relevance_score: f64,
}
```

### LiteLLM Response Header Parsing

```rust
/// Extracted from every LiteLLM response.
pub struct LiteLlmMetadata {
    pub cost: Option<f64>,              // x-litellm-response-cost
    pub key_spend: Option<f64>,         // x-litellm-key-spend
    pub duration_ms: Option<u64>,       // x-litellm-response-duration-ms
    pub overhead_ms: Option<u64>,       // x-litellm-overhead-duration-ms
    pub retries: Option<u32>,           // x-litellm-attempted-retries
    pub fallbacks: Option<u32>,         // x-litellm-attempted-fallbacks
    pub model_id: Option<String>,       // x-litellm-model-id
    pub cache_key: Option<String>,      // x-litellm-cache-key
    pub remaining_tokens: Option<u64>,  // x-ratelimit-remaining-tokens
}
```

### Request Metadata Convention

```rust
/// Sent with every request to LiteLLM for cost attribution.
pub struct RequestMetadata {
    pub conversation_id: Uuid,
    pub task_type: String,  // "foreground_chat", "background_compaction", "background_rating", etc.
    pub tags: Vec<String>,
}
```

---

## 8. Red/Green Team

### Green Team (Claim Validation)

| Claim | Verdict | Notes |
|-------|---------|-------|
| `/embeddings` supports 15+ providers | CONFIRMED | OpenAI, Cohere, Bedrock, Gemini, Mistral, Voyage, NVIDIA, HuggingFace, Jina, etc. |
| `/rerank` supports 14 providers | CONFIRMED | Cohere, Jina, Together, Bedrock, Vertex, Azure, Infinity, etc. |
| `/batches` gives 50% cost reduction | CONFIRMED | OpenAI batch API pricing is 50% of real-time |
| `x-litellm-response-cost` header exists | CONFIRMED | Primary per-request cost mechanism |
| Semantic caching with similarity threshold | CONFIRMED | Redis Semantic + Qdrant backends, tunable threshold |
| Prompt caching auto-injection | CONFIRMED | `cache_control_injection_points` in LiteLLM config |
| `/v1/messages` native Anthropic passthrough | CONFIRMED | Full Anthropic API support including cache_control |
| `/model/info` returns capabilities | CONFIRMED | max_tokens, cost, supports_function_calling, supports_vision, etc. |
| Pre-call token validation | CONFIRMED | `enable_pre_call_checks: true` in config |

### Red Team (Risks & Limitations)

**R1: Embedding quality varies by provider.** Different embedding models produce incomparable vectors. If the user switches embedding models, all stored vectors must be recomputed. **Mitigation:** Store model name alongside vectors; recompute on model change.

**R2: Reranking adds latency to context construction.** Each rerank call is an API round-trip (~50-200ms). For interactive use, this delays the user's first token. **Mitigation:** Use reranking only for background context construction, not foreground. Or cache rerank results.

**R3: Batch API has 24h completion window.** Results are not immediate — unsuitable for time-sensitive operations. **Mitigation:** Use only for truly background operations (compaction, rating). Foreground uses real-time API.

**R4: Semantic cache can return stale/wrong answers.** If the threshold is too loose, semantically similar but meaningfully different queries return cached results from a different query. **Mitigation:** Use high similarity threshold (>0.95), namespace per task type, short TTL for volatile analyses.

**R5: Cost header depends on LiteLLM's pricing database accuracy.** If model pricing is stale or wrong, costs are inaccurate. **Mitigation:** Cross-validate with provider invoices periodically.

**R6: Prompt caching + extended thinking has known bugs (GitHub #18950).** Cache-enabled requests with thinking blocks can fail. **Mitigation:** Disable auto-injection for thinking-enabled requests.

### Code Accuracy

All file references verified against codebase — see corrections applied in §2 table.

---

## 9. Sources

### LiteLLM Documentation
- [Supported Endpoints](https://docs.litellm.ai/docs/supported_endpoints)
- [Embeddings](https://docs.litellm.ai/docs/embedding/supported_embedding)
- [Reranking](https://docs.litellm.ai/docs/rerank)
- [Batch API](https://docs.litellm.ai/docs/batch_completion)
- [Caching](https://docs.litellm.ai/docs/caching/all_caches) — 7 backends, semantic cache
- [Cost Tracking](https://docs.litellm.ai/docs/proxy/cost_tracking) — `x-litellm-response-cost`
- [Prompt Caching](https://docs.litellm.ai/docs/completion/prompt_caching)
- [Model Info](https://docs.litellm.ai/docs/proxy/model_management)
- [Guardrails](https://docs.litellm.ai/docs/proxy/guardrails)
- [MCP Gateway](https://docs.litellm.ai/docs/mcp)
- [Rate Limiting](https://docs.litellm.ai/docs/proxy/rate_limiting)
- [Structured Output](https://docs.litellm.ai/docs/completion/json_mode)
- [Tag-Based Routing](https://docs.litellm.ai/docs/proxy/tag_routing)
- [Health Checks](https://docs.litellm.ai/docs/proxy/health)
- [Responses API](https://docs.litellm.ai/docs/response_api)
- [A2A Protocol](https://docs.litellm.ai/docs/a2a)
- [Benchmarks](https://docs.litellm.ai/docs/benchmarks) — 1,170 RPS, 8ms P95 overhead
- [Prompt Caching + Thinking Bug #18950](https://github.com/BerriAI/litellm/issues/18950)

### Internal References
- `VISION.md` §3.2 — Context construction ordering
- `VISION.md` §4.2 — Multi-perspective compaction
- `VISION.md` §4.3 — Background processing / batch API
- `VISION.md` §4.4 — Multi-rater relevance / cascade evaluation
- `VISION.md` §4.8 — MCP tool protocol
- `VISION.md` §5.4 — Multi-model provider table
- `VISION.md` §5.5 — $24/month cost model
- `VISION.md` §6.3 — PR summarization bootstrap
- `docs/research/09-embedding-based-connection-suggestions.md` — Embedding-based graph connections
- `src/llm/mod.rs:61-75` — `LlmProvider` trait
- `src/llm/mod.rs:108-136` — `background_llm_call()` with semaphore
- `src/llm/anthropic.rs` — Current provider (375 lines)
- `src/config.rs` — AppConfig (80 lines)
