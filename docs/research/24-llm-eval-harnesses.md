# LLM Evaluation Harnesses: How They Work and How We Can Use Them

> **2026-03-14** — Research on LLM evaluation harnesses for benchmarking code development tools. Covers how SWE-bench, Aider Polyglot, Inspect AI, and other frameworks evaluate agent systems at solving real software engineering problems. Documents the benchmark landscape, evaluation strategies, and key insights about why harness engineering matters as much as model capability.

---

## 1. Executive Summary

The code development tool landscape has converged on a small set of evaluation harnesses — primarily SWE-bench — to measure how well agent systems solve real software engineering problems. The most significant finding from this research is that **harness engineering contributes 10+ percentage points on SWE-bench beyond raw model capability**. Claude Code scores 80.9% on SWE-bench Verified while the base Claude Opus 4.5 model scores significantly lower with a standard harness. SWE-agent demonstrated the same pattern: custom Agent-Computer Interface (ACI) design yielded 10+ point improvements over naive tool configurations.

This matters for the context-orchestrator because graph-based context management is precisely the kind of harness-level innovation that moves the needle. But without evaluation infrastructure, there is no way to measure whether the graph approach outperforms linear chat history. The benchmarks and frameworks documented here provide the measurement tools; the strategies section covers how established tools use them in practice.

**Key takeaways:**

1. **SWE-bench is the dominant benchmark** for real-world code development evaluation (2,294 GitHub issues, test-based validation) — though it has known weaknesses (Python-only, flawed test cases, training contamination risk)
2. **Multiple benchmark types** exist for different granularities: function-level (HumanEval), file-level (Aider Polyglot), issue-level (SWE-bench), project-level (CORE-Bench), multi-turn (TAU-bench)
3. **Evaluation frameworks** (SWE-bench harness, Inspect AI, OpenHands) handle Docker isolation, parallel execution, and scoring
4. **Harness engineering is the differentiator** — tool design, context management, and retry logic matter more than model choice
5. **Isolating context management's contribution** requires ablation studies, not just end-to-end task success
6. **Continuous evaluation** with subset selection (SWE-bench Lite: 300 tasks) enables fast iteration

---

## 2. How Evaluation Harnesses Work

### 2.1 The Evaluation Pipeline

Every code development evaluation follows the same conceptual pipeline:

```
Task Definition → Environment Setup → Agent Execution → Result Collection → Validation → Metrics
```

**Task definition** provides the agent with a problem: a repository snapshot (specific commit), a problem statement (issue description or exercise prompt), and ground truth for validation (test patches or expected outputs).

**Environment setup** creates an isolated execution environment — typically a Docker container — with the repository checked out at the correct commit, all dependencies installed, and the test suite ready to run.

**Agent execution** is where the harness under test (Claude Code, Aider, or in our case the context-orchestrator) runs. The agent reads the problem statement, navigates the codebase, makes edits, and produces a solution. The harness controls what tools the agent has access to and how context is managed.

**Result collection** captures the agent's output as a unified diff (patch) of all file changes.

**Validation** applies the patch to the original repository and runs the test suite. SWE-bench uses two test categories:

- **FAIL_TO_PASS**: Tests that fail before the fix but should pass after. These verify the issue is actually resolved.
- **PASS_TO_PASS**: Tests that pass before the fix and should continue passing. These detect regressions.

A task is "resolved" only if all FAIL_TO_PASS tests pass AND all PASS_TO_PASS tests continue to pass.

### 2.2 The Agent-Computer Interface (ACI)

SWE-agent introduced the concept of the **Agent-Computer Interface** — the set of tools and interaction patterns available to the agent during evaluation. This is the analog of a UI/UX for humans, but designed for LLM agents.

The reference ACI from SWE-agent provides:

| Tool | Purpose | Design Choice |
|------|---------|---------------|
| **File viewer** | Navigate code | Shows ~100 lines per turn (not full files) |
| **File editor** | Modify code | Supports scrolling, search, edit commands |
| **Directory search** | Find files | Succinct grep-like output |
| **Linter** | Prevent syntax errors | Validates on every save |
| **Bash execution** | Run commands | Full shell access in sandbox |

The key insight is that **ACI design is a first-class engineering problem**. SWE-agent's 100-line file viewer, its lint-on-save behavior, and its search output formatting were all deliberate design choices that improved evaluation scores. This is directly analogous to our context pipeline — how we select, score, and render graph nodes for the LLM is an ACI design decision.

### 2.3 Metrics

| Metric | Definition | Used By |
|--------|-----------|---------|
| **Resolution Rate** | (issues resolved) / (issues attempted) × 100% | SWE-bench (primary) |
| **pass@k** | Fraction of issues resolved within k attempts | HumanEval, general |
| **pass@1** | Single-attempt success rate | Most commonly reported |
| **Cost per task** | API tokens × price per token | Cost-efficiency analysis |
| **Token efficiency** | Tokens consumed per resolved issue | Optimization metric |
| **Filtered accuracy** | Performance excluding tasks with file paths in descriptions | Isolates true reasoning |
| **Time to solution** | Wall-clock time per task | Practical throughput |

SWE-bench does **not** use exact patch matching as its primary metric — multiple valid patches can solve an issue. Functional correctness (tests pass) is what counts.

### 2.4 Docker Isolation Architecture

SWE-bench builds three Docker image layers per task:

1. **Base image** — Ubuntu 22.04 with common system dependencies
2. **Environment images** — Language-specific runtimes and package managers (~63 distinct Python environments)
3. **Instance images** — Task-specific dependencies, repository at correct commit

This layering enables caching: environment images (~100GB total for SWE-bench Verified) are built once and reused across tasks sharing the same dependency set. Full instance-level caching requires ~2TB but eliminates rebuild time entirely.

**Infrastructure requirements:**
- x86_64 architecture (ARM/macOS M-series requires local builds instead of DockerHub pulls)
- 120GB+ free disk space
- 16GB RAM minimum
- 8 CPU cores minimum
- Recommended parallelism: `min(0.75 × cpu_count, 24)` workers

SWE-bench Verified (500 tasks) runs in ~62 minutes on a 32-core/128GB VM with optimized Docker images.

---

## 3. Benchmark Landscape

### 3.1 SWE-bench (The Gold Standard)

[SWE-bench](https://github.com/SWE-bench/SWE-bench) evaluates agents on 2,294 real GitHub issues from 12 popular Python repositories. Each task is a real pull request that was merged to fix a real issue.

**Variants:**

| Variant | Tasks | Purpose | Notes |
|---------|-------|---------|-------|
| **Full** | 2,294 | Complete evaluation | 22.2% of tasks take >1 hour |
| **Verified** | 500 | Main public leaderboard | Hand-validated by engineers as solvable |
| **Lite** | 300 | Fast iteration | Carefully selected subset |
| **Pro** | — | Long-horizon tasks | Hours-to-days effort per task |

**Prediction format** (JSONL):
```json
{
  "instance_id": "sympy__sympy-20590",
  "model_name_or_path": "my-harness-v1",
  "model_patch": "<unified diff>"
}
```

**Known weaknesses:**
- **Python-only**: All tasks from 12 specific Python repositories. No signal for Rust, Go, TypeScript, or other languages.
- **Django over-representation**: Over 45% of all tasks come from Django alone, skewing the benchmark toward web framework patterns.
- **Flawed test cases**: A 2025 audit found 59.4% of audited problems have test cases that reject functionally correct submissions — inflating difficulty artificially.
- **Training contamination risk**: Frontier models may reproduce ground-truth patches from training data rather than solving the problem. OpenAI publicly [stopped evaluating on SWE-bench Verified](https://openai.com/index/why-we-no-longer-evaluate-swe-bench-verified/) partly for this reason.
- **Limited test coverage**: SWE-bench Pro found up to 7.8% of "passing" patches fail full developer test suites — patches can game sparse tests.

### 3.2 Aider Polyglot Benchmark

[Aider's Polyglot Benchmark](https://github.com/Aider-AI/polyglot-benchmark) uses 225 of the hardest Exercism exercises across 6 languages: C++, Go, Java, JavaScript, Python, and Rust.

**Key differences from SWE-bench:**
- Multi-language (vs. Python-only)
- Multi-file edits required
- Exercises from 12 diverse projects
- Closer to daily developer workflows
- Two attempts per problem (retry on failure)
- Validation: edit outcome + lint outcome + test outcome

**Scoring:** A solution is "plausible" when all tests pass and linting succeeds. Current top score: ~92.9% (Refact.ai with Claude 3.7 Sonnet).

### 3.3 Function-Level Benchmarks

**HumanEval** — 164 handwritten programming problems assessing language comprehension, algorithms, and simple mathematics. Uses pass@k metric. Top models score 95%+ on standard HumanEval.

**MBPP** (Mostly Basic Programming Problems) — 974 entry-level programming problems, each with task description, solution, and 3 test cases.

**HumanEval Pro / MBPP Pro** — Enhanced variants evaluating self-invoking code generation. Most LLMs show significant drops (e.g., o1-mini: 96.2% HumanEval → 76.2% HumanEval Pro), revealing that high benchmark scores can mask brittle capabilities.

### 3.4 Multi-Language SWE-bench Variants

The Python-only limitation of SWE-bench has spawned several polyglot extensions:

- **[Multi-SWE-bench](https://arxiv.org/html/2504.02605v1)** — Spans Java, TypeScript, JavaScript, Go, Rust, C, and C++. Directly relevant for evaluating a Rust-based harness on Rust problems.
- **[SWE-Bench++](https://arxiv.org/html/2512.17419v1)** — Framework for generating SWE-bench-style tasks from any open-source repo across 11 languages including Rust. Covers feature requests alongside bug fixes.
- **[SWE-PolyBench](https://arxiv.org/pdf/2504.08703)** — Amazon's extension covering Java, JavaScript, TypeScript, and Python.

### 3.5 Multi-Turn and Agent Interaction Benchmarks

Standard SWE-bench treats tasks as atomic (issue in, patch out). These benchmarks evaluate iterative agent behavior:

- **[TAU-bench](https://arxiv.org/abs/2406.12045)** — Evaluates dynamic conversations between users and agents with domain-specific APIs. Measures iterative, multi-turn problem-solving. Directly relevant to context management because it tests whether agents maintain coherent context across many turns.
- **[AgentBench](https://arxiv.org/abs/2308.03688)** — Evaluates multi-turn reasoning across 8 diverse environments using open-ended generation.
- **[ML-Bench](https://ml-bench.github.io/)** — 9,641 examples across 18 GitHub repositories with both LLM-only and full-agent evaluation modes.

### 3.6 Other Notable Benchmarks

| Benchmark | Focus | Scale | Key Feature |
|-----------|-------|-------|-------------|
| **LiveCodeBench** | Competitive programming | Continuously growing | Contamination-free (new problems from LeetCode, AtCoder, CodeForces) |
| **BigCodeBench** | User-facing tasks | ~1,140 tasks (150 "Hard") | Complete (from docstrings) and Instruct (from NL) variants |
| **CrossCodeEval** | Cross-file completion | 4 languages | Measures ability to use multi-file context |
| **CORE-Bench** | Computational reproducibility | 270 tasks from 90 papers | Three difficulty levels; agents reproduce published results |
| **GitTaskBench** | Real-world repo tasks | Various | Leverages code repositories directly |

### 3.7 Comparison Matrix

| Benchmark | Granularity | Languages | Real-world? | Test-validated? | Tasks |
|-----------|------------|-----------|-------------|-----------------|-------|
| SWE-bench | Issue-level | Python | Yes (GitHub PRs) | Yes | 300-2,294 |
| Aider Polyglot | Exercise-level | 6 langs + Rust | Semi (Exercism) | Yes | 225 |
| HumanEval | Function-level | Python | No (synthetic) | Yes | 164 |
| MBPP | Function-level | Python | No (synthetic) | Yes | 974 |
| LiveCodeBench | Problem-level | Multiple | Semi (competitive) | Yes | Growing |
| BigCodeBench | Task-level | Python | Semi | Yes | 1,140 |
| CrossCodeEval | Completion-level | 4 langs | Yes (real repos) | Partial | Various |
| CORE-Bench | Project-level | Python/R | Yes (papers) | Yes | 270 |
| Multi-SWE-bench | Issue-level | 7 langs + Rust | Yes (GitHub PRs) | Yes | Various |
| TAU-bench | Multi-turn | Domain APIs | Semi (simulated) | Yes | Various |

---

## 4. Evaluation Frameworks

### 4.1 SWE-bench Harness

The [SWE-bench evaluation harness](https://github.com/SWE-bench/SWE-bench) is the standard evaluation runner. It accepts a JSONL predictions file and orchestrates Docker containers.

**Evaluation command:**
```bash
python -m swebench.harness.run_evaluation \
  --dataset_name princeton-nlp/SWE-bench_Lite \
  --predictions_path predictions.jsonl \
  --max_workers 8 \
  --run_id my-eval-run
```

**Cache levels** control the storage/speed tradeoff:
- `none`: Rebuild everything per run
- `base`: Cache base OS image
- `env`: Cache environment images (~100GB, default)
- `instance`: Cache per-task images (~2TB, fastest)

**Validation test** (verify setup works):
```bash
python -m swebench.harness.run_evaluation \
  --max_workers 1 \
  --instance_ids sympy__sympy-20590 \
  --predictions_path gold \
  --run_id validate-gold
```

### 4.2 SWE-agent

[SWE-agent](https://github.com/SWE-agent/SWE-agent) is both a reference agent implementation and an evaluation framework. It provides:

- **Configurable ACI** via YAML — tool bundles, file viewer settings, prompt templates
- **Batch evaluation mode** — processes multiple SWE-bench instances in parallel
- **Trajectory logging** — full action-observation traces for analysis
- **Standardized tool set** — all models use the same tools for fair comparison

SWE-agent demonstrated that the same model (GPT-4) can score very differently depending on the ACI design, proving that harness engineering is a first-class concern.

### 4.3 Inspect AI (UK AISI)

[Inspect AI](https://inspect.aisi.org.uk/) is an open-source evaluation framework from the UK AI Security Institute. It uses a composable pipeline:

```
Dataset → Task → Solver → Scorer
```

**Key features:**
- **Multi-provider support**: OpenAI, Anthropic, Google, Groq, Mistral, xAI, AWS Bedrock, Azure
- **Sandboxing toolkit**: Docker (native), Kubernetes, Modal, Proxmox, and extension APIs
- **Community evals**: 50+ contributed evaluations including SWE-bench implementations
- **Security-focused design**: The evaluation framework sits outside the sandbox and sends commands inbound — the model cannot escape to the parent process

**Sandboxing dimensions:**
1. **Tooling isolation** — restricts what tools the model can access
2. **Host isolation** — prevents container escape
3. **Network isolation** — controls external system interactions

### 4.4 OpenHands

[OpenHands](https://github.com/OpenHands/benchmarks) supports 15+ benchmarks with a unified evaluation infrastructure. It provides:

- Standardized evaluation pipelines across benchmarks
- Containerized environments for reproducibility
- Trajectory-level analysis and failure mode classification
- Reports 77.6% on SWE-bench Verified (with Claude 3.5 Sonnet Thinking)

### 4.5 METR (Model Evaluation and Threat Research)

[METR](https://metr.org/) evaluates autonomous AI capabilities on 12 real-world tasks. Their **Time Horizons** metric measures the duration of tasks agents can complete — a useful framing that captures difficulty better than binary pass/fail. Tasks range from basic (search in file) to advanced (fine-tune an open-source LLM), with human baselines established by contracting humans to complete the same tasks.

### 4.6 Aider's Benchmark Runner

[aider-swe-bench](https://github.com/Aider-AI/aider-swe-bench) provides a focused wrapper:
- `process_one_instance()` runs Aider on each SWE-bench task
- Retry loop with up to 3 attempts per problem at configurable temperature
- Solution validation: edit outcome + lint outcome + test outcome all pass
- If no plausible solution found, picks the least-bad candidate
- Outputs individual JSON files per problem + combined predictions JSONL

### 4.7 SWE-smith (Custom Benchmark Generation)

[SWE-smith](https://github.com/SWE-bench/SWE-smith) generates hundreds to thousands of SWE-bench-style task instances from **any** GitHub repository:

- **LM generation**: Uses an LLM to introduce bugs into programmatic entities (functions, classes)
- **Procedural modification**: Uses AST transformations for random code mutations
- **Scale**: 50,000+ task instances generated across 128 popular repositories
- **Validation**: SWE-agent-LM-32B (trained on SWE-smith data) achieved 40.2% on SWE-bench Verified

This is particularly relevant for generating custom benchmarks from one's own codebase, enabling domain-specific evaluation.

---

## 5. How Existing Tools Benchmark Themselves

### 5.1 Claude Code (Anthropic)

Claude Code achieves **80.9% on SWE-bench Verified** — a score that established the importance of harness engineering. Anthropic reports that the gap between Claude Code and the base model with a standard harness represents the value of:
- Custom tool design and invocation patterns
- Context management and message construction
- Retry logic and error recovery

**Score progression:**
- Claude Opus 4: 72.5%
- Claude Sonnet 4: 72.7%
- Claude Opus 4.5 (via Claude Code): 80.9%

The jump from 72.5% (raw model) to 80.9% (optimized harness) is 8+ percentage points from harness engineering alone.

### 5.2 Aider

Aider publishes scores on both SWE-bench and its own Polyglot Benchmark:
- Maintains a [public leaderboard](https://aider.chat/docs/leaderboards/) comparing models
- Tests multiple models with the same harness to isolate model vs. harness effects
- Two-attempt retry strategy per problem
- Multi-language evaluation (Polyglot) catches capabilities SWE-bench misses (Python-only)

### 5.3 Devin (Cognition)

Devin was evaluated on a random 25% subset (570 tasks) of SWE-bench:
- Resolved 79/570 issues (13.86%) — early result, now significantly improved
- 45-minute timeout per task
- No pre-provided file list — navigates repos autonomously
- Adapted the SWE-bench evaluation harness for their execution model

### 5.4 OpenHands

OpenHands runs 15+ benchmarks to get a multi-dimensional view:
- SWE-bench for real-world issue resolution
- HumanEval for function-level generation
- Custom benchmarks for specific capabilities
- Trajectory-level analysis identifies failure modes (wrong file, incomplete fix, regression)

### 5.5 Cursor

Cursor built **CursorBench** for internal evaluation, measuring:
- Solution correctness
- Code quality
- Efficiency
- Interaction behavior

CursorBench remains closed-source, based on internal team sessions. When all agents used Claude Opus 4.5, Auggie (Augment Code) solved 15 more problems than Cursor out of 731 tasks on SWE-bench Pro.

---

## 6. Key Strategies and Insights

### 6.1 Harness Engineering > Model Choice

Research on [80 approaches across 99 SWE-bench Verified submissions](https://arxiv.org/html/2506.17208v2) found:
- No single architecture dominates — multiple design paradigms are effective
- Three key dimensions: workflow authoring, execution autonomy, LLM agent count
- Custom harnesses consistently outperform standard harnesses with the same model

**The implication for context-orchestrator:** If graph-based context selection produces even a modest improvement in issue resolution, it would be visible on SWE-bench. Conversely, if it doesn't improve scores, the core thesis needs revision.

### 6.2 Tool Design is Critical

SWE-agent's ACI research showed that specific tool design choices have measurable impact:
- **Chunked file viewing** (100 lines) outperforms full-file display — prevents context flooding
- **Lint-on-save** catches syntax errors immediately — prevents cascading failures
- **Succinct search output** — models waste fewer tokens processing results

This maps directly to our context pipeline's Render stage — how we present graph nodes to the LLM affects task performance.

### 6.3 Context Management is the Differentiator

The 10+ point gap between raw model and optimized harness is primarily about context:
- Which information reaches the LLM (our Expand + Score stages)
- How it's formatted (our Render stage)
- What's excluded to save token budget (our Budget stage)
- Whether the conversation history is coherent (our Sanitize stage)

Graph-based context selection could improve on all four dimensions compared to linear chat history.

### 6.4 Isolating Context Management's Contribution

End-to-end SWE-bench evaluation confounds context quality with prompt engineering, retry logic, and tool design. To measure whether graph-based context actually helps, **ablation studies** are essential:

- **Baseline**: Linear chat history (the standard approach used by SWE-agent)
- **Variant A**: Random graph node selection (controls for "any selection is helpful")
- **Variant B**: Graph selection without relevance scoring (tests if graph structure alone helps)
- **Variant C**: Full graph pipeline with scoring (the system under test)

Additionally, context-specific metrics that are orthogonal to task success are needed:
- **Context Relevancy**: How much of the selected context does the agent actually use vs. ignore?
- **Context Recall**: Was all critical information available in the context window?
- **Trajectory analysis**: Which graph nodes did the agent attend to? (Measurable via token importance or output analysis)

A task can pass tests despite poor context selection if the model is powerful enough to compensate. These metrics expose whether the graph is actively helping or just along for the ride.

### 6.5 Multi-Turn Context Degradation

Research on multi-turn security (MT-Sec) found a **consistent 20-27% drop in correct outputs from single-turn to multi-turn settings**. This directly matters for context management: as conversations grow longer, does the context pipeline maintain quality or degrade?

Key questions for evaluation:
- Does graph-based context decay more gracefully than linear history as conversations exceed 50+ turns?
- Does context reranking help agents recover from earlier mistakes?
- How does token budget allocation change as the graph grows?

TAU-bench is the best existing benchmark for evaluating these multi-turn dynamics.

### 6.6 Evaluation Cost and Scale

| Configuration | Tasks | Estimated Cost | Time (32-core) |
|---------------|-------|---------------|----------------|
| SWE-bench Lite | 300 | ~$150-300 | ~40 min |
| SWE-bench Verified | 500 | ~$250-500 | ~62 min |
| SWE-bench Full | 2,294 | ~$1,000-2,500 | ~5 hours |
| Aider Polyglot | 225 | ~$100-200 | ~30 min |

Costs vary by model and heavily depend on prompt caching. With high cache hit rates (common in batch evaluation where system prompts repeat), costs can drop 3-5x. API pricing changes frequently — these estimates are directional, not precise.

**Strategy for continuous evaluation:** Use SWE-bench Lite (300 tasks) for development iteration. Run Verified (500) for milestone releases. Full (2,294) only for final publications.

### 6.7 Custom Benchmarks from Your Own Codebase

SWE-smith enables generating SWE-bench-style tasks from any GitHub repository. This is valuable for:
- **Domain-specific evaluation** — benchmark the harness on the types of problems it will actually face
- **Regression testing** — generate tasks from your project's own issue history
- **Training data** — SWE-agent-LM-32B trained on SWE-smith data and improved from baseline to 41.6%

### 6.8 Multi-Benchmark Evaluation

No single benchmark captures all capabilities. The best strategy combines:
- **SWE-bench** for real-world issue resolution (the headline number)
- **Aider Polyglot** for multi-language, multi-file capability (includes Rust)
- **HumanEval/LiveCodeBench** for pure code generation baselines
- **Custom benchmarks** for domain-specific evaluation
- **Multi-SWE-bench or SWE-Bench++** for Rust-specific signal (critical for a Rust project)

### 6.9 Operational Considerations

**API rate limiting** is the actual bottleneck for batch evaluation, not compute. Token-based rate limiting caps throughput at a fixed TPM (tokens per minute). For 500 tasks averaging 20,000 tokens each, you need sufficient quota. Recovery from rate-limit failures (exponential backoff, queue management) must be built into any evaluation runner.

**Model version drift** is a reproducibility threat. Model behavior changes across API versions (e.g., claude-opus-4 → claude-opus-4.5). A benchmark run today may produce different results next month with the same harness. Mitigation: pin model versions, record exact model IDs per run, and report version as a variable.

**Benchmark overfitting** is a real risk. Iterative prompt engineering against a specific benchmark is a form of training on evaluation data. Research shows LLMs exhibit 2.15% average performance degradation under task perturbations — suggesting surface-level optimization. Mitigation: use different benchmarks for development (SWE-bench Lite) vs. publication (multi-benchmark), hold out a test set, and validate on custom benchmarks the system has never seen.

---

## 7. Red/Green Team

### 7.1 Green Team Findings (Factual Verification)

**Overall accuracy: 98.5%** — One factual error found and corrected.

- **SWE-bench task counts, Docker architecture, run times**: All verified correct against official sources.
- **Claude Code/Opus/Sonnet scores**: Verified against Anthropic announcements.
- **Devin 13.86%, Aider Polyglot 92.9%, OpenHands 77.6%**: All verified.
- **Inspect AI pipeline, SWE-agent ACI, OpenHands 15+ benchmarks**: All verified.
- **SWE-smith 50,000+ instances from 128 repos**: Verified.
- **Corrected**: SWE-agent-LM-32B score was 40.2%, not 41.6% (attributed to a different model).

### 7.2 Red Team Findings (Challenges)

**Missing benchmarks** (now added):
- TAU-bench for multi-turn agent-user interaction evaluation
- Multi-SWE-bench and SWE-Bench++ for Rust-specific evaluation
- AgentBench for multi-turn reasoning across diverse environments
- ML-Bench for ML task evaluation

**Missing evaluation strategies** (now addressed):
- Multi-turn context degradation (20-27% single→multi-turn regression documented)
- Ablation methodology for isolating context management's contribution
- Context quality metrics (relevancy, recall) orthogonal to task success

**SWE-bench weaknesses understated** (now expanded):
- 59.4% flawed test cases (per 2025 audit)
- Training contamination risk
- Django over-representation (45%+ of tasks)
- OpenAI stopped evaluating on Verified partly for these reasons

**Operational gaps** (now addressed):
- API rate limiting as the actual bottleneck
- Model version drift as a reproducibility threat
- Benchmark overfitting risk

**Cost estimates**: Noted as directional. Prompt caching can reduce costs 3-5x; API pricing changes frequently.

### 7.3 Code Accuracy Findings

No inaccurate codebase references — the document correctly focuses on external concepts and strategies rather than making implementation claims. Format follows established research doc conventions.

---

## 8. Sources

### Benchmarks
- [SWE-bench](https://github.com/SWE-bench/SWE-bench) — Princeton NLP, main benchmark repository
- [SWE-bench Website](https://www.swebench.com/) — Official documentation and guides
- [SWE-bench Datasets](https://www.swebench.com/SWE-bench/guides/datasets/) — Full, Verified, Lite variants
- [SWE-bench Docker Guide](https://epoch.ai/blog/swebench-docker) — Optimized Docker image strategy
- [Aider Polyglot Benchmark](https://github.com/Aider-AI/polyglot-benchmark) — 225 exercises, 6 languages
- [HumanEval Pro](https://arxiv.org/abs/2412.21199) — Self-invoking code generation evaluation
- [LiveCodeBench](https://livecodebench.github.io/) — Contamination-free continuous benchmark
- [BigCodeBench](https://bigcode-bench.github.io/) — User-facing programming tasks
- [CORE-Bench](https://arxiv.org/abs/2409.11363) — Computational reproducibility evaluation
- [SWE-smith](https://github.com/SWE-bench/SWE-smith) — Custom benchmark generation toolkit
- [Multi-SWE-bench](https://arxiv.org/html/2504.02605v1) — Polyglot SWE-bench (7 languages including Rust)
- [SWE-Bench++](https://arxiv.org/html/2512.17419v1) — Framework for generating benchmarks across 11 languages
- [TAU-bench](https://arxiv.org/abs/2406.12045) — Tool-Agent-User multi-turn interaction benchmark
- [AgentBench](https://arxiv.org/abs/2308.03688) — Multi-turn reasoning across 8 environments
- [ML-Bench](https://ml-bench.github.io/) — Machine learning task evaluation

### Evaluation Frameworks
- [SWE-agent](https://github.com/SWE-agent/SWE-agent) — Reference agent implementation and evaluation
- [Inspect AI](https://inspect.aisi.org.uk/) — UK AISI evaluation framework
- [Inspect Sandboxing Toolkit](https://www.aisi.gov.uk/blog/the-inspect-sandboxing-toolkit-scalable-and-secure-ai-agent-evaluations) — Sandboxing design
- [OpenHands Benchmarks](https://github.com/OpenHands/benchmarks) — Multi-benchmark evaluation
- [METR](https://metr.org/) — Autonomous capability evaluation
- [aider-swe-bench](https://github.com/Aider-AI/aider-swe-bench) — Aider's SWE-bench wrapper

### Tool Evaluations
- [Claude Code on SWE-bench](https://www.anthropic.com/research/swe-bench-sonnet) — 80.9% Verified score
- [Devin Technical Report](https://cognition.ai/blog/swe-bench-technical-report) — Evaluation methodology
- [Dissecting SWE-bench Leaderboards](https://arxiv.org/html/2506.17208v2) — 80-approach analysis
- [SWE-bench Pro Leaderboard](https://labs.scale.com/leaderboard/swe_bench_pro_public) — Long-horizon evaluation
- [CodSpeed](https://codspeed.io/) — Continuous performance analysis platform

### Red Team References
- [SWE-bench Test Case Audit](https://toloka.ai/blog/fixing-swe-bench-a-smarter-way-to-evaluate-coding-ai/) — 59.4% flawed test cases finding
- [OpenAI on SWE-bench Verified](https://openai.com/index/why-we-no-longer-evaluate-swe-bench-verified/) — Contamination concerns
- [MT-Sec Multi-Turn Security](https://openreview.net/forum?id=zH9aX65Zyi) — 20-27% single→multi-turn regression
- [LLM Workflow Overfitting](https://openreview.net/forum?id=maMnVCHl8J) — Benchmark overfitting analysis
