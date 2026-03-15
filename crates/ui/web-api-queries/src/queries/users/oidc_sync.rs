//! OIDC user-resolution DB helpers.
//!
//! This module contains pure-database helpers used by the OIDC route layer:
//! - Looking up an active OIDC provider by ID.
//! - Building synthetic claim values for deferred role-sync flows (account
//!   linking, deferred registration completion) where the original ID token
//!   is no longer available.
//!
//! All functions in this module operate only on [`sea_orm::ConnectionTrait`]
//! and standard Rust types — no HTTP or `openidconnect` types are imported.

use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter};
use uptrakit_shared_db::entity::{oidc_provider, prelude::OidcProvider};

/// Look up a single active, non-deactivated OIDC provider by `id` within the
/// given `tenant_id`. Returns `None` when the provider is missing, inactive,
/// or soft-deleted.
pub async fn find_active_provider(
    db: &impl ConnectionTrait,
    tenant_id: uuid::Uuid,
    id: uuid::Uuid,
) -> Option<oidc_provider::Model> {
    OidcProvider::find_by_id(id)
        .filter(oidc_provider::Column::TenantId.eq(tenant_id))
        .filter(oidc_provider::Column::IsActive.eq(true))
        .filter(oidc_provider::Column::DeactivatedAt.is_null())
        .one(db)
        .await
        .ok()
        .flatten()
}

/// Build a synthetic `serde_json::Value` that re-maps stored `mapped_roles`
/// back to the provider's original role-claim keys, suitable for passing to
/// `sync_oidc_roles`.
///
/// This is required in deferred flows (registration completion, account
/// linking) where the original OIDC ID token is no longer available. The
/// provider's `role_mapping` is reversed to recover the external claim values
/// corresponding to the already-computed local role names.
pub fn build_fake_claims_for_sync(
    provider: &oidc_provider::Model,
    mapped_roles: &[String],
) -> serde_json::Value {
    let mut fake_claims = serde_json::Map::new();
    if let Some(ref path) = provider.role_claim_path {
        let reverse_mapped: Vec<String> = mapped_roles
            .iter()
            .filter_map(|local_name| {
                provider
                    .role_mapping
                    .0
                    .iter()
                    .find(|(_, v)| v.as_str() == local_name)
                    .map(|(k, _)| k.clone())
            })
            .collect();
        let first_segment = path.split('.').next().unwrap_or(path);
        fake_claims.insert(
            first_segment.to_string(),
            serde_json::Value::Array(
                reverse_mapped
                    .into_iter()
                    .map(serde_json::Value::String)
                    .collect(),
            ),
        );
    }
    serde_json::Value::Object(fake_claims)
}
