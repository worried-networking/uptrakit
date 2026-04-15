//! Surface action handlers for the Webhook notification plugin.

use sea_orm::DatabaseConnection;
use uptrakit_plugin_infrastructure_core::SurfaceActionContext;

/// Handle a surface action for the webhook notification plugin.
///
/// Supported actions:
/// - `list` — list webhook channels with masked secrets.
#[tracing::instrument(skip_all, fields(extension_id, action_id))]
pub async fn handle_action(
    ctx: &SurfaceActionContext<'_>,
    extension_id: &str,
    action_id: &str,
    params: serde_json::Value,
) -> std::result::Result<serde_json::Value, String> {
    let db = ctx
        .db
        .downcast_ref::<DatabaseConnection>()
        .ok_or_else(|| "expected DatabaseConnection".to_string())?;

    match action_id {
        "list" => {
            let tenant_id = ctx
                .tenant_id
                .ok_or_else(|| "tenant_id is required for listing channels".to_string())?;

            uptrakit_notification_plugin_core::list_channels::list_channels(
                db,
                tenant_id,
                "webhook",
                &params,
                |_channel_type, config| {
                    let mut masked = config.clone();
                    if let Some(obj) = masked.as_object_mut()
                        && let Some(secret) = obj.get("secret")
                        && secret.as_str().is_some_and(|s| !s.is_empty())
                    {
                        obj.insert("secret".to_string(), serde_json::json!("***"));
                    }
                    masked
                },
            )
            .await
        }
        _ => Err(format!(
            "unknown action '{action_id}' for extension '{extension_id}'"
        )),
    }
}
