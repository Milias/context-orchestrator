# Design: Git Repository Graph Nodes

**Date:** 2026-03-14
**Status:** Draft
**Prerequisite:** Research doc `docs/research/13-git-integration.md`

---

## 1. Motivation

The context-orchestrator currently represents git data through a single `GitFile` node storing only `path`, `status` (Tracked/Modified/Staged/Untracked), and `updated_at`. The git watcher (`scan_git_files()` in `src/tasks.rs:210-243`) calls `git2::Repository::statuses()` to populate these nodes. This tells us *which* files changed but not *what* changed, *who* changed them, *when*, or *why*.

Git is the richest source of structured project context available to any development tool. Every commit is a timestamped, attributed, message-annotated snapshot of intent. Every branch is an isolation boundary. Every diff is a precise description of change. None of this reaches the graph today.

This document specifies new node types, edge types, enrichments to the existing `GitFile` node, a sync strategy, and context integration points. All proposed work uses the existing `git2 = "0.20"` dependency — no new crates are required.

**Design principle:** Git artifacts are first-class graph citizens. Branches, commits, files, and tags are nodes. Relationships between them (parentage, containment, modification) are typed edges. The LLM receives a concise summary of repository state as part of the system prompt, with full details available via graph traversal and future git tools (doc 13's `git_diff`, `git_log`, `git_blame`, `git_status` tools). Git tools operate on the live repository for on-demand data; git graph nodes are cached metadata. They serve complementary purposes and do not replace each other.

---

## 2. New Node Types

### 2.1 GitRepository

Root anchor for all git-derived data from a single repository. The graph may contain multiple `GitRepository` nodes when the workspace spans multiple repos (e.g., submodules, polyrepo setups, or an orchestrator managing several projects).

```rust
/// Root anchor for git-derived data from a single repository.
/// Multiple instances supported for multi-repo workspaces.
GitRepository {
    id: Uuid,
    /// Absolute path to the repository root (workdir).
    workdir: String,
    /// Current HEAD reference (branch name or detached OID).
    head_ref: String,
    /// Whether the working tree has uncommitted changes.
    is_dirty: bool,
    updated_at: DateTime<Utc>,
}
```

**Rationale:** Currently `GitFile` nodes connect to the branch leaf via `Indexes` edges, which is semantically wrong — they index the repo, not the conversation. A dedicated root node fixes this. Using one node per repository means the graph naturally supports multi-repo scenarios — all git nodes `BelongsTo` their specific `GitRepository`, making it unambiguous which repo a branch, commit, or file belongs to.

**`Node::content()` mapping:** Returns `workdir`.
**`Node::created_at()` mapping:** Returns `updated_at`.

### 2.2 GitBranch

Local branches with upstream tracking information.

```rust
/// A local git branch with optional upstream tracking info.
GitBranch {
    id: Uuid,
    /// Branch name (e.g., "main", "feat/new-feature").
    name: String,
    /// Whether this branch is currently checked out (HEAD points here).
    /// In detached HEAD state, no GitBranch has `is_head: true`.
    is_head: bool,
    /// Upstream tracking branch, if configured (e.g., "origin/main").
    upstream: Option<String>,
    /// Commits ahead of upstream. `None` if no upstream configured.
    ahead: Option<u32>,
    /// Commits behind upstream. `None` if no upstream configured.
    behind: Option<u32>,
    /// OID of the branch tip commit.
    tip_oid: String,
    updated_at: DateTime<Utc>,
}
```

**Rationale:** Branch tracking info (ahead/behind) is high-value context for the LLM when it needs to understand where the developer is relative to upstream. The `is_head` flag allows a single query to find the current branch. In detached HEAD state, `GitRepository.head_ref` contains the raw OID and no branch has `is_head: true`; the context builder renders this as "Detached HEAD at abc1234".

**`Node::content()` mapping:** Returns `name`.
**`Node::created_at()` mapping:** Returns `updated_at`.

### 2.3 GitCommit

Recent commits within a bounded window (default 50 on the current branch).

```rust
/// A git commit. Only recent commits are materialized (bounded window).
GitCommit {
    id: Uuid,
    /// Full 40-character hex SHA.
    oid: String,
    /// Commit author name.
    author_name: String,
    /// Commit author email.
    author_email: String,
    /// First line of the commit message.
    summary: String,
    /// Full commit message body (excluding summary).
    body: Option<String>,
    /// Author timestamp (not committer timestamp).
    authored_at: DateTime<Utc>,
    /// Number of files changed in this commit.
    files_changed: u32,
    /// Total lines added across all files in this commit.
    insertions: u32,
    /// Total lines removed across all files in this commit.
    deletions: u32,
    updated_at: DateTime<Utc>,
}
```

**Rationale:** Commits are the fundamental unit of work in git. The `summary`/`body` split follows git convention. `files_changed`, `insertions`, `deletions` are stored (not computed on demand) because `diff_tree_to_tree()` is expensive. The `oid` is `String` for display/serde convenience. Only the last `DEFAULT_COMMIT_WINDOW` (50) commits on the current branch are materialized — older commits add bloat without LLM value. For merge commits with multiple parents, `diff_tree_to_tree` diffs against the first parent only (matching `git log --first-parent` behavior); all parent OIDs get `ChildOf` edges.

**`Node::content()` mapping:** Returns `summary`.
**`Node::created_at()` mapping:** Returns `updated_at`.

### 2.4 GitTag

Release markers (lightweight and annotated).

```rust
/// A git tag (lightweight or annotated). Represents release markers.
GitTag {
    id: Uuid,
    /// Tag name (e.g., "v1.0.0").
    name: String,
    /// OID of the tagged commit.
    target_oid: String,
    /// Tag message for annotated tags, `None` for lightweight tags.
    message: Option<String>,
    /// When the tag was created (tagger date for annotated, commit date for lightweight).
    tagged_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}
```

**Rationale:** Tags mark releases and milestones. Knowing the most recent tag and its distance from HEAD is valuable context ("3 commits since v0.4.2"). Lightweight storage — only a handful of tags are expected. For lightweight tags, `tagged_at` is approximate (uses target commit's author date as proxy, since lightweight tags have no tagger object).

**`Node::content()` mapping:** Returns `name`.
**`Node::created_at()` mapping:** Returns `updated_at`.

---

## 3. Enriched GitFile

The existing `GitFile` variant gains four optional fields with `#[serde(default)]` for backward compatibility:

```rust
/// A file tracked (or untracked) by git. Enriched with diff and churn metadata.
GitFile {
    id: Uuid,
    path: String,
    status: GitFileStatus,
    /// Lines added in the current working tree diff (unstaged changes).
    #[serde(default)]
    insertions: Option<u32>,
    /// Lines removed in the current working tree diff (unstaged changes).
    #[serde(default)]
    deletions: Option<u32>,
    /// Number of commits that have touched this file in the materialized window.
    /// Higher values indicate "hot" files — actively changing code.
    #[serde(default)]
    change_frequency: Option<u32>,
    /// Timestamp of the most recent commit that modified this file.
    /// `None` for untracked files. Enables staleness detection.
    #[serde(default)]
    last_commit_at: Option<DateTime<Utc>>,
    updated_at: DateTime<Utc>,
}
```

### 3.1 GitFileStatus Enhancement

Add `Deleted` variant. Currently `WT_DELETED` is mapped to `Modified`, which loses precision:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GitFileStatus {
    Tracked,
    Modified,
    Staged,
    Untracked,
    /// File has been deleted from the working tree but not staged.
    Deleted,
}
```

### 3.2 Field Rationale

| Field | Signal | LLM Value |
|---|---|---|
| `insertions` / `deletions` | How much a file changed | Immediate visibility into change magnitude without running a tool |
| `change_frequency` | Hot file indicator | Files changing frequently are likely relevant to current work |
| `last_commit_at` | Staleness signal | Old files are stable core; recently changed files are active |

---

## 4. New Edge Types

Five new `EdgeKind` variants (one existing variant removed):

```rust
pub enum EdgeKind {
    // ... existing variants unchanged, EXCEPT Indexes is removed ...

    /// GitBranch/GitCommit/GitFile/GitTag → GitRepository: structural containment.
    BelongsTo,
    /// GitCommit (child) → GitCommit (parent): "this commit is a child of that commit."
    /// Merge commits produce multiple ChildOf edges (one per parent).
    ChildOf,
    /// GitCommit → GitFile: this commit touched (added/modified/deleted/renamed) this file.
    Touches,
    /// GitBranch → GitCommit: the branch tip points to this commit.
    TipOf,
    /// GitTag → GitCommit: the tag targets this commit.
    Tags,
}
```

**Note:** The `Indexes` edge kind is **removed**. It was used exclusively for `GitFile → branch leaf` edges, which is replaced by `BelongsTo → GitRepository`. No other code path uses `Indexes`. Per CLAUDE.md: no dead code.

### 4.1 Edge Semantics

All edges follow `from → to` convention matching `add_edge(from, to, kind)`:

| Edge | From | To | Reads as |
|---|---|---|---|
| `BelongsTo` | GitBranch/GitCommit/GitFile/GitTag | GitRepository | "branch belongs to repo" |
| `ChildOf` | GitCommit (child) | GitCommit (parent) | "child commit is child of parent commit" |
| `Touches` | GitCommit | GitFile | "commit touches file" |
| `TipOf` | GitBranch | GitCommit | "branch is tip of commit" |
| `Tags` | GitTag | GitCommit | "tag tags commit" |

**Edge creation example:**
```rust
// Branch belongs to repo:
g.add_edge(branch_id, repo_id, EdgeKind::BelongsTo);
// Commit is child of its parent:
g.add_edge(child_commit_id, parent_commit_id, EdgeKind::ChildOf);
// Commit touches a file:
g.add_edge(commit_id, file_id, EdgeKind::Touches);
```

### 4.2 Migration from `Indexes`

The `Indexes` variant is removed from `EdgeKind` entirely. It was used exclusively for `GitFile → branch leaf` edges (`src/app/task_handler.rs:31`). No other code path uses it. The replacement is `BelongsTo → GitRepository`, which has correct semantics.

If a future feature needs an "indexes" relationship, it should define a new edge kind with specific semantics rather than reusing this generic name.

### 4.3 Why `GitCommit` also gets `BelongsTo`

Commits need `BelongsTo → GitRepository` edges for two reasons:

1. **Scoped removal correctness.** Without this edge, the removal step ("remove all nodes that `BelongsTo` this repo") would miss commits, leaving orphaned nodes that accumulate on every refresh cycle.

2. **Direct repo membership queries.** Without `BelongsTo`, a commit's repo can only be determined by traversing `ChildOf → ... → TipOf → BelongsTo`, which is fragile when commits are shared between branches.

### 4.4 Derived Queries from Edges

The new edge structure enables queries that were previously impossible:

| Query | Traversal |
|---|---|
| "What files did commit X touch?" | `Touches` edges from GitCommit X |
| "Which commits modified file Y?" | `Touches` edges to GitFile Y |
| "What are the parents of commit X?" | `ChildOf` edges from X |
| "What's the latest tag on this branch?" | `TipOf` → `ChildOf` chain → find `Tags` target |
| "Which files change together?" | Co-occurrence of files in `Touches` edges from same GitCommit (computed on demand, O(n^2)) |
| "Who owns this file?" | `Touches` edges to file → commit authors (computed on demand) |
| "All git data for repo X?" | `BelongsTo` edges to GitRepository X |

---

## 5. Sync Strategy

### 5.1 New Snapshot Types

Replace `GitFileSnapshot` with a comprehensive `GitStateSnapshot`:

```rust
/// Complete snapshot of git repository state for graph replacement.
pub struct GitStateSnapshot {
    /// When this snapshot was captured (used for `updated_at` on all created nodes).
    pub scanned_at: DateTime<Utc>,
    /// Absolute path to the repository root.
    pub workdir: String,
    /// Current HEAD reference (branch name or detached OID).
    pub head_ref: String,
    /// Whether the working tree has uncommitted changes.
    pub is_dirty: bool,
    pub branches: Vec<GitBranchSnapshot>,
    pub commits: Vec<GitCommitSnapshot>,
    pub files: Vec<GitFileSnapshot>,
    pub tags: Vec<GitTagSnapshot>,
    /// Commit OID → file path associations for Touches edges.
    /// Resolved to node UUIDs during graph application using lookup maps.
    pub commit_file_touches: Vec<(String, String)>,
}
```

Sub-snapshot types mirror the corresponding node fields (same pattern as existing `GitFileSnapshot` and `ToolSnapshot`).

### 5.2 Data Collection: `scan_git_state()`

Replaces `scan_git_files()`. Opens `git2::Repository` once, then:

1. **HEAD + dirty**: `repo.head()` for ref name, `repo.statuses()` non-empty check for dirty flag. In detached HEAD state, `head_ref` is the raw OID.
2. **Branches**: `repo.branches(Some(BranchType::Local))` for each branch; `branch.upstream()` and `repo.graph_ahead_behind()` for tracking info
3. **Commits**: `repo.revwalk()` from HEAD with limit (`DEFAULT_COMMIT_WINDOW: usize = 50`). For each commit, read `commit.summary()`, `commit.author()`, and diff against **first parent only** via `repo.diff_tree_to_tree()` for diffstats and file touches. All parent OIDs are collected for `ChildOf` edges. If a single `diff_tree_to_tree` call exceeds 50ms, skip diffstats for that commit (set `files_changed`/`insertions`/`deletions` to 0) to prevent large commits from blocking the watcher.
4. **Files**: `repo.statuses()` with `include_untracked(true)`, `recurse_untracked_dirs(true)`, `exclude_submodules(true)` for file status (existing logic), enriched with diffstat from `repo.diff_index_to_workdir()` for per-file insertions/deletions. `.gitignore`d files are excluded by default (git2 respects `.gitignore`).
5. **Tags**: `repo.tag_names()` → resolve each to target commit OID
6. **Derived fields**: Count commits per file path → `change_frequency`; find most recent commit per file → `last_commit_at`

**Backpressure:** If a scan is already in progress when the watcher triggers, the new trigger is skipped (prevents unbounded queuing on repos with large commits).

### 5.3 Watcher Integration

The existing `run_git_watcher()` structure is preserved:
- Still uses `notify-debouncer-mini` (500ms debounce)
- Still runs on `spawn_blocking`
- Calls `scan_git_state()` instead of `scan_git_files()`
- Sends `TaskMessage::GitStateRefreshed(GitStateSnapshot)` instead of `GitFilesUpdated`

### 5.4 Graph Application

In `task_handler.rs`, the `GitStateRefreshed` handler:

1. **Scoped removal**: Find the `GitRepository` node by matching `workdir`. Collect all node IDs with a `BelongsTo` edge targeting that repo ID using `sources_by_edge(repo_id, EdgeKind::BelongsTo)`. Remove the repo node and all collected nodes. This requires no new graph method — `sources_by_edge` already exists. If no `GitRepository` exists yet (first scan), skip removal.

2. **Create GitRepository**: One node for this repo, using `snapshot.scanned_at` for `updated_at`.

3. **Create child nodes with lookup maps**: Create all `GitBranch`, `GitCommit`, `GitFile`, `GitTag` nodes. During creation, build two `HashMap` lookup tables:
   - `oid_to_uuid: HashMap<String, Uuid>` — maps commit OID to node UUID
   - `path_to_uuid: HashMap<String, Uuid>` — maps file path to node UUID

   Each node gets a `BelongsTo → GitRepository` edge via `g.add_edge(node_id, repo_id, EdgeKind::BelongsTo)`.

4. **Create inter-node edges using lookup maps**:
   - `ChildOf`: For each commit with parent OIDs, look up parent UUID via `oid_to_uuid` and create `g.add_edge(child_id, parent_id, EdgeKind::ChildOf)`
   - `Touches`: Iterate `commit_file_touches`, resolve both OID and path via the lookup maps, create edges
   - `TipOf`: For each branch, resolve `tip_oid` via `oid_to_uuid`
   - `Tags`: For each tag, resolve `target_oid` via `oid_to_uuid`

5. **Emit event**: `GraphEvent::GitStateRefreshed { file_count, branch_count, commit_count }`.

### 5.5 Why Full Replace Per Repo

The total per repo is bounded (~1 repo + ~5 branches + 50 commits + ~200 files + ~10 tags ≈ 266 nodes). Incremental diffing adds complexity without proportional benefit at this scale. The per-repo scoping ensures multiple repos don't interfere with each other. The existing pattern for `Tool` and `GitFile` nodes already uses full replacement successfully.

---

## 6. Stored vs. Computed on Demand

| Data | Stored | On Demand | Rationale |
|---|---|---|---|
| Branch name, upstream, ahead/behind | Yes | — | Cheap to read from git, useful for display |
| Commit summary, author, diffstats | Yes | — | Diff computation is expensive; cache in node |
| File status, insertions/deletions | Yes | — | Already computed by git2 statuses |
| File change_frequency, last_commit_at | Yes | — | Requires counting across commits; cache |
| Co-change patterns (temporal coupling) | — | Yes | O(n^2) across commits; agent computes from `Touches` edges |
| Blame data (per-line authorship) | — | Yes | Very expensive; compute per-file via git tool |
| Commit intent classification (fix/feat/refactor) | — | Yes | Requires message parsing; future agent capability |
| Full diff content | — | Yes | Can be tens of thousands of tokens; available via git tools |

**Principle:** Store anything that would require re-opening the git repo or walking history. Compute anything that requires cross-node analysis or would bloat individual nodes.

---

## 7. Context Integration

### 7.1 Git Section Builder

New `build_git_section()` in `src/app/context/git_context.rs` generates a concise system prompt section by querying graph nodes (no git repo access at context-build time):

```
## Repository State
Branch: feat/new-feature (3 ahead, 1 behind origin/feat/new-feature)
Working tree: 4 modified, 2 staged, 1 untracked

### Recent Commits (last 5)
- abc1234 fix: resolve panic in graph mutation (2h ago)
- def5678 feat: add branch tracking to git watcher (4h ago)
- 1234abc refactor: split node.rs into module (yesterday)
...

### Hot Files (change_frequency >= 3)
- src/graph/node.rs (8 changes, +42/-18 unstaged)
- src/tasks.rs (5 changes, +15/-3 staged)
```

**Budget target:** Under 500 tokens. The git section is injected into the system prompt alongside the existing error section injection point in `conversational.rs`.

### 7.2 Multi-Repo Display

When multiple `GitRepository` nodes exist, the section is repeated per repo with the workdir as a header:

```
## Repository: /home/user/project-a
Branch: main (up to date)
...

## Repository: /home/user/project-b
Branch: feat/experiment (2 ahead)
...
```

---

## 8. GraphEvent Change

Replace `GitFilesRefreshed { count: usize }` with:

```rust
/// Git repository state was fully refreshed.
GitStateRefreshed {
    file_count: usize,
    branch_count: usize,
    commit_count: usize,
}
```

---

## 9. Module Restructuring

### 9.1 `node.rs` → `node/` Module

`node.rs` is currently 363 lines. Adding 4 new `Node` variants with their fields, 5 new `EdgeKind` variants, and a `GitFileStatus::Deleted` variant pushes it well over the 400-line limit.

Split into:
- `src/graph/node/mod.rs` — `Node` enum with all 16 variants (12 existing + 4 new), `Edge` struct, `EdgeKind` enum (21 variants), `impl Node` block with `content()`/`created_at()` match arms for new variants
- `src/graph/node/enums.rs` — supporting enums: `Role`, `StopReason`, `WorkItemKind`, `WorkItemStatus`, `GitFileStatus` (with `Deleted`), `BackgroundTaskKind`, `TaskStatus`, `QuestionDestination`, `QuestionStatus`

### 9.2 `tasks.rs` → `tasks/` Module

`tasks.rs` is currently 296 lines. Adding `scan_git_state()` with richer data collection logic pushes it over 400.

Split into:
- `src/tasks/mod.rs` — `TaskMessage` enum, spawn functions, `AgentEvent`, `AgentPhase`, existing snapshot types
- `src/tasks/git_scan.rs` — `scan_git_state()`, `GitStateSnapshot`, sub-snapshot structs (`GitBranchSnapshot`, `GitCommitSnapshot`, `GitTagSnapshot`), enriched `GitFileSnapshot`, `DEFAULT_COMMIT_WINDOW` constant

---

## 10. Files to Modify

| File | Change |
|---|---|
| `src/graph/node.rs` | Add 4 new `Node` variants (GitRepository, GitBranch, GitCommit, GitTag), enrich `GitFile`, add 5 `EdgeKind` variants (`BelongsTo`, `ChildOf`, `Touches`, `TipOf`, `Tags`), remove `Indexes`, add `GitFileStatus::Deleted`. Add `content()`/`created_at()` match arms. Split into `node/` module directory. |
| `src/graph/event.rs` | Replace `GitFilesRefreshed` with `GitStateRefreshed { file_count, branch_count, commit_count }` |
| `src/graph/mod.rs` | Update module path `node.rs` → `node/`. Update `Node` match arms. Remove any `Indexes` references. |
| `src/graph/mutation.rs` | No new methods needed — scoped removal uses existing `sources_by_edge()` + `remove_nodes_by()` in task handler |
| `src/tasks.rs` | Replace `scan_git_files()` → `scan_git_state()`, add `GitStateSnapshot` and sub-snapshots, add `DEFAULT_COMMIT_WINDOW` const, add backpressure flag, update `TaskMessage`. Split into `tasks/` module. |
| `src/app/task_handler.rs` | Rewrite `GitFilesUpdated` handler → `GitStateRefreshed` with scoped removal via `sources_by_edge`, multi-node creation with `HashMap` lookup maps, edge creation |
| `src/app/event_dispatch.rs` | Update `GitFilesRefreshed` match arm → `GitStateRefreshed` |
| `src/app/context/policies/conversational.rs` | Add match arms for 4 new node types in skip list, inject git context section |
| New: `src/app/context/git_context.rs` | `build_git_section()` for system prompt injection |
| `src/tui/widgets/tool_status.rs` | Add display labels for new node types if rendered |
| `src/tui/tabs/overview.rs` | Surface branch/commit info in stats panel |

---

## 11. Red/Green Team

### Green Team

1. **Bounded complexity**: ~266 git nodes per repo, full-replace scoped per repo via `BelongsTo` edges. No unbounded growth.
2. **Follows existing patterns exactly**: Snapshot → `TaskMessage` → bulk replace → `GraphEvent` is the same pipeline as `ToolSnapshot` and `GitFileSnapshot`.
3. **Multi-repo ready**: Each `GitRepository` anchors its own subgraph; `BelongsTo` edges (including on commits) disambiguate ownership. No singleton assumptions.
4. **`git2` handles everything**: Branches (`repo.branches()`), commits (`repo.revwalk()`), diffs (`repo.diff_tree_to_tree()`), tags (`repo.tag_names()`), ahead/behind (`repo.graph_ahead_behind()`) are all available in the existing `git2 = "0.20"` dependency.
5. **Backward compatible**: All new `GitFile` fields are `Option` with `#[serde(default)]`. Existing serialized graphs deserialize correctly. `GitFileStatus::Deleted` is a new variant — downgrade to older versions will fail on graphs containing deleted files (acceptable since git nodes are ephemeral and rebuilt on startup).
6. **High-value LLM signals**: Branch tracking, recent commits, file churn give the LLM situational awareness that dramatically improves code suggestions and plan quality.
7. **Context injection is cheap**: The git section builder only queries nodes already in the graph; no git repo access at context-build time.
8. **Edge names read naturally in `from → to` direction**: `BelongsTo`, `ChildOf`, `Touches`, `TipOf`, `Tags` all read correctly as "X is/does Y to Z".

### Red Team

1. **Performance: diffstat per commit in watcher.** The revwalk is bounded at `DEFAULT_COMMIT_WINDOW` (50). `diff_tree_to_tree` for diffstat-only is fast in libgit2. Per-commit timeout (50ms) prevents large commits from blocking the scan. Watcher backpressure (skip trigger if scan in progress) prevents unbounded queuing.

2. **Full replacement wasteful when nothing changed.** The watcher triggers on every filesystem event (after debouncing). A snapshot hash comparison could skip no-ops. The current full-replace pattern works at this scale; optimize later if profiling shows a bottleneck.

3. **`Touches` edges could be numerous.** With 50 commits touching an average of 5 files each, that's ~250 edges. This is within normal bounds for a graph that already has O(hundreds) of edges. The full-replace pattern cleans up stale edges.

4. **`EdgeKind` variant count changes from 17 to 21.** (Added 5, removed `Indexes`.) All variants are semantically distinct. Sustainable to ~30; consider grouping into sub-enums beyond that.

5. **Blame data not stored.** Correct decision: blame is per-line and per-file, potentially megabytes for a full repo. Available as an on-demand tool (future `git_blame` tool from research doc 13). The `Touches` edges provide commit-level authorship as a lighter alternative.

6. **Co-change patterns not stored.** Correct: computing which files change together requires O(n^2) pairwise comparison across commits. The graph has the raw data (`Touches` edges) to compute this on demand when an agent requests it.

7. **Full diff content not stored in nodes.** Large diffs (formatter runs, bulk renames) can be tens of thousands of tokens. Diff content should be available via git tools, not baked into nodes. The `insertions`/`deletions` counts are the right granularity for always-present metadata.

8. **Multi-repo watcher complexity.** Initially, only the repo containing the current working directory is watched. Multi-repo support requires spawning additional watchers, each producing `GitStateRefreshed` with their own `workdir`. This is additive — each watcher is independent. Not required for Phase 1.

9. **`BelongsTo` is overloaded for 4 source types.** All represent structural containment. Consumers must filter by node type after querying `sources_by_edge`. This is acceptable and documented — no alternative edge kind offers better clarity without proliferating near-identical variants.

10. **Submodules deferred.** Submodules are explicitly excluded via `exclude_submodules(true)` in the status options. Whether each submodule becomes a separate `GitRepository` node is a future design decision. Submodule paths will not appear as regular `GitFile` nodes.

11. **Merge commits.** Handled: `diff_tree_to_tree` uses first parent only. All parent OIDs get `ChildOf` edges. Documented in §2.3.

---

## 12. Verification

1. `cargo build` — zero warnings (new node variants, edge kinds, match arms all compile)
2. `cargo clippy --all-targets` — zero warnings
3. `cargo test` — existing + new tests:
   - New node type serialization round-trip
   - `scan_git_state()` produces valid snapshot from test repository
   - Task handler creates correct nodes and edges from snapshot
   - `build_git_section()` generates expected output format
   - `GitFileStatus::Deleted` maps correctly from `WT_DELETED`
4. Visual: git section appears in system prompt (debug context inspector)
5. Visual: TUI overview shows branch/commit info

---

## 13. Graph Topology Diagram

All arrows show `from ──[EdgeKind]──▸ to` direction matching `add_edge(from, to, kind)`:

```
                          GitRepository (workdir: "/home/user/project")
                                ▲                    ▲
                                │ BelongsTo           │ BelongsTo
                                │                     │
  GitBranch ("main", is_head) ──┘                     │
        │                                             │
        │ TipOf                                       │
        ▼                                             │
  GitCommit (oid: "abc1234") ─────────────────────────┘
        │           │
        │ ChildOf   │ Touches
        ▼           ▼
  GitCommit     GitFile ("src/main.rs")
  ("def5678")       ▲
        │           │ Touches
        │ ChildOf   │
        ▼      GitCommit ("abc1234")   [same commit, two Touches edges]
  GitCommit
  ("...")

  GitTag ("v0.4.2") ──[Tags]──▸ GitCommit ("def5678")
        │
        │ BelongsTo
        ▼
  GitRepository

  GitFile ("src/tasks.rs", Staged, +15/-3) ──[BelongsTo]──▸ GitRepository
```

**Key:** Every git node has a `BelongsTo` edge to its `GitRepository`. Commits also have `ChildOf` edges to their parents and `Touches` edges to modified files. The diagram shows a subset of edges for clarity — in practice every `GitBranch`, `GitCommit`, `GitFile`, and `GitTag` has a `BelongsTo → GitRepository` edge.

---

## 14. Sources

- `docs/research/13-git-integration.md` — comprehensive git integration research
- `docs/design/01-graph-extensions-and-context-panel.md` — original graph extension design
- `docs/VISION.md` §3.1 (graph model), §6.1 (git history crawl), §6.5 (codebase structure)
- `git2` crate docs: `Repository::branches()`, `Repository::revwalk()`, `Diff::stats()`, `Repository::tag_names()`, `Repository::graph_ahead_behind()`
- Reflectoring/Neo4j git graph model: Commit, File, FileSnapshot, Author nodes with TOUCHES edges
- GitLab Orbit: Property graph from SDLC metadata, Rust-based, ClickHouse-backed
