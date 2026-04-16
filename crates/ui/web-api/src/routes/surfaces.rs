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
    SurfaceProviderInfo, SurfaceReadResponse, SurfaceResponse, SurfaceRuntimeStatusResponse,
};
use uuid::Uuid;

use uptrakit_shared_db::entity::system_service;

use crate::AppState;
use crate::error_response::{error_response, error_response_with_code};
use crate::middleware::require_auth::AuthenticatedUser;
use crate::middleware::tenant_context::TenantContext;
use crate::surface_proxy::{SurfaceCallerOrigin, SurfaceInvokeRequest, SurfaceProxyError};
use crate::surface_registry::{SurfaceCatalogItem, SurfaceRegistryLookupError};

#[tracing::instrument(skip_all)]
pub async fn list_surfaces(
    State(state): State<Arc<AppState>>,
    tenant_ctx: TenantContext,
    Query(query): Query<ListSurfacesQuery>,
) -> Response {
    if !state.surface_runtime_rollout.snapshot().active {
        return (StatusCode::OK, Json(Vec::<SurfaceResponse>::new())).into_response();
    }

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
    if !state.surface_runtime_rollout.snapshot().active {
        return error_response_with_code(StatusCode::NOT_FOUND, "Surface not found", "not_found");
    }

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
pub async fn get_surface_read(
    State(state): State<Arc<AppState>>,
    tenant_ctx: TenantContext,
    axum::Extension(auth_user): axum::Extension<AuthenticatedUser>,
    Path(surface_id): Path<String>,
) -> Response {
    if !state.surface_runtime_rollout.snapshot().active {
        return surface_runtime_inactive_response();
    }

    let resolved = match state
        .surface_registry
        .resolve_surface_read(tenant_ctx.tenant_id, &surface_id)
    {
        Ok(resolved) => resolved,
        Err(error) => return map_lookup_error(error),
    };

    if let Some(response) = enforce_required_permission(
        resolved.descriptor.required_permission.as_deref(),
        &auth_user,
        &surface_id,
        "surface",
    ) {
        return response;
    }

    (
        StatusCode::OK,
        Json(SurfaceReadResponse {
            descriptor: resolved.descriptor,
            interactions: resolved.interactions,
            data_sources: resolved.data_sources,
        }),
    )
        .into_response()
}

#[tracing::instrument(skip_all)]
pub async fn invoke_surface_interaction(
    State(state): State<Arc<AppState>>,
    tenant_ctx: TenantContext,
    axum::Extension(auth_user): axum::Extension<crate::middleware::require_auth::AuthenticatedUser>,
    Path((surface_id, interaction_id)): Path<(String, String)>,
    Json(body): Json<InvokeSurfaceInteractionRequest>,
) -> Response {
    if !state.surface_runtime_rollout.snapshot().active {
        return surface_runtime_inactive_response();
    }

    let resolved = match state.surface_registry.resolve_surface_action(
        tenant_ctx.tenant_id,
        &surface_id,
        &interaction_id,
        body.target_provider_id.as_deref(),
    ) {
        Ok(resolved) => resolved,
        Err(error) => return map_lookup_error(error),
    };

    if let Some(response) = enforce_required_permission(
        resolved.descriptor.required_permission.as_deref(),
        &auth_user,
        &surface_id,
        "interaction",
    ) {
        return response;
    }
    if let Some(response) = enforce_required_permission(
        resolved.interaction.required_permission.as_deref(),
        &auth_user,
        &surface_id,
        "interaction",
    ) {
        return response;
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
        .invoke_with_rollout(
            &state.service_connections,
            &state.surface_registry,
            &state.surface_runtime_rollout,
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
        SurfaceProxyError::RuntimeInactive => surface_runtime_inactive_response(),
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
        SurfaceProxyError::Conflict { message, code } => {
            error_response_with_code(StatusCode::CONFLICT, message, code)
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

fn surface_runtime_inactive_response() -> Response {
    error_response_with_code(
        StatusCode::SERVICE_UNAVAILABLE,
        "Shared surface runtime is inactive",
        "surface_runtime_inactive",
    )
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

fn enforce_required_permission(
    required_permission: Option<&str>,
    auth_user: &AuthenticatedUser,
    surface_id: &str,
    access_kind: &'static str,
) -> Option<Response> {
    let required_permission = required_permission?;
    let Ok(permission) = required_permission.parse::<Permission>() else {
        tracing::error!(
            surface_id = %surface_id,
            permission = required_permission,
            "invalid required permission in registered surface contract"
        );
        return Some(error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Internal server error",
        ));
    };
    if auth_user.has_permission(permission) {
        return None;
    }
    Some(error_response_with_code(
        StatusCode::FORBIDDEN,
        format!("Insufficient permissions for this {access_kind}"),
        "forbidden",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::AuthMethod;
    use crate::auth::permissions::Permission as AuthPermission;
    use crate::auth::registration::{RegistrationMode, RegistrationSettings};
    use crate::ca_snapshot::{CaKeyStore, CaPublicSnapshot, TrustedCaPublic};
    use crate::cert_signer::{AgentCertSigner, CertSignerError, SignedCertBundle};
    use crate::middleware::require_auth::AuthenticatedUser;
    use crate::{AppState, ServiceCredentialSources};
    use axum::body::to_bytes;
    use std::collections::BTreeMap;
    use std::sync::Arc;
    use time::{Duration as TimeDuration, OffsetDateTime};
    use uptrakit_internal_wire::ControllerMessage;
    use uptrakit_web_api_types::error::ErrorResponse;

    fn auth_user_with_permissions(permissions: Vec<AuthPermission>) -> AuthenticatedUser {
        AuthenticatedUser {
            user_id: Uuid::nil(),
            auth_method: AuthMethod::Password,
            permissions,
        }
    }

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

    #[test]
    fn enforce_required_permission_accepts_missing_permission() {
        let auth_user = auth_user_with_permissions(vec![]);
        let response = enforce_required_permission(None, &auth_user, "surface.one", "surface");
        assert!(response.is_none());
    }

    #[test]
    fn enforce_required_permission_rejects_missing_user_permission() {
        let auth_user = auth_user_with_permissions(vec![]);
        let response = enforce_required_permission(
            Some("view_software"),
            &auth_user,
            "surface.one",
            "surface",
        )
        .expect("permission check should reject missing permission");
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[test]
    fn enforce_required_permission_accepts_granted_permission() {
        let auth_user = auth_user_with_permissions(vec![AuthPermission::ViewSoftware]);
        let response = enforce_required_permission(
            Some("view_software"),
            &auth_user,
            "surface.one",
            "surface",
        );
        assert!(response.is_none());
    }

    #[test]
    fn enforce_required_permission_rejects_invalid_permission_strings() {
        let auth_user = auth_user_with_permissions(vec![AuthPermission::ViewSoftware]);
        let response = enforce_required_permission(
            Some("invalid_permission"),
            &auth_user,
            "surface.one",
            "surface",
        )
        .expect("invalid permissions must be rejected");
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    struct NoopCertSigner;

    #[async_trait::async_trait]
    impl AgentCertSigner for NoopCertSigner {
        async fn sign_agent_csr(
            &self,
            _: &str,
            _: &uuid::Uuid,
            _: time::Duration,
        ) -> std::result::Result<SignedCertBundle, rootcause::Report<CertSignerError>> {
            Err(rootcause::report!(CertSignerError::Signing(
                "noop signer".to_string()
            )))
        }

        fn active_ca_fingerprint(&self) -> String {
            "0".repeat(64)
        }
    }

    fn service_surface_registration(
        provider_id: &str,
        tenant_id: Uuid,
    ) -> surfaces::SurfaceRegistration {
        surfaces::SurfaceRegistration {
            provider: surfaces::ProviderIdentity {
                provider_id: provider_id.to_string(),
                provider_kind: surfaces::ProviderKind::Service,
                provider_namespace: "service".to_string(),
            },
            framework_generation: surfaces::FrameworkGeneration::new(1, 0),
            capabilities: surfaces::CapabilitySet::from_capabilities([
                surfaces::Capability::TextBlockNode,
                surfaces::Capability::TargetedTargeting,
                surfaces::Capability::MutationAction,
                surfaces::Capability::ProviderInitiatedActions,
            ]),
            effective_tenant_binding: surfaces::EffectiveTenantBinding {
                scope: surfaces::Scope::Tenant,
                tenant_id: Some(tenant_id.to_string()),
            },
            surfaces: vec![surfaces::RegisteredSurface {
                descriptor: surfaces::SurfaceDescriptor {
                    surface_id: surfaces::SurfaceId::new("ssh.guest.panel").unwrap(),
                    label: "SSH Guest Panel".to_string(),
                    priority: 100,
                    slot: surfaces::SLOT_SOFTWARE_TABS.to_string(),
                    scope: surfaces::Scope::Tenant,
                    targeting: surfaces::Targeting::Targeted,
                    required_permission: Some("view_software".to_string()),
                    provider_kind: surfaces::ProviderKind::Service,
                    required_capabilities: surfaces::CapabilitySet::from_capabilities([
                        surfaces::Capability::TextBlockNode,
                        surfaces::Capability::TargetedTargeting,
                        surfaces::Capability::MutationAction,
                    ]),
                    root_node: surfaces::SurfaceNode::TextBlock {
                        text: "ok".to_string(),
                    },
                },
                interactions: vec![surfaces::InteractionDescriptor {
                    interaction_id: surfaces::InteractionId::new("refresh").unwrap(),
                    kind: surfaces::InteractionKind::MutationAction,
                    label: None,
                    required_permission: Some("update_software".to_string()),
                    input_schema: Some(surfaces::SchemaContract::Object),
                    result_schema: Some(surfaces::SchemaContract::Object),
                    sensitive_fields: vec![],
                    timeout_seconds: Some(5),
                    confirmation: None,
                    transport: surfaces::InteractionTransport::ProviderProxied,
                    workflow_steps: vec![],
                    form_ui: None,
                }],
                data_sources: vec![],
            }],
            encryption_metadata: None,
        }
    }

    fn plugin_surface_registration() -> surfaces::SurfaceRegistration {
        surfaces::SurfaceRegistration {
            provider: surfaces::ProviderIdentity {
                provider_id: "plugin.notifications_email".to_string(),
                provider_kind: surfaces::ProviderKind::Plugin,
                provider_namespace: "plugin".to_string(),
            },
            framework_generation: surfaces::FrameworkGeneration::new(1, 0),
            capabilities: surfaces::CapabilitySet::from_capabilities([
                surfaces::Capability::TextBlockNode,
                surfaces::Capability::UniversalTargeting,
                surfaces::Capability::MutationAction,
            ]),
            effective_tenant_binding: surfaces::EffectiveTenantBinding {
                scope: surfaces::Scope::Global,
                tenant_id: None,
            },
            surfaces: vec![surfaces::RegisteredSurface {
                descriptor: surfaces::SurfaceDescriptor {
                    surface_id: surfaces::SurfaceId::new("notifications.email.global_smtp")
                        .unwrap(),
                    label: "SMTP Defaults".to_string(),
                    priority: 100,
                    slot: surfaces::SLOT_SETTINGS_BELOW_GLOBAL.to_string(),
                    scope: surfaces::Scope::Global,
                    targeting: surfaces::Targeting::Universal,
                    required_permission: None,
                    provider_kind: surfaces::ProviderKind::Plugin,
                    required_capabilities: surfaces::CapabilitySet::from_capabilities([
                        surfaces::Capability::TextBlockNode,
                        surfaces::Capability::UniversalTargeting,
                        surfaces::Capability::MutationAction,
                    ]),
                    root_node: surfaces::SurfaceNode::TextBlock {
                        text: "ok".to_string(),
                    },
                },
                interactions: vec![surfaces::InteractionDescriptor {
                    interaction_id: surfaces::InteractionId::new("save_global_smtp").unwrap(),
                    kind: surfaces::InteractionKind::MutationAction,
                    label: None,
                    required_permission: None,
                    input_schema: Some(surfaces::SchemaContract::Object),
                    result_schema: Some(surfaces::SchemaContract::Object),
                    sensitive_fields: vec![],
                    timeout_seconds: Some(5),
                    confirmation: None,
                    transport: surfaces::InteractionTransport::ControllerLocal,
                    workflow_steps: vec![],
                    form_ui: None,
                }],
                data_sources: vec![],
            }],
            encryption_metadata: None,
        }
    }

    fn plugin_shared_surface_registration(provider_id: &str) -> surfaces::SurfaceRegistration {
        surfaces::SurfaceRegistration {
            provider: surfaces::ProviderIdentity {
                provider_id: provider_id.to_string(),
                provider_kind: surfaces::ProviderKind::Plugin,
                provider_namespace: "plugin".to_string(),
            },
            framework_generation: surfaces::FrameworkGeneration::new(1, 0),
            capabilities: surfaces::CapabilitySet::from_capabilities([
                surfaces::Capability::TextBlockNode,
                surfaces::Capability::UniversalTargeting,
                surfaces::Capability::MutationAction,
            ]),
            effective_tenant_binding: surfaces::EffectiveTenantBinding {
                scope: surfaces::Scope::Tenant,
                tenant_id: Some(Uuid::nil().to_string()),
            },
            surfaces: vec![surfaces::RegisteredSurface {
                descriptor: surfaces::SurfaceDescriptor {
                    surface_id: surfaces::SurfaceId::new("ssh.guest.panel").unwrap(),
                    label: "Plugin SSH Guest Panel".to_string(),
                    priority: 100,
                    slot: surfaces::SLOT_SOFTWARE_TABS.to_string(),
                    scope: surfaces::Scope::Tenant,
                    targeting: surfaces::Targeting::Universal,
                    required_permission: Some("view_software".to_string()),
                    provider_kind: surfaces::ProviderKind::Plugin,
                    required_capabilities: surfaces::CapabilitySet::from_capabilities([
                        surfaces::Capability::TextBlockNode,
                        surfaces::Capability::UniversalTargeting,
                        surfaces::Capability::MutationAction,
                    ]),
                    root_node: surfaces::SurfaceNode::TextBlock {
                        text: "plugin-fallback".to_string(),
                    },
                },
                interactions: vec![surfaces::InteractionDescriptor {
                    interaction_id: surfaces::InteractionId::new("refresh").unwrap(),
                    kind: surfaces::InteractionKind::MutationAction,
                    label: None,
                    required_permission: Some("update_software".to_string()),
                    input_schema: Some(surfaces::SchemaContract::Object),
                    result_schema: Some(surfaces::SchemaContract::Object),
                    sensitive_fields: vec![],
                    timeout_seconds: Some(5),
                    confirmation: None,
                    transport: surfaces::InteractionTransport::ControllerLocal,
                    workflow_steps: vec![],
                    form_ui: None,
                }],
                data_sources: vec![],
            }],
            encryption_metadata: None,
        }
    }

    fn rollout(active: bool) -> crate::app_state::SurfaceRuntimeRolloutState {
        crate::app_state::SurfaceRuntimeRolloutState::phase0(active, Vec::new(), BTreeMap::new())
    }

    async fn build_surface_route_test_state(
        rollout: crate::app_state::SurfaceRuntimeRolloutState,
    ) -> Arc<AppState> {
        let ca_pem = "-----BEGIN CERTIFICATE-----\ntest\n-----END CERTIFICATE-----\n";
        let snapshot_data = CaPublicSnapshot {
            active_cert_pem: ca_pem.to_string(),
            active_fingerprint: "0".repeat(64),
            previous_cert_pem: None,
            previous_fingerprint: None,
            trusted_cas: vec![TrustedCaPublic {
                cert_pem: ca_pem.to_string(),
                fingerprint: "0".repeat(64),
                not_after: OffsetDateTime::now_utc() + TimeDuration::days(365),
            }],
            trusted_ca_cns: Vec::new(),
            bundle_pem: ca_pem.to_string(),
            bundle_hash: "0".repeat(64),
            managed: true,
            active_not_after: OffsetDateTime::now_utc() + TimeDuration::days(365),
            pki_addr: None,
        };
        let (_ca_tx, ca_rx) = tokio::sync::watch::channel(snapshot_data);
        let ca_key_store: crate::CaKeyStoreRef = Arc::new(tokio::sync::RwLock::new(CaKeyStore {
            active_key_pem: zeroize::Zeroizing::new(String::new()),
            previous_key_pem: None,
            trusted_ca_keys: vec![],
        }));

        let rustls_cfg = {
            let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
            let key_pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
                .expect("test key generation should succeed");
            let cert = rcgen::CertificateParams::new(vec!["localhost".into()])
                .expect("test cert params should be valid")
                .self_signed(&key_pair)
                .expect("test certificate should self-sign");
            let server_config = rustls::ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(
                    vec![rustls::pki_types::CertificateDer::from(cert.der().to_vec())],
                    rustls::pki_types::PrivateKeyDer::try_from(key_pair.serialize_der())
                        .expect("test private key should parse"),
                )
                .expect("test rustls config should build");
            axum_server::tls_rustls::RustlsConfig::from_config(Arc::new(server_config))
        };

        let db = sea_orm::Database::connect(sea_orm::ConnectOptions::new("sqlite::memory:"))
            .await
            .expect("test db should connect");
        let settings = crate::settings::Settings::new(
            RegistrationSettings {
                mode: RegistrationMode::Open,
                token_hash: None,
                require_token_for_oidc: false,
            },
            168,
        );
        let service_connections = crate::service_connections::ServiceConnectionRegistry::new();
        let controller_id = Uuid::nil();
        let notification_service = crate::notification_service::NotificationService::new(
            service_connections.clone(),
            controller_id,
        );
        let plugin_ops: Arc<dyn uptrakit_plugin_infrastructure_registry::PluginOps> = Arc::new(
            uptrakit_plugin_infrastructure_registry::build_catalog(
                &uptrakit_plugin_infrastructure_registry::CatalogConfig::default(),
            )
            .expect("catalog should build in tests"),
        );
        let notification_dispatcher = crate::notifications::dispatcher::NotificationDispatcher::new(
            db.clone(),
            Arc::clone(&plugin_ops),
            "https://localhost".to_string(),
        );

        Arc::new(AppState {
            db: crate::app_state::DbState::new(db.clone()),
            cert: crate::app_state::CertState {
                ca_snapshot: ca_rx,
                ca_key_store,
                revocation_notify: Arc::new(tokio::sync::Notify::const_new()),
                crl_pem_cache: Arc::new(tokio::sync::RwLock::new(String::new())),
                ca_rotation_trigger: Arc::new(tokio::sync::Notify::const_new()),
            },
            auth: crate::app_state::AuthState {
                jwt: Arc::new(crate::auth::jwt::JwtManager::from_secret(
                    b"test-secret-surfaces",
                )),
                device_flow_store: crate::auth::device_flow::DeviceFlowStore::new(db.clone()),
                rate_limit_store: crate::auth::rate_limit::RateLimitStore::new(db.clone()),
                token_denylist: Arc::new(crate::auth::token_denylist::TokenDenylist::new()),
            },
            notification: crate::app_state::NotificationState {
                notification_service,
                notification_dispatcher,
                event_broadcaster: crate::event_broadcaster::EventBroadcaster::new(),
            },
            broadcast: crate::app_state::BroadcastState {
                device_flow_broadcaster: crate::device_flow_broadcaster::DeviceFlowBroadcaster::new(
                ),
                update_output_broadcaster:
                    crate::update_output_broadcaster::UpdateOutputBroadcaster::new(),
                batch_progress_broadcaster:
                    crate::batch_progress_broadcaster::BatchProgressBroadcaster::new(),
            },
            #[cfg(feature = "oidc")]
            oidc: crate::app_state::OidcState {
                oidc_flow_store: crate::auth::oidc_state::OidcFlowStore::new(db.clone()),
                account_link_store: crate::auth::oidc_state::AccountLinkStore::new(db.clone()),
                oidc_token_exchange_store: crate::auth::oidc_state::OidcTokenExchangeStore::new(
                    db.clone(),
                ),
                oidc_registration_store: crate::auth::oidc_state::OidcRegistrationStore::new(
                    db.clone(),
                ),
            },
            settings,
            cert_signer: Arc::new(NoopCertSigner),
            service_connections,
            plugin_ops,
            credential_sources: ServiceCredentialSources::default(),
            shutdown_token: Default::default(),
            embedded_service_notifier: None,
            audit_log_filter: uptrakit_audit_log::AuditFilter::default(),
            audit_log_dispatcher: uptrakit_audit_log::AuditLogDispatcher::new(Arc::new(
                uptrakit_audit_log::NoopBackend,
            )),
            surface_registry: Arc::new(crate::surface_registry::SurfaceRegistry::new(
                crate::surface_registry::SurfaceRegistryConfig::default(),
            )),
            surface_proxy: Arc::new(crate::surface_proxy::SurfaceProxy::new()),
            config_test_proxy: Arc::new(crate::config_test_proxy::ConfigTestProxy::new()),
            workload_claim_registry: Arc::new(crate::workload_claims::WorkloadClaimRegistry::new()),
            pki_path: std::path::PathBuf::from("/tmp/test-pki"),
            rustls_config: rustls_cfg,
            default_tenant_id: Uuid::nil(),
            controller_id,
            reject_dangerous_commands: false,
            surface_runtime_rollout: rollout,
            #[cfg(feature = "interactive")]
            interactive_sessions: crate::interactive_sessions::InteractiveSessionRegistry::new(),
        })
    }

    async fn error_body(response: Response) -> ErrorResponse {
        let body = to_bytes(response.into_body(), 1024 * 16)
            .await
            .expect("response body should read");
        serde_json::from_slice(&body).expect("response body should deserialize")
    }

    async fn json_body<T: serde::de::DeserializeOwned>(response: Response) -> T {
        let body = to_bytes(response.into_body(), 1024 * 16)
            .await
            .expect("response body should read");
        serde_json::from_slice(&body).expect("response body should deserialize")
    }

    #[tokio::test]
    async fn surface_http_endpoints_require_active_rollout() {
        let inactive_state = build_surface_route_test_state(rollout(false)).await;
        inactive_state
            .surface_registry
            .register_service(
                Uuid::now_v7(),
                "uptrakit-agent-ssh",
                Some(Uuid::nil()),
                service_surface_registration("provider-a", Uuid::nil()),
            )
            .expect("service registration should succeed");
        inactive_state
            .surface_registry
            .bootstrap_plugin(plugin_surface_registration())
            .expect("plugin registration should succeed");

        let listed = list_surfaces(
            State(Arc::clone(&inactive_state)),
            TenantContext {
                tenant_id: Uuid::nil(),
            },
            Query(ListSurfacesQuery {
                slot: None,
                page: None,
            }),
        )
        .await;
        assert_eq!(listed.status(), StatusCode::OK);
        let listed: Vec<SurfaceResponse> = json_body(listed).await;
        assert!(listed.is_empty());

        let read = get_surface_read(
            State(Arc::clone(&inactive_state)),
            TenantContext {
                tenant_id: Uuid::nil(),
            },
            axum::Extension(auth_user_with_permissions(vec![
                AuthPermission::ViewSoftware,
            ])),
            Path("ssh.guest.panel".to_string()),
        )
        .await;
        assert_eq!(read.status(), StatusCode::SERVICE_UNAVAILABLE);
        let read_error = error_body(read).await;
        assert_eq!(read_error.code.as_deref(), Some("surface_runtime_inactive"));

        let invoke = invoke_surface_interaction(
            State(Arc::clone(&inactive_state)),
            TenantContext {
                tenant_id: Uuid::nil(),
            },
            axum::Extension(auth_user_with_permissions(vec![
                AuthPermission::ViewSoftware,
                AuthPermission::UpdateSoftware,
            ])),
            Path(("ssh.guest.panel".to_string(), "refresh".to_string())),
            Json(InvokeSurfaceInteractionRequest {
                params: serde_json::Map::new(),
                encrypted_sensitive_params: None,
                target_provider_id: Some("provider-a".to_string()),
                idempotency_key: Some("inactive".to_string()),
                timeout_seconds: Some(5),
            }),
        )
        .await;
        assert_eq!(invoke.status(), StatusCode::SERVICE_UNAVAILABLE);
        let invoke_error = error_body(invoke).await;
        assert_eq!(
            invoke_error.code.as_deref(),
            Some("surface_runtime_inactive")
        );

        let active_state = build_surface_route_test_state(rollout(true)).await;
        let service_id = Uuid::now_v7();
        active_state
            .surface_registry
            .register_service(
                service_id,
                "uptrakit-agent-ssh",
                Some(Uuid::nil()),
                service_surface_registration("provider-a", Uuid::nil()),
            )
            .expect("service registration should succeed");
        let (mut rx, _cancel) = active_state
            .service_connections
            .register(
                service_id,
                std::collections::BTreeSet::new(),
                None,
                None,
                Some("uptrakit-agent-ssh".to_string()),
            )
            .await;

        let proxy = Arc::clone(&active_state.surface_proxy);
        tokio::spawn(async move {
            if let Some(ControllerMessage::SurfaceActionRequest(request)) = rx.recv().await {
                proxy.complete(
                    request.request_id,
                    surfaces::SurfaceActionResponse {
                        request_id: request.request_id,
                        success: true,
                        result: Some(serde_json::json!({"ok": true})),
                        error: None,
                    },
                );
            }
        });

        let listed = list_surfaces(
            State(Arc::clone(&active_state)),
            TenantContext {
                tenant_id: Uuid::nil(),
            },
            Query(ListSurfacesQuery {
                slot: None,
                page: None,
            }),
        )
        .await;
        let listed: Vec<SurfaceResponse> = json_body(listed).await;
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].descriptor.surface_id.as_str(), "ssh.guest.panel");

        let read = get_surface_read(
            State(Arc::clone(&active_state)),
            TenantContext {
                tenant_id: Uuid::nil(),
            },
            axum::Extension(auth_user_with_permissions(vec![
                AuthPermission::ViewSoftware,
            ])),
            Path("ssh.guest.panel".to_string()),
        )
        .await;
        assert_eq!(read.status(), StatusCode::OK);

        let invoke = invoke_surface_interaction(
            State(Arc::clone(&active_state)),
            TenantContext {
                tenant_id: Uuid::nil(),
            },
            axum::Extension(auth_user_with_permissions(vec![
                AuthPermission::ViewSoftware,
                AuthPermission::UpdateSoftware,
            ])),
            Path(("ssh.guest.panel".to_string(), "refresh".to_string())),
            Json(InvokeSurfaceInteractionRequest {
                params: serde_json::Map::new(),
                encrypted_sensitive_params: None,
                target_provider_id: Some("provider-a".to_string()),
                idempotency_key: Some("active".to_string()),
                timeout_seconds: Some(5),
            }),
        )
        .await;
        assert_eq!(invoke.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn invoke_inactive_rollout_short_circuits_before_resolution_and_auth() {
        let state = build_surface_route_test_state(rollout(false)).await;
        state.surface_registry.register_provider_for_test(
            service_surface_registration("provider-a", Uuid::nil()),
            Some(Uuid::now_v7()),
            Some("uptrakit-agent-ssh"),
        );

        let denied = invoke_surface_interaction(
            State(Arc::clone(&state)),
            TenantContext {
                tenant_id: Uuid::nil(),
            },
            axum::Extension(auth_user_with_permissions(vec![])),
            Path(("ssh.guest.panel".to_string(), "refresh".to_string())),
            Json(InvokeSurfaceInteractionRequest {
                params: serde_json::Map::new(),
                encrypted_sensitive_params: None,
                target_provider_id: Some("provider-a".to_string()),
                idempotency_key: Some("inactive-no-perm".to_string()),
                timeout_seconds: Some(5),
            }),
        )
        .await;
        assert_eq!(denied.status(), StatusCode::SERVICE_UNAVAILABLE);
        let denied_error = error_body(denied).await;
        assert_eq!(
            denied_error.code.as_deref(),
            Some("surface_runtime_inactive")
        );

        let missing = invoke_surface_interaction(
            State(Arc::clone(&state)),
            TenantContext {
                tenant_id: Uuid::nil(),
            },
            axum::Extension(auth_user_with_permissions(vec![
                AuthPermission::ViewSoftware,
                AuthPermission::UpdateSoftware,
            ])),
            Path(("unknown.surface".to_string(), "refresh".to_string())),
            Json(InvokeSurfaceInteractionRequest {
                params: serde_json::Map::new(),
                encrypted_sensitive_params: None,
                target_provider_id: Some("provider-a".to_string()),
                idempotency_key: Some("inactive-missing".to_string()),
                timeout_seconds: Some(5),
            }),
        )
        .await;
        assert_eq!(missing.status(), StatusCode::SERVICE_UNAVAILABLE);
        let missing_error = error_body(missing).await;
        assert_eq!(
            missing_error.code.as_deref(),
            Some("surface_runtime_inactive")
        );
    }

    #[tokio::test]
    async fn inactive_rollout_shared_surface_read_requires_active_runtime() {
        let state = build_surface_route_test_state(rollout(false)).await;
        state.surface_registry.register_provider_for_test(
            service_surface_registration("provider-a", Uuid::nil()),
            Some(Uuid::now_v7()),
            Some("uptrakit-agent-ssh"),
        );
        state.surface_registry.register_provider_for_test(
            plugin_shared_surface_registration("plugin-a"),
            None,
            None,
        );

        let listed = list_surfaces(
            State(Arc::clone(&state)),
            TenantContext {
                tenant_id: Uuid::nil(),
            },
            Query(ListSurfacesQuery {
                slot: None,
                page: None,
            }),
        )
        .await;
        assert_eq!(listed.status(), StatusCode::OK);
        let listed: Vec<SurfaceResponse> = json_body(listed).await;
        assert!(listed.is_empty());

        let read = get_surface_read(
            State(Arc::clone(&state)),
            TenantContext {
                tenant_id: Uuid::nil(),
            },
            axum::Extension(auth_user_with_permissions(vec![
                AuthPermission::ViewSoftware,
            ])),
            Path("ssh.guest.panel".to_string()),
        )
        .await;
        assert_eq!(read.status(), StatusCode::SERVICE_UNAVAILABLE);
        let read_error = error_body(read).await;
        assert_eq!(read_error.code.as_deref(), Some("surface_runtime_inactive"));
    }

    #[tokio::test]
    async fn inactive_rollout_surface_read_unknown_surface_returns_runtime_inactive() {
        let state = build_surface_route_test_state(rollout(false)).await;

        let read = get_surface_read(
            State(Arc::clone(&state)),
            TenantContext {
                tenant_id: Uuid::nil(),
            },
            axum::Extension(auth_user_with_permissions(vec![
                AuthPermission::ViewSoftware,
            ])),
            Path("unknown.surface".to_string()),
        )
        .await;
        assert_eq!(read.status(), StatusCode::SERVICE_UNAVAILABLE);
        let read_error = error_body(read).await;
        assert_eq!(read_error.code.as_deref(), Some("surface_runtime_inactive"));
    }

    #[tokio::test]
    async fn inactive_rollout_provider_listing_returns_absence() {
        let state = build_surface_route_test_state(rollout(false)).await;
        let service_id = Uuid::now_v7();
        state.surface_registry.register_provider_for_test(
            service_surface_registration("provider-a", Uuid::nil()),
            Some(service_id),
            Some("uptrakit-agent-ssh"),
        );
        state.surface_registry.register_provider_for_test(
            plugin_shared_surface_registration("plugin-a"),
            None,
            None,
        );
        let (_rx, _cancel) = state
            .service_connections
            .register(
                service_id,
                std::collections::BTreeSet::new(),
                None,
                None,
                Some("uptrakit-agent-ssh".to_string()),
            )
            .await;

        let response = list_surface_providers(
            State(Arc::clone(&state)),
            TenantContext {
                tenant_id: Uuid::nil(),
            },
            Path("ssh.guest.panel".to_string()),
        )
        .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let error = error_body(response).await;
        assert_eq!(error.code.as_deref(), Some("not_found"));
    }
}
