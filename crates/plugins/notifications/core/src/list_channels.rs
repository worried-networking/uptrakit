//! Shared helper for listing notification channels of a given type.
//!
//! All notification plugins use the same pagination and flattening logic
//! for the `list` extension action. This module provides a single
//! implementation that each plugin delegates to.

use sea_orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect,
};
use uptrakit_shared_db::entity::notification_channel;

/// List notification channels of a given type with flattened config fields.
///
/// The `mask_fn` callback is called for each channel's config to mask secrets.
/// It receives `(channel_type, config_json)` and returns the masked config.
pub async fn list_channels(
    db: &DatabaseConnection,
    tenant_id: uuid::Uuid,
    channel_type: &str,
    params: &serde_json::Value,
    mask_fn: impl Fn(&str, &serde_json::Value) -> serde_json::Value,
) -> std::result::Result<serde_json::Value, String> {
    let page = params
        .get("page")
        .and_then(|v| v.as_u64())
        .unwrap_or(1)
        .max(1);
    let per_page = params
        .get("per_page")
        .and_then(|v| v.as_u64())
        .unwrap_or(50)
        .clamp(1, 100);

    let offset = (page - 1) * per_page;

    let total = notification_channel::Entity::find()
        .filter(notification_channel::Column::TenantId.eq(tenant_id))
        .filter(notification_channel::Column::ChannelType.eq(channel_type))
        .count(db)
        .await
        .map_err(|e| {
            tracing::error!("Failed to count notification channels: {e:?}");
            "Internal server error".to_string()
        })?;

    let total_pages = if total == 0 {
        1
    } else {
        total.div_ceil(per_page)
    };

    let channels = notification_channel::Entity::find()
        .filter(notification_channel::Column::TenantId.eq(tenant_id))
        .filter(notification_channel::Column::ChannelType.eq(channel_type))
        .order_by_desc(notification_channel::Column::CreatedAt)
        .offset(Some(offset))
        .limit(Some(per_page))
        .all(db)
        .await
        .map_err(|e| {
            tracing::error!("Failed to list notification channels: {e:?}");
            "Internal server error".to_string()
        })?;

    let mut items = Vec::with_capacity(channels.len());
    for ch in channels {
        let config: serde_json::Value =
            serde_json::from_str(ch.config.expose_secret()).unwrap_or_default();
        let masked_config = mask_fn(&ch.channel_type, &config);

        let mut row = serde_json::json!({
            "id": ch.id,
            "name": ch.name,
            "enabled": ch.enabled,
            "created_at": ch.created_at,
        });

        // Generic flatten: merge all top-level config keys into the row.
        if let (Some(obj), Some(row_obj)) = (masked_config.as_object(), row.as_object_mut()) {
            for (key, val) in obj {
                row_obj.insert(key.clone(), val.clone());
            }
        }

        items.push(row);
    }

    Ok(serde_json::json!({
        "items": items,
        "total": total,
        "page": page,
        "per_page": per_page,
        "total_pages": total_pages,
    }))
}
