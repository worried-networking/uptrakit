use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use sea_orm::EntityTrait;
use serde_json::Value;
use uptrakit_controller_core::access::AccessEngine;
use uptrakit_shared_types::access::{Action, Decision};
use uptrakit_web_api_types::surfaces::{
    InvokeSurfaceInteractionRequest, ListSurfacesQuery, ReadSurfaceInteractionQuery,
    SurfaceProviderAvailability, SurfaceProviderInfo, SurfaceReadResponse, SurfaceResponse,
};
use uptrakit_wire::surfaces;
use uuid::Uuid;

use uptrakit_shared_db::entity::system_service;

use crate::AppState;
use crate::error_response::{error_response, error_response_with_code};
use crate::middleware::action::AccessAuthority;
use crate::middleware::require_auth::{AuthenticatedApiTokenId, AuthenticatedUser};
use crate::middleware::tenant_context::TenantContext;
use crate::surface_proxy::entity_enrichment::enrich_entity_links;
use crate::surface_proxy::{SurfaceCallerOrigin, SurfaceInvokeRequest, SurfaceProxyError};
use crate::surface_registry::{SurfaceCatalogItem, SurfaceRegistryLookupError};

/// List registered surfaces visible to the authenticated tenant.
#[utoipa::path(
    get,
    path = "/api/v1/surfaces",
    params(ListSurfacesQuery),
    responses(
        (status = 200, description = "Surfaces visible to the caller, filtered by descriptor visibility", body = Vec<SurfaceResponse>),
        (status = 401, description = "Not authenticated")
    ),
    tag = "Surfaces",
    extensions(("x-required-permission" = json!("authenticated-only: results filtered by descriptor visibility"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn list_surfaces(
    State(state): State<Arc<AppState>>,
    tenant_ctx: TenantContext,
    Query(query): Query<ListSurfacesQuery>,
) -> Response {
    let catalog = state.surface_proxy_deps.registry.list_surfaces_for_tenant(
        tenant_ctx.tenant_id,
        query.slot.as_deref(),
        query.page.as_deref(),
        state.surface_proxy_deps.visibility.as_ref(),
    );

    with_private_no_store((StatusCode::OK, Json(group_surface_catalog(catalog))).into_response())
}

/// Surface GET responses are per-tenant and per-permission; shared caches and
/// bfcache must never serve them across users (spec 2026-07-16 §1).
fn with_private_no_store(mut response: Response) -> Response {
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    response
}

fn group_surface_catalog(catalog: Vec<SurfaceCatalogItem>) -> Vec<SurfaceResponse> {
    let mut grouped: BTreeMap<(String, String), SurfaceResponse> = BTreeMap::new();
    for item in catalog {
        let descriptor_key = serde_json::to_string(&item.descriptor).unwrap_or_else(|error| {
            // `SurfaceDescriptor` has no non-string map keys or non-finite
            // floats, so this is infallible in practice; fail safe rather
            // than panic by treating an unexpected failure as a unique
            // (never-merged) key instead of an unwrap/expect.
            tracing::error!(
                ?error,
                "surface descriptor failed to serialize for grouping key; treating as unique"
            );
            Uuid::now_v7().to_string()
        });
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

/// List targeted providers for a surface.
#[utoipa::path(
    get,
    path = "/api/v1/surfaces/{surface_id}/providers",
    params(("surface_id" = String, Path, description = "Surface ID")),
    responses(
        (status = 200, description = "Providers targeting this surface", body = Vec<SurfaceProviderInfo>),
        (status = 401, description = "Not authenticated"),
        (status = 404, description = "Surface not found")
    ),
    tag = "Surfaces",
    extensions(("x-required-permission" = json!("authenticated-only: results filtered by descriptor visibility"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn list_surface_providers(
    State(state): State<Arc<AppState>>,
    tenant_ctx: TenantContext,
    Path(surface_id): Path<String>,
) -> Response {
    let response = async {
        let providers = state
            .surface_proxy_deps
            .registry
            .list_targeted_providers_for_surface(
                &surface_id,
                tenant_ctx.tenant_id,
                state.surface_proxy_deps.visibility.as_ref(),
            );
        if providers.is_empty() {
            return error_response_with_code(
                StatusCode::NOT_FOUND,
                "Surface not found",
                "not_found",
            );
        }

        let mut response = Vec::with_capacity(providers.len());
        for provider in providers {
            let availability = if !provider.tenant_compatible {
                SurfaceProviderAvailability::IncompatibleTenant
            } else if let Some(service_id) = provider.service_id {
                if state.service_connections.is_connected(&service_id).await
                    && !state.service_connections.is_yielded(&service_id)
                {
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
    .await;
    with_private_no_store(response)
}

/// Read a surface: descriptor, interactions, and data sources.
#[utoipa::path(
    get,
    path = "/api/v1/surfaces/{surface_id}",
    params(("surface_id" = String, Path, description = "Surface ID")),
    responses(
        (status = 200, description = "Surface read model", body = SurfaceReadResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Missing the permission declared by the surface descriptor"),
        (status = 404, description = "Surface not found")
    ),
    tag = "Surfaces",
    extensions(("x-required-permission" = json!("dynamic: declared by the surface descriptor / interaction"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn get_surface_read(
    State(state): State<Arc<AppState>>,
    tenant_ctx: TenantContext,
    axum::Extension(_auth_user): axum::Extension<AuthenticatedUser>,
    axum::Extension(authority): axum::Extension<AccessAuthority>,
    Path(surface_id): Path<String>,
) -> Response {
    let response = async {
        let resolved = match state.surface_proxy_deps.registry.resolve_surface_read(
            tenant_ctx.tenant_id,
            &surface_id,
            state.surface_proxy_deps.visibility.as_ref(),
        ) {
            Ok(resolved) => resolved,
            Err(error) => return map_lookup_error(error),
        };

        if let Some(response) = enforce_required_action(
            resolved.required_action.as_ref(),
            &authority,
            &state.access_engine,
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
    .await;
    with_private_no_store(response)
}

/// Caller-context bundle threaded through [`dispatch_surface_interaction`],
/// grouping the extractors every method-mapped wrapper handler collects —
/// keeps the shared dispatch fn's own argument list under clippy's
/// `too_many_arguments` threshold.
struct InteractionCallCtx {
    state: Arc<AppState>,
    tenant_ctx: TenantContext,
    auth_user: crate::middleware::require_auth::AuthenticatedUser,
    api_token_id: Option<AuthenticatedApiTokenId>,
    authority: AccessAuthority,
}

/// Bundles the `Method` and raw-query extractors every GET-family
/// wrapper handler needs, keeping their own argument list under clippy's
/// `too_many_arguments` threshold now that `authority` joins the list.
pub struct GetInteractionRequest {
    is_head: bool,
    raw_query: Vec<(String, String)>,
}

impl axum::extract::FromRequestParts<Arc<AppState>> for GetInteractionRequest {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        let is_head = parts.method == axum::http::Method::HEAD;
        let Query(raw_query) = Query::<Vec<(String, String)>>::from_request_parts(parts, state)
            .await
            .map_err(IntoResponse::into_response)?;
        Ok(Self { is_head, raw_query })
    }
}

/// Per-method-family input to [`dispatch_surface_interaction`]: a JSON body
/// (POST/PUT/DELETE) or raw GET query pairs (GET/HEAD, pre-coercion).
enum InteractionInput {
    Body(InvokeSurfaceInteractionRequest),
    Get {
        raw_query: Vec<(String, String)>,
        is_head: bool,
    },
}

/// Builds a `405 Method Not Allowed` response carrying an `Allow` header
/// listing every method actually registered for the interaction — used once
/// the anti-probe permission gate (see [`dispatch_surface_interaction`]) has
/// confirmed the caller is authorized to know that set.
fn method_not_allowed_response(allowed: &[surfaces::InteractionHttpMethod]) -> Response {
    let allow_value = allowed
        .iter()
        .map(|method| method.as_str().to_ascii_uppercase())
        .collect::<Vec<_>>()
        .join(", ");
    let mut response = error_response_with_code(
        StatusCode::METHOD_NOT_ALLOWED,
        "Method not allowed for this interaction",
        "method_not_allowed",
    );
    if let Ok(header_value) = header::HeaderValue::from_str(&allow_value) {
        response.headers_mut().insert(header::ALLOW, header_value);
    }
    response
}

/// Shared dispatch path for every method-mapped surface interaction route
/// (`read`/`invoke`/`update`/`delete`, base and `:item_id` variants). Owns
/// the full resolution order: envelope split → registry resolution →
/// anti-probe permission gate on `MethodNotAllowed` → descriptor/interaction
/// permission checks → HEAD short-circuit → GET coercion / body read → item
/// segment overlay → provider dispatch → audit emission.
///
/// `Cache-Control: private, no-store` is applied to every response reached
/// via a `Get` input (success and error alike); non-GET responses are
/// returned as-is (existing behavior for the mutation-verb routes).
async fn dispatch_surface_interaction(
    ctx: InteractionCallCtx,
    method: surfaces::InteractionHttpMethod,
    surface_id: String,
    interaction_id: String,
    item_id: Option<String>,
    input: InteractionInput,
) -> Response {
    let is_get = matches!(input, InteractionInput::Get { .. });
    let wrap = |response: Response| -> Response {
        if is_get {
            with_private_no_store(response)
        } else {
            response
        }
    };

    // Step 1: split the GET envelope, or read the body's envelope fields.
    let (target_provider_id, timeout_seconds, rest, is_head, body) = match input {
        InteractionInput::Body(body) => (
            body.target_provider_id.clone(),
            body.timeout_seconds,
            None,
            false,
            Some(body),
        ),
        InteractionInput::Get { raw_query, is_head } => {
            let (envelope, rest) = match split_get_envelope(raw_query) {
                Ok(pair) => pair,
                Err(response) => return wrap(*response),
            };
            (
                envelope.target_provider_id,
                envelope.timeout_seconds,
                Some(rest),
                is_head,
                None,
            )
        }
    };

    let audit_ctx = SurfaceAuditContext {
        state: &ctx.state,
        tenant_id: ctx.tenant_ctx.tenant_id,
        auth_user: &ctx.auth_user,
        api_token_id: ctx.api_token_id,
        method: &method,
        surface_id: &surface_id,
        interaction_id: &interaction_id,
        target_provider_id: target_provider_id.as_deref(),
    };

    // Step 2: resolve against the concrete method.
    let resolved = match ctx
        .state
        .surface_proxy_deps
        .registry
        .resolve_surface_action_for_method(
            ctx.tenant_ctx.tenant_id,
            &surface_id,
            &interaction_id,
            Some(&method),
            target_provider_id.as_deref(),
            ctx.state.surface_proxy_deps.visibility.as_ref(),
        ) {
        Ok(resolved) => resolved,
        Err(SurfaceRegistryLookupError::MethodNotAllowed {
            allowed,
            descriptor_required_action,
            interaction_required_actions,
        }) => {
            // Anti-probe ordering: check every candidate action (descriptor,
            // then each sibling interaction registration) BEFORE disclosing the
            // 405/Allow set, so an unauthorized caller gets 403 instead of a
            // methods-that-exist leak.
            if let Some(response) = enforce_required_action(
                descriptor_required_action.as_ref(),
                &ctx.authority,
                &ctx.state.access_engine,
                &surface_id,
                "surface",
            ) {
                if response.status() == StatusCode::FORBIDDEN
                    && let Some(required) = descriptor_required_action.as_ref()
                {
                    emit_surface_action_permission_denied_audit(
                        &audit_ctx,
                        "surface",
                        &required.to_string(),
                    );
                }
                return wrap(response);
            }
            for candidate_action in &interaction_required_actions {
                if let Some(response) = enforce_required_action(
                    candidate_action.as_ref(),
                    &ctx.authority,
                    &ctx.state.access_engine,
                    &surface_id,
                    "interaction",
                ) {
                    if response.status() == StatusCode::FORBIDDEN
                        && let Some(required) = candidate_action.as_ref()
                    {
                        emit_surface_action_permission_denied_audit(
                            &audit_ctx,
                            "interaction",
                            &required.to_string(),
                        );
                    }
                    return wrap(response);
                }
            }
            return wrap(method_not_allowed_response(&allowed));
        }
        Err(error) => {
            let (outcome, reason_code) = classify_surface_lookup_error_for_audit(&error);
            emit_surface_action_invoke_audit(&audit_ctx, None, outcome, Some(reason_code));
            return wrap(map_lookup_error(error));
        }
    };

    // Step 3: descriptor then interaction permission checks (unchanged from
    // the legacy route, now method-aware via the audit `method` field).
    if let Some(response) = enforce_required_action(
        resolved.descriptor_required_action.as_ref(),
        &ctx.authority,
        &ctx.state.access_engine,
        &surface_id,
        "interaction",
    ) {
        if response.status() == StatusCode::FORBIDDEN
            && let Some(required) = resolved.descriptor_required_action.as_ref()
        {
            emit_surface_action_permission_denied_audit(
                &audit_ctx,
                "surface",
                &required.to_string(),
            );
        }
        return wrap(response);
    }
    if let Some(response) = enforce_required_action(
        resolved.interaction_required_action.as_ref(),
        &ctx.authority,
        &ctx.state.access_engine,
        &surface_id,
        "interaction",
    ) {
        if response.status() == StatusCode::FORBIDDEN
            && let Some(required) = resolved.interaction_required_action.as_ref()
        {
            emit_surface_action_permission_denied_audit(
                &audit_ctx,
                "interaction",
                &required.to_string(),
            );
        }
        return wrap(response);
    }

    // Step 4: HEAD short-circuit — after permission checks, before any
    // coercion or provider dispatch. The provider is never reached.
    if is_head {
        return with_private_no_store(StatusCode::OK.into_response());
    }

    // Step 5: GET coercion (422 short-circuits) or direct body read.
    let (mut params, idempotency_key, encrypted_sensitive_params) = match body {
        Some(body) => {
            let idempotency_key = body
                .idempotency_key
                .clone()
                .unwrap_or_else(|| Uuid::now_v7().to_string());
            (
                body.params,
                idempotency_key,
                body.encrypted_sensitive_params,
            )
        }
        None => {
            let params =
                match coerce_get_params(rest.unwrap_or_default(), &resolved.interaction.params) {
                    Ok(params) => params,
                    Err(response) => return wrap(*response),
                };
            (params, Uuid::now_v7().to_string(), None)
        }
    };
    // Path segment overwrites any `id` carried in the query/body.
    if let Some(item_id) = item_id {
        params.insert("id".to_string(), serde_json::Value::String(item_id));
    }

    let session_id = match ctx.auth_user.auth_method {
        crate::auth::AuthMethod::Password => "password".to_string(),
        crate::auth::AuthMethod::ApiToken => "api_token".to_string(),
        crate::auth::AuthMethod::Oidc { provider_id } => format!("oidc:{provider_id}"),
    };

    // Step 6/7: build and dispatch the invocation.
    let request = SurfaceInvokeRequest::new(
        ctx.tenant_ctx.tenant_id,
        surface_id.clone(),
        interaction_id.clone(),
        Some(method.clone()),
        idempotency_key,
        target_provider_id.clone(),
        SurfaceCallerOrigin::UserSession {
            user_id: ctx.auth_user.user_id,
            session_id,
        },
        params,
        encrypted_sensitive_params,
    );
    let timeout_override = timeout_seconds.map(|seconds| Duration::from_secs(u64::from(seconds)));

    let result = ctx
        .state
        .surface_proxy_deps
        .proxy
        .invoke(
            &ctx.state.service_connections,
            &ctx.state.surface_proxy_deps.registry,
            request,
            timeout_override,
        )
        .await;

    let mut result = result;
    if let Ok(ref mut action_response) = result
        && let Some(result_value) = action_response.result.take()
    {
        action_response.result = Some(
            enrich_entity_links(
                ctx.state.db(),
                Some(ctx.tenant_ctx.tenant_id),
                &resolved.descriptor.root_node,
                result_value,
            )
            .await,
        );
    }

    let response = match result {
        Ok(response) => response,
        Err(error) => {
            let (outcome, reason_code) = classify_surface_proxy_error_for_audit(&error);
            emit_surface_action_invoke_audit(
                &audit_ctx,
                Some(&resolved),
                outcome,
                Some(reason_code),
            );
            return wrap(map_proxy_error(error));
        }
    };

    let (outcome, reason_code) = classify_surface_action_response_for_audit(&response);
    emit_surface_action_invoke_audit(&audit_ctx, Some(&resolved), outcome, reason_code);

    if response.success {
        return wrap(
            (StatusCode::OK, Json(response.result.unwrap_or(Value::Null))).into_response(),
        );
    }

    let (error_message, error_code) = if let Some(error) = response.error {
        (error.message, action_error_code(&error.code).to_string())
    } else {
        (
            "surface interaction failed".to_string(),
            "action_failed".to_string(),
        )
    };
    wrap(error_response_with_code(
        StatusCode::UNPROCESSABLE_ENTITY,
        error_message,
        error_code,
    ))
}

/// Read a surface interaction via `GET`. Query keys: reserved (`page`,
/// `per_page`) coerce to numbers; declared keys parse strictly per their
/// schema; undeclared keys pass through as strings. `target_provider_id` and
/// `timeout_seconds` are envelope keys and never reach provider `params`.
/// `HEAD` is auto-derived from this registration and short-circuits before
/// the provider is ever reached.
#[utoipa::path(
    get,
    path = "/api/v1/surfaces/{surface_id}/interactions/{interaction_id}",
    params(
        ("surface_id" = String, Path, description = "Surface ID"),
        ("interaction_id" = String, Path, description = "Interaction ID"),
        ReadSurfaceInteractionQuery
    ),
    responses(
        (status = 200, description = "Provider-defined free-form JSON result", body = serde_json::Value),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Missing a permission declared by the descriptor or interaction"),
        (status = 404, description = "Surface, interaction, or provider not found"),
        (status = 405, description = "Interaction is not registered under GET (Allow header lists methods)"),
        (status = 422, description = "Reserved or declared query key failed strict parsing"),
        (status = 503, description = "Provider unavailable"),
        (status = 504, description = "Surface action timed out")
    ),
    tag = "Surfaces",
    extensions(("x-required-permission" = json!("dynamic: declared by the surface descriptor / interaction"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn read_surface_interaction(
    State(state): State<Arc<AppState>>,
    tenant_ctx: TenantContext,
    axum::Extension(auth_user): axum::Extension<crate::middleware::require_auth::AuthenticatedUser>,
    axum::Extension(authority): axum::Extension<AccessAuthority>,
    api_token_id: Option<axum::Extension<AuthenticatedApiTokenId>>,
    Path((surface_id, interaction_id)): Path<(String, String)>,
    get_request: GetInteractionRequest,
) -> Response {
    dispatch_surface_interaction(
        InteractionCallCtx {
            state,
            tenant_ctx,
            auth_user,
            api_token_id: api_token_id.map(|axum::Extension(id)| id),
            authority,
        },
        surfaces::InteractionHttpMethod::Get,
        surface_id,
        interaction_id,
        None,
        InteractionInput::Get {
            raw_query: get_request.raw_query,
            is_head: get_request.is_head,
        },
    )
    .await
}

/// Invoke a surface interaction via `POST`. The success body is a free-form,
/// provider-defined JSON value.
#[utoipa::path(
    post,
    path = "/api/v1/surfaces/{surface_id}/interactions/{interaction_id}",
    params(
        ("surface_id" = String, Path, description = "Surface ID"),
        ("interaction_id" = String, Path, description = "Interaction ID")
    ),
    request_body = InvokeSurfaceInteractionRequest,
    responses(
        (status = 200, description = "Provider-defined free-form JSON result", body = serde_json::Value),
        (status = 400, description = "Invalid or missing target provider"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Missing a permission declared by the descriptor or interaction"),
        (status = 404, description = "Surface, interaction, or provider not found"),
        (status = 405, description = "Interaction is not registered under POST (Allow header lists methods)"),
        (status = 409, description = "Duplicate idempotency key or provider-reported conflict"),
        (status = 422, description = "Schema validation failed or provider-reported failure"),
        (status = 429, description = "Rate limited"),
        (status = 503, description = "Provider unavailable"),
        (status = 504, description = "Surface action timed out")
    ),
    tag = "Surfaces",
    extensions(("x-required-permission" = json!("dynamic: declared by the surface descriptor / interaction"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn invoke_surface_interaction(
    State(state): State<Arc<AppState>>,
    tenant_ctx: TenantContext,
    axum::Extension(auth_user): axum::Extension<crate::middleware::require_auth::AuthenticatedUser>,
    axum::Extension(authority): axum::Extension<AccessAuthority>,
    api_token_id: Option<axum::Extension<AuthenticatedApiTokenId>>,
    Path((surface_id, interaction_id)): Path<(String, String)>,
    Json(body): Json<InvokeSurfaceInteractionRequest>,
) -> Response {
    dispatch_surface_interaction(
        InteractionCallCtx {
            state,
            tenant_ctx,
            auth_user,
            api_token_id: api_token_id.map(|axum::Extension(id)| id),
            authority,
        },
        surfaces::InteractionHttpMethod::Post,
        surface_id,
        interaction_id,
        None,
        InteractionInput::Body(body),
    )
    .await
}

/// Invoke a surface interaction via `PUT` (full-replace semantics by REST
/// convention; the provider defines actual behavior).
#[utoipa::path(
    put,
    path = "/api/v1/surfaces/{surface_id}/interactions/{interaction_id}",
    params(
        ("surface_id" = String, Path, description = "Surface ID"),
        ("interaction_id" = String, Path, description = "Interaction ID")
    ),
    request_body = InvokeSurfaceInteractionRequest,
    responses(
        (status = 200, description = "Provider-defined free-form JSON result", body = serde_json::Value),
        (status = 400, description = "Invalid or missing target provider"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Missing a permission declared by the descriptor or interaction"),
        (status = 404, description = "Surface, interaction, or provider not found"),
        (status = 405, description = "Interaction is not registered under PUT (Allow header lists methods)"),
        (status = 409, description = "Duplicate idempotency key or provider-reported conflict"),
        (status = 422, description = "Schema validation failed or provider-reported failure"),
        (status = 429, description = "Rate limited"),
        (status = 503, description = "Provider unavailable"),
        (status = 504, description = "Surface action timed out")
    ),
    tag = "Surfaces",
    extensions(("x-required-permission" = json!("dynamic: declared by the surface descriptor / interaction"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn update_surface_interaction(
    State(state): State<Arc<AppState>>,
    tenant_ctx: TenantContext,
    axum::Extension(auth_user): axum::Extension<crate::middleware::require_auth::AuthenticatedUser>,
    axum::Extension(authority): axum::Extension<AccessAuthority>,
    api_token_id: Option<axum::Extension<AuthenticatedApiTokenId>>,
    Path((surface_id, interaction_id)): Path<(String, String)>,
    body: Option<Json<InvokeSurfaceInteractionRequest>>,
) -> Response {
    dispatch_surface_interaction(
        InteractionCallCtx {
            state,
            tenant_ctx,
            auth_user,
            api_token_id: api_token_id.map(|axum::Extension(id)| id),
            authority,
        },
        surfaces::InteractionHttpMethod::Put,
        surface_id,
        interaction_id,
        None,
        InteractionInput::Body(body.map(|Json(b)| b).unwrap_or_default()),
    )
    .await
}

/// Invoke a surface interaction via `DELETE`.
#[utoipa::path(
    delete,
    path = "/api/v1/surfaces/{surface_id}/interactions/{interaction_id}",
    params(
        ("surface_id" = String, Path, description = "Surface ID"),
        ("interaction_id" = String, Path, description = "Interaction ID")
    ),
    request_body(content = InvokeSurfaceInteractionRequest, description = "Optional body", content_type = "application/json"),
    responses(
        (status = 200, description = "Provider-defined free-form JSON result", body = serde_json::Value),
        (status = 400, description = "Invalid or missing target provider"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Missing a permission declared by the descriptor or interaction"),
        (status = 404, description = "Surface, interaction, or provider not found"),
        (status = 405, description = "Interaction is not registered under DELETE (Allow header lists methods)"),
        (status = 409, description = "Duplicate idempotency key or provider-reported conflict"),
        (status = 422, description = "Schema validation failed or provider-reported failure"),
        (status = 429, description = "Rate limited"),
        (status = 503, description = "Provider unavailable"),
        (status = 504, description = "Surface action timed out")
    ),
    tag = "Surfaces",
    extensions(("x-required-permission" = json!("dynamic: declared by the surface descriptor / interaction"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn delete_surface_interaction(
    State(state): State<Arc<AppState>>,
    tenant_ctx: TenantContext,
    axum::Extension(auth_user): axum::Extension<crate::middleware::require_auth::AuthenticatedUser>,
    axum::Extension(authority): axum::Extension<AccessAuthority>,
    api_token_id: Option<axum::Extension<AuthenticatedApiTokenId>>,
    Path((surface_id, interaction_id)): Path<(String, String)>,
    body: Option<Json<InvokeSurfaceInteractionRequest>>,
) -> Response {
    dispatch_surface_interaction(
        InteractionCallCtx {
            state,
            tenant_ctx,
            auth_user,
            api_token_id: api_token_id.map(|axum::Extension(id)| id),
            authority,
        },
        surfaces::InteractionHttpMethod::Delete,
        surface_id,
        interaction_id,
        None,
        InteractionInput::Body(body.map(|Json(b)| b).unwrap_or_default()),
    )
    .await
}

/// Read a surface interaction targeting a specific item via `GET
/// .../{item_id}`. Identical semantics to [`read_surface_interaction`]; the
/// path segment overwrites any `id` carried in the query string.
#[utoipa::path(
    get,
    path = "/api/v1/surfaces/{surface_id}/interactions/{interaction_id}/{item_id}",
    params(
        ("surface_id" = String, Path, description = "Surface ID"),
        ("interaction_id" = String, Path, description = "Interaction ID"),
        ("item_id" = String, Path, description = "Item ID (overwrites `id` from the query string)"),
        ReadSurfaceInteractionQuery
    ),
    responses(
        (status = 200, description = "Provider-defined free-form JSON result", body = serde_json::Value),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Missing a permission declared by the descriptor or interaction"),
        (status = 404, description = "Surface, interaction, or provider not found"),
        (status = 405, description = "Interaction is not registered under GET (Allow header lists methods)"),
        (status = 422, description = "Reserved or declared query key failed strict parsing"),
        (status = 503, description = "Provider unavailable"),
        (status = 504, description = "Surface action timed out")
    ),
    tag = "Surfaces",
    extensions(("x-required-permission" = json!("dynamic: declared by the surface descriptor / interaction"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn read_surface_interaction_item(
    State(state): State<Arc<AppState>>,
    tenant_ctx: TenantContext,
    axum::Extension(auth_user): axum::Extension<crate::middleware::require_auth::AuthenticatedUser>,
    axum::Extension(authority): axum::Extension<AccessAuthority>,
    api_token_id: Option<axum::Extension<AuthenticatedApiTokenId>>,
    Path((surface_id, interaction_id, item_id)): Path<(String, String, String)>,
    get_request: GetInteractionRequest,
) -> Response {
    dispatch_surface_interaction(
        InteractionCallCtx {
            state,
            tenant_ctx,
            auth_user,
            api_token_id: api_token_id.map(|axum::Extension(id)| id),
            authority,
        },
        surfaces::InteractionHttpMethod::Get,
        surface_id,
        interaction_id,
        Some(item_id),
        InteractionInput::Get {
            raw_query: get_request.raw_query,
            is_head: get_request.is_head,
        },
    )
    .await
}

/// `POST .../{item_id}` is never a valid registration (POST is create/base
/// only) — always `405`, listing the item-addressable methods.
#[utoipa::path(
    post,
    path = "/api/v1/surfaces/{surface_id}/interactions/{interaction_id}/{item_id}",
    params(
        ("surface_id" = String, Path, description = "Surface ID"),
        ("interaction_id" = String, Path, description = "Interaction ID"),
        ("item_id" = String, Path, description = "Item ID")
    ),
    responses(
        (status = 405, description = "POST is not valid on an item-addressed interaction (Allow: GET, PUT, DELETE)")
    ),
    tag = "Surfaces",
    extensions(("x-required-permission" = json!("none: always 405"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn invoke_surface_interaction_item(
    Path((_surface_id, _interaction_id, _item_id)): Path<(String, String, String)>,
) -> Response {
    method_not_allowed_response(&[
        surfaces::InteractionHttpMethod::Get,
        surfaces::InteractionHttpMethod::Put,
        surfaces::InteractionHttpMethod::Delete,
    ])
}

/// Update a specific item via `PUT .../{item_id}`.
#[utoipa::path(
    put,
    path = "/api/v1/surfaces/{surface_id}/interactions/{interaction_id}/{item_id}",
    params(
        ("surface_id" = String, Path, description = "Surface ID"),
        ("interaction_id" = String, Path, description = "Interaction ID"),
        ("item_id" = String, Path, description = "Item ID (overwrites `id` from the body)")
    ),
    request_body = InvokeSurfaceInteractionRequest,
    responses(
        (status = 200, description = "Provider-defined free-form JSON result", body = serde_json::Value),
        (status = 400, description = "Invalid or missing target provider"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Missing a permission declared by the descriptor or interaction"),
        (status = 404, description = "Surface, interaction, or provider not found"),
        (status = 405, description = "Interaction is not registered under PUT (Allow header lists methods)"),
        (status = 409, description = "Duplicate idempotency key or provider-reported conflict"),
        (status = 422, description = "Schema validation failed or provider-reported failure"),
        (status = 429, description = "Rate limited"),
        (status = 503, description = "Provider unavailable"),
        (status = 504, description = "Surface action timed out")
    ),
    tag = "Surfaces",
    extensions(("x-required-permission" = json!("dynamic: declared by the surface descriptor / interaction"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn update_surface_interaction_item(
    State(state): State<Arc<AppState>>,
    tenant_ctx: TenantContext,
    axum::Extension(auth_user): axum::Extension<crate::middleware::require_auth::AuthenticatedUser>,
    axum::Extension(authority): axum::Extension<AccessAuthority>,
    api_token_id: Option<axum::Extension<AuthenticatedApiTokenId>>,
    Path((surface_id, interaction_id, item_id)): Path<(String, String, String)>,
    body: Option<Json<InvokeSurfaceInteractionRequest>>,
) -> Response {
    dispatch_surface_interaction(
        InteractionCallCtx {
            state,
            tenant_ctx,
            auth_user,
            api_token_id: api_token_id.map(|axum::Extension(id)| id),
            authority,
        },
        surfaces::InteractionHttpMethod::Put,
        surface_id,
        interaction_id,
        Some(item_id),
        InteractionInput::Body(body.map(|Json(b)| b).unwrap_or_default()),
    )
    .await
}

/// Delete a specific item via `DELETE .../{item_id}`.
#[utoipa::path(
    delete,
    path = "/api/v1/surfaces/{surface_id}/interactions/{interaction_id}/{item_id}",
    params(
        ("surface_id" = String, Path, description = "Surface ID"),
        ("interaction_id" = String, Path, description = "Interaction ID"),
        ("item_id" = String, Path, description = "Item ID (overwrites `id` from the body)")
    ),
    request_body(content = InvokeSurfaceInteractionRequest, description = "Optional body", content_type = "application/json"),
    responses(
        (status = 200, description = "Provider-defined free-form JSON result", body = serde_json::Value),
        (status = 400, description = "Invalid or missing target provider"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Missing a permission declared by the descriptor or interaction"),
        (status = 404, description = "Surface, interaction, or provider not found"),
        (status = 405, description = "Interaction is not registered under DELETE (Allow header lists methods)"),
        (status = 409, description = "Duplicate idempotency key or provider-reported conflict"),
        (status = 422, description = "Schema validation failed or provider-reported failure"),
        (status = 429, description = "Rate limited"),
        (status = 503, description = "Provider unavailable"),
        (status = 504, description = "Surface action timed out")
    ),
    tag = "Surfaces",
    extensions(("x-required-permission" = json!("dynamic: declared by the surface descriptor / interaction"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn delete_surface_interaction_item(
    State(state): State<Arc<AppState>>,
    tenant_ctx: TenantContext,
    axum::Extension(auth_user): axum::Extension<crate::middleware::require_auth::AuthenticatedUser>,
    axum::Extension(authority): axum::Extension<AccessAuthority>,
    api_token_id: Option<axum::Extension<AuthenticatedApiTokenId>>,
    Path((surface_id, interaction_id, item_id)): Path<(String, String, String)>,
    body: Option<Json<InvokeSurfaceInteractionRequest>>,
) -> Response {
    dispatch_surface_interaction(
        InteractionCallCtx {
            state,
            tenant_ctx,
            auth_user,
            api_token_id: api_token_id.map(|axum::Extension(id)| id),
            authority,
        },
        surfaces::InteractionHttpMethod::Delete,
        surface_id,
        interaction_id,
        Some(item_id),
        InteractionInput::Body(body.map(|Json(b)| b).unwrap_or_default()),
    )
    .await
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
        // This legacy id-only route never resolves with a concrete method
        // (`SurfaceInvokeRequest::method` is always `None` here), so it can
        // still surface `MethodNotAllowed` for a multi-method interaction id
        // — no `Allow` header on this route; the method-model route family
        // (Task 3) owns that.
        SurfaceRegistryLookupError::MethodNotAllowed { .. } => error_response_with_code(
            StatusCode::METHOD_NOT_ALLOWED,
            "Method not allowed for this interaction",
            "method_not_allowed",
        ),
        unknown => {
            tracing::warn!(?unknown, "unhandled SurfaceRegistryLookupError variant");
            error_response_with_code(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Surface lookup error",
                "surface_lookup_error",
            )
        }
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
        unknown => {
            tracing::warn!(?unknown, "unhandled SurfaceProxyError variant");
            error_response_with_code(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Surface proxy error",
                "proxy_error",
            )
        }
    }
}

/// Envelope keys pulled out of a GET query string before the remaining pairs
/// are coerced into interaction `params` — see [`split_get_envelope`].
struct GetInvokeEnvelope {
    target_provider_id: Option<String>,
    timeout_seconds: Option<u16>,
}

/// The envelope plus the leftover (non-envelope) query pairs, as split by
/// [`split_get_envelope`].
type GetEnvelopeSplit = (GetInvokeEnvelope, Vec<(String, String)>);

/// Splits raw GET query pairs into envelope keys (`target_provider_id`,
/// `timeout_seconds`) and the remaining pairs destined for `coerce_get_params`.
/// Duplicate keys: last one wins (fold in encounter order).
fn split_get_envelope(raw: Vec<(String, String)>) -> Result<GetEnvelopeSplit, Box<Response>> {
    let mut envelope = GetInvokeEnvelope {
        target_provider_id: None,
        timeout_seconds: None,
    };
    let mut rest = Vec::with_capacity(raw.len());
    for (key, value) in raw {
        match key.as_str() {
            // Empty value normalizes to None: `?target_provider_id=` must mean
            // implicit provider resolution, not a lookup for provider id "".
            "target_provider_id" => {
                envelope.target_provider_id = Some(value).filter(|v| !v.is_empty());
            }
            "timeout_seconds" => match value.parse::<u16>() {
                Ok(parsed) => envelope.timeout_seconds = Some(parsed),
                Err(_parse_error) => {
                    return Err(Box::new(error_response_with_code(
                        StatusCode::UNPROCESSABLE_ENTITY,
                        "timeout_seconds must be an unsigned integer",
                        "schema_validation_failed",
                    )));
                }
            },
            _ => rest.push((key, value)),
        }
    }
    Ok((envelope, rest))
}

/// Coerces the non-envelope GET query pairs into a JSON params object using a
/// deterministic three-tier rule (no inference): reserved typed keys
/// (`page`/`per_page`) coerce to numbers; declared keys parse strictly per
/// their `SchemaContract`; undeclared keys pass through as JSON strings.
/// Duplicate keys: last one wins (fold in encounter order).
fn coerce_get_params(
    rest: Vec<(String, String)>,
    declared: &[surfaces::ParamFieldDescriptor],
) -> Result<serde_json::Map<String, serde_json::Value>, Box<Response>> {
    let mut params = serde_json::Map::new();
    for (key, value) in rest {
        let coerced = match key.as_str() {
            // Tier 2: framework-reserved typed keys.
            "page" | "per_page" => match value.parse::<u64>() {
                Ok(number) => serde_json::json!(number),
                Err(_parse_error) => {
                    return Err(Box::new(error_response_with_code(
                        StatusCode::UNPROCESSABLE_ENTITY,
                        format!("query key `{key}` must be an unsigned integer"),
                        "schema_validation_failed",
                    )));
                }
            },
            // Tier 3: declared -> strict parse per SchemaContract; undeclared -> string passthrough.
            _ => match declared.iter().find(|field| field.key == key) {
                None => serde_json::Value::String(value),
                Some(field) => coerce_declared(&key, value, &field.schema)?,
            },
        };
        params.insert(key, coerced);
    }
    Ok(params)
}

/// Strictly parses a single declared query value per its `SchemaContract`.
/// Non-scalar schemas are unreachable on DataLoads (admission rule, Plan 2);
/// treated defensively as a string passthrough.
fn coerce_declared(
    key: &str,
    value: String,
    schema: &surfaces::SchemaContract,
) -> Result<serde_json::Value, Box<Response>> {
    let invalid = |expected: &str| {
        Box::new(error_response_with_code(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("query key `{key}` must be {expected} per its declared schema"),
            "schema_validation_failed",
        ))
    };
    match schema {
        surfaces::SchemaContract::String => Ok(serde_json::Value::String(value)),
        surfaces::SchemaContract::Integer => value
            .parse::<i64>()
            .map(|number| serde_json::json!(number))
            .map_err(|_parse_error| invalid("an integer")),
        surfaces::SchemaContract::Number => value
            .parse::<f64>()
            .ok()
            .and_then(serde_json::Number::from_f64)
            .map(serde_json::Value::Number)
            .ok_or_else(|| invalid("a number")),
        surfaces::SchemaContract::Boolean => match value.as_str() {
            "true" => Ok(serde_json::Value::Bool(true)),
            "false" => Ok(serde_json::Value::Bool(false)),
            _ => Err(invalid("`true` or `false`")),
        },
        _ => Ok(serde_json::Value::String(value)),
    }
}

fn surface_action_target_display(
    method: &surfaces::InteractionHttpMethod,
    surface_id: &str,
    interaction_id: &str,
) -> String {
    format!(
        "{} {surface_id}/{interaction_id}",
        method.as_str().to_ascii_uppercase()
    )
}

fn auth_method_name(auth_method: &crate::auth::AuthMethod) -> &'static str {
    match auth_method {
        crate::auth::AuthMethod::Password => "password",
        crate::auth::AuthMethod::ApiToken => "api_token",
        crate::auth::AuthMethod::Oidc { .. } => "oidc",
    }
}

struct SurfaceAuditContext<'a> {
    state: &'a AppState,
    tenant_id: Uuid,
    auth_user: &'a AuthenticatedUser,
    api_token_id: Option<AuthenticatedApiTokenId>,
    method: &'a surfaces::InteractionHttpMethod,
    surface_id: &'a str,
    interaction_id: &'a str,
    target_provider_id: Option<&'a str>,
}

fn emit_surface_action_permission_denied_audit(
    ctx: &SurfaceAuditContext<'_>,
    permission_scope: &'static str,
    required_permission: &str,
) {
    let (actor_type, actor_id) = ctx.auth_user.audit_actor(ctx.api_token_id);
    let entry = uptrakit_audit_log::AuditEntry::<uptrakit_audit_log::Event>::builder_event(
        uptrakit_audit_log::AuditActionType::SURFACE_ACTION_INVOKE,
    )
    .tenant_scope(ctx.tenant_id)
    .actor(actor_type, actor_id)
    .target_opt(
        Some("surface_action".to_string()),
        None,
        Some(surface_action_target_display(
            ctx.method,
            ctx.surface_id,
            ctx.interaction_id,
        )),
    )
    .outcome(uptrakit_audit_log::AuditOutcome::Denied)
    .details(serde_json::json!({
        "surface_id": ctx.surface_id,
        "interaction_id": ctx.interaction_id,
        "target_provider_id": ctx.target_provider_id,
        "permission_scope": permission_scope,
        "required_action": required_permission,
        "auth_method": auth_method_name(&ctx.auth_user.auth_method),
        "reason_code": "missing_required_permission",
        "http_method": ctx.method.as_str(),
    }))
    .build();

    match entry {
        Ok(entry) => ctx.state.audit_emitter.emit_event(entry),
        Err(error) => tracing::warn!(
            tenant_id = %ctx.tenant_id,
            surface_id = %ctx.surface_id,
            interaction_id = %ctx.interaction_id,
            permission_scope,
            %error,
            "failed to build surface permission denial audit entry"
        ),
    }
}

fn emit_surface_action_invoke_audit(
    ctx: &SurfaceAuditContext<'_>,
    resolved: Option<&crate::surface_registry::ResolvedSurfaceAction>,
    outcome: uptrakit_audit_log::AuditOutcome,
    reason_code: Option<&'static str>,
) {
    let (actor_type, actor_id) = ctx.auth_user.audit_actor(ctx.api_token_id);
    let mut details = serde_json::Map::from_iter([
        ("surface_id".to_string(), serde_json::json!(ctx.surface_id)),
        (
            "interaction_id".to_string(),
            serde_json::json!(ctx.interaction_id),
        ),
        (
            "target_provider_id".to_string(),
            serde_json::json!(
                resolved
                    .map(|value| value.provider_id.as_str())
                    .or(ctx.target_provider_id)
            ),
        ),
        (
            "http_method".to_string(),
            serde_json::json!(ctx.method.as_str()),
        ),
    ]);
    if let Some(resolved) = resolved {
        details.insert(
            "provider_kind".to_string(),
            serde_json::json!(surface_provider_kind_name(resolved.provider_kind)),
        );
        details.insert(
            "auth_method".to_string(),
            serde_json::json!(auth_method_name(&ctx.auth_user.auth_method)),
        );
    }
    if let Some(service_app_name) = resolved.and_then(|value| value.service_app_name.as_deref()) {
        details.insert(
            "provider_service_app_name".to_string(),
            serde_json::json!(service_app_name),
        );
    }
    if let Some(reason_code) = reason_code {
        details.insert("reason_code".to_string(), serde_json::json!(reason_code));
    }

    let entry = uptrakit_audit_log::AuditEntry::<uptrakit_audit_log::Event>::builder_event(
        uptrakit_audit_log::AuditActionType::SURFACE_ACTION_INVOKE,
    )
    .tenant_scope(ctx.tenant_id)
    .actor(actor_type, actor_id)
    .target_opt(
        Some("surface_action".to_string()),
        None,
        Some(surface_action_target_display(
            ctx.method,
            ctx.surface_id,
            ctx.interaction_id,
        )),
    )
    .outcome(outcome)
    .details(serde_json::Value::Object(details))
    .build();

    match entry {
        Ok(entry) => ctx.state.audit_emitter.emit_event(entry),
        Err(error) => tracing::warn!(
            tenant_id = %ctx.tenant_id,
            surface_id = %ctx.surface_id,
            interaction_id = %ctx.interaction_id,
            outcome = %outcome,
            %error,
            "failed to build surface invocation audit entry"
        ),
    }
}

fn classify_surface_lookup_error_for_audit(
    error: &SurfaceRegistryLookupError,
) -> (uptrakit_audit_log::AuditOutcome, &'static str) {
    match error {
        SurfaceRegistryLookupError::SurfaceNotFound => (
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            "surface_not_found",
        ),
        SurfaceRegistryLookupError::InteractionNotFound => (
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            "interaction_not_found",
        ),
        SurfaceRegistryLookupError::TargetProviderRequired => (
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            "target_provider_required",
        ),
        SurfaceRegistryLookupError::InvalidProvider(_) => (
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            "invalid_provider",
        ),
        SurfaceRegistryLookupError::NoTenantCompatibleProvider => {
            (uptrakit_audit_log::AuditOutcome::Failed, "no_provider")
        }
        SurfaceRegistryLookupError::MethodNotAllowed { .. } => (
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            "method_not_allowed",
        ),
        _ => {
            tracing::warn!(
                ?error,
                "unhandled SurfaceRegistryLookupError variant in audit classification"
            );
            (
                uptrakit_audit_log::AuditOutcome::Failed,
                "surface_lookup_error",
            )
        }
    }
}

fn classify_surface_action_response_for_audit(
    response: &surfaces::SurfaceActionResponse,
) -> (uptrakit_audit_log::AuditOutcome, Option<&'static str>) {
    if response.success {
        return (uptrakit_audit_log::AuditOutcome::Success, None);
    }

    let Some(error) = response.error.as_ref() else {
        return (
            uptrakit_audit_log::AuditOutcome::Failed,
            Some("action_failed"),
        );
    };

    let outcome = match error.code {
        surfaces::SurfaceActionErrorCode::PermissionDenied
        | surfaces::SurfaceActionErrorCode::DuplicateRequest => {
            uptrakit_audit_log::AuditOutcome::Denied
        }
        surfaces::SurfaceActionErrorCode::InvalidRequest
        | surfaces::SurfaceActionErrorCode::SchemaValidationFailed => {
            uptrakit_audit_log::AuditOutcome::ValidationFailed
        }
        surfaces::SurfaceActionErrorCode::UnsupportedCapability
        | surfaces::SurfaceActionErrorCode::ProviderUnavailable
        | surfaces::SurfaceActionErrorCode::Timeout
        | surfaces::SurfaceActionErrorCode::InternalError => {
            uptrakit_audit_log::AuditOutcome::Failed
        }
    };

    (outcome, Some(action_error_code(&error.code)))
}

fn classify_surface_proxy_error_for_audit(
    error: &SurfaceProxyError,
) -> (uptrakit_audit_log::AuditOutcome, &'static str) {
    match error {
        SurfaceProxyError::NoProvider => (uptrakit_audit_log::AuditOutcome::Failed, "no_provider"),
        SurfaceProxyError::TargetProviderRequired => (
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            "target_provider_required",
        ),
        SurfaceProxyError::InvalidProvider(_) => (
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            "invalid_provider",
        ),
        SurfaceProxyError::InteractionNotFound => (
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            "interaction_not_found",
        ),
        SurfaceProxyError::PermissionDenied(_) => (
            uptrakit_audit_log::AuditOutcome::Denied,
            "permission_denied",
        ),
        SurfaceProxyError::Conflict { code, .. } => {
            (uptrakit_audit_log::AuditOutcome::Denied, code)
        }
        SurfaceProxyError::SchemaValidationFailed(_)
        | SurfaceProxyError::SensitiveFieldRejected(_) => (
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            "invalid_request",
        ),
        SurfaceProxyError::DuplicateRequest => (
            uptrakit_audit_log::AuditOutcome::Denied,
            "duplicate_request",
        ),
        SurfaceProxyError::RateLimited => {
            (uptrakit_audit_log::AuditOutcome::Denied, "rate_limited")
        }
        SurfaceProxyError::ServiceDisconnected | SurfaceProxyError::SendFailed => (
            uptrakit_audit_log::AuditOutcome::Failed,
            "provider_unavailable",
        ),
        SurfaceProxyError::Timeout => (uptrakit_audit_log::AuditOutcome::Failed, "timeout"),
        _ => {
            tracing::warn!(
                ?error,
                "unhandled SurfaceProxyError variant in audit classification"
            );
            (uptrakit_audit_log::AuditOutcome::Failed, "proxy_error")
        }
    }
}

fn surface_provider_kind_name(provider_kind: surfaces::ProviderKind) -> &'static str {
    match provider_kind {
        surfaces::ProviderKind::Service => "service",
        surfaces::ProviderKind::BuiltIn => "built_in",
        surfaces::ProviderKind::Plugin => "plugin",
        _ => {
            tracing::warn!(
                ?provider_kind,
                "unknown ProviderKind variant; defaulting to 'plugin'"
            );
            "plugin"
        }
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

/// Engine-backed gate for the shared surface resolution path.
///
/// `None` action → allow. `Ready` + `Allow` → allow; `Ready` + deny → 403
/// (+ deny counter). `Unavailable` → 500 fail-closed, mirroring the
/// `action_extractor!` verdict set (`middleware/action.rs`).
fn enforce_required_action(
    required: Option<&Action>,
    authority: &AccessAuthority,
    engine: &AccessEngine,
    surface_id: &str,
    access_kind: &'static str,
) -> Option<Response> {
    let required = required?;
    match authority {
        AccessAuthority::Ready(ctx) => match engine.authorize(ctx, required, None) {
            Decision::Allow => None,
            Decision::Deny(reason) => {
                metrics::counter!(
                    "uptrakit_access_denies_total",
                    "reason" => reason.as_str()
                )
                .increment(1);
                Some(error_response_with_code(
                    StatusCode::FORBIDDEN,
                    format!("Insufficient permissions for this {access_kind}"),
                    "forbidden",
                ))
            }
            // `Decision` is #[non_exhaustive] in another crate.
            _ => Some(error_response_with_code(
                StatusCode::FORBIDDEN,
                format!("Insufficient permissions for this {access_kind}"),
                "forbidden",
            )),
        },
        _ => {
            tracing::error!(
                surface_id,
                "authorization unavailable: access engine failed for this request"
            );
            Some(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error",
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::let_underscore_must_use,
        reason = "fire-and-forget sends in tests drop results intentionally"
    )]

    use super::*;
    use crate::auth::AuthMethod;
    use crate::auth::permissions::Permission as AuthPermission;
    #[cfg(feature = "db-sqlite")]
    use crate::auth::registration::{RegistrationMode, RegistrationSettings};
    #[cfg(feature = "db-sqlite")]
    use crate::ca_snapshot::{CaKeyStore, CaPublicSnapshot, TrustedCaPublic};
    #[cfg(feature = "db-sqlite")]
    use crate::cert_signer::{AgentCertSigner, CertSignerError, SignedCertBundle};
    #[cfg(feature = "db-sqlite")]
    use crate::middleware::require_auth::AuthenticatedApiTokenId;
    use crate::middleware::require_auth::AuthenticatedUser;
    #[cfg(feature = "db-sqlite")]
    use crate::{AppState, ServiceCredentialSources};
    #[cfg(feature = "db-sqlite")]
    use axum::body::to_bytes;
    #[cfg(feature = "db-sqlite")]
    use std::sync::Arc;
    #[cfg(feature = "db-sqlite")]
    use time::{Duration as TimeDuration, OffsetDateTime};
    #[cfg(feature = "db-sqlite")]
    use uptrakit_web_api_types::error::ErrorResponse;
    #[cfg(feature = "db-sqlite")]
    use uptrakit_wire::ControllerMessage;

    fn auth_user_with_permissions(permissions: Vec<AuthPermission>) -> AuthenticatedUser {
        AuthenticatedUser::new(Uuid::nil(), AuthMethod::Password, permissions, None)
    }

    #[cfg(feature = "db-sqlite")]
    fn api_token_auth_user_with_permissions(permissions: Vec<AuthPermission>) -> AuthenticatedUser {
        AuthenticatedUser::new(Uuid::now_v7(), AuthMethod::ApiToken, permissions, None)
    }

    fn catalog_item(surface_id: &str, label: &str, provider_id: &str) -> SurfaceCatalogItem {
        SurfaceCatalogItem::new(
            surface_id.to_string(),
            surfaces::SLOT_SOFTWARE_TABS.to_string(),
            provider_id.to_string(),
            surfaces::Targeting::Targeted,
            surfaces::SurfaceDescriptor::builder()
                .surface_id(surfaces::SurfaceId::new(surface_id).unwrap())
                .label(label)
                .priority(100)
                .slot(surfaces::SLOT_SOFTWARE_TABS)
                .scope(surfaces::Scope::Tenant)
                .targeting(surfaces::Targeting::Targeted)
                .required_action(uptrakit_shared_types::access::actions::SOFTWARE_READ)
                .provider_kind(surfaces::ProviderKind::Service)
                .required_capabilities(surfaces::CapabilitySet::from_capabilities([
                    surfaces::Capability::TextBlockNode,
                    surfaces::Capability::TargetedTargeting,
                ]))
                .root_node(surfaces::SurfaceNode::TextBlock {
                    text: "ok".to_string(),
                })
                .build(),
        )
    }

    #[test]
    fn group_surface_catalog_merges_only_identical_descriptors() {
        let grouped = group_surface_catalog(vec![
            catalog_item("ssh.guest.panel", "SSH Guest Panel", "service.provider-a"),
            catalog_item("ssh.guest.panel", "SSH Guest Panel", "service.provider-b"),
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
    fn classify_surface_lookup_error_for_audit_maps_method_not_allowed_reason() {
        let error = SurfaceRegistryLookupError::MethodNotAllowed {
            allowed: vec![surfaces::InteractionHttpMethod::Get],
            descriptor_required_action: None,
            interaction_required_actions: vec![Some(
                uptrakit_shared_types::access::actions::SOFTWARE_READ,
            )],
        };

        let (outcome, reason_code) = classify_surface_lookup_error_for_audit(&error);

        assert_eq!(outcome, uptrakit_audit_log::AuditOutcome::ValidationFailed);
        assert_eq!(reason_code, "method_not_allowed");
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn enforce_required_action_accepts_missing_action_without_engine_lookup() {
        // `required = None` short-circuits before touching authority/engine, so an
        // `Unavailable` authority (which would otherwise 500) must not matter here.
        let db = crate::test_harness::setup_migrated_db().await;
        let engine = AccessEngine::new(db);
        let response = enforce_required_action(
            None,
            &AccessAuthority::Unavailable,
            &engine,
            "surface.one",
            "surface",
        );
        assert!(response.is_none());
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn enforce_required_action_rejects_roleless_user() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let engine = AccessEngine::new(db);
        let user_id = Uuid::now_v7();
        let ctx = engine
            .context(tenant_id, user_id, None)
            .await
            .expect("access context");
        let response = enforce_required_action(
            Some(&uptrakit_shared_types::access::actions::SOFTWARE_READ),
            &AccessAuthority::Ready(ctx),
            &engine,
            "surface.one",
            "surface",
        )
        .expect("roleless user must be denied");
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn enforce_required_action_accepts_granted_action() {
        use uptrakit_shared_db::access_grants::{GrantSubject, NewGrant, insert_grant};
        use uptrakit_shared_types::access::{ActionPattern, Selector};

        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let user_id = Uuid::now_v7();
        let patterns = vec![
            "*:read"
                .parse::<ActionPattern>()
                .expect("valid action pattern"),
        ];
        insert_grant(
            &db,
            NewGrant {
                subject: GrantSubject::User(user_id),
                tenant_id: Some(tenant_id),
                patterns: &patterns,
                selector: Selector::All,
                description: None,
                created_by: None,
            },
        )
        .await
        .expect("insert grant");

        let engine = AccessEngine::new(db);
        let ctx = engine
            .context(tenant_id, user_id, None)
            .await
            .expect("access context");
        let response = enforce_required_action(
            Some(&uptrakit_shared_types::access::actions::SOFTWARE_READ),
            &AccessAuthority::Ready(ctx),
            &engine,
            "surface.one",
            "surface",
        );
        assert!(response.is_none());
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn enforce_required_action_unavailable_authority_is_500() {
        let db = crate::test_harness::setup_migrated_db().await;
        let engine = AccessEngine::new(db);
        let response = enforce_required_action(
            Some(&uptrakit_shared_types::access::actions::SOFTWARE_READ),
            &AccessAuthority::Unavailable,
            &engine,
            "surface.one",
            "surface",
        )
        .expect("unavailable authority must fail closed");
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[cfg(feature = "db-sqlite")]
    struct NoopCertSigner;

    #[cfg(feature = "db-sqlite")]
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

    #[cfg(feature = "db-sqlite")]
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
                descriptor: surfaces::SurfaceDescriptor::builder()
                    .surface_id(surfaces::SurfaceId::new("ssh.guest.panel").unwrap())
                    .label("SSH Guest Panel")
                    .priority(100)
                    .slot(surfaces::SLOT_SOFTWARE_TABS)
                    .scope(surfaces::Scope::Tenant)
                    .targeting(surfaces::Targeting::Targeted)
                    .required_action(uptrakit_shared_types::access::actions::SOFTWARE_READ)
                    .provider_kind(surfaces::ProviderKind::Service)
                    .required_capabilities(surfaces::CapabilitySet::from_capabilities([
                        surfaces::Capability::TextBlockNode,
                        surfaces::Capability::TargetedTargeting,
                        surfaces::Capability::MutationAction,
                    ]))
                    .root_node(surfaces::SurfaceNode::TextBlock {
                        text: "ok".to_string(),
                    })
                    .build(),
                interactions: vec![{
                    let mut i = surfaces::InteractionDescriptor::new(
                        surfaces::InteractionId::new("refresh").unwrap(),
                        surfaces::InteractionKind::MutationAction,
                        "Refresh",
                        surfaces::InteractionTransport::ProviderProxied,
                    );
                    i.required_action = Some(
                        uptrakit_shared_types::access::actions::SOFTWARE_UPDATE_STR.to_string(),
                    );
                    i.input_schema = Some(surfaces::SchemaContract::Object);
                    i.result_schema = Some(surfaces::SchemaContract::Object);
                    i.timeout_seconds = Some(5);
                    i
                }],
                data_sources: vec![],
            }],
            encryption_metadata: None,
        }
    }

    #[cfg(feature = "db-sqlite")]
    async fn build_surface_route_test_state_with_db_audit()
    -> (Arc<AppState>, sea_orm::DatabaseConnection, Uuid, Uuid) {
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
            let key_pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P384_SHA384)
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

        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;

        // Stage a full-role admin principal: the direct-handler-call db-sqlite
        // tests in this module build their `AccessAuthority` from a real
        // engine context, not a synthetic zero-grant one, so allow legs need
        // a user actually holding the software:read/software:update grants
        // this module's fixtures gate on. `software_manager` seeds
        // `software:*`, covering both.
        let staged_admin_id = {
            use sea_orm::{ActiveModelTrait, ColumnTrait, QueryFilter, Set};
            let now = time::OffsetDateTime::now_utc();
            let user = uptrakit_shared_db::entity::user::ActiveModel {
                id: Set(Uuid::now_v7()),
                email: Set(uptrakit_shared_types::MaskedEmail::new(
                    "staged-admin@test.local",
                )),
                first_name: Set("Staged".to_string()),
                last_name: Set("Admin".to_string()),
                password_hash: Set(None),
                is_active: Set(true),
                deactivated_at: Set(None),
                created_at: Set(now),
                updated_at: Set(now),
            }
            .insert(&db)
            .await
            .expect("insert staged admin user");
            let software_manager_role_id = uptrakit_shared_db::entity::role::Entity::find()
                .filter(uptrakit_shared_db::entity::role::Column::Name.eq("software_manager"))
                .one(&db)
                .await
                .expect("query roles")
                .expect("seeded software_manager role")
                .id;
            uptrakit_shared_db::entity::user_role::Entity::insert(
                uptrakit_shared_db::entity::user_role::ActiveModel {
                    tenant_id: Set(tenant_id),
                    user_id: Set(user.id),
                    role_id: Set(software_manager_role_id),
                    assigned_at: Set(now),
                },
            )
            .exec(&db)
            .await
            .expect("assign software_manager role");
            user.id
        };

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
                uptrakit_plugin_infrastructure_registry::InstancePluginStates::all_disabled(),
            )
            .expect("catalog should build in tests"),
        );
        let notification_dispatcher = crate::notifications::dispatcher::NotificationDispatcher::new(
            db.clone(),
            Arc::clone(&plugin_ops),
            "https://localhost".to_string(),
        );

        let backend = Arc::new(uptrakit_audit_log::DatabaseBackend::new(db.clone()));

        let (_, config_rx_for_surfaces) =
            uptrakit_config_reload::RuntimeConfigChannels::from_runtime(
                &uptrakit_config_reload::RuntimeConfig::default(),
            );

        (
            Arc::new(AppState {
                db: crate::app_state::DbState::new(db.clone()),
                access_engine: Arc::new(uptrakit_controller_core::access::AccessEngine::new(
                    db.clone(),
                )),
                cert: crate::app_state::CertState {
                    ca_snapshot: ca_rx,
                    ca_key_store,
                    revocation_notify: Arc::new(tokio::sync::Notify::const_new()),
                    crl_pem_cache: Arc::new(parking_lot::RwLock::new(String::new())),
                    ca_rotation_trigger: Arc::new(tokio::sync::Notify::const_new()),
                },
                auth: crate::app_state::AuthState::new(
                    Arc::new(crate::auth::jwt::JwtManager::from_secret(
                        b"test-secret-surfaces",
                    )),
                    crate::auth::device_flow::DeviceFlowStore::new(db.clone()),
                    crate::auth::rate_limit::RateLimitStore::new(db.clone()),
                    Arc::new(crate::auth::token_denylist::TokenDenylist::new()),
                ),
                notification: crate::app_state::NotificationState::new(
                    notification_service,
                    notification_dispatcher,
                    crate::event_broadcaster::EventBroadcaster::new(),
                ),
                broadcast: crate::app_state::BroadcastState {
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
                plugin: crate::app_state::PluginState::new(
                    plugin_ops,
                    Arc::new(crate::global_providers::GlobalProviders::new(db.clone())),
                ),
                credential_sources: ServiceCredentialSources::default(),
                shutdown_token: Default::default(),
                embedded_service_notifier: None,
                audit_log_filter_rx: tokio::sync::watch::channel(std::sync::Arc::new(
                    uptrakit_config_reload::config::AuditConfig::default(),
                ))
                .1,
                audit_log_dispatcher: uptrakit_audit_log::AuditLogDispatcher::new(backend.clone()),
                audit_emitter: uptrakit_audit_log::AuditEmitter::new(
                    uptrakit_audit_log::AuditLogDispatcher::new(backend),
                ),
                surface_proxy_deps: crate::app_state::SurfaceProxyDeps::new(
                    Arc::new(crate::surface_registry::SurfaceRegistry::new(
                        crate::surface_registry::SurfaceRegistryConfig::default(),
                    )),
                    Arc::new(crate::surface_proxy::SurfaceProxy::new()),
                    Arc::new(crate::surface_proxy::AllProvidersVisible),
                ),
                config_test_proxy: Arc::new(crate::config_test_proxy::ConfigTestProxy::new()),
                workload_claim_registry: Arc::new(
                    crate::workload_claims::WorkloadClaimRegistry::new(),
                ),
                server: crate::app_state::ServerState::new(
                    std::path::PathBuf::from("/tmp/test-pki"),
                    rustls_cfg,
                ),
                default_tenant_id: tenant_id,
                controller_id,
                reject_dangerous_commands: false,
                #[cfg(feature = "interactive")]
                interactive_sessions: crate::interactive_sessions::InteractiveSessionRegistry::new(
                ),
                #[cfg(feature = "test-utils")]
                test_reexec_notify: None,
                update_dispatcher: Arc::new(uptrakit_controller_core::update::NoopUpdateDispatcher),
                instance_plugin_snapshot: Arc::new(arc_swap::ArcSwap::from_pointee(
                    uptrakit_web_api_queries::instance_plugin_settings::InstancePluginSnapshot::empty(),
                )),
                coordinator_handle: {
                    let (tx, _) = tokio::sync::mpsc::unbounded_channel();
                    uptrakit_config_reload::ReloadCoordinator::new(vec![], tx, std::sync::Arc::new(uptrakit_config_reload::NoopAlertWriter)).1
                },
                settings_version_cache: uptrakit_config_reload::SettingsVersionCache::new(),
                db_config_rx: config_rx_for_surfaces.db,
                network_config_rx: config_rx_for_surfaces.network,
                nats_config_rx: config_rx_for_surfaces.nats,
                tls_config_rx: config_rx_for_surfaces.tls,
                audit_config_rx: config_rx_for_surfaces.audit,
                log_config_rx: config_rx_for_surfaces.log,
                master_key_config_rx: config_rx_for_surfaces.master_key,
                embedded_services_config_rx: config_rx_for_surfaces.embedded_services,
                zeroconf_config_rx: config_rx_for_surfaces.zeroconf,
                oauth: crate::oauth::OAuthState::disabled(),
                config_file_state: tokio::sync::watch::channel(
                    uptrakit_config_reload::ConfigFileState::default(),
                ).1,
                last_reload: tokio::sync::watch::channel(None).1,
                recent_reload_events: tokio::sync::watch::channel(Vec::new()).1,
            }),
            db,
            tenant_id,
            staged_admin_id,
        )
    }

    #[cfg(feature = "db-sqlite")]
    async fn tenant_audit_row_for_action(
        db: &sea_orm::DatabaseConnection,
        action_type: uptrakit_audit_log::RegisteredAuditAction,
    ) -> uptrakit_shared_db::entity::audit_log::Model {
        use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};

        for _ in 0..50 {
            if let Some(row) = uptrakit_shared_db::entity::audit_log::Entity::find()
                .filter(uptrakit_shared_db::entity::audit_log::Column::ActionType.eq(action_type))
                .order_by_desc(uptrakit_shared_db::entity::audit_log::Column::OccurredAt)
                .one(db)
                .await
                .expect("query audit rows")
            {
                return row;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        panic!("expected tenant audit row for action {action_type}");
    }

    #[cfg(feature = "db-sqlite")]
    async fn error_body(response: Response) -> ErrorResponse {
        let body = to_bytes(response.into_body(), 1024 * 16)
            .await
            .expect("response body should read");
        serde_json::from_slice(&body).expect("response body should deserialize")
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn invoke_surface_interaction_missing_surface_permission_emits_denied_audit_row() {
        let (state, db, tenant_id, _staged_admin_id) =
            build_surface_route_test_state_with_db_audit().await;
        state
            .surface_proxy_deps
            .registry
            .register_provider_for_test(
                service_surface_registration("service.provider-a", tenant_id),
                Some(Uuid::now_v7()),
                Some("uptrakit-agent-ssh"),
            );

        let denied = invoke_surface_interaction(
            State(Arc::clone(&state)),
            TenantContext { tenant_id },
            axum::Extension(auth_user_with_permissions(vec![])),
            axum::Extension(AccessAuthority::Ready(
                state
                    .access_engine
                    .context(tenant_id, Uuid::nil(), None)
                    .await
                    .expect("access context"),
            )),
            None,
            Path(("ssh.guest.panel".to_string(), "refresh".to_string())),
            Json(InvokeSurfaceInteractionRequest {
                params: serde_json::Map::new(),
                encrypted_sensitive_params: None,
                target_provider_id: Some("service.provider-a".to_string()),
                idempotency_key: Some("surface-permission-denied".to_string()),
                timeout_seconds: Some(5),
            }),
        )
        .await;

        assert_eq!(denied.status(), StatusCode::FORBIDDEN);
        let denied_error = error_body(denied).await;
        assert_eq!(denied_error.code.as_deref(), Some("forbidden"));

        let row = tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SURFACE_ACTION_INVOKE,
        )
        .await;
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::User.as_str()
        );
        assert_eq!(row.actor_id, Some(Uuid::nil()));
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
        assert_eq!(row.target_type.as_deref(), Some("surface_action"));
        assert_eq!(row.target_id, None);
        assert_eq!(
            row.target_display.as_deref(),
            Some("POST ssh.guest.panel/refresh")
        );
        let details = row
            .details_json
            .as_ref()
            .expect("permission denial audit should include details");
        assert_eq!(details["surface_id"], "ssh.guest.panel");
        assert_eq!(details["interaction_id"], "refresh");
        assert_eq!(details["target_provider_id"], "service.provider-a");
        assert_eq!(details["permission_scope"], "surface");
        assert_eq!(
            details["required_action"],
            uptrakit_shared_types::access::actions::SOFTWARE_READ_STR
        );
        assert_eq!(details["reason_code"], "missing_required_permission");
        assert!(details.get("params").is_none());
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn invoke_surface_interaction_missing_interaction_permission_emits_denied_audit_row() {
        use sea_orm::{ActiveModelTrait, ColumnTrait, QueryFilter};

        let (state, db, tenant_id, _staged_admin_id) =
            build_surface_route_test_state_with_db_audit().await;
        state
            .surface_proxy_deps
            .registry
            .register_provider_for_test(
                service_surface_registration("service.provider-a", tenant_id),
                Some(Uuid::now_v7()),
                Some("uptrakit-agent-ssh"),
            );
        // Stage the denied caller (Uuid::nil(), no pre-existing roles) as
        // viewer-only. Viewer's seeded `*:read` grant keeps the descriptor's
        // SOFTWARE_READ gate allowed while the interaction's SOFTWARE_UPDATE
        // gate still denies — a single-privilege denial, not a zero-grant
        // everything-denies state. `user_role.user_id` is FK-constrained to
        // `users.id`, so the synthetic actor needs a real row of its own
        // before it can hold a role (unlike the read-only access-context
        // lookup, which tolerates a nonexistent user_id as authorized-but-
        // grantless).
        let now = time::OffsetDateTime::now_utc();
        uptrakit_shared_db::entity::user::ActiveModel {
            id: sea_orm::Set(Uuid::nil()),
            email: sea_orm::Set(uptrakit_shared_types::MaskedEmail::new(
                "denied-viewer@test.local",
            )),
            first_name: sea_orm::Set("Denied".to_string()),
            last_name: sea_orm::Set("Viewer".to_string()),
            password_hash: sea_orm::Set(None),
            is_active: sea_orm::Set(true),
            deactivated_at: sea_orm::Set(None),
            created_at: sea_orm::Set(now),
            updated_at: sea_orm::Set(now),
        }
        .insert(&db)
        .await
        .expect("insert denied-caller user row");
        let viewer_role_id = uptrakit_shared_db::entity::role::Entity::find()
            .filter(uptrakit_shared_db::entity::role::Column::Name.eq("viewer"))
            .one(&db)
            .await
            .expect("query roles")
            .expect("seeded viewer role")
            .id;
        uptrakit_shared_db::entity::user_role::Entity::insert(
            uptrakit_shared_db::entity::user_role::ActiveModel {
                tenant_id: sea_orm::Set(tenant_id),
                user_id: sea_orm::Set(Uuid::nil()),
                role_id: sea_orm::Set(viewer_role_id),
                assigned_at: sea_orm::Set(time::OffsetDateTime::now_utc()),
            },
        )
        .exec(&db)
        .await
        .expect("assign viewer role");
        state.access_engine.invalidate_subjects(&[Uuid::nil()], &[]);

        let denied = invoke_surface_interaction(
            State(Arc::clone(&state)),
            TenantContext { tenant_id },
            axum::Extension(auth_user_with_permissions(vec![
                AuthPermission::ViewSoftware,
            ])),
            axum::Extension(AccessAuthority::Ready(
                state
                    .access_engine
                    .context(tenant_id, Uuid::nil(), None)
                    .await
                    .expect("access context"),
            )),
            None,
            Path(("ssh.guest.panel".to_string(), "refresh".to_string())),
            Json(InvokeSurfaceInteractionRequest {
                params: serde_json::Map::new(),
                encrypted_sensitive_params: None,
                target_provider_id: Some("service.provider-a".to_string()),
                idempotency_key: Some("interaction-permission-denied".to_string()),
                timeout_seconds: Some(5),
            }),
        )
        .await;

        assert_eq!(denied.status(), StatusCode::FORBIDDEN);
        let denied_error = error_body(denied).await;
        assert_eq!(denied_error.code.as_deref(), Some("forbidden"));

        let row = tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SURFACE_ACTION_INVOKE,
        )
        .await;
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::User.as_str()
        );
        assert_eq!(row.actor_id, Some(Uuid::nil()));
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
        assert_eq!(row.target_type.as_deref(), Some("surface_action"));
        assert_eq!(row.target_id, None);
        assert_eq!(
            row.target_display.as_deref(),
            Some("POST ssh.guest.panel/refresh")
        );
        let details = row
            .details_json
            .as_ref()
            .expect("permission denial audit should include details");
        assert_eq!(details["surface_id"], "ssh.guest.panel");
        assert_eq!(details["interaction_id"], "refresh");
        assert_eq!(details["target_provider_id"], "service.provider-a");
        assert_eq!(details["permission_scope"], "interaction");
        assert_eq!(
            details["required_action"],
            uptrakit_shared_types::access::actions::SOFTWARE_UPDATE_STR
        );
        assert_eq!(details["reason_code"], "missing_required_permission");
        assert!(details.get("params").is_none());
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn invoke_surface_interaction_invalid_provider_emits_validation_failed_audit_row() {
        let (state, db, tenant_id, staged_admin_id) =
            build_surface_route_test_state_with_db_audit().await;
        let api_token_id = AuthenticatedApiTokenId(Uuid::now_v7());
        state
            .surface_proxy_deps
            .registry
            .register_provider_for_test(
                service_surface_registration("service.provider-a", tenant_id),
                Some(Uuid::now_v7()),
                Some("uptrakit-agent-ssh"),
            );

        let denied = invoke_surface_interaction(
            State(Arc::clone(&state)),
            TenantContext { tenant_id },
            axum::Extension(api_token_auth_user_with_permissions(vec![
                AuthPermission::ViewSoftware,
                AuthPermission::UpdateSoftware,
            ])),
            axum::Extension(AccessAuthority::Ready(
                state
                    .access_engine
                    .context(tenant_id, staged_admin_id, None)
                    .await
                    .expect("access context"),
            )),
            Some(axum::Extension(api_token_id)),
            Path(("ssh.guest.panel".to_string(), "refresh".to_string())),
            Json(InvokeSurfaceInteractionRequest {
                params: serde_json::Map::new(),
                encrypted_sensitive_params: None,
                target_provider_id: Some("missing-provider".to_string()),
                idempotency_key: Some("invalid-provider".to_string()),
                timeout_seconds: Some(5),
            }),
        )
        .await;

        assert_eq!(denied.status(), StatusCode::BAD_REQUEST);
        let denied_error = error_body(denied).await;
        assert_eq!(denied_error.code.as_deref(), Some("invalid_provider"));

        let row = tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SURFACE_ACTION_INVOKE,
        )
        .await;
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::ApiToken.as_str()
        );
        assert_eq!(row.actor_id, Some(api_token_id.0));
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::ValidationFailed.as_str()
        );
        assert_eq!(row.target_type.as_deref(), Some("surface_action"));
        assert_eq!(row.target_id, None);
        assert_eq!(
            row.target_display.as_deref(),
            Some("POST ssh.guest.panel/refresh")
        );
        let details = row
            .details_json
            .as_ref()
            .expect("lookup failure audit should include details");
        assert_eq!(details["surface_id"], "ssh.guest.panel");
        assert_eq!(details["interaction_id"], "refresh");
        assert_eq!(details["target_provider_id"], "missing-provider");
        assert_eq!(details["reason_code"], "invalid_provider");
        assert!(details.get("provider_kind").is_none());
        assert!(details.get("params").is_none());
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn invoke_surface_interaction_success_emits_success_audit_row_for_api_token_actor() {
        let (state, db, tenant_id, staged_admin_id) =
            build_surface_route_test_state_with_db_audit().await;
        let service_id = Uuid::now_v7();
        let api_token_id = AuthenticatedApiTokenId(Uuid::now_v7());
        state
            .surface_proxy_deps
            .registry
            .register_provider_for_test(
                service_surface_registration("service.provider-a", tenant_id),
                Some(service_id),
                Some("uptrakit-agent-ssh"),
            );
        let (mut rx, _cancel) = state
            .service_connections
            .register(
                service_id,
                std::collections::BTreeSet::new(),
                None,
                None,
                Some("uptrakit-agent-ssh".to_string()),
            )
            .await;

        let proxy = Arc::clone(&state.surface_proxy_deps.proxy);
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

        let invoke = invoke_surface_interaction(
            State(Arc::clone(&state)),
            TenantContext { tenant_id },
            axum::Extension(api_token_auth_user_with_permissions(vec![
                AuthPermission::ViewSoftware,
                AuthPermission::UpdateSoftware,
            ])),
            axum::Extension(AccessAuthority::Ready(
                state
                    .access_engine
                    .context(tenant_id, staged_admin_id, None)
                    .await
                    .expect("access context"),
            )),
            Some(axum::Extension(api_token_id)),
            Path(("ssh.guest.panel".to_string(), "refresh".to_string())),
            Json(InvokeSurfaceInteractionRequest {
                params: serde_json::Map::new(),
                encrypted_sensitive_params: None,
                target_provider_id: Some("service.provider-a".to_string()),
                idempotency_key: Some("api-token-success".to_string()),
                timeout_seconds: Some(5),
            }),
        )
        .await;

        assert_eq!(invoke.status(), StatusCode::OK);

        let row = tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SURFACE_ACTION_INVOKE,
        )
        .await;
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::ApiToken.as_str()
        );
        assert_eq!(row.actor_id, Some(api_token_id.0));
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
        assert_eq!(row.target_type.as_deref(), Some("surface_action"));
        assert_eq!(
            row.target_display.as_deref(),
            Some("POST ssh.guest.panel/refresh")
        );
        let details = row
            .details_json
            .as_ref()
            .expect("success audit should include details");
        assert_eq!(details["surface_id"], "ssh.guest.panel");
        assert_eq!(details["interaction_id"], "refresh");
        assert_eq!(details["target_provider_id"], "service.provider-a");
        assert_eq!(details["provider_kind"], "service");
        assert_eq!(details["auth_method"], "api_token");
        assert_eq!(details["provider_service_app_name"], "uptrakit-agent-ssh");
        assert!(details.get("reason_code").is_none());
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn invoke_surface_interaction_provider_unavailable_emits_failed_audit_row() {
        let (state, db, tenant_id, staged_admin_id) =
            build_surface_route_test_state_with_db_audit().await;
        let service_id = Uuid::now_v7();
        state
            .surface_proxy_deps
            .registry
            .register_provider_for_test(
                service_surface_registration("service.provider-a", tenant_id),
                Some(service_id),
                Some("uptrakit-agent-ssh"),
            );
        let (rx, _cancel) = state
            .service_connections
            .register(
                service_id,
                std::collections::BTreeSet::new(),
                None,
                None,
                Some("uptrakit-agent-ssh".to_string()),
            )
            .await;
        drop(rx);

        let invoke = invoke_surface_interaction(
            State(Arc::clone(&state)),
            TenantContext { tenant_id },
            axum::Extension(auth_user_with_permissions(vec![
                AuthPermission::ViewSoftware,
                AuthPermission::UpdateSoftware,
            ])),
            axum::Extension(AccessAuthority::Ready(
                state
                    .access_engine
                    .context(tenant_id, staged_admin_id, None)
                    .await
                    .expect("access context"),
            )),
            None,
            Path(("ssh.guest.panel".to_string(), "refresh".to_string())),
            Json(InvokeSurfaceInteractionRequest {
                params: serde_json::Map::new(),
                encrypted_sensitive_params: None,
                target_provider_id: Some("service.provider-a".to_string()),
                idempotency_key: Some("provider-unavailable".to_string()),
                timeout_seconds: Some(5),
            }),
        )
        .await;

        assert_eq!(invoke.status(), StatusCode::SERVICE_UNAVAILABLE);
        let error = error_body(invoke).await;
        assert_eq!(error.code.as_deref(), Some("provider_unavailable"));

        let row = tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SURFACE_ACTION_INVOKE,
        )
        .await;
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::User.as_str()
        );
        assert_eq!(row.actor_id, Some(Uuid::nil()));
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Failed.as_str()
        );
        let details = row
            .details_json
            .as_ref()
            .expect("failed audit should include details");
        assert_eq!(details["surface_id"], "ssh.guest.panel");
        assert_eq!(details["interaction_id"], "refresh");
        assert_eq!(details["target_provider_id"], "service.provider-a");
        assert_eq!(details["provider_kind"], "service");
        assert_eq!(details["auth_method"], "password");
        assert_eq!(details["reason_code"], "provider_unavailable");
        assert!(details.get("params").is_none());
    }

    #[test]
    fn coerce_reserved_page_to_number_and_undeclared_to_string() {
        let params = coerce_get_params(
            vec![("page".into(), "2".into()), ("foo".into(), "bar".into())],
            &[],
        )
        .expect("coerces");
        assert_eq!(params.get("page"), Some(&serde_json::json!(2)));
        assert_eq!(params.get("foo"), Some(&serde_json::json!("bar")));
    }

    #[test]
    fn coerce_unparsable_reserved_key_is_422() {
        let err = coerce_get_params(vec![("page".into(), "abc".into())], &[]).expect_err("rejects");
        assert_eq!(err.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[test]
    fn coerce_declared_field_parses_strictly() {
        let declared = vec![
            surfaces::ParamFieldDescriptor::new("count", surfaces::SchemaContract::Integer),
            surfaces::ParamFieldDescriptor::new("enabled", surfaces::SchemaContract::Boolean),
        ];
        let params = coerce_get_params(
            vec![
                ("count".into(), "7".into()),
                ("enabled".into(), "true".into()),
            ],
            &declared,
        )
        .expect("coerces");
        assert_eq!(params.get("count"), Some(&serde_json::json!(7)));
        assert_eq!(params.get("enabled"), Some(&serde_json::json!(true)));
        let err =
            coerce_get_params(vec![("count".into(), "x".into())], &declared).expect_err("rejects");
        assert_eq!(err.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[test]
    fn envelope_keys_never_reach_params() {
        let (envelope, rest) = split_get_envelope(vec![
            ("target_provider_id".into(), "p1".into()),
            ("timeout_seconds".into(), "30".into()),
            ("q".into(), "x".into()),
        ])
        .expect("splits");
        assert_eq!(envelope.target_provider_id.as_deref(), Some("p1"));
        assert_eq!(envelope.timeout_seconds, Some(30));
        assert_eq!(rest, vec![("q".to_string(), "x".to_string())]);
    }

    #[test]
    fn empty_target_provider_id_normalizes_to_none() {
        let (envelope, _) =
            split_get_envelope(vec![("target_provider_id".into(), "".into())]).expect("splits");
        assert_eq!(envelope.target_provider_id, None);
    }
}
