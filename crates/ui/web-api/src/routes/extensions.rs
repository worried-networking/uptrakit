use std::sync::Arc;
use std::time::Duration;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use sea_orm::EntityTrait;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use uptrakit_internal_wire::extension::ExtensionManifest;
use uptrakit_plugin_infrastructure_registry::ExtensionActionContext;
use uptrakit_web_api_types::extensions::InvokeExtensionActionRequest;

use crate::AppState;
use crate::error_response::{error_response, error_response_with_code};
use crate::extension_proxy::ExtensionProxyError;
use crate::extension_registry::ExtensionOwner;

// ── Response types ──────────────────────────────────────────────────────────

/// A single extension in the list response.
#[derive(Serialize)]
pub struct ExtensionListItem {
    /// The full extension manifest.
    #[serde(flatten)]
    pub manifest: ExtensionManifest,
    /// Number of connected service instances providing this extension.
    pub provider_count: usize,
}

/// Information about a service instance that provides an extension.
#[derive(Serialize)]
pub struct ExtensionProviderInfo {
    pub service_id: Uuid,
    pub service_label: String,
    pub hostname: Option<String>,
    /// Base64-encoded uncompressed P-256 public key for ECIES encryption.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encryption_public_key: Option<String>,
}

/// Query parameters for the invoke action endpoint.
#[derive(Deserialize)]
pub struct InvokeActionQuery {
    pub service_id: Option<Uuid>,
}

// ── Default timeout ─────────────────────────────────────────────────────────

const DEFAULT_ACTION_TIMEOUT_SECS: u64 = 30;

// ── Endpoints ───────────────────────────────────────────────────────────────

/// List all active extension manifests.
#[tracing::instrument(skip_all)]
pub async fn list_extensions(State(state): State<Arc<AppState>>) -> Response {
    let manifests = state.extension_registry.all_manifests();
    let items: Vec<ExtensionListItem> = manifests
        .into_iter()
        .map(|manifest| {
            let provider_count = state.extension_registry.providers(&manifest.id).len();
            ExtensionListItem {
                manifest,
                provider_count,
            }
        })
        .collect();

    (StatusCode::OK, Json(items)).into_response()
}

/// List connected service instances that provide a specific extension.
#[tracing::instrument(skip_all)]
pub async fn list_extension_providers(
    State(state): State<Arc<AppState>>,
    Path(extension_id): Path<String>,
) -> Response {
    let owner = state.extension_registry.find_owner(&extension_id);

    match owner {
        ExtensionOwner::NotFound => {
            error_response_with_code(StatusCode::NOT_FOUND, "Extension not found", "not_found")
        }
        ExtensionOwner::Plugin => {
            // Plugin-backed extensions have no service providers.
            (StatusCode::OK, Json(Vec::<ExtensionProviderInfo>::new())).into_response()
        }
        ExtensionOwner::Service { providers } => {
            let mut infos = Vec::with_capacity(providers.len());

            for service_id in providers {
                match uptrakit_shared_db::entity::prelude::Service::find_by_id(service_id)
                    .one(&state.db)
                    .await
                {
                    Ok(Some(svc)) => {
                        let encryption_public_key =
                            state.extension_registry.encryption_public_key(&service_id);
                        infos.push(ExtensionProviderInfo {
                            service_id,
                            service_label: svc.friendly_name,
                            hostname: Some(svc.hostname),
                            encryption_public_key,
                        });
                    }
                    Ok(None) => {
                        // Service no longer in DB; include with minimal info.
                        let encryption_public_key =
                            state.extension_registry.encryption_public_key(&service_id);
                        infos.push(ExtensionProviderInfo {
                            service_id,
                            service_label: service_id.to_string(),
                            hostname: None,
                            encryption_public_key,
                        });
                    }
                    Err(e) => {
                        tracing::error!("Failed to look up service {service_id}: {e}");
                        return error_response(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "Internal server error",
                        );
                    }
                }
            }

            (StatusCode::OK, Json(infos)).into_response()
        }
    }
}

/// Invoke an extension action, proxying the request to a connected service.
#[tracing::instrument(skip_all)]
pub async fn invoke_action(
    State(state): State<Arc<AppState>>,
    Path((extension_id, action_id)): Path<(String, String)>,
    Query(query): Query<InvokeActionQuery>,
    Json(body): Json<InvokeExtensionActionRequest>,
) -> Response {
    let owner = state.extension_registry.find_owner(&extension_id);

    match owner {
        ExtensionOwner::NotFound => {
            return error_response_with_code(
                StatusCode::NOT_FOUND,
                "Extension not found",
                "extension_not_found",
            );
        }
        ExtensionOwner::Plugin => {
            let ctx = ExtensionActionContext {
                db: state.db(),
                tenant_id: None, // TODO: extract from auth context when tenant-scoped
            };
            return match state
                .plugin_ops
                .handle_extension_action(&ctx, &extension_id, &action_id, body.params.clone())
                .await
            {
                Ok(data) => (StatusCode::OK, Json(data)).into_response(),
                Err(msg) => {
                    error_response_with_code(StatusCode::UNPROCESSABLE_ENTITY, msg, "action_failed")
                }
            };
        }
        ExtensionOwner::Service { .. } => {
            // Proceed below.
        }
    }

    // Determine timeout from the manifest's action definition, or use default.
    let timeout = resolve_action_timeout(&state, &extension_id, &action_id);

    // For targeted extensions, a service_id query param is required.
    let manifests = state.extension_registry.all_manifests();
    let manifest = manifests.iter().find(|m| m.id == extension_id);
    if let Some(m) = manifest
        && matches!(
            m.targeting,
            uptrakit_internal_wire::extension::ExtensionTargeting::Targeted
        )
        && query.service_id.is_none()
    {
        return error_response_with_code(
            StatusCode::BAD_REQUEST,
            "Targeted extension requires service_id query parameter",
            "missing_service_id",
        );
    }

    match state
        .extension_proxy
        .invoke(
            &state.service_connections,
            &state.extension_registry,
            &extension_id,
            &action_id,
            body.params,
            body.sensitive_params,
            query.service_id,
            timeout,
        )
        .await
    {
        Ok(response) => {
            if response.success {
                (StatusCode::OK, Json(response.data)).into_response()
            } else {
                let msg = response
                    .error
                    .unwrap_or_else(|| "Action returned failure".to_string());
                error_response_with_code(StatusCode::UNPROCESSABLE_ENTITY, msg, "action_failed")
            }
        }
        Err(ExtensionProxyError::NoProvider | ExtensionProxyError::InvalidProvider(_)) => {
            error_response_with_code(
                StatusCode::NOT_FOUND,
                "No available provider for this extension",
                "no_provider",
            )
        }
        Err(ExtensionProxyError::ServiceDisconnected | ExtensionProxyError::SendFailed) => {
            error_response_with_code(
                StatusCode::SERVICE_UNAVAILABLE,
                "Service is not connected",
                "service_disconnected",
            )
        }
        Err(ExtensionProxyError::Timeout) => error_response_with_code(
            StatusCode::GATEWAY_TIMEOUT,
            "Service did not respond in time",
            "timeout",
        ),
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Resolves the timeout for an action invocation.
///
/// Searches the manifest for an `ActionDef` matching `action_id` and uses its
/// `timeout_seconds` if present. Otherwise returns the default 30-second timeout.
fn resolve_action_timeout(state: &AppState, extension_id: &str, action_id: &str) -> Duration {
    let manifests = state.extension_registry.all_manifests();
    let manifest = match manifests.iter().find(|m| m.id == extension_id) {
        Some(m) => m,
        None => return Duration::from_secs(DEFAULT_ACTION_TIMEOUT_SECS),
    };

    let timeout_secs =
        find_action_timeout(&manifest.ui, action_id).unwrap_or(DEFAULT_ACTION_TIMEOUT_SECS);

    Duration::from_secs(timeout_secs)
}

/// Searches an `ExtensionUi` for an `ActionDef` with the given `action_id`
/// and returns its `timeout_seconds` if set.
fn find_action_timeout(
    ui: &uptrakit_internal_wire::extension::ExtensionUi,
    action_id: &str,
) -> Option<u64> {
    use uptrakit_internal_wire::extension::ExtensionUi;

    let actions: Vec<&uptrakit_internal_wire::extension::ActionDef> = match ui {
        ExtensionUi::DataTable {
            row_actions,
            primary_actions,
            ..
        } => row_actions.iter().chain(primary_actions.iter()).collect(),
        ExtensionUi::Actions { actions } => actions.iter().collect(),
        _ => {
            tracing::warn!("Unknown ExtensionUi variant when resolving action timeout");
            return None;
        }
    };

    actions
        .into_iter()
        .find(|a| a.action_id == action_id)
        .and_then(|a| a.timeout_seconds)
        .map(u64::from)
}
