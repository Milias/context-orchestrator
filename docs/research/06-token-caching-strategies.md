# Token Caching Strategies for Graph-Reconstructed Prompts

> Research conducted 2026-03-12 investigating LLM prompt caching strategies, provider APIs,
> and cache-optimized architectures for the Context Manager's graph-based context construction.

---

## Executive Summary

Context Manager reconstructs LLM input from graph nodes on every request — walking `RespondsTo` edges from leaf to root, extracting `Message` and `SystemDirective` nodes into a linear prompt. Without deliberate architectural design, this approach is **cache-hostile**: every prompt reconstruction risks producing a slightly different token prefix, invalidating provider caches and paying full input token costs on every turn.

Provider prefix caching (Anthropic, OpenAI, Google) offers **50-90% cost reduction** and **up to 85% latency reduction** for long prompts, but requires the prompt to begin with an **identical token sequence** across requests. A January 2026 [arXiv paper](https://arxiv.org/abs/2601.06007) confirms 41-80% cost reduction for agentic workloads when caching is applied correctly.

Our graph architecture is **uniquely positioned** to exploit prefix caching: we control serialization order, we decide which nodes enter the prompt and in what sequence, and we can design compaction to preserve cache-friendly prefixes. This document proposes a **4-layer prompt model** that maps naturally to provider cache breakpoints and to our graph's node type hierarchy.

**Key recommendation**: Treat prompt serialization order as a first-class architectural concern. Design every component that touches LLM input — context construction, compaction, multi-agent handoff — with prefix stability as a constraint.

---

## 1. The Caching Landscape

### 1.1 Provider Prefix Caching (Primary Focus)

Prefix caching exploits the fact that LLM inference computes key-value (KV) pairs for every input token through the model's attention layers. When multiple requests share an identical prefix, the KV computation for that prefix is redundant. Providers store the computed KV state and let subsequent requests skip recomputation, starting inference from the cached state.

**The fundamental constraint: prefix caching requires a 100% exact byte match.** Any difference in the prefix — a changed timestamp, reordered JSON keys, an extra whitespace character — invalidates the entire cache.

#### Anthropic (Claude)

- **Mechanism**: Explicit `cache_control` breakpoints (up to 4) on content blocks, or automatic caching
- **Cache hierarchy**: Tools → System → Messages (in that order). Changing a tool definition invalidates the system cache downstream
- **Minimum cacheable size**: 1,024 tokens (Opus 4, Sonnet 4, Sonnet 3.7); 2,048 tokens (Haiku 3.5)
- **TTL**: 5 minutes (default, refreshed on each hit) or 1 hour (at additional cost)
- **Pricing**: Cache writes cost 1.25x base input price; cache reads cost 0.1x base input price (90% discount)
- **Break-even**: 2+ cache hits per cached prefix
- **Concurrency note**: A cache entry becomes available only after the first response begins. Parallel requests should wait for the first response to establish the cache
- **Block limit**: Automatic prefix checking looks back ~20 content blocks from each explicit breakpoint. Prompts with >20 blocks before a breakpoint need additional breakpoints

Source: [Anthropic Prompt Caching Documentation](https://platform.claude.com/docs/en/build-with-claude/prompt-caching)

#### OpenAI

- **Mechanism**: Fully automatic, no developer-controlled breakpoints
- **Pricing**: 50% discount on cached input tokens
- **Scope**: Cache shared within an organization (not across orgs)
- **Routing hint**: Optional `prompt_cache_key` parameter improves routing to cache-warm instances. One customer improved cache hit rate from 60% to 87% using this parameter

Source: [OpenAI Prompt Caching 201](https://developers.openai.com/cookbook/examples/prompt_caching_201/)

#### Google (Gemini)

- **Implicit caching**: Activates automatically, no guaranteed cost savings
- **Explicit context caching**: Developer-created caches with guaranteed discounts, charged for storage
- **Use case**: Best for very large context (e.g., entire codebases) reused across multiple queries

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

Context construction in `src/app.rs:build_context` (lines 47-108):

1. Walk `RespondsTo` edges from the active branch's leaf node to the root
2. Extract `SystemDirective` nodes as the system prompt
3. Extract `Message` nodes as the conversation history (user/assistant turns)
4. Skip non-conversation nodes (`WorkItem`, `GitFile`, `Tool`, `BackgroundTask`)
5. If total tokens exceed the budget, remove oldest messages proportionally

The API call in `src/llm/anthropic.rs` (lines 56-90) sends:
- `system`: Optional string (from `SystemDirective`)
- `messages`: Vec of `{role, content}` objects
- No `cache_control` breakpoints — every request pays full input token cost

### 2.2 Why This Is Cache-Hostile Without Design

**Prefix instability from compaction**: When background compaction replaces a sequence of messages with a summary, the serialized content changes at that position. Every message after the compaction boundary shifts, invalidating the prefix from that point forward.

**Dynamic metadata in serialization**: If node metadata (timestamps, token counts, UUIDs) leaks into the serialized prompt, prefix stability is destroyed. Even `created_at` timestamps on otherwise identical content cause cache misses.

**Content position shifts**: As the architecture evolves to include more node types in the prompt (e.g., relevant `WorkItem` descriptions, `GitFile` diffs), inserting content at arbitrary positions shifts everything after it.

**Multi-agent divergence**: Different agents working on the same project may share system instructions but have completely different conversation histories. Without deliberate design, no prefix sharing occurs.

**Non-deterministic serialization**: If graph traversal order or JSON key ordering varies between reconstructions of the same logical state, the serialized bytes differ and caching fails.

### 2.3 Why Our Architecture Has an Advantage

Despite these risks, a graph-based context manager has structural advantages that most chat frameworks lack:

**We control serialization order.** The graph traversal is deterministic: `get_branch_history` in `src/graph.rs` (lines 276-305) always produces the same node sequence for the same branch state. This determinism is the foundation of prefix stability.

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
| 1 | After Layer 1 | All agents in project (same API key) |
| 2 | After Layer 2 | All turns within a session |
| 3 | After penultimate message | Adjacent turns |
| 4 | Automatic (Anthropic) | N/A |

**Cost model example** (10-turn conversation, 50K token system+tools, 20K compacted summary, 30K conversation):

| Scenario | Input tokens | Cost multiplier |
|----------|-------------|-----------------|
| No caching | 100K per turn = 1M total | 1.0x |
| Layer 1 cached | 50K at 0.1x + 50K at 1.0x per turn | ~0.55x |
| Layers 1+2 cached | 70K at 0.1x + 30K at 1.0x per turn | ~0.37x |
| Layers 1+2+3 cached | ~95K at 0.1x + ~5K at 1.0x per turn | ~0.14x |

The savings compound: on turn 10, only the latest user message and the delta from the previous turn are uncached.

### 3.2 Graph-to-Prompt Serialization Rules

To maintain prefix stability, the serialization pipeline must follow strict invariants:

1. **Deterministic byte output**: Same graph state produces the same serialized bytes. Use sorted keys in any structured data, stable formatting, no timestamps or request-specific metadata in serialized content.

2. **Append-only conversation growth**: New messages are always appended at the end of Layer 3. Never insert messages into the middle of the conversation.

3. **Immutable node content**: Once a node is created and serialized, its content never changes. Compaction creates *new* summary nodes rather than modifying existing ones.

4. **Stable layer boundaries**: Layer 1 content is determined by project configuration (changes require explicit user action). Layer 2 content is determined by compaction (changes only when compaction runs). These boundaries are not affected by conversation growth.

5. **Hash verification**: Compute a hash of the serialized prefix at each breakpoint. Compare against the previous request's hash. Log a warning if they differ unexpectedly — this indicates a serialization stability bug.

### 3.3 Compaction Strategy for Cache Preservation

Compaction is the most disruptive operation for caching: it replaces a sequence of messages with a shorter summary. The key insight is to design compaction so that it **extends the stable prefix rather than disrupting it**.

**Compact from the middle, not the front.** Layer 1 (system/tools) never changes. Compaction operates on Layer 3 messages, producing a new summary node that joins Layer 2. The existing Layer 2 summaries remain untouched — the new summary is appended after them.

**Immutable compaction output.** Once a compaction summary is created, it is permanent. If further compaction is needed, a new "meta-summary" summarizes existing summaries — but the originals remain in the graph (marked as superseded, not deleted from the serialized prefix).

**Compaction boundary alignment.** Compaction windows should not cross cache breakpoint boundaries. Compact only messages within Layer 3, producing output that joins Layer 2.

**Minimum size threshold.** Only run compaction when the resulting summary would be ≥1,024 tokens (Anthropic's minimum cacheable size). Summaries below this threshold provide no caching benefit and should be deferred until more material accumulates.

**Compaction triggers cache write.** After compaction, the next request pays the 1.25x cache write cost for the new Layer 2 content. Subsequent requests benefit from the 0.1x cache read discount. This means compaction should be infrequent enough that the write cost is amortized across many subsequent reads.

### 3.4 Multi-Agent Cache Sharing

In a multi-agent system, cache sharing across agents is a significant cost multiplier:

**Layer 1 sharing.** All agents in a project should use identical Layer 1 content (system prompt, tool definitions, project context). On Anthropic, cache hits work across requests from the same API key — so if Agent A establishes the Layer 1 cache, Agent B's next request gets a cache hit for free.

**Layer 2 partial sharing.** Agents working on related tasks may share compacted history (e.g., a handoff summary). Standardizing the serialization format for handoff summaries enables cross-agent Layer 2 cache hits.

**Layer 3+ divergence.** Each agent's conversation history is unique. No cache sharing expected here, but each agent benefits from its own turn-to-turn prefix caching.

**Cache warming for new agents.** When spawning a new agent, send a "priming" request that establishes the Layer 1 cache. The agent's first real request then gets a cache hit on the shared prefix. The arXiv paper [2601.06007](https://arxiv.org/abs/2601.06007) specifically recommends this pattern.

---

## 4. Research Validation: "Don't Break the Cache"

The paper ["Don't Break the Cache: An Evaluation of Prompt Caching for Long-Horizon Agentic Tasks"](https://arxiv.org/abs/2601.06007) (arXiv 2601.06007, January 2026) provides the most rigorous evaluation of prompt caching strategies for agentic workloads to date.

### Key findings

- **Cost reduction**: 41-80% across three major providers (OpenAI, Anthropic, Google)
- **Latency improvement**: 13-31% reduction in time to first token (TTFT)
- **Strategy comparison**: Three strategies tested — full context caching, system prompt only caching, and caching that excludes dynamic tool results
- **Critical insight**: "Strategic cache boundary control, such as caching only system prompts while excluding dynamic tool results, provides more consistent benefits than naive full context caching"

### Implications for Context Manager

This directly validates our layered approach:

1. **Layer 1 caching (system + tools) provides the most consistent savings** — it's stable across all turns and all agents. This should be the first and most important optimization.

2. **Full context caching has diminishing returns for agentic tasks** — tool results are inherently dynamic and break prefix continuity. Our Layer 4 (volatile) explicitly excludes these from caching expectations.

3. **The paper found that naive "cache everything" approaches sometimes increase costs** due to cache write premiums (1.25x) on content that changes too frequently to benefit from reads. Our tiered approach avoids this by only placing breakpoints at natural stability boundaries.

---

## 5. How Other Systems Handle Caching

| System | Caching Approach | Strengths | Weaknesses |
|--------|-----------------|-----------|------------|
| **Claude Code** | Automatic prefix caching; stable system prompt ordering | Zero configuration, reliable Layer 1 caching | No explicit optimization for conversation growth |
| **Cursor / Copilot** | RAG + prefix caching on retrieved context | Good for code context, provider-managed | Retrieved context changes per query, limiting prefix reuse |
| **Google ADK** | Session-layer compaction; stable/variable zone split | Explicit "working context" concept; closest to our layered model | Compaction can disrupt cached prefixes |
| **OpenAI Codex** | Stable system prompt + tool ordering; `prompt_cache_key` routing | Cache-aware prompt structure as first-class concern | Limited to OpenAI ecosystem |
| **LangChain** | Semantic caching via Redis (LangCache) | Good for repeated/similar queries (~73% savings) | Not effective for unique agentic turns |
| **Beads / Gastown** | No explicit caching strategy documented | N/A | Token costs unaddressed despite heavy LLM usage in compaction |

**Notable pattern**: The Codex agent loop treats prompt structure as a "first-class performance surface" — system instructions, tool definitions, sandbox config, and environment context are kept identical and consistently ordered between requests. This validates our approach of treating serialization order as an architectural concern.

---

## 6. Implementation Strategy for Context Manager

### 6.1 Short-Term: Immediate Wins (Current Architecture)

**Add `cache_control` breakpoints to Anthropic API calls.** The `MessagesRequest` struct in `src/llm/anthropic.rs` currently sends a plain `system: Option<String>`. Changing this to Anthropic's structured system content blocks with `cache_control` markers requires:

- Convert `system` from `Option<String>` to a structured content block array
- Add `cache_control: {"type": "ephemeral"}` after the system content block
- Optionally add a breakpoint after the penultimate user message

**Ensure deterministic serialization.** `build_context()` in `src/app.rs` already traverses the graph deterministically. Verify that no non-deterministic elements (HashMap iteration order, floating-point formatting) affect serialized output.

**Track cache metrics.** Anthropic's response includes `cache_creation_input_tokens` and `cache_read_input_tokens` fields. Parse these in the SSE stream handler and surface them in the TUI (or at minimum, log them).

**Estimated savings**: 50-70% on input token costs for conversations with system prompts >1,024 tokens.

### 6.2 Medium-Term: Cache-Aware Compaction

**Design compaction to produce immutable summary nodes.** When compaction runs on Layer 3 messages, it creates a new `CompactedMessage` node in the graph. This node is placed in Layer 2 during serialization — after the system prompt but before the recent conversation.

**Cache breakpoint after Layer 2.** Add a second `cache_control` breakpoint after the compacted summary block. This means the system prompt + compacted history prefix is cached together, and only the recent conversation is uncached.

**Compaction frequency tuning.** Use cache hit/miss metrics (from 6.1) to determine optimal compaction frequency. If cache writes are frequent but reads are rare, compaction is running too often.

**Estimated savings**: Additional 15-30% beyond 6.1, depending on conversation length and compaction quality.

### 6.3 Long-Term: Multi-Agent Cache Architecture

**Shared project context graph.** All agents in a project reference the same Layer 1 subgraph (system directives, tool definitions, project context). Serialization produces identical bytes for all agents.

**Per-agent conversation subgraph.** Each agent maintains its own conversation branch in the graph. Layer 3-4 content diverges, but Layer 1-2 cache is shared.

**Agent handoff protocol.** When Agent A hands off to Agent B, A's compacted history becomes part of B's Layer 2. The handoff summary is designed to be cache-compatible: deterministic format, immutable content.

**Cache warming on spawn.** When a new agent is created, issue a minimal "priming" request with just Layer 1 content to establish the cache. The agent's first real request then benefits from a cache hit on the shared prefix.

**Estimated savings**: 2-5x cost reduction per agent beyond individual caching, due to shared prefix amortization across the agent fleet.

### 6.4 Graph Metadata for Cache Optimization

Extend the graph model to support cache-aware context construction:

- **`cache_layer` annotation on nodes**: Classify each node as belonging to Layer 1 (stable), Layer 2 (session-stable), Layer 3 (conversation), or Layer 4 (volatile)
- **Serialization hash tracking**: Store the hash of the serialized prefix at each breakpoint boundary. Detect prefix drift across requests.
- **Cache metrics per session**: Track cumulative cache hit/miss/write counts. Feed into compaction decisions — e.g., defer compaction if it would invalidate a hot cache.

---

## 7. Red Team / Green Team

### Green Team (Validates Approach)

- **Proven at scale**: Provider prefix caching is battle-tested. Anthropic reports up to 90% cost reduction; the arXiv paper independently confirms 41-80% for agentic workloads.
- **Architectural alignment**: Our graph architecture gives us unique control over serialization order, node filtering, and content placement. Most chat frameworks don't have this leverage.
- **Natural layer mapping**: The 4-layer model maps directly to both our node type hierarchy and Anthropic's 4-breakpoint limit. This isn't forced — it's a natural structural alignment.
- **Immutable compaction is cache-friendly**: Producing new summary nodes (rather than mutating existing content) is both a good graph design principle and a cache stability requirement. The two concerns reinforce each other.
- **Multi-agent multiplier**: In a 10-agent system, shared Layer 1 caching means the 50K-token system prompt is computed once and reused across all agents. The savings grow linearly with agent count.
- **Incremental adoption**: Layer 1 caching can be added immediately with minimal code changes. Each subsequent layer adds value independently.

### Red Team (Challenges)

- **100% exact match is fragile**: Any serialization drift — a library update that changes JSON formatting, a new field added to a struct, even whitespace differences — breaks the cache silently. There's no partial match.
- **Compaction disrupts Layer 2**: Every time compaction runs, the Layer 2 prefix changes, invalidating the Layer 2 cache. If compaction runs too frequently, the cache write premium (1.25x) may exceed the savings.
- **5-minute TTL is short**: If the user pauses for more than 5 minutes between messages, the cache expires. The 1-hour TTL option costs more. For interactive use, this may not be a problem; for background agents with long think times, it could be.
- **Minimum 1,024 tokens**: Small system prompts can't be cached. If our system prompt is under 1,024 tokens, Layer 1 caching provides zero benefit. This sets a floor on prompt complexity.
- **Multi-provider abstraction**: Anthropic, OpenAI, and Google have fundamentally different caching APIs. Supporting all three requires an abstraction layer that may limit provider-specific optimizations (e.g., OpenAI's `prompt_cache_key`).
- **RAG and dynamic context injection**: Future features that inject retrieved documents or relevant work items into the prompt will challenge prefix stability. These dynamic elements need to be placed carefully (in Layer 3-4, not Layer 1-2).
- **Concurrency hazards**: Parallel agent requests may not benefit from shared caching if the first request hasn't completed yet (Anthropic constraint). Cache warming mitigates this but adds latency.
- **Observability gap**: Cache hits/misses are only visible in API response metadata. Without monitoring, silent cache degradation (e.g., from a serialization bug) can go unnoticed for a long time.

---

## 8. Recommendations

1. **Adopt the 4-layer prompt model** as the canonical architecture for context construction. All components that produce LLM input should classify their output by layer.

2. **Implement deterministic serialization** with hash verification. Compute and log a hash of the serialized prefix at each breakpoint. Alert on unexpected changes.

3. **Add `cache_control` breakpoints** to the Anthropic provider immediately. This is the highest-ROI change: minimal code, 50-70% savings.

4. **Design compaction to produce immutable summary nodes** that become stable Layer 2 content. Never modify an existing summary — create new ones.

5. **Track cache metrics** (`cache_creation_input_tokens`, `cache_read_input_tokens`) from API responses. Surface in TUI or logs. Use to inform compaction frequency.

6. **Standardize Layer 1 across agents** in multi-agent scenarios. Identical system prompt + tool definitions = shared cache across the agent fleet.

7. **Treat serialization order as a breaking change.** Any PR that changes the order of content in the serialized prompt should be reviewed as carefully as an API breaking change — it invalidates all active caches.

---

## 9. Sources

- [Anthropic Prompt Caching Documentation](https://platform.claude.com/docs/en/build-with-claude/prompt-caching)
- [Don't Break the Cache: An Evaluation of Prompt Caching for Long-Horizon Agentic Tasks (arXiv 2601.06007)](https://arxiv.org/abs/2601.06007)
- [OpenAI Prompt Caching 201](https://developers.openai.com/cookbook/examples/prompt_caching_201/)
- [Google ADK Context-Aware Multi-Agent Framework](https://developers.googleblog.com/architecting-efficient-context-aware-multi-agent-framework-for-production/)
- [Redis LLM Token Optimization](https://redis.io/blog/llm-token-optimization-speed-up-apps/)
- [Prompt Caching: The Secret to 60% Cost Reduction (Thomson Reuters Labs)](https://medium.com/tr-labs-ml-engineering-blog/prompt-caching-the-secret-to-60-cost-reduction-in-llm-applications-6c792a0ac29b)
- [How Prompt Caching Works: Paged Attention and Automatic Prefix Caching](https://sankalp.bearblog.dev/how-prompt-caching-works/)
- [How We Extended LLM Conversations by 10x with Intelligent Context Compaction](https://dev.to/amitksingh1490/how-we-extended-llm-conversations-by-10x-with-intelligent-context-compaction-4h0a)
- [Context Window Management Strategies for Long-Context AI Agents](https://www.getmaxim.ai/articles/context-window-management-strategies-for-long-context-ai-agents-and-chatbots/)
- [LLM Cost Optimization: Reducing AI Expenses by 80%](https://ai.koombea.com/blog/llm-cost-optimization)
- [How to Reduce LLM Costs by 40% in 24 Hours](https://scalemind.ai/blog/reduce-llm-costs)
- [Prefix Caching (LLM Inference Handbook)](https://bentoml.com/llm/inference-optimization/prefix-caching)
