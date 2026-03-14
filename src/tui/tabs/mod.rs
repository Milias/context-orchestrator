//! Top-level tab views for the monitoring-centric TUI layout.
//!
//! Each tab fills the left content area and has its own rendering logic.
//! The tab bar at the top shows all tabs with the active one highlighted.
//! Currently a single `overview` tab combines agents, work, and stats.

pub mod agents;
pub mod overview;
pub mod work;
