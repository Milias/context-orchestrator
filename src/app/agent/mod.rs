//! Agent loop, streaming, registry, and worktree management for concurrent agents.

pub(in crate::app) mod r#loop;
pub(in crate::app) mod registry;
pub(in crate::app) mod streaming;

pub(in crate::app) use r#loop::{spawn_agent_loop, AgentLoopConfig};
pub(in crate::app) use registry::AgentRegistry;
