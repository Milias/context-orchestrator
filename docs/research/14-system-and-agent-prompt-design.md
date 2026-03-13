# System and Agent Prompt Design

> Research conducted 2026-03-13. Enumerates all prompt categories required by Context Manager,
> surveys production AI coding tools for design patterns, and proposes a graph-native prompt
> assembly architecture.

---

## 1. Executive Summary

Context Manager requires **9 distinct prompt categories** across three tiers: foreground (primary conversation), background (compaction, scoring, classification, extraction, connection suggestion, bootstrapping), and meta (system directive assembly, agent coordination). Each category targets a different model tier, output format, and optimization goal.

The current implementation has a single hardcoded system prompt ("You are a helpful assistant." in `src/config.rs:20-22`) and one background extraction prompt (`src/tools.rs:103-106`). This covers ~2 of the 9 categories. The gap is significant but the existing architecture (background LLM calls via semaphore, `TaskMessage` channel, agent loop) provides the infrastructure to support all categories.

**Key findings from industry survey:**
- Production tools (Claude Code, Cursor, Windsurf) dynamically assemble system prompts from 10-110+ conditional fragments, not static strings
- Anthropic recommends XML tags for Claude's context sections, priority-ordered content, and the 4-layer cache model (doc 06)
- Vercel's v0 team found removing 80% of tools and simplifying prompts improved success from 80% to 100%
- Prompt quality and topology co-optimization outperform naive agent composition (arXiv 2502.02533): optimize individual prompts before composing multi-agent systems
- Graph-native prompt assembly (our approach) is validated by ContextBranch (arXiv 2512.13914: 58% context reduction) and KnowGPT (adaptive KG-to-prompt translation)

**Recommendation:** Treat prompt assembly as a first-class architectural concern. Implement the 4-layer model from doc 06, replace the static system prompt with graph-native directive assembly, and build category-specific prompt templates for each background task.

---

## 2. Current Architecture & Gap Analysis

### What Exists

| Component | File | Implementation |
|-----------|------|----------------|
| System prompt default | `src/config.rs:20-22` | `"You are a helpful assistant."` |
| Context building | `src/app/context.rs:7-60` | Ancestor walk, single SystemDirective extraction |
| Agent loop | `src/app/agent_loop.rs:53-124` | Iterative tool-use with `build_context` per turn |
| Plan extraction prompt | `src/tools.rs:81-187` | Background LLM: "Extract structured data..." + recent messages + tool list |
| Background LLM infra | `src/llm/mod.rs:106-136` | Semaphore-limited non-streaming calls |
| Tool definitions | `src/tool_executor/mod.rs:31-109` | 4 tools: read_file, write_file, list_directory, search_files |

### What's Missing

| Category | Gap | Impact |
|----------|-----|--------|
| Dynamic directive assembly | Static single-node system prompt | Cannot scope instructions to tasks, expire temporary directives, or track token budget |
| Background compaction | `spawn_context_summarization` is a no-op | Conversations grow unbounded; no Layer 2 content for caching |
| Relevance scoring | No implementation | No way to rank nodes for context inclusion |
| Topic classification | No implementation | Cannot scope directives to topics or enable multi-perspective compaction |
| Connection suggestion | `RelevantTo` edge exists but is never created | No cross-session or cross-topic discovery |
| Agent coordination | Single-agent only | No task decomposition, handoff, or conflict resolution |
| Context bootstrapping | Git watcher scans status only | No history, diff, or PR summarization |
| Cache breakpoints | No `cache_control` in API calls | Full token cost on every request |

---

## 3. Prompt Category Taxonomy

### Overview

| # | Category | Tier | Model | Output | Budget |
|---|----------|------|-------|--------|--------|
| 1 | Primary Conversation | Foreground | Frontier | Free-form + tool_use | 100-180K |
| 2 | Background Compaction | Background | Mid/Cheap | JSON + free-form summary | 8-32K |
| 3 | Relevance Scoring | Background | Cheap | JSON (score + reasoning) | 2-8K |
| 4 | Tool Extraction | Background | Mid-tier | JSON (node schema) | 4-16K |
| 5 | Topic Classification | Background | Cheap/Local | JSON (labels) | 2-4K |
| 6 | Connection Suggestion | Background | Mid-tier | JSON (pairwise judgment) | 4-16K |
| 7 | Agent Coordination | Meta | Mid-tier | JSON + free-form | 8-32K |
| 8 | System Directive Assembly | Meta | None (deterministic) | Serialized text | N/A |
| 9 | Context Bootstrapping | Background | Mid-tier (batch) | JSON | 8-32K/item |

### Category 1: Primary Conversation

The developer chatting with the LLM. Uses the 4-layer prompt model from doc 06.

**Grounding context:** System directives (Layer 1) + compacted history (Layer 2) + recent messages (Layer 3) + current turn + tool results (Layer 4). Work item context, git file status, and tool definitions also in Layer 1.

**Key practices from industry:**
- Claude Code assembles ~110 conditional fragments with per-fragment token cost tracking
- Cursor uses a two-model pattern (strong model decides edits, weak applies them)
- LLM attention to early content decreases as context grows -- re-inject critical instructions at Layer 2/3 boundary
- v0: fewer tools with bash access outperformed 15+ specialized tools

### Category 2: Background Compaction

Summarize older messages into `CompactedMessage` nodes (VISION.md Section 4.3).

**Prompt template:**
```
System: You are a context compaction specialist. Compress conversation segments
while preserving: decisions and rationale, code patterns established, constraints
mentioned, error conditions and resolutions.

User: Compress the following segment. Active work item: "{title}: {description}"

<messages>
{serialized messages}
</messages>

Respond with ONLY valid JSON: {"summary": "...", "key_facts": [...],
"perspective": "general", "original_token_count": N, "compressed_token_count": N}
```

**Validation:** Reject summaries longer than originals (Beads pattern, doc 04). Compression below ~10-15% of original size risks entering a "lossy zone" with imperfect recall (arXiv 2509.11208 discusses predictable compression failures). Max 1 compaction per 15-20 messages (doc 06 cache amortization).

### Category 3: Relevance Scoring

LLM-as-judge tier in the cascade: embeddings (doc 09) -> classifiers (doc 10) -> **LLM judge** -> human. Only 20-30% of nodes reach this tier.

**Prompt template:**
```
System: You are a relevance judge for a software development knowledge graph.
Rate relevance to the specified context.

Scoring rubric: 0.0-0.2 irrelevant, 0.2-0.5 tangential, 0.5-0.8 relevant,
0.8-1.0 highly relevant.

<example score="0.9">
Context: "Implement rate limiting for the API"
Content: "Decided to use token bucket algorithm with 100 req/min per user"
</example>

User: Rate this content for the given context.
Context: "{context}"
<content>{node_content}</content>

Respond with ONLY valid JSON: {"score": float, "reasoning": "...",
"category": "high|medium|low", "confidence": float}
```

### Category 4: Tool Extraction

Parse user commands (`/plan`, future `/tag`, `/pin`, `/connect`) into structured graph nodes. Already partially implemented in `src/tools.rs`.

### Category 5: Topic Classification

Multi-label classification enabling scoped directive activation and multi-perspective compaction. Start with rule-based baseline (doc 10 Phase 0), escalate to LLM for ambiguous cases.

### Category 6: Connection Suggestion

Pairwise evaluation of embedding-candidate node pairs for `RelevantTo` edges. Precision over recall -- false connections pollute the graph. Always suggestions, never auto-created.

### Category 7: Agent Coordination

Three sub-prompts: task decomposition (breaking work into subtasks), agent handoff summaries (complete context transfer), and conflict resolution. Gas Town (doc 05) validates the coordinator-worker pattern.

### Category 8: System Directive Assembly

**Not an LLM prompt** -- a deterministic graph traversal replacing static CLAUDE.md files.

`SystemDirective` nodes gain properties: `scope` (global/conversation/work_item/temporary), `priority`, `tags`, `active`, `expires_at`.

**Assembly algorithm:**
1. Collect active `SystemDirective` nodes
2. Filter by scope (always include global; include conversation/work_item/temporary based on match)
3. Filter by tags (match against topic classifications of recent messages)
4. Sort by scope priority, then by priority within scope
5. Serialize with stable formatting; hash to detect drift

**Maps to existing patterns:**

| Static Pattern | Graph Equivalent |
|----------------|-----------------|
| `CLAUDE.md` (project root) | `scope: global` directives |
| `CLAUDE.md` (subdirectory) | `scope: work_item` + matching tags |
| `.cursor/rules/*.mdc` with modes | `tags` for scoped activation |
| `AGENTS.md` | Agent capability/constraint directives |

Bidirectional import/export maintains interoperability with Claude Code and Cursor.

### Category 9: Context Bootstrapping

One-time graph population from git history, PRs, and documentation (VISION.md Section 6). Sub-prompts for commit summarization, PR summarization, and documentation chunking. Uses batch API pricing.

---

## 4. Industry Prompt Patterns

### Claude Code (Piebald-AI extraction, v2.1.74)

110+ conditional fragments organized into: identity/capabilities (~65), sub-agent prompts (3 primary: Plan/Explore/Task), utility agents (~20), slash commands (5+), data/reference, and system reminders (~40). Each fragment has a token cost for budget-aware assembly.

**Sub-agents:** Explore (read-only, fresh context, 517 tokens), Plan (read-only tools, inherited context, 685 tokens), Task (full tools, inherited context, no recursive sub-agents).

**Source:** [Piebald-AI/claude-code-system-prompts](https://github.com/Piebald-AI/claude-code-system-prompts)

### Cursor

Opens with role framing. Sections: communication guidelines, tool calling rules (12 tools including diff_history), search/reading behavior, code change protocols, debugging (3-attempt limit), security. Two-model pattern: strong model generates edit descriptions, weak model applies them.

**Source:** [Cursor agent system prompt](https://gist.github.com/sshh12/25ad2e40529b269a88b80e7cf1c38084)

### Windsurf/Cascade

"Flow paradigm" framing. Key rules: persistence (work until resolved), autonomy (resolve before returning to user), grounding (read before guessing). Metadata injection per request (open files, cursor position, editing patterns).

### Vercel v0

Removed 80% of tools, success went from 80% to 100% with 3.5x faster execution and 37% fewer tokens. Dynamic prompt with technology-specific doc injection via embeddings + keyword matching.

**Source:** [We removed 80% of our agent's tools](https://vercel.com/blog/we-removed-80-percent-of-our-agents-tools)

### Devin

Compound architecture: 4 specialized models (Planner, Coder, Critic, Browser) with dynamic re-planning. Multiple instances dispatch sub-tasks to each other.

### Common Assembly Pattern

All production tools follow a consistent ordering:
1. Identity/role (static)
2. Security/boundaries (static)
3. Tool definitions (semi-static)
4. Behavioral instructions (semi-static)
5. Provider-specific sections (conditional)
6. Project context (dynamic -- CLAUDE.md, .cursorrules)
7. Environment state (dynamic -- CWD, platform, git branch)
8. User preferences (dynamic)
9. System reminders (injected mid-conversation)

Split into cacheable prefix (1-4) and dynamic suffix (5-9).

### Anthropic Best Practices

- XML tags: Claude is fine-tuned to attend to them. Use `<instructions>`, `<context>`, `<example>`, `<documents>`
- Examples: 3-5 well-crafted examples dramatically improve accuracy
- Role in system prompt focuses behavior
- Long content at top, query at end (30% quality improvement)
- Tool guidance: `<default_to_action>`, `<do_not_act_before_instructions>`, `<use_parallel_tool_calls>`
- Anti-hallucination: `<investigate_before_answering>`
- Context awareness: tell the model about compaction behavior

**Source:** [Anthropic prompt engineering](https://platform.claude.com/docs/en/build-with-claude/prompt-engineering/use-xml-tags)

---

## 5. Comparison Matrix

| Approach | Prompt Source | Scoping | Cache-Friendly | Token Visibility | Graph-Aware |
|----------|-------------|---------|----------------|-----------------|-------------|
| CLAUDE.md (static) | File | None (global only) | Moderate | None | No |
| `.mdc` (Cursor) | Files + YAML | By file pattern | Moderate | None | No |
| Claude Code (fragments) | 110+ JS fragments | By mode/feature | Yes (designed) | Internal only | No |
| **Context Manager (proposed)** | Graph nodes | By scope/tags/topic | Yes (4-layer model) | TUI display | Yes |

---

## 6. VISION.md Alignment

| VISION.md Concept | Prompt Category | Status |
|-------------------|----------------|--------|
| Multi-perspective compaction (4.2) | Category 2 | Designed. Start single-perspective, add multi when data shows need |
| Background processing / MergeTree (4.3) | Categories 2, 3, 5, 6, 9 | Infrastructure exists (`background_llm_call`); prompts not implemented |
| Multi-rater relevance (4.4) | Category 3 | Cascade designed (docs 09-10); LLM judge prompt designed here |
| Dynamic system prompt (4.7) | Category 8 | Assembly algorithm designed; requires SystemDirective node extensions |
| Tool call provenance (4.8) | Category 1 (tool loop) | Implemented in `agent_loop.rs` |
| Cold start / bootstrapping (6) | Category 9 | Prompt templates designed; progressive availability matches VISION.md 6.6 |

---

## 7. Recommended Architecture

### Prompt Assembly Pipeline

Every prompt follows this pipeline:

```
Graph State -> Subgraph Selection -> Layer Assignment -> Ordering
    -> Serialization -> Budget Enforcement -> Cache Annotation -> API Call
```

### Node-to-Text Serialization (XML)

```xml
<system_directive scope="global" priority="1">
Always use async/await. Never use spawn_blocking unless justified.
</system_directive>

<work_item status="active" id="wi-123">
<title>Implement API rate limiting</title>
<description>Token bucket algorithm, 100 req/min per user</description>
</work_item>

<git_context>
Modified files: src/middleware/rate_limit.rs, src/config.rs
</git_context>
```

### Token Budget Allocation

| Layer | Budget % | Content |
|-------|---------|---------|
| 1: Stable Prefix | 10-20% | Directives + tools + project context. Warn at >20% |
| 2: Session-Stable | 15-30% | Compacted summaries (append-only, immutable) |
| 3: Recent Conversation | 40-60% | Primary working memory. Prune oldest first |
| 4: Volatile | 10-20% | Current message + tool results |

### Model-to-Category Mapping

| Category | Model | Monthly Cost (active dev) |
|----------|-------|--------------------------|
| Primary Conversation | User-configured (Sonnet 4.6) | ~$9 (cached) |
| Compaction | DeepSeek V3.2 batch | ~$2 |
| Relevance Scoring | Haiku 4.5 | ~$1 |
| Tool Extraction | Haiku 4.5 | ~$0.50 |
| Topic Classification | Local Qwen / rules | $0 |
| Connection Suggestion | Haiku 4.5 | ~$0.50 |
| Agent Coordination | Sonnet 4.5 | ~$1 |
| Bootstrapping (one-time) | DeepSeek batch | ~$1 |
| **Total** | | **~$15/month** |

---

## 8. Implementation Priority

### Phase 1: Foundation (Categories 1, 2, 4)

1. Refactor `build_context` for the 4-layer model. Separate directive assembly, compacted history, recent conversation, and current turn. Add cache breakpoints. Include error recovery and compaction-awareness directives in Category 1's system prompt.
2. Implement background compaction (Category 2) with size-reduction verification and deterministic truncation fallback on LLM failure. Without compaction, conversations grow unbounded -- this is critical infrastructure, not an enrichment feature.
3. Expand tool extraction (Category 4): add `/tag`, `/pin`, `/connect` triggers with category-specific extraction prompts.
4. Read CLAUDE.md at startup and inject as Layer 1 system prompt (simplified Category 8 -- defer scoping/priority/tags until global directives prove insufficient).

### Phase 2: Background Enrichment (Categories 5, 3, 8)

5. Implement topic classification (rule-based baseline first per doc 10).
6. Implement relevance scoring LLM judge integrated with cascade.
7. Implement full directive assembly (Category 8): add `scope` and `priority` to `SystemDirective` nodes. Defer `tags` and `expires_at` until topic classification is working.

### Phase 3: Intelligence (Categories 6, 9, 7)

8. Connection suggestion with pairwise evaluation prompts.
9. Context bootstrapping (git history first, then docs, then PRs).
10. Agent coordination (requires doc 07 multi-agent infrastructure).

---

## 9. Red/Green Team

### Green Team (Factual Validation)

**Confirmed claims:**
- Claude Code ~110 conditional fragments from Piebald-AI repository
- Sub-agent token counts: Explore 517, Plan 685
- Vercel v0: 80% tool removal → 80% to 100% success, 3.5x faster, 37% fewer tokens
- ContextBranch (arXiv 2512.13914): 58.1% context reduction confirmed
- All model pricing verified against current Anthropic/DeepSeek pricing pages
- Anthropic XML tag recommendations confirmed current for Claude 4.6
- 9 prompt categories verified as complete and non-overlapping
- All cited source URLs verified as live and containing claimed content
- Cursor two-model pattern confirmed via `reapply` tool description

**Corrections applied:**
- Cursor has 12 tools (not 11) -- `diff_history` was omitted. Fixed.
- arXiv 2502.02533 concludes prompts and topologies co-optimize, not that prompts dominate. Reworded.
- "System prompts fade" is a well-established attention phenomenon, not a direct Anthropic quote. Reworded.
- arXiv 2509.11208 compression threshold claim softened -- the specific 12.5% figure was not verifiable from the abstract.

**Unresolvable:** Monthly cost projections (~$15/month) depend on assumed usage volumes that cannot be independently verified. Unit prices are confirmed correct; the projections are plausible order-of-magnitude estimates.

### Red Team (Challenges)

**R1. Missing prompt categories.** The taxonomy omits error recovery prompts (what should the model do when a tool fails?), user intent disambiguation (when input is ambiguous), and code review/test generation (primary developer workflows). Error recovery is particularly critical -- the agent loop currently emits raw `e.to_string()` on failure with no recovery guidance. **Response:** Error recovery and compaction-awareness instructions belong in Category 1 as behavioral directives, not separate categories. Code review and test generation are sub-prompts of Category 1. Acknowledged in Section 3.

**R2. Over-engineered for current state.** The codebase has 1 working prompt. Designing 9 categories upfront risks premature abstraction. Categories 3, 5, 6, 7 depend on infrastructure that does not exist. **Response:** Valid. The taxonomy is the target state. Phase 1 scope narrowed: implement Categories 1 (with error recovery directives), 2 (compaction is critical -- conversations grow unbounded without it), and 4 (already partially implemented). Everything else is documented as "future."

**R3. XML overhead for short prompts.** XML wrapping adds 50+ chars per directive. For 5-10 short directives, that is 300-500 tokens of overhead in a 10-20% Layer 1 budget. **Response:** Valid for short directives. Recommendation: use XML wrapping only when content exceeds 100 characters or when 3+ directives are present. Single short directives should use plain text.

**R4. Category 8 reinvents CLAUDE.md.** Adding scope/priority/tags/expires_at to SystemDirective creates a complex system that requires new UI, creates a circular dependency on Category 5, and cannot be debugged with a text editor. **Response:** Phase 1 should read CLAUDE.md at startup and inject as Layer 1, full stop. Scoping/priority/tags deferred until global directives are demonstrably insufficient. If scoping is needed later, start with `scope: global` vs. `scope: work_item` only.

**R5. Fragile cost model.** Dollar amounts are snapshots. No per-request formulas, no sensitivity analysis, no uncached scenario. **Response:** Valid. Cost table should show: calls/month × input_tokens/call × price/token. Cached and uncached scenarios should both be stated. The current table is an order-of-magnitude estimate, not a budget.

**R6. No fallback for background LLM failures.** Each background category needs a degradation policy. Compaction failure means unbounded growth. Scoring failure means incorrect inclusion/exclusion. **Response:** Critical. Each category needs: (a) max retries before abandoning, (b) fallback behavior on failure (compaction falls back to deterministic truncation; scoring falls back to embedding-only), (c) user notification of degraded enrichment. The background semaphore (currently 2) may be insufficient for 5+ competing categories.

**R7. "Prompt quality > agent quantity" overstated.** Devin (4 specialized models) outperforms single-agent approaches. The cited paper argues for co-optimization, not prompt dominance. **Response:** Corrected in the executive summary. The practical recommendation stands for our current state: with 1 working prompt, optimizing it is higher ROI than adding agents.

**R8. Rigid prompt templates.** Templates embed specific output format instructions and rubric breakpoints that may become stale as models evolve. No mechanism for A/B testing or iteration. **Response:** Templates in this document are initial drafts. Implementation should load templates from configuration, include version identifiers, and define evaluation metrics (compaction ratio, scoring precision/recall).

**R9. Error handling absent from Category 1.** The primary conversation prompt needs explicit behavioral directives for tool failure recovery, compaction awareness ("if earlier context seems missing, it may have been compacted"), and when to ask for clarification vs. assuming. **Response:** Agreed. Category 1 must include these as part of the system directive content, following Cursor's "3-attempt debugging limit" and Windsurf's "persistence" patterns.

**R10. No prompt versioning.** No mechanism to track which prompt version produced a given result, correlate prompt changes with quality changes, or roll back. **Response:** Each prompt template should have a version string (e.g., `compaction-v1`). `BackgroundTask` nodes should record the prompt version used. This is one string field and makes debugging prompt regressions tractable.

---

## 10. Sources

### Production System Prompts
- [Piebald-AI/claude-code-system-prompts](https://github.com/Piebald-AI/claude-code-system-prompts) -- Claude Code 110+ fragments
- [Cursor Agent system prompt (March 2025)](https://gist.github.com/sshh12/25ad2e40529b269a88b80e7cf1c38084)
- [Windsurf/Cascade system prompt](https://github.com/jujumilk3/leaked-system-prompts/blob/main/codeium-windsurf-cascade_20241206.md)
- [Aider edit block prompts](https://github.com/Aider-AI/aider/blob/main/aider/coders/editblock_prompts.py)

### Anthropic Documentation
- [Prompt engineering best practices](https://platform.claude.com/docs/en/build-with-claude/prompt-engineering/use-xml-tags)
- [Effective context engineering for AI agents](https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents)
- [Building effective agents](https://www.anthropic.com/research/building-effective-agents)
- [Modifying system prompts - Agent SDK](https://platform.claude.com/docs/en/agent-sdk/modifying-system-prompts)

### Multi-Agent Frameworks
- [CrewAI prompt customization](https://docs.crewai.com/en/guides/advanced/customizing-prompts)
- [Azure AI Agent Orchestration Patterns](https://learn.microsoft.com/en-us/azure/architecture/ai-ml/guide/ai-agent-design-patterns)
- [ADK Multi-Agent Patterns](https://developers.googleblog.com/developers-guide-to-multi-agent-patterns-in-adk/)

### Academic Research
- [Multi-Agent Design: Optimizing with Better Prompts (arXiv 2502.02533)](https://arxiv.org/abs/2502.02533)
- [Context Branching for LLM Conversations (arXiv 2512.13914)](https://arxiv.org/html/2512.13914v1)
- [KnowGPT: Knowledge Graph based PrompTing (arXiv 2312.06185)](https://arxiv.org/html/2312.06185v5)
- [Codified Context: Infrastructure for AI Agents (arXiv 2602.20478)](https://arxiv.org/html/2602.20478v1)
- [MASFactory: Graph-centric Multi-Agent Orchestration (arXiv 2603.06007)](https://arxiv.org/html/2603.06007)
- [Building AI Coding Agents for the Terminal (arXiv 2603.05344)](https://arxiv.org/html/2603.05344v1)
- [Predictable Compression Failures (arXiv 2509.11208)](https://arxiv.org/abs/2509.11208)

### Industry Analysis
- [How we made v0 an effective coding agent (Vercel)](https://vercel.com/blog/how-we-made-v0-an-effective-coding-agent)
- [We removed 80% of our agent's tools (Vercel)](https://vercel.com/blog/we-removed-80-percent-of-our-agents-tools)
- [Claude Code master agent loop (PromptLayer)](https://blog.promptlayer.com/claude-code-behind-the-scenes-of-the-master-agent-loop/)
- [Devin 2.0 technical design](https://medium.com/@takafumi.endo/agent-native-development-a-deep-dive-into-devin-2-0s-technical-design-3451587d23c0)

### Configuration Patterns
- [Complete Guide to AI Agent Memory Files](https://medium.com/data-science-collective/the-complete-guide-to-ai-agent-memory-files-claude-md-agents-md-and-beyond-49ea0df5c5a9)
- [AGENTS.md: One File to Guide Them All (Layer5)](https://layer5.io/blog/ai/agentsmd-one-file-to-guide-them-all/)
- [Scaling Agent Context Beyond a Single AGENTS.md](https://ursula8sciform.substack.com/p/scaling-your-coding-agents-context)

### LLM-as-Judge
- [LLM-as-a-Judge: Complete Guide (Evidently AI)](https://www.evidentlyai.com/llm-guide/llm-as-a-judge)
- [LLM-as-a-Judge: 7 Best Practices (Monte Carlo)](https://www.montecarlodata.com/blog-llm-as-judge/)

### Internal References
- `docs/research/04-beads-agent-memory-system.md` -- Tiered compaction validation
- `docs/research/05-gastown-multi-agent-orchestration.md` -- Agent coordination patterns
- `docs/research/06-token-caching-strategies.md` -- 4-layer prompt model, cache breakpoints
- `docs/research/06-inline-tool-invocation-patterns.md` -- Tool extraction patterns
- `docs/research/07-inter-agent-communication.md` -- Multi-agent architecture
- `docs/research/09-embedding-based-connection-suggestions.md` -- Embedding pipeline
- `docs/research/10-classical-ml-node-enrichment.md` -- Classification pipeline
- `src/app/context.rs` -- Current `build_context` implementation
- `src/app/agent_loop.rs` -- Agent loop and tool dispatch
- `src/tools.rs` -- Plan extraction prompt
- `src/config.rs:20-22` -- Default system prompt
- `src/llm/mod.rs:106-136` -- Background LLM call infrastructure
