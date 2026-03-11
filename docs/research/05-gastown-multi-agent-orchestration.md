# Gas Town: Multi-Agent Orchestration Prior Art

> Research conducted 2026-03-11. Analysis of Steve Yegge's Gas Town multi-agent orchestration
> system as prior art for Context Manager's agent orchestration design.

---

## 1. Executive Summary

Gas Town is the most sophisticated open-source multi-agent orchestration system for AI coding agents as of March 2026. Built by Steve Yegge (40-year industry veteran, ex-Amazon/Google/Sourcegraph), it coordinates 4-30 Claude Code agents working simultaneously on software development tasks. The system is written in Go, backed by Dolt (a SQL database with Git semantics) and the Beads issue-tracking framework, and uses tmux for session management.

Gas Town is directly relevant to Context Manager's design because it solves the same class of problem we intend to solve: orchestrating multiple AI agents to produce software. Where Gas Town uses a relational/Dolt-backed work-item model with specialized agent roles, Context Manager aims to use a graph-native context model where orchestration emerges from graph structure rather than role hierarchies. Understanding Gas Town's design decisions, trade-offs, and community-validated patterns is essential before we design our own orchestration layer.

**Key numbers:**

- 64 internal Go packages, 951 Go source files, ~199K lines of Go (non-test)
- 8 specialized agent roles across two hierarchy levels
- 2,400+ submitted PRs and 1,500+ merged from 450+ contributors in first two months
- $100-200/hour API burn rate at full scale (20-30 agents)
- Single Dolt SQL server per town, all agents writing to `main` with transaction discipline

---

## 2. Problem Statement and Motivation

### 2.1 Why Multi-Agent?

Individual AI coding agents have hard limits: a single Claude Code session can hold ~200K tokens of context, works on one task at a time, and loses all state when the session dies. For a developer managing a complex project, this means sequential work on tasks that could be parallelized.

Manual multi-agent setups (running 4-10 Claude Code sessions in separate terminal tabs) work but introduce coordination problems:

- **Context loss on restart.** An agent's understanding of what it was doing disappears when the session ends.
- **Work state scattered.** No single view of what is in flight, what is done, what is blocked.
- **Merge conflicts.** Multiple agents editing the same codebase create integration problems.
- **No supervision.** A stuck agent wastes API credits until a human notices.

### 2.2 The Scale Gap

At 4-10 agents, a skilled developer can manually coordinate work through terminal tabs. At 20-30 agents, this becomes impossible. Gas Town's thesis is that this gap requires purpose-built infrastructure: persistent agent identities, structured work decomposition, automated supervision, and a merge queue.

As Yegge wrote in the launch post: "This is my third $200/month Claude Pro Max plan. My first two have maxed out their weekly limits." The cost pressure alone demands that agents not spin idle or duplicate work.

---

## 3. Architecture

### 3.1 Two-Level Beads Architecture

Gas Town uses a two-level work-tracking system built on Beads (a Git-backed issue tracker):

| Level | Location | Prefix | Purpose |
|-------|----------|--------|---------|
| **Town** | `~/gt/.beads/` | `hq-*` | Cross-rig coordination, Mayor mail, agent identity |
| **Rig** | `<rig>/mayor/rig/.beads/` | project prefix | Implementation work, MRs, project issues |

This separation ensures that coordination metadata (who is assigned what, which convoys are in flight) lives at the town level, while implementation details (bugs, features, merge requests) live at the project level.

### 3.2 Single Dolt Server

All beads data is stored in a single Dolt SQL Server process per town. There is no embedded fallback -- if the server is down, `bd` (the beads CLI) fails fast.

```
Dolt SQL Server (per town)
Port 3307, managed by daemon
Data: ~/gt/.dolt-data/
          |
    MySQL protocol
    |         |         |
  USE hq   USE gastown  USE beads  ...
```

All agents write directly to `main` using transaction discipline (`BEGIN` / `DOLT_COMMIT` / `COMMIT` atomically). This eliminates branch proliferation in the database and ensures immediate cross-agent visibility of work state changes.

### 3.3 Directory Structure

```
~/gt/                           Town root
|-- .beads/                     Town-level beads (hq-* prefix)
|-- .dolt-data/                 Centralized Dolt data directory
|   |-- hq/                     Town beads database
|   |-- gastown/                Per-rig databases
|   +-- beads/
|-- daemon/                     Daemon runtime state
|-- deacon/                     Deacon workspace
|   +-- dogs/<name>/            Dog worker directories
|-- mayor/                      Mayor agent home
|-- settings/                   Town-level settings
+-- <rig>/                      Project container
    |-- config.json             Rig identity and beads prefix
    |-- mayor/rig/              Canonical clone (beads source of truth)
    |-- refinery/               Refinery agent home (worktree)
    |-- witness/                Witness agent home (no clone)
    |-- crew/                   Human workspaces (full clones)
    |   +-- <name>/
    +-- polecats/               Worker worktrees
        +-- <name>/<rigname>/
```

Polecats and the Refinery use git worktrees from `mayor/rig/`, not full clones. This enables fast spawning and shared object storage. Crew workspaces use full clones for human developers who need independent repositories.

### 3.4 Worktree Architecture

From `internal/polecat/manager.go`:

```
git worktree add -b polecat/<name>-<timestamp> polecats/<name>
```

Each polecat gets a dedicated git worktree branched from the canonical clone. The branch naming convention (`polecat/<name>-<timestamp>`) enforces scope: a polecat can only push to its own branch prefix, enforced by the proxy server at the pkt-line level.

---

## 4. Role Taxonomy

Gas Town defines 8 agent roles organized into two levels. Each role has a dedicated Go template (`internal/templates/roles/*.md.tmpl`) that provides the agent's identity, responsibilities, and operating instructions via the `gt prime` context injection system.

### 4.1 Town-Level Roles

#### Mayor -- "The Main Drive Shaft"

- **Scope:** Global coordinator sitting above all rigs
- **Responsibilities:** Work dispatch (via `gt sling`), cross-rig coordination, escalation handling, strategic decisions
- **Session:** Persistent, user-initiated via `gt mayor attach`
- **Key metaphor from template:** "Gas Town is a steam engine. You are the main drive shaft."
- **Anti-pattern:** Filing beads "for later" while doing everything yourself. Sling work liberally to polecats.

#### Deacon -- "The Flywheel"

- **Scope:** Daemon beacon, continuous patrol executor
- **Responsibilities:** Keep Mayor and Witnesses alive, process lifecycle requests, run scheduled plugins, escalate unresolvable issues
- **Session:** Persistent, auto-restarted by daemon
- **Patrol molecule:** `mol-deacon-patrol` with 25 steps including heartbeat, inbox check, orphan cleanup, test pollution cleanup, health scan, wisp compaction, log rotation
- **Context management:** Hands off after 20 patrol loops or immediately after any extraordinary action (lifecycle request, agent remediation, escalation)

#### Dogs -- Boot, Reaper, Compactor

Dogs are the Deacon's helpers for narrow infrastructure tasks. They are NOT workers.

| Dog | Purpose | Lifecycle |
|-----|---------|-----------|
| **Boot** | Triages Deacon health on daemon tick. Fresh each tick -- makes single decision: should Deacon wake? | Ephemeral, spawned every 3 minutes |
| **Reaper** | Deletes stale bead rows older than 7-30 days | Short-lived, triggered by patrol |
| **Compactor** | Rebases old bead commits together | Short-lived, triggered by patrol |

The Boot dog is architecturally notable. The daemon (Go process) cannot reason about whether the Deacon is stuck vs. merely thinking (ZFC principle). Boot bridges this gap: a fresh AI session every 3 minutes that makes a single triage decision, then exits. This is cheaper than keeping a full AI agent running continuously in idle towns.

### 4.2 Rig-Level Roles

#### Polecat -- "The Piston"

- **Scope:** Worker agent with persistent identity but ephemeral sessions
- **Responsibilities:** Execute assigned issues, push branch, submit to merge queue via `gt done`
- **Lifecycle states** (from `internal/polecat/types.go`):
    - `working` -- session active, doing assigned work
    - `idle` -- work completed, session killed, sandbox preserved for reuse
    - `done` -- transient state after `gt done`, should exit immediately (if stuck here: "zombie")
    - `stuck` -- explicitly signaled need for assistance
    - `zombie` -- tmux session exists but no corresponding worktree
    - Note: "stalled" (session stopped unexpectedly, never nudged back) is a *detected condition*, not a stored state. The Witness infers it from tmux state and age.
- **Session-per-step model:** Each molecule step gets one polecat session. The sandbox (branch, worktree) persists across sessions. "Sessions are the pistons; sandboxes are the cylinders."
- **Communication budget:** 0-1 mail messages per session. Use `gt nudge` (ephemeral, zero Dolt overhead) for everything else.
- **Cleanup safety** (from `internal/polecat/types.go`): The Witness checks `CleanupStatus` before nuking a worktree: `clean` (safe to remove), `has_uncommitted`, `has_stash`, `has_unpushed` (require recovery), or `unknown`. Only `clean` status allows removal; uncommitted changes can be force-removed but stashes and unpushed commits cannot.

#### Witness -- "The Pressure Gauge"

- **Scope:** Per-rig polecat lifecycle manager
- **Responsibilities:** Monitor polecat health, process lifecycle events (POLECAT_DONE, LIFECYCLE:Shutdown), nudge stuck workers, escalate to Mayor
- **What it never does:** Write code, spawn polecats, close issues for work it did not do
- **Patrol molecule:** `mol-witness-patrol` with steps for inbox check, polecat health scan, cleanup processing
- **Swim Lane Rule:** May ONLY close wisps that the witness itself created. Foreign wisp closure kills active polecat work.
- **Protocol messages handled** (from `internal/witness/protocol.go`):
    - `POLECAT_DONE` -- polecat signaling work completion
    - `LIFECYCLE:Shutdown` -- daemon-triggered polecat shutdown
    - `HELP:` -- polecat requesting intervention
    - `MERGED` -- refinery confirms branch merged
    - `MERGE_FAILED` -- refinery reporting merge failure
    - `MERGE_READY` -- witness notifying refinery work is ready
    - `SWARM_START` -- mayor initiating batch work
    - `DISPATCH_ATTEMPT/OK/FAIL` -- dispatch lifecycle tracking
    - `IDLE_PASSIVATED` -- polecat passivated after idle timeout

#### Refinery -- "The Gearbox"

- **Scope:** Per-rig merge queue processor
- **Responsibilities:** Process MRs through sequential rebase, run tests, merge to target branch
- **Cardinal rule:** "You are a merge processor, NOT a developer." Never reads polecat implementation code.
- **Merge strategy:** Batch-then-bisect (Bors-style):

```
MRs waiting:  [A, B, C, D]
                    |
Batch:        Rebase A..D as a stack on main
                    |
Test tip:     Run tests on D (tip of stack)
                    |
If PASS:      Fast-forward merge all 4
If FAIL:      Binary bisect to find the breaker
```

- **Event-driven protocol:**

```
Polecat completes --> POLECAT_DONE --> Witness
                                        |
                                MERGE_READY --> Refinery
                                        |
                                Process MR, merge to target branch
                                        |
                                     MERGED --> Witness
                                        |
                                Witness cleans up polecat
```

#### Crew

- **Scope:** Long-lived, user-managed workspaces
- **Lifecycle:** Persistent, full git clones, no automated supervision
- **Use cases:** Exploratory work, long-running projects, work requiring human judgment

---

## 5. Key Concepts

### 5.1 MEOW -- Molecular Expression of Work

Breaking large goals into detailed instructions for agents. MEOW is the decomposition strategy, supported by:

- **Beads** -- atomic work units stored in Dolt
- **Molecules** -- durable chained bead workflows where each step is tracked
- **Formulas** -- TOML-defined reusable workflow templates (e.g., `release.formula.toml` with `bump-version`, `run-tests`, `build`, `create-tag`, `publish` steps)
- **Protomolecules** -- template classes for instantiating molecules

### 5.2 GUPP -- Gas Town Universal Propulsion Principle

> "If there is work on your Hook, YOU MUST RUN IT."

This is the core operational principle that appears in every role template. GUPP ensures agents autonomously proceed with available work without waiting for external input. Every role template opens with a "Theory of Operation" section emphasizing this principle with a steam engine metaphor specific to the role.

The failure mode GUPP prevents: Agent restarts, announces itself, waits for human confirmation, human is AFK, downstream agents wait, system stops.

### 5.3 NDI -- Nondeterministic Idempotence

The overarching durability guarantee. Unlike Temporal's deterministic replay, Gas Town achieves workflow durability through:

- Work expressed as molecules (chained bead sequences) stored in Dolt
- Each step has clear acceptance criteria
- If an agent crashes mid-step, the next session discovers position via `gt prime --hook` and `bd mol current`
- No explicit "handoff payload" needed -- the beads state IS the handoff

State continuity between sessions is maintained through:

- **Git state:** Commits, staged changes, branch position
- **Beads state:** Molecule progress (which steps are closed)
- **Hook state:** `hook_bead` on agent bead persists across sessions
- **Agent bead:** `agent_state`, `cleanup_status`, `hook_bead` fields

### 5.4 Hooks -- Pinned Beads Per Agent

A Hook is a special pinned bead for each agent representing its current assignment. When work appears on your hook, GUPP dictates you execute it immediately. Hooks are the primary dispatch mechanism:

```bash
gt sling <bead-id> <rig>    # Spawns polecat, hooks work, starts session
gt hook                     # Check your hooked work
```

### 5.5 Claude Code Hooks Management

Distinct from Hooks as a work dispatch concept (Section 5.4), Gas Town has a centralized Claude Code hooks management system (`internal/hooks/`) that generates `.claude/settings.json` files for every agent workspace.

**Architecture** (from `docs/HOOKS.md` and `internal/hooks/config.go`):

```
~/.gt/hooks-base.json              Base config (all agents)
~/.gt/hooks-overrides/
  crew.json                        Override for all crew workers
  witness.json                     Override for all witnesses
  gastown__crew.json               Override for gastown crew specifically
```

Merge strategy: `base -> role -> rig+role` (more specific wins). Settings are installed in gastown-managed parent directories and passed to Claude Code via the `--settings` flag, keeping customer repos clean.

**Default base hooks** enforce safety and context injection across all agents:

| Event | Hook | Purpose |
|-------|------|---------|
| `SessionStart` | `gt prime --hook` | Inject role template and hooked work on every session start |
| `PreCompact` | `gt prime --hook` | Re-inject context when compaction triggers |
| `UserPromptSubmit` | `gt mail check --inject` | Check for incoming mail on every prompt |
| `Stop` | `gt costs record` | Record API cost data on session end |
| `PreToolUse` | `gt tap guard pr-workflow` | Block `gh pr create`, `git checkout -b`, `git switch -c` (polecat safety) |
| `PreToolUse` | `gt tap guard dangerous-command` | Block `rm -rf /`, `git push --force` |

**Role-specific overrides** (built-in, from `DefaultOverrides()`):

- **Crew**: `PreCompact` replaced with `gt handoff --cycle --reason compaction` -- instead of lossy context compaction, crew workers cycle to a fresh session that inherits hooked work.
- **Witness/Deacon/Refinery**: `PreToolUse` blocks `bd mol pour` for patrol formulas -- patrols MUST use wisps (`bd mol wisp`), not persistent molecules, to prevent unbounded accumulation.

The hooks system also supports multiple AI editors beyond Claude Code, with templates for Copilot, Cursor, Gemini, and others (from `internal/hooks/templates/`).

### 5.6 Convoys -- Grouped Beads Across Rigs

Convoys are the primary work-order abstraction, bundling related beads for batch tracking:

```bash
gt convoy create "Feature X" gt-abc12 gt-def34 --notify --human
gt convoy list              # Dashboard of active convoys
gt convoy status hq-cv-abc  # Detailed progress
```

A convoy tracks: which beads are in flight, which agents are assigned (the "swarm"), completion status, and auto-notification when work lands. Convoys provide the single-pane view of "what is happening" that becomes essential at 20+ agents.

Convoys use event-driven feeding (`internal/convoy/operations.go`): when an issue closes, `CheckConvoysForIssue` finds tracking convoys and reactively dispatches the next ready issue (open, unassigned, unblocked, slingable type). Blocking dependency types that prevent dispatch are: `blocks`, `conditional-blocks`, `waits-for`, and `merge-blocks`. Notably, `parent-child` is NOT blocking -- a child task is dispatchable even if its parent epic is open. For `merge-blocks`, the blocker must have a close reason starting with "Merged in " to confirm code was actually integrated.

### 5.7 Wisps -- Ephemeral Beads with TTL

Wisps are lightweight, ephemeral beads used for transient operations that do not need permanent tracking. Patrol cycles, plugin runs, and cleanup tasks use wisps. This keeps the permanent beads database clean while still providing structured tracking for operational work.

The data plane lifecycle for all beads:

```
CREATE --> LIVE --> CLOSE --> DECAY --> COMPACT --> FLATTEN
  |         |        |         |         |           |
  Dolt    active   done    DELETE     REBASE      SQUASH
  commit   work    bead    rows >7d   commits     all history
                                      together    to 1 commit
```

### 5.8 The Capability Ledger

Every agent maintains a permanent work history as beads. Every completion is recorded. Every handoff is logged. This creates:

- **Attribution:** All work attributed to the specific agent that performed it (git commits include agent identity)
- **Track record:** Which agents are reliable, which need tuning
- **A/B testing:** Deploy different models on similar tasks and compare outcomes via `bd stats`
- **Portable reputation:** Foundation for the Wasteland federation (see Section 12)

---

## 6. Agent Lifecycle and Communication

### 6.1 Polecat Lifecycle

The polecat lifecycle follows a sling-to-cleanup flow:

```
gt sling <bead> <rig>
    |
    +-- Create worktree (git worktree add -b polecat/<name>-<ts>)
    +-- Hook bead to agent
    +-- Start tmux session
    +-- gt prime injects role context
    |
    v
WORKING (gt hook --> read bead --> execute steps)
    |
    +-- Push branch
    +-- gt done (submits to merge queue)
    |
    v
DONE (transient)
    |
    +-- Session killed by Witness
    +-- Worktree preserved for reuse
    |
    v
IDLE (ready for next assignment)
```

**Session cycling vs. step cycling** are distinct:

| Concept | Trigger | What Changes | What Persists |
|---------|---------|-------------|---------------|
| **Session cycle** | Handoff, compaction, crash | Claude context window | Branch, worktree, molecule state |
| **Step cycle** | Step bead closed | Current step focus | Branch, worktree, remaining steps |

A single step may span multiple sessions (if complex). Multiple steps may fit in a single session (if small). The session-per-step model is a design target, not a hard constraint.

### 6.2 Mail System

Inter-agent communication uses `type=message` beads routed through the beads system. There are three communication primitives:

| Mechanism | Persistence | Cost | Use Case |
|-----------|------------|------|----------|
| **Mail** (`gt mail send`) | Permanent bead + Dolt commit | High | Protocol messages (POLECAT_DONE, MERGE_READY, etc.), escalations |
| **Nudge** (`gt nudge`) | Ephemeral, session-scoped | Zero | Health checks, wake signals, status requests |
| **Handoff** (`gt handoff`) | Mail + new session | Medium | Context transfer between sessions |

Mail messages support three routing modes (from `internal/mail/types.go`): **Direct** (addressed to a specific agent via `To`), **Queue** (first-come-first-served work claiming), and **Channel** (broadcast to all readers). These are mutually exclusive. Each message also carries a delivery mode: **Queue** (agent checks periodically via `gt mail check`) or **Interrupt** (injected directly into the agent's session as a system-reminder for lifecycle events or urgent matters).

Priority levels are: `urgent` (P0), `high` (P1), `normal` (P2, default), `low` (P3). Message types are: `task` (requires action), `scavenge` (optional first-come-first-served), `notification` (informational, default), `reply` (response to another message). Messages support threading via `thread_id`, CC recipients, and two-phase delivery with acknowledgment tracking.

**Communication hygiene** is enforced through role templates:

- Polecats: 0-1 mail messages per session. Use nudge for everything else.
- Witnesses: Nudge first, mail rarely. "If the recipient dies and restarts, do they need this message?"
- Deacon: Dogs should NEVER receive mail. "Every `gt mail send` creates a Dolt commit in the permanent history."

### 6.3 Protocol Message Types

The Witness inbox routing handles structured protocol messages (from `internal/witness/protocol.go`):

| Message | Route | Purpose |
|---------|-------|---------|
| `POLECAT_DONE <name>` | Polecat --> Witness | Signal work completion (with exit type: COMPLETED, ESCALATED, DEFERRED, PHASE_COMPLETE) |
| `MERGE_READY <name>` | Witness --> Refinery | Branch ready for merge queue |
| `MERGED <name>` | Refinery --> Witness | Confirm branch merged |
| `MERGE_FAILED <name>` | Refinery --> Witness | Merge attempt failed (with failure type and error) |
| `LIFECYCLE:Shutdown <name>` | Daemon --> Witness | Triggered polecat shutdown |
| `HELP: <topic>` | Polecat --> Witness | Requesting intervention (auto-classified by severity) |
| `HANDOFF` | Agent --> Self | Session continuity: context transfer between sessions |
| `SWARM_START` | Mayor --> Witness | Batch work initiated (with swarm ID and bead list) |
| `DISPATCH_ATTEMPT <name>` | Witness --> Witness | Witness attempting to dispatch polecat to bead |
| `DISPATCH_OK <name>` | Witness --> Witness | Dispatch succeeded |
| `DISPATCH_FAIL <name>` | Witness --> Witness | Dispatch failed (with reason) |
| `IDLE_PASSIVATED <name>` | Witness --> Witness | Polecat passivated after idle timeout |

Each message type has a structured body format with typed payloads parsed by the witness protocol package (`internal/witness/protocol.go`). Help requests are automatically classified by keyword matching into categories with default escalation routing:

| Category | Severity | Routes To | Trigger Keywords |
|----------|----------|-----------|------------------|
| Emergency | Critical | Overseer (human) | security, vulnerability, data corruption |
| Failed | High | Deacon | crash, panic, OOM, database error |
| Blocked | High | Mayor | blocked, merge conflict, deadlock |
| Decision | Medium | Deacon | architecture, design choice, ambiguous |
| Lifecycle | Medium | Witness | session, zombie, timeout, no progress |

### 6.4 Session Continuity via Handoffs

When a session's context fills up or the agent needs a fresh start:

1. Agent runs `gt handoff -s "Subject" -m "Context notes"`
2. This creates a mail bead addressed to the agent's own identity
3. Current session ends
4. Witness or daemon respawns a new session
5. New session runs `gt prime`, discovers hook, reads handoff mail
6. Work continues from where the previous session left off

No explicit "handoff payload" is needed because beads state, git state, and the hook bead together reconstruct the agent's position.

---

## 7. Merge Queue and Sandboxed Execution

### 7.1 Batch-then-Bisect Merge Queue

The Refinery implements a Bors-style batch-then-bisect merge queue. This is a core capability, not a pluggable strategy.

**Sequential rebase protocol** (critical):

```
WRONG (parallel merge - causes conflicts):
  main ----------------------------------------+
    |-- branch-A (based on old main)           |-- CONFLICTS
    +-- branch-B (based on old main)           |

RIGHT (sequential rebase):
  main ---------+--------+------> (clean history)
                |        |
           merge A   merge B
                |        |
           A rebased  B rebased
           on main    on main+A
```

After every merge, the target branch moves. The next branch MUST rebase on the new baseline.

**Implementation phases** (from `internal/refinery/engineer.go`):

| Phase | Description | Status |
|-------|-------------|--------|
| 1: GatesParallel | Run test + lint concurrently per MR | Implemented, enabled by default |
| 2: Batch-then-bisect | Bors-style batching with binary bisect | In progress |
| 3: Pre-verification | Polecats run tests before MR submission, skip gates if base matches | In progress (fast-path check exists) |

The refinery also implements MR priority scoring (`internal/refinery/score.go`): a composite score based on convoy age (prevent starvation), issue priority (P0 gets +400 points), retry penalty (capped at 300 to prevent permanent deprioritization), and MR age (FIFO tiebreaker). Higher scores are processed first.

### 7.2 Proxy Server -- Sandboxed Execution

Gas Town includes a proxy server system (`gt-proxy-server` and `gt-proxy-client`) for running polecats in containers or isolated environments.

**Security model:**

| Layer | Enforcement |
|-------|-------------|
| Transport | TLS 1.3 minimum, all traffic encrypted |
| Server identity | Server cert signed by shared CA |
| Client identity | mTLS -- client cert CN format `gt-<rig>-<name>` |
| Exec allowlist | Only `gt` and `bd` commands permitted |
| Subcommand allowlist | Per-command filtering (e.g., polecats cannot run `gt install`) |
| Branch scope | Polecat can only push to `refs/heads/polecat/<name>-*` |
| Path traversal | Rig names validated against `[a-zA-Z0-9_-]+` |
| Rate limiting | 10 req/s per client, burst 20, 32 max concurrent subprocesses |
| Certificate revocation | In-memory deny list, checked at TLS handshake via local admin API |
| Identity injection | Server derives identity from cert CN, cannot be overridden by client |
| Body size limits | `/v1/exec` capped at 1 MiB; receive-pack ref list capped at 32 MiB |
| Env isolation | Subprocesses only see `HOME` and `PATH`; no `GITHUB_TOKEN` or credentials leak |

This is a production-grade sandboxing system. The proxy auto-discovers safe subcommands via `gt proxy-subcmds`, so upgrading `gt` on the host automatically propagates allowed commands to the proxy.

---

## 8. Monitoring and Observability

### 8.1 TUI Feed

`gt feed` launches a three-panel TUI dashboard:

- **Agent Tree** -- Hierarchical view of all agents grouped by rig and role
- **Convoy Panel** -- In-progress and recently-landed convoys
- **Event Stream** -- Chronological feed of creates, completions, slings, nudges

Navigation: `j`/`k` scroll, `Tab` switch panels, `p` toggle problems view, `n` nudge agent, `h` handoff.

### 8.2 Problems View

The problems view groups agents by health state for intervention at scale (20-50+ agents):

| State | Condition |
|-------|-----------|
| **GUPP Violation** | Hooked work with no progress for extended period |
| **Stalled** | Hooked work with reduced progress |
| **Zombie** | Dead tmux session with no corresponding worktree |
| **Working** | Active, progressing normally |
| **Idle** | No hooked work |

### 8.3 Web Dashboard

An htmx-based web dashboard (`gt dashboard`) provides a single-page overview: agents, convoys, hooks, queues, issues, escalations. Auto-refreshes, includes a command palette for running `gt` commands from the browser.

### 8.4 Watchdog Chain

Three-tier autonomous health monitoring:

```
Daemon (Go process)          <-- Dumb transport, 3-min heartbeat
    |
    +-> Boot (AI agent)       <-- Intelligent triage, fresh each tick
            |
            +-> Deacon (AI agent)  <-- Continuous patrol, long-running
                    |
                    +-> Witnesses and Refineries  <-- Per-rig agents
```

The daemon is mechanical (Go code, cannot reason about agent state per ZFC). Boot bridges the intelligence gap: a fresh AI agent every 3 minutes that makes a single triage decision about whether Deacon should be woken. This gives intelligent triage without the cost of keeping a full AI running in idle towns.

---

## 9. Design Philosophy

### 9.1 Zero Framework Cognition (ZFC)

> "Go provides transport. Agents provide cognition."

ZFC is Gas Town's architectural principle: Go code handles plumbing (tmux sessions, message delivery, hooks, nudges, file transport, observability primitives), while all reasoning, judgment calls, and decision-making happen in the AI agents via molecule formulas and role templates.

Practical implications:

- **No hardcoded thresholds in Go.** Do not write `if age > 5*time.Minute` to decide if an agent is stuck. Expose the age as data and let the agent decide.
- **No heuristics in Go.** Do not write detection logic that pattern-matches agent behavior. Give agents the tools to observe, and let them reason.
- **Formulas over subcommands.** If the feature is "detect X and do Y," it should be a molecule step, not a new `gt` subcommand.

**The test:** Before adding Go code, ask -- "Am I adding transport or cognition?" If the answer is cognition, it should be a molecule step or formula instruction.

### 9.2 Bitter Lesson Alignment

Gas Town bets on models getting smarter, not on hand-crafted heuristics getting more elaborate. If an AI agent can observe data and reason about it, Gas Town exposes the data (transport) rather than encoding the reasoning (cognition). Today's clumsy heuristic is tomorrow's technical debt -- but a clean observability primitive ages well.

| Good (transport) | Bad (cognition in Go) |
|---|---|
| `gt nudge <session> "message"` | Go code deciding *when* to nudge |
| `bd show --json` exposing step status | Go code deciding *what* step status means |
| `tmux has-session` checking liveness | Go code with hardcoded "stuck after N minutes" |

### 9.3 Plugin System -- Dogs as Extensible Workers

The plugin system enables extensible, project-specific automation during Deacon patrol cycles. Plugins live as `plugin.md` files with TOML frontmatter defining gates (cooldown timers or event triggers), tracking labels, execution timeouts, and failure severity. They are dispatched to Dog workers:

```
Deacon Patrol                    Dog Worker
--------------------             --------------------
1. Scan plugins
2. Evaluate gates
3. For open gates:
   +-- gt dog dispatch plugin --> 4. Execute plugin
```

Plugin state lives on the ledger as wisps (discover, don't track). Gate evaluation queries the ledger directly. The Deacon (agent) evaluates gates and decides whether to dispatch. Go code provides transport (`gt dog dispatch`) but does not make decisions.

**Shipped plugins** (from `plugins/` directory, each with `plugin.md`):

| Plugin | Gate | Purpose |
|--------|------|---------|
| `compactor-dog` | Cooldown 30m | Monitor Dolt commit growth, escalate when compaction needed |
| `dolt-archive` | Cooldown 1h | Offsite backup: JSONL snapshots to git, `dolt push` to GitHub/DoltHub |
| `dolt-snapshots` | Event: convoy.created | Tag Dolt databases at convoy boundaries for audit/diff/rollback |
| `git-hygiene` | Cooldown 12h | Clean stale branches, stashes, and loose objects across rig repos |
| `github-sheriff` | Cooldown 5m | Monitor GitHub CI checks on open PRs, create beads for failures |
| `quality-review` | Cooldown 6h | Review merge quality and track per-worker trends |
| `rebuild-gt` | Cooldown 1h | Rebuild stale `gt` binary from gastown source |
| `session-hygiene` | Cooldown 30m | Clean zombie tmux sessions and orphaned dog sessions |
| `stuck-agent-dog` | Cooldown 5m | Context-aware stuck/crashed agent detection and restart |

Each plugin's TOML frontmatter specifies its gate type, execution timeout, failure notification severity, and tracking labels. This structure means adding a new periodic task requires only a new `plugin.md` file -- no Go code changes.

---

## 10. Comparison to Context Manager

| Aspect | Gas Town | Context Manager |
|--------|---------|-----------------|
| **Scale** | Multi-agent (4-30 agents) | Multi-agent with graph-native context |
| **Purpose** | Orchestrate agent colonies via work items | Orchestrate agents via context graph + work graph |
| **Work unit** | Molecules (multi-step workflows stored as beads) | Graph traversal + context construction + work items |
| **State persistence** | Beads + Dolt (relational, SQL + Git semantics) | Property graph + Cozo (graph-native, Datalog queries) |
| **Agent model** | 8 specialized roles (Mayor, Polecat, Witness, etc.) | TBD role taxonomy, graph-aware agents |
| **Communication** | Mail + Nudge + Handoff (beads-backed) | Graph edges + inter-agent context sharing |
| **Supervision** | Witness + Deacon watchdog chain | TBD supervision model |
| **Merge strategy** | Batch-then-bisect queue (Bors-style) | TBD |
| **Background processing** | Dogs (plugin workers) + data lifecycle (decay/compact/flatten) | MergeTree-inspired compaction pipeline |
| **Monitoring** | TUI feed + web dashboard + problems view | Context inspector panel + TUI |
| **Context management** | `gt prime` injects role template; handoff via beads state | Graph traversal constructs context deterministically |
| **Language** | Go (951 files, ~199K LOC non-test) | Rust (petgraph + Cozo + ratatui) |
| **Cost model** | $100-200/hr at full scale | Target $24/month per developer |

### 10.1 What We Can Learn

**Hooks and the Propulsion Principle.** The GUPP pattern (if work is on your hook, execute immediately) is a powerful anti-pattern avoidance mechanism. Context Manager should adopt a similar "anchor node" concept: when an agent's work-item node has pending edges, the agent proceeds without waiting for external confirmation.

**Session-per-step model.** Gas Town's insight that sessions are ephemeral but sandboxes are persistent maps directly to our graph model: the context window is transient, but the graph is durable. Each agent session should construct its context from the graph, not from a handoff payload.

**Watchdog patterns.** The three-tier watchdog chain (Daemon --> Boot --> Deacon --> Witnesses) is well-reasoned. Boot's "fresh triage every N minutes" pattern is particularly clever and cheap. Context Manager should implement similar supervision, potentially using graph analysis (stale work-item nodes, missing progress edges) rather than tmux session monitoring.

**Convoys for batch tracking.** The convoy abstraction (grouping related work items with cross-agent progress tracking) is directly applicable. In our graph model, a convoy would be a subgraph connecting related work-item nodes with a shared completion predicate.

**ZFC and the Bitter Lesson.** Gas Town's strict separation of transport (Go) and cognition (agents) is sound engineering. Context Manager should adopt the same principle: Rust provides graph infrastructure, traversal, and rendering; agents provide reasoning about what to do.

**Role taxonomy and separation of concerns.** The Mayor/Witness/Refinery/Polecat split prevents agents from stepping on each other. Even if Context Manager uses a different taxonomy, the principle of specialized roles with clear boundaries is validated.

**Mail/Nudge communication hygiene.** The distinction between persistent (expensive, permanent Dolt commit) and ephemeral (free, session-scoped) communication is operationally critical. Context Manager should adopt tiered communication: graph-edge creation for durable messages, lightweight pub/sub for ephemeral signals.

**The Capability Ledger.** Persistent per-agent work history enables A/B testing of models and capability-based routing. Our graph model naturally supports this: each agent's completed work items and quality ratings form a subgraph that can be queried.

### 10.2 Where We Diverge

**Graph-native foundation vs. relational.** Gas Town uses Dolt (SQL with Git semantics) for flat, tabular work-item storage. Context Manager uses a property graph (petgraph + Cozo) where relationships between work items, messages, tool calls, and artifacts are first-class edges. This enables multi-hop reasoning ("show me everything connected to this failing test") that Gas Town cannot do without ad-hoc queries.

**Multi-perspective compaction vs. static role templates.** Gas Town injects a static role template via `gt prime` and relies on beads state for continuity. Context Manager constructs context dynamically by traversing the graph and selecting compaction variants optimized for the current task. The same design discussion compacts differently when building the API versus writing tests.

**Observable context construction.** Gas Town's agents receive their context opaquely via `gt prime`. Context Manager makes context construction visible and debuggable: developers can see exactly which graph nodes were selected, at what compaction level, and why.

**Unified context + orchestration.** Gas Town separates orchestration (gt/beads) from context management (role templates, handoff mail). Context Manager unifies them: the graph IS both the context and the orchestration state. Work items, agent assignments, conversation history, and tool results all live in the same graph.

**Cost efficiency.** Gas Town burns $100-200/hour at full scale because each agent session is a full Claude Code instance. Context Manager's graph-native context construction enables precise token budgeting and aggressive compaction, targeting $24/month per developer.

---

## 11. Red Team / Green Team

### Green Team (Strengths)

- **Most sophisticated open-source multi-agent system.** No other project has shipped 8 specialized roles, a merge queue, a watchdog chain, sandboxed execution, and a TUI dashboard for agent monitoring.
- **ZFC is architecturally sound.** The transport/cognition split ages well as models improve. Hard-coded heuristics become debt; clean observability primitives become foundations.
- **Durable workflows survive crashes.** The NDI approach (beads state + git state + hook bead = full position recovery) is battle-tested through 2,400+ PRs.
- **Role taxonomy is well-reasoned.** Each role has clear boundaries, distinct lifecycle characteristics, and a specific failure mode it prevents.
- **Community traction.** 450+ contributors, 1,500+ merged PRs in two months. Multiple HN front-page posts. Podcast coverage (Software Engineering Daily). Third-party analysis from Maggie Appleton, Justin Abrahms, paddo.dev, DoltHub.
- **Proven at scale.** Yegge routinely runs 20-30 agents simultaneously on production codebases.
- **The Capability Ledger enables model evaluation.** Persistent per-agent work history enables data-driven decisions about which models to deploy for which tasks.

### Red Team (Risks and Weaknesses)

- **Massive complexity.** 64 internal Go packages, 951 source files, ~199K lines of non-test Go code. As Maggie Appleton noted: Yegge "had to keep adding components until it was a self-sustaining machine" rather than thoughtfully architecting upfront. The result is "a bunch of overlapping and ad hoc concepts."
- **Heavy Dolt dependency.** Single Dolt server is a single point of failure. If Dolt is down, the entire system is inoperable. No embedded fallback.
- **Extreme compute requirements.** $100-200/hour at full scale. Requires Claude Pro Max ($200/month) and routinely maxes out weekly limits across multiple accounts. The "Gas Town Serial Killer Murder Mystery" (where the Deacon killed other agents) illustrates the cost of bugs at scale.
- **ZFC taken to extreme.** Refusing to put ANY decision logic in Go code means every judgment call requires a full LLM inference. Simple threshold checks that could be O(1) in Go instead require spinning up an AI session.
- **Design bottleneck identified by Appleton.** "When agents write code rapidly, design and planning become the bottleneck -- not development speed." Gas Town churns through implementation plans so fast that keeping it fed with well-designed work becomes the limiting factor.
- **Early adopter feedback is mixed.** Justin Abrahms: "Gastown is too complex, but with refinement, I think it is a very big unlock." Paddo.dev: "Gas Town's chaos is real: $100/hour burn rate, auto-merged failing tests, murderous rampaging Deacon."
- **Monitoring is necessary but insufficient.** The TUI feed and problems view help, but at 20-30 agents, the human is still plate-spinning. Gas Town does not yet solve the fundamental attention-allocation problem.
- **Git-worktree-based isolation has limits.** All polecats share the same object store. Large repos with many concurrent worktrees can hit performance issues. Branch naming collisions are possible despite timestamps.

---

## 12. Community Reception and Ecosystem

### 12.1 Hacker News Discussions

Gas Town has generated sustained HN engagement across multiple front-page posts:

- [Welcome to Gas Town](https://news.ycombinator.com/item?id=46458936) (January 2026) -- initial launch discussion
- [Gas Town Emergency User Manual](https://news.ycombinator.com/item?id=46487580) (January 2026) -- 12 days post-launch, 100+ PRs from 50 contributors
- [Welcome to the Wasteland: A Thousand Gas Towns](https://news.ycombinator.com/item?id=46994362) (March 2026) -- federation announcement

### 12.2 Third-Party Analysis

- **Maggie Appleton** -- [Gas Town's Agent Patterns, Design Bottlenecks, and Vibecoding at Scale](https://maggieappleton.com/gastown). Extracted four orchestration patterns (specialized roles with hierarchy, persistent roles with ephemeral sessions, continuous work streams, agent-managed merge conflicts). Critiqued the ad-hoc design and identified design planning as the new bottleneck.

- **Justin Abrahms** -- [Wrapping my head around Gas Town](https://justin.abrah.ms/blog/2026-01-05-wrapping-my-head-around-gas-town.html). Practical first-impressions report. Key insight: "Aside from keeping Gas Town on the rails, probably the hardest problem is keeping it fed." Concluded: "too complex, but with refinement, a very big unlock."

- **Paddo.dev** -- [GasTown and the Two Kinds of Multi-Agent](https://paddo.dev/blog/gastown-two-kinds-of-multi-agent/). Distinguishes Gas Town's operational roles from the "org chart simulation" anti-pattern (BMAD, SpecKit). Argues most developers do not need Gas Town -- vanilla parallel Claude Code sessions with focused CLAUDE.md files suffice.

- **DoltHub** -- [A Day in Gas Town](https://www.dolthub.com/blog/2026-01-15-a-day-in-gas-town/). Technical walkthrough of Beads architecture and the case for Dolt as a unified backend replacing SQLite + Git sync. Notes $100-200/hour cost at full scale.

### 12.3 Podcast Coverage

- **Software Engineering Daily** -- [Gas Town, Beads, and the Rise of Agentic Development with Steve Yegge](https://softwareengineeringdaily.com/2026/02/12/gas-town-beads-and-the-rise-of-agentic-development-with-steve-yegge/) (February 2026, 69 minutes). Discusses evolution from chat-based assistance to full agent orchestration, task graphs, Git-backed ledgers, and the eight stages of developer evolution.

### 12.4 Broader Influence

- **Anthropic's Tasks.** Anthropic released Claude Code Tasks shortly after Gas Town launched, providing a simpler multi-agent primitive that can be composed into Gas Town-like workflows without the full orchestration stack.

- **The Wasteland.** Yegge's federation vision ([Welcome to the Wasteland: A Thousand Gas Towns](https://steve-yegge.medium.com/welcome-to-the-wasteland-a-thousand-gas-towns-a5eb9bc8dc1f)) proposes linking thousands of Gas Towns via a shared Wanted Board, PR-style completions, validator stamps, and portable reputation. Built on Dolt's fork/merge model. Key contributors: Julian Knutsen (ex-CashApp CTO), Dr. Matt Beane (author of *The Skill Code*), Tim Sehn (DoltHub CEO).

- **Goosetown.** Block's Goose project published [Gas Town Explained: How to Use Goosetown for Parallel Agentic Engineering](https://block.github.io/goose/blog/2026/02/19/gastown-explained-goosetown/), adapting Gas Town patterns for their own multi-agent system.

- **Industry context.** Gartner reported a 1,445% surge in multi-agent system inquiries between Q1 2024 and Q2 2025. Anthropic lists "mastering multi-agent coordination" as a top organizational priority for 2026.

---

## 13. Sources

### Primary Sources (Steve Yegge)

- Yegge, S. (2026) - [Welcome to Gas Town](https://steve-yegge.medium.com/welcome-to-gas-town-4f25ee16dd04) -- Medium launch post
- Yegge, S. (2026) - [Gas Town Emergency User Manual](https://steve-yegge.medium.com/gas-town-emergency-user-manual-cf0e4556d74b) -- 12-day post-launch report
- Yegge, S. (2026) - [The Future of Coding Agents](https://steve-yegge.medium.com/the-future-of-coding-agents-e9451a84207c) -- Industry analysis
- Yegge, S. (2026) - [Welcome to the Wasteland: A Thousand Gas Towns](https://steve-yegge.medium.com/welcome-to-the-wasteland-a-thousand-gas-towns-a5eb9bc8dc1f) -- Federation vision
- Yegge, S. (2026) - [Zero Framework Cognition](https://steve-yegge.medium.com/zero-framework-cognition-a-way-to-build-resilient-ai-applications-56b090ed3e69) -- Architectural philosophy

### Source Repository

- [steveyegge/gastown](https://github.com/steveyegge/gastown) -- GitHub repository
- Source files referenced: `internal/polecat/types.go`, `internal/witness/protocol.go`, `internal/daemon/daemon.go`, `internal/mail/types.go`, `internal/hooks/config.go`, `internal/convoy/operations.go`, `internal/refinery/types.go`, `internal/refinery/engineer.go`, `internal/refinery/score.go`, `internal/templates/roles/*.md.tmpl`, `plugins/*/plugin.md`, `docs/design/architecture.md`, `docs/design/polecat-lifecycle-patrol.md`, `docs/design/dog-infrastructure.md`, `docs/design/mail-protocol.md`, `docs/design/plugin-system.md`, `docs/proxy-server.md`, `docs/HOOKS.md`, `docs/glossary.md`, `docs/overview.md`

### Community Analysis

- Appleton, M. (2026) - [Gas Town's Agent Patterns, Design Bottlenecks, and Vibecoding at Scale](https://maggieappleton.com/gastown)
- Abrahms, J. (2026) - [Wrapping my head around Gas Town](https://justin.abrah.ms/blog/2026-01-05-wrapping-my-head-around-gas-town.html)
- Paddo (2026) - [GasTown and the Two Kinds of Multi-Agent](https://paddo.dev/blog/gastown-two-kinds-of-multi-agent/)
- DoltHub (2026) - [A Day in Gas Town](https://www.dolthub.com/blog/2026-01-15-a-day-in-gas-town/)

### Podcast

- Ball, K. and Yegge, S. (2026) - [Gas Town, Beads, and the Rise of Agentic Development](https://softwareengineeringdaily.com/2026/02/12/gas-town-beads-and-the-rise-of-agentic-development-with-steve-yegge/) -- Software Engineering Daily

### Industry Coverage

- ASCII News (2026) - [Steve Yegge Releases Gas Town](https://ascii.co.uk/news/article/news-20260102-190a5f9f/steve-yegge-releases-gas-town-multi-agent-orchestrator-for-c)
- Cloud Native Now (2026) - [Gas Town: What Kubernetes for AI Coding Agents Actually Looks Like](https://cloudnativenow.com/features/gas-town-what-kubernetes-for-ai-coding-agents-actually-looks-like/)
- Block/Goose (2026) - [Gas Town Explained: Goosetown for Parallel Agentic Engineering](https://block.github.io/goose/blog/2026/02/19/gastown-explained-goosetown/)

### Hacker News Threads

- [Welcome to Gas Town](https://news.ycombinator.com/item?id=46458936)
- [Gas Town Emergency User Manual](https://news.ycombinator.com/item?id=46487580)
- [Welcome to the Wasteland](https://news.ycombinator.com/item?id=46994362)
- [Wrapping my head around Gas Town](https://news.ycombinator.com/item?id=46611265)
