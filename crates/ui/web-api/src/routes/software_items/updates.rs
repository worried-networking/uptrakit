use crate::AppState;
use crate::api_error::ApiError;
use crate::extract::Unvalidated;
use crate::middleware::action::CanTriggerUpdates;
use crate::middleware::require_auth::AuthenticatedApiTokenId;
use crate::queries::update_types::ActorType;
use crate::tenant_db::TenantDb;
use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use std::sync::Arc;
use uuid::Uuid;

use super::{TriggerUpdateRequest, TriggerUpdateResponse, TriggerUpdateStatus};

/// Trigger a software update for a specific host.
#[utoipa::path(
    post,
    path = "/api/v1/software-items/{id}/hosts/{host_id}/update",
    params(
        ("id" = Uuid, Path, description = "Software item UUID"),
        ("host_id" = Uuid, Path, description = "Host UUID")
    ),
    request_body = TriggerUpdateRequest,
    responses(
        (status = 200, description = "Update triggered", body = TriggerUpdateResponse),
        (status = 400, description = "Invalid input or validation failed"),
        (status = 404, description = "Software item, host, or agent not found"),
        (status = 409, description = "Update already in progress")
    ),
    tag = "Software Items",
    security(("oauth2" = ["updates:trigger"]), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn trigger_update(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanTriggerUpdates(user): CanTriggerUpdates,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Path((item_id, host_id)): Path<(Uuid, Uuid)>,
    body: Unvalidated<TriggerUpdateRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let req = match body.require_valid() {
        Ok(req) => req,
        Err(e) => {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                e.to_string(),
                "validation_error",
                None,
            ));
        }
    };
    let api_token_id = api_token_id.map(|value| value.0);
    let (update_actor_type, update_actor_id) = match api_token_id {
        Some(token_id) => (ActorType::ApiToken, token_id.0.to_string()),
        None => (ActorType::User, user.user_id.to_string()),
    };
    let to_version = req.to_version.clone();
    let interactive = req.interactive;
    // Convert the API release_info type to the wire type then serialise for
    // UpdateDispatchParams, which carries an opaque JSON value.
    let release_info = req.release_info.map(|ri| {
        let wire = uptrakit_wire::ReleaseInfo {
            tag: ri.tag,
            release_url: ri.release_url,
            assets: ri
                .assets
                .into_iter()
                .map(|a| uptrakit_wire::ReleaseAsset {
                    name: a.name,
                    download_url: a.download_url,
                    size: a.size,
                    content_type: None,
                    sha256_digest: None,
                })
                .collect(),
            // Attestation fields are enriched server-side at dispatch time from
            // latest_release_metadata and the fetch_releases plugin config.
            attestation_status: None,
            require_attestation: false,
        };
        serde_json::to_value(wire).unwrap_or_else(|e| {
            tracing::warn!("failed to serialize release info: {e}");
            serde_json::Value::Null
        })
    });

    let result = state
        .update_dispatcher
        .dispatch(uptrakit_controller_core::update::UpdateDispatchParams::new(
            tenant_db.tenant_id(),
            host_id,
            item_id,
            to_version.clone(),
            uptrakit_controller_core::update::ActorInfo::new(update_actor_type, update_actor_id),
            release_info,
            interactive,
        ))
        .await?;

    let status = match result.outcome {
        uptrakit_controller_core::update::DispatchOutcome::Sent => TriggerUpdateStatus::Pending,
        uptrakit_controller_core::update::DispatchOutcome::Queued => TriggerUpdateStatus::Queued,
        uptrakit_controller_core::update::DispatchOutcome::Failed => TriggerUpdateStatus::Failed,
        _ => {
            tracing::warn!("unhandled DispatchOutcome in HTTP trigger_update response mapping");
            TriggerUpdateStatus::Failed
        }
    };

    let resp = TriggerUpdateResponse {
        update_history_id: result.update_history_id,
        status,
    };
    Ok((StatusCode::OK, Json(resp)).into_response())
}
