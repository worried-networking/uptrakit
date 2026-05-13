//! Static-analysis library for the `audit-coverage-check` tool.
//!
//! Provides modules for loading the audit catalog, parsing the action registry,
//! walking source files for audit emit call sites, and sweeping for stateful
//! actions that are never emitted.

pub mod catalog;
pub mod emit_sweep;
pub mod registry;
pub mod walker;
