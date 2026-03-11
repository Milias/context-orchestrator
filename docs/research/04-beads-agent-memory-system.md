# Beads: A Distributed Agent Memory System

> Analysis conducted 2026-03-11 examining the Beads project (v0.56+) as prior art
> for persistent agent memory, tiered compaction, dependency-aware work tracking,
> and agent-optimized CLI design. Source: repository at `steveyegge/beads`,
> ~234K lines of Go across 740 files.

---

## 1. Executive Summary

Beads is a distributed, git-backed graph issue tracker designed specifically for AI coding agents. Created by Steve Yegge and built almost entirely through "vibe coding" with Claude, it solves the **"50 First Dates" problem** -- agents lose all context between sessions and must rediscover project state every time they start.

The system provides agents with persistent, structured memory through a dependency-aware issue graph backed by [Dolt](https://github.com/dolthub/dolt), a version-controlled SQL database. Key mechanisms include hash-based deterministic IDs (zero merge conflicts), tiered AI compaction (memory decay for old closed tasks), a molecule/formula workflow system, and a federation model for peer-to-peer sync.

**Why it matters for Context Manager:**

- **Validates compaction as essential.** Beads independently arrived at tiered compaction to manage context window pressure, confirming our MergeTree-inspired background processing design.
- **Validates dependency-aware work tracking.** 19 dependency types organized by semantic category show that rich edge semantics are necessary, not overengineering.
- **Demonstrates agent-first CLI.** JSON output, `bd ready` for unblocked work, atomic `--claim`, and batch commit mode show what agents actually need from tooling.
- **Highlights different trade-offs.** Beads chose a relational model (Dolt SQL) over a property graph, a single monolithic Issue struct over typed nodes, and single-perspective compaction over multi-perspective. These divergences sharpen our design rationale.

---

## 2. Problem Statement and Motivation

### Agent Amnesia

Every coding agent session starts from zero. The agent reads `CLAUDE.md`, scans the codebase, and reconstructs context that existed moments ago in a prior session. Yegge calls this the "50 First Dates" problem -- the agent has no persistent memory of what it was working on, what decisions were made, or what depends on what.

Traditional issue trackers (Jira, GitHub Issues, Linear) compound the problem:

- They are designed for humans, not agents -- HTML UIs, no structured output
- They lack dependency graphs that agents can traverse programmatically
- They have no concept of context window optimization or compaction
- They cannot federate across agent-local workflows without centralized servers

### Origin and Scale

Beads was built by Steve Yegge starting in late 2025, largely through extended Claude coding sessions. Yegge describes the initial version as built in "6 days of vibe coding" -- the first working prototype with epics, child issues, and agent-compatible JSON output. The project then grew from that simple markdown-based task tracker to a full distributed issue system over approximately six weeks of intensive development. As of March 2026, the codebase comprises ~234K lines of Go across 740 source files, with a Dolt SQL backend, federation support, and a molecule/formula workflow engine.

The v0.56 release ("The Great Purge") removed the embedded Dolt driver, SQLite ephemeral store, and all JSONL plumbing, reducing the binary from 168MB to 41MB and establishing external Dolt as the sole backend. This release also introduced transaction infrastructure (proper isolation, retry, batch semantics for `bd mol bond/squash/cook`), opt-in OpenTelemetry instrumentation for hook and storage operations, and queryable metadata (`bd list --metadata-field key=value`).

---

## 3. Architecture

### Two-Layer Design

Beads follows a CLI-first architecture built on two primary layers:

```
CLI (Cobra commands, 100+ commands including subcommands)
    |
    v
Storage Layer (Dolt SQL, version-controlled)
    |
    v
Remote Sync (Dolt remotes: DoltHub, S3, GCS, SSH)
```

**Write path.** Every `bd` write command (create, update, close, dep add) executes within a SQL transaction against the local Dolt database, then immediately issues `CALL DOLT_COMMIT(...)` to record the change in Dolt's version history. Each write is one Dolt commit -- providing cell-level version control with full audit trail.

**Read path.** Read operations (`bd ready`, `bd list`, `bd show`) query the local Dolt database directly. The `ready_issues` view uses a recursive CTE to compute transitive blocking, so `bd ready` returns only truly unblocked work in a single query.

### Dolt as Foundation

Dolt is "Git for data" -- a MySQL-compatible SQL database with branching, merging, diffing, and push/pull semantics. Beads uses Dolt because:

- **Cell-level merge.** When two agents modify different fields of the same issue concurrently, Dolt merges them automatically. Traditional databases would require conflict resolution.
- **Native branching.** Agents can work on Dolt branches and merge, just like Git branches for code.
- **Built-in sync.** `dolt push` / `dolt pull` to DoltHub, S3, GCS, or SSH remotes -- no custom sync protocol needed.
- **SQL compatibility.** Standard SQL queries, views, indexes, foreign keys, recursive CTEs. No proprietary query language.

The schema is versioned (`currentSchemaVersion = 6`) with migrations applied on initialization.

---

## 4. Data Model Deep Dive

### The Issue Struct

Beads uses a single `Issue` struct as its universal entity (`internal/types/types.go`, 1234 lines). Every trackable item -- task, bug, feature, epic, message, gate, agent identity, event -- is an Issue with different field combinations populated. The struct contains 60+ fields organized into logical groups:

**Core Identification:**

```go
ID          string // Hash-based: "bd-a3f2dd"
ContentHash string // SHA256 of canonical content
```

**Issue Content:**

```
Title, Description, Design, AcceptanceCriteria, Notes, SpecID
```

**Status and Workflow:**

```
Status    // open, in_progress, blocked, deferred, closed, pinned, hooked
Priority  // 0 (critical) through 4 (backlog)
IssueType // bug, feature, task, epic, chore, decision, message, molecule
```

**Compaction Metadata:**

```
CompactionLevel   // 0 = uncompacted, 1 = tier 1, 2 = tier 2
CompactedAt       // When compaction occurred
CompactedAtCommit // Git commit hash at compaction time
OriginalSize      // Pre-compaction byte count
```

**Agent Identity Fields:**

```
HookBead, RoleBead, AgentState (idle|spawning|running|working|stuck|done|stopped|dead),
LastActivity, RoleType, Rig
```

**Gate Fields (async coordination):**

```
AwaitType  // gh:pr, gh:run, timer, human, mail
AwaitID    // External identifier (PR number, run ID)
Timeout    // Max wait before escalation
Waiters    // Notification addresses
```

**Molecule and Work Fields:**

```
MolType    // swarm, patrol, work
WorkType   // mutex, open_competition
```

**HOP Fields (entity tracking for CV chains):**

```
Creator, Validations, QualityScore (0.0-1.0), Crystallizes (compounds vs evaporates)
```

**Bonding, Slot, Event, and Scheduling Fields:**

```
BondedFrom        // Compound molecule lineage
Holder            // Exclusive slot access (empty = available)
EventKind, Actor, Target, Payload  // Operational event fields
DueAt, DeferUntil // Time-based scheduling (GH#820)
SourceFormula, SourceLocation      // Formula cooking origin tracing
Metadata          // Arbitrary JSON extension point (validated on create/update)
```

### Status Values

| Status | Meaning |
|--------|---------|
| `open` | Ready or waiting for assignment |
| `in_progress` | Actively being worked |
| `blocked` | Explicitly marked blocked |
| `deferred` | Deliberately postponed |
| `closed` | Complete |
| `pinned` | Persistent context marker, stays open indefinitely |
| `hooked` | Attached to an agent's work hook |

### Issue Types

Core built-in types: `bug`, `feature`, `task`, `epic`, `chore`, `decision`, `message`, `molecule`. Gas Town types (`gate`, `convoy`, `merge-request`, `slot`, `agent`, `role`, `rig`) were removed from beads core and are now purely custom types configured via `types.custom` in config. The `event` type was originally a Gas Town type but was promoted to a built-in internal type for audit trail beads (`internal/types/types.go:436-446`).

### 19 Dependency Types

Dependencies are stored in a separate `dependencies` table as directed edges. The 19 built-in types are organized into six categories:

**Workflow types (affect ready work calculation):**

| Type | Semantics |
|------|-----------|
| `blocks` | B cannot start until A closes |
| `parent-child` | Children blocked when parent blocked |
| `conditional-blocks` | B runs only if A fails |
| `waits-for` | Fanout gate -- wait for all dynamic children |

**Association types:**

| Type | Semantics |
|------|-----------|
| `related` | Informational link |
| `discovered-from` | Found during work on parent |

**Graph link types:**

| Type | Semantics |
|------|-----------|
| `replies-to` | Conversation threading |
| `relates-to` | Loose knowledge graph edges |
| `duplicates` | Deduplication link |
| `supersedes` | Version chain link |

**Entity types (HOP foundation):**

| Type | Semantics |
|------|-----------|
| `authored-by` | Creator relationship |
| `assigned-to` | Assignment relationship |
| `approved-by` | Approval relationship |
| `attests` | Skill attestation |

**Cross-project and tracking:**

| Type | Semantics |
|------|-----------|
| `tracks` | Convoy-to-issue tracking (non-blocking) |

**Reference types (cross-referencing without blocking):**

| Type | Semantics |
|------|-----------|
| `until` | Active until target closes (e.g., muted until issue resolved) |
| `caused-by` | Triggered by target (audit trail) |
| `validates` | Approval/validation relationship |

**Delegation types:**

| Type | Semantics |
|------|-----------|
| `delegated-from` | Work delegated from parent; completion cascades up |

Source: `internal/types/types.go:675-711`

### Dolt Schema v6

The database schema (`internal/storage/dolt/schema.go`) defines 15 persistent tables, plus 5 `dolt_ignore`d wisp tables created via migrations (see Wisps section below).

**Persistent tables (15):**

| Table | Purpose |
|-------|---------|
| `issues` | Primary entity table (60+ columns) |
| `dependencies` | Edge table (issue_id, depends_on_id, type) |
| `labels` | Issue labels (many-to-many) |
| `comments` | Threaded comments on issues |
| `events` | Audit trail (status changes, field updates) |
| `config` | Key-value configuration |
| `metadata` | Key-value metadata |
| `child_counters` | Hierarchical ID generation |
| `issue_snapshots` | Pre-compaction content preservation |
| `compaction_snapshots` | Compaction result storage |
| `repo_mtimes` | Multi-repo file tracking |
| `routes` | Prefix-to-path routing |
| `issue_counter` | Sequential ID generation |
| `interactions` | Agent audit log (LLM calls, tool use) |
| `federation_peers` | Peer-to-peer sync credentials |

**Key views:**

- `ready_issues` -- Recursive CTE that computes transitive blocking through `blocks` and `parent-child` dependencies, respecting `defer_until` timestamps. Uses `LEFT JOIN` instead of `NOT EXISTS` to work around a Dolt `mergeJoinIter` panic.
- `blocked_issues` -- Issues with at least one open blocker, with blocker count.

**Key indexes:** Status, priority, issue_type, assignee, created_at, spec_id, external_ref on `issues`; issue_id, depends_on_id, thread_id on `dependencies`.

### Comparison to Our Property Graph Model

Beads encodes everything as rows in a relational database, while Context Manager uses a property graph with typed nodes and edges.

The Beads `Issue` struct serves as a "God object" -- a single type that represents tasks, bugs, epics, messages, gates, agents, events, and molecules through different field combinations. This simplifies the storage layer (one table) but sacrifices type safety and makes the data model harder to reason about.

Context Manager's approach of distinct node types (`Message`, `CompactedMessage`, `Requirement`, `WorkItem`, `ToolCall`, `ToolResult`, `Artifact`, `SystemDirective`, `Rating`) enables type-safe graph traversal and makes the data model self-documenting.

---

## 5. Key Mechanisms

### Hash-Based ID Generation

Beads generates deterministic, content-based IDs using SHA256 (`internal/types/id_generator.go`):

```go
func GenerateHashID(prefix, title, description string,
    created time.Time, workspaceID string) string {
    h := sha256.New()
    h.Write([]byte(title))
    h.Write([]byte(description))
    h.Write([]byte(created.Format(time.RFC3339Nano)))
    h.Write([]byte(workspaceID))
    return hex.EncodeToString(h.Sum(nil))
}
```

**Progressive collision resolution.** The caller takes `hash[:6]` initially (e.g., `bd-a3f2dd`), extending to `hash[:7]` or `hash[:8]` on collision. With 6 hex chars (24 bits), collision probability is ~2.94% at 1,000 issues. 97% of issues stay at 6 chars.

**Hierarchical IDs.** Children use dot notation: `bd-a3f8.1` (first child), `bd-a3f8.1.2` (grandchild). Maximum depth: 3 levels.

**Why this matters.** Hash-based IDs mean two agents can independently create issues on separate branches and merge without ID conflicts. This is critical for multi-agent workflows where no central ID allocator exists.

### Ready Work Computation

The `ready_issues` view (`internal/storage/dolt/schema.go:271-309`) implements a three-phase algorithm:

**Phase 1 -- Direct blocking.** Find issues directly blocked by an open `blocks` dependency:

```sql
blocked_directly AS (
    SELECT DISTINCT d.issue_id
    FROM dependencies d
    WHERE d.type = 'blocks'
      AND EXISTS (
        SELECT 1 FROM issues blocker
        WHERE blocker.id = d.depends_on_id
          AND blocker.status NOT IN ('closed', 'pinned')
      )
)
```

**Phase 2 -- Transitive blocking.** Propagate blocking through `parent-child` edges up to depth 50:

```sql
blocked_transitively AS (
    SELECT issue_id, 0 as depth FROM blocked_directly
    UNION ALL
    SELECT d.issue_id, bt.depth + 1
    FROM blocked_transitively bt
    JOIN dependencies d ON d.depends_on_id = bt.issue_id
    WHERE d.type = 'parent-child' AND bt.depth < 50
)
```

**Phase 3 -- Filter.** Return open, non-ephemeral issues that are not in the blocked set and are not deferred past the current time.

The Go-side `computeBlockedIDs` function extends this with `waits-for` gate evaluation and `conditional-blocks` semantics that cannot be expressed in pure SQL.

### Compaction / Memory Decay

Beads implements tiered AI compaction (`internal/compact/compactor.go`, `internal/storage/dolt/compact.go`) to reduce the context window footprint of old closed issues.

**Tier 1 (30 days).** Closed issues older than 30 days with `compaction_level=0` are eligible. The compactor:

1. Checks eligibility (closed, age threshold, not already compacted)
2. Calculates original size: `len(Description) + len(Design) + len(Notes) + len(AcceptanceCriteria)`
3. Sends content to Claude Haiku via the Anthropic API with a structured prompt
4. Verifies the summary is shorter than the original (rejects expansions)
5. Replaces Description with summary; clears Design, Notes, and AcceptanceCriteria
6. Records compaction metadata: level, timestamp, git commit hash, original size

**Tier 2 (90 days).** Issues already at `compaction_level=1` that were closed 90+ days ago. Must have been tier-1 compacted first.

**What is preserved vs. summarized:**

| Preserved | Summarized/Cleared |
|-----------|--------------------|
| Title | Description (replaced with AI summary) |
| ID and all edges | Design (cleared) |
| Status, priority, timestamps | Notes (cleared) |
| Dependencies graph | Acceptance criteria (cleared) |
| Labels | Comments (archived to snapshot) |
| Compaction metadata | Event history (archived) |

**Compaction prompt** (`internal/compact/haiku.go:264-291`):

```
You are summarizing a closed software issue for long-term storage.
Your goal is to COMPRESS the content...

Provide a summary in this exact format:
**Summary:** [2-3 concise sentences]
**Key Decisions:** [Brief bullet points]
**Resolution:** [One sentence on final outcome]
```

**Batch compaction.** `CompactTier1Batch` processes multiple issues concurrently with configurable parallelism (default 5 workers), using a semaphore pattern.

**Configuration defaults** (`internal/storage/dolt/schema.go:250-261`):

```
compaction_enabled = false (opt-in)
auto_compact_enabled = false
compact_tier1_days = 30
compact_tier1_dep_levels = 2
compact_tier2_days = 90
compact_tier2_dep_levels = 5
compact_tier2_commits = 100
compact_batch_size = 50
compact_parallel_workers = 5
```

### Molecules and Formulas

The molecule/formula system is Beads' workflow templating engine.

**Molecules** are epics with execution semantics. An agent picks up a molecule (parent issue), executes ready children in parallel, and works through the dependency graph until all children close. Key molecule types:

- **Work molecules** -- Standard feature/bug workflows
- **Patrol molecules** -- Recurring operational tasks (ephemeral wisps)
- **Swarm molecules** -- Multi-agent coordination

**Formulas** (`internal/formula/types.go`) are JSON/TOML templates that compile into issue hierarchies. A formula defines:

- **Variables** with defaults, validation, enums, and patterns
- **Steps** that become issues with titles, descriptions, priorities, and inter-step dependencies
- **Compose rules** for bonding formulas together (bond points, hooks, expand/map rules)
- **Advice rules** for AOP-style step transformations (before/after/around)
- **Conditions** for optional steps based on variable values (`"{{var}}"`, `"!{{var}}"`, `"{{var}} == value"`)
- **Loops** for iteration (`count`, `until` with max, `range` with expression evaluator supporting `+`, `-`, `*`, `/`, `^` and variable substitution)
- **Gates** for async wait conditions (GitHub PR merge, CI pass, timer, human approval)
- **OnComplete / for-each** for runtime expansion over step output (bond a formula per item in a collection)
- **Pointcuts** for aspect formulas (glob, type, and label matchers that target steps for advice application)

**Phase metaphor:**

| Phase | Name | Storage | Synced | Purpose |
|-------|------|---------|--------|---------|
| Solid | Proto | .beads/ | Yes | Frozen template |
| Liquid | Mol (pour) | .beads/ | Yes | Active persistent work |
| Vapor | Wisp | .beads/ (dolt_ignore) | No | Ephemeral operations |

**Pour vs. Wisp mode.** `bd mol pour` creates persistent child issues for each step (full audit trail, checkpoint recovery). `bd mol wisp` creates ephemeral issues in the `wisps` table (excluded from push/pull via `dolt_ignore`). Wisps are for routine operations where step-level tracking is not worth the database overhead.

### Cycle Detection

Cycle detection operates at two levels:

**Write-time prevention** (`internal/storage/dolt/dependencies.go:94-116`). When adding a `blocks` dependency, a recursive CTE checks whether the target can already reach the source through existing blocking edges, with a depth limit of 100. The CTE unions both `dependencies` and `wisp_dependencies` tables to detect cross-table cycles:

```sql
WITH RECURSIVE reachable AS (
    SELECT ? AS node, 0 AS depth
    UNION ALL
    SELECT d.depends_on_id, r.depth + 1
    FROM reachable r
    JOIN (
        SELECT issue_id, depends_on_id FROM dependencies WHERE type = 'blocks'
        UNION ALL
        SELECT issue_id, depends_on_id FROM wisp_dependencies WHERE type = 'blocks'
    ) d ON d.issue_id = r.node
    WHERE r.depth < 100
)
SELECT COUNT(*) FROM reachable WHERE node = ?
```

**Post-hoc detection** (`DetectCycles` in `dependencies.go:785-869`). DFS with recursion stack tracking across both dependency tables. Reports full cycle paths with issue details.

### Wisps (Ephemeral Beads)

Wisps are ephemeral issues stored in `dolt_ignore`d tables that Dolt tracks locally but excludes from push/pull. The wisp table family mirrors the main schema: `wisps` (same columns as `issues`), `wisp_labels`, `wisp_dependencies`, `wisp_events`, and `wisp_comments`. The `dolt_ignore` patterns (`wisps` and `wisp_%`) are registered before table creation via migration 004, ensuring these tables never enter Dolt commit history.

Source: `internal/storage/dolt/wisps.go:17-19`, `internal/storage/dolt/migrations/004_wisps_table.go`

Wisps serve as temporary scratchpads for:

- Patrol cycles (recurring operational checks)
- One-shot scaffolding operations
- Agent-internal coordination that has no long-term audit value

**WispType categories with TTL assignments** (`internal/types/types.go:575-592`):

| Category | WispType | TTL | Purpose |
|----------|----------|-----|---------|
| High-churn | `heartbeat`, `ping` | 6h | Liveness pings, health check ACKs |
| Operational | `patrol`, `gc_report` | 24h | Patrol cycle reports, GC reports |
| Significant | `recovery`, `error`, `escalation` | 7d | Force-kill, error reports, human escalations |

Wisps have TTL-based lifecycle management. `bd mol wisp gc` garbage collects old closed wisps. Important discoveries during wisp execution can be promoted to persistent molecules via `--pour`.

Cycle detection operates across both `dependencies` and `wisp_dependencies` tables, ensuring that cross-table cycles are caught.

---

## 6. Agent-Optimized CLI Design

Beads provides over 100 commands (including subcommands) organized into semantic groups, designed for agent consumption:

**Core workflow commands:**

| Command | Purpose |
|---------|---------|
| `bd ready --json` | List tasks with no open blockers |
| `bd create "Title" -p 0 --json` | Create a task with priority |
| `bd update <id> --claim --json` | Atomically claim (sets assignee + in_progress) |
| `bd close <id> --reason "Done" --json` | Complete work with reason |
| `bd blocked --json` | Show what is blocked and why |

**Dependency commands:**

| Command | Purpose |
|---------|---------|
| `bd dep add <child> <parent>` | Add dependency edge |
| `bd dep tree <id>` | Visualize dependency tree |
| `bd dep cycles` | Detect circular dependencies |
| `bd graph --html <id>` | Interactive D3.js graph |

**Molecule commands:**

| Command | Purpose |
|---------|---------|
| `bd mol pour <proto>` | Instantiate workflow template |
| `bd mol wisp <proto>` | Ephemeral instantiation |
| `bd mol bond A B` | Connect work graphs |
| `bd mol squash <id>` | Compress to digest |

**Key design patterns for agent compatibility:**

- **`--json` flag on every command.** Structured output for programmatic consumption. Agents parse JSON, not human-readable tables.
- **Atomic operations.** `--claim` sets assignee AND status in one command, preventing race conditions between agents.
- **Batch commit mode.** Each write auto-commits to Dolt history. No manual save/commit step.
- **`last-touched` tracking.** Issues record who last modified them and when, enabling "what changed since my last session?" queries.
- **Non-interactive design.** `bd edit` opens an editor (agents cannot use it). `bd update` with flags is the agent-compatible alternative.
- **Discovered-from links.** `bd create "Bug" --deps discovered-from:bd-123` creates provenance chains.

---

## 7. Federation and Integrations

### Peer-to-Peer Sync

Federation uses Dolt's distributed version control for peer-to-peer synchronization between independent teams ("Gas Towns"). Each town maintains its own database while sharing work items with configured peers.

**Architecture:**

1. Each Gas Town has its own Dolt database
2. `bd federation add-peer` registers a Dolt remote (like `git remote add`)
3. Push/pull operations sync Dolt commits between peers
4. Cell-level three-way merge resolves most conflicts automatically

**Supported endpoints:** DoltHub (`dolthub://org/repo`), S3 (`s3://bucket/path`), GCS (`gs://bucket/path`), local filesystem, HTTPS, SSH, Git SSH.

### Data Sovereignty Tiers

| Tier | Description | Use Case |
|------|-------------|----------|
| T1 | No restrictions | Public data |
| T2 | Organization-level | Regional/company compliance |
| T3 | Pseudonymous | Identifiers removed |
| T4 | Anonymous | Maximum privacy |

Issues track their `SourceSystem` to identify which federated system created them, enabling proper attribution and trust chains.

### Bidirectional Sync

Beads supports bidirectional synchronization with external issue trackers:

- **GitHub Issues** -- Via `gh` CLI integration and external references
- **GitLab Issues** -- Via API adapter
- **Jira** -- Via REST API v3 (search endpoint fixed in v0.56)
- **Linear** -- Via API adapter

External references are stored as `external_ref` fields (e.g., `"gh-9"`, `"jira-ABC"`) on Issue nodes, with dependency edges connecting the synced representations.

---

## 8. Comparison to Context Manager

| Aspect | Beads | Context Manager |
|--------|-------|-----------------|
| Data model | Relational (Dolt SQL) | Property graph (petgraph + Cozo) |
| Primary entity | Issue (60+ fields, single struct) | Typed nodes (Message, WorkItem, ToolCall, etc.) |
| Edge model | 19 DependencyType constants in edge table | ~9 typed edges (responds_to, compacted_from, etc.) |
| Graph operations | Recursive CTEs + Go-side filtering | petgraph algorithms (DFS, topo sort, PageRank) |
| Compaction | Tier-based AI summary of closed tasks | Multi-perspective compaction of conversation context |
| ID system | Deterministic hash (SHA256, progressive collision) | UUID + path references |
| Persistence | Dolt (version-controlled SQL) | Cozo (Datalog graph DB) + sled snapshots |
| Query language | SQL (MySQL-compatible) | Datalog (Cozo) + graph traversal |
| Purpose | Agent work coordination and memory | Multi-agent context orchestration |
| Federation | Native (Dolt remotes, peer-to-peer) | Not yet designed |
| Interface | CLI-first (100+ commands) | TUI-first (ratatui) |
| Language | Go (234K lines) | Rust |
| Compaction model | Single-perspective (summary, key decisions, resolution) | Multi-perspective (topic-specific summaries) |

### What We Can Learn

1. **Hash-based IDs are powerful for multi-agent.** Deterministic IDs from content hash eliminate the need for a central ID allocator. We should consider this for WorkItem and Artifact nodes that may be created concurrently by different agents.

2. **Ready work computation is the killer feature.** `bd ready` is the single most important command -- it answers "what should I work on next?" by traversing the full dependency graph. Our graph traversal for context construction should support equivalent queries.

3. **Compaction must verify size reduction.** Beads rejects compactions that are longer than the original. This simple guard prevents a common failure mode where AI summaries add boilerplate that inflates rather than compresses.

4. **Tiered compaction with configurable thresholds.** The 30/90 day defaults with config overrides are pragmatic. Our MergeTree-inspired compaction should similarly allow per-project tuning.

5. **Gates bridge external systems into the dependency graph.** PR merge gates, CI gates, and timer gates are a pattern we should consider for our WorkItem dependencies.

6. **Wisps (ephemeral entities) are necessary.** Not everything deserves permanent storage. Our graph needs a concept of ephemeral nodes that participate in traversal but are garbage-collected.

### Where We Diverge

1. **Property graph vs. relational.** Beads forces everything through a single Issue struct and SQL table. This makes ad-hoc queries easy but graph operations expensive (recursive CTEs instead of native traversal). Our petgraph + Cozo approach enables native graph algorithms and type-safe traversal.

2. **Multi-perspective compaction vs. single-perspective.** Beads produces one summary per issue. Context Manager's multi-perspective compaction -- where the same conversation compacts differently for different query contexts -- is genuinely novel and addresses a limitation Beads does not attempt to solve.

3. **Context construction vs. work tracking.** Beads answers "what should I work on?" Context Manager answers "what should the model see?" These are complementary but distinct problems. Beads does not construct LLM inputs; Context Manager does not manage work items as a primary concern.

4. **Typed nodes vs. God object.** The Issue struct's 60+ fields make it flexible but opaque. An Issue with `IssueType="gate"` and `AwaitType="gh:pr"` is semantically very different from one with `IssueType="message"` and `Sender="agent-1"`, but they share the same struct. Our typed node model makes these distinctions explicit at the type level.

5. **Background processing scope.** Beads' compaction is opt-in and runs on explicit command (`bd compact`). Context Manager's background processing is continuous and automatic -- closer to MergeTree's always-on merge process.

---

## 9. Red Team / Green Team

### Green Team (Strengths and Validations)

**Validates core assumptions.**

- Independent convergence on compaction as essential for long-running agent work confirms our research findings (doc 01, Topic 3).
- Dependency tracking with 19 semantic edge types validates that rich relationship semantics are necessary, not premature complexity. Our ~9 edge types may actually be conservative.
- Dolt's cell-level merge demonstrates that version-controlled persistence with automatic conflict resolution is viable and valuable for multi-agent workflows.

**Agent-first CLI is the right interface.**

- JSON output, atomic claims, batch commits, and `bd ready` as the primary entry point show what agents actually need. Human-friendly output is secondary.
- The "Land the Plane" protocol (push before session end, file issues for remaining work, hand off context for next session) is a practical pattern for agent session management.

**Rich dependency semantics enable autonomous execution.**

- The `waits-for` gate type enables fan-out/fan-in patterns that are essential for complex workflows.
- `conditional-blocks` (run only if predecessor fails) enables error-handling paths in agent workflows.
- `discovered-from` creates provenance chains that answer "why does this issue exist?"
- Cross-type blocking validation (epics can only block epics, tasks can only block tasks) prevents nonsensical dependency structures.

**Dolt is a genuinely clever choice.**

- Cell-level three-way merge eliminates most multi-agent conflicts without custom CRDT implementation.
- Built-in version history means every issue change is auditable by default.
- Native push/pull to cloud storage (S3, GCS, DoltHub) provides federation without a custom sync protocol.

**Molecule/formula system shows workflow complexity is real.**

- The progression from simple issues to epics to molecules to formulas reflects genuine complexity in agent workflow orchestration.
- Aspect-oriented advice rules, conditional steps, and runtime expansion demonstrate patterns that any serious workflow system will eventually need.

### Red Team (Weaknesses and Risks)

**The Issue struct is a God object.**

- 60+ fields on a single struct means most fields are irrelevant for most issue types. A "message" issue does not need `AwaitType`, `MolType`, `WorkType`, `QualityScore`, etc. This wastes storage, complicates validation, and makes the data model hard to reason about.
- Adding new capabilities requires modifying the universal struct, which touches all serialization, validation, and query code. This is visible in the codebase: many fields have `DEFAULT ''` in SQL because they must exist on every row even when irrelevant.

**Dolt dependency is heavy.**

- Dolt must be running as an external server (`bd dolt start`). This is an operational dependency that agents and developers must manage.
- The binary dropped from 168MB to 41MB by removing the embedded driver, but Dolt itself is a large binary. Total install footprint is significant.
- Dolt's MySQL compatibility has quirks (the `mergeJoinIter` panic workaround, `LEFT JOIN` instead of `NOT EXISTS`, batched `IN` clauses to avoid query planner spikes).

**No multi-perspective compaction.**

- Beads produces a single summary per issue. When an agent needs context about an issue from a security perspective vs. a performance perspective, it gets the same generic summary. This is the gap Context Manager's multi-perspective compaction addresses.

**Relational model limits relationship expressiveness.**

- Graph queries require recursive CTEs with depth limits (50 for ready work, 100 for cycle detection). Native graph databases handle this with constant-time traversal.
- The dependencies table stores a single `type` string per edge. There is no support for edge properties beyond type, metadata JSON, and thread_id. Rich edge attributes (weights, confidence scores, temporal validity) would require schema changes.

**Compaction is destructive.**

- Tier 1 compaction replaces the Description and clears Design, Notes, and AcceptanceCriteria. The original content is preserved in `issue_snapshots`, but the primary issue record is permanently altered. Context Manager's approach of creating new CompactedMessage nodes while preserving originals (linked via `compacted_from` edges) is less destructive.

**Scale concerns.**

- Benchmarks show 30ms for `GetReadyWork` on 10K issues (M2 Pro). At 100K issues, the recursive CTE-based approach may become problematic. Native graph traversal should scale better.
- The `bd ready` view queries the full issues table each time. There is no incremental computation or caching of the ready set (beyond a per-invocation `blockedIDsCache`).

**No conversation context management.**

- Beads tracks work items but does not manage LLM conversation context. It does not construct prompts, manage context windows, or optimize token usage. These are the core problems Context Manager solves.

---

## 10. Sources

### Repository Source Code

- `internal/types/types.go` -- Issue struct (60+ fields), Status/IssueType enums, 19 DependencyType constants, content hash computation
- `internal/types/id_generator.go` -- SHA256 hash-based ID generation, progressive collision resolution, hierarchical IDs
- `internal/storage/dolt/schema.go` -- Dolt SQL schema v6, 15 tables, ready_issues/blocked_issues views, compaction config defaults
- `internal/storage/dolt/dependencies.go` -- Dependency CRUD, cycle detection (recursive CTE + DFS), blocking info, cross-table wisp handling
- `internal/storage/dolt/compact.go` -- Tier eligibility checks, compaction metadata recording, candidate queries
- `internal/compact/compactor.go` -- Compaction orchestration, batch processing with semaphore concurrency
- `internal/compact/haiku.go` -- Anthropic API client, tier 1 prompt template, retry with exponential backoff
- `internal/formula/types.go` -- Formula/Step/VarDef/ComposeRules/AdviceRule/Gate/Loop/OnComplete/Pointcut data model
- `internal/storage/dolt/wisps.go` -- Wisp table routing, dolt_ignore'd table operations
- `internal/storage/dolt/migrations/004_wisps_table.go` -- Wisps table creation with dolt_ignore patterns
- `internal/storage/dolt/migrations/005_wisp_auxiliary_tables.go` -- Wisp auxiliary tables (wisp_labels, wisp_dependencies, wisp_events, wisp_comments)

### Repository Documentation

- [README.md](https://github.com/steveyegge/beads) -- Project overview, features, quick start
- AGENTS.md / AGENT_INSTRUCTIONS.md -- Agent workflow, session management, "land the plane" protocol
- NEWSLETTER.md -- v0.56 "The Great Purge" release notes (embedded removal, binary size reduction)
- BENCHMARKS.md -- Performance benchmarks (30ms ready work on 10K issues)
- FEDERATION-SETUP.md -- Peer-to-peer sync setup, data sovereignty tiers
- docs/MOLECULES.md -- Molecule execution model, bonding, phase metaphor
- docs/DEPENDENCIES.md -- Dependency types, gates, ready work, cycle detection

### Steve Yegge Articles

- [Introducing Beads: A Coding Agent Memory System](https://steve-yegge.medium.com/introducing-beads-a-coding-agent-memory-system-637d7d92514a) -- Origin story, "50 First Dates" problem
- [The Beads Revolution](https://steve-yegge.medium.com/the-beads-revolution-how-i-built-the-todo-system-that-ai-agents-actually-want-to-use-228a5f9be2a9) -- Built in 6 days with Claude, epics, child issues
- [Beads Blows Up](https://steve-yegge.medium.com/beads-blows-up-a0a61bb889b4) -- "Land the plane" protocol, session cleanup
- [Beads Best Practices](https://steve-yegge.medium.com/beads-best-practices-2db636b9760c) -- Multi-agent coordination
- [Beads for Blobfish](https://steve-yegge.medium.com/beads-for-blobfish-80c7a2977ffa) -- Quick start motivation
- [The Future of Coding Agents](https://steve-yegge.medium.com/the-future-of-coding-agents-e9451a84207c) -- Agent maturity, natural adoption
- [Welcome to Gas Town](https://steve-yegge.medium.com/welcome-to-gas-town-4f25ee16dd04) -- Multi-agent system built on Beads

### Community and Discussion

- [From Beads to Tasks: Anthropic Productizes Agent Memory](https://paddo.dev/blog/from-beads-to-tasks/) -- How Beads influenced broader agent memory approaches
- [Beads: Memory for Your Coding Agents](https://paddo.dev/blog/beads-memory-for-coding-agents/) -- Deep dive into Git-backed storage
- [GasTown and the Two Kinds of Multi-Agent](https://paddo.dev/blog/gastown-two-kinds-of-multi-agent/) -- Multi-agent patterns
- [Beads: A Git-Friendly Issue Tracker for AI Coding Agents](https://betterstack.com/community/guides/ai/beads-issue-tracker-ai-agents/) -- Getting started guide
- [An Introduction to Beads](https://ianbull.com/posts/beads) by Ian Bull -- Practical introduction with setup instructions
- [A Day in Gas Town (DoltHub Blog)](https://www.dolthub.com/blog/) -- Dolt perspective on Beads usage
- [HN Discussion: Beads](https://news.ycombinator.com/item?id=46075616) -- Community reception and critique
- [HN Discussion: Beads (second thread)](https://news.ycombinator.com/item?id=45566864) -- Follow-up discussion
- [HN Discussion: Rust port](https://news.ycombinator.com/item?id=46674515) -- Community interest in Rust rewrite
- Software Engineering Daily: "Gas Town, Beads, and the Rise of Agentic Development" -- Podcast interview
