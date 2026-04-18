//! Surface action handler dispatch for the Docker plugin.
//!
//! ## Action flow
//!
//! 1. The frontend opens the context menu on a host row that has a Docker
//!    plugin assignment.
//! 2. The user clicks **Switch Tag** — the form pre-loads the current image
//!    reference (without `#container` suffix) via `get-current-tag`.
//! 3. The user edits the tag and submits — `switch-tag` updates all
//!    `host_software_item_plugin` rows and the `host_software_item` row,
//!    then clears stale version data so the next check reflects the new tag.
//!
//! All Docker-specific logic (`ImageRef` parsing, `#container` suffix handling,
//! `validate_identifier` SSRF guard) lives here and does not leak to callers.

use std::future::Future;
use std::pin::Pin;

use uuid::Uuid;

use uptrakit_plugin_infrastructure_core::{
    FormFieldDescriptor, FormFieldType, SurfaceActionContext, SurfaceActionDescriptor,
    SurfaceActionError, SurfaceActionUi, SurfaceFormDescriptor,
};
use uptrakit_shared_types::Permission;

use crate::image_ref::{ImageRef, validate_identifier};

// ── Public API ───────────────────────────────────────────────────────────────

/// Returns the surface action library for the Docker plugin.
///
/// All action IDs referenced by shared-surface interaction definitions must be
/// defined here.
pub fn surface_actions() -> Vec<SurfaceActionDescriptor> {
    vec![switch_tag_action(), get_current_tag_action()]
}

// ── Action definitions ───────────────────────────────────────────────────────

/// Form action: switch the Docker image tag for a specific host assignment.
fn switch_tag_action() -> SurfaceActionDescriptor {
    SurfaceActionDescriptor::new("switch-tag", "Switch Tag")
        .with_permission(Permission::UpdateSoftware)
        .with_ui(SurfaceActionUi::Form(
            SurfaceFormDescriptor::new(vec![
                FormFieldDescriptor::new("software_item_id", "")
                    .with_type(FormFieldType::Hidden)
                    .required(),
                FormFieldDescriptor::new("host_id", "")
                    .with_type(FormFieldType::Hidden)
                    .required(),
                FormFieldDescriptor::new("new_image_ref", "New Image Reference")
                    .required()
                    .with_placeholder("ghcr.io/example/app:26.2.6")
                    .with_help_text(
                        "Enter the full image reference with the new tag. \
                         The container name is preserved automatically.",
                    ),
            ])
            .with_pre_load_action("get-current-tag"),
        ))
}

/// Pre-load helper: returns the current image reference for form pre-population.
///
/// Not shown as a button in the UI — invoked automatically when the Switch Tag
/// form opens to populate the `new_image_ref` field with the current value.
fn get_current_tag_action() -> SurfaceActionDescriptor {
    SurfaceActionDescriptor::new("get-current-tag", "").with_permission(Permission::UpdateSoftware)
}

// ── Surface action handler ───────────────────────────────────────────────────

/// Dispatch a surface action for the Docker plugin.
///
/// Routes based on `(surface_id, action_id)` to the appropriate handler.
///
/// Surface handlers now receive a typed `SurfaceActionContext` and must return
/// typed `SurfaceActionError` values at the boundary.
pub fn handle_surface_action<'a>(
    ctx: &'a SurfaceActionContext<'a>,
    surface_id: &'a str,
    action_id: &'a str,
    params: serde_json::Value,
) -> Pin<
    Box<
        dyn Future<Output = std::result::Result<serde_json::Value, SurfaceActionError>> + Send + 'a,
    >,
> {
    Box::pin(handle_surface_action_inner(
        ctx, surface_id, action_id, params,
    ))
}

#[tracing::instrument(skip_all, fields(surface_id, action_id))]
async fn handle_surface_action_inner(
    ctx: &SurfaceActionContext<'_>,
    surface_id: &str,
    action_id: &str,
    params: serde_json::Value,
) -> std::result::Result<serde_json::Value, SurfaceActionError> {
    tracing::debug!("dispatching Docker surface action");

    let result: std::result::Result<serde_json::Value, String> = match (surface_id, action_id) {
        ("docker.item-host-actions", "switch-tag") => handle_switch_tag(ctx, params).await,
        ("docker.item-host-actions", "get-current-tag") => {
            handle_get_current_tag(ctx, params).await
        }
        _ => Err(format!(
            "unknown action '{action_id}' for surface '{surface_id}'"
        )),
    };

    match &result {
        Ok(_) => tracing::debug!("Docker surface action succeeded"),
        Err(e) => tracing::warn!(error = %e, "Docker surface action failed"),
    }

    result.map_err(surface_action_error_from_string)
}

fn require_docker_store<'a>(
    ctx: &'a SurfaceActionContext<'a>,
) -> std::result::Result<
    &'a dyn uptrakit_plugin_infrastructure_core::DockerSurfaceStore,
    SurfaceActionError,
> {
    ctx.controller.docker_surface_store().ok_or_else(|| {
        SurfaceActionError::ControllerIntegration(
            "docker surface store is not available".to_string(),
        )
    })
}

fn surface_action_error_from_string(message: String) -> SurfaceActionError {
    if message.starts_with("missing ")
        || message.starts_with("invalid ")
        || message.starts_with("unknown action")
        || message.starts_with("no plugin assignments")
    {
        SurfaceActionError::InvalidInput(message)
    } else if message.starts_with("database error")
        || message.starts_with("failed to")
        || message.contains("not found for host")
    {
        SurfaceActionError::ControllerIntegration(message)
    } else {
        SurfaceActionError::PluginInternal(message)
    }
}

// ── Action handlers ──────────────────────────────────────────────────────────

/// Pre-load handler: return the current image reference (without `#container`
/// suffix) so the Switch Tag form can pre-populate the `new_image_ref` field.
async fn handle_get_current_tag(
    ctx: &SurfaceActionContext<'_>,
    params: serde_json::Value,
) -> std::result::Result<serde_json::Value, String> {
    let host_id = parse_uuid_param(&params, "host_id")?;
    let software_item_id = parse_uuid_param(&params, "software_item_id")?;

    tracing::debug!(%host_id, %software_item_id, "fetching current Docker tag");

    let store = require_docker_store(ctx).map_err(|error| error.to_string())?;
    let image_ref = store
        .load_current_image_ref(host_id, software_item_id)
        .await
        .map_err(normalize_store_error)?;

    tracing::debug!(%host_id, %software_item_id, image_ref = %image_ref, "resolved current Docker tag");

    Ok(serde_json::json!({ "new_image_ref": image_ref }))
}

/// Switch tag handler: update all Docker plugin rows for a specific
/// `(host_id, software_item_id)` pair and clear stale version data.
///
/// Preserves the `#container_name` suffix on each plugin row so subsequent
/// update operations still target the correct named container.
async fn handle_switch_tag(
    ctx: &SurfaceActionContext<'_>,
    params: serde_json::Value,
) -> std::result::Result<serde_json::Value, String> {
    let host_id = parse_uuid_param(&params, "host_id")?;
    let software_item_id = parse_uuid_param(&params, "software_item_id")?;

    let new_image_ref = params
        .get("new_image_ref")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing required parameter 'new_image_ref'".to_string())?
        .trim()
        .to_string();

    tracing::debug!(%host_id, %software_item_id, new_image_ref = %new_image_ref, "switching Docker tag");

    // Validate format first (parses the image ref) then SSRF check.
    let new_ref: ImageRef = new_image_ref
        .parse()
        .map_err(|e| format!("invalid image reference: {e}"))?;

    validate_identifier(&new_image_ref).map_err(|e| format!("invalid image reference: {e}"))?;
    let store = require_docker_store(ctx).map_err(|error| error.to_string())?;
    store
        .switch_image_ref(host_id, software_item_id, new_ref.full_ref.clone())
        .await
        .map_err(normalize_store_error)?;

    tracing::info!(
        %host_id,
        %software_item_id,
        new_image_ref = %new_ref.full_ref,
        "Docker tag switched successfully"
    );

    Ok(serde_json::json!({
        "ok": true,
        "message": "Tag switched. Run a version check to update status.",
    }))
}

/// Parse a UUID parameter from JSON params.
fn parse_uuid_param(params: &serde_json::Value, key: &str) -> std::result::Result<Uuid, String> {
    let val = params
        .get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("missing required parameter '{key}'"))?;
    Uuid::parse_str(val).map_err(|e| format!("invalid UUID for '{key}': {e}"))
}

fn normalize_store_error(error: impl std::fmt::Display) -> String {
    let message = error.to_string();
    message
        .strip_prefix("plugin internal error: ")
        .unwrap_or(message.as_str())
        .to_string()
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surface_actions_returns_two() {
        let actions = surface_actions();
        assert_eq!(actions.len(), 2);
        let ids: Vec<&str> = actions.iter().map(|a| a.action_id.as_str()).collect();
        assert!(ids.contains(&"switch-tag"));
        assert!(ids.contains(&"get-current-tag"));
    }

    #[test]
    fn switch_tag_action_has_form_with_pre_load() {
        let action = switch_tag_action();
        assert!(action.ui.is_some());
        if let Some(SurfaceActionUi::Form(form)) = &action.ui {
            assert_eq!(form.pre_load_action.as_deref(), Some("get-current-tag"));
            // Hidden fields present
            let keys: Vec<&str> = form.fields.iter().map(|f| f.key.as_str()).collect();
            assert!(keys.contains(&"software_item_id"));
            assert!(keys.contains(&"host_id"));
            assert!(keys.contains(&"new_image_ref"));
        } else {
            panic!("expected Form UI");
        }
    }

    #[test]
    fn get_current_tag_action_has_no_ui() {
        let action = get_current_tag_action();
        assert!(action.ui.is_none(), "pre-load helper must have no UI");
    }

    #[test]
    fn parse_uuid_param_valid() {
        let params = serde_json::json!({ "id": "01944c3c-6a3a-7000-8000-000000000001" });
        assert!(parse_uuid_param(&params, "id").is_ok());
    }

    #[test]
    fn parse_uuid_param_missing() {
        let params = serde_json::json!({});
        assert!(parse_uuid_param(&params, "id").is_err());
    }

    #[test]
    fn parse_uuid_param_invalid() {
        let params = serde_json::json!({ "id": "not-a-uuid" });
        assert!(parse_uuid_param(&params, "id").is_err());
    }
}
