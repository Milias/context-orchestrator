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
- **Pricing:** Exa Instant $5/1K; Standard $0.003/search + $0.001/content
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
- **Pricing:** $10-25/1K searches
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
- **How it works:** Metasearch engine aggregating ~242 search services
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
| **Cost at 10K/month** | $10 | $80 | $50 | $50 | $0 (hosting) | Depends on backend |
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

### Green Team (Validation)

1. **Pricing verified** (2026-03-14): All pricing checked against primary sources. Serper's 2,500 free confirmed via serper.dev. Tavily's 1,000 free/month confirmed via docs.tavily.com. Exa Instant $5/1K confirmed via exa.ai/pricing.

2. **Exa as Claude Code/Cursor backend** confirmed via:
   - Claude API docs reference `web_search_tool` powered by external search
   - Cursor docs explicitly mention Exa.ai for `@web`
   - Multiple third-party analyses confirm Exa usage

3. **SerpAPI Rust SDK** exists on crates.io (`serp-sdk`) — confirmed published, async-ready

4. **MCP web search servers** exist and are functional — Brave Search MCP server is official (maintained by Brave), Exa MCP server maintained by Exa Labs

5. **SearXNG** Docker deployment confirmed straightforward — official docker-compose provided, ~300MB image

6. **Bing Web Search API retirement** confirmed — fully retired August 11, 2025. Replaced by "Grounding with Bing" at $35/1K (significantly more expensive)

### Red Team (Challenges)

**R1: MCP-first approach has a bootstrap problem.** Doc 23's MCP client doesn't exist yet. If web search is needed before MCP infrastructure, a built-in implementation is required. **Counter:** True. The built-in implementation should be simple enough (HTTP call + JSON parse) that it doesn't violate the "no custom adapters" principle — it's a stopgap.

**R2: Exa being used by Claude Code doesn't mean it's best for us.** Claude Code gets preferential pricing/access from Exa. We'd pay retail. Serper is 5-10x cheaper. **Acknowledgment:** Valid. Exa's neural search quality may not justify the cost for a tool where the LLM post-processes results anyway.

**R3: Self-hosted SearXNG sounds free but isn't.** Hosting costs, maintenance burden, upstream rate limiting, and result quality variation make it more expensive than $5-10/month for an API. **Counter:** True for cloud hosting, but for a developer running it locally alongside the orchestrator, it's genuinely zero-cost with Docker.

**R4: The search-fetch separation adds complexity.** For many queries, the snippet from search results is sufficient. Adding a web_fetch tool doubles the API surface. **Counter:** Valid for v1. Start with search-only; add fetch when users need it.

**R5: No evaluation of result quality.** Pricing and speed are compared but not result relevance. A cheap API returning bad results is worse than an expensive API returning good ones. **Acknowledgment:** True. Quality benchmarks would require empirical testing with representative queries. This is a gap in this research.

**R6: Rate limiting design is unspecified.** Agent loops can fire dozens of searches per minute. Without rate limiting, costs can spiral. **Counter:** True. Whatever implementation is chosen must include configurable rate limits (e.g., max N searches per agent turn, max M per minute).

**R7: Missing consideration of content licensing.** Search APIs return snippets under fair use, but full content fetching may violate terms of service for some sites. **Acknowledgment:** Important legal consideration. web_fetch should respect robots.txt at minimum.

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
