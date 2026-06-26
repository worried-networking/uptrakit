//! Display-name lookups for software items and hosts (best-effort, infallible).

use std::sync::Arc;

use sea_orm::EntityTrait;
use uptrakit_shared_db::entity::{host, software_item};

use crate::AppState;

/// Resolve a software item name by ID (for batch progress events).
pub(in super::super) async fn resolve_software_item_name(
    state: &Arc<AppState>,
    item_id: uuid::Uuid,
) -> String {
    software_item::Entity::find_by_id(item_id)
        .one(state.db())
        .await
        .ok()
        .flatten()
        .map(|sw| sw.name)
        .unwrap_or_else(|| "Unknown Software".to_string())
}

/// Resolve a host name by ID (for batch progress events).
pub(in super::super) async fn resolve_host_name(
    state: &Arc<AppState>,
    host_id: uuid::Uuid,
) -> String {
    host::Entity::find_by_id(host_id)
        .one(state.db())
        .await
        .ok()
        .flatten()
        .map(|h| h.friendly_name)
        .unwrap_or_else(|| "Unknown Host".to_string())
}
