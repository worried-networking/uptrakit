use crate::AppState;
use crate::auth::authentication::{
    OidcUserParams, OidcUserResolution, extract_mapped_roles, resolve_oidc_user, sync_oidc_roles,
};
use crate::auth::password;
use crate::auth::refresh_cookie::set_refresh_token_cookie;
use crate::auth::session::SessionService;
use crate::auth::token::{generate_secure_token, generate_uuid};
use crate::error_response::error_response;
use crate::extract::SessionSvc;
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Redirect, Response},
};
use openidconnect::{
    AuthenticationFlow, AuthorizationCode, ClientId, ClientSecret, CsrfToken, EndpointMaybeSet,
    EndpointNotSet, EndpointSet, IssuerUrl, Nonce, PkceCodeChallenge, PkceCodeVerifier,
    RedirectUrl, Scope, TokenResponse,
    core::{CoreClient, CoreProviderMetadata, CoreResponseType},
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, Set, TransactionTrait,
};
use serde::Deserialize;
use std::sync::Arc;
use time::OffsetDateTime;
use uptrakit_shared_db::entity::prelude::*;
use uptrakit_shared_db::entity::{oidc_provider, user_oidc_link, user_role};
use uptrakit_shared_types::MaskedEmail;
use uptrakit_web_api_queries::queries::users::oidc_sync::{
    build_fake_claims_for_sync, find_active_provider,
};

use crate::api_error::ApiError;
use crate::auth::AuthMethod;
use uptrakit_web_api_types::SecretString;
use uuid::Uuid;

pub use super::auth::AuthResponse;
use crate::auth::registration::RegistrationMode;
pub use uptrakit_web_api_types::oidc_auth::{
    AuthMethodsResponse, OidcAuthorizeResponse, OidcCompleteRegistrationRequest,
    OidcExchangeRequest, OidcLinkRequest, OidcProviderInfo,
};

#[derive(Deserialize)]
pub struct OidcCallbackParams {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
}

/// Claims extracted from the OIDC ID token after successful code exchange.
struct ExtractedOidcClaims {
    sub: String,
    email: String,
    email_verified: Option<bool>,
    first_name: Option<String>,
    last_name: Option<String>,
    additional_claims: serde_json::Value,
}

/// Validated OIDC callback state: the pending flow, resolved provider, built
/// client, and redirect URL, ready for code exchange.
struct ValidatedOidcCallback {
    flow: crate::auth::oidc_state::PendingOidcFlowData,
    provider: oidc_provider::Model,
    client: DiscoveredCoreClient,
    redirect_url: RedirectUrl,
    allow_private_network_issuers: bool,
}

/// Stage-1 callback validation failure. Carries the early response and, when
/// available, the provider id resolved from the pending flow so audit emission
/// can preserve target context.
struct OidcStateValidationFailure {
    response: Response,
    provider_id: Option<Uuid>,
}

const ACTION_AUTH_OIDC_EXCHANGE: uptrakit_audit_log::RegisteredAuditAction =
    uptrakit_audit_log::AuditActionType::AUTH_OIDC_EXCHANGE;
const ACTION_AUTH_OIDC_LINK: uptrakit_audit_log::RegisteredAuditAction =
    uptrakit_audit_log::AuditActionType::AUTH_OIDC_LINK;

impl OidcStateValidationFailure {
    fn new(response: Response, provider_id: Option<Uuid>) -> Self {
        Self {
            response,
            provider_id,
        }
    }
}

fn emit_oidc_route_audit(
    state: &AppState,
    action_type: uptrakit_audit_log::RegisteredAuditAction,
    outcome: uptrakit_audit_log::AuditOutcome,
    provider: Option<&oidc_provider::Model>,
    provider_id: Option<Uuid>,
    details: serde_json::Value,
) {
    let mut builder = uptrakit_audit_log::AuditEntry::builder(action_type)
        .tenant_scope(state.default_tenant_id)
        .actor(uptrakit_audit_log::AuditActorType::Oidc, None)
        .outcome(outcome)
        .details(details);

    if let Some(target_provider_id) = provider.map(|p| p.id).or(provider_id) {
        builder = builder.target(
            "oidc_provider",
            target_provider_id.to_string(),
            provider.map(|p| p.name.clone()),
        );
    }

    if let Ok(entry) = builder.build() {
        state.audit_emitter.emit_best_effort(entry);
    }
}

fn emit_oidc_user_create_audit(
    state: &AppState,
    user_id: Option<Uuid>,
    provider_id: Option<Uuid>,
    provider_name: Option<&str>,
    outcome: uptrakit_audit_log::AuditOutcome,
    reason_code: Option<&str>,
    is_first_user: Option<bool>,
) {
    let mut details =
        serde_json::Map::from_iter([("auth_method".to_string(), serde_json::json!("oidc"))]);
    if let Some(provider_id) = provider_id {
        details.insert("provider_id".to_string(), serde_json::json!(provider_id));
    }
    if let Some(provider_name) = provider_name {
        details.insert(
            "provider_name".to_string(),
            serde_json::json!(provider_name),
        );
    }
    if let Some(reason_code) = reason_code {
        details.insert("reason_code".to_string(), serde_json::json!(reason_code));
    }
    if let Some(is_first_user) = is_first_user {
        details.insert(
            "is_first_user".to_string(),
            serde_json::json!(is_first_user),
        );
    }

    let mut builder =
        uptrakit_audit_log::AuditEntry::builder(uptrakit_audit_log::AuditActionType::USER_CREATE)
            .tenant_scope(state.default_tenant_id)
            .actor(uptrakit_audit_log::AuditActorType::Oidc, None)
            .outcome(outcome)
            .details(serde_json::Value::Object(details));

    if let Some(user_id) = user_id {
        builder = builder.target("user", user_id.to_string(), None);
    }

    if let Ok(entry) = builder.build() {
        state.audit_emitter.emit_best_effort(entry);
    }
}

fn emit_oidc_exchange_audit(
    state: &AppState,
    outcome: uptrakit_audit_log::AuditOutcome,
    provider_id: Option<Uuid>,
    http_status: StatusCode,
    reason_code: Option<&str>,
) {
    let mut details = serde_json::Map::from_iter([(
        "http_status".to_string(),
        serde_json::json!(http_status.as_u16()),
    )]);
    if let Some(reason_code) = reason_code {
        details.insert("reason_code".to_string(), serde_json::json!(reason_code));
    }

    emit_oidc_route_audit(
        state,
        uptrakit_audit_log::AuditActionType::from_static(ACTION_AUTH_OIDC_EXCHANGE),
        outcome,
        None,
        provider_id,
        serde_json::Value::Object(details),
    );
}

fn emit_oidc_link_audit(
    state: &AppState,
    outcome: uptrakit_audit_log::AuditOutcome,
    provider_id: Option<Uuid>,
    http_status: StatusCode,
    reason_code: Option<&str>,
) {
    let mut details = serde_json::Map::from_iter([(
        "http_status".to_string(),
        serde_json::json!(http_status.as_u16()),
    )]);
    if let Some(reason_code) = reason_code {
        details.insert("reason_code".to_string(), serde_json::json!(reason_code));
    }

    emit_oidc_route_audit(
        state,
        uptrakit_audit_log::AuditActionType::from_static(ACTION_AUTH_OIDC_LINK),
        outcome,
        None,
        provider_id,
        serde_json::Value::Object(details),
    );
}

fn parse_callback_redirect_query(location: &str) -> (Option<String>, bool) {
    let query = location
        .split_once('?')
        .map(|(_, q)| q.split('#').next().unwrap_or_default())
        .unwrap_or_default();
    if query.is_empty() {
        return (None, false);
    }

    let mut error_code = None;
    let mut has_exchange_code = false;
    for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
        if key == "error" {
            error_code = Some(value.into_owned());
        } else if key == "oidc_code" {
            has_exchange_code = true;
        }
    }

    (error_code, has_exchange_code)
}

fn oidc_callback_outcome_for_error_code(error_code: &str) -> uptrakit_audit_log::AuditOutcome {
    match error_code {
        "oidc_missing_params" | "oidc_missing_host" | "oidc_invalid_redirect" => {
            uptrakit_audit_log::AuditOutcome::ValidationFailed
        }
        "oidc_denied"
        | "oidc_state_expired"
        | "oidc_provider_gone"
        | "oidc_no_account"
        | "oidc_email_unverified"
        | "account_deactivated"
        | "oidc_no_email" => uptrakit_audit_log::AuditOutcome::Denied,
        _ => uptrakit_audit_log::AuditOutcome::Failed,
    }
}

fn classify_oidc_callback_response(
    response: &Response,
) -> (uptrakit_audit_log::AuditOutcome, Option<String>, bool) {
    if let Some(location) = response
        .headers()
        .get(header::LOCATION)
        .and_then(|value| value.to_str().ok())
    {
        let (error_code, has_exchange_code) = parse_callback_redirect_query(location);
        if has_exchange_code {
            return (uptrakit_audit_log::AuditOutcome::Success, None, true);
        }
        if let Some(error_code) = error_code {
            let outcome = oidc_callback_outcome_for_error_code(&error_code);
            return (outcome, Some(error_code), false);
        }
    }

    if response.status() == StatusCode::BAD_REQUEST {
        return (
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            Some("bad_request".to_string()),
            false,
        );
    }

    if response.status().is_client_error() {
        return (
            uptrakit_audit_log::AuditOutcome::Denied,
            Some("client_error".to_string()),
            false,
        );
    }

    (
        uptrakit_audit_log::AuditOutcome::Failed,
        Some("internal_error".to_string()),
        false,
    )
}

fn emit_oidc_callback_audit_for_response(
    state: &AppState,
    provider: Option<&oidc_provider::Model>,
    provider_id: Option<Uuid>,
    response: &Response,
    provider_error_code: Option<&str>,
) {
    let (outcome, reason_code, has_exchange_code) = classify_oidc_callback_response(response);
    let mut details = serde_json::Map::from_iter([(
        "http_status".to_string(),
        serde_json::json!(response.status().as_u16()),
    )]);
    if let Some(reason_code) = reason_code {
        details.insert("reason_code".to_string(), serde_json::json!(reason_code));
    }
    if let Some(provider_error_code) = provider_error_code {
        details.insert(
            "provider_error_code".to_string(),
            serde_json::json!(provider_error_code),
        );
    }
    if has_exchange_code {
        details.insert("has_exchange_code".to_string(), serde_json::json!(true));
    }

    emit_oidc_route_audit(
        state,
        uptrakit_audit_log::AuditActionType::from_static(
            uptrakit_audit_log::AuditActionType::AUTH_OIDC_CALLBACK,
        ),
        outcome,
        provider,
        provider_id,
        serde_json::Value::Object(details),
    );
}

/// Get available auth methods (public)
#[utoipa::path(
    get,
    path = "/api/v1/auth/methods",
    responses(
        (status = 200, description = "Available auth methods", body = AuthMethodsResponse),
    ),
    tag = "Authentication"
)]
#[tracing::instrument(skip_all)]
pub async fn auth_methods(State(state): State<Arc<AppState>>) -> Response {
    let auth_settings = state.settings.authentication();

    let providers = match OidcProvider::find()
        .filter(oidc_provider::Column::TenantId.eq(state.default_tenant_id))
        .filter(oidc_provider::Column::IsActive.eq(true))
        .filter(oidc_provider::Column::DeactivatedAt.is_null())
        .all(state.db())
        .await
    {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(err = %e, "Failed to load OIDC providers");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let oidc_providers: Vec<OidcProviderInfo> = providers
        .into_iter()
        .map(|p| OidcProviderInfo {
            id: p.id,
            name: p.name,
            slug: p.slug,
            logo_url: p.logo_url,
        })
        .collect();

    let setup_required = match User::find().count(state.db()).await {
        Ok(count) => count == 0,
        Err(e) => {
            tracing::error!(err = %e, "Failed to count users for setup check");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let reg_settings = state.settings.registration();
    let registration_token_required = reg_settings.needs_token_for_oidc(setup_required);

    let response = AuthMethodsResponse {
        password: auth_settings.password_auth_enabled,
        oidc_providers,
        setup_required,
        registration_token_required,
    };

    (StatusCode::OK, Json(response)).into_response()
}

/// Start OIDC authorization flow (public)
#[utoipa::path(
    get,
    path = "/api/v1/auth/oidc/{provider_id}/authorize",
    params(("provider_id" = Uuid, Path, description = "OIDC Provider ID")),
    responses(
        (status = 200, description = "Authorization URL", body = OidcAuthorizeResponse),
        (status = 404, description = "Provider not found or inactive")
    ),
    tag = "Authentication"
)]
#[tracing::instrument(skip_all)]
pub async fn oidc_authorize(
    State(state): State<Arc<AppState>>,
    Path(provider_id): Path<Uuid>,
    external_base_url: Option<Extension<crate::extract::ExternalBaseUrl>>,
    headers: HeaderMap,
) -> Response {
    let base_url = external_base_url
        .map(|Extension(u)| u.0)
        .or_else(|| base_url_from_headers(&headers));
    let base_url = match base_url {
        Some(url) => url,
        None => {
            emit_oidc_route_audit(
                &state,
                uptrakit_audit_log::AuditActionType::from_static(
                    uptrakit_audit_log::AuditActionType::AUTH_OIDC_AUTHORIZE,
                ),
                uptrakit_audit_log::AuditOutcome::ValidationFailed,
                None,
                Some(provider_id),
                serde_json::json!({
                    "reason_code": "missing_host_header",
                }),
            );
            return error_response(StatusCode::BAD_REQUEST, "Missing Host header");
        }
    };

    let redirect_url = match RedirectUrl::new(format!("{base_url}/api/v1/auth/oidc/callback")) {
        Ok(url) => url,
        Err(e) => {
            tracing::error!(error = %e, "Invalid OIDC redirect URL");
            emit_oidc_route_audit(
                &state,
                uptrakit_audit_log::AuditActionType::from_static(
                    uptrakit_audit_log::AuditActionType::AUTH_OIDC_AUTHORIZE,
                ),
                uptrakit_audit_log::AuditOutcome::ValidationFailed,
                None,
                Some(provider_id),
                serde_json::json!({
                    "reason_code": "invalid_redirect_url",
                }),
            );
            return error_response(StatusCode::BAD_REQUEST, "Invalid redirect URL");
        }
    };

    let provider =
        match find_active_provider(state.db(), state.default_tenant_id, provider_id).await {
            Some(p) => p,
            None => {
                emit_oidc_route_audit(
                    &state,
                    uptrakit_audit_log::AuditActionType::from_static(
                        uptrakit_audit_log::AuditActionType::AUTH_OIDC_AUTHORIZE,
                    ),
                    uptrakit_audit_log::AuditOutcome::Denied,
                    None,
                    Some(provider_id),
                    serde_json::json!({
                        "reason_code": "provider_not_found_or_inactive",
                    }),
                );
                return error_response(StatusCode::NOT_FOUND, "Provider not found or inactive");
            }
        };
    let multi_tenancy_enabled =
        match crate::settings_store::is_multi_tenancy_enabled(state.db()).await {
            Ok(enabled) => enabled,
            Err(e) => {
                tracing::error!(error = ?e, "Failed to load multi-tenancy mode");
                emit_oidc_route_audit(
                    &state,
                    uptrakit_audit_log::AuditActionType::from_static(
                        uptrakit_audit_log::AuditActionType::AUTH_OIDC_AUTHORIZE,
                    ),
                    uptrakit_audit_log::AuditOutcome::Failed,
                    Some(&provider),
                    None,
                    serde_json::json!({
                        "reason_code": "multi_tenancy_lookup_failed",
                    }),
                );
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        };
    let allow_private_network_issuers =
        provider.allow_private_network_issuers && !multi_tenancy_enabled;

    // Build OIDC client via discovery
    let client =
        match build_oidc_client(&provider, redirect_url, allow_private_network_issuers).await {
            Some(c) => c,
            None => {
                emit_oidc_route_audit(
                    &state,
                    uptrakit_audit_log::AuditActionType::from_static(
                        uptrakit_audit_log::AuditActionType::AUTH_OIDC_AUTHORIZE,
                    ),
                    uptrakit_audit_log::AuditOutcome::Failed,
                    Some(&provider),
                    None,
                    serde_json::json!({
                        "reason_code": "provider_unavailable",
                    }),
                );
                return error_response(StatusCode::BAD_GATEWAY, "OIDC provider unavailable");
            }
        };

    // Generate PKCE challenge
    let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();
    let nonce = Nonce::new_random();

    // Build authorization URL
    let nonce_for_url = nonce.clone();
    let mut auth_request = client.authorize_url(
        AuthenticationFlow::<CoreResponseType>::AuthorizationCode,
        CsrfToken::new_random,
        move || nonce_for_url,
    );

    // Add scopes
    for scope in provider.scopes.split_whitespace() {
        if scope != "openid" {
            auth_request = auth_request.add_scope(Scope::new(scope.to_string()));
        }
    }

    let (auth_url, csrf_state, _nonce) = auth_request.set_pkce_challenge(pkce_challenge).url();

    // Store the pending flow in the database
    if let Err(e) = state
        .oidc
        .oidc_flow_store
        .insert(
            csrf_state.secret().clone(),
            provider_id,
            &pkce_verifier,
            &nonce,
        )
        .await
    {
        tracing::error!(error = ?e, "Failed to store OIDC flow");
        emit_oidc_route_audit(
            &state,
            uptrakit_audit_log::AuditActionType::from_static(
                uptrakit_audit_log::AuditActionType::AUTH_OIDC_AUTHORIZE,
            ),
            uptrakit_audit_log::AuditOutcome::Failed,
            Some(&provider),
            None,
            serde_json::json!({
                "reason_code": "flow_store_insert_failed",
            }),
        );
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    let response = OidcAuthorizeResponse {
        authorize_url: auth_url.to_string(),
    };

    emit_oidc_route_audit(
        &state,
        uptrakit_audit_log::AuditActionType::from_static(
            uptrakit_audit_log::AuditActionType::AUTH_OIDC_AUTHORIZE,
        ),
        uptrakit_audit_log::AuditOutcome::Success,
        Some(&provider),
        None,
        serde_json::json!({
            "allow_private_network_issuers": allow_private_network_issuers,
            "requested_scopes_count": provider.scopes.split_whitespace().count(),
        }),
    );

    (StatusCode::OK, Json(response)).into_response()
}

/// OIDC callback handler (public)
#[utoipa::path(
    get,
    path = "/api/v1/auth/oidc/callback",
    params(
        ("code" = Option<String>, Query, description = "Authorization code"),
        ("state" = Option<String>, Query, description = "CSRF state"),
        ("error" = Option<String>, Query, description = "Error from provider")
    ),
    responses(
        (status = 302, description = "Redirect to frontend"),
    ),
    tag = "Authentication"
)]
#[tracing::instrument(skip_all)]
pub async fn oidc_callback(
    State(state): State<Arc<AppState>>,
    Query(params): Query<OidcCallbackParams>,
    external_base_url: Option<Extension<crate::extract::ExternalBaseUrl>>,
    headers: HeaderMap,
) -> Response {
    // Handle error from provider
    if let Some(provider_error) = params.error.as_deref() {
        let response = Redirect::to("/login?error=oidc_denied").into_response();
        emit_oidc_callback_audit_for_response(&state, None, None, &response, Some(provider_error));
        return response;
    }

    let (code, csrf_state) = match (params.code, params.state) {
        (Some(c), Some(s)) => (c, s),
        _ => {
            let response = Redirect::to("/login?error=oidc_missing_params").into_response();
            emit_oidc_callback_audit_for_response(&state, None, None, &response, None);
            return response;
        }
    };

    // Stage 1: Validate state token, load provider, build OIDC client
    let ValidatedOidcCallback {
        flow,
        provider,
        client,
        redirect_url,
        allow_private_network_issuers,
    } = match validate_oidc_state(&state, &csrf_state, external_base_url, &headers).await {
        Ok(v) => v,
        Err(validation_failure) => {
            emit_oidc_callback_audit_for_response(
                &state,
                None,
                validation_failure.provider_id,
                &validation_failure.response,
                None,
            );
            return validation_failure.response;
        }
    };

    // Save provider_id before consuming flow fields
    let provider_id = flow.provider_id;

    // Stage 2: Exchange authorization code for tokens and extract claims
    let claims = match exchange_code_for_claims(
        &client,
        code,
        flow.pkce_verifier,
        flow.nonce,
        redirect_url,
        allow_private_network_issuers,
    )
    .await
    {
        Ok(c) => c,
        Err(response) => {
            emit_oidc_callback_audit_for_response(
                &state,
                Some(&provider),
                Some(provider_id),
                &response,
                None,
            );
            return response;
        }
    };

    // Stage 3: Resolve or create the user, sync roles, and produce the final response
    let response = resolve_or_create_oidc_user(&state, provider_id, &provider, claims).await;
    emit_oidc_callback_audit_for_response(
        &state,
        Some(&provider),
        Some(provider_id),
        &response,
        None,
    );
    response
}

/// Stage 1: Look up the pending OIDC flow by CSRF state, load the associated
/// provider, resolve the external base URL, and build the OIDC client.
///
/// Returns `Err(OidcStateValidationFailure)` with the appropriate redirect on
/// any validation failure so the caller can propagate it directly while
/// preserving provider target context for audit emission when available.
async fn validate_oidc_state(
    state: &AppState,
    csrf_state: &str,
    external_base_url: Option<Extension<crate::extract::ExternalBaseUrl>>,
    headers: &HeaderMap,
) -> Result<ValidatedOidcCallback, OidcStateValidationFailure> {
    // Retrieve pending flow from database
    let flow = match state.oidc.oidc_flow_store.take(csrf_state).await {
        Ok(Some(f)) => f,
        Ok(None) => {
            return Err(OidcStateValidationFailure::new(
                Redirect::to("/login?error=oidc_state_expired").into_response(),
                None,
            ));
        }
        Err(e) => {
            tracing::error!(error = ?e, "Failed to retrieve OIDC flow");
            return Err(OidcStateValidationFailure::new(
                Redirect::to("/login?error=oidc_internal_error").into_response(),
                None,
            ));
        }
    };
    let provider_id = flow.provider_id;

    // Load provider
    let provider =
        match find_active_provider(state.db(), state.default_tenant_id, provider_id).await {
            Some(p) => p,
            None => {
                return Err(OidcStateValidationFailure::new(
                    Redirect::to("/login?error=oidc_provider_gone").into_response(),
                    Some(provider_id),
                ));
            }
        };

    let base_url = external_base_url
        .map(|Extension(u)| u.0)
        .or_else(|| base_url_from_headers(headers));
    let base_url = match base_url {
        Some(url) => url,
        None => {
            return Err(OidcStateValidationFailure::new(
                Redirect::to("/login?error=oidc_missing_host").into_response(),
                Some(provider_id),
            ));
        }
    };
    let redirect_url = match RedirectUrl::new(format!("{base_url}/api/v1/auth/oidc/callback")) {
        Ok(url) => url,
        Err(e) => {
            tracing::error!(error = %e, "Invalid OIDC redirect URL during callback");
            return Err(OidcStateValidationFailure::new(
                Redirect::to("/login?error=oidc_invalid_redirect").into_response(),
                Some(provider_id),
            ));
        }
    };
    let multi_tenancy_enabled =
        match crate::settings_store::is_multi_tenancy_enabled(state.db()).await {
            Ok(enabled) => enabled,
            Err(e) => {
                tracing::error!(error = ?e, "Failed to load multi-tenancy mode");
                return Err(OidcStateValidationFailure::new(
                    Redirect::to("/login?error=oidc_discovery_failed").into_response(),
                    Some(provider_id),
                ));
            }
        };
    let allow_private_network_issuers =
        provider.allow_private_network_issuers && !multi_tenancy_enabled;

    // Build OIDC client via discovery
    let client = match build_oidc_client(
        &provider,
        redirect_url.clone(),
        allow_private_network_issuers,
    )
    .await
    {
        Some(c) => c,
        None => {
            return Err(OidcStateValidationFailure::new(
                Redirect::to("/login?error=oidc_discovery_failed").into_response(),
                Some(provider_id),
            ));
        }
    };

    Ok(ValidatedOidcCallback {
        flow,
        provider,
        client,
        redirect_url,
        allow_private_network_issuers,
    })
}

/// Stage 3: Check invite-mode registration gating, resolve or create the user
/// inside a transaction, sync OIDC roles, and produce the final redirect
/// response.
async fn resolve_or_create_oidc_user(
    state: &AppState,
    provider_id: Uuid,
    provider: &oidc_provider::Model,
    claims: ExtractedOidcClaims,
) -> Response {
    let ExtractedOidcClaims {
        sub,
        email,
        email_verified,
        first_name,
        last_name,
        additional_claims,
    } = claims;

    // Pre-check: if registration mode is Invite and auto_create is enabled,
    // check whether this would create a new user requiring a registration token.
    if let Some(response) = check_registration_eligibility(
        state,
        provider_id,
        provider,
        &sub,
        &email,
        first_name.as_deref(),
        last_name.as_deref(),
        &additional_claims,
    )
    .await
    {
        return response;
    }

    // Resolve user inside a transaction to prevent the race where two concurrent
    // OIDC callbacks both see user_count == 1 and both get the owner role.
    let txn = match state.db().begin().await {
        Ok(txn) => txn,
        Err(e) => {
            tracing::error!(error = %e, "Failed to start OIDC callback transaction");
            return Redirect::to("/login?error=oidc_internal_error").into_response();
        }
    };

    let resolution = match resolve_oidc_user(OidcUserParams {
        db: &txn,
        tenant_id: state.default_tenant_id,
        provider_id,
        oidc_subject: &sub,
        email: &email,
        first_name: first_name.as_deref(),
        last_name: last_name.as_deref(),
        auto_create: provider.auto_create_users,
        email_verified,
    })
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = ?e, "OIDC user resolution failed");
            return Redirect::to("/login?error=oidc_internal_error").into_response();
        }
    };

    execute_oidc_resolution(
        state,
        txn,
        resolution,
        provider_id,
        provider,
        &sub,
        &email,
        first_name,
        last_name,
        &additional_claims,
    )
    .await
}

/// Execute the OIDC user resolution match inside the transaction.
///
/// Returns the response to send to the client. The caller is responsible for
/// beginning and committing the transaction.
#[allow(clippy::too_many_arguments)]
async fn execute_oidc_resolution(
    state: &AppState,
    txn: sea_orm::DatabaseTransaction,
    resolution: OidcUserResolution,
    provider_id: Uuid,
    provider: &oidc_provider::Model,
    sub: &str,
    email: &str,
    first_name: Option<String>,
    last_name: Option<String>,
    additional_claims: &serde_json::Value,
) -> Response {
    match resolution {
        OidcUserResolution::LinkedUser(user_id) => {
            let user_id =
                match handle_linked_user(state, &txn, user_id, provider, additional_claims).await {
                    Ok(uid) => uid,
                    Err(response) => return response,
                };
            if let Err(e) = txn.commit().await {
                tracing::error!(error = %e, "Failed to commit OIDC callback transaction");
                return Redirect::to("/login?error=oidc_internal_error").into_response();
            }
            create_oidc_exchange_and_redirect(state, user_id, provider_id).await
        }
        OidcUserResolution::NewUser(user_id) => {
            let (user_id, is_first_user) =
                match handle_new_user(state, &txn, user_id, provider, additional_claims).await {
                    Ok(uid) => uid,
                    Err(response) => return response,
                };
            if let Err(e) = txn.commit().await {
                tracing::error!(error = %e, "Failed to commit OIDC callback transaction");
                return Redirect::to("/login?error=oidc_internal_error").into_response();
            }
            emit_oidc_user_create_audit(
                state,
                Some(user_id),
                Some(provider_id),
                Some(provider.name.as_str()),
                uptrakit_audit_log::AuditOutcome::Success,
                None,
                Some(is_first_user),
            );
            create_oidc_exchange_and_redirect(state, user_id, provider_id).await
        }
        OidcUserResolution::LinkViaPasswordRequired { user_id } => {
            drop(txn);
            handle_link_via_password(
                state,
                provider_id,
                provider,
                sub,
                email,
                first_name,
                last_name,
                additional_claims,
                user_id,
            )
            .await
        }
        OidcUserResolution::LinkViaOidcRequired {
            user_id,
            existing_provider_id,
        } => {
            drop(txn);
            handle_link_via_oidc(
                state,
                provider_id,
                provider,
                sub,
                email,
                first_name,
                last_name,
                additional_claims,
                user_id,
                existing_provider_id,
            )
            .await
        }
        OidcUserResolution::NotAllowed => {
            drop(txn);
            Redirect::to("/login?error=oidc_no_account").into_response()
        }
        OidcUserResolution::EmailNotVerified => {
            drop(txn);
            Redirect::to("/login?error=oidc_email_unverified").into_response()
        }
        OidcUserResolution::Deactivated => {
            drop(txn);
            Redirect::to("/login?error=account_deactivated").into_response()
        }
    }
}

/// Pre-check: when registration mode is Invite and auto-create is enabled,
/// verify whether the OIDC subject already has a link or a matching user.
/// If neither exists and a registration token is required, store a pending
/// registration and return a redirect response. Returns `None` when the
/// normal flow should continue.
#[allow(clippy::too_many_arguments)]
async fn check_registration_eligibility(
    state: &AppState,
    provider_id: Uuid,
    provider: &oidc_provider::Model,
    sub: &str,
    email: &str,
    first_name: Option<&str>,
    last_name: Option<&str>,
    additional_claims: &serde_json::Value,
) -> Option<Response> {
    let reg_settings = state.settings.registration();
    if reg_settings.mode != RegistrationMode::Invite || !provider.auto_create_users {
        return None;
    }

    // Check if an OIDC link already exists for this subject
    let has_link = match UserOidcLink::find()
        .filter(user_oidc_link::Column::ProviderId.eq(provider_id))
        .filter(user_oidc_link::Column::OidcSubject.eq(sub))
        .count(state.db())
        .await
    {
        Ok(n) => n > 0,
        Err(e) => {
            tracing::error!(err = %e, "DB error checking OIDC link");
            return Some(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error",
            ));
        }
    };

    if has_link {
        return None;
    }

    // Check if a user with this email already exists
    let has_user = match User::find()
        .filter(uptrakit_shared_db::entity::user::Column::Email.eq(email))
        .count(state.db())
        .await
    {
        Ok(n) => n > 0,
        Err(e) => {
            tracing::error!(err = %e, "DB error checking user by email");
            return Some(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error",
            ));
        }
    };

    if has_user {
        return None;
    }

    // This would be a brand-new user -- check if token is required
    let is_first_user = User::find()
        .count(state.db())
        .await
        .map(|c| c == 0)
        .unwrap_or(false);

    if !reg_settings.needs_token_for_oidc(is_first_user) {
        return None;
    }

    // Store pending registration and redirect to token input form
    let mapped_roles = extract_mapped_roles(provider, additional_claims);
    let code = match generate_secure_token() {
        Ok(t) => t,
        Err(e) => {
            tracing::error!(err = %e, "failed to generate secure registration code");
            return Some(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error",
            ));
        }
    };

    if let Err(e) = state
        .oidc
        .oidc_registration_store
        .insert(crate::auth::oidc_state::PendingOidcRegistrationParams {
            registration_code: code.clone(),
            provider_id,
            oidc_subject: sub.to_owned(),
            email: email.to_owned(),
            first_name: first_name.map(str::to_owned),
            last_name: last_name.map(str::to_owned),
            mapped_roles,
        })
        .await
    {
        tracing::error!(error = ?e, "Failed to store pending OIDC registration");
        return Some(Redirect::to("/login?error=oidc_internal_error").into_response());
    }

    // Use a hash fragment so the registration code never appears in
    // server-side access logs (HTTP clients strip fragments before sending
    // the request).
    Some(
        Redirect::to(&format!(
            "/login#registration_token_required=true&registration_code={code}"
        ))
        .into_response(),
    )
}

/// Handle the `LinkedUser` resolution: verify the user is still active and
/// sync OIDC roles within the transaction.
///
/// Returns `Ok(user_id)` so the caller can commit the transaction and create
/// the exchange redirect, or `Err(Response)` on failure.
async fn handle_linked_user(
    state: &AppState,
    txn: &sea_orm::DatabaseTransaction,
    user_id: Uuid,
    provider: &oidc_provider::Model,
    additional_claims: &serde_json::Value,
) -> Result<Uuid, Response> {
    // Defense-in-depth: verify user is still active before creating session
    match User::find_by_id(user_id).one(txn).await {
        Ok(Some(user)) if !user.is_active => {
            return Err(Redirect::to("/login?error=account_deactivated").into_response());
        }
        Ok(None) => {
            return Err(Redirect::to("/login?error=oidc_internal_error").into_response());
        }
        Err(e) => {
            tracing::error!(error = ?e, "failed to load user for OIDC login");
            return Err(Redirect::to("/login?error=oidc_internal_error").into_response());
        }
        _ => {}
    }

    // Sync roles
    let _ = sync_oidc_roles(
        txn,
        state.default_tenant_id,
        user_id,
        provider,
        additional_claims,
    )
    .await;

    Ok(user_id)
}

/// Handle the `NewUser` resolution: check if this is the first user (owner
/// setup) and sync OIDC roles within the transaction.
///
/// Returns `Ok(user_id)` so the caller can commit the transaction and create
/// the exchange redirect, or `Err(Response)` on failure.
async fn handle_new_user(
    state: &AppState,
    txn: &sea_orm::DatabaseTransaction,
    user_id: Uuid,
    provider: &oidc_provider::Model,
    additional_claims: &serde_json::Value,
) -> Result<(Uuid, bool), Response> {
    // Atomically check if this is the first user (threshold 1 because the
    // user was just created by resolve_oidc_user) and handle owner role +
    // initial setup inside the same transaction.
    let user_count = match User::find().count(txn).await {
        Ok(n) => n,
        Err(e) => {
            tracing::error!(error = %e, "Failed to count users during OIDC registration");
            return Err(Redirect::to("/login?error=oidc_internal_error").into_response());
        }
    };
    if user_count == 1 {
        // Delete the default 'user' role assigned by resolve_oidc_user
        let _ = UserRole::delete_many()
            .filter(user_role::Column::TenantId.eq(state.default_tenant_id))
            .filter(user_role::Column::UserId.eq(user_id))
            .exec(txn)
            .await;

        // Assign all roles (owner preset)
        if let Err(e) = super::auth::assign_owner_roles(txn, state.default_tenant_id, user_id).await
        {
            tracing::error!(error = ?e, "Failed to assign owner roles to first OIDC user");
        }

        // Complete initial setup (close registration, remove token)
        let mut reg = state.settings.registration();
        if let Err(e) = reg
            .complete_initial_setup(txn, state.default_tenant_id)
            .await
        {
            tracing::error!(error = ?e, "Failed to complete initial setup for first OIDC user");
        }
        state.settings.set_registration(reg).await;

        tracing::info!("first user registered via OIDC, assigned owner role");
    }

    // Sync roles
    let _ = sync_oidc_roles(
        txn,
        state.default_tenant_id,
        user_id,
        provider,
        additional_claims,
    )
    .await;

    Ok((user_id, user_count == 1))
}

/// Handle the `LinkViaPasswordRequired` resolution: store a pending link and
/// redirect to the frontend password-confirmation form.
#[allow(clippy::too_many_arguments)]
async fn handle_link_via_password(
    state: &AppState,
    provider_id: Uuid,
    provider: &oidc_provider::Model,
    sub: &str,
    email: &str,
    first_name: Option<String>,
    last_name: Option<String>,
    additional_claims: &serde_json::Value,
    user_id: Uuid,
) -> Response {
    let mapped_roles = extract_mapped_roles(provider, additional_claims);
    let link_token_value = match generate_secure_token() {
        Ok(t) => t,
        Err(e) => {
            tracing::error!(err = %e, "failed to generate secure link token");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "internal server error");
        }
    };
    let link_token = match store_pending_link(
        state,
        crate::auth::oidc_state::PendingAccountLinkParams {
            token: link_token_value,
            provider_id,
            oidc_subject: sub.to_owned(),
            email: email.to_owned(),
            user_id,
            first_name,
            last_name,
            mapped_roles,
            existing_link_provider_id: None,
        },
    )
    .await
    {
        Ok(token) => token,
        Err(e) => {
            tracing::error!(error = ?e, "Failed to store pending link");
            return Redirect::to("/login?error=oidc_internal_error").into_response();
        }
    };

    link_redirect_with_no_referrer(email, &link_token, None)
}

/// Handle the `LinkViaOidcRequired` resolution: store a pending link and
/// redirect to the frontend OIDC re-authentication form.
#[allow(clippy::too_many_arguments)]
async fn handle_link_via_oidc(
    state: &AppState,
    provider_id: Uuid,
    provider: &oidc_provider::Model,
    sub: &str,
    email: &str,
    first_name: Option<String>,
    last_name: Option<String>,
    additional_claims: &serde_json::Value,
    user_id: Uuid,
    existing_provider_id: Uuid,
) -> Response {
    let mapped_roles = extract_mapped_roles(provider, additional_claims);
    let link_token_value = match generate_secure_token() {
        Ok(t) => t,
        Err(e) => {
            tracing::error!(err = %e, "failed to generate secure link token");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "internal server error");
        }
    };
    let link_token = match store_pending_link(
        state,
        crate::auth::oidc_state::PendingAccountLinkParams {
            token: link_token_value,
            provider_id,
            oidc_subject: sub.to_owned(),
            email: email.to_owned(),
            user_id,
            first_name,
            last_name,
            mapped_roles,
            existing_link_provider_id: Some(existing_provider_id),
        },
    )
    .await
    {
        Ok(token) => token,
        Err(e) => {
            tracing::error!(error = ?e, "Failed to store pending link");
            return Redirect::to("/login?error=oidc_internal_error").into_response();
        }
    };

    link_redirect_with_no_referrer(email, &link_token, Some(existing_provider_id))
}

/// Build a redirect response for account-linking flows, suppressing the
/// `Referer` header so the link token is not forwarded to third-party
/// resources loaded by the login page.
fn link_redirect_with_no_referrer(
    email: &str,
    link_token: &str,
    existing_provider_id: Option<Uuid>,
) -> Response {
    let mut link_headers = HeaderMap::new();
    link_headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    (
        link_headers,
        Redirect::to(&build_link_required_redirect(
            email,
            link_token,
            existing_provider_id,
        )),
    )
        .into_response()
}

/// Exchange an OIDC exchange code for tokens (deferred token creation).
///
/// The exchange code maps to `(user_id, provider_id)` in the database.
/// Actual JWT and refresh tokens are created on-demand here.
#[utoipa::path(
    post,
    path = "/api/v1/auth/oidc/exchange",
    request_body = OidcExchangeRequest,
    responses(
        (status = 200, description = "Exchange successful", body = AuthResponse),
        (status = 400, description = "Invalid or expired exchange code")
    ),
    tag = "Authentication"
)]
#[tracing::instrument(skip_all)]
pub async fn oidc_exchange(
    State(state): State<Arc<AppState>>,
    session_svc: SessionSvc,
    Json(req): Json<OidcExchangeRequest>,
) -> Response {
    let pending = match state.oidc.oidc_token_exchange_store.take(&req.code).await {
        Ok(Some(p)) => p,
        Ok(None) => {
            let response =
                error_response(StatusCode::BAD_REQUEST, "Invalid or expired exchange code");
            emit_oidc_exchange_audit(
                &state,
                uptrakit_audit_log::AuditOutcome::ValidationFailed,
                None,
                response.status(),
                Some("invalid_or_expired_exchange_code"),
            );
            return response;
        }
        Err(e) => {
            tracing::error!(error = ?e, "Failed to retrieve OIDC exchange");
            let response =
                error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            emit_oidc_exchange_audit(
                &state,
                uptrakit_audit_log::AuditOutcome::Failed,
                None,
                response.status(),
                Some("exchange_load_failed"),
            );
            return response;
        }
    };

    let response =
        mint_oidc_auth_response(&state, &session_svc, pending.user_id, pending.provider_id).await;
    let (outcome, reason_code) = if response.status() == StatusCode::OK {
        (uptrakit_audit_log::AuditOutcome::Success, None)
    } else {
        (
            uptrakit_audit_log::AuditOutcome::Failed,
            Some("mint_auth_response_failed"),
        )
    };
    emit_oidc_exchange_audit(
        &state,
        outcome,
        Some(pending.provider_id),
        response.status(),
        reason_code,
    );
    response
}

/// Complete OIDC registration with a registration token (public).
///
/// Used when the OIDC callback determined that a new user would be created but
/// the system requires a registration token (first user or `require_token_for_oidc` enabled).
#[utoipa::path(
    post,
    path = "/api/v1/auth/oidc/complete-registration",
    request_body = OidcCompleteRegistrationRequest,
    responses(
        (status = 200, description = "Registration completed", body = AuthResponse),
        (status = 400, description = "Invalid or expired registration code"),
        (status = 403, description = "Invalid registration token")
    ),
    tag = "Authentication"
)]
#[tracing::instrument(skip_all)]
pub async fn oidc_complete_registration(
    State(state): State<Arc<AppState>>,
    session_svc: SessionSvc,
    Json(req): Json<OidcCompleteRegistrationRequest>,
) -> Result<impl IntoResponse, ApiError> {
    // 1. Validate the registration token first (pure check, no side effects).
    // This must happen before consuming the one-time-use code so that a wrong
    // token does not permanently burn a valid registration_code.
    let reg_settings = state.settings.registration();
    if let Err(err) = reg_settings.validate(Some(req.registration_token.expose_secret())) {
        emit_oidc_user_create_audit(
            &state,
            None,
            None,
            None,
            uptrakit_audit_log::AuditOutcome::Denied,
            Some("registration_not_allowed"),
            None,
        );
        return Err(err.into());
    }

    // 2. Atomically consume the pending registration so the code is one-time use.
    let pending = match state
        .oidc
        .oidc_registration_store
        .take(req.registration_code.expose_secret())
        .await
    {
        Ok(Some(p)) => p,
        Ok(None) => {
            emit_oidc_user_create_audit(
                &state,
                None,
                None,
                None,
                uptrakit_audit_log::AuditOutcome::ValidationFailed,
                Some("invalid_or_expired_registration_code"),
                None,
            );
            return Ok(error_response(
                StatusCode::BAD_REQUEST,
                "Invalid or expired registration code",
            ));
        }
        Err(e) => {
            tracing::error!(error = ?e, "Failed to consume pending OIDC registration");
            emit_oidc_user_create_audit(
                &state,
                None,
                None,
                None,
                uptrakit_audit_log::AuditOutcome::Failed,
                Some("pending_registration_load_failed"),
                None,
            );
            return Ok(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error",
            ));
        }
    };

    // 3. Wrap user creation + first-user check + role assignment in a transaction
    // to prevent the race where two concurrent registrations both see count == 0.
    let txn = match state.db().begin().await {
        Ok(txn) => txn,
        Err(e) => {
            tracing::error!(error = %e, "Failed to start OIDC complete-registration transaction");
            emit_oidc_user_create_audit(
                &state,
                None,
                Some(pending.provider_id),
                None,
                uptrakit_audit_log::AuditOutcome::Failed,
                Some("registration_transaction_start_failed"),
                None,
            );
            return Ok(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error",
            ));
        }
    };

    // 4. Race condition guard: verify user still doesn't exist
    let user_exists = match User::find()
        .filter(uptrakit_shared_db::entity::user::Column::Email.eq(&pending.email))
        .count(&txn)
        .await
    {
        Ok(n) => n > 0,
        Err(e) => {
            tracing::error!(err = %e, "DB error checking for duplicate user during OIDC registration");
            emit_oidc_user_create_audit(
                &state,
                None,
                Some(pending.provider_id),
                None,
                uptrakit_audit_log::AuditOutcome::Failed,
                Some("duplicate_user_check_failed"),
                None,
            );
            return Ok(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error",
            ));
        }
    };

    if user_exists {
        emit_oidc_user_create_audit(
            &state,
            None,
            Some(pending.provider_id),
            None,
            uptrakit_audit_log::AuditOutcome::Denied,
            Some("email_already_exists"),
            None,
        );
        return Ok(error_response(
            StatusCode::CONFLICT,
            "A user with this email already exists",
        ));
    }

    // 5. Create user (no password, same as resolve_oidc_user NewUser path)
    let user_id = generate_uuid();
    let now = OffsetDateTime::now_utc();
    let user_model = uptrakit_shared_db::entity::user::ActiveModel {
        id: Set(user_id),
        email: Set(MaskedEmail::new(pending.email.clone())),
        first_name: Set(pending.first_name.unwrap_or_default()),
        last_name: Set(pending.last_name.unwrap_or_default()),
        password_hash: Set(None),
        is_active: Set(true),
        deactivated_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    };

    if let Err(e) = user_model.insert(&txn).await {
        tracing::error!(error = %e, "Failed to create user during OIDC registration");
        emit_oidc_user_create_audit(
            &state,
            Some(user_id),
            Some(pending.provider_id),
            None,
            uptrakit_audit_log::AuditOutcome::Failed,
            Some("user_insert_failed"),
            None,
        );
        return Ok(error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Internal server error",
        ));
    }

    // 6. Create OIDC link
    let link = user_oidc_link::ActiveModel {
        id: Set(generate_uuid()),
        user_id: Set(user_id),
        provider_id: Set(pending.provider_id),
        oidc_subject: Set(pending.oidc_subject),
        linked_at: Set(now),
    };
    if let Err(e) = link.insert(&txn).await {
        tracing::error!(error = %e, "Failed to create OIDC link during registration");
        emit_oidc_user_create_audit(
            &state,
            Some(user_id),
            Some(pending.provider_id),
            None,
            uptrakit_audit_log::AuditOutcome::Failed,
            Some("oidc_link_insert_failed"),
            None,
        );
        return Ok(error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Internal server error",
        ));
    }

    // 7. Atomically check if this is the first user (threshold 1 because we just created)
    //    and assign owner role + complete initial setup inside the same transaction.
    let is_first_user = match super::auth::handle_first_user_setup(
        &txn,
        &state.settings,
        state.default_tenant_id,
        user_id,
        1,
    )
    .await
    {
        Ok(is_first) => is_first,
        Err(e) => {
            tracing::error!(error = ?e, "Failed to handle first-user setup for OIDC complete-registration");
            false
        }
    };

    if is_first_user {
        tracing::info!("first user registered via OIDC complete-registration, assigned all roles");
    } else {
        // Assign default viewer role
        if let Err(e) =
            super::auth::assign_viewer_role(&txn, state.default_tenant_id, user_id).await
        {
            tracing::error!(error = ?e, "Failed to assign default role during OIDC registration");
        }
    }

    // 8. Sync OIDC roles using stored mapped_roles
    if !pending.mapped_roles.is_empty()
        && let Some(provider) =
            find_active_provider(&txn, state.default_tenant_id, pending.provider_id).await
    {
        let fake_claims = build_fake_claims_for_sync(&provider, &pending.mapped_roles);
        let _ = sync_oidc_roles(
            &txn,
            state.default_tenant_id,
            user_id,
            &provider,
            &fake_claims,
        )
        .await;
    }

    if let Err(e) = txn.commit().await {
        tracing::error!(error = %e, "Failed to commit OIDC complete-registration transaction");
        emit_oidc_user_create_audit(
            &state,
            Some(user_id),
            Some(pending.provider_id),
            None,
            uptrakit_audit_log::AuditOutcome::Failed,
            Some("registration_commit_failed"),
            Some(is_first_user),
        );
        return Ok(error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Internal server error",
        ));
    }

    emit_oidc_user_create_audit(
        &state,
        Some(user_id),
        Some(pending.provider_id),
        None,
        uptrakit_audit_log::AuditOutcome::Success,
        None,
        Some(is_first_user),
    );

    // 9. Create session + JWT
    Ok(mint_oidc_auth_response(&state, &session_svc, user_id, pending.provider_id).await)
}

/// Link a pending OIDC account (public)
#[utoipa::path(
    post,
    path = "/api/v1/auth/oidc/link",
    request_body = OidcLinkRequest,
    responses(
        (status = 200, description = "Account linked and logged in", body = AuthResponse),
        (status = 400, description = "Invalid link token or verification failed"),
        (status = 401, description = "Verification failed")
    ),
    tag = "Authentication"
)]
#[tracing::instrument(skip_all)]
pub async fn oidc_link(
    State(state): State<Arc<AppState>>,
    session_svc: SessionSvc,
    req: axum::extract::Request,
) -> Response {
    // Parse the body manually since we also need headers
    let (parts, body) = req.into_parts();
    let bytes = match axum::body::to_bytes(body, 1024 * 16).await {
        Ok(b) => b,
        Err(_) => {
            let response = error_response(StatusCode::BAD_REQUEST, "Invalid request body");
            emit_oidc_link_audit(
                &state,
                uptrakit_audit_log::AuditOutcome::ValidationFailed,
                None,
                response.status(),
                Some("invalid_request_body"),
            );
            return response;
        }
    };
    let link_req: OidcLinkRequest = match serde_json::from_slice(&bytes) {
        Ok(r) => r,
        Err(_) => {
            let response = error_response(StatusCode::BAD_REQUEST, "Invalid JSON");
            emit_oidc_link_audit(
                &state,
                uptrakit_audit_log::AuditOutcome::ValidationFailed,
                None,
                response.status(),
                Some("invalid_json"),
            );
            return response;
        }
    };

    // Retrieve pending link from database
    let pending = match state
        .oidc
        .account_link_store
        .take(link_req.link_token.expose_secret())
        .await
    {
        Ok(Some(p)) => p,
        Ok(None) => {
            let response =
                error_response(StatusCode::BAD_REQUEST, "Link token not found or expired");
            emit_oidc_link_audit(
                &state,
                uptrakit_audit_log::AuditOutcome::ValidationFailed,
                None,
                response.status(),
                Some("invalid_or_expired_link_token"),
            );
            return response;
        }
        Err(e) => {
            tracing::error!(error = ?e, "Failed to retrieve pending link");
            let response =
                error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            emit_oidc_link_audit(
                &state,
                uptrakit_audit_log::AuditOutcome::Failed,
                None,
                response.status(),
                Some("pending_link_load_failed"),
            );
            return response;
        }
    };

    // Verify ownership
    let (verified, denied_reason_code) = if let Some(ref pwd) = link_req.password {
        if let Some(message) = password::validate_password_length(pwd.expose_secret()) {
            let response = error_response(StatusCode::BAD_REQUEST, message);
            emit_oidc_link_audit(
                &state,
                uptrakit_audit_log::AuditOutcome::ValidationFailed,
                Some(pending.provider_id),
                response.status(),
                Some("password_length_invalid"),
            );
            return response;
        }
        // Password verification
        let user = match User::find_by_id(pending.user_id).one(state.db()).await {
            Ok(Some(u)) => u,
            _ => {
                let response = error_response(StatusCode::UNAUTHORIZED, "User not found");
                emit_oidc_link_audit(
                    &state,
                    uptrakit_audit_log::AuditOutcome::Denied,
                    Some(pending.provider_id),
                    response.status(),
                    Some("ownership_user_not_found"),
                );
                return response;
            }
        };
        let hash = match user.password_hash.as_ref() {
            Some(h) => h,
            None => {
                let response = error_response(StatusCode::UNAUTHORIZED, "User has no password");
                emit_oidc_link_audit(
                    &state,
                    uptrakit_audit_log::AuditOutcome::Denied,
                    Some(pending.provider_id),
                    response.status(),
                    Some("ownership_user_has_no_password"),
                );
                return response;
            }
        };
        (
            matches!(
                password::verify_password(pwd.expose_secret(), hash.expose_secret()),
                Ok(true)
            ),
            Some("ownership_verification_failed"),
        )
    } else {
        // Bearer token verification (OIDC-to-OIDC linking) — now JWT-based
        let bearer = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .map(|s| s.to_string());

        if let Some(token) = bearer {
            match state.auth.jwt.decode_access_token(&token) {
                Ok(claims) => match uuid::Uuid::parse_str(&claims.sub) {
                    Ok(uid) if uid == pending.user_id => (true, None),
                    Ok(_) => (false, Some("user_mismatch")),
                    Err(_) => (false, Some("invalid_bearer_subject")),
                },
                Err(_) => (false, Some("invalid_bearer_token")),
            }
        } else {
            (false, Some("missing_bearer_token"))
        }
    };

    if !verified {
        let response = error_response(StatusCode::UNAUTHORIZED, "Verification failed");
        emit_oidc_link_audit(
            &state,
            uptrakit_audit_log::AuditOutcome::Denied,
            Some(pending.provider_id),
            response.status(),
            denied_reason_code,
        );
        return response;
    }

    // Create the link
    let link = user_oidc_link::ActiveModel {
        id: Set(generate_uuid()),
        user_id: Set(pending.user_id),
        provider_id: Set(pending.provider_id),
        oidc_subject: Set(pending.oidc_subject),
        linked_at: Set(OffsetDateTime::now_utc()),
    };

    if let Err(e) = link.insert(state.db()).await {
        tracing::error!(error = %e, "Failed to create OIDC link");
        let response = error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        emit_oidc_link_audit(
            &state,
            uptrakit_audit_log::AuditOutcome::Failed,
            Some(pending.provider_id),
            response.status(),
            Some("oidc_link_insert_failed"),
        );
        return response;
    }

    // Sync roles if we have mapped roles
    if !pending.mapped_roles.is_empty()
        && let Some(provider) =
            find_active_provider(state.db(), state.default_tenant_id, pending.provider_id).await
    {
        let fake_claims = build_fake_claims_for_sync(&provider, &pending.mapped_roles);
        let _ = sync_oidc_roles(
            state.db(),
            state.default_tenant_id,
            pending.user_id,
            &provider,
            &fake_claims,
        )
        .await;
    }

    let response =
        mint_oidc_auth_response(&state, &session_svc, pending.user_id, pending.provider_id).await;
    let (outcome, reason_code) = if response.status() == StatusCode::OK {
        (uptrakit_audit_log::AuditOutcome::Success, None)
    } else {
        (
            uptrakit_audit_log::AuditOutcome::Failed,
            Some("mint_auth_response_failed"),
        )
    };
    emit_oidc_link_audit(
        &state,
        outcome,
        Some(pending.provider_id),
        response.status(),
        reason_code,
    );
    response
}

// Helper functions

/// Concrete type of a `CoreClient` returned by OIDC discovery: auth URL is set,
/// token and user-info URLs may be set (depending on provider metadata),
/// device-auth, introspection and revocation are not set.
type DiscoveredCoreClient = CoreClient<
    EndpointSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointMaybeSet,
    EndpointMaybeSet,
>;

/// Build an OIDC `CoreClient` for the given provider via OIDC discovery.
///
/// Returns `None` if the issuer URL is invalid or if discovery fails.
async fn build_oidc_client(
    provider: &oidc_provider::Model,
    redirect_url: RedirectUrl,
    allow_private_network_issuers: bool,
) -> Option<DiscoveredCoreClient> {
    let issuer_url = IssuerUrl::new(provider.issuer_url.clone())
        .map_err(|e| tracing::error!(error = %e, provider_id = %provider.id, "Invalid OIDC issuer URL for provider"))
        .ok()?;
    let http_client = crate::oidc_http_client::OidcHttpClient::new(allow_private_network_issuers)
        .map_err(|e| tracing::error!(error = %e, "Failed to build OIDC HTTP client"))
        .ok()?;
    let provider_metadata = CoreProviderMetadata::discover_async(issuer_url, &http_client)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, provider_id = %provider.id, "OIDC provider discovery failed for provider");
        })
        .ok()?;
    Some(
        CoreClient::from_provider_metadata(
            provider_metadata,
            ClientId::new(provider.client_id.clone()),
            Some(ClientSecret::new(
                provider.client_secret.expose_secret().to_string(),
            )),
        )
        .set_redirect_uri(redirect_url),
    )
}

/// Exchange an authorization code for tokens, validate the ID token, and
/// extract claims into [`ExtractedOidcClaims`].
///
/// On any error returns `Err(Response)` with an appropriate redirect so the
/// caller can propagate it directly.
async fn exchange_code_for_claims(
    client: &DiscoveredCoreClient,
    code: String,
    pkce_verifier: PkceCodeVerifier,
    nonce: Nonce,
    redirect_url: RedirectUrl,
    allow_private_network_issuers: bool,
) -> Result<ExtractedOidcClaims, Response> {
    let http_client = crate::oidc_http_client::OidcHttpClient::new(allow_private_network_issuers)
        .map_err(|e| {
        tracing::error!(error = %e, "Failed to build OIDC HTTP client");
        Redirect::to("/login?error=oidc_token_exchange_failed").into_response()
    })?;
    let token_request = client
        .exchange_code(AuthorizationCode::new(code))
        .map_err(|e| {
            tracing::error!(error = %e, "OIDC token endpoint not configured");
            Redirect::to("/login?error=oidc_token_exchange_failed").into_response()
        })?;
    let token_response = token_request
        .set_redirect_uri(std::borrow::Cow::Owned(redirect_url))
        .set_pkce_verifier(pkce_verifier)
        .request_async(&http_client)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "OIDC token exchange failed");
            Redirect::to("/login?error=oidc_token_exchange_failed").into_response()
        })?;

    let id_token = token_response
        .id_token()
        .ok_or_else(|| Redirect::to("/login?error=oidc_no_id_token").into_response())?;

    let id_token_verifier = client.id_token_verifier();
    let claims = id_token.claims(&id_token_verifier, &nonce).map_err(|e| {
        tracing::error!(error = %e, "OIDC ID token validation failed");
        Redirect::to("/login?error=oidc_token_validation_failed").into_response()
    })?;

    let sub = claims.subject().to_string();
    let email = claims.email().map(|e| e.to_string()).unwrap_or_default();
    let email_verified = claims.email_verified();
    let first_name = claims
        .given_name()
        .and_then(|n| n.get(None))
        .map(|n| n.to_string());
    let last_name = claims
        .family_name()
        .and_then(|n| n.get(None))
        .map(|n| n.to_string());

    if email.is_empty() {
        return Err(Redirect::to("/login?error=oidc_no_email").into_response());
    }

    let additional_claims = serde_json::to_value(claims.additional_claims()).unwrap_or_default();

    Ok(ExtractedOidcClaims {
        sub,
        email,
        email_verified,
        first_name,
        last_name,
        additional_claims,
    })
}

/// Create an OIDC refresh token, access token, and return a complete
/// [`AuthResponse`].
///
/// This is the shared session-creation step used by [`oidc_exchange`],
/// [`oidc_complete_registration`], and [`oidc_link`] after any provider-
/// specific work (user creation, linking, role sync) has been committed.
async fn mint_oidc_auth_response(
    state: &AppState,
    session_svc: &SessionService,
    user_id: Uuid,
    provider_id: Uuid,
) -> Response {
    let refresh_token = match session_svc
        .create_refresh_token(user_id, AuthMethod::Oidc { provider_id }, None, None)
        .await
    {
        Ok(t) => t,
        Err(e) => {
            tracing::error!(error = ?e, "Failed to create OIDC refresh token");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let user = match User::find_by_id(user_id).one(state.db()).await {
        Ok(Some(u)) => u,
        _ => return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error"),
    };

    let permissions = crate::middleware::require_auth::get_user_permissions(
        state.db(),
        state.default_tenant_id,
        user_id,
    )
    .await
    .unwrap_or_default();

    let access_token =
        match state
            .auth
            .jwt
            .create_access_token(user_id, &permissions, "oidc", Some(provider_id))
        {
            Ok(t) => t,
            Err(e) => {
                tracing::error!(error = ?e, "Failed to create OIDC access token");
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        };

    let cookie = set_refresh_token_cookie(&refresh_token);
    let response = AuthResponse {
        access_token: SecretString::new(access_token),
        refresh_token: SecretString::new(refresh_token),
        expires_in: state.auth.jwt.expires_in(),
        token_type: "Bearer".to_string(),
        user: super::auth::UserResponse {
            id: user.id,
            email: user.email.expose_email().to_string(),
            first_name: user.first_name,
            last_name: user.last_name,
            permissions,
        },
    };

    (
        StatusCode::OK,
        [(header::SET_COOKIE, cookie)],
        Json(response),
    )
        .into_response()
}

/// Store only (user_id, provider_id) in the database and redirect with exchange code.
/// Token creation is deferred to the `oidc_exchange` endpoint.
async fn create_oidc_exchange_and_redirect(
    state: &AppState,
    user_id: uuid::Uuid,
    provider_id: uuid::Uuid,
) -> Response {
    // Generate exchange code
    let exchange_code = match generate_secure_token() {
        Ok(t) => t,
        Err(e) => {
            tracing::error!(err = %e, "failed to generate secure exchange code");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "internal server error");
        }
    };

    if let Err(e) = state
        .oidc
        .oidc_token_exchange_store
        .insert(exchange_code.clone(), user_id, provider_id)
        .await
    {
        tracing::error!(error = ?e, "Failed to store OIDC exchange");
        emit_oidc_exchange_audit(
            state,
            uptrakit_audit_log::AuditOutcome::Failed,
            Some(provider_id),
            StatusCode::SEE_OTHER,
            Some("exchange_store_insert_failed"),
        );
        return Redirect::to("/login?error=oidc_session_failed").into_response();
    }

    Redirect::to(&format!("/login?oidc_code={exchange_code}")).into_response()
}

async fn store_pending_link(
    state: &AppState,
    params: crate::auth::oidc_state::PendingAccountLinkParams,
) -> std::result::Result<String, rootcause::Report<crate::auth::oidc_state::OidcStoreError>> {
    let link_token = params.token.clone();
    state.oidc.account_link_store.insert(params).await?;
    Ok(link_token)
}

fn build_link_required_redirect(
    email: &str,
    link_token: &str,
    existing_provider_id: Option<Uuid>,
) -> String {
    let encoded_email =
        percent_encoding::utf8_percent_encode(email, percent_encoding::NON_ALPHANUMERIC);
    let encoded_link_token =
        percent_encoding::utf8_percent_encode(link_token, percent_encoding::NON_ALPHANUMERIC);

    match existing_provider_id {
        Some(provider_id) => format!(
            "/login?link_required=true&email={encoded_email}&link_provider_id={provider_id}#link_token={encoded_link_token}"
        ),
        None => format!(
            "/login?link_required=true&email={encoded_email}#link_token={encoded_link_token}"
        ),
    }
}

fn base_url_from_headers(headers: &HeaderMap) -> Option<String> {
    let origin = headers
        .get("origin")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim_end_matches('/').to_string());

    if origin.as_deref().is_some_and(|s| !s.is_empty()) {
        return origin;
    }

    headers
        .get("host")
        .and_then(|v| v.to_str().ok())
        .map(|h| format!("https://{}", h.trim_end_matches('/')))
}

#[cfg(test)]
mod tests {
    use super::{base_url_from_headers, build_link_required_redirect};
    use axum::http::{HeaderMap, HeaderValue};
    use uuid::Uuid;

    #[test]
    fn base_url_prefers_origin() {
        let mut headers = HeaderMap::new();
        headers.insert("origin", HeaderValue::from_static("https://example.test/"));
        headers.insert("host", HeaderValue::from_static("ignored.test"));

        let base = base_url_from_headers(&headers).unwrap();
        assert_eq!(base, "https://example.test");
    }

    #[test]
    fn base_url_uses_host_when_origin_missing() {
        let mut headers = HeaderMap::new();
        headers.insert("host", HeaderValue::from_static("example.test:8443"));

        let base = base_url_from_headers(&headers).unwrap();
        assert_eq!(base, "https://example.test:8443");
    }

    #[test]
    fn base_url_none_when_headers_missing() {
        let headers = HeaderMap::new();
        let base = base_url_from_headers(&headers);
        assert!(base.is_none());
    }

    #[test]
    fn link_redirect_uses_fragment_for_token() {
        let redirect = build_link_required_redirect("user@example.com", "link/token", None);
        assert_eq!(
            redirect,
            "/login?link_required=true&email=user%40example%2Ecom#link_token=link%2Ftoken"
        );
    }

    #[test]
    fn link_redirect_keeps_provider_id_in_query() {
        let provider_id = Uuid::nil();
        let redirect = build_link_required_redirect("user@example.com", "token", Some(provider_id));
        assert_eq!(
            redirect,
            "/login?link_required=true&email=user%40example%2Ecom&link_provider_id=00000000-0000-0000-0000-000000000000#link_token=token"
        );
    }
}

#[cfg(all(test, feature = "db-sqlite", feature = "oidc"))]
mod audit_tests {
    use crate::auth::oidc_state::PendingAccountLinkParams;
    use crate::auth::oidc_state::PendingOidcRegistrationParams;
    use crate::auth::password;
    use crate::test_harness::TestApp;
    use axum::body::Body;
    use axum::http::Request;
    use http::header;
    use openidconnect::{Nonce, PkceCodeChallenge};
    use sea_orm::{
        ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, Set,
    };
    use time::OffsetDateTime;
    use tower::ServiceExt;
    use uptrakit_shared_db::entity::prelude::User;
    use uptrakit_shared_db::entity::{audit_log, oidc_provider, user};
    use uptrakit_shared_types::MaskedEmail;

    const ACTION_AUTH_OIDC_EXCHANGE: &str = uptrakit_audit_log::AuditActionType::AUTH_OIDC_EXCHANGE;
    const ACTION_AUTH_OIDC_LINK: &str = uptrakit_audit_log::AuditActionType::AUTH_OIDC_LINK;

    async fn tenant_audit_row_for_action(
        db: &sea_orm::DatabaseConnection,
        action_type: &'static str,
    ) -> audit_log::Model {
        for _ in 0..50 {
            if let Some(row) = audit_log::Entity::find()
                .filter(audit_log::Column::ActionType.eq(action_type))
                .order_by_desc(audit_log::Column::OccurredAt)
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

    async fn insert_test_user_with_password(
        db: &sea_orm::DatabaseConnection,
        email: &str,
        password_plaintext: &str,
    ) -> uuid::Uuid {
        let user_id = uuid::Uuid::now_v7();
        let now = OffsetDateTime::now_utc();
        let password_hash =
            password::hash_password(password_plaintext).expect("hash user test password");
        user::ActiveModel {
            id: Set(user_id),
            email: Set(MaskedEmail::new(email.to_string())),
            first_name: Set("Oidc".to_string()),
            last_name: Set("Audit".to_string()),
            password_hash: Set(Some(password_hash)),
            is_active: Set(true),
            deactivated_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(db)
        .await
        .expect("insert test user");
        user_id
    }

    async fn drop_table(db: &sea_orm::DatabaseConnection, table: &str) {
        db.execute_unprepared(&format!("DROP TABLE {table};"))
            .await
            .expect("drop table");
    }

    async fn insert_active_oidc_provider(
        db: &sea_orm::DatabaseConnection,
        tenant_id: uuid::Uuid,
        name: &str,
        slug: &str,
    ) -> uuid::Uuid {
        let provider_id = uuid::Uuid::now_v7();
        let now = OffsetDateTime::now_utc();
        oidc_provider::ActiveModel {
            id: Set(provider_id),
            tenant_id: Set(tenant_id),
            name: Set(name.to_string()),
            slug: Set(slug.to_string()),
            logo_url: Set(None),
            issuer_url: Set("https://issuer.example.test".to_string()),
            client_id: Set("client-id".to_string()),
            client_secret: Set(uptrakit_crypto::EncryptedString::new(
                "client-secret".to_string(),
                "uptrakit:oidc_providers:client_secret",
            )
            .expect("encrypt client secret")),
            scopes: Set("openid email profile".to_string()),
            auto_create_users: Set(true),
            allow_private_network_issuers: Set(false),
            role_claim_path: Set(None),
            role_mapping: Set(oidc_provider::RoleMapping(std::collections::HashMap::new())),
            is_active: Set(true),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .expect("insert oidc provider");
        provider_id
    }

    #[tokio::test]
    async fn oidc_authorize_missing_host_writes_validation_failed_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();
        let provider_id = insert_active_oidc_provider(
            &app.db,
            app.state.default_tenant_id,
            "OIDC Exchange Success",
            "oidc-exchange-success",
        )
        .await;

        let response = client
            .get(&format!("/api/v1/auth/oidc/{provider_id}/authorize"))
            .send()
            .await;
        assert_eq!(response.status(), http::StatusCode::BAD_REQUEST);

        let row = tenant_audit_row_for_action(
            &app.db,
            uptrakit_audit_log::AuditActionType::AUTH_OIDC_AUTHORIZE,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::ValidationFailed.as_str()
        );
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::Oidc.as_str()
        );
        assert_eq!(row.target_type.as_deref(), Some("oidc_provider"));
        let provider_id_str = provider_id.to_string();
        assert_eq!(row.target_id.as_deref(), Some(provider_id_str.as_str()));
        let details = row.details_json.expect("audit details");
        assert_eq!(
            details["reason_code"],
            serde_json::json!("missing_host_header")
        );
    }

    #[tokio::test]
    async fn oidc_callback_provider_error_writes_denied_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();

        let response = client
            .get("/api/v1/auth/oidc/callback?error=access_denied")
            .send()
            .await;
        assert_eq!(response.status(), http::StatusCode::SEE_OTHER);

        let row = tenant_audit_row_for_action(
            &app.db,
            uptrakit_audit_log::AuditActionType::AUTH_OIDC_CALLBACK,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::Oidc.as_str()
        );
        assert!(row.target_type.is_none());
        let details = row.details_json.expect("audit details");
        assert_eq!(details["reason_code"], serde_json::json!("oidc_denied"));
        assert_eq!(
            details["provider_error_code"],
            serde_json::json!("access_denied")
        );
    }

    #[tokio::test]
    async fn oidc_callback_stage1_missing_host_keeps_provider_target_in_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();
        let provider_id = insert_active_oidc_provider(
            &app.db,
            app.state.default_tenant_id,
            "OIDC Exchange Mint Failure",
            "oidc-exchange-mint-failure",
        )
        .await;
        let now = OffsetDateTime::now_utc();
        let csrf_state = "pending-stage1-state";
        let (_pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();
        let nonce = Nonce::new_random();

        oidc_provider::ActiveModel {
            id: Set(provider_id),
            tenant_id: Set(app.state.default_tenant_id),
            name: Set("OIDC Callback Test".to_string()),
            slug: Set("oidc-callback-test".to_string()),
            logo_url: Set(None),
            issuer_url: Set("https://issuer.example.test".to_string()),
            client_id: Set("client-id".to_string()),
            client_secret: Set(uptrakit_crypto::EncryptedString::new(
                "client-secret".to_string(),
                "uptrakit:oidc_providers:client_secret",
            )
            .expect("encrypt client secret")),
            scopes: Set("openid email profile".to_string()),
            auto_create_users: Set(true),
            allow_private_network_issuers: Set(false),
            role_claim_path: Set(None),
            role_mapping: Set(oidc_provider::RoleMapping(std::collections::HashMap::new())),
            is_active: Set(true),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(&app.db)
        .await
        .expect("insert oidc provider");

        app.state
            .oidc
            .oidc_flow_store
            .insert(csrf_state.to_string(), provider_id, &pkce_verifier, &nonce)
            .await
            .expect("store pending oidc flow");

        let response = client
            .get(&format!(
                "/api/v1/auth/oidc/callback?code=auth-code&state={csrf_state}"
            ))
            .send()
            .await;
        assert_eq!(response.status(), http::StatusCode::SEE_OTHER);

        let row = tenant_audit_row_for_action(
            &app.db,
            uptrakit_audit_log::AuditActionType::AUTH_OIDC_CALLBACK,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::ValidationFailed.as_str()
        );
        assert_eq!(row.target_type.as_deref(), Some("oidc_provider"));
        let provider_id_str = provider_id.to_string();
        assert_eq!(row.target_id.as_deref(), Some(provider_id_str.as_str()));
        let details = row.details_json.expect("audit details");
        assert_eq!(
            details["reason_code"],
            serde_json::json!("oidc_missing_host")
        );
    }

    #[tokio::test]
    async fn oidc_complete_registration_writes_user_create_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();
        let registration_code = "pending-oidc-registration";
        let provider_id = insert_active_oidc_provider(
            &app.db,
            app.state.default_tenant_id,
            "OIDC Link Session Mint Failure",
            "oidc-link-session-mint-failure",
        )
        .await;
        let now = OffsetDateTime::now_utc();

        oidc_provider::ActiveModel {
            id: Set(provider_id),
            tenant_id: Set(app.state.default_tenant_id),
            name: Set("OIDC Test".to_string()),
            slug: Set("oidc-test".to_string()),
            logo_url: Set(None),
            issuer_url: Set("https://issuer.example.test".to_string()),
            client_id: Set("client-id".to_string()),
            client_secret: Set(uptrakit_crypto::EncryptedString::new(
                "client-secret".to_string(),
                "uptrakit:oidc_providers:client_secret",
            )
            .expect("encrypt client secret")),
            scopes: Set("openid email profile".to_string()),
            auto_create_users: Set(true),
            allow_private_network_issuers: Set(false),
            role_claim_path: Set(None),
            role_mapping: Set(oidc_provider::RoleMapping(std::collections::HashMap::new())),
            is_active: Set(true),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(&app.db)
        .await
        .expect("insert oidc provider");

        app.state
            .oidc
            .oidc_registration_store
            .insert(PendingOidcRegistrationParams {
                registration_code: registration_code.to_string(),
                provider_id,
                oidc_subject: "oidc-subject".to_string(),
                email: "oidc-user@test.local".to_string(),
                first_name: Some("Oidc".to_string()),
                last_name: Some("User".to_string()),
                mapped_roles: Vec::new(),
            })
            .await
            .expect("store pending registration");

        let (status, _body): (http::StatusCode, serde_json::Value) = client
            .post_json(
                "/api/v1/auth/oidc/complete-registration",
                &serde_json::json!({
                    "registration_code": registration_code,
                    "registration_token": "unused-for-open-registration"
                }),
            )
            .send_json()
            .await;
        assert_eq!(status, http::StatusCode::OK);

        let row =
            tenant_audit_row_for_action(&app.db, uptrakit_audit_log::AuditActionType::USER_CREATE)
                .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::Oidc.as_str()
        );
        assert_eq!(row.target_type.as_deref(), Some("user"));
        let details = row.details_json.expect("audit details");
        assert_eq!(details["auth_method"], serde_json::json!("oidc"));
        assert!(details.get("provider_id").is_some());
    }

    #[tokio::test]
    async fn oidc_exchange_success_writes_success_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();
        let email = "oidc-exchange-success@test.local";
        let (register_status, _register_body): (http::StatusCode, serde_json::Value) = client
            .post_json(
                "/api/v1/auth/register",
                &serde_json::json!({
                    "email": email,
                    "password": "password123",
                    "first_name": "Oidc",
                    "last_name": "Audit",
                }),
            )
            .send_json()
            .await;
        assert_eq!(register_status, http::StatusCode::CREATED);
        let user_id = User::find()
            .filter(user::Column::Email.eq(email))
            .one(&app.db)
            .await
            .expect("query registered user")
            .expect("registered user should exist")
            .id;
        let provider_id = insert_active_oidc_provider(
            &app.db,
            app.state.default_tenant_id,
            "OIDC Exchange Mint Failure",
            "oidc-exchange-mint-failure",
        )
        .await;
        let exchange_code = "oidc-exchange-success-code";
        app.state
            .oidc
            .oidc_token_exchange_store
            .insert(exchange_code.to_string(), user_id, provider_id)
            .await
            .expect("store exchange code");

        let (status, _body): (http::StatusCode, serde_json::Value) = client
            .post_json(
                "/api/v1/auth/oidc/exchange",
                &serde_json::json!({ "code": exchange_code }),
            )
            .send_json()
            .await;
        assert_eq!(status, http::StatusCode::OK);

        let row = tenant_audit_row_for_action(&app.db, ACTION_AUTH_OIDC_EXCHANGE).await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::Oidc.as_str()
        );
        assert_eq!(row.target_type.as_deref(), Some("oidc_provider"));
        let details = row.details_json.expect("audit details");
        assert_eq!(details["http_status"], serde_json::json!(200));
    }

    #[tokio::test]
    async fn oidc_exchange_invalid_code_writes_validation_failed_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();

        let (status, _body): (http::StatusCode, serde_json::Value) = client
            .post_json(
                "/api/v1/auth/oidc/exchange",
                &serde_json::json!({ "code": "missing-exchange-code" }),
            )
            .send_json()
            .await;
        assert_eq!(status, http::StatusCode::BAD_REQUEST);

        let row = tenant_audit_row_for_action(&app.db, ACTION_AUTH_OIDC_EXCHANGE).await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::ValidationFailed.as_str()
        );
        let details = row.details_json.expect("audit details");
        assert_eq!(
            details["reason_code"],
            serde_json::json!("invalid_or_expired_exchange_code")
        );
    }

    #[tokio::test]
    async fn oidc_exchange_load_failure_writes_failed_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();

        drop_table(&app.db, "pending_oidc_token_exchanges").await;

        let (status, _body): (http::StatusCode, serde_json::Value) = client
            .post_json(
                "/api/v1/auth/oidc/exchange",
                &serde_json::json!({ "code": "any-code" }),
            )
            .send_json()
            .await;
        assert_eq!(status, http::StatusCode::INTERNAL_SERVER_ERROR);

        let row = tenant_audit_row_for_action(&app.db, ACTION_AUTH_OIDC_EXCHANGE).await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Failed.as_str()
        );
        let details = row.details_json.expect("audit details");
        assert_eq!(
            details["reason_code"],
            serde_json::json!("exchange_load_failed")
        );
    }

    #[tokio::test]
    async fn oidc_exchange_mint_failure_writes_failed_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();
        let user_id =
            insert_test_user_with_password(&app.db, "oidc-exchange-mint@test.local", "password123")
                .await;
        let provider_id = uuid::Uuid::now_v7();
        let exchange_code = "oidc-exchange-mint-failure";
        app.state
            .oidc
            .oidc_token_exchange_store
            .insert(exchange_code.to_string(), user_id, provider_id)
            .await
            .expect("store exchange code");

        drop_table(&app.db, "sessions").await;

        let (status, _body): (http::StatusCode, serde_json::Value) = client
            .post_json(
                "/api/v1/auth/oidc/exchange",
                &serde_json::json!({ "code": exchange_code }),
            )
            .send_json()
            .await;
        assert_eq!(status, http::StatusCode::INTERNAL_SERVER_ERROR);

        let row = tenant_audit_row_for_action(&app.db, ACTION_AUTH_OIDC_EXCHANGE).await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Failed.as_str()
        );
        let details = row.details_json.expect("audit details");
        assert_eq!(
            details["reason_code"],
            serde_json::json!("mint_auth_response_failed")
        );
    }

    #[tokio::test]
    async fn oidc_link_invalid_body_writes_validation_failed_audit_event() {
        let app = TestApp::new().await;

        let oversized = vec![b'a'; (1024 * 16) + 1];
        let response = app
            .router
            .clone()
            .oneshot(
                Request::builder()
                    .method(http::Method::POST)
                    .uri("/api/v1/auth/oidc/link")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(oversized))
                    .expect("build request"),
            )
            .await
            .expect("request should succeed");
        assert_eq!(response.status(), http::StatusCode::BAD_REQUEST);

        let row = tenant_audit_row_for_action(&app.db, ACTION_AUTH_OIDC_LINK).await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::ValidationFailed.as_str()
        );
        let details = row.details_json.expect("audit details");
        assert_eq!(
            details["reason_code"],
            serde_json::json!("invalid_request_body")
        );
    }

    #[tokio::test]
    async fn oidc_link_invalid_json_writes_validation_failed_audit_event() {
        let app = TestApp::new().await;

        let response = app
            .router
            .clone()
            .oneshot(
                Request::builder()
                    .method(http::Method::POST)
                    .uri("/api/v1/auth/oidc/link")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(br#"{"link_token":"unterminated"#.to_vec()))
                    .expect("build request"),
            )
            .await
            .expect("request should succeed");
        assert_eq!(response.status(), http::StatusCode::BAD_REQUEST);

        let row = tenant_audit_row_for_action(&app.db, ACTION_AUTH_OIDC_LINK).await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::ValidationFailed.as_str()
        );
        let details = row.details_json.expect("audit details");
        assert_eq!(details["reason_code"], serde_json::json!("invalid_json"));
    }

    #[tokio::test]
    async fn oidc_link_invalid_token_writes_validation_failed_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();

        let (status, _body): (http::StatusCode, serde_json::Value) = client
            .post_json(
                "/api/v1/auth/oidc/link",
                &serde_json::json!({ "link_token": "missing-link-token" }),
            )
            .send_json()
            .await;
        assert_eq!(status, http::StatusCode::BAD_REQUEST);

        let row = tenant_audit_row_for_action(&app.db, ACTION_AUTH_OIDC_LINK).await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::ValidationFailed.as_str()
        );
        let details = row.details_json.expect("audit details");
        assert_eq!(
            details["reason_code"],
            serde_json::json!("invalid_or_expired_link_token")
        );
    }

    #[tokio::test]
    async fn oidc_link_denied_password_verification_writes_denied_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();
        let user_id = insert_test_user_with_password(
            &app.db,
            "oidc-link-denied-password@test.local",
            "correct-password",
        )
        .await;
        let provider_id = insert_active_oidc_provider(
            &app.db,
            app.state.default_tenant_id,
            "OIDC Link Session Mint Failure",
            "oidc-link-session-mint-failure",
        )
        .await;
        let link_token = "oidc-link-token-denied";
        app.state
            .oidc
            .account_link_store
            .insert(PendingAccountLinkParams {
                token: link_token.to_string(),
                provider_id,
                oidc_subject: "subject".to_string(),
                email: "oidc-link-denied-password@test.local".to_string(),
                user_id,
                first_name: Some("Oidc".to_string()),
                last_name: Some("Audit".to_string()),
                mapped_roles: Vec::new(),
                existing_link_provider_id: None,
            })
            .await
            .expect("store pending link");

        let (status, _body): (http::StatusCode, serde_json::Value) = client
            .post_json(
                "/api/v1/auth/oidc/link",
                &serde_json::json!({
                    "link_token": link_token,
                    "password": "wrong-password",
                }),
            )
            .send_json()
            .await;
        assert_eq!(status, http::StatusCode::UNAUTHORIZED);

        let row = tenant_audit_row_for_action(&app.db, ACTION_AUTH_OIDC_LINK).await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
        let details = row.details_json.expect("audit details");
        assert_eq!(
            details["reason_code"],
            serde_json::json!("ownership_verification_failed")
        );
    }

    #[tokio::test]
    async fn oidc_link_denied_user_mismatch_writes_denied_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();
        let user_id = insert_test_user_with_password(
            &app.db,
            "oidc-link-denied-mismatch@test.local",
            "correct-password",
        )
        .await;
        let provider_id = uuid::Uuid::now_v7();
        let link_token = "oidc-link-token-mismatch";
        app.state
            .oidc
            .account_link_store
            .insert(PendingAccountLinkParams {
                token: link_token.to_string(),
                provider_id,
                oidc_subject: "subject".to_string(),
                email: "oidc-link-denied-mismatch@test.local".to_string(),
                user_id,
                first_name: Some("Oidc".to_string()),
                last_name: Some("Audit".to_string()),
                mapped_roles: Vec::new(),
                existing_link_provider_id: None,
            })
            .await
            .expect("store pending link");

        let bearer = app
            .state
            .auth
            .jwt
            .create_access_token(uuid::Uuid::now_v7(), &[], "oidc", Some(provider_id))
            .expect("create bearer token");
        let (status, _body): (http::StatusCode, serde_json::Value) = client
            .post_json(
                "/api/v1/auth/oidc/link",
                &serde_json::json!({
                    "link_token": link_token,
                }),
            )
            .bearer(&bearer)
            .send_json()
            .await;
        assert_eq!(status, http::StatusCode::UNAUTHORIZED);

        let row = tenant_audit_row_for_action(&app.db, ACTION_AUTH_OIDC_LINK).await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
        let details = row.details_json.expect("audit details");
        assert_eq!(details["reason_code"], serde_json::json!("user_mismatch"));
    }

    #[tokio::test]
    async fn oidc_link_db_insert_failure_writes_failed_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();
        let user_id = insert_test_user_with_password(
            &app.db,
            "oidc-link-insert-failed@test.local",
            "correct-password",
        )
        .await;
        let provider_id = uuid::Uuid::now_v7();
        let link_token = "oidc-link-token-insert-failed";
        app.state
            .oidc
            .account_link_store
            .insert(PendingAccountLinkParams {
                token: link_token.to_string(),
                provider_id,
                oidc_subject: "subject".to_string(),
                email: "oidc-link-insert-failed@test.local".to_string(),
                user_id,
                first_name: Some("Oidc".to_string()),
                last_name: Some("Audit".to_string()),
                mapped_roles: Vec::new(),
                existing_link_provider_id: None,
            })
            .await
            .expect("store pending link");

        drop_table(&app.db, "user_oidc_links").await;

        let (status, _body): (http::StatusCode, serde_json::Value) = client
            .post_json(
                "/api/v1/auth/oidc/link",
                &serde_json::json!({
                    "link_token": link_token,
                    "password": "correct-password",
                }),
            )
            .send_json()
            .await;
        assert_eq!(status, http::StatusCode::INTERNAL_SERVER_ERROR);

        let row = tenant_audit_row_for_action(&app.db, ACTION_AUTH_OIDC_LINK).await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Failed.as_str()
        );
        let details = row.details_json.expect("audit details");
        assert_eq!(
            details["reason_code"],
            serde_json::json!("oidc_link_insert_failed")
        );
    }

    #[tokio::test]
    async fn oidc_link_session_mint_failure_writes_failed_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();
        let user_id = insert_test_user_with_password(
            &app.db,
            "oidc-link-session-failed@test.local",
            "correct-password",
        )
        .await;
        let provider_id = uuid::Uuid::now_v7();
        let link_token = "oidc-link-token-session-failed";
        app.state
            .oidc
            .account_link_store
            .insert(PendingAccountLinkParams {
                token: link_token.to_string(),
                provider_id,
                oidc_subject: "subject".to_string(),
                email: "oidc-link-session-failed@test.local".to_string(),
                user_id,
                first_name: Some("Oidc".to_string()),
                last_name: Some("Audit".to_string()),
                mapped_roles: Vec::new(),
                existing_link_provider_id: None,
            })
            .await
            .expect("store pending link");

        app.db
            .execute_unprepared(
                "CREATE TRIGGER delete_user_after_oidc_link_insert \
                 AFTER INSERT ON user_oidc_links \
                 BEGIN DELETE FROM users WHERE id = NEW.user_id; END;",
            )
            .await
            .expect("create test trigger");

        let (status, _body): (http::StatusCode, serde_json::Value) = client
            .post_json(
                "/api/v1/auth/oidc/link",
                &serde_json::json!({
                    "link_token": link_token,
                    "password": "correct-password",
                }),
            )
            .send_json()
            .await;
        assert_eq!(status, http::StatusCode::INTERNAL_SERVER_ERROR);

        let row = tenant_audit_row_for_action(&app.db, ACTION_AUTH_OIDC_LINK).await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Failed.as_str()
        );
        let details = row.details_json.expect("audit details");
        assert_eq!(
            details["reason_code"],
            serde_json::json!("mint_auth_response_failed")
        );
    }
}
