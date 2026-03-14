# System Prompt Integration

> Research conducted 2026-03-14. Investigates how system prompts integrate with the current
> application, surveys industry patterns for prompt assembly, and analyses the gap between
> the codebase's static pipeline and the graph-native dynamic assembly described in VISION.md.

---

## 1. Executive Summary

The current system prompt pipeline is a 4-step linear chain: a static default string
(`src/config.rs:20-22`) is embedded as a `SystemDirective` graph node at conversation
creation (`src/graph/mod.rs:101-124`), extracted during ancestor walk
(`src/app/context/policies/conversational.rs:14-61`), dynamically extended with plan context
(lines 53-58), and sent to the Anthropic API as a plain `Option<String>`
(`src/llm/anthropic.rs`). The system prompt is never truncated by the sanitizer — only
conversation messages are trimmed when the token budget is exceeded.

Doc 14 designed 9 prompt categories and a dynamic assembly algorithm with scoped, prioritised
`SystemDirective` nodes. The codebase implements one of these categories. This document
investigates what the industry has learned about bridging that gap.

**Key findings:**

- **Claude Code** assembles 110+ conditional fragments at runtime. Sub-agents (Plan, Explore,
  Task) get purpose-specific prompts. System reminders are injected mid-conversation to
  counteract instruction fading. Source: [Piebald-AI repository](https://github.com/Piebald-AI/claude-code-system-prompts).
- **Developer experience:** Over-specification is the most common failure mode with CLAUDE.md.
  Reducing context from 180K to 60K tokens decreased hallucinations by 35% and improved task
  completion from 68% to 89%. Best practice guidance suggests 5-15% of context window for
  system prompts.
- **Security:** Config files (CLAUDE.md, .mdc, memory files) are indirect prompt injection
  vectors. 36.82% of agent skills contain security flaws (Snyk ToxicSkills study).
- **Graph-native assembly** is validated by KnowGPT (92.6% accuracy on OpenBookQA via
  RL-driven subgraph selection) and ContextBranch (58% context reduction through branching).

**Recommendation:** Three-phase migration — static file injection now, scoped directives when
multi-agent matures, event-driven assembly long-term.

---

## 2. Current Architecture & Gap Analysis

### 2.1 The Pipeline

| Step | File | What Happens |
|------|------|--------------|
| 1. Default string | `src/config.rs:20-22` | `"You are a helpful assistant."` loaded via figment env/config |
| 2. Graph root | `src/graph/mod.rs:101-124` | `ConversationGraph::new()` creates `Node::SystemDirective` as first node, sets it as branch root |
| 3. Startup init | `src/main.rs` | `ConversationGraph::new(&config.system_prompt)` at launch |
| 4. Extraction | `src/app/context/policies/conversational.rs:22-26` | `build_messages()` walks branch history, matches `Node::SystemDirective`, clones content |
| 5. Plan injection | `src/app/context/policies/conversational.rs:53-58` | Appends `build_plan_section()` output to the system prompt string |
| 6. Finalization | `src/app/context/sanitize.rs` | Token count check; truncates *messages*, never the system prompt |
| 7. API call | `src/llm/anthropic.rs` | `MessagesRequest.system: Option<String>` — plain string, no structured blocks |

Key observations:

- The system prompt is `Option<String>`, not structured content blocks. This means
  `cache_control` breakpoints cannot be placed on it (prerequisite from doc 06).
- Plan context injection (step 5) is the **only dynamic element**. Everything else is
  static across the conversation lifetime.
- Agent loops (`src/app/agent/loop.rs`) rebuild the full context on every iteration.
  No caching of the system prompt extraction result between turns.
- The SSE parser has no `message_start` handler, so cache hit/miss metrics from the
  Anthropic API are silently discarded.

### 2.2 The SystemDirective Node

Current definition (`src/graph/node.rs`):

```
SystemDirective { id: Uuid, content: String, created_at: DateTime<Utc> }
```

Doc 14 section 3.8 proposed extending this with: `scope` (global/conversation/work_item/
temporary), `priority`, `tags`, `active`, `expires_at`. None of these exist today.

### 2.3 Gap Summary

| Capability | Current State | Industry Standard | Severity |
|-----------|--------------|-------------------|----------|
| Dynamic assembly | Static string + plan append | 110+ conditional fragments (Claude Code) | High |
| Scoped activation | None (global only) | Glob-pattern rules (.mdc), nested CLAUDE.md | Medium |
| Cache integration | No `cache_control` on system field | Explicit breakpoints, stable prefix (doc 06) | High |
| Mid-conversation re-injection | None | System reminders (Claude Code) | Medium |
| Token budget awareness | Never truncated, no warnings | 5-15% allocation, warn at threshold | Low |
| Config file loading | No CLAUDE.md/AGENTS.md reading | Standard in Claude Code, Cursor, Windsurf | High |
| Security boundaries | None | Permission system, blocklist, sandboxing | High |

---

## 3. Requirements

Derived from VISION.md §4.7, doc 14 Category 8, doc 06 (caching), and user feedback
(event-driven architecture, graph as source of truth).

| ID | Requirement | Source |
|----|-------------|--------|
| R1 | System prompt assembled from graph nodes, not static strings | VISION.md §4.7 |
| R2 | Config files (CLAUDE.md, AGENTS.md) loadable as SystemDirective nodes | Industry standard |
| R3 | Directives must support scope (global/conversation/work_item) and priority | Doc 14 §3.8 |
| R4 | Assembly must produce deterministic byte output for cache stability | Doc 06 |
| R5 | System prompt budget must not exceed configurable % of context window | Industry consensus |
| R6 | Mid-conversation re-injection of critical directives must be possible | Attention research |
| R7 | Assembly must be a pure graph traversal with no I/O | Doc 14 Category 8 |
| R8 | Config file content must be treated as untrusted input | Security research |
| R9 | Each assembled prompt should carry a content hash for version tracking | Doc 14 Red Team R10 |
| R10 | Assembly must support per-agent customisation for multi-agent contexts | Design 04 |

---

## 4. Options Analysis

### Option A: Static File Injection

Read CLAUDE.md / AGENTS.md at startup, concatenate with config default, store as a single
`SystemDirective` node. No structural changes.

| Dimension | Assessment |
|-----------|-----------|
| Strengths | Trivially implementable. Compatible with existing Claude Code / Cursor workflows. Delivers the highest-severity gap (config file loading) immediately. |
| Weaknesses | No scoping, no dynamic assembly, no mid-conversation updates. File changes require restart. Does not advance toward VISION.md target. |
| Satisfies | R2, R5 (partial) |

### Option B: Multi-Node Directive Assembly

Extend `SystemDirective` with `scope`, `priority`, `tags`. Assembly algorithm collects active
directives, filters by scope/tags, sorts by priority, serialises with stable formatting.
This is the full design from doc 14 section 3.8.

| Dimension | Assessment |
|-----------|-----------|
| Strengths | Full VISION.md alignment. Scoped activation matches .mdc pattern. Enables per-agent customisation. Cache-friendly if ordering is deterministic. |
| Weaknesses | Requires UI for managing directives. Circular dependency on topic classification (Category 5) for tag-based filtering. Over-engineered for current single-agent state. |
| Satisfies | R1, R2, R3, R4, R7, R9, R10 |

### Option C: Layered Template with Injection Points

Define a prompt template with named injection points (`{identity}`, `{security}`, `{tools}`,
`{project_context}`, `{environment}`, `{reminders}`). Each point maps to a graph query. Template
ordering follows the industry common pattern: identity → security → tools → behavioural →
provider → project → environment → preferences → reminders.

| Dimension | Assessment |
|-----------|-----------|
| Strengths | Explicit ordering for cache stability. Clear separation of static vs. dynamic. Compatible with the 4-layer model (doc 06). |
| Weaknesses | Template rigidity — new injection points require template changes. Template is not itself a graph node (breaks "graph as source of truth" principle). |
| Satisfies | R1, R4, R5, R7 |

### Option D: Event-Driven Incremental Assembly

System prompt assembled once at session start, incrementally updated via `GraphEvent`
notifications. `DirectiveAdded` / `DirectiveExpired` events trigger re-assembly. Content hash
comparison detects when the prompt actually changed.

| Dimension | Assessment |
|-----------|-----------|
| Strengths | Minimal redundant work. Natural fit with EventBus. Cache-aware: re-assembly only on content change. Hash tracking enables R9. |
| Weaknesses | Complexity of incremental updates. Race conditions if multiple events fire simultaneously. Requires EventBus to be battle-tested first. |
| Satisfies | R1, R4, R7, R9 |

---

## 5. Comparison Matrix

| Dimension | A: Static | B: Multi-Node | C: Template | D: Event-Driven |
|-----------|-----------|---------------|-------------|-----------------|
| VISION.md alignment | Low | High | Medium | High |
| Cache friendliness | None | High (deterministic) | High | High |
| Implementation cost | 1 day | 1-2 weeks | 1 week | 2+ weeks |
| Scoping support | None | Full | Partial | Full (with B) |
| Security posture | Basic (sanitise) | Basic + scope | Basic | Basic + scope |
| Multi-agent readiness | None | Yes (R10) | Partial | Yes (with B) |
| Operational complexity | Minimal | Medium (UI) | Low | High (events) |

---

## 6. VISION.md Alignment

| VISION.md Concept | Best Option | Blocked On |
|-------------------|-------------|------------|
| Dynamic system prompt (§4.7) | B or D | B needs UI; D needs EventBus |
| Pinning tiers (§4.7) | B | `scope` field on SystemDirective |
| Token budget visibility (§4.7) | Any | Token counting infrastructure |
| Export to CLAUDE.md (§4.7) | B | Serialisation of graph directives to markdown |
| Multi-perspective compaction (§4.2) | B + D | Topic classification (doc 14 Category 5) |
| Background processing (§4.3) | D | EventBus, async task scheduling |
| Deterministic context (§3.2) | B or C | Stable ordering algorithm |

The VISION describes a system that does not yet exist. The research recommends a migration
path delivering incremental value at each step, not an all-or-nothing leap. Option A is the
first step: it costs almost nothing, closes the highest-severity gap (no config file loading),
and is fully compatible with every subsequent option.

---

## 7. Recommended Architecture

### Phase 1: Option A + Foundations for B

1. **Load CLAUDE.md/AGENTS.md at startup.** Parse by heading sections. Store each section as
   a separate `SystemDirective` node with `scope: global` (field added to node). Link via
   `RespondsTo` chain at the root. Assembly is trivial: concatenate all global directives
   in insertion order.

2. **Plan context as a directive node.** Instead of string concatenation at line 55-58 of
   `conversational.rs`, emit plan context as a `SystemDirective` node with
   `scope: conversation`. This moves one step toward graph-native assembly.

3. **Content hash.** SHA-256 of the assembled system prompt string, stored on the agent loop
   context. Enables prompt version tracking for debugging without new infrastructure.

4. **Structured content blocks.** Change `system: Option<String>` to the Anthropic structured
   content block format. This is a prerequisite for `cache_control` breakpoints (doc 06).

### Phase 2: Option B (Scoped Directives)

When multi-agent and work items are mature:

1. Add `scope` (global / conversation / work_item / temporary) and `priority` fields to
   `SystemDirective`.
2. Assembly algorithm from doc 14: collect active directives → filter by scope → sort by
   priority → serialise with stable formatting.
3. Tag-based filtering deferred until topic classification (Category 5) exists.
4. Per-agent policy: `ConversationalPolicy` uses global + conversation directives.
   `TaskExecutionPolicy` adds work-item-scoped directives. `QuestionResponsePolicy`
   adds question-context directives.

### Phase 3: Option D (Event-Driven)

When EventBus is proven and concurrent agents are common:

1. System prompt assembled once at session start.
2. `GraphEvent::DirectiveAdded` / `GraphEvent::DirectiveExpired` trigger re-assembly.
3. Hash comparison: if unchanged, skip cache invalidation.
4. Incremental assembly for mid-conversation re-injection of temporary directives.

### Integration with 4-Layer Prompt Model (Doc 06)

System directives are Layer 1 content. Assembly must produce a stable byte prefix for cache
breakpoint placement. The move from `Option<String>` to structured content blocks (Phase 1
step 4) is the mechanical prerequisite.

---

## 8. Integration Design

### 8.1 Config File Loading

How the ecosystem handles project instructions:

- **Claude Code:** Loads project root CLAUDE.md + nested subdirectory files on-demand when
  files in those directories are read. Recommends under 300 lines. Frontier LLMs reliably
  follow 150-200 instructions; Claude Code's system prompt already consumes ~50, leaving
  budget for 100-150 in CLAUDE.md (empirical observation from community experience).
- **Cursor:** `.mdc` files with YAML frontmatter: `description`, `globs`, `alwaysApply`,
  `type`. Glob patterns scope rules to file paths.
- **AGENTS.md:** Emerging tool-agnostic standard. Claude Code reads it as fallback when no
  CLAUDE.md exists. Gaining cross-tool adoption.

For this project: load project-root CLAUDE.md at startup. Parse into sections by `##` heading.
Each section becomes a `SystemDirective` node. This preserves the graph-as-source-of-truth
principle while remaining compatible with the file-based ecosystem.

### 8.2 Token Budget Strategy

Best practice guidance suggests system prompts should consume 5-15% of context window.
This range appears across multiple production system analyses but lacks a single primary
source; treat it as an empirical starting point, not a hard constraint.

| Context Window | 5% Budget | 15% Budget | Current Usage |
|---------------|-----------|-----------|---------------|
| 180K tokens | 9K | 27K | ~50 tokens |
| 128K tokens | 6.4K | 19.2K | ~50 tokens |

The current ~50-token system prompt is far below any threshold. With CLAUDE.md loading, typical
usage rises to 200-500 tokens. With full directive assembly, expect 1K-5K tokens.

Recommendation: emit a diagnostic warning if system prompt exceeds 20% of context window.
Do not hard-cap — different tasks have legitimately different needs. Token budget allocation
should be dynamic based on agent role (coding tasks need more tool definitions; research tasks
need more system context).

### 8.3 Attention Degradation and Re-injection

Research documents a U-shaped attention curve: models attend most to content at the beginning
(primacy bias) and end (recency bias) of context, with significant degradation (>30%) for
content in the middle. Multiple architectural factors contribute, including Rotary Position Embedding (RoPE)
long-term decay characteristics in transformer attention heads.

**Practical impact:** System prompt instructions placed at the start receive high initial
attention but fade as conversations grow. Claude Code counteracts this by injecting "system
reminders" mid-conversation — special messages that re-state critical instructions.

**Architectural implication:** `SystemDirective` nodes with `scope: temporary` serve as
re-injection points. The Render stage of the context pipeline places them at the Layer 2/3
boundary (between compacted history and recent conversation), where they receive recency
attention from the model.

A case study found that reducing context from 180K to 60K tokens decreased hallucinations
by 35% and improved task completion accuracy from 68% to 89%. This validates the compaction
strategy (doc 24) and supports aggressive pruning of low-relevance content over retaining
everything "just in case."

### 8.4 XML Formatting

Anthropic officially recommends XML tags for Claude. Claude is fine-tuned to attend to XML
structure. Research findings:

- Anthropic fine-tuned Claude to attend to XML structure; XML is the only format all major
  providers (Anthropic, Google, OpenAI) explicitly encourage
- Research suggests XML outperforms JSON for structured reasoning tasks, though precise
  magnitude varies by domain and model; creative tasks may favour markdown
- Recommended tags: `<instructions>`, `<context>`, `<example>`, `<documents>`

For system directive serialisation: each directive wrapped in a scope-indicating tag with
content preserved verbatim. Long content at top, query at end (Anthropic long-context tips:
up to 30% quality improvement in multi-document inputs).

```xml
<system_directive scope="global" priority="1">
Always use async/await. Never use spawn_blocking unless justified.
</system_directive>

<project_context source="CLAUDE.md">
Use Rust language features FIRST: enums, structs, traits.
</project_context>
```

### 8.5 Security Considerations

Config files are untrusted input. The indirect prompt injection threat is well-documented:

- **Snyk ToxicSkills study:** 36.82% of agent skills contain security flaws at any severity
  level; 13.4% contain critical-level issues.
- **Attack vectors:** Malicious CLAUDE.md files, poisoned memory files, misleading filenames
  (e.g., "Important! Read me!.md") designed to be ingested by agents.
- **Persistence:** Instructions injected into memory files survive session restarts.

Claude Code defences (for reference): permission system for sensitive operations,
command blocklist (blocks `curl`, `wget` by default), isolated context windows for
web-fetched content, PostToolUse hooks as security scanners.

Mitigations for this project:
1. Sanitise config file content before injection (strip control characters, limit length)
2. Display loaded directives in the TUI for user awareness
3. Do not auto-execute tool calls originating solely from injected directives
4. Log content hashes of loaded files for audit trail

---

## 9. Red/Green Team

### Green Team (Factual Validation)

**Verified claims (12):**
- Claude Code 110+ conditional fragments: confirmed via Piebald-AI repository
- Sub-agents (Plan, Explore, Task): confirmed via Piebald-AI and Claude Code docs
- KnowGPT 92.6% on OpenBookQA: confirmed via arXiv 2312.06185 and NeurIPS 2024
- ContextBranch 58% context reduction: confirmed (specifically 58.1%, arXiv 2512.13914)
- Snyk ToxicSkills 36.82%: confirmed (1,467 of 3,984 scanned skills)
- 180K→60K hallucination reduction: confirmed via MCP deployment case studies
- Anthropic XML tag recommendation: confirmed via official docs ("trained specifically
  to recognize XML tags as a prompt organizing mechanism")
- Long content at top, up to 30% improvement: confirmed via Anthropic long-context tips
- CLAUDE.md under 300 lines: confirmed via Claude Code Best Practices
- Prompt caching 5-minute TTL: confirmed via Anthropic docs
- All file:line references: all 10 references verified against actual codebase
- SSE parser lacks message_start handler: confirmed

**Corrections applied:**
- XML accuracy benchmarks (23%/31%/18%) lacked verifiable primary sources. Replaced with
  qualitative statement that XML outperforms JSON for structured reasoning per Anthropic
  guidance. Domain specificity acknowledged.
- "Industry consensus" for 5-15% token budget rephrased as "best practice guidance" with
  caveat that no single primary source establishes this range.
- "150-200 instructions" frontier LLM limit marked as empirical observation, not formally
  established.
- RoPE attribution for U-shaped attention softened — RoPE contributes but is not the sole
  architectural cause.
- Codified Context 10.87% accuracy figure (arXiv 2602.20478): paper exists but specific
  metric not independently verified from abstract. Retained as cited.

**Unresolvable:** Industry references (Claude Code fragments, Cursor prompts) are based on
leaked/reverse-engineered data, not official documentation. They may be outdated by the
time of implementation. Treat as illustrative examples, not specifications.

### Red Team (Challenges)

**R1. Phase 1 lock-in risk.** The three-phase migration (A→B→D) creates a perverse incentive:
Phase 1 is cheap, immediately useful, and has no forcing function to trigger Phase 2. Phase 2
depends on multi-agent maturity and topic classification (Category 5), both of which may stall.
If Phase 1 becomes permanent, the system never achieves graph-native assembly. **Response:**
Valid. Phase 1 should include a documented "graduation criteria" checklist (e.g., "when >3
concurrent agents exist" or "when directive count exceeds N"). Without forcing functions, Phase
1 will persist indefinitely.

**R2. Missing option: file watcher reload.** The document omits runtime CLAUDE.md reloading
via filesystem watcher (`notify` crate). This bridges Phase 1→2 without full directive
machinery — ~100 lines of code, incremental, already standard in Cursor and LSP servers.
**Response:** Valid omission. A "Phase 1.5" with file watching would reduce lock-in risk
and make Phase 1 iterative without Phase 2 complexity.

**R3. Missing option: LLM-assisted directive selection.** A cheap model (Haiku) could select
which CLAUDE.md sections are relevant to the current task, reducing prompt bloat without
manual scoping. Cost: ~100 tokens per decision. **Response:** Interesting but adds an LLM
call to the critical path. Worth exploring as a Phase 2 alternative to tag-based filtering.

**R4. CLAUDE.md parsing fragility.** "Parse by heading sections" is undefined. Edge cases:
`##` inside code blocks, YAML frontmatter, h3-only files, no headings at all. No formal
grammar or reference parser specified. **Response:** Critical. Implementation must use a
markdown-aware parser (e.g., `pulldown-cmark`) that tracks code fences, not naive line
splitting. The document should specify: split on `## ` at top-level outside code blocks.

**R5. Security mitigations are vague.** "Strip control characters" does not specify which
characters (ASCII 0-31? Unicode bidi overrides? homoglyphs?). "Limit length" does not
specify per-section or per-file. No allowlist/denylist policy. A malicious CLAUDE.md could
inject fake system messages or declare tool definitions. **Response:** Valid. Security
section is directional, not actionable. Implementation must define: (a) concrete character
denylist, (b) per-file size limit, (c) prohibition on tool definition injection from config
files, (d) TUI display of loaded directives for user verification.

**R6. Token budget 5-15% lacks primary source.** Multiple blog posts repeat this range but
none cite a primary study. Tool definitions (separate API field) are excluded from this
budget, making the real Layer 1 footprint larger. The budget may differ by agent type
(coding vs. research) and model variant. **Response:** Corrected in document — rephrased as
empirical starting point. The recommendation should be: measure and adjust dynamically.

**R7. "Graph as source of truth" inconsistently applied.** Option C is dismissed for violating
this principle, but Option A also violates it — CLAUDE.md is a file, not a graph node. The
graph node is a copy that becomes stale if the file changes. **Response:** Valid
inconsistency. The principle applies to conversation data, not project configuration. Option A
is accepted because config files are inherently file-native; the graph synchronises with them
rather than replacing them. This distinction should be made explicit.

**R8. Event-driven assembly failure modes (Phase 3).** The EventBus uses tokio broadcast
channels with a buffer of 256. If events arrive faster than the assembly handler consumes,
events are silently dropped. Incremental assembly with dropped events produces stale prompts.
Ordering consistency under concurrent agent mutations is unspecified. **Response:** Valid.
Phase 3 needs: (a) event loss detection via sequence numbers, (b) fallback to full
re-assembly on detected loss, (c) single-writer guarantee for prompt assembly (serialise via
mpsc, not broadcast). These are implementation details, appropriate for the Phase 3 design
doc rather than this research document.

**R9. Multi-agent directive conflicts unresolved.** If global rules say "use enums" and a task
directive says "use macros," which wins? No exclusion mechanism exists for agents that need to
suppress global rules. Serialisation order affects cache stability but ordering algorithm is
undefined. **Response:** Valid. Phase 2 must define: (a) priority is ascending (higher number
= higher precedence, task overrides global), (b) per-policy directive filter lists for
exclusions, (c) deterministic ordering: scope tier first (global < conversation < work_item <
temporary), then priority within tier, then creation timestamp for ties.

**R10. Deferring to compaction may be higher ROI.** The 35% hallucination reduction came from
context reduction (compaction), not prompt engineering. Until compaction is implemented (doc
24), token budget is not the constraint — conversations grow unbounded regardless of system
prompt quality. **Response:** Partially valid. Compaction and prompt assembly address
different problems: compaction reduces noise in conversation history (Layers 2-3), prompt
assembly improves signal in instructions (Layer 1). Both matter, but compaction has higher
immediate impact on the documented pain point (unbounded growth).

---

## 10. Sources

### Production System Prompts
- [Piebald-AI/claude-code-system-prompts](https://github.com/Piebald-AI/claude-code-system-prompts) — Claude Code 110+ fragment analysis
- [Cursor Agent system prompt (March 2025)](https://gist.github.com/sshh12/25ad2e40529b269a88b80e7cf1c38084) — Cursor prompt architecture
- [Windsurf/Cascade system prompt](https://github.com/jujumilk3/leaked-system-prompts/blob/main/codeium-windsurf-cascade_20241206.md) — "Flow paradigm" approach

### Anthropic Documentation
- [Prompt engineering best practices](https://platform.claude.com/docs/en/build-with-claude/prompt-engineering/use-xml-tags) — XML tags, examples, role framing
- [Effective context engineering for AI agents](https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents) — Just-in-time context, compaction, sub-agents
- [Prompt caching](https://platform.claude.com/docs/en/build-with-claude/prompt-caching) — `cache_control`, stable prefixes, 5-min TTL
- [Claude prompting best practices](https://platform.claude.com/docs/en/build-with-claude/prompt-engineering/claude-prompting-best-practices) — Long content at top, query at end

### Configuration Patterns
- [Complete Guide to AI Agent Memory Files](https://medium.com/data-science-collective/the-complete-guide-to-ai-agent-memory-files-claude-md-agents-md-and-beyond-49ea0df5c5a9) — CLAUDE.md, AGENTS.md, .cursorrules comparison
- [AGENTS.md: One File to Guide Them All](https://layer5.io/blog/ai/agentsmd-one-file-to-guide-them-all/) — Tool-agnostic standard
- [Scaling Agent Context Beyond a Single AGENTS.md](https://ursula8sciform.substack.com/p/scaling-your-coding-agents-context) — Multi-file patterns
- [Claude Code Best Practices](https://www.anthropic.com/engineering/claude-code-best-practices) — Under 300 lines, repository-specific rules

### Academic Research
- [KnowGPT: Adaptive KG-to-prompt (arXiv 2312.06185)](https://arxiv.org/html/2312.06185v5) — RL subgraph selection, 92.6% accuracy
- [ContextBranch (arXiv 2512.13914)](https://arxiv.org/html/2512.13914v1) — 58% context reduction
- [Multi-Agent Design: Optimizing with Better Prompts (arXiv 2502.02533)](https://arxiv.org/abs/2502.02533) — Prompt-topology co-optimisation
- [Codified Context: Infrastructure for AI Agents (arXiv 2602.20478)](https://arxiv.org/html/2602.20478v1) — 10.87% accuracy from repo-specific rules
- [Lost in the Middle (TACL)](https://direct.mit.edu/tacl/article/doi/10.1162/tacl_a_00638/119630) — U-shaped attention, primacy/recency bias
- [MASFactory (arXiv 2603.06007)](https://arxiv.org/html/2603.06007) — Graph-centric multi-agent orchestration

### Developer Experience
- [Mastering Claude Code Best Practices](https://dinanjana.medium.com/mastering-the-vibe-claude-code-code-best-practices-that-actually-work-823371daf64c) — Over-specification as failure mode
- [CLAUDE.md optimisation with prompt learning](https://arize.com/blog/claude-md-best-practices-learned-from-optimizing-claude-code-with-prompt-learning/) — A/B testing, rich feedback
- [Why Long System Prompts Hurt](https://medium.com/data-science-collective/why-long-system-prompts-hurt-context-windows-and-how-to-fix-it-7a3696e1cdf9) — 180K→60K: 35% hallucination reduction
- [AI coding tools still suck at context](https://blog.logrocket.com/fixing-ai-context-problem/) — Developer frustrations

### Security
- [Snyk ToxicSkills study](https://snyk.io/blog/toxicskills-malicious-ai-agent-skills-clawhub/) — 36.82% skills with flaws
- [Claude Code Security docs](https://code.claude.com/docs/en/security) — Permission system, blocklist, sandboxing
- [OWASP LLM Prompt Injection Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/LLM_Prompt_Injection_Prevention_Cheat_Sheet.html) — Defence taxonomy
- [Prompt injection and agentic tools](https://www.securecodewarrior.com/article/prompt-injection-and-the-security-risks-of-agentic-coding-tools) — Indirect injection vectors

### Multi-Agent Frameworks
- [CrewAI vs LangGraph vs AutoGen](https://www.datacamp.com/tutorial/crewai-vs-langgraph-vs-autogen) — Per-agent prompt comparison
- [Comparing 4 Agentic Frameworks](https://medium.com/@a.posoldova/comparing-4-agentic-frameworks-langgraph-crewai-autogen-and-strands-agents-b2d482691311) — Memory and prompt patterns

### Internal References
- `docs/research/06-token-caching-strategies.md` — 4-layer prompt model, cache breakpoints
- `docs/research/14-system-and-agent-prompt-design.md` — 9 categories, industry survey, assembly algorithm
- `docs/research/22-graph-context-building-strategies.md` — 6-stage context pipeline
- `docs/research/24-graph-conversation-compaction.md` — Compaction and Layer 2 content
- `docs/design/04-graph-scheduler-qa-relationships.md` — ContextPolicy trait, multi-agent coordination
