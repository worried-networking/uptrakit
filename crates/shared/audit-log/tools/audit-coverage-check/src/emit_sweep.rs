//! Emit-site sweep for stateful audit actions.
//!
//! Scans the workspace source tree to verify that every [`Kind::Stateful`]
//! action registered in the [`Registry`] has at least one emit call site in
//! the source tree (excluding the `audit-log` crate's own `src/` where the
//! methods are defined).
//!
//! The method name is derived from the action value by replacing `.` with `_`.
//! For example, action `"plugin_config.update"` matches either
//! `AuditEntry::plugin_config_update(` (plain form) or
//! `AuditEntry::<Stateful>::plugin_config_update(` (explicit type-parameter form).

use std::path::Path;

use crate::registry::{Kind, Registry};

/// Results from sweeping the source tree for stateful action emit sites.
#[derive(Debug)]
#[non_exhaustive]
pub struct EmitReport {
    /// Registered stateful actions for which no emit call site was found.
    pub stateful_actions_without_emit_site: Vec<String>,
}

/// Scan the workspace rooted at `root` to verify emit coverage for all
/// stateful actions in `registry`.
///
/// For each `Stateful` action, searches all `.rs` files under `crates/`
/// (excluding `audit-log/src/`) for either `AuditEntry::<method>(` or
/// `AuditEntry::<Stateful>::<method>(`, where `<method>` is the action value
/// with `.` replaced by `_`.
///
/// # Errors
///
/// Returns a descriptive string if the workspace tree cannot be traversed.
pub fn scan(root: &Path, registry: &Registry) -> Result<EmitReport, String> {
    let files = crate::walker::collect_rust_sources(root);
    let stateful: Vec<&crate::registry::RegistryEntry> = registry
        .actions
        .values()
        .filter(|e| e.kind == Kind::Stateful)
        .collect();

    let mut missing = Vec::new();
    for action in stateful {
        let method = action.value.replace('.', "_");
        // Match both plain `AuditEntry::method(` and the turbofish form
        // `AuditEntry::<Stateful>::method(` used at call sites that name the
        // type parameter explicitly.
        let needle_plain = format!("AuditEntry::{method}(");
        let needle_stateful = format!("AuditEntry::<Stateful>::{method}(");
        let mut found = false;
        'files: for f in &files {
            // Skip the audit-log crate itself (where the methods are *defined*).
            if f.to_string_lossy().contains("/audit-log/src/") {
                continue;
            }
            match std::fs::read_to_string(f) {
                Ok(src) if src.contains(&needle_plain) || src.contains(&needle_stateful) => {
                    found = true;
                    break 'files;
                }
                _ => {}
            }
        }
        if !found {
            missing.push(action.value.clone());
        }
    }
    Ok(EmitReport {
        stateful_actions_without_emit_site: missing,
    })
}
