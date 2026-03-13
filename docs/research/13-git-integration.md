# Full Git Integration

> **Date:** 2026-03-13 | Research into comprehensive git integration: enriching LLM context with diffs/blame/log, git-native agent tools, worktree-based isolation, graph bootstrapping from history, and conversation versioning.

---

## 1. Executive Summary

The context-orchestrator currently uses git in exactly one way: `scan_git_files()` (`src/tasks.rs:209-236`) calls `git2::Repository::statuses()` to populate `GitFile` nodes with path and status (Tracked/Modified/Staged/Untracked). This is a file-status reporter. It knows *which* files changed but not *what* changed, *who* changed them, *when*, or *why*.

Git is the richest source of structured project context available to any development tool. Every commit is a timestamped, attributed, message-annotated snapshot of intent. Every diff is a precise description of what changed. Every branch is an isolation boundary. Every blame line is a provenance chain. None of this reaches the LLM today.

This document surveys eight integration categories, evaluates them against the VISION.md architecture and the existing tool system (Design 03), and proposes a three-phase rollout. The core recommendation: **Phase 1 should inject git diff and branch context into every LLM call** (highest value, lowest complexity), **Phase 2 should add git-native agent tools and history bootstrapping**, and **Phase 3 should add worktree-based agent isolation and graph versioning**.

All proposed work uses the existing `git2 = "0.20"` dependency (`Cargo.toml:21`). No new crates are required for Phases 1-2.

---

## 2. Current Architecture & Gap Analysis

### 2.1 What Exists

The git integration lives in three places:

1. **`spawn_git_watcher()`** (`src/tasks.rs:132-155`): Spawns a `spawn_blocking` task that runs `run_git_watcher()`. Sends `TaskStatusChanged` messages for lifecycle tracking.

2. **`run_git_watcher()`** (`src/tasks.rs:163-207`): Opens the repository via `git2::Repository::open_from_env()`, does an initial `scan_git_files()`, then watches the workdir with `notify-debouncer-mini` (500ms debounce). On any filesystem event, re-scans and sends `TaskMessage::GitFilesUpdated`.

3. **`scan_git_files()`** (`src/tasks.rs:209-236`): Queries `repo.statuses()` with `include_untracked(true)` and `recurse_untracked_dirs(true)`. Maps git2 status flags to `GitFileStatus` variants. Returns `Vec<GitFileSnapshot>` (path + status).

4. **`GitFile` node** (`src/graph/mod.rs:104-109`): Stored in the graph with `id`, `path`, `status`, `updated_at`. Connected to the branch root via `Indexes` edges (created by the task handler when processing `GitFilesUpdated` messages).

5. **`GitFileStatus` enum** (`src/graph/mod.rs:30-35`): Four variants — `Tracked`, `Modified`, `Staged`, `Untracked`.

### 2.2 What's Missing

| Git capability | Status | Value for LLM context |
|---|---|---|
| File status (which files changed) | Implemented | Low — names without content are marginally useful |
| Diff content (what changed in each file) | Missing | **High** — the single most informative context for coding tasks |
| Current branch name | Missing | Medium — signals what the developer is working on |
| Recent commit log | Missing | Medium — shows project momentum and recent decisions |
| Blame annotations | Missing | Medium — shows who wrote code and when |
| Staging operations (add, reset) | Missing | High for agent tools — enables commit workflows |
| Commit creation | Missing | High for agent tools — enables autonomous checkpointing |
| Branch creation/switching | Missing | Medium — enables agent-driven branching workflows |
| Worktree management | Missing | High for multi-agent — isolation primitive |
| Stash operations | Missing | Low — niche use case |
| Merge/rebase | Missing | Low — too dangerous for autonomous agents in Phase 1 |

### 2.3 The `git2` API Surface

The project already depends on `git2 = "0.20"` (libgit2 bindings). The crate provides everything needed without additional dependencies:

- `Repository::diff_index_to_workdir()` — unstaged changes
- `Repository::diff_tree_to_index()` — staged changes
- `Repository::diff_tree_to_workdir_with_index()` — all uncommitted changes
- `Diff::print()` / `Diff::stats()` — unified diff output and statistics
- `Repository::revwalk()` — commit log traversal
- `Repository::blame_file()` — per-line authorship
- `Repository::head()` — current branch/ref
- `Repository::index()` — staging area manipulation
- `Signature::now()` — commit creation
- `Repository::worktree()` / `Worktree::open_from_repository()` — worktree management

---

## 3. Requirements

Derived from VISION.md and the existing architecture:

| Requirement | Source | Description |
|---|---|---|
| R1: Git context in LLM calls | VISION.md §3.2 (context construction) | The context builder should include git-derived information when relevant |
| R2: Git data as graph nodes | VISION.md §3.1 (node types), §4.8 (tool provenance) | Git artifacts (commits, diffs) should be first-class graph nodes with typed edges |
| R3: Git-native agent tools | Design 03 (`ToolCallArguments` enum) | The agent should be able to perform git operations via the tool system |
| R4: History bootstrapping | VISION.md §6.1 (Git History Crawl) | Cold start by populating the graph from commit history |
| R5: Agent isolation | VISION.md §5.1 (Rust/safety), Doc 05 (Gas Town worktrees) | Parallel agents need isolated working directories |
| R6: No new heavy dependencies | CLAUDE.md (minimal deps), Cargo.toml | Use existing `git2` crate; avoid adding `gix` or other git libraries |
| R7: Background processing | VISION.md §4.3 (MergeTree) | Git operations that don't need user interaction run asynchronously |
| R8: Source provenance | Doc 11 (multi-source input) | All git-derived nodes carry provenance metadata |

---

## 4. Options Analysis

### 4.A — Git Diff as Context

**Description.** Inject the output of `git diff` (uncommitted changes) into the LLM context as part of every conversation turn. This gives the model a precise understanding of what the developer is currently working on — far more informative than a list of modified file names.

**Implementation.**

```rust
// New function in src/tasks.rs or a new src/git/ module
fn get_uncommitted_diff(repo: &Repository) -> anyhow::Result<String> {
    let head_tree = repo.head()?.peel_to_tree()?;
    let diff = repo.diff_tree_to_workdir_with_index(Some(&head_tree), None)?;
    let mut output = Vec::new();
    diff.print(git2::DiffFormat::Patch, |_delta, _hunk, line| {
        output.extend_from_slice(line.content());
        true
    })?;
    String::from_utf8(output).map_err(Into::into)
}
```

**Context injection.** The diff would be injected as a `SystemDirective`-like prefix in the context builder (`src/app/context.rs`), or as a dedicated `GitDiff` node type attached to the branch root.

**Token budget strategy:** A single-file change is typically 100-300 tokens; a 5-file PR is 500-1,000 tokens; an active refactor across 15 files can reach 3,000-8,000 tokens; a code formatter run can produce tens of thousands. The injection strategy must handle all cases:
- **Under 2,000 tokens:** Include the full unified diff.
- **Over 2,000 tokens:** Include full diffs only for files mentioned in recent messages. Show 1-line stats (`foo.rs: +45 -12`) for other files.
- **Over 5,000 tokens even after filtering:** Show diff stats only (no patch content). The agent can use the `git_diff` tool (Phase 2) to fetch specific files on demand.

**Strengths:**
- Highest-value, lowest-complexity integration — immediate improvement to every LLM response
- Uses only `git2` APIs already available
- No new node types required (can use existing `SystemDirective` or a new `GitContext` section in context builder)

**Weaknesses:**
- Large diffs can consume significant context budget
- Binary file diffs are noise
- Needs truncation strategy for monorepo-scale changes

**Crate:** `git2 = "0.20"` (existing)

### 4.B — Git Log/Blame as Graph Nodes

**Description.** Represent commits as graph nodes with metadata (author, date, message, files changed). Blame annotations connect code lines to the commits that last touched them. This enables the LLM to answer "who wrote this?", "when did this change?", and "what was the intent behind this code?"

**Implementation.** New node type `Commit` with fields: `oid: String`, `author: String`, `message: String`, `timestamp: DateTime<Utc>`, `files_changed: Vec<String>`. Edges: `Commit --[Modifies]--> GitFile`. Blame data stored as metadata on `GitFile` nodes or as a separate `BlameAnnotation` type.

**Strengths:**
- Enables project history as LLM context
- Blame data is highly relevant for code understanding ("this function was last changed 2 days ago by Alice in commit 'fix auth timeout'")
- Commit messages capture developer intent — exactly what LLMs need

**Weaknesses:**
- Full history crawl can produce thousands of nodes (100 commits = 100 nodes)
- Blame is per-file and O(n) in file length — expensive for large files
- Node count explosion requires aggressive compaction strategy
- Requires new node types and edge kinds in `src/graph/mod.rs`

**Crate:** `git2 = "0.20"` (existing). `revwalk()` for log, `blame_file()` for blame.

### 4.C — Git-Native Agent Tools

**Description.** Add git operations to the `ToolCallArguments` enum (`src/graph/tool_types.rs:15`), allowing the agent to perform git operations through the existing tool dispatch system (Design 03).

**Proposed tools:**

| Tool | Arguments | Description | Risk |
|---|---|---|---|
| `git_diff` | `path: Option<String>`, `staged: bool` | Show diff (all or per-file, staged or unstaged) | Read-only, safe |
| `git_log` | `count: u32`, `path: Option<String>` | Show recent commits | Read-only, safe |
| `git_blame` | `path: String`, `line_start: Option<u32>`, `line_end: Option<u32>` | Blame annotations for a file | Read-only, safe |
| `git_status` | — | Full status with diff stats | Read-only, safe |
| `git_stage` | `paths: Vec<String>` | Stage files for commit | Write, low risk |
| `git_commit` | `message: String` | Create a commit from staged changes | Write, medium risk |
| `git_branch` | `name: String`, `checkout: bool` | Create and optionally switch to a branch | Write, medium risk |
| `git_stash` | `action: StashAction` | Push/pop/list stash entries | Write, low risk |

**Implementation.** Add variants to `ToolCallArguments` in `src/graph/tool_types.rs`. Add executor arms in `src/tool_executor.rs`. Register tool definitions via `registered_tool_definitions()`. All operations use `git2` — no shell commands.

**Strengths:**
- Fits perfectly into the existing tool architecture (Design 03)
- Read-only tools are zero-risk and immediately useful
- Write tools enable the agent to checkpoint its own work
- Follows the same pattern as `ReadFile` — add variant, add executor, add definition

**Weaknesses:**
- Write operations (commit, branch) need human-in-the-loop confirmation
- `git2` commit creation requires explicit signature, tree, and parent management — more complex than `git commit -m`
- Branch switching changes the working directory state for all concurrent processes

**Crate:** `git2 = "0.20"` (existing)

### 4.D — Git Worktrees for Agent Isolation

**Description.** Use `git worktree add` to create isolated working directories for parallel agent tasks. Each agent gets its own checkout, branch, and filesystem state — shared git object database means minimal disk overhead. This is the isolation pattern used by Gas Town (Doc 05, §4.2) and similar to Claude Code's agent isolation approach.

**Implementation.**

```rust
fn create_agent_worktree(
    repo: &Repository,
    task_name: &str,
) -> anyhow::Result<(PathBuf, String)> {
    let branch_name = format!("agent/{task_name}");
    let head = repo.head()?.peel_to_commit()?;
    repo.branch(&branch_name, &head, false)?;
    let worktree_path = repo.workdir()
        .unwrap()
        .join(format!("../.worktrees/{task_name}"));
    repo.worktree(
        task_name,
        &worktree_path,
        Some(WorktreeAddOptions::new().reference(/* branch ref */)),
    )?;
    Ok((worktree_path, branch_name))
}
```

**Strengths:**
- Gold standard for parallel agent isolation — proven by Gas Town at 20-30 agent scale
- Shared object database means creating a worktree is near-instant (no full clone)
- Each agent can commit, branch, and modify files without affecting others
- Natural merge workflow: agent completes work on its branch, PR/merge back to main

**Weaknesses:**
- Worktree management adds operational complexity (cleanup, orphaned worktrees)
- `git2`'s worktree API is less mature than its core APIs — may need fallback to CLI
- Disk usage scales with number of concurrent worktrees (each has a full checkout)
- Requires coordination to merge results back (conflict resolution)

**Crate:** `git2 = "0.20"` (existing, though worktree support may be limited — verify)

### 4.E — Graph Bootstrapping from Git History

**Description.** On first launch in a repository, crawl the git history to populate the graph with project context. Each commit becomes a node. Commits grouped by PR/merge become `WorkItem` nodes. File change patterns create `RelevantTo` edges. This is the "Git History Crawl" described in VISION.md §6.1.

**Implementation.** Use `repo.revwalk()` to iterate commits in reverse chronological order. For each commit:
1. Create a `Commit` node (or reuse `Message` with a `role: System` variant)
2. Parse the commit message for issue references (#123), PR references
3. Diff against parent to extract changed files
4. Create `RelevantTo` edges to `GitFile` nodes for changed files

Depth-configurable: `--bootstrap-depth 100` (last 100 commits), `--bootstrap-since 2026-01-01`, or `--bootstrap-full`.

**Strengths:**
- Solves the cold-start problem — new conversations start with project context
- Commit messages are high-signal, low-noise context (developers wrote them for humans)
- PR grouping creates natural `WorkItem` nodes without manual entry
- Completely automatic — no developer action required

**Weaknesses:**
- Full history crawl on large repos (10K+ commits) is slow and produces massive graphs
- Commit messages vary wildly in quality ("fix", "wip", "asdf" are common)
- PR detection heuristic (merge commits) is imperfect — squash merges lose PR boundaries
- Requires new node type (`Commit`) or overloading existing types

**Crate:** `git2 = "0.20"` (existing). `revwalk()`, `Commit::tree()`, `Diff::new()`.

### 4.F — Conversation Graph Versioning via Git

**Description.** Use git to version the conversation graph itself. Each save of `graph.json` becomes a commit in a dedicated branch (or a separate git repository). This enables undo, time-travel, and diffing of graph states.

**Implementation.** After `persistence.rs` writes `graph.json`, stage and commit it:
```rust
fn version_graph(repo: &Repository, graph_path: &Path) -> anyhow::Result<()> {
    let mut index = repo.index()?;
    index.add_path(graph_path.strip_prefix(repo.workdir().unwrap())?)?;
    index.write()?;
    let tree_id = index.write_tree()?;
    let tree = repo.find_tree(tree_id)?;
    let sig = Signature::now("context-manager", "noreply@context-manager")?;
    let parent = repo.head()?.peel_to_commit()?;
    repo.commit(Some("HEAD"), &sig, &sig, "auto: graph update", &tree, &[&parent])?;
    Ok(())
}
```

**Strengths:**
- Free undo: `git log` on graph.json shows every state, `git checkout` reverts
- Diff between graph states: "what nodes were added in the last 10 minutes?"
- Familiar tooling: developers already know git for versioning
- Zero new dependencies

**Weaknesses:**
- Versioning a single JSON file with git is inefficient — every save is a full copy
- Merge conflicts on graph.json are unparseable (the file is one big JSON blob)
- Commits pile up fast (every message exchange = one commit)
- Pollutes the project's git history unless using a separate repo or orphan branch
- Event sourcing (Doc 11, §4.2) is a better fit for this problem

**Crate:** `git2 = "0.20"` (existing)

### 4.G — Diff-Based Context Injection

**Description.** Go beyond raw `git diff` output. Parse diffs into semantic chunks (per-file, per-hunk), store them as graph nodes, and selectively inject only the hunks relevant to the current conversation. A hunk modifying `auth.rs` is included when discussing authentication but excluded when discussing the build system.

**Implementation.** Use `git2::Diff::foreach()` to iterate files and hunks:
```rust
diff.foreach(
    &mut |delta, _| { /* file-level callback */ true },
    None, // binary callback
    &mut |_delta, hunk| { /* hunk-level: header, old_start, new_start, lines */ true },
    &mut |_delta, _hunk, line| { /* line-level: origin, content */ true },
)?;
```

Each hunk becomes a lightweight node: `DiffHunk { file_path, old_start, new_start, content, header }`. Connect to `GitFile` nodes via `RelevantTo` edges. During context construction, select hunks whose file paths overlap with files mentioned in the conversation.

**Strengths:**
- Granular context: only the relevant parts of a diff enter the context window
- Enables intelligent token budgeting — 50 tokens per relevant hunk vs. 5,000 for full diff
- Hunk-level provenance: "the model saw this specific change when it made this suggestion"
- Aligns with multi-perspective compaction (VISION.md §4.2) — same diff, different views

**Weaknesses:**
- Adds complexity to context construction — hunk selection logic
- Hunks are ephemeral (change on every file save) — constant node churn in the graph
- Relevance scoring for hunks requires file-path matching at minimum, semantic analysis at maximum
- Over-engineering for Phase 1 — raw diff output is sufficient to start

**Crate:** `git2 = "0.20"` (existing)

### 4.H — Git Hooks Integration

**Description.** Register git hooks (pre-commit, post-commit, post-checkout, post-merge) that notify the context-orchestrator of git events. This turns the application into a git-event-driven system rather than a filesystem-event-driven one.

**Implementation.** Two approaches:
1. **Install hook scripts** in `.git/hooks/` that send signals (e.g., write to a named pipe or Unix socket)
2. **Poll git state** on filesystem events (current approach, enhanced with more detail)

Git events of interest:

| Hook | Event | Action |
|---|---|---|
| `post-commit` | Developer (or agent) made a commit | Create `Commit` node, update `GitFile` statuses |
| `post-checkout` | Branch switched | Update branch context in graph, refresh diff context |
| `post-merge` | Merge completed | Refresh graph, detect conflict resolution patterns |
| `pre-commit` | About to commit | Validate agent-generated changes (optional) |
| `post-rewrite` | Rebase/amend | Update commit nodes to reflect rewritten history |

**Strengths:**
- Event-driven is more efficient than polling — react to git events precisely
- Hook-based approach captures events that filesystem watching misses (e.g., `git stash`, `git checkout` don't always trigger file changes visible to `notify`)
- Enables commit-triggered workflows (auto-summarize, auto-tag)

**Weaknesses:**
- Git hooks are per-repository — requires installation step per project
- Hook installation modifies `.git/hooks/` which may conflict with existing hooks
- Hooks run synchronously and block git operations if slow
- Named pipe / socket approach adds IPC complexity
- Core hooks (client-side) are difficult to share via version control (`.git/hooks/` is not tracked)

**Crate:** No crate needed — hooks are shell scripts or executables

---

## 5. Comparison Matrix

| Criterion | A: Diff Context | B: Log/Blame | C: Agent Tools | D: Worktrees | E: Bootstrap | F: Graph Version | G: Diff Chunks | H: Hooks |
|---|---|---|---|---|---|---|---|---|
| **Value for LLM quality** | Very High | High | High | Medium | High | Low | Very High | Medium |
| **Implementation complexity** | Low | Medium | Medium | High | Medium | Medium | High | Medium |
| **New node types needed** | No | Yes | No | No | Yes | No | Yes | No |
| **New dependencies** | None | None | None | None | None | None | None | None |
| **Risk** | Low | Low | Medium (writes) | Medium | Low | Medium | Low | Medium |
| **VISION.md alignment** | §3.2 | §6.1, §3.1 | §4.8 | §5.1 | §6.1 | — | §4.2 | — |
| **Standalone value** | Yes | Yes | Yes | Needs multi-agent | Yes | Marginal | Needs A first | Needs listeners |
| **Phase** | 1 | 2 | 2 | 3 | 2 | 3 | 2-3 | 3 |

---

## 6. VISION.md Alignment

| VISION.md Section | Relevant Options | How They Connect |
|---|---|---|
| §3.1 Graph Model (node types) | B, E, G | Commits, blame annotations, and diff hunks are new node types that enrich the graph |
| §3.2 Context Construction | A, G | Diff context and selective hunk injection directly improve context quality |
| §4.2 Multi-Perspective Compaction | G | The same diff can be compacted differently for different perspectives (security vs. performance) |
| §4.3 Background Processing | B, E, H | History crawl, blame indexing, and hook-triggered updates are background tasks |
| §4.8 Tool Calls as Graph Citizens | C | Git tools are tool calls with full provenance — message → tool call → result chain |
| §5.2 Storage Stack | F | Graph versioning relates to the snapshot/versioning layer |
| §6.1 Git History Crawl | B, E | Direct implementation of the specified bootstrapping strategy |
| §6.5 Codebase Structure | B | Blame data enriches the file-level structural understanding |

---

## 7. Recommended Architecture

### Phase 1: Git Context in Every LLM Call (Low-hanging fruit)

**Goal:** The LLM always knows what the developer is working on.

1. **Inject current branch name** into the system directive or context builder. One `git2::Repository::head()` call.

2. **Inject uncommitted diff summary** — file names with +/- line counts via `Diff::stats()`. Costs ~50-200 tokens. Always included.

3. **Inject full diff for relevant files** — when the conversation mentions a file that has uncommitted changes, include the full diff for that file. Controlled by token budget.

4. **Enhance `GitFile` nodes** with diff stats (lines added/removed). Extend `GitFileStatus` or add fields to the `GitFile` variant.

**Files modified:**
- `src/tasks.rs` — enhance `scan_git_files()` to include diff stats and branch name
- `src/app/context.rs` — inject git context into the message list
- `src/graph/mod.rs` — add diff stats fields to `GitFile` node (optional)

**Estimated effort:** 1-2 days.

### Phase 2: Git Tools + History (Agent capabilities)

**Goal:** The agent can read and write git state, and the graph starts populated.

1. **Read-only git tools** — `git_diff`, `git_log`, `git_blame`, `git_status` as `ToolCallArguments` variants. Zero-risk, high-value.

2. **Write git tools** — `git_stage`, `git_commit` with human-in-the-loop confirmation (TUI prompt before execution). Safety constraints: `git_stage` rejects paths matching `.gitignore` and secret patterns (`*.env`, `*.key`); `git_commit` validates message length. The agent can checkpoint its work.

3. **History bootstrapping** — `spawn_git_history_crawl()` as a background task. Configurable depth. Creates lightweight commit-summary nodes.

4. **Diff-based context injection** — Parse diffs into per-file chunks. Store as ephemeral in-memory context (not persisted graph nodes — avoids node churn) that the context builder can select from. Persistent diff hunk nodes are deferred to Phase 3 and contingent on event sourcing (Doc 11).

**Files modified:**
- `src/graph/tool_types.rs` — new `ToolCallArguments` variants
- `src/tool_executor.rs` — executor arms for git tools
- `src/tasks.rs` — `spawn_git_history_crawl()` background task
- `src/app/context.rs` — diff chunk selection logic

**Estimated effort:** 1-2 weeks.

### Phase 3: Isolation + Advanced Workflows (Multi-agent)

**Goal:** Agents can work in parallel without conflicts.

1. **Worktree management** — create/destroy worktrees for agent tasks. Each agent gets `agent/{task-name}` branch + isolated checkout.

2. **Git hooks integration** — post-commit and post-checkout hooks that notify the context-orchestrator via a Unix socket or file-based signal.

3. **Diff hunk nodes** — if event sourcing (Doc 11) is implemented, persist diff hunks as immutable events with relevance scoring. If not, hunks remain ephemeral (Phase 2 approach). Do not add persistent hunk nodes to the graph without an event log — the churn would be unsustainable.

4. **Graph versioning** — if event sourcing (Doc 11) hasn't been implemented, activate orphan-branch versioning (`refs/heads/graph-versions`) as interim. Hard gate: Phase 3 requires some form of graph state history.

**Files modified:**
- New `src/git/` module for worktree management
- `src/tasks.rs` — hook listener task
- `src/graph/mod.rs` — `DiffHunk` node type, `Commit` node type
- `src/app/agent_loop.rs` — worktree-aware agent spawning

**Estimated effort:** 2-4 weeks.

---

## 8. Integration Design

### 8.1 Enhanced Git Watcher

The current `scan_git_files()` returns only path + status. The enhanced version returns:

```rust
pub struct GitFileSnapshot {
    pub path: String,
    pub status: GitFileStatus,
    pub diff_stats: Option<DiffStats>,  // NEW
}

pub struct DiffStats {
    pub lines_added: u32,
    pub lines_removed: u32,
}

pub struct GitContextSnapshot {
    pub branch: String,
    pub files: Vec<GitFileSnapshot>,
    pub diff_summary: String,       // "+45 -12 across 3 files"
    pub total_lines_added: u32,
    pub total_lines_removed: u32,
}
```

A new `TaskMessage::GitContextUpdated(GitContextSnapshot)` replaces `GitFilesUpdated` (or extends it).

### 8.2 Git Tool Definitions

```rust
// Added to ToolCallArguments in src/graph/tool_types.rs
pub enum ToolCallArguments {
    // ... existing variants ...
    GitDiff {
        path: Option<String>,
        staged: Option<bool>,
    },
    GitLog {
        count: Option<u32>,
        path: Option<String>,
    },
    GitBlame {
        path: String,
        line_start: Option<u32>,
        line_end: Option<u32>,
    },
    GitStatus,
    GitStage {
        paths: Vec<String>,
    },
    GitCommit {
        message: String,
    },
}
```

### 8.3 Context Builder Injection

In `build_context()` (`src/app/context.rs`), after collecting branch history messages:

```rust
// Inject git context as a system-level prefix
if let Some(git_ctx) = &self.git_context {
    let git_section = format!(
        "<git-context>\nBranch: {}\nUncommitted changes:\n{}\n</git-context>",
        git_ctx.branch, git_ctx.diff_summary
    );
    messages.insert(1, ChatMessage::system(git_section)); // after system directive
}
```

For file-specific diffs, check if any file mentioned in recent user messages has uncommitted changes, and include the full diff for those files.

### 8.4 Data Flow

```
                     ┌──────────────┐
                     │  git2 repo   │
                     └──────┬───────┘
                            │
              ┌─────────────┼─────────────┐
              │             │             │
         scan_files    get_diff     get_branch
              │             │             │
              v             v             v
        ┌─────────────────────────────────────┐
        │       GitContextSnapshot            │
        └──────────────┬──────────────────────┘
                       │
            TaskMessage::GitContextUpdated
                       │
              ┌────────┼────────┐
              │        │        │
              v        v        v
          GitFile   Context   TUI
          nodes     Builder   status
          (graph)   (inject)  (display)
```

---

## 9. Red/Green Team

### Green Team (Factual Verification)

10 of 10 technical claims verified. Two items unverifiable:

- **git2 0.20 APIs** (diff_tree_to_workdir_with_index, revwalk, blame_file, worktree, Diff::print/stats/foreach): All verified on docs.rs. `Diff::foreach()` callback order (file, binary, hunk, line) confirmed.
- **git2 used by Cargo**: Verified — Cargo uses libgit2 for fetching git dependencies.
- **gitui** (https://github.com/extrawurst/gitui): Verified, Rust TUI for git.
- **lazygit** (https://github.com/jesseduffield/lazygit): Verified, Go TUI for git.
- **gix/gitoxide** (https://github.com/GitoxideLabs/gitoxide): Verified, pure Rust git.
- **Aider auto-commits**: Verified — documented default behavior (disableable with `--no-auto-commits`).
- **git-diff-blame** (https://github.com/dmnd/git-diff-blame): Verified, exists and maintained.
- **Claude Code worktree mode**: UNVERIFIED — official docs do not mention a dedicated "worktree mode." Corrected in §4.D and §10 to describe general agent isolation.
- **Semantic diff chunking Medium article**: UNVERIFIABLE — URL returns HTTP 403. Retained as reference but may be inaccessible.

### Code Accuracy

14 of 15 code references verified correct. One line number error corrected:

- `run_git_watcher()` was cited at `src/tasks.rs:157` but is at line **163**. Corrected in §2.1.
- All other file:line references (`spawn_git_watcher:132`, `scan_git_files:209`, `GitFileStatus:30-35`, `EdgeKind:57-67`, `Node:82-114`, `ConversationGraph:199`, `ToolCallArguments:15`, `Cargo.toml:21`) verified accurate.
- `GitFile` Indexes edges: created by task handler (not `scan_git_files` itself). Clarified in §2.1.

### Red Team (Challenges)

**C-1 (Critical): Agent tool security unspecified.** The `git_stage` and `git_commit` tools lack safety constraints. An agent could stage `.env` files, commit secrets, or create commits with sensitive content in the message. **Fix:** Phase 2 must include:
- `git_stage` rejects paths matching `.gitignore` and known secret patterns (`*.env`, `*.key`, `credentials*`, `secrets/`)
- `git_commit` validates message length (1-500 chars) and must be preceded by `git_stage`
- All write git tools require human-in-the-loop confirmation (TUI prompt)
- Consider integrating a pre-commit hook scan for secrets

**C-2 (Critical): Diff node persistence contradicts between phases.** Phase 2 (§7) says "store as ephemeral context (not persisted nodes)" while Phase 3 says "persistent, per-hunk graph nodes." **Fix:** Phase 2 uses ephemeral in-memory diff context (no graph nodes, no churn). Phase 3 evolves to event-sourced diff history if Doc 11's event sourcing is implemented. If not, diff hunks remain ephemeral — accept the provenance trade-off. Clarified in §7.

**M-1: Missing git integration options.** The 8 categories miss several patterns:
- **Git notes** (`git2::Repository::note()`): attach agent-generated metadata to commits without modifying history
- **Git attributes**: tag files as binary or generated to exclude from diff injection
- **Tags**: enable "what changed since v1.2.3?" queries
- **Reflog**: recovery log for detecting unexpected resets
- **Signed commits**: audit trail distinguishing agent vs. human commits
- **Git bisect**: automated regression finding

These are lower priority than the core 8 categories. Git attributes (for filtering noisy diffs) should be considered in Phase 1. Tags and notes fit Phase 2. The rest is Phase 3+.

**M-2: Phase ordering not explicitly justified.** Read-only git tools (Phase 2) are also zero-risk and low-complexity — arguably equal priority to diff context injection (Phase 1). **Resolution:** Phase 1 diff context benefits *every* LLM call (user conversations and agent loops). Phase 2 tools benefit only agent autonomy. The ordering is correct but should be understood as: Phase 1 = passive context enrichment for all interactions, Phase 2 = active agent capabilities.

**M-3: gix dismissal undervalues build-time benefits.** gix is pure Rust (no C compilation), which means faster builds and better portability. git2 requires compiling libgit2 via libgit2-sys. **Resolution:** git2 is the pragmatic choice for Phases 1-2 (mature API, already in Cargo.toml). Phase 3 should include a decision gate: if git2's worktree API proves limiting or build times become a concern, evaluate migrating to gix.

**M-4: Graph versioning dismissed without interim solution.** Event sourcing (Doc 11) is not implemented. Leaving graph versioning undefined until it arrives creates a feature gap — no undo, no time-travel. **Resolution:** If event sourcing is not implemented by Phase 2 completion, activate orphan-branch graph versioning as an interim measure. Commits to `refs/heads/graph-versions` are cheap and isolated from project history.

**M-5: blame_file() performance ungrounded.** The claim "O(n) in file length — expensive for large files" lacks numbers. Real-world: libgit2 blame on a 10K-line file is typically 50-150ms. Files over 50K LOC may take 500ms+. **Resolution:** Phase 2 should cache blame results per file per commit OID, invalidating when the file is modified. Background blame indexing (whole codebase, once per session) is a Phase 3 optimization.

**M-6: Worktree scaling on developer laptops.** Gas Town runs 20-30 agents on servers. A developer laptop may not support that scale — each worktree is a full checkout. For a 5GB monorepo, 20 worktrees = 100GB. **Resolution:** Phase 3 should target 2-5 concurrent agents (laptop scale). For larger scale, evaluate shallow clone + sparse checkout to reduce per-worktree footprint. Consider tmpdir copies for short-lived ephemeral agents (< 1 minute).

**M-7: Worktree API maturity unverified.** The document notes git2's worktree API may be less mature but doesn't verify. **Resolution:** Before Phase 3 implementation, verify: can git2 create a worktree on a specific branch? List existing worktrees? Delete with cleanup? If any gap, fall back to `std::process::Command::new("git")` for that operation.

**M-8: Multi-agent coordination strategy not sketched.** Phase 3 proposes worktrees but doesn't explain how results merge back. **Resolution:** Each agent works on `agent/{task-name}` branch. Results merge via fast-forward (if no conflicts) or human review (if conflicts). Conflict detection is a Phase 3 requirement.

**M-9: Event sourcing deferral risk.** If event sourcing (Doc 11) is indefinitely deferred, Phase 3 has no graph versioning story. **Resolution:** Hard gate — Phase 3 requires either event sourcing or orphan-branch versioning. Do not release Phase 3 without some form of graph state history.

---

## 10. Sources

### Crates
- **git2** (0.20.x): https://crates.io/crates/git2 — Rust bindings to libgit2. Thread-safe, mature. Used by Cargo itself.
- **gix** (gitoxide): https://github.com/GitoxideLabs/gitoxide — Pure Rust git implementation. Not recommended for this project due to less mature high-level API, but worth monitoring.

### Git Integration Prior Art
- **Aider**: CLI coding assistant with deep git integration. Auto-commits after each LLM edit. Uses git diff for context. https://aider.chat/
- **Claude Code**: Agent isolation via git operations (branching, commits, PRs). https://docs.anthropic.com/en/docs/claude-code/
- **Gas Town** (Doc 05): 20-30 agents using git worktrees + Dolt for coordination
- **lazygit**: Go TUI for git. Single-screen interface, interactive staging. https://github.com/jesseduffield/lazygit
- **gitui**: Rust TUI for git. High performance with large repos. https://github.com/extrawurst/gitui

### Context Engineering
- **Semantic diff chunking**: https://medium.com/@yehezkieldio/precision-dissection-of-git-diffs-for-llm-consumption-7ce5d2ca5d47
- **git-diff-blame**: Combines diff and blame output. https://github.com/dmnd/git-diff-blame

### Internal References
- `VISION.md` §3.1 (graph model), §3.2 (context construction), §4.3 (background processing), §4.8 (tool provenance), §6.1 (git history crawl)
- `docs/research/05-gastown-multi-agent-orchestration.md` — worktree isolation patterns
- `docs/research/11-multi-source-input-architecture.md` — git as concurrent knowledge source
- `docs/design/03-tool-call-foundation.md` — tool dispatch architecture
- `src/tasks.rs:132-236` — current git integration
- `src/graph/mod.rs:30-35` — `GitFileStatus`, `src/graph/mod.rs:82-114` — `Node` enum
- `src/graph/tool_types.rs:15-41` — `ToolCallArguments` enum
