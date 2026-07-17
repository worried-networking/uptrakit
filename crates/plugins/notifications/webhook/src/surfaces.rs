//! Surface action handlers for the Webhook notification plugin.

use uptrakit_plugin_infrastructure_core::{SurfaceActionContext, SurfaceActionError};

/// Dispatch shim for the `list` interaction (exact-id dispatch map entry).
pub(crate) fn webhook_list_handler<'a>(
    ctx: &'a SurfaceActionContext<'a>,
    params: serde_json::Value,
) -> std::pin::Pin<
    Box<
        dyn std::future::Future<Output = std::result::Result<serde_json::Value, SurfaceActionError>>
            + Send
            + 'a,
    >,
> {
    Box::pin(async move { handle_list(ctx, &params).await })
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
