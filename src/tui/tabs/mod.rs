//! Top-level tab views for the monitoring-centric TUI layout.
//!
//! Each tab fills the left content area and has its own rendering logic.
//! The tab bar at the top shows all tabs with the active one highlighted.
//! `overview` is the real-time operational dashboard. `graph` provides the
//! interactive graph explorer. `edge_inspector` and `explorer` hold
//! per-section navigation state for the graph tab.

pub mod agents;
pub mod edge_inspector;
pub mod explorer;
pub mod graph;
pub mod overview;
pub mod system;
pub mod work;
