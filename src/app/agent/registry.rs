//! Registry of active agent loops with tool result routing.
//!
//! Each agent loop is identified by a `Uuid` (the same ID used in `ClaimedBy`
//! graph edges). The registry owns per-agent channels, cancellation tokens,
//! and phase tracking. It routes `AgentToolResult` notifications to the correct
//! agent based on which agent dispatched each tool call.

use crate::tasks::AgentToolResult;

use std::collections::{HashMap, HashSet};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// Tracks all active agent loops and routes tool completions.
/// All agents are ephemeral and equal — no "primary" distinction.
pub struct AgentRegistry {
    /// Active agent loops keyed by agent ID.
    agents: HashMap<Uuid, AgentHandle>,
    /// Reverse index: `tool_call_id` → `agent_id` for O(1) routing.
    tool_call_owner: HashMap<Uuid, Uuid>,
    /// Reverse index: `work_item_id` → `agent_id` for preventing double-spawn.
    work_item_agents: HashMap<Uuid, Uuid>,
}

/// Per-agent metadata held by the registry.
struct AgentHandle {
    /// Sender for forwarding tool completions to this agent's loop.
    tool_tx: mpsc::UnboundedSender<AgentToolResult>,
    /// Root cancellation token for this agent.
    cancel_token: CancellationToken,
    /// Per-tool-call child cancellation tokens.
    task_tokens: HashMap<Uuid, CancellationToken>,
    /// Node IDs of currently active `BackgroundTask` (phase) nodes.
    active_phase_ids: HashSet<Uuid>,
    /// Working directory for file operations (git worktree path for task agents,
    /// `None` for conversational agents using process CWD).
    working_dir: Option<std::path::PathBuf>,
}

impl AgentRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            agents: HashMap::new(),
            tool_call_owner: HashMap::new(),
            work_item_agents: HashMap::new(),
        }
    }

    /// Number of currently active agents.
    pub fn active_count(&self) -> usize {
        self.agents.len()
    }

    /// Register a new agent. Returns the tool-result receiver and cancellation
    /// token for the agent loop to use. `working_dir` is the git worktree path
    /// for task agents (`None` for conversational agents).
    pub fn register(
        &mut self,
        agent_id: Uuid,
        working_dir: Option<std::path::PathBuf>,
    ) -> (mpsc::UnboundedReceiver<AgentToolResult>, CancellationToken) {
        let (tx, rx) = mpsc::unbounded_channel();
        let cancel_token = CancellationToken::new();
        self.agents.insert(
            agent_id,
            AgentHandle {
                tool_tx: tx,
                cancel_token: cancel_token.clone(),
                task_tokens: HashMap::new(),
                active_phase_ids: HashSet::new(),
                working_dir,
            },
        );
        (rx, cancel_token)
    }

    /// Check if a work item already has an assigned agent.
    pub fn agent_for_work_item(&self, work_item_id: Uuid) -> Option<Uuid> {
        self.work_item_agents.get(&work_item_id).copied()
    }

    /// Track the association between a work item and its agent.
    pub fn track_work_item(&mut self, work_item_id: Uuid, agent_id: Uuid) {
        self.work_item_agents.insert(work_item_id, agent_id);
    }

    /// Get the working directory for an agent's file operations.
    pub fn working_dir(&self, agent_id: Uuid) -> Option<std::path::PathBuf> {
        self.agents
            .get(&agent_id)
            .and_then(|h| h.working_dir.clone())
    }

    /// Record a tool call dispatched by a specific agent.
    pub fn track_tool_call(
        &mut self,
        agent_id: Uuid,
        tool_call_id: Uuid,
        token: CancellationToken,
    ) {
        self.tool_call_owner.insert(tool_call_id, agent_id);
        if let Some(handle) = self.agents.get_mut(&agent_id) {
            handle.task_tokens.insert(tool_call_id, token);
        }
    }

    /// Route a tool completion to the owning agent. Returns `false` if the
    /// tool call has no known owner (e.g., user-triggered or agent already removed).
    pub fn route_tool_result(&mut self, tool_call_id: Uuid) -> bool {
        let Some(agent_id) = self.tool_call_owner.remove(&tool_call_id) else {
            return false;
        };
        let Some(handle) = self.agents.get_mut(&agent_id) else {
            return false;
        };
        handle.task_tokens.remove(&tool_call_id);
        let _ = handle.tool_tx.send(AgentToolResult { tool_call_id });
        true
    }

    /// Get a child cancellation token for a tool call under a specific agent.
    pub fn child_cancel_token(&self, agent_id: Uuid) -> CancellationToken {
        self.agents
            .get(&agent_id)
            .map_or_else(CancellationToken::new, |h| h.cancel_token.child_token())
    }

    /// Cancel an entire agent and all its tool calls.
    pub fn cancel_agent(&mut self, agent_id: Uuid) {
        if let Some(handle) = self.agents.get(&agent_id) {
            handle.cancel_token.cancel();
        }
    }

    /// Remove an agent on completion. Returns `true` if the agent existed.
    pub fn remove(&mut self, agent_id: Uuid) -> bool {
        let existed = self.agents.remove(&agent_id).is_some();
        if existed {
            self.tool_call_owner.retain(|_, owner| *owner != agent_id);
            self.work_item_agents.retain(|_, owner| *owner != agent_id);
        }
        existed
    }

    /// Track a phase node for an agent.
    pub fn track_phase(&mut self, agent_id: Uuid, phase_id: Uuid) {
        if let Some(handle) = self.agents.get_mut(&agent_id) {
            handle.active_phase_ids.insert(phase_id);
        }
    }

    /// Complete a phase for an agent. Returns the previous phase ID if any.
    pub fn complete_phase(&mut self, agent_id: Uuid, phase_id: &Uuid) {
        if let Some(handle) = self.agents.get_mut(&agent_id) {
            handle.active_phase_ids.remove(phase_id);
        }
    }

    /// Drain all active phase IDs for an agent (on finish/error).
    pub fn drain_phases(&mut self, agent_id: Uuid) -> Vec<Uuid> {
        self.agents
            .get_mut(&agent_id)
            .map_or_else(Vec::new, |h| h.active_phase_ids.drain().collect())
    }

    /// Cancel and clear all agents (for shutdown).
    pub fn cancel_all(&mut self) {
        for handle in self.agents.values() {
            handle.cancel_token.cancel();
        }
        self.agents.clear();
        self.tool_call_owner.clear();
        self.work_item_agents.clear();
    }
}

impl Default for AgentRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[path = "registry_tests.rs"]
mod tests;
