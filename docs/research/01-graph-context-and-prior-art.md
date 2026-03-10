# Context Manager Research Findings: Graph-Based Context & Prior Art

> Research conducted 2026-03-10 by exploration agent investigating graph-based context management,
> message compaction, MergeTree-inspired background processing, and multi-rater relevance systems.

---

## Executive Summary

Your core idea of using a graph-based context system to codify and control LLM-driven software development is sound and aligns with the direction of multiple successful projects (GraphRAG, LangGraph, Mem0, LLM-as-Judge systems). However, the multi-perspective message compaction idea is relatively novel and requires careful validation. This document synthesizes findings across four major research areas.

---

## Topic 1: Graph-Based Context Management for LLMs

### Does the idea make sense?

**Yes. Strongly validated.**

Graph-based context management is not speculative — it's an active area of research and production deployment. The key insight is that graphs enable **multi-hop reasoning** and **targeted retrieval** without overwhelming the context window.

### How have others done this before?

1. **[Microsoft GraphRAG](https://microsoft.github.io/graphrag/)** - Production system that uses LLMs to extract entities and relationships, builds a knowledge graph, then performs two types of queries:
   - Global Search: reason over community summaries for holistic questions
   - Local Search: fan out from specific entities to their neighbors
   - Result: 70-80% win rate vs. naive RAG on comprehensiveness/diversity

2. **[Neo4j Knowledge Graphs + LLMs](https://neo4j.com/blog/genai/knowledge-graph-llm-multi-hop-reasoning/)** - Shows that combining graph databases with LLMs improves accuracy by 54.2% on average (Gartner data)

3. **[LangGraph](https://www.langchain.com/langgraph)** - Agentic workflow framework using directed graphs to manage state, nodes (functions), and edges (connections). Supports persistent state management and multi-agent workflows. MIT-licensed, actively developed.

4. **[Memgraph/FalkorDB](https://www.falkordb.com/)** - Specialized graph databases optimized for LLM workloads

5. **[Mem0](https://aws.amazon.com/blogs/database/build-persistent-memory-for-agentic-ai-applications-with-mem0-open-source-amazon-elasticache-for-valkey-and-amazon-neptune-analytics/)** - Memory orchestration layer that manages episodic, semantic, procedural, and associative memories for agents

### Possible approaches?

**Durable Master Graph + Query-Specific Subgraphs**

The convergence pattern is:
- Maintain a **persistent master graph** of the entire codebase, requirements, decisions, messages, tool outputs
- For each query/task, **extract a targeted subgraph** (neighborhood sampling)
- This keeps token budgets reasonable while preserving relationships
- Pair with **vector search within the subgraph** for final ranking

**Community Detection for Hierarchical Summarization**

Following GraphRAG's approach: automatically detect clusters of related entities (commits, PRs, requirements, discussions) and generate summaries at multiple levels of the hierarchy. This allows:
- Quick answers at high level ("what's the status?")
- Deep dives into specific communities when needed

**Graph Traversal Patterns**

Define explicit traversal rules:
- Relevance edges (A mentions B, B is tagged with C)
- Causality edges (fix A caused problem B)
- Dependency edges (task X depends on Y)
- Temporal ordering

### What could go wrong?

1. **Graph Construction Overhead**: Building accurate graphs requires high-quality entity extraction. Errors compound (garbage in, garbage out). You'll need continuous refinement.

2. **Staleness**: Master graph grows unbounded. Need aggressive archival/pruning strategies or background consolidation (see Topic 3).

3. **Relationship Ambiguity**: A message might be related to multiple things. How to weight edges? Multi-perspective evaluation (Topic 4) becomes critical.

4. **Query Formulation**: Users must learn how to query the graph. Simple keyword search won't suffice. Need query understanding or query rewriting.

5. **Edge Density**: In large projects, the graph can become a hairball. Need principled pruning (recent-first, relevance-scored, topic-filtered).

### Red team / Green team

**Red team (risks):**
- Expensive graph maintenance: Every new message requires entity extraction (LLM call), relationship detection, conflict resolution
- Lost-in-the-middle problem doesn't vanish: Even with perfect subgraphs, LLMs still fail on long contexts
- Graph updates are asynchronous: Race conditions where agent decisions are based on stale edges
- Integration hell: Existing tools (Git, issue trackers, CI/CD) aren't graph-aware; need robust ETL

**Green team (strengths):**
- Enables **explainable decisions**: "Here's the subgraph we used to answer your question"
- **Separates concerns**: Graph maintenance is decoupled from reasoning
- **Reusable context**: Same subgraph can answer many questions
- **Foundation for reliability**: Audit trail built-in by design

---

## Topic 2: Message Compaction / Summarization with Multiple Perspectives

### Does the idea make sense?

**Partially. The multi-perspective angle is novel but risky.**

Single-perspective summarization is well-understood (used by Claude, ChatGPT internally). **Multi-perspective compaction is less established but theoretically sound** — the same event can be relevant differently depending on the observer's role/goal.

### How do current tools handle context window limits?

[Claude's Automatic Context Compaction](https://platform.claude.com/docs/en/build-with-claude/compaction) provides a practical reference:
- At ~95% capacity, Claude summarizes older messages automatically
- Preserves full history server-side (users can still reference old stuff)
- Summaries are lossy but identify key points
- Observations: accuracy/recall degrade as token count grows ("context rot") — LLMs remember beginning/end clearly, middle gets "fuzzy"

Other approaches:
- **Prompt compression** (LLMLingua): Token-level pruning guided by model signals, preserving meaning while cutting 20-50% of tokens
- **Verbatim compaction**: Delete low-signal tokens but keep survivors character-for-character identical (zero hallucination risk but lower compression ratios)
- **Dynamic summarization**: Keep last N turns in full, plus rolling summary of everything older

### Is multi-perspective compaction a novel idea? Has anyone done this?

**Mostly novel, but adjacent work exists:**

1. **[Personalized Summarization](https://arxiv.org/html/2410.14545v1)** - Research shows LLMs can summarize the same meeting differently for different personas (Product Owner vs. Technical Lead vs. QA):
   - Same content, different emphasis
   - Questions vary based on persona
   - Measurable viewpoint-specific differences

2. **[Multi-Agent Debate for Evaluations](https://mirascope.com/tutorials/prompt_engineering/chaining_based/sim_to_m/)** - Framework where agents with different identities debate to reach consensus:
   - Shows subjective analysis fosters diverse interpretation
   - Avoids mono-perspective bias

3. **Aggressive Compression Harms Minorities** - Critical finding: "Cutting tokens silences minority viewpoints in long feedback collections." If you compress too aggressively, you lose alternative views.

### Possible approaches?

1. **Perspective Indexing**:
   - Tag each message with implicit personas/topics (e.g., "security consideration", "performance trade-off", "user experience")
   - When compacting, generate multiple summaries — one optimized for each perspective
   - Store all as nodes connected to original via edges

2. **Selective Retention**:
   - Different perspectives have different "signal" thresholds
   - Marketing might care about user feedback; backend engineer might care about error traces
   - Use multi-rater scoring (Topic 4) to decide what to compress for whom

3. **Debate-Based Compression**:
   - Spawn mini-agents with different viewpoints to jointly decide what's important
   - Synthesize consensus summary plus minority-view summary
   - Store both; context selection uses the relevant one

### What could go wrong / what are we missing?

1. **Hallucination Risk**: Summarization is inherently lossy. Research shows:
   - Hallucinations are "predictable compression failures" when information budgets fall below thresholds
   - 12.5% compression threshold: beyond this, LLMs enter "lossy zone" with imperfect recall
   - Technical details (file paths, exact error messages) get paraphrased or lost

2. **Perspective Explosion**: If you generate K perspectives for each message, storage and retrieval become expensive. K = 5 perspectives means 5x more summaries to maintain.

3. **Calibration Nightmare**: How do you know your "security-focused summary" actually captures security risks? Requires ground truth or human validation.

4. **Information Coupling**: One person's minority view might be critical for another task. Aggressive pruning burns bridges you'll later need.

5. **Missing Context Rot**: Even multi-perspective summaries don't solve the core problem — LLMs perform worse with longer context. You still hit the wall.

### Red team / Green team

**Red team:**
- Multi-perspective summarization adds complexity without proven ROI
- Hallucination risk is non-trivial: A compacted message that looks good might hide critical details
- Perspectives are hard to define objectively (who decides what matters?)
- Storage overhead of K perspectives defeats the purpose of compression

**Green team:**
- Reduces information loss vs. single-summary approach
- Enables context selection: "Give me the developer's view of this PR discussion" vs. "give me the business value view"
- Aligns with how humans actually think (same fact matters differently in different contexts)
- Foundation for auditable decisions: "This decision was made using the security-focused summary"

---

## Topic 3: Background Processes for Graph Maintenance (MergeTree Inspiration)

### Does the idea make sense?

**Yes, but with important caveats about cost and latency.**

Continuous background LLM processing is an emerging pattern (MemGPT, Letta's continual learning, MergeTree inspiration). The analogy is sound: **spend background compute to reorganize data so foreground operations (user queries) are fast.**

### ClickHouse MergeTree deep dive

MergeTree is **not an LLM system**, but the architectural insight is valuable:

[ClickHouse MergeTree Engine](https://clickhouse.com/docs/engines/table-engines/mergetree-family/mergetree):
- Data arrives as separate **immutable parts** (e.g., hourly data)
- Background process **continuously merges** smaller parts into larger ones
- Key win: Two sorted parts can merge with a **single linear scan** (interleave rows, no re-sorting, no temporary buffers)
- Original parts marked inactive and deleted when no queries reference them
- Frequency/settings are configurable (balance storage vs. resource use)
- **Result**: Write-optimized (append parts immediately), read-optimized (queries use few, large parts)

### How could this apply to context graphs?

1. **Message Parts**: New interactions arrive as fine-grained parts (individual messages, tool calls, decisions)
2. **Background Consolidation**: Spawn background tasks that:
   - Detect clusters of related parts (using graph analysis + embeddings)
   - Call LLMs to generate summaries/compactions for each cluster
   - Replace N small parts with 1-2 large consolidated parts
   - Update graph edges to point to new consolidated nodes

3. **Merge Strategy**:
   - Merge-by-age: Older parts get more aggressive consolidation
   - Merge-by-topic: Parts about same feature get merged together
   - Merge-by-relevance: Unused parts get archived/pruned

### Possible approaches?

**Tiered Architecture**:
```
Tier 1 (Hot): Recent messages, active discussions
Tier 2 (Warm): Last week's work, occasionally queried
Tier 3 (Cold): Archived decisions, rarely accessed
Tier 4 (Archive): Historical record, search-only
```

Background processes migrate from Tier 1 → 2 → 3 → 4, compacting at each step.

**Async Compaction Pipeline**:
- Inbox: Raw events arrive
- Analysis: Background LLM extracts structure, detects clusters
- Summarization: Generate multi-perspective summaries (Topic 2)
- Scoring: Rate relevance/importance for different contexts (Topic 4)
- Consolidation: Write summary node, update edges
- Cleanup: Delete/archive original parts if score is low

**Conflict Resolution**:
- Original parts are immutable (like MergeTree)
- Summaries are separate nodes (can be regenerated if needed)
- Graph points to both raw and summary (user can drill down)

### Is continuous background LLM processing cost-effective?

**Nuanced. Context-dependent.**

[Research findings on LLM inference costs](https://arxiv.org/html/2509.18101v1):
- On-premise LLMs: 2.6x more cost-effective than IaaS, 4.1x vs. GPT-4 API
- API-based (OpenAI, Claude): Cost per evaluation is $0.01-0.10
- Techniques can reduce inference cost by up to 80% (quantization, distillation, batch optimization)

**For your use case:**
- Small teams (<50 devs, <1000 commits): Background processing likely 5-20% overhead, worthwhile
- Large teams (100s of devs, 10k+ commits): Risk of runaway costs; need tight budget controls
- Best case: Use cheaper models for background (Claude 3.5 Haiku) and reserve strong models (Claude 3.5 Sonnet) for user-facing queries

### What about latency implications?

[Continuous batching research](https://huggingface.co/blog/continuous_batching) shows:
- Async/background tasks don't block user requests if properly architected
- **But**: Stale data. If background compaction hasn't finished, user gets old subgraph
- Mitigation: Maintain both raw and compacted versions; update graph asynchronously

### What could go wrong?

1. **Runaway Background Jobs**: Without cost controls, background processes explode. Need to cap:
   - Max LLM calls per day for compaction
   - Max tokens per compaction task
   - Only compact when below cost budget

2. **Stale Compactions**: A background summary might be based on incomplete data. Later messages contradict it. No way to "recall" a compaction.

3. **Context Loss**: Aggressive MergeTree-style merging loses nuance. A 100-message discussion becomes a 3-sentence summary. User later needs the 100 messages, but they're deleted.

4. **Complexity**: MergeTree works because it's just data reorganization. Compacting LLM conversations requires LLM inference, which is non-deterministic. Versioning and rollback are hard.

5. **Configuration Hell**: How many tiers? When to merge? Which LLM for background tasks? Different projects need different settings.

### Red team / Green team

**Red team:**
- Background processing is often waste: compact something nobody queries
- Stale graph: compaction finishes after the user's decision is already made
- Hard to debug: background failures are silent until query hits stale data
- Overshooting MergeTree analogy: MergeTree data is deterministic; LLM summaries are probabilistic

**Green team:**
- Extends effective horizon: background processing can compact older interactions, freeing up token budget
- Scales better: without compaction, graph grows unbounded; with it, you can handle multi-year projects
- Enables time-travel: if you keep old parts around, you can replay decisions
- Data-aware compaction: unlike naive summarization, graph-aware merging preserves relationships

---

## Topic 4: Relevance Rating System (Multi-Rater)

### Does the idea make sense?

**Yes. Strongly validated in production LLM systems.**

LLM-as-Judge is an active, proven field. [Multiple judges improve correlation with human judgment](https://eugeneyan.com/writing/llm-evaluators/). Multi-rater systems are real and deployed.

### How do existing systems score relevance?

1. **[LLM-as-a-Judge](https://www.evidentlyai.com/llm-guide/llm-as-a-judge)** - Core framework:
   - Prompt an LLM with: input + output + scoring rubric → judge returns score/label
   - Single Output Scoring: Judge scores one piece without reference
   - Pairwise Comparison: Judge chooses better of two options
   - Multi-Judge: Run multiple judges, aggregate via voting or averaging

2. **[Cross-Encoders for Relevance](https://adasci.org/a-hands-on-guide-to-enhance-rag-with-re-ranking/)** - Specialized models (BERT-based) that jointly encode query + document for precise relevance scores:
   - More accurate than embedding similarity
   - ~0.5-2ms per pair on GPU
   - Cheaper than LLM judges but less flexible

3. **[NDCG and Ranking Metrics](https://deconvoluteai.com/blog/rag/metrics-retrieval)** - Standard IR metrics:
   - NDCG: normalized score accounting for position (top-k results weighted higher)
   - MRR (Mean Reciprocal Rank): position of first correct result
   - Recall@k: fraction of relevant items in top-k

4. **[RankRAG](https://arxiv.org/html/2407.02485v1)** - Unified framework:
   - LLM reranks retrieved contexts **while generating answer**
   - Learns to balance relevance ranking + task performance
   - Outperforms naive retrieval + generation

### What's the cost of using multiple LLMs as raters?

[Cost analysis](https://www.confident-ai.com/docs/llm-evaluation/core-concepts/llm-as-a-judge):
- Cost per evaluation: $0.01-0.10 (varies by model/token count)
- For 1000 evaluations: $10-100
- For 10,000 evaluations: $100-1000

**Budget strategies:**
- Cascade: Use cheap judge first (Haiku), escalate to strong judge (Sonnet) only if uncertain
  - Empirically: ~20-30% escalation rate, 70% cost savings vs. all-Sonnet
- Pairwise comparison is cheaper than absolute scoring (fewer tokens, simpler decision)

### Calibration problems between raters?

[Significant challenges documented](https://www.kinde.com/learn/ai-for-software-engineering/best-practice/llm-as-a-judge-done-right-calibrating-guarding-debiasing-your-evaluators/):

1. **Rater Disagreement**: GPT-4 achieves 80%+ agreement with humans; humans among themselves ~80%. So LLMs ≈ human-level but not identical.

2. **Domain Expertise Gaps**: LLM judges struggle in specialized domains (medicine, law, low-resource languages) — accuracy drops significantly

3. **Surface-Level Fluency Bias**: LLMs overvalue surface quality and miss subtle errors

**Calibration solutions**:
- Anchor examples: Pre-graded samples in prompt ("5-star response looks like X, 1-star like Y")
- Few-shot learning: Include examples of correct and incorrect judgments
- Rubrics: Detailed scoring guidelines, not just "rate relevance 1-5"
- Logprobs: Use model's confidence (log probabilities) to weight multi-judge outputs
- Human validation set: 30-50 examples evaluated by domain experts, measure judge agreement. If >20% disagreement, iterate on prompt before deploying

### Possible approaches for your context manager?

1. **Multi-Rater Relevance Scoring**:
   - When a node is queried, run 3 judges in parallel:
     - Judge A: General-purpose (Claude 3.5 Haiku)
     - Judge B: Code-aware (fine-tuned or specialized prompting)
     - Judge C: Domain-aware (specific to your team's domain)
   - Aggregate scores via weighted average (weight by calibration accuracy)
   - Store scores as node metadata

2. **Perspective-Based Judges**:
   - Judge: "Is this relevant for SECURITY review?"
   - Judge: "Is this relevant for PERFORMANCE review?"
   - Judge: "Is this relevant for DESIGN review?"
   - Each question gets a separate score
   - Used by compaction/summarization (Topic 2) to select what to keep

3. **Iterative Recalibration**:
   - Monthly: Sample 50 nodes that were rated and marked for archival
   - Ask domain experts: "Was this archived correctly?"
   - Measure judge disagreement; update prompts/rubrics if >20% error

4. **Cascade Evaluation**:
   - Fast check: Embed node, compute cosine similarity to query (cheap)
   - If score > 0.9 or < 0.2: Use that score
   - If 0.2-0.9: Escalate to LLM judge (slow but accurate)
   - Result: 70% of nodes evaluated for $0.001, 30% for $0.10

### What could go wrong?

1. **Judge Hallucination**: LLM judge scores high relevance to a node that's actually unrelated, then the subgraph includes it, misleading the main model

2. **Cascade Failures**: Fast heuristic (embedding similarity) misses contextual relevance that only an LLM would catch. System uses cheap, wrong scores.

3. **Feedback Loops**: Judge learns from the system's outputs (e.g., "nodes in the final answer are probably relevant"). Circular reasoning.

4. **Domain Drift**: Judge calibrated on old code/requirements. Codebase evolves, judge doesn't. Relevance scores become stale.

5. **Expensive to Scale**: 10,000 nodes, 3 judges each, means 30,000 LLM calls. At $0.01 each = $300. Monthly recalibration = $3600/year per project.

### Red team / Green team

**Red team:**
- Multiple judges don't guarantee correctness, just reduce variance
- Calibration is ongoing work, not one-time
- Cost creep: Easy to start with 3 judges, end up with 10 (business context, security context, architectural context...)
- Judge disagreement unresolved: If Judge A says relevant and Judge B says not, how to weight?

**Green team:**
- Empirically proven: Multi-rater evaluation correlates well with human judgment
- Explainability: "Sonnet rated this 0.8 relevant to your query" is actionable feedback
- Flexible: Same framework works for many judgment tasks (relevance, importance, correctness, sentiment)
- Scaling: Can be made cost-effective with cascading + aggregation

---

## Synthesis & Recommendations

### What makes sense to build first?

1. **Graph foundations** (Topic 1):
   - Start with a simple graph: nodes (messages, requirements, decisions), edges (mentions, depends-on, contradicts)
   - Use LLM extraction (one-shot, guided by schema) for reliable entity/relationship detection
   - Build query interface: given a task/question, extract subgraph and return as context

2. **Basic compaction** (Topic 2 - simpler version):
   - Start with single-perspective summarization (simpler than multi-perspective)
   - Use Claude's approach: rolling summaries of older messages
   - Defer multi-perspective until you have data showing it's needed

3. **Cheap relevance scoring** (Topic 4):
   - Embed all nodes once (one-time cost)
   - For each query, use embedding similarity as primary filter
   - Optional LLM judge only for edge cases (top-ranked nodes, low-confidence cases)

4. **Background processes** (Topic 3):
   - Start simple: Nightly archival of nodes older than 30 days
   - Skip LLM-based compaction initially; use deterministic summarization
   - Add LLM-driven compaction once you have metrics proving it's worth the cost

### What's risky or premature?

1. **Multi-perspective compaction right away**: Build single-perspective first, measure if it's actually lossy in practice
2. **Aggressive MergeTree-style background processing**: Start read-heavy, add background optimization once the graph is large enough to need it
3. **Cascading judge evaluation**: Measure if 1 judge is good enough before adding complexity

### Missing / to investigate further

1. **Determinism and reproducibility**: How to version the graph? If a background process changes a summary, how do you audit the change?
2. **Query interface**: How does a developer ask the system questions? Keyword search + graph traversal? Natural language? Structured queries?
3. **Integration with existing tools**: How does data flow from Git, issue trackers, Slack, code review tools into the graph?
4. **Visualization**: Can developers *see* the graph? Essential for debugging and trust.
5. **Cold-start problem**: First day, the graph is empty. How to bootstrap it? Crawl Git history? Summarize PRs retroactively?

---

## Sources

### Topic 1: Graph-based Context Management
- [Context Graphs: A Practical Guide](https://medium.com/@adnanmasood/context-graphs-a-practical-guide-to-governed-context-for-llms-agents-and-knowledge-systems-c49610c8ff27)
- [GraphRAG: Microsoft Research](https://www.microsoft.com/en-us/research/blog/graphrag-unlocking-llm-discovery-on-narrative-private-data/)
- [GraphRAG GitHub](https://github.com/microsoft/graphrag)
- [Neo4j Knowledge Graph & LLM Multi-Hop Reasoning](https://neo4j.com/blog/genai/knowledge-graph-llm-multi-hop-reasoning/)
- [LangGraph: Multi-Agent Workflows](https://blog.langchain.com/langgraph-multi-agent-workflows/)
- [Why Knowledge Graphs for LLM Personalization](https://memgraph.com/blog/why-knowledge-graphs-for-llm)
- [Graph Database Performance for LLM Applications](https://neo4j.com/blog/genai/advanced-rag-techniques/)
- [Glean: Knowledge Graphs in Agentic Engines](https://www.glean.com/blog/knowledge-graph-agentic-engine)

### Topic 2: Message Compaction & Multi-Perspective Summarization
- [How We Extended LLM Conversations by 10x with Intelligent Context Compaction](https://dev.to/amitksingh1490/how-we-extended-llm-conversations-by-10x-with-intelligent-context-compaction-4h0a)
- [The Fundamentals of Context Management and Compaction in LLMs](https://kargarisaac.medium.com/the-fundamentals-of-context-management-and-compaction-in-llms-171ea31741a2)
- [ForgeCode: Context Compaction](https://forgecode.dev/docs/context-compaction/)
- [Claude API: Automatic Context Compaction](https://platform.claude.com/cookbook/tool-use-automatic-context-compaction)
- [Claude API: Compaction Docs](https://platform.claude.com/docs/en/build-with-claude/compaction)
- [Compaction vs Summarization: Agent Context Management Compared](https://www.morphllm.com/compaction-vs-summarization)
- [Understanding Claude's Conversation Compacting](https://www.ajeetraina.com/understanding-claudes-conversation-compacting-a-deep-dive-into-context-management/)
- [Predictable Compression Failures: Why Language Models Hallucinate](https://arxiv.org/abs/2509.11208)
- [Personalized Abstractive Multi-Source Meeting Summarization](https://arxiv.org/html/2410.14545v1)
- [Multi-Perspective Evaluation Framework](https://arxiv.org/html/2412.05579v2)

### Topic 3: Background Processes & MergeTree
- [Continual Learning in Token Space (Letta)](https://www.letta.com/blog/continual-learning)
- [ClickHouse MergeTree Engine Docs](https://clickhouse.com/docs/engines/table-engines/mergetree-family/mergetree)
- [Understanding ClickHouse MergeTree](https://chistadata.com/understanding-clickhouse-mergetree-data-organization-merging-replication-and-mutations-explained/)
- [Continuous Batching from First Principles](https://huggingface.co/blog/continuous_batching)
- [LLM Context Management: Performance & Latency](https://eval.16x.engineer/blog/llm-context-management-guide)
- [Async Operations in LangChain](https://apxml.com/courses/langchain-production-llm/chapter-1-advanced-langchain-architecture/async-concurrency)
- [Effective Context Engineering for AI Agents (Anthropic)](https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents)
- [A Cost-Benefit Analysis of LLM Deployment](https://arxiv.org/html/2509.18101v1)
- [LLM Inference Optimization Techniques](https://www.clarifai.com/blog/llm-inference-optimization/)

### Topic 4: Relevance Rating & Multi-Rater Systems
- [LLM-as-a-Judge: Complete Guide](https://www.evidentlyai.com/llm-guide/llm-as-a-judge)
- [Confident AI: LLM-as-a-Judge Best Practices](https://www.confident-ai.com/blog/why-llm-as-a-judge-is-the-best-llm-evaluation-method)
- [Evaluating the Effectiveness of LLM-Evaluators](https://eugeneyan.com/writing/llm-evaluators/)
- [Langfuse: LLM-as-a-Judge Evaluation](https://langfuse.com/docs/evaluation/evaluation-methods/llm-as-a-judge)
- [LLM-as-a-Judge: 7 Best Practices](https://www.montecarlodata.com/blog-llm-as-a-judge/)
- [Kinde: LLM-as-Judge Done Right](https://www.kinde.com/learn/ai-for-software-engineering/best-practice/llm-as-a-judge-done-right-calibrating-guarding-debiasing-your-evaluators/)
- [Multi-Perspective LLM Evaluation Framework](https://arxiv.org/html/2412.05579v2)
- [RAG Evaluation: Metrics, Testing & Best Practices](https://www.evidentlyai.com/llm-guide/rag-evaluation)
- [RankRAG: Unifying Context Ranking with Retrieval-Augmented Generation](https://arxiv.org/html/2407.02485v1)
- [Rerankers and Two-Stage Retrieval](https://www.pinecone.io/learn/series/rag/rerankers/)
- [Layered Ranking for RAG Applications](https://blog.vespa.ai/introducing-layered-ranking-for-rag-applications/)

### Supporting Research
- [AI Agent Memory: Build Stateful AI Systems](https://redis.io/blog/ai-agent-memory-stateful-systems/)
- [LLM Context Window Limitations](https://atlan.com/know/llm-context-window-limitations/)
- [LLM Context Window Overflow in 2026](https://redis.io/blog/context-window-overflow/)
- [How to Write a Good Spec for AI Agents](https://www.oreilly.com/radar/how-to-write-a-good-spec-for-ai-agents/)
- [Software Development Process with Autonomous Agents](https://www.gocodeo.com/post/the-ai-driven-software-development-process-from-requirements-to-deployment-with-autonomous-agents/)
- [Memory for AI Agents: Designing Persistent, Adaptive Memory Systems](https://medium.com/@20011002nimeth/memory-for-ai-agents-designing-persistent-adaptive-memory-systems-0fb3d25adab2)
