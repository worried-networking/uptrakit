//! Host matching logic: match discovered Proxmox guests to Uptrakit hosts.
//!
//! Auto-matching is not performed because no reliable stable identifier
//! (such as `machine_id`) is available through the Proxmox VE REST API.
//! All matching is manual — users explicitly link Proxmox guests to Uptrakit
//! hosts.

use sea_orm::{ActiveModelTrait, DatabaseConnection, EntityTrait, Set};
use uuid::Uuid;

use crate::error::{ProxmoxError, Result};

/// Match method used to link a Proxmox guest to an Uptrakit host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchMethod {
    /// Manually matched by a user.
    Manual,
}

impl MatchMethod {
    /// Returns the string representation stored in the database.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Manual => "manual",
        }
    }
}

/// Set or update a manual match between a mapping and a host.
pub async fn manual_match(db: &DatabaseConnection, mapping_id: Uuid, host_id: Uuid) -> Result<()> {
    use uptrakit_shared_db::entity::proxmox_host_mapping;

    tracing::debug!(%mapping_id, %host_id, "performing manual Proxmox guest-to-host match");

    let mapping = proxmox_host_mapping::Entity::find_by_id(mapping_id)
        .one(db)
        .await
        .map_err(|e| {
            rootcause::report!(ProxmoxError::Database(format!(
                "failed to find mapping: {e}"
            )))
        })?
        .ok_or_else(|| {
            rootcause::report!(ProxmoxError::Database(format!(
                "mapping {mapping_id} not found"
            )))
        })?;

    let mut active: proxmox_host_mapping::ActiveModel = mapping.into();
    active.host_id = Set(Some(host_id));
    active.match_method = Set(Some(MatchMethod::Manual.as_str().to_string()));
    active.update(db).await.map_err(|e| {
        rootcause::report!(ProxmoxError::Database(format!(
            "failed to update mapping: {e}"
        )))
    })?;

    tracing::info!(%mapping_id, %host_id, "Proxmox guest matched to host");

    Ok(())
}

/// Remove a match from a mapping.
pub async fn unmatch(db: &DatabaseConnection, mapping_id: Uuid) -> Result<()> {
    use uptrakit_shared_db::entity::proxmox_host_mapping;

    tracing::debug!(%mapping_id, "removing Proxmox guest-to-host match");

    let mapping = proxmox_host_mapping::Entity::find_by_id(mapping_id)
        .one(db)
        .await
        .map_err(|e| {
            rootcause::report!(ProxmoxError::Database(format!(
                "failed to find mapping: {e}"
            )))
        })?
        .ok_or_else(|| {
            rootcause::report!(ProxmoxError::Database(format!(
                "mapping {mapping_id} not found"
            )))
        })?;

    let mut active: proxmox_host_mapping::ActiveModel = mapping.into();
    active.host_id = Set(None);
    active.match_method = Set(None);
    active.update(db).await.map_err(|e| {
        rootcause::report!(ProxmoxError::Database(format!(
            "failed to update mapping: {e}"
        )))
    })?;

    tracing::info!(%mapping_id, "Proxmox guest-to-host match removed");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn match_method_as_str() {
        assert_eq!(MatchMethod::Manual.as_str(), "manual");
    }
}
