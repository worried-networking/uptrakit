use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use sea_orm::EntityTrait;
use serde_json::Value;
use uptrakit_internal_wire::surfaces;
use uptrakit_shared_types::Permission;
use uptrakit_web_api_types::surfaces::{
    InvokeSurfaceInteractionRequest, ListSurfacesQuery, SurfaceProviderAvailability,
    SurfaceProviderInfo, SurfaceResponse, SurfaceRuntimeStatusResponse,
};
use uuid::Uuid;

use uptrakit_shared_db::entity::system_service;

use crate::AppState;
use crate::error_response::{error_response, error_response_with_code};
use crate::middleware::tenant_context::TenantContext;
use crate::surface_proxy::{SurfaceCallerOrigin, SurfaceInvokeRequest, SurfaceProxyError};
use crate::surface_registry::{SurfaceCatalogItem, SurfaceRegistryLookupError};

#[tracing::instrument(skip_all)]
pub async fn list_surfaces(
    State(state): State<Arc<AppState>>,
    tenant_ctx: TenantContext,
    Query(query): Query<ListSurfacesQuery>,
) -> Response {
    let catalog = state.surface_registry.list_surfaces_for_tenant(
        tenant_ctx.tenant_id,
        query.slot.as_deref(),
        query.page.as_deref(),
    );

    (StatusCode::OK, Json(group_surface_catalog(catalog))).into_response()
}

fn group_surface_catalog(catalog: Vec<SurfaceCatalogItem>) -> Vec<SurfaceResponse> {
    let mut grouped: BTreeMap<(String, String), SurfaceResponse> = BTreeMap::new();
    for item in catalog {
        let descriptor_key =
            serde_json::to_string(&item.descriptor).expect("surface descriptor should serialize");
        let entry = grouped
            .entry((item.surface_id, descriptor_key))
            .or_insert_with(|| SurfaceResponse {
                descriptor: item.descriptor,
                provider_count: 0,
            });
        entry.provider_count += 1;
    }
    grouped.into_values().collect()
}

#[tracing::instrument(skip_all)]
pub async fn get_surface_runtime_status(State(state): State<Arc<AppState>>) -> Response {
    let snapshot = state.surface_runtime_rollout.snapshot();
    (
        StatusCode::OK,
        Json(SurfaceRuntimeStatusResponse {
            active: snapshot.active,
        }),
    )
        .into_response()
}

#[tracing::instrument(skip_all)]
pub async fn list_surface_providers(
    State(state): State<Arc<AppState>>,
    tenant_ctx: TenantContext,
    Path(surface_id): Path<String>,
) -> Response {
    let providers = state
        .surface_registry
        .list_targeted_providers_for_surface(&surface_id, tenant_ctx.tenant_id);
    if providers.is_empty() {
        return error_response_with_code(StatusCode::NOT_FOUND, "Surface not found", "not_found");
    }

    let mut response = Vec::with_capacity(providers.len());
    for provider in providers {
        let availability = if !provider.tenant_compatible {
            SurfaceProviderAvailability::IncompatibleTenant
        } else if let Some(service_id) = provider.service_id {
            if state.service_connections.is_connected(&service_id).await {
                SurfaceProviderAvailability::Available
            } else {
                SurfaceProviderAvailability::Disconnected
            }
        } else {
            SurfaceProviderAvailability::Available
        };

        let display_label = if let Some(service_id) = provider.service_id {
            match uptrakit_shared_db::entity::prelude::Service::find_by_id(service_id)
                .one(state.db())
                .await
            {
                Ok(Some(svc)) => svc.friendly_name,
                Ok(None) => {
                    match system_service::Entity::find_by_id(service_id)
                        .one(state.db())
                        .await
                    {
                        Ok(Some(sys_svc)) => sys_svc.friendly_name,
                        Ok(None) => provider
                            .service_app_name
                            .unwrap_or_else(|| provider.provider_id.clone()),
                        Err(error) => {
                            tracing::error!(%error, %service_id, "failed to look up system service");
                            return error_response(
                                StatusCode::INTERNAL_SERVER_ERROR,
                                "Internal server error",
                            );
                        }
                    }
                }
                Err(error) => {
                    tracing::error!(%error, %service_id, "failed to look up service");
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Internal server error",
                    );
                }
            }
        } else {
            provider
                .service_app_name
                .map(|name| format!("Built-in ({name})"))
                .unwrap_or_else(|| format!("Built-in ({})", provider.provider_id))
        };

        response.push(SurfaceProviderInfo {
            provider_id: provider.provider_id,
            display_label,
            service_id: provider.service_id,
            availability,
            encryption_metadata: provider.encryption_metadata,
        });
    }

    (StatusCode::OK, Json(response)).into_response()
}

#[tracing::instrument(skip_all)]
pub async fn invoke_surface_interaction(
    State(state): State<Arc<AppState>>,
    tenant_ctx: TenantContext,
    axum::Extension(auth_user): axum::Extension<crate::middleware::require_auth::AuthenticatedUser>,
    Path((surface_id, interaction_id)): Path<(String, String)>,
    Json(body): Json<InvokeSurfaceInteractionRequest>,
) -> Response {
    let resolved = match state.surface_registry.resolve_surface_action(
        tenant_ctx.tenant_id,
        &surface_id,
        &interaction_id,
        body.target_provider_id.as_deref(),
    ) {
        Ok(resolved) => resolved,
        Err(error) => return map_lookup_error(error),
    };

    for required_permission in [
        resolved.descriptor.required_permission.as_deref(),
        resolved.interaction.required_permission.as_deref(),
    ] {
        let Some(required_permission) = required_permission else {
            continue;
        };
        let Ok(permission) = required_permission.parse::<Permission>() else {
            tracing::error!(
                surface_id = %resolved.descriptor.surface_id,
                interaction_id = %resolved.interaction.interaction_id,
                permission = required_permission,
                "invalid required permission in registered surface contract"
            );
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        };
        if !auth_user.has_permission(permission) {
            return error_response_with_code(
                StatusCode::FORBIDDEN,
                "Insufficient permissions for this interaction",
                "forbidden",
            );
        }
    }

    let idempotency_key = body
        .idempotency_key
        .clone()
        .unwrap_or_else(|| Uuid::now_v7().to_string());
    let session_id = match auth_user.auth_method {
        crate::auth::AuthMethod::Password => "password".to_string(),
        crate::auth::AuthMethod::ApiToken => "api_token".to_string(),
        crate::auth::AuthMethod::Oidc { provider_id } => format!("oidc:{provider_id}"),
    };

    let request = SurfaceInvokeRequest {
        tenant_id: tenant_ctx.tenant_id,
        surface_id,
        interaction_id,
        idempotency_key,
        target_provider_id: body.target_provider_id.clone(),
        caller_origin: SurfaceCallerOrigin::UserSession {
            user_id: auth_user.user_id,
            session_id,
        },
        params: body.params.clone(),
        encrypted_sensitive_params: body.encrypted_sensitive_params.clone(),
    };
    let timeout_override = body
        .timeout_seconds
        .map(|seconds| Duration::from_secs(u64::from(seconds)));

    let result = state
        .surface_proxy
        .invoke(
            &state.service_connections,
            &state.surface_registry,
            request,
            timeout_override,
        )
        .await;

    let response = match result {
        Ok(response) => response,
        Err(error) => return map_proxy_error(error),
    };

    if response.success {
        return (StatusCode::OK, Json(response.result.unwrap_or(Value::Null))).into_response();
    }

    let (error_message, error_code) = if let Some(error) = response.error {
        (error.message, action_error_code(&error.code).to_string())
    } else {
        (
            "surface interaction failed".to_string(),
            "action_failed".to_string(),
        )
    };
    error_response_with_code(StatusCode::UNPROCESSABLE_ENTITY, error_message, error_code)
}

fn map_lookup_error(error: SurfaceRegistryLookupError) -> Response {
    match error {
        SurfaceRegistryLookupError::SurfaceNotFound => error_response_with_code(
            StatusCode::NOT_FOUND,
            "Surface not found",
            "surface_not_found",
        ),
        SurfaceRegistryLookupError::InteractionNotFound => error_response_with_code(
            StatusCode::NOT_FOUND,
            "Interaction not found",
            "interaction_not_found",
        ),
        SurfaceRegistryLookupError::TargetProviderRequired => error_response_with_code(
            StatusCode::BAD_REQUEST,
            "target_provider_id is required for targeted surfaces",
            "target_provider_required",
        ),
        SurfaceRegistryLookupError::InvalidProvider(_) => error_response_with_code(
            StatusCode::BAD_REQUEST,
            "Invalid target provider",
            "invalid_provider",
        ),
        SurfaceRegistryLookupError::NoTenantCompatibleProvider => error_response_with_code(
            StatusCode::NOT_FOUND,
            "No tenant-compatible provider available",
            "no_provider",
        ),
    }
}

fn map_proxy_error(error: SurfaceProxyError) -> Response {
    match error {
        SurfaceProxyError::NoProvider => error_response_with_code(
            StatusCode::NOT_FOUND,
            "No provider available",
            "no_provider",
        ),
        SurfaceProxyError::TargetProviderRequired => error_response_with_code(
            StatusCode::BAD_REQUEST,
            "target_provider_id is required for targeted surfaces",
            "target_provider_required",
        ),
        SurfaceProxyError::InvalidProvider(_) => error_response_with_code(
            StatusCode::BAD_REQUEST,
            "Invalid target provider",
            "invalid_provider",
        ),
        SurfaceProxyError::InteractionNotFound => error_response_with_code(
            StatusCode::NOT_FOUND,
            "Interaction not found",
            "interaction_not_found",
        ),
        SurfaceProxyError::PermissionDenied(message) => {
            error_response_with_code(StatusCode::FORBIDDEN, message, "forbidden")
        }
        SurfaceProxyError::SchemaValidationFailed(message)
        | SurfaceProxyError::SensitiveFieldRejected(message) => {
            error_response_with_code(StatusCode::UNPROCESSABLE_ENTITY, message, "invalid_request")
        }
        SurfaceProxyError::DuplicateRequest => error_response_with_code(
            StatusCode::CONFLICT,
            "Duplicate idempotency key",
            "duplicate_request",
        ),
        SurfaceProxyError::RateLimited => error_response_with_code(
            StatusCode::TOO_MANY_REQUESTS,
            "Rate limited",
            "rate_limited",
        ),
        SurfaceProxyError::ServiceDisconnected | SurfaceProxyError::SendFailed => {
            error_response_with_code(
                StatusCode::SERVICE_UNAVAILABLE,
                "Surface provider unavailable",
                "provider_unavailable",
            )
        }
        SurfaceProxyError::Timeout => error_response_with_code(
            StatusCode::GATEWAY_TIMEOUT,
            "Surface action timed out",
            "timeout",
        ),
    }
}

fn action_error_code(code: &surfaces::SurfaceActionErrorCode) -> &'static str {
    match code {
        surfaces::SurfaceActionErrorCode::PermissionDenied => "permission_denied",
        surfaces::SurfaceActionErrorCode::InvalidRequest => "invalid_request",
        surfaces::SurfaceActionErrorCode::SchemaValidationFailed => "schema_validation_failed",
        surfaces::SurfaceActionErrorCode::UnsupportedCapability => "unsupported_capability",
        surfaces::SurfaceActionErrorCode::ProviderUnavailable => "provider_unavailable",
        surfaces::SurfaceActionErrorCode::Timeout => "timeout",
        surfaces::SurfaceActionErrorCode::DuplicateRequest => "duplicate_request",
        surfaces::SurfaceActionErrorCode::InternalError => "internal_error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog_item(surface_id: &str, label: &str, provider_id: &str) -> SurfaceCatalogItem {
        SurfaceCatalogItem {
            surface_id: surface_id.to_string(),
            slot: surfaces::SLOT_SOFTWARE_TABS.to_string(),
            provider_id: provider_id.to_string(),
            targeting: surfaces::Targeting::Targeted,
            descriptor: surfaces::SurfaceDescriptor {
                surface_id: surfaces::SurfaceId::new(surface_id).unwrap(),
                label: label.to_string(),
                priority: 100,
                slot: surfaces::SLOT_SOFTWARE_TABS.to_string(),
                scope: surfaces::Scope::Tenant,
                targeting: surfaces::Targeting::Targeted,
                required_permission: Some("view_software".to_string()),
                provider_kind: surfaces::ProviderKind::Service,
                required_capabilities: surfaces::CapabilitySet::from_capabilities([
                    surfaces::Capability::TextBlockNode,
                    surfaces::Capability::TargetedTargeting,
                ]),
                root_node: surfaces::SurfaceNode::TextBlock {
                    text: "ok".to_string(),
                },
            },
        }
    }

    #[test]
    fn group_surface_catalog_merges_only_identical_descriptors() {
        let grouped = group_surface_catalog(vec![
            catalog_item("ssh.guest.panel", "SSH Guest Panel", "provider-a"),
            catalog_item("ssh.guest.panel", "SSH Guest Panel", "provider-b"),
            catalog_item("ssh.guest.panel", "Different Label", "provider-c"),
        ]);

        assert_eq!(grouped.len(), 2);
        assert_eq!(
            grouped
                .iter()
                .find(|entry| entry.descriptor.label == "SSH Guest Panel")
                .map(|entry| entry.provider_count),
            Some(2)
        );
        assert_eq!(
            grouped
                .iter()
                .find(|entry| entry.descriptor.label == "Different Label")
                .map(|entry| entry.provider_count),
            Some(1)
        );
    }
}
