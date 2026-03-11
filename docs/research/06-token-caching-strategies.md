# Token Caching Strategies for Graph-Reconstructed Prompts

> Research conducted 2026-03-12 investigating LLM prompt caching strategies, provider APIs,
> and cache-optimized architectures for the Context Manager's graph-based context construction.
> Audited by red/green team agents on the same date.

---

## Executive Summary

Context Manager reconstructs LLM input from graph nodes on every request — walking `RespondsTo` edges from leaf to root, extracting `Message` and `SystemDirective` nodes into a linear prompt. Without deliberate architectural design, this approach is **cache-hostile**: every prompt reconstruction risks producing a slightly different token prefix, invalidating provider caches and paying full input token costs on every turn.

Provider prefix caching (Anthropic, OpenAI, Google) offers **50-90% cost reduction** and **up to 85% latency reduction** for long prompts, but requires the prompt to begin with an **identical token sequence** across requests. A January 2026 [arXiv paper](https://arxiv.org/abs/2601.06007) confirms 41-80% cost reduction for agentic workloads when caching is applied correctly.

Our graph architecture is **uniquely positioned** to exploit prefix caching: we control serialization order, we decide which nodes enter the prompt and in what sequence, and we can design compaction to preserve cache-friendly prefixes. This document proposes a **4-layer prompt model** that maps naturally to provider cache breakpoints and to our graph's node type hierarchy.

As the Claude Code team puts it: *"You fundamentally have to design agents for prompt caching first, almost every feature touches on it somehow."* — Thariq Shihipar, Claude Code engineer. Claude Code declares SEVs when cache hit rates drop.

**Key recommendation**: Treat prompt serialization order as a first-class architectural concern. Design every component that touches LLM input — context construction, compaction, multi-agent handoff — with prefix stability as a constraint.

---

## 1. The Caching Landscape

### 1.1 Provider Prefix Caching (Primary Focus)

Prefix caching exploits the fact that LLM inference computes key-value (KV) pairs for every input token through the model's attention layers. When multiple requests share an identical prefix, the KV computation for that prefix is redundant. Providers store the computed KV state and let subsequent requests skip recomputation, starting inference from the cached state.

**The fundamental constraint: prefix caching requires a 100% exact byte match.** Any difference in the prefix — a changed timestamp, reordered JSON keys, an extra whitespace character — invalidates the entire cache.

#### Provider Feature Matrix (March 2026)

| Feature | Anthropic | OpenAI | Google |
|---------|-----------|--------|--------|
| Mechanism | Explicit breakpoints (up to 4) or automatic | Fully automatic | Implicit (auto) + Explicit (named caches) |
| Read discount | 90% | 50% | 90% (Gemini 2.5+), 75% (Gemini 2.0) |
| Write premium | 1.25x (5min TTL) / 2x (1hr TTL) | None | None (implicit) / Storage cost (explicit) |
| Min tokens | 1,024–4,096 (model-dependent, see below) | 1,024 | 1,024–2,048 (implicit), 32,768 (explicit) |
| TTL | 5min (default) / 1hr (at 2x write cost) | 5–10min (up to 24hr with `prompt_cache_retention`) | Custom (explicit) |
| Developer control | High (explicit breakpoints) | Low (routing hints only) | Medium (named caches) |
| Cache scope | Workspace-level (changed Feb 2026) | Organization-level | Project-level |

#### Anthropic (Claude)

- **Mechanism**: Explicit `cache_control` breakpoints (up to 4) on content blocks, or automatic caching
- **Cache hierarchy**: Tools → System → Messages (in that order). Changing a tool definition invalidates the system cache downstream. Changing `tool_choice` invalidates the messages cache.
- **Minimum cacheable size by model**:

  | Model | Min tokens |
  |-------|-----------|
  | Claude Opus 4.6 / 4.5 | 4,096 |
  | Claude Opus 4.1 / 4, Sonnet 4.5 / 4 / 3.7 | 1,024 |
  | Claude Sonnet 4.6 | 2,048 |
  | Claude Haiku 4.5 | 4,096 |
  | Claude Haiku 3.5 | 2,048 |

- **TTL**: 5 minutes (default, refreshed on each hit; write cost 1.25x) or 1 hour (write cost **2x** base input price). Cache is best-effort — not guaranteed to persist for the full TTL under high provider load.
- **Pricing**: Cache reads cost 0.1x base input price (90% discount). Break-even at 2+ cache hits per cached prefix.
- **Concurrency note**: A cache entry becomes available only after the first response begins. Parallel requests should wait for the first response to establish the cache.
- **Block limit**: Automatic prefix checking looks back ~20 content blocks from each explicit breakpoint. Long conversations (>20 message blocks before a breakpoint) need additional explicit breakpoints to ensure earlier content is checked.
- **Cache scope change (February 2026)**: Caches are now isolated at the **workspace level**, not the organization level. Multi-agent systems where agents run in different workspaces will NOT share caches. All agents must use the same workspace for Layer 1 cache sharing.
- **Extended thinking**: Enabling/disabling extended thinking or changing the thinking budget invalidates previously cached message-level prefixes. Thinking blocks in assistant turns ARE cached when they appear as input.
- **Images**: Adding or removing image content **anywhere** in the prompt invalidates the entire message-level cache.

Source: [Anthropic Prompt Caching Documentation](https://platform.claude.com/docs/en/build-with-claude/prompt-caching)

#### OpenAI

- **Mechanism**: Fully automatic, no developer-controlled breakpoints
- **Pricing**: 50% discount on cached input tokens; no write premium
- **Scope**: Cache shared within an organization (not across orgs)
- **Routing hint**: Optional `prompt_cache_key` parameter (available on `/v1/responses` and `chat.completions.create`) influences request routing to cache-warm instances. One customer improved cache hit rate from 60% to 87%. Note: requires ~50 requests to warm the cache, and rate limits apply (~15 req/min per prefix+key).
- **Extended retention**: `prompt_cache_retention` parameter (e.g., `"24h"`) for longer cache lifetimes

Source: [OpenAI Prompt Caching 201](https://developers.openai.com/cookbook/examples/prompt_caching_201/)

#### Google (Gemini)

- **Implicit caching**: Activates automatically; 90% discount on Gemini 2.5+, 75% on Gemini 2.0 models. Min 1,024 tokens (Flash) or 2,048 tokens (Pro).
- **Explicit context caching**: Developer-created named caches with guaranteed discounts. Charged for storage duration in addition to token usage. Requires minimum **32,768 tokens** — significantly higher than Anthropic/OpenAI thresholds.
- **Use case**: Best for very large, stable context (e.g., entire codebases) reused across multiple queries

### 1.2 Semantic Caching

Semantic caching uses vector embeddings to match logically equivalent queries, returning cached responses without invoking the LLM. Redis LangCache has achieved ~73% cost reduction in high-repetition workloads.

**Relevance to Context Manager**: Limited for conversational agents (each turn is unique), but potentially useful for:
- Repeated tool definition lookups
- Caching embeddings of compacted summaries for retrieval
- Deduplicating semantically equivalent questions across agents

### 1.3 Response Caching (Exact Match)

Store complete request→response pairs and return cached responses for identical requests. This is the simplest strategy but has very low hit rates for conversational agents where each turn is inherently different.

**Relevance to Context Manager**: Minimal for conversation, but potentially useful for idempotent tool calls (e.g., file reads, git status queries that haven't changed).

### 1.4 Multi-Tier Architecture

Production systems typically layer multiple caching strategies:

```
Request → Exact Match Cache (100% savings if hit)
        → Semantic Cache (100% savings if hit)
        → Prefix Cache (50-90% savings on prefix)
        → Full Inference
```

Each tier captures a different optimization opportunity. The tiers are complementary — prefix caching provides value even when semantic caching misses, because the shared prefix portion of the prompt still avoids recomputation.

---

## 2. The Problem: Graph-Reconstructed Prompts

### 2.1 Current Architecture

Context construction in `src/app.rs:build_context` (lines 52-113):

1. Walk `RespondsTo` edges from the active branch's leaf node to the root
2. Extract `SystemDirective` nodes as the system prompt
3. Extract `Message` nodes as the conversation history (user/assistant turns)
4. Skip non-conversation nodes (`WorkItem`, `GitFile`, `Tool`, `BackgroundTask`)
5. If total tokens exceed the budget, remove oldest messages proportionally

The API call in `src/llm/anthropic.rs` (lines 56-89) sends:
- `system`: `Option<String>` — a plain string, not structured content blocks
- `messages`: Vec of `{role, content}` objects where content is a plain `String`
- No `cache_control` breakpoints — every request pays full input token cost
- `anthropic-version` header set to `"2023-06-01"` (oldest supported version)
- The SSE parser does **not** parse `message_start` events, so `cache_creation_input_tokens` and `cache_read_input_tokens` are silently discarded

### 2.2 Why This Is Cache-Hostile Without Design

**Prefix instability from compaction**: When background compaction replaces a sequence of messages with a summary, the serialized content changes at that position. Every message after the compaction boundary shifts, invalidating the prefix from that point forward.

**Dynamic metadata in serialization**: If node metadata (timestamps, token counts, UUIDs) leaks into the serialized prompt, prefix stability is destroyed. Even `created_at` timestamps on otherwise identical content cause cache misses.

**Content position shifts**: As the architecture evolves to include more node types in the prompt (e.g., relevant `WorkItem` descriptions, `GitFile` diffs), inserting content at arbitrary positions shifts everything after it.

**Multi-agent divergence**: Different agents working on the same project may share system instructions but have completely different conversation histories. Without deliberate design, no prefix sharing occurs.

**Non-deterministic serialization**: The `ConversationGraph` stores nodes in a `HashMap<Uuid, Node>`. While `get_branch_history` avoids iterating the HashMap directly (it follows `responds_to` edges), any future code path that serializes HashMap-backed data into the prompt will produce non-deterministic byte output, silently breaking prefix stability.

**Tool definition churn**: The codebase discovers tools dynamically via `TaskMessage::ToolsDiscovered` (in `src/app.rs`). If tool definitions change between requests (e.g., tool discovery runs mid-conversation) and tools are included in the prompt, the entire cache hierarchy is invalidated — because tools sit at the top of Anthropic's cache hierarchy (Tools → System → Messages).

### 2.3 Why Our Architecture Has an Advantage

Despite these risks, a graph-based context manager has structural advantages that most chat frameworks lack:

**We control serialization order.** The graph traversal is deterministic: `get_branch_history` in `src/graph.rs` (lines 277-305) always produces the same node sequence for the same branch state. It follows a single linked-list chain (each node has exactly one `RespondsTo` parent), so there is no ambiguity. This determinism is the foundation of prefix stability.

**We control what enters the prompt.** Node filtering is explicit — the `match` statement in `build_context` decides which node types reach the LLM. This is a policy decision, not an accident.

**We can partition the graph.** Nodes naturally divide into stable subgraphs (system directives, tool definitions, project context) and volatile subgraphs (recent messages, tool results). This partition maps directly to cache prefix boundaries.

**We can design compaction to be cache-aware.** Unlike chat frameworks that compact by rewriting history in-place, we can produce immutable summary nodes that become part of the stable prefix, never modified after creation.

---

## 3. Cache-Optimized Prompt Architecture

### 3.1 The 4-Layer Prompt Model

We propose structuring every LLM prompt as four layers, ordered from most stable to most volatile:

```
┌─────────────────────────────────────────────────────────┐
│ Layer 1: STABLE PREFIX                                  │
│ SystemDirective + Tool definitions + Project context    │
│ Changes: rarely (project reconfiguration)               │
│ Cache scope: shared across ALL agents in project        │
│ ── cache breakpoint 1 ──────────────────────────────── │
├─────────────────────────────────────────────────────────┤
│ Layer 2: SESSION-STABLE                                 │
│ Compacted history summaries                             │
│ Changes: when compaction runs (every N messages)        │
│ Cache scope: shared across turns within a session       │
│ ── cache breakpoint 2 ──────────────────────────────── │
├─────────────────────────────────────────────────────────┤
│ Layer 3: GROWING CONVERSATION                           │
│ Recent message history (user + assistant turns)         │
│ Changes: grows by 2 messages per turn (append-only)     │
│ Cache scope: prefix reuse from turn to turn             │
│ ── cache breakpoint 3 (penultimate message) ─────────  │
├─────────────────────────────────────────────────────────┤
│ Layer 4: VOLATILE                                       │
│ Latest user message + tool results                      │
│ Changes: every single turn                              │
│ Cache scope: none (always new)                          │
└─────────────────────────────────────────────────────────┘
```

This maps to Anthropic's 4 breakpoint limit. Each breakpoint marks the end of a cacheable prefix:

| Breakpoint | Position | Shared across |
|------------|----------|---------------|
| 1 | After Layer 1 | All agents in project (same workspace) |
| 2 | After Layer 2 | All turns within a session |
| 3 | After penultimate message | Adjacent turns |
| 4 | Automatic (Anthropic) | N/A |

**Design note**: The 4-layer model is optimized for Anthropic's specific API (4 explicit breakpoints). OpenAI has no developer-controlled breakpoints (automatic only), and Google's explicit caching uses named cache objects. A simpler 3-layer model (stable prefix → conversation → latest turn) may suffice for providers without fine-grained breakpoint control. The 4-layer model is the superset; simpler providers use a subset.

**20-block lookback constraint**: Anthropic's automatic prefix checking looks back only ~20 content blocks from each explicit breakpoint. In conversations exceeding 20 message blocks in Layer 3, earlier blocks won't be checked for cache hits unless an additional breakpoint is placed. For long conversations, this may require consuming a breakpoint slot for a mid-conversation marker, reducing available breakpoints for the layered model.

#### Cost Model

Using Claude Sonnet 4.5 pricing ($3/M input tokens, $0.30/M cached read, $3.75/M cached write):

**Example**: 10-turn conversation, 50K token system+tools, 20K compacted summary, 30K conversation growing by ~6K/turn.

| Scenario | Per-turn cost | 10-turn total | vs. uncached |
|----------|--------------|---------------|--------------|
| No caching | 100K × $3/M = $0.30 | $3.00 | 1.0x |
| Layer 1 cached (turn 1 write) | Turn 1: 50K×$3.75/M + 50K×$3/M = $0.34; Turns 2-10: 50K×$0.30/M + 50K×$3/M = $0.165 | $1.83 | 0.61x |
| Layers 1+2 cached | Turn 1: 70K×$3.75/M + 30K×$3/M = $0.35; Turns 2-10: 70K×$0.30/M + 30K×$3/M = $0.11 | $1.34 | 0.45x |
| Layers 1+2+3 cached | Turns 2-10: ~95K×$0.30/M + ~5K×$3/M = $0.044 | ~$0.75 | 0.25x |

**Realistic adjustments**: These assume 100% cache hits after warmup. With a conservative 80% hit rate (accounting for TTL expiry when users pause >5 minutes), multiply cached-read savings by 0.8. Also account for compaction-induced invalidation: each compaction event pays a full cache write for the new Layer 2 content.

**Real-world validation**: A [case study](https://medium.com/@labeveryday/prompt-caching-is-a-must-how-i-went-from-spending-720-to-72-monthly-on-api-costs-3086f3635d63) reports going from $720/month to $72/month (90% reduction) with prompt caching. Claude Code's economics depend on caching: without it, a 100-turn Opus session costs $50-100 in input tokens; with caching, $10-19.

### 3.2 Graph-to-Prompt Serialization Rules

To maintain prefix stability, the serialization pipeline must follow strict invariants:

1. **Deterministic byte output**: Same graph state produces the same serialized bytes. Use sorted keys in any structured data, stable formatting, no timestamps or request-specific metadata in serialized content. Beware of `HashMap` iteration order — any future code that serializes HashMap-backed data into prompt content will silently break determinism.

2. **Append-only conversation growth**: New messages are always appended at the end of Layer 3. Never insert messages into the middle of the conversation.

3. **Immutable node content**: Once a node is created and serialized, its content never changes. Compaction creates *new* summary nodes rather than modifying existing ones.

4. **Stable layer boundaries**: Layer 1 content is determined by project configuration (changes require explicit user action). Layer 2 content is determined by compaction (changes only when compaction runs). These boundaries are not affected by conversation growth.

5. **Hash verification**: Compute a hash of the serialized prefix at each breakpoint. Compare against the previous request's hash. Log a warning if they differ unexpectedly — this indicates a serialization stability bug.

6. **Stable tool definitions**: If tool definitions are included in Layer 1, they must not change mid-conversation. Version-pin tool definitions at conversation start, or exclude dynamically discovered tools from the cached prefix.

### 3.3 Cache-Breaking Events

A comprehensive catalog of events that invalidate prefix caches:

| Event | Cache impact | Mitigation |
|-------|-------------|------------|
| Tool definition change | Invalidates tools + system + messages (entire hierarchy) | Version-pin tools at session start |
| `tool_choice` parameter change | Invalidates messages cache | Keep tool_choice stable within a session |
| System prompt update | Invalidates system + messages | Batch system prompt changes; treat as session restart |
| Image content added/removed | Invalidates entire message-level cache | Place images in Layer 4 only |
| Extended thinking toggled | Invalidates message-level cache | Keep thinking settings stable per session |
| Compaction runs | Invalidates Layer 2+ caches | Limit compaction frequency; amortize write costs |
| Graph schema migration | Invalidates all caches (serialization format changes) | Treat as session restart |
| Serialization bug | Silent cache degradation | Hash verification on every request |
| TTL expiry (>5min pause) | All caches expire | Use 1hr TTL for background agents; accept for interactive use |
| Provider load shedding | Early eviction (best-effort) | No mitigation; design for graceful degradation |

### 3.4 Compaction Strategy for Cache Preservation

Compaction is the most disruptive operation for caching: it replaces a sequence of messages with a shorter summary. The key insight is to design compaction so that it **extends the stable prefix rather than disrupting it**.

**Compact from the middle, not the front.** Layer 1 (system/tools) never changes. Compaction operates on Layer 3 messages, producing a new summary node that joins Layer 2. The existing Layer 2 summaries remain untouched — the new summary is appended after them.

**Immutable compaction output.** Once a compaction summary is created, it is permanent. If further compaction is needed, a new "meta-summary" summarizes existing summaries — but the originals remain in the graph (marked as superseded, not deleted from the serialized prefix).

**Compaction boundary alignment.** Compaction windows should not cross cache breakpoint boundaries. Compact only messages within Layer 3, producing output that joins Layer 2.

**Minimum size threshold.** Only run compaction when the resulting summary would meet the minimum cacheable size for the target model (4,096 tokens for Opus 4.6; 1,024 for Sonnet 4). Summaries below this threshold provide no caching benefit and should be deferred until more material accumulates.

**Compaction cost analysis.** After compaction, the next request pays the cache write premium (1.25x for 5min TTL) on the new Layer 2 content. All subsequent Layer 3 content is also invalidated and must be re-cached. For a 100K-token conversation where compaction produces a 20K summary: the first post-compaction request pays 1.25x on ~70K tokens (new Layer 2 + Layer 3). If compaction runs every 20 messages in a 100-message conversation, that's 5 compaction events, each invalidating ~70% of cached content. **Compaction frequency must be tuned against cache amortization**: fewer, larger compaction events are more cache-friendly than frequent small ones.

**Large tool results.** Tool results in Layer 4 (e.g., 10K-token file reads, git diffs) pay full input token cost on every subsequent turn they remain in context. Consider summarizing large tool results into Layer 2 immediately, or evicting them from context after N turns to prevent unbounded cost growth.

### 3.5 Multi-Agent Cache Sharing

In a multi-agent system, cache sharing across agents is a significant cost multiplier:

**Layer 1 sharing.** All agents in a project should use identical Layer 1 content (system prompt, tool definitions, project context). On Anthropic, cache hits work across requests from the same **workspace** (not organization — this changed in February 2026). All agents must be configured to use the same workspace for shared caching.

**Caveat: tool set divergence.** If Agent A uses tools that Agent B does not, their Layer 1 content differs and no cache sharing occurs. For maximum sharing, all agents in a project should share the same tool set, even if individual agents only use a subset.

**Layer 2 partial sharing.** Agents working on related tasks may share compacted history (e.g., a handoff summary). Standardizing the serialization format for handoff summaries enables cross-agent Layer 2 cache hits.

**Layer 3+ divergence.** Each agent's conversation history is unique. No cache sharing expected here, but each agent benefits from its own turn-to-turn prefix caching.

**Cache warming for new agents.** When spawning a new agent, send a "priming" request that establishes the Layer 1 cache. The agent's first real request then gets a cache hit on the shared prefix. The arXiv paper [2601.06007](https://arxiv.org/abs/2601.06007) specifically recommends this pattern.

---

## 4. Research Validation: "Don't Break the Cache"

The paper ["Don't Break the Cache: An Evaluation of Prompt Caching for Long-Horizon Agentic Tasks"](https://arxiv.org/abs/2601.06007) (arXiv 2601.06007, January 2026) provides the most rigorous evaluation of prompt caching strategies for agentic workloads to date. It evaluates on DeepResearch Bench, a multi-turn agentic benchmark where agents autonomously execute web search tool calls.

### Key findings

- **Cost reduction**: 41-80% across three major providers (OpenAI, Anthropic, Google)
- **Latency improvement**: 13-31% reduction in time to first token (TTFT)
- **Strategy comparison**: Three strategies tested — full context caching, system prompt only caching, and caching that excludes dynamic tool results
- **Critical insight**: "Strategic cache boundary control, such as caching only system prompts while excluding dynamic tool results, provides more consistent benefits than naive full context caching"
- **Counter-intuitive finding**: Naive "cache everything" approaches can **paradoxically increase latency** due to cache write overhead on frequently-changing content. This is one of the paper's most important results.
- **System prompt size matters**: The evaluation used 10K-token system prompts with ablation studies from 500-50,000 tokens. Results vary for very small or very large system prompts.

### Implications for Context Manager

This directly validates our layered approach:

1. **Layer 1 caching (system + tools) provides the most consistent savings** — it's stable across all turns and all agents. This should be the first and most important optimization.

2. **Full context caching has diminishing returns for agentic tasks** — tool results are inherently dynamic and break prefix continuity. Our Layer 4 (volatile) explicitly excludes these from caching expectations.

3. **The paper confirms that naive "cache everything" approaches sometimes increase costs** due to cache write premiums (1.25x) on content that changes too frequently to benefit from reads. Our tiered approach avoids this by only placing breakpoints at natural stability boundaries.

---

## 5. How Other Systems Handle Caching

| System | Caching Approach | Strengths | Weaknesses |
|--------|-----------------|-----------|------------|
| **Claude Code** | Explicit `cache_control` breakpoints; append-only mid-conversation changes; SEV-level monitoring of cache hit rates | Production-proven, ~4K token system prompt always cached, caching-first design philosophy | Proprietary implementation details |
| **Cursor / Copilot** | RAG + prefix caching on retrieved context | Good for code context, provider-managed | Retrieved context changes per query, limiting prefix reuse |
| **Google ADK** | Session-layer compaction; "context as compiled view"; stable/variable zone split | Explicit "working context" concept; closest to our layered model | Compaction can disrupt cached prefixes |
| **OpenAI Codex** | Stable system prompt + deterministic tool ordering; `prompt_cache_key` routing | Cache-aware prompt structure as "first-class performance surface" | MCP tool ordering bug caused cache misses (validates fragility concern) |
| **LangChain** | Semantic caching via Redis (LangCache) | Good for repeated/similar queries (~73% savings) | Not effective for unique agentic turns |
| **Letta (MemGPT)** | OS-inspired memory hierarchy: core (always in context), archival (on-demand), recall (pageable) | Tiered model parallels our 4-layer approach; Context Repositories with git-based versioning | Focused on context window management, not prefix caching specifically |
| **Beads / Gastown** | No explicit caching strategy documented | N/A | Token costs unaddressed despite $100-200/hr burn rate at full scale |

**Notable patterns**:
- The Codex agent loop treats prompt structure as a "first-class performance surface" — system instructions, tool definitions, sandbox config, and environment context are kept identical and consistently ordered. The team discovered a bug where MCP tools were not enumerated in consistent order, causing cache misses. This is a concrete example of the serialization fragility described in Section 2.2.
- Google ADK's thesis that "context is a compiled view over a richer stateful system" closely parallels our graph-to-prompt construction model.
- Letta's core/archival/recall memory tiers map roughly to our Layer 1/Layer 2/Layer 3-4, validating the tiered approach from a memory management perspective.

---

## 6. Self-Hosted Models: Prefix Caching for Local Inference

If Context Manager supports local models in the future (VISION.md mentions Ollama and DeepSeek for background processing), the same prefix stability principles apply to self-hosted inference engines:

**vLLM**: Uses block-level hashing (PagedAttention) for automatic prefix caching. Fixed-size pages of typically 16 tokens. No developer intervention needed — stable prefixes are cached automatically. Source: [vLLM Automatic Prefix Caching](https://docs.vllm.ai/en/stable/design/prefix_caching/)

**SGLang**: Uses RadixAttention — a radix tree structure that achieves 85-95% cache hit rates for few-shot learning and 75-90% for multi-turn chat. SGLang outperforms vLLM by ~29% on multi-turn workloads specifically because of superior prefix caching. Source: [SGLang vs vLLM Prefix Caching](https://medium.com/byte-sized-ai/prefix-caching-sglang-vs-vllm-token-level-radix-tree-vs-block-level-hashing-b99ece9977a1)

**Distributed challenges**: When moving from single-instance to distributed clusters, the KV-cache becomes disaggregated. Each pod manages its own cache in isolation. Standard load balancers scatter related requests across pods, destroying cache locality. Projects like [llm-d](https://llm-d.ai/blog/kvcache-wins-you-can-see) are building global KV-cache views across replicas.

**Implication for Context Manager**: Our 4-layer prompt model benefits self-hosted models equally — stable prefixes hit the automatic prefix cache in vLLM/SGLang. No code changes needed beyond what we'd already implement for API-based caching.

---

## 7. Implementation Strategy for Context Manager

### 7.1 Implementation Prerequisites

Before implementing caching, several structural changes are needed in the API layer:

| Change | Files | Effort | Notes |
|--------|-------|--------|-------|
| Update `anthropic-version` header | `src/llm/anthropic.rs` | 15 min | Current `"2023-06-01"` is oldest supported; newer version needed for cache features |
| Convert `system` to structured content blocks | `src/llm/anthropic.rs`, `src/llm/mod.rs` | 2-4 hrs | Change from `Option<String>` to `Vec<ContentBlock>` with `cache_control` support |
| Convert `ChatMessage.content` to content blocks | `src/llm/mod.rs`, all providers | 4-8 hrs | Change from `String` to `Vec<ContentBlock>` to enable per-message `cache_control`. This propagates through the `LlmProvider` trait — need to decide if cache hints belong in generic `ChatConfig` or provider-specific extensions. |
| Parse `message_start` SSE event | `src/llm/anthropic.rs` | 1-2 hrs | Currently ignored (test: "test_parse_message_start_ignored"). Must extract `cache_creation_input_tokens` and `cache_read_input_tokens` from usage payload. |
| Surface cache metrics in TUI | `src/tui/` | 2-4 hrs | Display cache hit/miss/write counts per request and cumulative per session |

### 7.2 Short-Term: Immediate Wins

**Add `cache_control` breakpoints to Anthropic API calls.** After completing the prerequisites above, place breakpoints:
- After the system content block (Layer 1)
- After the penultimate user message (Layer 3 prefix reuse)

**Ensure deterministic serialization.** `build_context()` in `src/app.rs` already traverses the graph deterministically. Audit for any non-deterministic elements. Key risk: any future code that serializes `HashMap`-backed data structures into prompt content.

**Track cache metrics.** Parse `cache_creation_input_tokens` and `cache_read_input_tokens` from the `message_start` SSE event. Log per-request and surface cumulative stats.

**Estimated savings**: 40-70% on input token costs, depending on system prompt size and model (must exceed minimum cacheable threshold: 4,096 tokens for Opus 4.6).

### 7.3 Medium-Term: Cache-Aware Compaction

**Design compaction to produce immutable summary nodes.** When compaction runs on Layer 3 messages, it creates a new `CompactedMessage` node in the graph. This node is placed in Layer 2 during serialization — after the system prompt but before the recent conversation.

**Cache breakpoint after Layer 2.** Add a second `cache_control` breakpoint after the compacted summary block. This means the system prompt + compacted history prefix is cached together, and only the recent conversation is uncached.

**Compaction frequency tuning.** Use cache hit/miss metrics (from 7.2) to determine optimal compaction frequency. If cache writes are frequent but reads are rare, compaction is running too often. Target: compaction no more often than once every 15-20 messages to ensure write costs are amortized across many reads.

**Estimated savings**: Additional 10-25% beyond 7.2, depending on conversation length and compaction quality. Less than previously estimated due to compaction-induced cache invalidation costs.

### 7.4 Long-Term: Multi-Agent Cache Architecture

**Shared project context graph.** All agents in a project reference the same Layer 1 subgraph (system directives, tool definitions, project context). Serialization produces identical bytes for all agents. All agents must share the same tool set and same Anthropic workspace.

**Per-agent conversation subgraph.** Each agent maintains its own conversation branch in the graph. Layer 3-4 content diverges, but Layer 1-2 cache is shared.

**Agent handoff protocol.** When Agent A hands off to Agent B, A's compacted history becomes part of B's Layer 2. The handoff summary is designed to be cache-compatible: deterministic format, immutable content.

**Cache warming on spawn.** When a new agent is created, issue a minimal "priming" request with just Layer 1 content to establish the cache. The agent's first real request then benefits from a cache hit on the shared prefix.

**Estimated savings**: Significant for Layer 1 sharing (scales linearly with agent count), but dependent on all agents using the same workspace, tool set, and system prompt. Realistic improvement is 1.5-3x beyond individual caching.

### 7.5 Graph Metadata for Cache Optimization

Extend the graph model to support cache-aware context construction:

- **`cache_layer` annotation on nodes**: Classify each node as belonging to Layer 1 (stable), Layer 2 (session-stable), Layer 3 (conversation), or Layer 4 (volatile)
- **Serialization hash tracking**: Store the hash of the serialized prefix at each breakpoint boundary. Detect prefix drift across requests.
- **Cache metrics per session**: Track cumulative cache hit/miss/write counts. Feed into compaction decisions — e.g., defer compaction if it would invalidate a hot cache.

---

## 8. Red Team / Green Team

### Green Team (Validates Approach)

- **Proven at scale**: Provider prefix caching is battle-tested. Anthropic reports up to 90% cost reduction; the arXiv paper independently confirms 41-80% for agentic workloads. Claude Code's entire pricing model depends on caching working reliably.
- **Architectural alignment**: Our graph architecture gives us unique control over serialization order, node filtering, and content placement. Most chat frameworks don't have this leverage. Google ADK's "context as compiled view" philosophy independently validates the approach.
- **Natural layer mapping**: The 4-layer model aligns with both our node type hierarchy and Anthropic's breakpoint mechanism. Letta's core/archival/recall tiers independently converge on a similar tiered structure.
- **Immutable compaction is cache-friendly**: Producing new summary nodes (rather than mutating existing content) is both a good graph design principle and a cache stability requirement. The two concerns reinforce each other. This is the document's strongest original insight — no other system we found explicitly designs compaction to be cache-preserving.
- **Multi-agent multiplier**: In a multi-agent system, shared Layer 1 caching means the system prompt is computed once and reused across all agents (within the same workspace). The savings grow with agent count.
- **Incremental adoption**: Each layer can be implemented independently. Layer 1 caching provides value on its own; subsequent layers add additional savings.
- **Self-hosted compatibility**: The same prefix stability principles benefit vLLM/SGLang automatic prefix caching, future-proofing the design for local model support.

### Red Team (Challenges)

- **100% exact match is fragile**: Any serialization drift — a library update that changes JSON formatting, a new field added to a struct, even whitespace differences — breaks the cache silently. There's no partial match. The Codex MCP tool ordering bug is a concrete example.
- **Compaction disrupts Layer 2**: Every compaction event invalidates Layer 2+ caches. In a 100-message conversation with compaction every 20 messages, that's 5 invalidation events each affecting ~70% of cached content. The net savings may be significantly less than the optimistic estimates.
- **5-minute TTL is short**: If the user pauses for more than 5 minutes between messages, the cache expires. The 1-hour TTL costs 2x for writes (not 1.25x). For interactive use this may be acceptable; for background agents with long think times, it's a real cost.
- **High minimum thresholds for newer models**: Opus 4.6 requires 4,096 tokens minimum (not 1,024). If our system prompt is under 4,096 tokens, Layer 1 caching on Opus 4.6 provides zero benefit. This is a materially higher bar than older models.
- **Structured content block migration is non-trivial**: Moving from `system: Option<String>` and `content: String` to structured content block arrays propagates through the entire `LlmProvider` abstraction layer. This is a significant refactor, not a "minimal code change."
- **Tool definition churn**: Dynamic tool discovery can invalidate the entire cache hierarchy. Tools must be version-pinned at session start or excluded from the cached prefix.
- **Image content invalidation**: Adding or removing images anywhere in the prompt invalidates the entire message-level cache. Any future multimodal support must account for this.
- **Extended thinking invalidation**: Toggling thinking or changing the budget invalidates message caches. Thinking settings must be stable per session.
- **Workspace-level isolation (Feb 2026)**: Multi-agent cache sharing only works within a single Anthropic workspace, not across the organization. This limits the multi-agent sharing strategy.
- **20-block lookback limit**: Long conversations may exceed the automatic prefix checking window, requiring additional breakpoint slots and complicating the layered model.
- **Multi-provider abstraction**: Anthropic, OpenAI, and Google have fundamentally different caching APIs. Supporting all three requires an abstraction layer that may limit provider-specific optimizations.
- **Cache eviction is best-effort**: Providers don't guarantee caches persist for the full TTL under high load. Savings estimates should assume <100% cache hit rates.
- **RAG and dynamic context injection**: Future features that inject retrieved documents or relevant work items into the prompt will challenge prefix stability. These dynamic elements must be placed in Layer 3-4, not Layer 1-2.
- **Observability gap**: Cache hits/misses are only visible in API response metadata. The current SSE parser ignores `message_start` events entirely. Without monitoring, silent cache degradation can go unnoticed.

---

## 9. Recommendations

Ordered by ROI (highest first):

1. **Add `cache_control` breakpoints** to the Anthropic provider. This is the single highest-ROI action: the system prompt is already stable, and the structural changes (content blocks, SSE parsing) lay the foundation for all subsequent optimizations. Estimated effort: 1-2 days.

2. **Track cache metrics** from API responses. Deploy alongside #1 to verify it works. Parse `cache_creation_input_tokens` and `cache_read_input_tokens` from `message_start` SSE events. Surface in TUI or logs. Without observability, all other optimizations are flying blind. Estimated effort: 0.5 days.

3. **Adopt the 4-layer prompt model** as the canonical architecture for context construction. All components that produce LLM input should classify their output by layer. This frames all subsequent work. Estimated effort: design doc + refactor of `build_context()`.

4. **Implement deterministic serialization** with hash verification. Compute and log a hash of the serialized prefix at each breakpoint. Alert on unexpected changes. Guard against HashMap iteration order leaking into prompts. Estimated effort: 1 day.

5. **Treat serialization order as a breaking change.** Any change that alters the order or content of the serialized prompt should be reviewed as carefully as an API breaking change — it invalidates all active caches. Adopt this as a process rule immediately.

6. **Design compaction to produce immutable summary nodes** that become stable Layer 2 content. Never modify an existing summary — create new ones. Tune compaction frequency against cache amortization (target: ≤1 compaction per 15-20 messages). Estimated effort: part of compaction feature design.

7. **Standardize Layer 1 across agents** in multi-agent scenarios. Identical system prompt + tool definitions + same Anthropic workspace = shared cache across the agent fleet. Version-pin tool definitions at session start. Estimated effort: part of multi-agent feature design.

---

## 10. Sources

- [Anthropic Prompt Caching Documentation](https://platform.claude.com/docs/en/build-with-claude/prompt-caching)
- [Don't Break the Cache: An Evaluation of Prompt Caching for Long-Horizon Agentic Tasks (arXiv 2601.06007)](https://arxiv.org/abs/2601.06007)
- [OpenAI Prompt Caching 201](https://developers.openai.com/cookbook/examples/prompt_caching_201/)
- [Google ADK Context-Aware Multi-Agent Framework](https://developers.googleblog.com/architecting-efficient-context-aware-multi-agent-framework-for-production/)
- [Unrolling the Codex Agent Loop (OpenAI)](https://openai.com/index/unrolling-the-codex-agent-loop/)
- [How Prompt Caching Actually Works in Claude Code](https://www.claudecodecamp.com/p/how-prompt-caching-actually-works-in-claude-code)
- [Prompt Caching Is A Must: $720 to $72/month](https://medium.com/@labeveryday/prompt-caching-is-a-must-how-i-went-from-spending-720-to-72-monthly-on-api-costs-3086f3635d63)
- [Prompt Caching: The Secret to 60% Cost Reduction (Thomson Reuters Labs)](https://medium.com/tr-labs-ml-engineering-blog/prompt-caching-the-secret-to-60-cost-reduction-in-llm-applications-6c792a0ac29b)
- [vLLM Automatic Prefix Caching](https://docs.vllm.ai/en/stable/design/prefix_caching/)
- [SGLang vs vLLM: Prefix Caching Comparison](https://medium.com/byte-sized-ai/prefix-caching-sglang-vs-vllm-token-level-radix-tree-vs-block-level-hashing-b99ece9977a1)
- [KVFlow: Efficient Prefix Caching for Multi-Agent Workflows (arXiv 2507.07400)](https://arxiv.org/html/2507.07400v1)
- [Letta/MemGPT Memory Architecture](https://docs.letta.com/concepts/memgpt/)
- [Redis LLM Token Optimization](https://redis.io/blog/llm-token-optimization-speed-up-apps/)
- [How Prompt Caching Works: Paged Attention and Automatic Prefix Caching](https://sankalp.bearblog.dev/how-prompt-caching-works/)
- [How We Extended LLM Conversations by 10x with Intelligent Context Compaction](https://dev.to/amitksingh1490/how-we-extended-llm-conversations-by-10x-with-intelligent-context-compaction-4h0a)
- [Context Window Management Strategies for Long-Context AI Agents](https://www.getmaxim.ai/articles/context-window-management-strategies-for-long-context-ai-agents-and-chatbots/)
- [Prefix Caching (LLM Inference Handbook)](https://bentoml.com/llm/inference-optimization/prefix-caching)
- [KV-Cache Wins: From Prefix Caching to Distributed Scheduling (llm-d)](https://llm-d.ai/blog/kvcache-wins-you-can-see)
