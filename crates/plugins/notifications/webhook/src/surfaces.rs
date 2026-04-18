//! Surface action handlers for the Webhook notification plugin.

use uptrakit_plugin_infrastructure_core::{
    NotificationChannelListRequest, SurfaceActionContext, SurfaceActionError,
};

/// Handle a surface action for the webhook notification plugin.
///
/// Supported actions:
/// - `list` — list webhook channels with masked secrets.
#[tracing::instrument(skip_all, fields(surface_id, action_id))]
pub async fn handle_surface_action(
    ctx: &SurfaceActionContext<'_>,
    surface_id: &str,
    action_id: &str,
    params: serde_json::Value,
) -> std::result::Result<serde_json::Value, SurfaceActionError> {
    match action_id {
        "list" => handle_list(ctx, &params).await,
        _ => Err(SurfaceActionError::InvalidInput(format!(
            "unknown action '{action_id}' for surface '{surface_id}'",
        ))),
    }
}

async fn handle_list(
    ctx: &SurfaceActionContext<'_>,
    params: &serde_json::Value,
) -> std::result::Result<serde_json::Value, SurfaceActionError> {
    let store = ctx.controller.notification_channel_store().ok_or_else(|| {
        SurfaceActionError::ControllerIntegration(
            "notification channel store is not available".to_string(),
        )
    })?;

    let page = store
        .list_channels(NotificationChannelListRequest {
            tenant_id: ctx.tenant_id(),
            channel_type: "webhook",
            page: parse_page(params),
            per_page: parse_per_page(params),
        })
        .await
        .map_err(|error| {
            tracing::error!(error = ?error, "failed to list webhook channels");
            SurfaceActionError::ControllerIntegration("failed to list channels".to_string())
        })?;

    let mut items = Vec::with_capacity(page.items.len());
    for channel in page.items {
        let mut row = serde_json::json!({
            "id": channel.id,
            "name": channel.name,
            "enabled": channel.enabled,
            "created_at": channel.created_at_rfc3339,
        });
        if let (Some(config), Some(row_obj)) = (channel.config.as_object(), row.as_object_mut()) {
            for (key, value) in config {
                row_obj.insert(key.clone(), value.clone());
            }
        }
        items.push(row);
    }

    Ok(serde_json::json!({
        "items": items,
        "total": page.total,
        "page": page.page,
        "per_page": page.per_page,
        "total_pages": page.total_pages,
    }))
}

fn parse_page(params: &serde_json::Value) -> u64 {
    params
        .get("page")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(1)
        .max(1)
}

fn parse_per_page(params: &serde_json::Value) -> u64 {
    params
        .get("per_page")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(50)
        .clamp(1, 100)
}
