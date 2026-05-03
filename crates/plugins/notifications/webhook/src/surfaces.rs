//! Surface action handlers for the Webhook notification plugin.

use uptrakit_plugin_infrastructure_core::{SurfaceActionContext, SurfaceActionError};

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
    uptrakit_notification_plugin_core::list_channels::list_channels(
        ctx.tenant_db().db(),
        ctx.tenant_id(),
        "webhook",
        params,
        |_channel_type, config| config.clone(),
    )
    .await
    .map_err(SurfaceActionError::ControllerIntegration)
}
