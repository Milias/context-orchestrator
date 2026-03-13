# 16 — LLM File Editing Strategies

> **2026-03-13** — Comprehensive analysis of every known approach to file editing with LLMs, with accuracy data, tool comparisons, and recommendations for context-orchestrator's write_file/edit implementation.

---

## 1. Executive Summary

File editing is the highest-stakes operation an LLM coding tool performs — every other capability (chat, search, analysis) is read-only. The industry has converged on ~14 distinct approaches ranging from whole-file rewrites to semantic AST-based merging, with accuracy spanning 20% to 98% depending on format and model. The dominant insight: **edit format choice causes a 3X swing in success rate with the same model**. Claude models lead real-world benchmarks (EDIT-Bench: only claude-sonnet-4 exceeds 60% at 64.8% pass@1), but even top models fail 35%+ of the time without proper validation loops. We recommend a phased approach: Phase 1 uses Claude's native `str_replace`-style tool with tree-sitter validation; Phase 2 adds an architect/editor split for complex multi-file changes; Phase 3 integrates a specialized apply model (Morph-style) for speed-critical paths.

---

## 2. Current Architecture & Gap Analysis

### What Exists Today

The context-orchestrator has implemented foundational tool call infrastructure:

- **Tool call graph nodes**: `ToolCall` and `ToolResult` as first-class graph citizens with `Invoked`/`Produced` edges (`docs/design/03-tool-call-foundation.md`)
- **`read_file` executor**: Real `tokio::fs::read_to_string` with 100KB truncation and char-boundary safety
- **Closed-enum arguments**: `ToolCallArguments` enum with `ReadFile { path }` and `Unknown { tool_name, raw_json }` variants — no `serde_json::Value`
- **SSE streaming parser**: Accumulates partial JSON across `input_json_delta` events, emits `StreamChunk::ToolUse`
- **Agent loop**: Stream → record tool uses → create assistant node → spawn executors → wait for results (60s timeout) → loop
- **Path validation**: Canonicalize + `starts_with(cwd)` to prevent directory traversal

### What's Missing

| Gap | Impact | Relevant Vision Section |
|-----|--------|------------------------|
| No `write_file` / `edit_file` executor | Cannot modify code — the core use case | 4.8 |
| No edit validation (syntax, lint, types) | Silent corruption of files | — |
| No retry/self-correction loop | Single-shot edits fail 35%+ of the time | — |
| No edit format strategy | Ad-hoc implementation risks low accuracy | — |
| No multi-file coordination | Cannot make atomic cross-file changes | — |
| No undo/rollback mechanism | Destructive edits are unrecoverable | — |
| No edit compaction for graph | Tool results for edits balloon token usage | 4.2, 4.3 |

---

## 3. Requirements

Derived from VISION.md, existing architecture, and research findings:

1. **Accuracy**: Edit success rate ≥90% on first attempt for single-file changes
2. **Safety**: Edits must be validated before persisting; rollback must be possible
3. **Provenance**: Every edit is a ToolCall→ToolResult chain in the graph
4. **Token efficiency**: Edit format must minimize output tokens (cost scales with output)
5. **Streaming**: Edits should be displayable as they stream in
6. **Multi-file**: Support atomic changes across 2+ files
7. **Validation**: At minimum syntax checking; ideally lint + type check
8. **Compaction**: Edit history must be compactable for graph processing
9. **Model-agnostic**: Edit format must work across Claude, DeepSeek, local models
10. **No `serde_json::Value`**: All edit arguments must be typed structs

---

## 4. Options Analysis

### Option A: Whole-File Rewrite

The LLM outputs the complete updated file content.

**How it works**: LLM receives file content, generates entire file including unchanged lines.

**Strengths:**
- Simplest implementation (replace file contents)
- No diff parsing required
- Works with any model
- No match failure modes

**Weaknesses:**
- High token cost (entire file for a one-line change)
- LLM can silently introduce unintended changes in unmodified sections
- Doesn't scale to files >400 lines (accuracy degrades, token cost explodes)
- No streaming benefit (must wait for full output)

**Accuracy**: 60-85% depending on file size and model

**Used by**: Cursor (files <400 lines), Aider (fallback `whole` format)

---

### Option B: Search/Replace Blocks

LLM provides exact text to find and exact replacement text.

**How it works**: LLM outputs `old_string`/`new_string` pairs. Tool finds exact match in file, replaces it.

**Format** (Claude Code style):
```json
{
  "file_path": "/path/to/file.rs",
  "old_string": "fn old_name() {\n    todo!()\n}",
  "new_string": "fn new_name() -> Result<()> {\n    Ok(())\n}"
}
```

**Strengths:**
- Low token cost (only changed sections)
- Exact match semantics — either works perfectly or fails explicitly
- No ambiguous partial application
- Easy to validate (did the match exist? is the result syntactically valid?)
- Streams well (can display intent before applying)

**Weaknesses:**
- Requires exact string match including whitespace/indentation
- Fails if target string appears multiple times (ambiguity)
- LLMs sometimes hallucinate the search block (doesn't match actual file)
- 64.9% task correctness for Claude 3.7 Sonnet (97.8% format compliance)

**Accuracy**: 65-90% depending on model and file familiarity

**Used by**: Claude Code (primary), Aider (`diff` format), Roo Code

---

### Option C: Unified Diff Format

Standard GNU unified diff with context lines and hunk headers.

**How it works**: LLM generates diff output with `---`/`+++` headers and `@@` hunk markers.

**Strengths:**
- Standard format, widely understood
- Good for complex multi-location changes
- Higher accuracy than search/replace for some models (59% vs 20% baseline for GPT-4 Turbo)
- Reduces "lazy" edits (4/89 vs 12/89 tasks compared to search/replace)

**Weaknesses:**
- LLMs frequently generate incorrect hunk header line numbers
- Requires sophisticated parsing that ignores line numbers and uses context matching
- Out-of-distribution for most models (trained on more whole-files than diffs)
- 59-61% accuracy even after optimization

**Accuracy**: 59-61% (optimized), 20% (baseline)

**Used by**: Aider (`udiff` format), OpenHands

---

### Option D: Line-Numbered Edits

Edits reference specific line ranges.

**How it works**: LLM specifies line range and replacement content.

**Strengths:**
- Explicit location specification
- Non-sequential (can reference any line)
- Stream-friendly

**Weaknesses:**
- LLMs get line numbers wrong on evolved codebases
- Line numbers shift after previous edits in same session
- Limited production data on accuracy
- Not robust to minor file changes

**Accuracy**: Limited data, theoretical improvement over unified diff

**Used by**: ln-diff (experimental)

---

### Option E: Architect/Editor Split (Two-Model)

Separate the "what to change" reasoning from the "how to format the edit" execution.

**How it works**:
1. Architect model (strong, expensive) describes changes in natural language
2. Editor model (weaker, cheaper, or same model with different prompt) translates to specific file edits

**Strengths:**
- Architect focuses on problem-solving without format constraints
- Editor focuses on correct formatting without reasoning burden
- Can use expensive model for reasoning, cheap model for formatting
- Higher accuracy than single-model approaches

**Weaknesses:**
- Two LLM calls per edit (latency and cost)
- Communication format between architect and editor must be well-defined
- Editor can misinterpret architect's intent
- More complex orchestration

**Accuracy**: Not independently benchmarked, but Aider reports improved results

**Used by**: Aider (architect mode), Cursor (implicitly with Apply model)

---

### Option F: Specialized Apply Model

A small, fine-tuned model that takes (original file + edit description/sketch) and outputs the merged result.

**How it works**:
1. Primary LLM generates edit intent/sketch (possibly with "existing code" markers)
2. Specialized 7B model takes original + sketch, outputs merged file
3. Semantic understanding, not string matching

**Strengths:**
- **98% accuracy** (Morph's benchmark)
- **10,500 tokens/second** (vs ~100 tok/s for general models)
- Handles edge cases (imports, structure preservation) automatically
- 50-60% token usage reduction, 90%+ latency improvement

**Weaknesses:**
- Requires running a separate model (infrastructure complexity)
- Proprietary implementations (Morph) — open alternatives less proven
- Additional inference cost ($0.80/M input tokens for Morph)
- Dependency on external service or local GPU

**Accuracy**: 98% (Morph benchmark)

**Used by**: Cursor (Fast Apply), Morph

---

### Option G: AST-Based Semantic Edits

Edits operate on the Abstract Syntax Tree rather than raw text.

**How it works**: Parse file to AST via tree-sitter, LLM operates on/describes structural changes, changes mapped back through AST.

**Strengths:**
- Understands code structure, not just text
- Resilient to formatting variations
- Enables symbol-aware refactoring (rename, extract, inline)
- Can enforce structural invariants

**Weaknesses:**
- Requires language-specific parsers (tree-sitter covers many, but not all)
- LLMs don't natively output AST operations
- Translation layer between natural language intent and AST operations is complex
- Limited adoption in production tools

**Accuracy**: Theoretically high, limited production data

**Used by**: Polyglot-LS (experimental), Windsurf (partially)

---

## 5. Comparison Matrix

| Criterion | Whole File | Search/Replace | Unified Diff | Line-Numbered | Architect/Editor | Apply Model | AST-Based |
|-----------|-----------|---------------|-------------|--------------|-----------------|------------|----------|
| **Accuracy** | 60-85% | 65-90% | 59-61% | Unknown | Higher* | **98%** | High* |
| **Token cost** | Very High | **Low** | **Low** | **Low** | Medium (2 calls) | Medium | Low |
| **Implementation** | Trivial | Simple | Complex | Simple | Medium | Complex | Very Complex |
| **Streaming** | Poor | Good | Fair | Good | N/A | Good | Poor |
| **Multi-file** | Works | Works | Works | Fragile | Natural | Works | Complex |
| **Validation** | Hard | Easy | Medium | Medium | Easy | Easy | Built-in |
| **Model-agnostic** | Yes | Yes | Partial | Partial | Yes | No | Yes |
| **Rollback** | Easy (keep old) | Easy | Medium | Medium | Easy | Easy | Medium |
| **File size limit** | ~400 lines | None | None | None | None | ~1000 lines | None |
| **Production proven** | Yes | **Yes** | Yes | No | Yes | Yes | No |

\* Limited production data

---

## 6. VISION.md Alignment

| Vision Concept | Relevance to File Editing |
|---------------|--------------------------|
| **Graph-native context** (§3.1) | Every edit is a `ToolCall` → `ToolResult` chain. Edit diffs stored as node properties. |
| **Multi-perspective compaction** (§4.2) | Edit history compacts differently for "what changed" vs "why it changed" vs "what broke." |
| **Background processing** (§4.3) | Post-edit validation (lint, typecheck) runs as background tasks. Edit compaction is async. |
| **Tool calls as citizens** (§4.8) | Already implemented. Edit tools extend the existing `ToolCallArguments` enum. |
| **Deterministic construction** (§3.2) | Edit provenance enables replay: given same graph state, same edits can be re-applied. |

**Key alignment**: The graph model naturally supports edit provenance, rollback (walk back to pre-edit state), and compaction (compress "5 edits to auth.rs" into "rewrote authentication module"). The search/replace format maps cleanly to graph nodes with typed arguments.

---

## 7. Recommended Architecture

### Phase 1: Search/Replace with Validation (Immediate)

Implement Claude Code's proven `str_replace` pattern:

```rust
// Extends existing ToolCallArguments enum
pub enum ToolCallArguments {
    ReadFile { path: String },
    EditFile {
        file_path: String,
        old_string: String,
        new_string: String,
        replace_all: bool,
    },
    CreateFile {
        file_path: String,
        content: String,
    },
    // ... existing variants
}
```

**Validation pipeline** (post-edit, before persisting):
1. **Exact match check**: Does `old_string` exist uniquely in the file?
2. **Apply edit** in memory (don't write to disk yet)
3. **Syntax check**: Run tree-sitter incremental parse on result
4. **Write to disk** only if syntax is valid (or user overrides)
5. **Record in graph**: `ToolCall(EditFile)` → `ToolResult(success/failure)`

**Rollback**: Keep pre-edit content in `ToolCall` node properties. Walk graph backward to reconstruct any prior state.

**Retry loop**: On failure (match not found, syntax error), feed error back to LLM with file context. Max 3 attempts (Cursor's proven limit).

**Why this first**: Search/replace is the format Claude is most trained on. It's what Claude Code uses. It has clear success/failure semantics. It maps cleanly to typed structs.

### Phase 2: Architect/Editor Split for Complex Changes

For multi-file changes or large refactors:

1. **Architect phase**: Primary model describes all changes in structured natural language
   ```
   File: src/auth.rs
   - Add `validate_token` method to `AuthService` that checks expiry and signature
   - Update `authenticate` to call `validate_token` instead of inline check

   File: src/routes.rs
   - Update `auth_middleware` to use new `AuthService::authenticate` return type
   ```

2. **Editor phase**: Same or cheaper model converts each description to `EditFile` tool calls

**Why Phase 2**: This is additive — architect output feeds into Phase 1's edit tool. No new edit format needed, just a new orchestration layer. Matches Aider's architect mode pattern.

### Phase 3: Specialized Apply Model (Future)

For speed-critical or high-volume editing:

1. Primary model generates edit sketch with `// ... existing code ...` markers
2. Local or remote apply model (7B, fine-tuned) merges sketch into original
3. Result validated through same Phase 1 pipeline

**Why Phase 3**: Requires running a second model. Only worth it when edit volume or latency requirements justify the infrastructure. Morph's 98% accuracy and 10,500 tok/s make this compelling for production but premature for MVP.

---

## 8. Integration Design

### Tool Registration

```rust
// In tool registry, alongside existing read_file
ToolDefinition {
    name: "edit_file",
    description: "Make a targeted edit to a file by replacing exact text",
    parameters: json!({
        "type": "object",
        "properties": {
            "file_path": {
                "type": "string",
                "description": "Absolute path to the file to edit"
            },
            "old_string": {
                "type": "string",
                "description": "The exact text to find and replace (must be unique in file)"
            },
            "new_string": {
                "type": "string",
                "description": "The replacement text"
            },
            "replace_all": {
                "type": "boolean",
                "description": "Replace all occurrences (default: false)",
                "default": false
            }
        },
        "required": ["file_path", "old_string", "new_string"]
    }),
}

ToolDefinition {
    name: "create_file",
    description: "Create a new file with the given content",
    parameters: json!({
        "type": "object",
        "properties": {
            "file_path": {
                "type": "string",
                "description": "Absolute path for the new file"
            },
            "content": {
                "type": "string",
                "description": "The file content to write"
            }
        },
        "required": ["file_path", "content"]
    }),
}
```

### Execution Flow

```
User message → LLM streams response
  → ToolUse(edit_file) detected
  → Parse into EditFile { file_path, old_string, new_string, replace_all }
  → Create ToolCall node in graph
  → Executor:
      1. Read current file content
      2. Validate old_string exists (unique unless replace_all)
      3. Apply replacement in memory
      4. tree-sitter parse → check for new errors
      5. If valid: write to disk, return success
      6. If invalid: return error with details
  → Create ToolResult node
  → If error: LLM sees error in next iteration, can retry
```

### Graph Representation

```
Message(user: "add error handling to parse_config")
  └── responds_to
Message(assistant: "I'll add error handling...")
  ├── Invoked → ToolCall(read_file, path: "src/config.rs")
  │                └── Produced → ToolResult(content: "...")
  ├── Invoked → ToolCall(edit_file, path: "src/config.rs",
  │               old: "fn parse_config()...", new: "fn parse_config() -> Result<>...")
  │                └── Produced → ToolResult(success, diff_summary: "+3/-1 lines")
  └── Invoked → ToolCall(edit_file, path: "src/config.rs",
                  old: "let config = parse_config();", new: "let config = parse_config()?;")
                   └── Produced → ToolResult(success, diff_summary: "+1/-1 lines")
```

### Edit Compaction (for graph background processing)

Multiple edits to the same file compress to:
- **Light summary**: "3 edits to `src/config.rs`: added error handling to `parse_config`, changed return type to `Result<Config>`, updated 2 call sites"
- **Metadata-only**: "Modified `src/config.rs` (config parsing, error handling)"

### Safety Controls

1. **Path validation**: Same canonicalize + `starts_with(cwd)` as `read_file`
2. **Read-before-write invariant**: `edit_file` must verify file has been read in current context (or reads it automatically)
3. **Backup**: Optional `.bak` file creation before edit (configurable)
4. **Size limit**: Refuse to create files >1MB (configurable)
5. **Binary detection**: Refuse to edit binary files
6. **Permissions check**: Verify write permission before attempting

---

## 9. Red/Green Team

### Green Team: Factual Verification

| # | Claim | Status | Evidence |
|---|-------|--------|----------|
| 1 | EDIT-Bench: claude-sonnet-4 at 64.8% pass@1, only model >60% | VERIFIED | arxiv 2511.04486 confirms both figures |
| 2 | Morph: 98% accuracy, 10,500 tok/s | VERIFIED | morphllm.com official specs |
| 3 | Aider: 20% baseline → 59-61% with unified diff (3X improvement) | VERIFIED | aider.chat/docs/unified-diffs.html (original draft had 26% baseline — corrected to 20%) |
| 4 | SWE-bench Verified: Claude 3.5 Sonnet at 49% | VERIFIED | anthropic.com/research/swe-bench-sonnet |
| 5 | Cursor speculative edits: ~1000 tok/s on 70B models, 13X speedup | VERIFIED | fireworks.ai/blog/cursor |
| 6 | Claude text_editor tool uses str_replace with old_str/new_str | VERIFIED | platform.claude.com docs |
| 7 | Morph pricing: $0.80/M input tokens | VERIFIED | pricepertoken.com |
| 8 | Aider lazy edits: 12/89 with search/replace, 4/89 with unified diff | VERIFIED | aider.chat/docs/unified-diffs.html |
| 9 | AutoPrompter: 27% improvement in edit correctness | VERIFIED | arxiv 2504.20196v1 |
| 10 | Retry limits (Cursor 3-attempt, Aider [0.5,1,2]s backoff) | UNVERIFIABLE | No official documentation found for either specific claim |

### Red Team: Challenge Recommendations

**C1: Search/replace may not be the optimal Phase 1 format.** Unified diff achieves 3X improvement over search/replace baseline for GPT-4 Turbo (20% → 59%). The recommendation favors search/replace partly from familiarity with Claude Code's approach. Counter: Claude models specifically excel at structured tool output (JSON with old_string/new_string), and exact-match semantics give clear success/failure signals. The 65-90% range for search/replace with Claude is higher than unified diff's 59-61%.

**C2: Missing approaches not covered.**
- **Git-native editing**: LLM generates `git diff` output, applied via `git apply` — gives atomic commits with semantic history for free
- **Guided generation with regex constraints**: Constrain LLM output at decode time to match diff format exactly, eliminating hallucinated hunk headers ([Outlines library](https://github.com/noamgat/lm-format-enforcer))
- **Multi-hunk spatial proximity**: Research shows LLMs fail when fixes span disjoint regions (mean hunk divergence 1.60 for failures vs 0.20 for success) — multi-hunk edits should be broken into sequential single-hunk operations ([arxiv 2506.04418](https://arxiv.org/abs/2506.04418))
- **Editor commands (vim-style)**: Compact, TUI-natural, though niche

**C3: Tree-sitter validation is insufficient.** Syntactically valid code can be semantically broken. Tree-sitter with error recovery may even accept invalid code as valid (error nodes still parse). Research shows 75% of erroneous GPT-4 code passes syntax checks but fails semantically. Phase 1 should require at minimum `cargo check`/`tsc`/language-equivalent type checking, not just tree-sitter parse.

**C4: Multi-file atomicity is unaddressed.** Three specific failure modes:
1. *Partial application*: Edit 1 succeeds on file A, edit 2 fails on file B → inconsistent state
2. *Cross-file dependencies*: File B calls method added in file A, but B is edited first
3. *Sequential invalidation*: Edit 1 changes a block, edit 2's search string no longer matches

These need transactional writes or preview-all-before-applying-any semantics. The [SQLite super-journal pattern](https://sqlite.org/atomiccommit.html) is a proven approach.

**C5: The apply model deferral may be wrong.** [Kortix Fast Apply](https://github.com/kortix-ai/fast-apply) is open-source. If the MVP needs ≥90% accuracy on first attempt (Requirement 1), search/replace can't reliably hit that (cited range: 65-90%, upper bound with ideal conditions). The apply model hits 98%. The cost of building Phase 1 retry loops may exceed the cost of integrating an apply model.

**C6: Cost analysis is absent.** The document doesn't quantify:
- Retry cost: At 75% success rate, expect 1.3 attempts per edit (25% of edits need 1+ retry, each consuming 15-55K tokens for re-read + re-attempt)
- Validation wall time: lint + type check adds 2-10s per edit
- Architect/editor cost multiplier: ~1.5-2X for multi-file work (two LLM calls)

**C7: Missing failure modes.**
- Hallucinated file paths (LLM invents paths that don't exist)
- Context window exhaustion mid-retry (file re-read consumes context)
- Indentation/whitespace corruption (2-space vs 4-space, tabs vs spaces)
- Line ending mismatches (CRLF vs LF)
- Concurrent edits (user saves file while LLM is editing)
- Empty file edge cases (search/replace can't match in empty files — need `create_file` or `append_to_file`)

**C8: TUI UX is not discussed.** How does the user approve edits? Auto-apply if validation passes? Batch approval? What does a failed edit look like in the TUI? How are parallel file edits visualized?

### Code Accuracy: Codebase Verification

All 9 claims about the codebase verified as **ACCURATE**:

| Claim | Status | Evidence |
|-------|--------|----------|
| `ToolCallArguments` enum with `ReadFile`, `Unknown` | ACCURATE | `src/graph/tool_types.rs:15-42` |
| `read_file`: `tokio::fs::read_to_string`, 100KB truncation, char-boundary safety | ACCURATE | `src/tool_executor/read_file.rs:5-52` |
| Path validation: canonicalize + `starts_with(cwd)` | ACCURATE | `src/tool_executor/security.rs:10-41` |
| SSE parser: accumulates `input_json_delta`, emits `StreamChunk::ToolUse` | ACCURATE | `src/llm/anthropic.rs:313-336` |
| Agent loop: stream → record → spawn executors → wait (60s timeout) | ACCURATE | `src/app/agent_loop.rs:53-124`, timeout at line 242 |
| `ToolCall` and `ToolResult` as graph node types | ACCURATE | `src/graph/mod.rs:83-149` |
| `Invoked` and `Produced` as edge types | ACCURATE | `src/graph/mod.rs:58-68` |
| 60-second timeout | ACCURATE | `src/app/agent_loop.rs:242` |
| No `serde_json::Value` in struct fields (transient use only) | ACCURATE | `src/graph/tool_types.rs:91-95`, `src/llm/tool_types.rs:147-149` |

---

## 10. Sources

### Benchmarks & Accuracy Data
- [EDIT-Bench: Real-World Instruction-Based Code Editing](https://arxiv.org/abs/2511.04486) — Only claude-sonnet-4 exceeds 60% (64.8% pass@1)
- [CodeEditorBench: 7,961 code editing tasks](https://arxiv.org/html/2404.03543v1)
- [Aider Polyglot Leaderboard: 225 exercises across 6 languages](https://aider.chat/docs/leaderboards/)
- [SWE-bench Verified: Claude 3.5 Sonnet at 49%](https://www.anthropic.com/research/swe-bench-sonnet)
- [Diff-XYZ: Evaluating diff understanding](https://arxiv.org/html/2510.12487v2)

### Edit Format Analysis
- [Aider Edit Formats Documentation](https://aider.chat/docs/more/edit-formats.html)
- [Unified Diffs Make GPT-4 Turbo 3X Less Lazy](https://aider.chat/docs/unified-diffs.html)
- [Morph: AI Code Edit Formats Guide](https://www.morphllm.com/edit-formats)
- [Code Surgery: How AI Assistants Make Precise Edits](https://fabianhertwig.com/blog/coding-assistants-file-edits/)
- [Context Over Line Numbers: Robust LLM Code Diffs](https://medium.com/@surajpotnuru/context-over-line-numbers-a-robust-way-to-apply-llm-code-diffs-eb239e56283f)

### Tool Implementations
- [Claude Text Editor Tool](https://platform.claude.com/docs/en/agents-and-tools/tool-use/text-editor-tool)
- [How Cursor Built Fast Apply](https://fireworks.ai/blog/cursor) — Speculative edits at 1000 tok/s
- [How Cursor AI IDE Works](https://blog.sshh.io/p/how-cursor-ai-ide-works)
- [Aider: Separating Code Reasoning and Editing](https://aider.chat/2024/09/26/architect.html)
- [Morph Fast Apply Model](https://www.morphllm.com/fast-apply-model) — 98% accuracy, 10,500 tok/s
- [Roo Code GitHub](https://github.com/RooCodeInc/Roo-Code)

### Validation & Failure Modes
- [Aider: Linting with Tree-sitter](https://aider.chat/2024/05/22/linting.html)
- [ChatRepair: Automated Program Repair](https://arxiv.org/html/2405.15690v1)
- [LLMLOOP: Feedback Loop Framework](https://valerio-terragni.github.io/assets/pdf/ravi-icsme-2025.pdf)
- [Fixing Function-Level Code Generation Errors](https://arxiv.org/abs/2409.00676)
- [Hallucinations in Code Are the Least Dangerous](https://simonwillison.net/2025/Mar/2/hallucinations-in-code/)
- [Prompting LLMs for Code Editing: Struggles and Remedies](https://arxiv.org/html/2504.20196v1) — AutoPrompter achieves 27% improvement

### Token Efficiency & Cost
- [Morph: LLM Cost Optimization](https://www.morphllm.com/llm-cost-optimization)
- [RFC: Token-Efficient Code Generation Through GNU Unified Diff](https://medium.com/@zackisland/rfc-token-efficient-and-consistent-ai-code-generation-through-gnu-unified-diff-md-a07f676c975a)

### Multi-Turn & Merge
- [LLM Merge Conflict Resolution](https://dl.acm.org/doi/10.1145/3533767.3534396)
- [Why LLMs Fail in Multi-Turn Conversations](https://www.prompthub.us/blog/why-llms-fail-in-multi-turn-conversations-and-how-to-fix-it)

### Multi-Hunk & Atomicity
- [Multi-Hunk Patches: Divergence, Proximity, and LLM Repair Challenges](https://arxiv.org/abs/2506.04418)
- [SagaLLM: Compensation-Based Rollback](https://www.vldb.org/pvldb/vol18/p4874-chang.pdf)
- [SQLite Atomic Commit](https://sqlite.org/atomiccommit.html)

### Semantic Validation
- [SemGuard: Real-Time Semantic Evaluator for LLM Code](https://arxiv.org/html/2509.24507v1)
- [Metamorphic Prompt Testing for LLM Validation](https://arxiv.org/html/2406.06864v1)
- [Towards Formal Verification of LLM-Generated Code](https://arxiv.org/html/2507.13290v1)

### Open-Source Apply Models
- [Kortix Fast Apply (Open Source)](https://github.com/kortix-ai/fast-apply)

### Prior Internal Research
- `docs/research/06-inline-tool-invocation-patterns.md` — Tool invocation spectrum, Cursor's two-model pattern, deferred tool loading
- `docs/research/14-system-and-agent-prompt-design.md` — 4-layer prompt model, error recovery directives, behavioral limits
- `docs/research/15-llm-written-plugins.md` — Rhai/WASM/MCP plugin architecture for dynamic tool generation
- `docs/design/03-tool-call-foundation.md` — ToolCall/ToolResult graph nodes, closed-enum arguments, SSE parsing
- `docs/design/02-background-llm-and-tool-invocation.md` — Shared LLM provider, graph snapshots, tool execution flow
