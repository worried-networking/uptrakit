//! Surface action handler dispatch for the Docker plugin.
//!
//! ## Action flow
//!
//! 1. The frontend opens the context menu on a host row that has a Docker
//!    plugin assignment.
//! 2. The user clicks **Switch Tag** — the form pre-loads the current image
//!    reference (without `#container` suffix) via `current-tag`.
//! 3. The user edits the tag and submits — `switch-tag` updates all
//!    `host_software_item_plugin` rows and the `host_software_item` row,
//!    then clears stale version data so the next check reflects the new tag.
//!
//! All Docker-specific logic (`ImageRef` parsing, `#container` suffix handling,
//! `validate_identifier` SSRF guard) lives here and does not leak to callers.

use std::future::Future;
use std::pin::Pin;
use uptrakit_shared_db::begin_immediate;

use serde::de::DeserializeOwned;

use uptrakit_plugin_infrastructure_core::{SurfaceActionContext, SurfaceActionError};

use crate::image_ref::{ImageRef, validate_identifier};

// ── Docker surface-action request types ─────────────────────────────────────

/// Typed host/software-item request for the `current-tag` surface action.
#[derive(Debug, serde::Deserialize)]
struct DockerItemHostRequest {
    pub host_id: uuid::Uuid,
    pub software_item_id: uuid::Uuid,
}

/// Typed switch-tag request for the `switch-tag` surface action.
#[derive(Debug, serde::Deserialize)]
struct DockerSwitchTagRequest {
    pub host_id: uuid::Uuid,
    pub software_item_id: uuid::Uuid,
    pub new_image_ref: String,
}

// ── String helpers ───────────────────────────────────────────────────────────

/// Docker releases plugin type identifier used as a DB filter value.
const DOCKER_RELEASES_CONFIG_TYPE: &str = "releases.docker";

/// Return the image reference without the `#container_name` suffix.
fn strip_container_suffix(id: &str) -> String {
    id.split_once('#')
        .map(|(base, _)| base)
        .unwrap_or(id)
        .to_string()
}

/// Return the container name from the `#container_name` suffix, if present.
fn extract_container_suffix(id: &str) -> Option<&str> {
    id.split_once('#').map(|(_, suffix)| suffix)
}

// ── Public API ───────────────────────────────────────────────────────────────

/// Dispatch shim for the `switch-tag` interaction (exact-id dispatch map entry).
pub(crate) fn docker_switch_tag_handler<'a>(
    ctx: &'a SurfaceActionContext<'a>,
    params: serde_json::Value,
) -> Pin<
    Box<
        dyn Future<Output = std::result::Result<serde_json::Value, SurfaceActionError>> + Send + 'a,
    >,
> {
    Box::pin(async move {
        let request = parse_action_params::<DockerSwitchTagRequest>(params, "switch-tag")?;
        handle_switch_tag(ctx, request).await
    })
}

/// Dispatch shim for the `current-tag` interaction (exact-id dispatch map entry).
pub(crate) fn docker_get_current_tag_handler<'a>(
    ctx: &'a SurfaceActionContext<'a>,
    params: serde_json::Value,
) -> Pin<
    Box<
        dyn Future<Output = std::result::Result<serde_json::Value, SurfaceActionError>> + Send + 'a,
    >,
> {
    Box::pin(async move {
        let request = parse_action_params::<DockerItemHostRequest>(params, "current-tag")?;
        handle_get_current_tag(ctx, request).await
    })
}

// ── Action handlers ──────────────────────────────────────────────────────────

/// Pre-load handler: return the current image reference (without `#container`
/// suffix) so the Switch Tag form can pre-populate the `new_image_ref` field.
async fn handle_get_current_tag(
    ctx: &SurfaceActionContext<'_>,
    request: DockerItemHostRequest,
) -> std::result::Result<serde_json::Value, SurfaceActionError> {
    use sea_orm::ColumnTrait as _;
    use sea_orm::EntityTrait as _;
    use sea_orm::QueryFilter as _;
    use uptrakit_shared_db::entity::host_software_item_plugin;

    let host_id = request.host_id;
    let software_item_id = request.software_item_id;

    tracing::debug!(%host_id, %software_item_id, "fetching current Docker tag");

    let db = ctx.tenant_db().db();
    let plugin_rows = host_software_item_plugin::Entity::find()
        .filter(host_software_item_plugin::Column::HostId.eq(host_id))
        .filter(host_software_item_plugin::Column::SoftwareItemId.eq(software_item_id))
        .filter(host_software_item_plugin::Column::PluginType.eq(DOCKER_RELEASES_CONFIG_TYPE))
        .all(db)
        .await
        .map_err(|e| SurfaceActionError::ControllerIntegration(e.to_string()))?;

    let image_ref = plugin_rows
        .into_iter()
        .next()
        .map(|r| strip_container_suffix(&r.package_identifier))
        .unwrap_or_default();

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
    request: DockerSwitchTagRequest,
) -> std::result::Result<serde_json::Value, SurfaceActionError> {
    use sea_orm::{
        ActiveModelTrait as _, ColumnTrait as _, EntityTrait as _, QueryFilter as _, Set,
    };
    use uptrakit_shared_db::entity::{host_software_item, host_software_item_plugin};

    let host_id = request.host_id;
    let software_item_id = request.software_item_id;
    let new_image_ref = request.new_image_ref.trim().to_string();

    tracing::debug!(%host_id, %software_item_id, new_image_ref = %new_image_ref, "switching Docker tag");

    // Validate format first (parses the image ref) then SSRF check.
    new_image_ref
        .parse::<ImageRef>()
        .map_err(|e| SurfaceActionError::InvalidInput(format!("invalid image reference: {e}")))?;
    validate_identifier(&new_image_ref)
        .map_err(|e| SurfaceActionError::InvalidInput(format!("invalid image reference: {e}")))?;

    let db = ctx.tenant_db().db();

    // Use BEGIN IMMEDIATE so SQLite promotes to RESERVED lock before the first read,
    // preventing SQLITE_BUSY_SNAPSHOT when another connection commits mid-transaction.
    let txn = begin_immediate(db).await.map_err(|e| {
        SurfaceActionError::ControllerIntegration(format!("failed to begin transaction: {e}"))
    })?;

    let plugin_rows = host_software_item_plugin::Entity::find()
        .filter(host_software_item_plugin::Column::HostId.eq(host_id))
        .filter(host_software_item_plugin::Column::SoftwareItemId.eq(software_item_id))
        .all(&txn)
        .await
        .map_err(|e| {
            SurfaceActionError::ControllerIntegration(format!(
                "database error loading plugin rows: {e}"
            ))
        })?;

    if plugin_rows.is_empty() {
        return Err(SurfaceActionError::ControllerIntegration(
            "no plugin assignments found for this host".to_string(),
        ));
    }

    let hsi_row = host_software_item::Entity::find()
        .filter(host_software_item::Column::HostId.eq(host_id))
        .filter(host_software_item::Column::SoftwareItemId.eq(software_item_id))
        .one(&txn)
        .await
        .map_err(|e| {
            SurfaceActionError::ControllerIntegration(format!(
                "database error loading host_software_item: {e}"
            ))
        })?
        .ok_or_else(|| {
            SurfaceActionError::ControllerIntegration(format!(
                "host_software_item not found for host {host_id} / item {software_item_id}"
            ))
        })?;

    for row in plugin_rows {
        if row.plugin_type != DOCKER_RELEASES_CONFIG_TYPE {
            continue;
        }
        let new_pkg_id = match extract_container_suffix(&row.package_identifier) {
            Some(container) => format!("{new_image_ref}#{container}"),
            None => new_image_ref.clone(),
        };
        let mut active: host_software_item_plugin::ActiveModel = row.into();
        active.package_identifier = Set(new_pkg_id);
        active.update(&txn).await.map_err(|e| {
            SurfaceActionError::ControllerIntegration(format!("failed to update plugin row: {e}"))
        })?;
    }

    let mut hsi_active: host_software_item::ActiveModel = hsi_row.into();
    hsi_active.package_identifier = Set(Some(new_image_ref.clone()));
    hsi_active.installed_version = Set(None);
    hsi_active.installed_display_version = Set(None);
    hsi_active.installed_version_detected_at = Set(None);
    hsi_active.latest_version = Set(None);
    hsi_active.latest_version_fetched_at = Set(None);
    hsi_active.latest_release_metadata = Set(None);
    hsi_active.update_category = Set("unknown".to_string());
    hsi_active.update(&txn).await.map_err(|e| {
        SurfaceActionError::ControllerIntegration(format!(
            "failed to update host_software_item: {e}"
        ))
    })?;

    txn.commit().await.map_err(|e| {
        SurfaceActionError::ControllerIntegration(format!("failed to commit transaction: {e}"))
    })?;

    tracing::info!(
        %host_id,
        %software_item_id,
        %new_image_ref,
        "Docker tag switched successfully"
    );

    Ok(serde_json::json!({
        "ok": true,
        "message": "Tag switched. Run a version check to update status.",
    }))
}

fn parse_action_params<T>(
    params: serde_json::Value,
    action_id: &str,
) -> Result<T, SurfaceActionError>
where
    T: DeserializeOwned,
{
    serde_json::from_value(params).map_err(|error| {
        SurfaceActionError::InvalidInput(format!(
            "invalid params for action '{action_id}': {error}"
        ))
    })
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_plugin_infrastructure_core::InteractionDeliveryKind;

    use crate::plugin::docker_plugin_surfaces;

    #[test]
    fn docker_plugin_surfaces_pair_every_interaction_with_plugin_handled_delivery() {
        let registrations = docker_plugin_surfaces();
        let mut seen: Vec<(String, String, InteractionDeliveryKind)> = Vec::new();
        for registration in &registrations {
            for surface in &registration.surfaces {
                for interaction in &surface.interactions {
                    assert_eq!(
                        interaction.descriptor().transport,
                        uptrakit_plugin_infrastructure_core::surfaces::InteractionTransport::ControllerLocal
                    );
                    seen.push((
                        surface.descriptor.surface_id.as_str().to_string(),
                        interaction.descriptor().interaction_id.as_str().to_string(),
                        interaction.delivery().kind(),
                    ));
                }
            }
        }
        let expected: Vec<(&str, &str, InteractionDeliveryKind)> = vec![
            (
                "docker.item-host-actions",
                "switch-tag",
                InteractionDeliveryKind::PluginHandled,
            ),
            (
                "docker.item-host-actions",
                "current-tag",
                InteractionDeliveryKind::PluginHandled,
            ),
        ];
        for (surface, id, kind) in &expected {
            assert!(
                seen.iter()
                    .any(|(s, i, k)| s == surface && i == id && k == kind),
                "missing ({surface}, {id}, {kind:?})"
            );
        }
        assert_eq!(
            seen.len(),
            expected.len(),
            "unexpected total interaction count across docker_plugin_surfaces()"
        );
    }

    #[test]
    fn parse_action_params_switch_tag_valid() {
        let params = serde_json::json!({
            "host_id": "01944c3c-6a3a-7000-8000-000000000001",
            "software_item_id": "01944c3c-6a3a-7000-8000-000000000002",
            "new_image_ref": "ghcr.io/example/app:1.2.3",
        });
        let parsed = parse_action_params::<DockerSwitchTagRequest>(params, "switch-tag")
            .expect("request should parse");
        assert_eq!(parsed.new_image_ref, "ghcr.io/example/app:1.2.3");
    }

    #[test]
    fn parse_action_params_get_current_tag_missing_field_is_invalid_input() {
        let params = serde_json::json!({
            "software_item_id": "01944c3c-6a3a-7000-8000-000000000002",
        });
        let error = parse_action_params::<DockerItemHostRequest>(params, "current-tag")
            .expect_err("request must fail");
        assert!(
            error
                .to_string()
                .contains("invalid params for action 'current-tag'"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn parse_action_params_switch_tag_invalid_uuid_is_invalid_input() {
        let params = serde_json::json!({
            "host_id": "not-a-uuid",
            "software_item_id": "01944c3c-6a3a-7000-8000-000000000002",
            "new_image_ref": "ghcr.io/example/app:1.2.3",
        });
        let error = parse_action_params::<DockerSwitchTagRequest>(params, "switch-tag")
            .expect_err("request must fail");
        assert!(
            error
                .to_string()
                .contains("invalid params for action 'switch-tag'"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn strip_container_suffix_with_suffix() {
        assert_eq!(
            strip_container_suffix("ghcr.io/example/app:1.0#web"),
            "ghcr.io/example/app:1.0"
        );
    }

    #[test]
    fn strip_container_suffix_without_suffix() {
        assert_eq!(
            strip_container_suffix("ghcr.io/example/app:1.0"),
            "ghcr.io/example/app:1.0"
        );
    }

    #[test]
    fn extract_container_suffix_with_suffix() {
        assert_eq!(
            extract_container_suffix("ghcr.io/example/app:1.0#web"),
            Some("web")
        );
    }

    #[test]
    fn extract_container_suffix_without_suffix() {
        assert_eq!(extract_container_suffix("ghcr.io/example/app:1.0"), None);
    }
}
