//! Test-only crate-internal exposure, gated behind `test-support`.
//!
//! `query_agent_assignment_rows`, `AgentAssignmentRow`, and the `queries`
//! module (`crate::executors::queries`) are all `pub(crate)` -- unreachable
//! from a `tests/` integration target, which only sees the crate's `pub`
//! surface. This module re-exports a trimmed view type plus a thin wrapper
//! so `tests/aliased_decrypt.rs` (its own compiled test binary -- required
//! because the crypto crate's DEK ring / master key are process-global
//! `OnceLock`s and in-crate scheduler tests call `enable_plaintext_mode()`,
//! so mixing the two in one process would be unsound) can drive the real
//! query and prove alias-immunity end to end: decryption happens inside the
//! real query's `TryGetable` decode, before the row is mapped into
//! `AssignmentRowView`.
//!
//! Existing items (`query_agent_assignment_rows`, `AgentAssignmentRow`, the
//! `executors::queries` module) keep their `pub(crate)` visibility
//! untouched -- this module only adds a new, additive `pub` surface.

use sea_orm::DatabaseConnection;
use uuid::Uuid;

use uptrakit_shared_db::encrypted_columns::EncryptedPluginConfig;

/// Trimmed view over `AgentAssignmentRow`, exposing only what real-key
/// acceptance tests need to assert alias-immune decryption.
pub struct AssignmentRowView {
    pub host_software_item_id: Uuid,
    pub profile_config: Option<EncryptedPluginConfig>,
}

/// Test-only wrapper around the real, `pub(crate)`
/// [`crate::executors::queries::query_agent_assignment_rows`]. Delegates to
/// the real query and maps rows into [`AssignmentRowView`]; the decryption
/// itself happens inside the real query's `TryGetable` decode, so this proves
/// alias-immunity end to end rather than merely re-testing the newtype.
pub async fn query_agent_assignment_rows(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    roles: &[&str],
) -> crate::error::Result<Vec<AssignmentRowView>> {
    let rows = crate::executors::queries::query_agent_assignment_rows(db, tenant_id, roles).await?;
    Ok(rows
        .into_iter()
        .map(|row| AssignmentRowView {
            host_software_item_id: row.host_software_item_id,
            profile_config: row.profile_config,
        })
        .collect())
}
