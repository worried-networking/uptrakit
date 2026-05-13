//! Emit-site sweep for stateful audit actions.
//!
//! Scans the workspace source tree to verify that every [`Kind::Stateful`]
//! action registered in the [`Registry`] has at least one `emit` call site.

use crate::registry::Registry;

/// Results from sweeping the source tree for stateful action emit sites.
#[derive(Debug)]
pub struct EmitReport {
    /// Registered stateful actions for which no emit call site was found.
    pub stateful_actions_without_emit_site: Vec<String>,
}

/// Scan the workspace rooted at `root` to verify emit coverage for all
/// stateful actions in `registry`.
///
/// This is a stub implementation that always returns an empty report.
/// The full implementation will use `walkdir` and `syn` to locate emit calls.
///
/// # Errors
///
/// Returns a descriptive string if the workspace tree cannot be traversed.
pub fn scan(_root: &std::path::Path, _registry: &Registry) -> Result<EmitReport, String> {
    Ok(EmitReport {
        stateful_actions_without_emit_site: vec![],
    })
}
