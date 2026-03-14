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
pub struct AgentRegistry {
    /// Active agent loops keyed by agent ID.
    agents: HashMap<Uuid, AgentHandle>,
    /// Reverse index: `tool_call_id` → `agent_id` for O(1) routing.
    tool_call_owner: HashMap<Uuid, Uuid>,
    /// ID of the primary conversation agent (for TUI display routing).
    /// `None` if no conversation agent is running.
    pub primary_agent_id: Option<Uuid>,
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
}

impl AgentRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            agents: HashMap::new(),
            tool_call_owner: HashMap::new(),
            primary_agent_id: None,
        }
    }

    /// Register a new agent. Returns the tool-result receiver and cancellation
    /// token for the agent loop to use.
    pub fn register(
        &mut self,
        agent_id: Uuid,
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
            },
        );
        (rx, cancel_token)
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

    /// Cancel a specific tool call.
    pub fn cancel_tool(&mut self, tool_call_id: Uuid) {
        if let Some(agent_id) = self.tool_call_owner.get(&tool_call_id) {
            if let Some(handle) = self.agents.get(agent_id) {
                if let Some(token) = handle.task_tokens.get(&tool_call_id) {
                    token.cancel();
                }
            }
        }
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
            // Clean up orphaned tool_call_owner entries for this agent.
            self.tool_call_owner.retain(|_, owner| *owner != agent_id);
            if self.primary_agent_id == Some(agent_id) {
                self.primary_agent_id = None;
            }
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

    /// Whether the given agent is the primary conversation agent.
    pub fn is_primary(&self, agent_id: Uuid) -> bool {
        self.primary_agent_id == Some(agent_id)
    }

    /// Clear all agents (for shutdown).
    pub fn cancel_all(&mut self) {
        for handle in self.agents.values() {
            handle.cancel_token.cancel();
        }
        self.agents.clear();
        self.tool_call_owner.clear();
        self.primary_agent_id = None;
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
