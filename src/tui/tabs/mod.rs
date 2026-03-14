//! Top-level tab views for the monitoring-centric TUI layout.
//!
//! Each tab fills the left content area and has its own rendering logic.
//! The tab bar at the top shows all tabs with the active one highlighted.
//! `overview` is the real-time operational dashboard. `graph` provides the
//! interactive graph explorer. `edge_inspector` and `explorer` hold
//! per-section navigation state for the graph tab.
//!
//! `overview_legacy` retains the old 3-column layout for removal in Phase 14.

pub mod agents;
pub mod edge_inspector;
pub mod explorer;
pub mod graph;
pub mod overview;
#[path = "overview_legacy.rs"]
pub mod overview_legacy;
pub mod system;
pub mod work;
