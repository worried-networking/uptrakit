//! Audit action registry loader.
//!
//! Parses `action_type.rs` from the `uptrakit-audit-log` crate and extracts
//! all `RegisteredAuditAction` constant declarations, classifying each as
//! [`Kind::Stateful`] or [`Kind::Event`].

use std::collections::HashMap;

/// All registered audit actions keyed by their constant identifier.
#[derive(Debug)]
pub struct Registry {
    /// Map from constant identifier string (e.g. `"AUTH_LOGIN"`) to its entry.
    pub actions: HashMap<String, RegistryEntry>,
}

/// Metadata for a single registered audit action.
#[derive(Debug, Clone)]
pub struct RegistryEntry {
    /// The Rust constant identifier as written in `action_type.rs`.
    pub const_ident: String,
    /// The runtime string value of the action.
    pub value: String,
    /// Whether this action records before/after state or is an event-only record.
    pub kind: Kind,
}

/// Classification of an audit action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// The action records a state transition (has before/after snapshots).
    Stateful,
    /// The action records a point-in-time event with no snapshot delta.
    Event,
}

/// Load the action registry by parsing the Rust source file at `path`.
///
/// This is a stub implementation that always returns an empty registry.
/// The full implementation will use `syn` to parse `action_type.rs`.
///
/// # Errors
///
/// Returns a descriptive string if the source file cannot be read.
pub fn load(_path: &std::path::Path) -> Result<Registry, String> {
    Ok(Registry {
        actions: HashMap::new(),
    })
}
