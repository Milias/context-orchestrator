//! Agent loop, streaming, registry, and worktree management for concurrent agents.

mod context_build;
pub(in crate::app) mod r#loop;
pub(in crate::app) mod registry;
pub(in crate::app) mod streaming;
pub(crate) mod worktree;

pub(in crate::app) use r#loop::{spawn_agent_loop, AgentLoopConfig};
pub(in crate::app) use registry::AgentRegistry;
