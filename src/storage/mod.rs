//! Persistent storage layer.
//!
//! Currently provides analytics (token usage tracking) via `SQLite`.
//! Designed to be the foundational home for all database code,
//! including future OLTP graph storage.

mod analytics;
mod schema;

pub use analytics::TokenStore;
pub use schema::{TokenDirection, TokenEvent, TokenTotals};

#[cfg(test)]
#[path = "analytics_tests.rs"]
mod tests;
