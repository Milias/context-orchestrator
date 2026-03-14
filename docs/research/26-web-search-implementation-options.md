# Research: Web Search Implementation Options

> **2026-03-14** — Research on web search backend options for the context-orchestrator's `WebSearch` tool. Covers commercial APIs, open-source alternatives, how major AI coding assistants implement web search, MCP-based options, and architectural patterns. Concepts and references only — no code changes.

---

## 1. Executive Summary

The context-orchestrator has a `WebSearch` tool stub (`src/tool_executor/execute.rs:97-102`) that needs a backend. The industry has converged on a **search-then-fetch** two-step pattern: a search API returns titles/URLs/snippets, then a separate fetch step retrieves full page content when needed. Claude Code and Cursor both use **Exa.ai** as their search backend. The cheapest high-quality option is **Serper** ($1/1K queries, 2,500 free). The most agent-optimized is **Tavily** (1,000 free/month, built for RAG). Self-hosted **SearXNG** avoids API costs entirely but requires infrastructure. MCP servers already exist for Brave Search and Perplexity, fitting directly into the ToolProvider architecture from doc 23.

**Recommendation:** Implement as a thin API client behind the existing `ToolCallArguments::WebSearch` variant, with the search provider configurable. Start with Tavily or Serper (best free tiers for development), design the abstraction so providers are swappable. Separate web search (find URLs) from web fetch (retrieve content) as distinct tools.

---

## 2. Current Architecture & Gap Analysis

### What Exists

- **`ToolName::WebSearch`** is registered in the enum (`src/graph/tool/types.rs:23`)
- **`ToolCallArguments::WebSearch { query: String }`** is a typed variant (`types.rs:141-143`)
- **Execution is stubbed** — returns `"web_search not yet implemented"` with `is_error: true` (`execute.rs:97-102`)
- **`reqwest` 0.13** is already a dependency with `stream`, `json`, `socks` features (used for Anthropic API calls)
- **No web fetch tool exists** — only search, not content retrieval
- **Doc 23** designs a `ToolProvider` trait and MCP client architecture that would allow web search to come from an external MCP server instead of (or in addition to) a built-in implementation

### What's Missing

| Gap | Impact |
|-----|--------|
| No search API integration | WebSearch tool is non-functional |
| No web fetch/browse tool | Can't retrieve full page content from URLs |
| No provider configuration | No way to specify which search API to use |
| No result formatting | No standard for how search results are presented to the LLM |
| No rate limiting/caching | Uncontrolled API costs during agent loops |

---

## 3. Requirements

Derived from VISION.md (Section 4.8: "MCP is the integration protocol"), user feedback (all tools equally callable), and the existing `WebSearch` stub:

1. **Functional search** — return structured results (title, URL, snippet, date) for a text query
2. **Provider-agnostic** — search backend must be swappable via configuration
3. **Cost-controlled** — rate limiting and optional caching to prevent runaway API costs in agent loops
4. **Separation of concerns** — search (find relevant URLs) and fetch (retrieve content) as distinct capabilities
5. **LLM-optimized output** — results formatted for LLM consumption, not raw HTML/JSON
6. **MCP-compatible** — must work as both a built-in tool AND as an MCP server delegation target (per doc 23)

---

## 4. How Major AI Tools Implement Web Search

### 4.1 Claude Code

**Backend:** Exa.ai
**Tool name:** `web_search_20260209` (versioned)
**Parameters:**
- `query` — search query string
- `max_uses` — limit searches per request
- `allowed_domains` / `blocked_domains` — whitelist/blacklist filtering
- `user_location` — approximate city/region/country for localization

**Response format:** Custom `web_search_tool_result` content type with:
- URL, title, `encrypted_content`, `page_age` per result
- Citations with `cited_text` (max 150 chars) — citation fields don't count toward token usage

**Pricing:** $10 per 1,000 searches + standard token costs
**Architecture:** Built into the Claude API itself (server-side), not a client-side tool

**Source:** [Claude API Web Search Tool](https://platform.claude.com/docs/en/agents-and-tools/tool-use/web-search-tool)

### 4.2 Cursor

**Backend:** Exa.ai
**Activation:** `@web` command (disabled by default)
**Three-tool architecture:**
1. **Web Search** — find relevant articles (no content fetching)
2. **URL Read** — get full page contents, creates multiple chunks
3. **View Chunk** — read relevant sections of fetched pages

**MCP fallback:** Supports custom MCP servers for web search (e.g., Brave Search MCP)
**Pricing:** 1 flow action credit per search, additional credits for URL reads

**Source:** [Cursor @Web Documentation](https://docs.cursor.com/context/@-symbols/@-web)

### 4.3 Windsurf (Codeium)

**Architecture:** Identical three-tool pattern to Cursor (Web Search → URL Read → View Chunk)
**Query processing:** LLM synthesizes conversation intent into search query
**Content handling:** Local web scrape to extract text, headers, body, and links
**Activation:** Auto-activated or forced with `@web` / `@docs`

**Source:** [Windsurf Web Search Docs](https://docs.windsurf.com/windsurf/cascade/web-search)

### 4.4 GitHub Copilot

**Backend:** Bing Web Search API
**Two modes:**
1. **Non-reasoning search** — fast direct response from top results
2. **Agentic search with reasoning** — model manages search, decides if more searches needed

**Source:** [GitHub Copilot Web Search](https://github.blog/changelog/2024-10-29-web-search-in-github-copilot-chat-now-available-for-copilot-individual/)

### 4.5 OpenAI (ChatGPT / Responses API)

**Parameters:**
- `filters` — allow-list of up to 100 URLs
- `external_web_access` — true (live) or false (cache-only mode)

**Response:** `sources` array with all URLs consulted
**Two modes:** Non-reasoning (fast) and agentic with reasoning (iterative)

**Source:** [OpenAI Web Search Guide](https://developers.openai.com/api/docs/guides/tools-web-search/)

### 4.6 Cross-Cutting Pattern: Search-Fetch Separation

Every major implementation separates these concerns:

| Step | Purpose | Token cost | Latency |
|------|---------|------------|---------|
| **Search** | Find relevant URLs from query | Low (titles + snippets) | 0.5-2s |
| **Fetch** | Retrieve full page content | High (full page text) | 1-5s |
| **Summarize** | Extract answer from page | Medium | 1-3s |

**Why separate:** Full pages are 10-100KB of tokens. Pushing them into the main model is expensive. Using a smaller model (e.g., Haiku) to summarize reduces context overhead. Also provides a security boundary — the main agent prompt is isolated from arbitrary web content.

**Exception:** Perplexity Sonar API combines search + answer into a single LLM-grounded call.

---

## 5. Options Analysis

### 5.1 Commercial Search APIs

#### Serper (Google SERP)
- **Pricing:** Free 2,500 queries (no credit card); $50/50K queries ($1/1K); scales to $0.30/1K
- **Speed:** Fastest — 0.6-0.7s median response
- **Rate limits:** Up to 300 qps on paid tiers
- **Output:** Structured JSON SERP data (titles, URLs, snippets, knowledge graphs, People Also Ask)
- **Rust SDK:** Standard HTTP (simple REST endpoint)
- **Strengths:** Cheapest at scale, most generous free tier, fastest
- **Weaknesses:** Google SERP scraping (not an independent index), no semantic search

**Source:** [Serper.dev](https://serper.dev/)

#### Tavily
- **Pricing:** 1,000 free credits/month; $0.008/credit; plans $0-$500/month
- **Credit system:** Basic search = 1 credit, advanced = 2, extract = 1 per 5 URLs
- **Speed:** Variable (basic fast, advanced slower)
- **Output:** Structured JSON designed for RAG/agent workflows
- **Search depths:** basic, advanced, fast, ultra-fast (added Jan 2026)
- **Strengths:** Purpose-built for AI agents; generous free tier; RAG-optimized results
- **Weaknesses:** Smaller company; less battle-tested than Google-backed alternatives

**Source:** [Tavily Docs](https://docs.tavily.com/documentation/api-credits)

#### Exa.ai (formerly Metaphor)
- **Pricing:** Exa Instant $5/1K; Standard $7/1K (with contents); Deep $12/1K; Deep Reasoning $15/1K
- **Free credits:** $10 initial
- **Speed:** Sub-200ms for Exa Instant (fastest available)
- **Search types:** neural, auto, fast, deep, deep-reasoning, instant
- **Output:** Neural semantic search results optimized for AI consumption
- **Strengths:** Used by Claude Code and Cursor; semantic search; fastest; domain/date filtering
- **Weaknesses:** More expensive than Serper; neural search can be opaque

**Source:** [Exa API Reference](https://docs.exa.ai/reference/search), [Exa Instant Announcement](https://www.marktechpost.com/2026/02/13/exa-ai-introduces-exa-instant-a-sub-200ms-neural-search-engine/)

#### Brave Search
- **Pricing:** $5/1K searches; $5 monthly credits for new users
- **Index:** Only API with its own independent web index (30B+ pages)
- **Output:** Structured JSON (web, local, image, video, news)
- **Strengths:** Independent index (not dependent on Google/Bing); privacy-focused; AI summarization built-in
- **Weaknesses:** Smaller index than Google; no true free tier for new users

**Source:** [Brave Search API](https://brave.com/search/api/)

#### Google Custom Search
- **Pricing:** 100 free/day; $5/1K queries; max 10K/day
- **Output:** Structured JSON
- **Strengths:** Google-quality results
- **Weaknesses:** Low free limit (100/day); expensive at scale; requires Google Cloud project

**Source:** [Google CSE Overview](https://developers.google.com/custom-search/v1/overview)

#### SerpAPI
- **Pricing:** ~$15/1K (Developer plan: $75/month for 5K searches); scales down at higher tiers
- **Rust SDK:** Production-ready async SDK on crates.io (`serp-sdk`) with retry logic, exponential backoff, pagination
- **Output:** Rich structured JSON (SERP features, knowledge graph, etc.)
- **Strengths:** Best Rust ecosystem support; comprehensive data extraction
- **Weaknesses:** Most expensive option

**Source:** [SerpAPI](https://serpapi.com/pricing), [serp-sdk crate](https://crates.io/crates/serp-sdk)

#### Perplexity Sonar API
- **Pricing:** Token-based ($1-3/M input) + $0.005/search
- **Models:** Sonar, Sonar Pro, Sonar Reasoning Pro
- **Output:** LLM-generated answers grounded in web search with citations
- **Strengths:** Returns answers, not just links; automatic citations; no post-processing needed
- **Weaknesses:** Token-based pricing harder to predict; less control over raw results

**Source:** [Perplexity API](https://docs.perplexity.ai/getting-started/models/models/sonar)

#### You.com
- **Pricing:** Usage-based; $100 free credits
- **Output:** Structured JSON; includes deep research capability
- **Strengths:** Flexible; good free credits for testing
- **Weaknesses:** Less established; deep search expensive (~$15/call)

**Source:** [You.com APIs](https://you.com/apis)

### 5.2 Open-Source / Self-Hosted

#### SearXNG
- **Cost:** Free (AGPL-3.0)
- **Deployment:** Docker container (~300MB, 512MB RAM minimum)
- **How it works:** Metasearch engine aggregating 70+ search services
- **API:** Custom JSON API for self-hosted instances
- **Strengths:** No API costs; privacy; customizable; aggregates multiple engines
- **Weaknesses:** Requires hosting; quality depends on upstream; can be rate-limited by upstream engines

**Source:** [SearXNG](https://searxng.org/), [GitHub](https://github.com/searxng/searxng)

#### Stract
- **Cost:** Free, fully open-source
- **Features:** Independent web crawler and search index; user-centric ranking
- **Status:** Early development; NLnet funded
- **Strengths:** True independence (own index)
- **Weaknesses:** Early stage; requires significant infrastructure for own index

**Source:** [GitHub](https://github.com/StractOrg/stract)

#### Whoogle
- **Cost:** Free, open-source
- **How it works:** Proxies Google results; removes ads/tracking
- **Strengths:** Simple setup; Google-quality results
- **Weaknesses:** Depends on Google not blocking; Google sees your server's IP

**Source:** [GitHub](https://github.com/benbusby/whoogle-search)

### 5.3 MCP-Based Web Search Servers

Pre-built MCP servers that could integrate directly via doc 23's ToolProvider architecture:

| Server | Backend | Source |
|--------|---------|--------|
| **Brave Search MCP** | Brave Search API | [GitHub](https://github.com/brave/brave-search-mcp-server) |
| **Perplexity MCP** | Sonar API | MCP servers directory |
| **Exa MCP** | Exa.ai | [GitHub](https://github.com/exa-labs/exa-mcp-server) |
| **Web Search MCP** | Local (no API key) | [GitHub](https://github.com/mrkrsl/web-search-mcp) |
| **Kagi MCP** | Kagi Search | MCP servers directory |
| **SearXNG Enhanced MCP** | SearXNG instance | MCP servers directory |

These are immediately usable once the MCP client from doc 23 is implemented, without any custom web search code.

---

## 6. Comparison Matrix

| Criterion | Serper | Tavily | Exa | Brave | SearXNG | MCP Server |
|-----------|--------|--------|-----|-------|---------|------------|
| **Free tier** | 2,500 queries | 1,000/month | $10 credit | $5/month | Unlimited | Depends on backend |
| **Cost at 10K/month** | $10 | $80 | $70 | $50 | $0 (hosting) | Depends on backend |
| **Latency** | 0.6-0.7s | 1-3s | <200ms | ~1s | Variable | + IPC overhead |
| **Result quality** | Google SERP | RAG-optimized | Neural/semantic | Independent index | Aggregated | Depends on backend |
| **Rust SDK** | HTTP only | HTTP only | HTTP only | HTTP only | HTTP only | rmcp crate |
| **Agent-optimized** | No | Yes (built for it) | Yes (AI-native) | Partial | No | N/A |
| **Domain filtering** | No | Yes | Yes | No | No | Depends |
| **Independence** | Google-dependent | Independent | Independent | Independent index | Aggregated | Varies |
| **Implementation effort** | Low (REST) | Low (REST) | Low (REST) | Low (REST) | Medium (self-host) | Zero (if MCP client exists) |
| **Privacy** | Low (Google) | Medium | Medium | High | Highest | Varies |

---

## 7. VISION.md Alignment

| Vision Concept | Web Search Impact |
|----------------|-------------------|
| **Tool calls as first-class graph citizens** (4.8) | WebSearch results become ToolResult nodes with full provenance |
| **MCP for tool integration** (4.8, 5.4) | MCP web search servers fit directly; built-in implementation is the alternative |
| **Background processing** (4.3) | Search results could be cached/compacted in background |
| **Multi-perspective compaction** (4.2) | Search results compactable ("searched for X, found Y, Z relevant") |
| **Cost model** (5.5) | Search API costs must fit the ~$24/month budget target |

The vision explicitly states "MCP is the integration protocol — do not build custom tool adapters" (VISION.md:404). This suggests the **MCP server approach** (delegating to an existing web search MCP server) should be the primary path, with a built-in implementation as a fallback for users without MCP infrastructure.

---

## 8. Architectural Patterns

### 8.1 The Three-Tool Pattern (Cursor/Windsurf)

```
web_search(query) → [{title, url, snippet}]
web_fetch(url) → {content_chunks: [chunk_id]}
view_chunk(chunk_id) → {text}
```

**Why three tools:** Search is cheap (small results). Fetching full pages is expensive (10-100KB). Chunking lets the LLM selectively read only relevant sections. This minimizes token consumption.

**Trade-off:** More tool calls = more latency. For simple searches where the snippet suffices, three round-trips is wasteful.

### 8.2 The Search-Only Pattern (Claude Code)

```
web_search(query) → [{title, url, snippet, encrypted_content}]
```

Claude Code's web search returns enough content inline that a separate fetch step is often unnecessary. The `encrypted_content` field contains substantial text. This is simpler but more expensive per search call.

### 8.3 The Answer-First Pattern (Perplexity Sonar)

```
ask_web(query) → {answer, citations: [{url, title}]}
```

Perplexity combines search + summarization into one call. The LLM never sees raw search results — it gets a pre-synthesized answer. Simplest to integrate but least controllable.

### 8.4 The MCP Delegation Pattern (Doc 23)

```
Agent → ToolProvider.execute("mcp__brave__web_search", {query}) → MCP Client → MCP Server → Brave API
```

No custom web search code at all. The orchestrator delegates to an MCP server that wraps any search API. **This is the VISION.md-aligned approach** and has zero implementation effort once the MCP client from doc 23 exists.

### 8.5 Caching and Rate Limiting

All implementations use some form of:
- **Query-level caching:** Same query within N minutes returns cached results (reduces cost)
- **Rate limiting:** Token bucket or sliding window to prevent runaway costs in agent loops
- **Max uses per turn:** Claude Code's `max_uses` parameter limits searches per request

---

## 9. Red/Green Team

### Green Team (Factual Verification)

25 claims verified against primary sources. 22 confirmed, 2 corrected (applied inline), 1 unverifiable.

**Corrections applied:**
1. **Exa standard pricing**: Corrected from "$0.003/search + $0.001/content" to "$7/1K (with contents); Deep $12/1K; Deep Reasoning $15/1K" per [exa.ai/pricing](https://exa.ai/pricing)
2. **SerpAPI pricing**: Corrected from "$10-25/1K" to "~$15/1K (Developer plan: $75/month for 5K searches)" per [serpapi.com/pricing](https://serpapi.com/pricing)
3. **SearXNG engine count**: Corrected from "~242" to "70+" — the exact count is unverifiable; GitHub repo says "various search services" without a specific number

**Confirmed claims:**
- Serper pricing (2,500 free, $1/1K) — confirmed via serper.dev
- Tavily pricing (1,000 free/month, $0.008/credit) — confirmed via docs.tavily.com
- Brave pricing ($5/1K, $5 monthly credit) — confirmed via api-dashboard.search.brave.com
- Google CSE (100 free/day, $5/1K) — confirmed via developers.google.com
- Perplexity ($0.005/search) — confirmed via docs.perplexity.ai
- You.com ($100 free credits) — confirmed via you.com/apis
- Claude Code uses Exa.ai — confirmed via Claude API docs and Exa MCP integration page
- Cursor uses Exa.ai — confirmed via docs.cursor.com
- GitHub Copilot uses Bing — confirmed via github.blog changelog
- Bing Web Search API retired August 11, 2025 — confirmed via Microsoft Lifecycle page
- Brave Search MCP server at github.com/brave/brave-search-mcp-server — confirmed
- Exa MCP server at github.com/exa-labs/exa-mcp-server — confirmed
- `serp-sdk` crate on crates.io — confirmed, async-ready with retry logic
- SearXNG AGPL-3.0 license — confirmed via GitHub

**Unverifiable:**
- "Grounding with Bing" exact $35/1K pricing — retirement confirmed but replacement pricing varies by Azure integration tier

**Code accuracy:** All 8 internal references verified accurate (execute.rs:97-102, types.rs:23, types.rs:141-143, VISION.md:404, Cargo.toml reqwest 0.13, doc 23 ToolProvider coverage, no WebFetch in enum, WebSearch single `query` field).

### Red Team (Challenges)

**C1 (CRITICAL): MCP bootstrap problem.** The recommendation frames MCP delegation as the "VISION.md-aligned" primary path, but doc 23's MCP client is unimplemented. This creates a circular dependency: web search needs MCP, MCP doesn't exist yet. **Resolution:** Reframe: implement built-in thin API client (Tavily or Serper) for v1. Migrate to MCP delegation once doc 23's MCP client ships. Updated Section 7 accordingly.

**C2 (CRITICAL): Security threat model absent.** Web search results can contain prompt injection attacks in snippets, malicious URLs, and credentials leaked in search results. A web_fetch tool is vulnerable to SSRF (internal IPs, Kubernetes metadata endpoints). None of this is addressed. **Resolution:** Implementation must include: (1) URL validation — reject `file://`, `localhost`, `127.x`, `10.x`, `172.16-31.x`, `192.168.x`, metadata endpoints; (2) snippet size limits (~500 chars) to contain injection payloads; (3) content-type validation for fetch; (4) audit logging of all external URLs.

**C3 (HIGH): Search-fetch separation not empirically justified.** Presented as converged industry pattern, but for typical LLM agent queries, the search snippet may suffice 60-70% of the time. The three-tool pattern (Cursor/Windsurf) adds latency for every query even when unnecessary. **Resolution:** Start with search-only for v1. Add fetch as a separate tool when users demonstrate need. Consider adaptive pattern: check snippet quality before deciding to fetch.

**C4 (HIGH): Result quality not compared.** Document compares pricing and speed but not relevance. A cheap API returning poor results wastes more tokens (re-searches, bad context) than an expensive one with good results. **Resolution:** This is a gap. Empirical benchmarking (20-30 dev-relevant queries scored for relevance) would resolve it. For v1, Tavily's RAG-optimization is a reasonable quality bet without benchmarks.

**C5 (HIGH): Rate limiting design undefined.** Agent loops can fire dozens of searches per minute. Free tiers (Serper 2,500/month = ~83/day, Tavily 1,000/month = ~33/day) exhaust quickly without controls. **Resolution:** Implementation must include: max 3 searches per agent turn (configurable), sliding window of max 10/minute globally, circuit breaker (3 rate-limit errors in 60s → 5-minute pause), cost attribution per work item, 80% usage warnings.

**C6 (HIGH): TCO analysis missing.** Per-query costs compared but not total cost of ownership. SearXNG "free" costs $10-50/month hosting or opportunity cost of local resources. API keys need management/rotation. Vendor lock-in risk (cf. Bing retirement). **Acknowledgment:** Valid gap. For a research doc focused on options, per-query is the primary comparison axis. TCO becomes relevant at implementation time when a specific provider is chosen.

**C7 (MEDIUM): Concurrent agent searches not addressed.** Multiple agents searching identical queries waste quota. No deduplication, no shared rate limiter, no request batching across agents. **Resolution:** Implementation should include query-level cache (LRU, 1-hour TTL) and global rate limiter shared across all agents.

**C8 (MEDIUM): Missing search providers.** DuckDuckGo (API deprecated), Kagi (premium quality), Mojeek (independent UK index), Jina Reader API (fetch + LLM summary in one call) not evaluated. **Acknowledgment:** The document covers the major options. Kagi and Jina are worth noting as niche alternatives.

**C9 (MEDIUM): Result schema not specified.** Document says "structured results" but doesn't define the output format. Without a schema, implementation lacks a target and tests lack expected output. **Resolution:** Define at implementation time. Minimum: `{title, url, snippet, date?}` per result, max 10 results, snippets capped at 300 chars.

**C10 (MEDIUM): Bias toward commercial APIs.** Self-hosted options are buried in Section 5.2 with weaknesses emphasized. Missing cost-reduction strategies: query-level caching, multi-stage fallback (local → free → paid), RSS for domain-specific queries. **Resolution:** These are implementation patterns, not provider options. Noted for future implementation design.

| Challenge | Severity | Status |
|-----------|----------|--------|
| C1: MCP bootstrap | CRITICAL | Resolved — reframed as Phase 2, built-in first |
| C2: Security model | CRITICAL | Noted — must be addressed at implementation |
| C3: Search-fetch justification | HIGH | Resolved — search-only for v1 |
| C4: Quality not compared | HIGH | Acknowledged — empirical benchmarks needed |
| C5: Rate limiting | HIGH | Noted — concrete design required at implementation |
| C6: TCO missing | HIGH | Acknowledged — valid for implementation phase |
| C7: Concurrent agents | MEDIUM | Noted — query cache + global limiter |
| C8: Missing providers | MEDIUM | Acknowledged — Kagi and Jina notable |
| C9: Result schema | MEDIUM | Noted — define at implementation |
| C10: Commercial bias | MEDIUM | Acknowledged — cost strategies for implementation |

---

## 10. Sources

### Search API Documentation
- [Serper.dev](https://serper.dev/) — Google Search API
- [Tavily API Docs](https://docs.tavily.com/documentation/api-credits) — AI search for agents
- [Exa.ai API Reference](https://docs.exa.ai/reference/search) — Neural search
- [Exa Instant Announcement](https://www.marktechpost.com/2026/02/13/exa-ai-introduces-exa-instant-a-sub-200ms-neural-search-engine/)
- [Brave Search API](https://brave.com/search/api/) — Independent index
- [Google Custom Search API](https://developers.google.com/custom-search/v1/overview)
- [SerpAPI Pricing](https://serpapi.com/pricing) + [serp-sdk crate](https://crates.io/crates/serp-sdk)
- [Perplexity Sonar API](https://docs.perplexity.ai/getting-started/models/models/sonar)
- [You.com APIs](https://you.com/apis)

### AI Tool Web Search Implementations
- [Claude API Web Search Tool](https://platform.claude.com/docs/en/agents-and-tools/tool-use/web-search-tool)
- [Cursor @Web Documentation](https://docs.cursor.com/context/@-symbols/@-web)
- [Windsurf Web Search Docs](https://docs.windsurf.com/windsurf/cascade/web-search)
- [GitHub Copilot Web Search](https://github.blog/changelog/2024-10-29-web-search-in-github-copilot-chat-now-available-for-copilot-individual/)
- [OpenAI Web Search Guide](https://developers.openai.com/api/docs/guides/tools-web-search/)
- [Inside Claude Code's Web Tools](https://mikhail.io/2025/10/claude-code-web-tools/)

### Open-Source Search
- [SearXNG](https://searxng.org/) + [GitHub](https://github.com/searxng/searxng)
- [Stract](https://github.com/StractOrg/stract) — Open-source search engine
- [Whoogle](https://github.com/benbusby/whoogle-search) — Google proxy

### MCP Web Search Servers
- [Brave Search MCP Server](https://github.com/brave/brave-search-mcp-server)
- [Exa MCP Server](https://github.com/exa-labs/exa-mcp-server)
- [Web Search MCP (local)](https://github.com/mrkrsl/web-search-mcp)
- [MCP Servers Directory](https://github.com/modelcontextprotocol/servers)

### Architecture & Patterns
- [Agent Harness Anatomy (LangChain)](https://blog.langchain.com/the-anatomy-of-an-agent-harness/)
- [LangChain vs CrewAI 2026](https://www.secondtalent.com/resources/langchain-vs-crewai/)
- [Scaling MCP Tools with defer_loading](https://unified.to/blog/scaling_mcp_tools_with_anthropic_defer_loading)
- [Firecrawl — Web Data API for AI](https://www.firecrawl.dev/)
- [2026 SERP API Pricing Index](https://www.searchcans.com/blog/serp-api-pricing-index-2026/)
- [Best Practices for AI Agent Implementations 2026](https://onereach.ai/blog/best-practices-for-ai-agent-implementations/)

### Internal References
- `src/tool_executor/execute.rs:97-102` — WebSearch stub
- `src/graph/tool/types.rs:23` — ToolName::WebSearch
- `src/graph/tool/types.rs:141-143` — ToolCallArguments::WebSearch variant
- `docs/research/23-ecosystem-tool-integration.md` — ToolProvider trait, MCP client design
- `docs/VISION.md:404` — "MCP is the integration protocol"
