//! Tool-related types: names, arguments, result content, and status tracking.

pub mod result;
pub mod types;

#[cfg(test)]
#[path = "types_tests.rs"]
mod types_tests;

#[cfg(test)]
#[path = "args_tests.rs"]
mod args_tests;
