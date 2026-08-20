use crate::AppState;
use crate::auth::authentication::{
    OidcUserParams, OidcUserResolution, RoleSyncOutcome, extract_mapped_roles, resolve_oidc_user,
    sync_oidc_roles,
};
use crate::auth::password;
use crate::auth::refresh_cookie::set_refresh_token_cookie;
use crate::auth::session::SessionService;
use crate::auth::token::{generate_secure_token, generate_uuid};
use crate::error_response::error_response;
use crate::extract::{SessionSvc, Unvalidated};
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
use rootcause::prelude::*;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, Set};
use serde::Deserialize;
use std::sync::Arc;
use time::OffsetDateTime;
use uptrakit_shared_db::begin_immediate;
use uptrakit_shared_db::entity::prelude::*;
use uptrakit_shared_db::entity::{oidc_provider, user_oidc_link};
use uptrakit_shared_types::MaskedEmail;
use uptrakit_web_api_queries::queries::users::oidc_sync::{
    build_fake_claims_for_sync, find_active_provider,
};

use crate::api_error::ApiError;
use crate::auth::AuthMethod;
use uptrakit_web_api_types::SecretString;
use uuid::Uuid;

pub use super::auth::AuthResponse;
use crate::auth::registration::{RegistrationMode, RegistrationSettings};
pub use uptrakit_web_api_types::oidc_auth::{
    AuthMethodsResponse, OidcAuthorizeResponse, OidcCompleteRegistrationRequest,
    OidcExchangeRequest, OidcLinkRequest, OidcProviderInfo,
};

#[derive(Deserialize, utoipa::IntoParams)]
pub struct OidcCallbackParams {
    /// Authorization code.
    pub code: Option<String>,
    /// CSRF state.
    pub state: Option<String>,
    /// Error from provider.
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
    /// Copied from the flow snapshot's `return_origin` before `flow` is
    /// partially consumed below — see `create_oidc_exchange_and_redirect` for
    /// the invariant this carries through to the final redirect.
    return_origin: Option<String>,
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
    let mut builder =
        uptrakit_audit_log::AuditEntry::<uptrakit_audit_log::Event>::builder_event(action_type)
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
        state.audit_emitter.emit_event(entry);
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

    let mut builder = uptrakit_audit_log::AuditEntry::<uptrakit_audit_log::Event>::builder_event(
        uptrakit_audit_log::AuditActionType::USER_CREATE,
    )
    .tenant_scope(state.default_tenant_id)
    .actor(uptrakit_audit_log::AuditActorType::Oidc, None)
    .outcome(outcome)
    .details(serde_json::Value::Object(details));

    if let Some(user_id) = user_id {
        builder = builder.target("user", user_id.to_string(), None);
    }

    if let Ok(entry) = builder.build() {
        state.audit_emitter.emit_event(entry);
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
        "oidc_missing_params" | "oidc_invalid_redirect" => {
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

/// Result of pinning the OIDC `redirect_uri` to the deployment's canonical
/// host. `return_origin: None` is the canonical-unset sentinel: it means the
/// observed request origin was never validated against a canonical host, so
/// callback-time redirects built from it must stay relative (see
/// `create_oidc_exchange_and_redirect`).
#[derive(Debug)]
pub(crate) struct PinnedRedirect {
    pub redirect_uri: String,
    pub return_origin: Option<String>,
}

/// Short, non-noisy description of a JSON value's type, for error payloads
/// that must not echo a potentially large/complex raw value verbatim.
fn json_type_name(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

/// Read `oauth.canonical_host` fresh from `global_settings` on every call.
///
/// Deliberately bypasses `state.oauth` (whose disabled-mode placeholder host
/// is `https://disabled.invalid`, see `oauth::canonical_url::disabled_placeholder`)
/// so that redirect pinning works per-request regardless of whether the
/// MCP/OAuth resource-server feature is enabled. Returns `Ok(None)` when the
/// setting is unset or empty — the deployment has not pinned a canonical host
/// yet, and callers must fall back to the observed request origin without
/// validation. Returns `Err(AuthError::InvalidCanonicalHost)` when the stored
/// value is not a JSON string, or is a string that fails `is_bare_host`
/// (contains userinfo, a path, or otherwise is not a bare host) — corrupt
/// configuration never silently falls back, since accepting a malformed
/// canonical host would defeat the pinning it's meant to enforce.
pub(crate) async fn canonical_origin(
    db: &sea_orm::DatabaseConnection,
) -> crate::auth::Result<Option<String>> {
    let Some(raw) =
        crate::settings_store::load_global_setting_raw(db, "oauth.canonical_host").await?
    else {
        return Ok(None);
    };
    let Some(host) = raw.as_str() else {
        return Err(report!(crate::auth::AuthError::InvalidCanonicalHost(
            format!(
                "stored value is not a string (got {})",
                json_type_name(&raw)
            )
        )));
    };
    let trimmed = host.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if !uptrakit_web_api_types::oauth::canonical_url::is_bare_host(trimmed) {
        return Err(report!(crate::auth::AuthError::InvalidCanonicalHost(
            trimmed.to_owned()
        )));
    }
    Ok(Some(format!("https://{}", trimmed.to_ascii_lowercase())))
}

/// Read `oauth.accepted_audience_hosts` (a JSON string array) fresh from
/// `global_settings`. Missing, malformed, or non-array values resolve to an
/// empty list — fail closed, since an alias silently accepted here would let
/// an attacker-observed origin bypass canonical pinning. Entries that fail
/// `is_bare_host` are dropped by the caller rather than here, so a single bad
/// entry doesn't invalidate the whole list.
async fn accepted_audience_hosts(
    db: &sea_orm::DatabaseConnection,
) -> crate::auth::Result<Vec<String>> {
    let Some(raw) =
        crate::settings_store::load_global_setting_raw(db, "oauth.accepted_audience_hosts").await?
    else {
        return Ok(Vec::new());
    };
    Ok(raw
        .as_array()
        .map(|entries| {
            entries
                .iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default())
}

/// Compute the `redirect_uri` to send to the OIDC provider and the
/// `return_origin` to snapshot alongside it, per the canonical-host pinning
/// invariant:
///
/// - Canonical host unset: `redirect_uri` is derived from the request-observed
///   `observed_base_url`; `return_origin: None` (unvalidated, callback-time
///   redirects stay relative).
/// - Canonical host set: `redirect_uri` always pins to the canonical origin.
///   `return_origin` is `Some(observed_base_url)` only when it equals the
///   canonical origin or matches an entry in `oauth.accepted_audience_hosts`
///   (case-insensitive host comparison, no port normalization); otherwise it
///   falls back to `Some(canonical)` — never `None`, since a canonical host is
///   configured and the flow can safely replay to it.
///
/// DB errors propagate via `?` — this never defaults or fails open.
pub(crate) async fn compute_pinned_redirect(
    db: &sea_orm::DatabaseConnection,
    observed_base_url: &str,
) -> crate::auth::Result<PinnedRedirect> {
    let observed = observed_base_url.trim_end_matches('/').to_string();

    let Some(canonical) = canonical_origin(db).await? else {
        return Ok(PinnedRedirect {
            redirect_uri: format!("{observed}/api/v1/auth/oidc/callback"),
            return_origin: None,
        });
    };

    let redirect_uri = format!("{canonical}/api/v1/auth/oidc/callback");
    let observed_lower = observed.to_ascii_lowercase();
    let canonical_lower = canonical.to_ascii_lowercase();

    if observed_lower == canonical_lower {
        return Ok(PinnedRedirect {
            redirect_uri,
            return_origin: Some(observed),
        });
    }

    let accepted = accepted_audience_hosts(db).await?;
    let observed_matches_alias = accepted.iter().any(|host| {
        uptrakit_web_api_types::oauth::canonical_url::is_bare_host(host)
            && format!("https://{}", host.to_ascii_lowercase()) == observed_lower
    });

    let return_origin = if observed_matches_alias {
        Some(observed)
    } else {
        Some(canonical)
    };

    Ok(PinnedRedirect {
        redirect_uri,
        return_origin,
    })
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

    // Pin the redirect_uri to the deployment's canonical host (when configured)
    // and compute the return_origin to snapshot alongside it — see
    // `compute_pinned_redirect` for the full invariant.
    let pinned = match compute_pinned_redirect(state.db(), &base_url).await {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(error = ?e, "Failed to compute pinned OIDC redirect");
            emit_oidc_route_audit(
                &state,
                uptrakit_audit_log::AuditActionType::from_static(
                    uptrakit_audit_log::AuditActionType::AUTH_OIDC_AUTHORIZE,
                ),
                uptrakit_audit_log::AuditOutcome::Failed,
                Some(&provider),
                None,
                serde_json::json!({
                    "reason_code": "canonical_settings_read_failed",
                }),
            );
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };
    let redirect_uri_string = pinned.redirect_uri.clone();
    let redirect_url = match RedirectUrl::new(redirect_uri_string.clone()) {
        Ok(url) => url,
        Err(e) => {
            tracing::error!(error = %e, "Invalid OIDC redirect URL");
            emit_oidc_route_audit(
                &state,
                uptrakit_audit_log::AuditActionType::from_static(
                    uptrakit_audit_log::AuditActionType::AUTH_OIDC_AUTHORIZE,
                ),
                uptrakit_audit_log::AuditOutcome::ValidationFailed,
                Some(&provider),
                None,
                serde_json::json!({
                    "reason_code": "invalid_redirect_url",
                }),
            );
            return error_response(StatusCode::BAD_REQUEST, "Invalid redirect URL");
        }
    };

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
            crate::auth::oidc_state::FlowSnapshot {
                redirect_uri: redirect_uri_string,
                return_origin: pinned.return_origin,
            },
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
    params(OidcCallbackParams),
    responses(
        (status = 302, description = "Redirect to frontend"),
    ),
    tag = "Authentication"
)]
#[tracing::instrument(skip_all)]
pub async fn oidc_callback(
    State(state): State<Arc<AppState>>,
    Query(params): Query<OidcCallbackParams>,
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
        return_origin,
    } = match validate_oidc_state(&state, &csrf_state).await {
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
    let response = resolve_or_create_oidc_user(
        &state,
        provider_id,
        &provider,
        claims,
        return_origin.as_deref(),
    )
    .await;
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

    // A flow snapshot with an empty redirect_uri predates B3 pinning (or was
    // never populated) — treat it the same as an expired/unknown state rather
    // than falling back to a header-derived redirect_uri.
    if flow.redirect_uri.is_empty() {
        tracing::warn!(
            provider_id = %provider_id,
            "OIDC flow snapshot has empty redirect_uri; treating as expired"
        );
        return Err(OidcStateValidationFailure::new(
            Redirect::to("/login?error=oidc_state_expired").into_response(),
            Some(provider_id),
        ));
    }

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

    // Replay the redirect_uri pinned at authorize time — never re-derive it
    // from this request's headers, or the pinning would be meaningless.
    let redirect_url = match RedirectUrl::new(flow.redirect_uri.clone()) {
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

    let return_origin = flow.return_origin.clone();
    Ok(ValidatedOidcCallback {
        flow,
        provider,
        client,
        redirect_url,
        allow_private_network_issuers,
        return_origin,
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
    return_origin: Option<&str>,
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
        return_origin,
    )
    .await
    {
        return response;
    }

    // Resolve user inside a transaction to prevent the race where two concurrent
    // OIDC callbacks both see user_count == 1 and both get the owner role.
    let txn = match begin_immediate(state.db()).await {
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
        return_origin,
    )
    .await
}

/// Execute the OIDC user resolution match inside the transaction.
///
/// Returns the response to send to the client. The caller is responsible for
/// beginning and committing the transaction.
#[expect(
    clippy::too_many_arguments,
    reason = "all parameters are required; decomposing into structs would add complexity without clarity"
)]
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
    return_origin: Option<&str>,
) -> Response {
    match resolution {
        OidcUserResolution::LinkedUser(user_id) => {
            let (user_id, sync_outcome) =
                match handle_linked_user(state, &txn, user_id, provider, additional_claims).await {
                    Ok(result) => result,
                    Err(response) => return response,
                };
            if let Err(e) = txn.commit().await {
                tracing::error!(error = %e, "Failed to commit OIDC callback transaction");
                return Redirect::to("/login?error=oidc_internal_error").into_response();
            }
            handle_role_sync_outcome(state, user_id, provider.name.as_str(), sync_outcome).await;
            create_oidc_exchange_and_redirect(state, user_id, provider_id, return_origin).await
        }
        OidcUserResolution::NewUser(user_id) => {
            let (user_id, first_user_registration, sync_outcome) =
                match handle_new_user(state, &txn, user_id, provider, additional_claims).await {
                    Ok(result) => result,
                    Err(response) => return response,
                };
            if let Err(e) = txn.commit().await {
                tracing::error!(error = %e, "Failed to commit OIDC callback transaction");
                return Redirect::to("/login?error=oidc_internal_error").into_response();
            }
            handle_role_sync_outcome(state, user_id, provider.name.as_str(), sync_outcome).await;
            let is_first_user = first_user_registration.is_some();
            if let Some(reg) = first_user_registration {
                state.settings.set_registration(reg).await;
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
            create_oidc_exchange_and_redirect(state, user_id, provider_id, return_origin).await
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
                return_origin,
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
                return_origin,
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

/// Compute the prefix that makes a relative post-login redirect absolute to
/// the pinned `return_origin` from the flow snapshot. Empty when
/// `return_origin` is `None` (canonical host was unset at authorize time),
/// which keeps the redirect relative rather than substituting a
/// header-derived value. Single shared identity for every prefix site
/// (`check_registration_eligibility`, `link_redirect_with_no_referrer`) --
/// do not reimplement per call site.
fn redirect_origin_prefix(return_origin: Option<&str>) -> &str {
    return_origin.unwrap_or("")
}

/// Pre-check: when registration mode is Invite and auto-create is enabled,
/// verify whether the OIDC subject already has a link or a matching user.
/// If neither exists and a registration token is required, store a pending
/// registration and return a redirect response. Returns `None` when the
/// normal flow should continue.
#[expect(
    clippy::too_many_arguments,
    reason = "all parameters are required; decomposing into structs would add complexity without clarity"
)]
async fn check_registration_eligibility(
    state: &AppState,
    provider_id: Uuid,
    provider: &oidc_provider::Model,
    sub: &str,
    email: &str,
    first_name: Option<&str>,
    last_name: Option<&str>,
    additional_claims: &serde_json::Value,
    return_origin: Option<&str>,
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
            "{}/login#registration_token_required=true&registration_code={code}",
            redirect_origin_prefix(return_origin)
        ))
        .into_response(),
    )
}

/// Post-commit handling of a role-sync outcome. `Applied` -> engine flush +
/// cross-instance publish. `SkippedLockout` -> audit Event naming the
/// provider and attempted set -- the login has already succeeded; the Event
/// is the operator's signal.
///
/// Not a route handler and not an `Executor::run` — the audit-coverage
/// walker's other discovery mechanisms don't see it, so it is marked
/// explicitly for the catalog's `user_role.sync_lockout_prevented` row.
#[uptrakit_audit_log::audit_required]
async fn handle_role_sync_outcome(
    state: &AppState,
    user_id: Uuid,
    provider_name: &str,
    outcome: RoleSyncOutcome,
) {
    match outcome {
        RoleSyncOutcome::Applied => {
            state.access_engine.invalidate_subjects(&[user_id], &[]);
            state
                .notification
                .notification_service
                .publish_controller_event(uptrakit_wire::ControllerMessage::AccessInvalidated(
                    uptrakit_wire::AccessInvalidatedPayload::new(vec![user_id], vec![]),
                ))
                .await;
        }
        RoleSyncOutcome::SkippedLockout {
            attempted_role_names,
        } => {
            // Fully-qualified paths: this file never imports audit types bare
            // (matches every existing emit in oidc_auth.rs).
            if let Ok(entry) =
                uptrakit_audit_log::AuditEntry::<uptrakit_audit_log::Event>::builder_event(
                    uptrakit_audit_log::AuditActionType::USER_ROLE_SYNC_LOCKOUT_PREVENTED,
                )
                .tenant_scope(state.default_tenant_id)
                .actor(uptrakit_audit_log::AuditActorType::Oidc, None)
                .target("user", user_id.to_string(), None)
                .outcome(uptrakit_audit_log::AuditOutcome::Denied)
                .details(serde_json::json!({
                    "provider": provider_name,
                    "attempted_roles": attempted_role_names,
                }))
                .build()
            {
                state.audit_emitter.emit_event(entry);
            }
        }
        RoleSyncOutcome::NoChange => {} // Closed enum (not #[non_exhaustive]) -- exhaustive match, no wildcard.
    }
}

/// Handle the `LinkedUser` resolution: verify the user is still active and
/// sync OIDC roles within the transaction.
///
/// Returns `Ok((user_id, sync_outcome))` so the caller can commit the
/// transaction, invoke the outcome handler, and create the exchange
/// redirect, or `Err(Response)` on failure.
async fn handle_linked_user(
    state: &AppState,
    txn: &sea_orm::DatabaseTransaction,
    user_id: Uuid,
    provider: &oidc_provider::Model,
    additional_claims: &serde_json::Value,
) -> Result<(Uuid, RoleSyncOutcome), Response> {
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
    let sync_outcome = match sync_oidc_roles(
        txn,
        state.default_tenant_id,
        state.default_tenant_id,
        user_id,
        provider,
        additional_claims,
    )
    .await
    {
        Ok(outcome) => outcome,
        Err(e) => {
            // error!, not warn!: a persistent guard/savepoint failure (e.g.
            // sentinel missing) means roles silently never sync on any
            // login — this line is the only operator signal.
            tracing::error!(error = ?e, "OIDC role sync failed (login continues)");
            RoleSyncOutcome::NoChange
        }
    };

    Ok((user_id, sync_outcome))
}

/// Handle the `NewUser` resolution: check if this is the first user (owner
/// setup) and sync OIDC roles within the transaction.
///
/// Returns `Ok((user_id, registration, sync_outcome))` so the caller can
/// commit the transaction, publish the returned registration snapshot
/// post-commit, invoke the outcome handler, and create the exchange
/// redirect; `Err(Response)` on failure (the caller drops the transaction,
/// rolling back).
async fn handle_new_user(
    state: &AppState,
    txn: &sea_orm::DatabaseTransaction,
    user_id: Uuid,
    provider: &oidc_provider::Model,
    additional_claims: &serde_json::Value,
) -> Result<(Uuid, Option<RegistrationSettings>, RoleSyncOutcome), Response> {
    // Atomically check if this is the first user (threshold 1 because the
    // user was just created by resolve_oidc_user) and handle owner role +
    // initial setup inside the same transaction. Clear the default role
    // resolve_oidc_user may have pre-assigned. A setup failure aborts the OIDC
    // registration; the caller drops the transaction (rollback).
    let first_user_registration = match super::auth::handle_first_user_setup(
        txn,
        &state.settings,
        state.default_tenant_id,
        user_id,
        1,
        super::auth::ClearDefaultRoles::Clear,
    )
    .await
    {
        Ok(reg) => reg,
        Err(e) => {
            tracing::error!(error = ?e, "Failed to handle first-user setup for OIDC registration");
            return Err(Redirect::to("/login?error=oidc_internal_error").into_response());
        }
    };
    if first_user_registration.is_some() {
        tracing::info!("first user registered via OIDC, assigned owner role");
    }

    // Sync roles
    let sync_outcome = match sync_oidc_roles(
        txn,
        state.default_tenant_id,
        state.default_tenant_id,
        user_id,
        provider,
        additional_claims,
    )
    .await
    {
        Ok(outcome) => outcome,
        Err(e) => {
            // error!, not warn!: a persistent guard/savepoint failure (e.g.
            // sentinel missing) means roles silently never sync on any
            // login — this line is the only operator signal.
            tracing::error!(error = ?e, "OIDC role sync failed (login continues)");
            RoleSyncOutcome::NoChange
        }
    };

    Ok((user_id, first_user_registration, sync_outcome))
}

/// Handle the `LinkViaPasswordRequired` resolution: store a pending link and
/// redirect to the frontend password-confirmation form.
#[expect(
    clippy::too_many_arguments,
    reason = "all parameters are required; decomposing into structs would add complexity without clarity"
)]
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
    return_origin: Option<&str>,
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

    link_redirect_with_no_referrer(email, &link_token, None, return_origin)
}

/// Handle the `LinkViaOidcRequired` resolution: store a pending link and
/// redirect to the frontend OIDC re-authentication form.
#[expect(
    clippy::too_many_arguments,
    reason = "all parameters are required; decomposing into structs would add complexity without clarity"
)]
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
    return_origin: Option<&str>,
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

    link_redirect_with_no_referrer(
        email,
        &link_token,
        Some(existing_provider_id),
        return_origin,
    )
}

/// Build a redirect response for account-linking flows, suppressing the
/// `Referer` header so the link token is not forwarded to third-party
/// resources loaded by the login page.
fn link_redirect_with_no_referrer(
    email: &str,
    link_token: &str,
    existing_provider_id: Option<Uuid>,
    return_origin: Option<&str>,
) -> Response {
    let mut link_headers = HeaderMap::new();
    link_headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    (
        link_headers,
        Redirect::to(&format!(
            "{}{}",
            redirect_origin_prefix(return_origin),
            build_link_required_redirect(email, link_token, existing_provider_id)
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
    body: Unvalidated<OidcExchangeRequest>,
) -> Response {
    let req = match body.require_valid() {
        Ok(req) => req,
        Err(e) => {
            let response = error_response(StatusCode::BAD_REQUEST, e.to_string());
            emit_oidc_exchange_audit(
                &state,
                uptrakit_audit_log::AuditOutcome::ValidationFailed,
                None,
                response.status(),
                Some("invalid_request"),
            );
            return response;
        }
    };

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
    body: Unvalidated<OidcCompleteRegistrationRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let req = match body.require_valid() {
        Ok(req) => req,
        Err(e) => {
            emit_oidc_user_create_audit(
                &state,
                None,
                None,
                None,
                uptrakit_audit_log::AuditOutcome::ValidationFailed,
                Some("invalid_request"),
                None,
            );
            return Ok(error_response(StatusCode::BAD_REQUEST, e.to_string()));
        }
    };

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
    let txn = match begin_immediate(state.db()).await {
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
    //    A failure aborts the whole registration (`?` → 500); the dropped txn rolls back.
    //    Note: the one-time `registration_code` is consumed on the pooled connection BEFORE
    //    this transaction — a `?`-500 rolls back user/roles/settings but does NOT restore the
    //    code; re-initiating the OIDC registration flow is required. Pre-existing burn
    //    behavior; the delta is that the old swallow committed a demoted user on failure.
    let first_user_registration = super::auth::handle_first_user_setup(
        &txn,
        &state.settings,
        state.default_tenant_id,
        user_id,
        1,
        super::auth::ClearDefaultRoles::Keep,
    )
    .await?;
    let is_first_user = first_user_registration.is_some();

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
    let mut role_sync: Option<(String, RoleSyncOutcome)> = None;
    if !pending.mapped_roles.is_empty()
        && let Some(provider) =
            find_active_provider(&txn, state.default_tenant_id, pending.provider_id).await
    {
        let fake_claims = build_fake_claims_for_sync(&provider, &pending.mapped_roles);
        let sync_outcome = match sync_oidc_roles(
            &txn,
            state.default_tenant_id,
            state.default_tenant_id,
            user_id,
            &provider,
            &fake_claims,
        )
        .await
        {
            Ok(outcome) => outcome,
            Err(e) => {
                // error!, not warn!: a persistent guard/savepoint failure
                // (e.g. sentinel missing) means roles silently never sync on
                // any login — this line is the only operator signal.
                tracing::error!(error = ?e, "OIDC role sync failed (registration continues)");
                RoleSyncOutcome::NoChange
            }
        };
        role_sync = Some((provider.name.clone(), sync_outcome));
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
    if let Some(reg) = first_user_registration {
        state.settings.set_registration(reg).await;
    }
    if let Some((provider_name, sync_outcome)) = role_sync {
        handle_role_sync_outcome(&state, user_id, provider_name.as_str(), sync_outcome).await;
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

    // Sync roles if we have mapped roles. The sync itself gets its own
    // Immediate transaction here (the guard requires a real transaction by
    // type) — `find_active_provider` and the `link.insert` above stay on the
    // pooled connection; only the sync write is wrapped. This is a net
    // improvement over the old pooled-autocommit delete_many + N-insert
    // loop, where a crash mid-loop left the user with zero roles: do not
    // "simplify" this back to the pooled connection.
    if !pending.mapped_roles.is_empty()
        && let Some(provider) =
            find_active_provider(state.db(), state.default_tenant_id, pending.provider_id).await
    {
        let fake_claims = build_fake_claims_for_sync(&provider, &pending.mapped_roles);
        match uptrakit_shared_db::access_grants::begin_guarded(state.db()).await {
            Ok(txn) => {
                let sync_outcome = match sync_oidc_roles(
                    &txn,
                    state.default_tenant_id,
                    state.default_tenant_id,
                    pending.user_id,
                    &provider,
                    &fake_claims,
                )
                .await
                {
                    Ok(outcome) => outcome,
                    Err(e) => {
                        tracing::error!(error = ?e, "OIDC role sync failed (link continues)");
                        RoleSyncOutcome::NoChange
                    }
                };
                // The outcome handler runs whatever the commit did. A commit
                // failure only invalidates the WRITE, so `Applied` degrades to
                // `NoChange` (nothing landed -- do not invalidate or publish
                // for it). A lockout denial performed no write at all: its
                // Event is the operator's only signal that an IdP group change
                // tried to strip the last holder, so it must survive.
                let sync_outcome = match txn.commit().await {
                    Ok(()) => sync_outcome,
                    Err(e) => {
                        tracing::error!(error = %e, "failed to commit OIDC role-sync transaction during link");
                        match sync_outcome {
                            RoleSyncOutcome::Applied => RoleSyncOutcome::NoChange,
                            outcome @ (RoleSyncOutcome::SkippedLockout { .. }
                            | RoleSyncOutcome::NoChange) => outcome,
                        }
                    }
                };
                handle_role_sync_outcome(
                    &state,
                    pending.user_id,
                    provider.name.as_str(),
                    sync_outcome,
                )
                .await;
            }
            Err(e) => {
                tracing::error!(error = ?e, "failed to open guarded transaction for OIDC role sync during link");
            }
        }
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
    // Load the user and enforce is_active BEFORE minting any token, mirroring the
    // refresh path (auth.rs) and me() (auth.rs). A deactivated user must never
    // receive OIDC-minted tokens.
    let user = match User::find_by_id(user_id).one(state.db()).await {
        Ok(Some(u)) => u,
        Ok(None) => {
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
        Err(e) => {
            tracing::error!(error = ?e, "Failed to load user during OIDC mint");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };
    if !user.is_active {
        return error_response(StatusCode::FORBIDDEN, "User is deactivated");
    }

    // Only now mint the refresh token — all validation has passed.
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

    let access_token =
        match state
            .auth
            .jwt
            .create_access_token(user_id, "oidc", Some(provider_id), None)
        {
            Ok(t) => t,
            Err(e) => {
                tracing::error!(error = ?e, "Failed to create OIDC access token");
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        };

    let (actions, authority) =
        super::auth::effective_actions(&state.access_engine, state.default_tenant_id, user.id)
            .await;

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
            actions,
            authority,
            has_pending_email_change: false,
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
    return_origin: Option<&str>,
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

    // `return_origin` was validated against the canonical host / accepted
    // audience hosts at authorize time and replayed verbatim from the flow
    // snapshot -- never substitute a header-derived value here, or the
    // absolute redirect becomes attacker-steerable again.
    match return_origin {
        Some(origin) => {
            Redirect::to(&format!("{origin}/login?oidc_code={exchange_code}")).into_response()
        }
        None => Redirect::to(&format!("/login?oidc_code={exchange_code}")).into_response(),
    }
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

pub(crate) fn base_url_from_headers(headers: &HeaderMap) -> Option<String> {
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
mod pinned_redirect_tests {
    use super::{PinnedRedirect, compute_pinned_redirect};
    use crate::auth::AuthError;
    use crate::test_harness::setup_migrated_db;
    use uptrakit_shared_db::raw_settings::upsert_global_setting_raw;

    #[tokio::test]
    async fn canonical_unset_uses_observed_base_url() {
        let db = setup_migrated_db().await;

        let PinnedRedirect {
            redirect_uri,
            return_origin,
        } = compute_pinned_redirect(&db, "https://observed.example.com")
            .await
            .expect("compute_pinned_redirect should succeed");

        assert_eq!(
            redirect_uri,
            "https://observed.example.com/api/v1/auth/oidc/callback"
        );
        assert_eq!(return_origin, None);
    }

    #[tokio::test]
    async fn canonical_set_observed_matches_canonical() {
        let db = setup_migrated_db().await;
        upsert_global_setting_raw(
            &db,
            "oauth.canonical_host",
            serde_json::json!("auth.example.com"),
        )
        .await
        .expect("insert canonical_host");

        let PinnedRedirect {
            redirect_uri,
            return_origin,
        } = compute_pinned_redirect(&db, "https://auth.example.com")
            .await
            .expect("compute_pinned_redirect should succeed");

        assert_eq!(
            redirect_uri,
            "https://auth.example.com/api/v1/auth/oidc/callback"
        );
        assert_eq!(return_origin, Some("https://auth.example.com".to_string()));
    }

    #[tokio::test]
    async fn canonical_set_observed_in_accepted_list() {
        let db = setup_migrated_db().await;
        upsert_global_setting_raw(
            &db,
            "oauth.canonical_host",
            serde_json::json!("auth.example.com"),
        )
        .await
        .expect("insert canonical_host");
        upsert_global_setting_raw(
            &db,
            "oauth.accepted_audience_hosts",
            serde_json::json!(["alias.example.com"]),
        )
        .await
        .expect("insert accepted_audience_hosts");

        let PinnedRedirect {
            redirect_uri,
            return_origin,
        } = compute_pinned_redirect(&db, "https://alias.example.com")
            .await
            .expect("compute_pinned_redirect should succeed");

        assert_eq!(
            redirect_uri, "https://auth.example.com/api/v1/auth/oidc/callback",
            "redirect_uri always pins to canonical, regardless of accepted-list match"
        );
        assert_eq!(return_origin, Some("https://alias.example.com".to_string()));
    }

    #[tokio::test]
    async fn canonical_set_observed_not_listed_falls_back_to_canonical() {
        let db = setup_migrated_db().await;
        upsert_global_setting_raw(
            &db,
            "oauth.canonical_host",
            serde_json::json!("auth.example.com"),
        )
        .await
        .expect("insert canonical_host");
        upsert_global_setting_raw(
            &db,
            "oauth.accepted_audience_hosts",
            serde_json::json!(["alias.example.com"]),
        )
        .await
        .expect("insert accepted_audience_hosts");

        let PinnedRedirect {
            redirect_uri,
            return_origin,
        } = compute_pinned_redirect(&db, "https://evil.example.com")
            .await
            .expect("compute_pinned_redirect should succeed");

        assert_eq!(
            redirect_uri,
            "https://auth.example.com/api/v1/auth/oidc/callback"
        );
        assert_eq!(return_origin, Some("https://auth.example.com".to_string()));
    }

    /// Accepted-list entries that fail `is_bare_host` (e.g. carry a path or
    /// userinfo) are skipped rather than causing the whole list to error —
    /// fail closed to canonical for that entry, not fail open.
    #[tokio::test]
    async fn accepted_list_skips_invalid_entries_fail_closed() {
        let db = setup_migrated_db().await;
        upsert_global_setting_raw(
            &db,
            "oauth.canonical_host",
            serde_json::json!("auth.example.com"),
        )
        .await
        .expect("insert canonical_host");
        upsert_global_setting_raw(
            &db,
            "oauth.accepted_audience_hosts",
            serde_json::json!(["evil.example.com/x@y", "alias.example.com"]),
        )
        .await
        .expect("insert accepted_audience_hosts");

        let PinnedRedirect { return_origin, .. } =
            compute_pinned_redirect(&db, "https://evil.example.com")
                .await
                .expect("compute_pinned_redirect should succeed");
        assert_eq!(
            return_origin,
            Some("https://auth.example.com".to_string()),
            "malformed accepted-list entry must not match; falls back to canonical"
        );

        let PinnedRedirect { return_origin, .. } =
            compute_pinned_redirect(&db, "https://alias.example.com")
                .await
                .expect("compute_pinned_redirect should succeed");
        assert_eq!(
            return_origin,
            Some("https://alias.example.com".to_string()),
            "well-formed accepted-list entry alongside a malformed one still matches"
        );
    }

    #[tokio::test]
    async fn canonical_host_case_normalized_to_lowercase_origin() {
        let db = setup_migrated_db().await;
        upsert_global_setting_raw(
            &db,
            "oauth.canonical_host",
            serde_json::json!("Auth.Example.COM"),
        )
        .await
        .expect("insert canonical_host");

        let PinnedRedirect { redirect_uri, .. } =
            compute_pinned_redirect(&db, "https://Auth.Example.COM")
                .await
                .expect("compute_pinned_redirect should succeed");

        assert_eq!(
            redirect_uri,
            "https://auth.example.com/api/v1/auth/oidc/callback"
        );
    }

    #[tokio::test]
    async fn canonical_host_with_userinfo_is_rejected() {
        let db = setup_migrated_db().await;
        upsert_global_setting_raw(&db, "oauth.canonical_host", serde_json::json!("x@evil.com"))
            .await
            .expect("insert canonical_host");

        let err = compute_pinned_redirect(&db, "https://observed.example.com")
            .await
            .expect_err("malformed canonical_host must fail closed, not silently default");

        assert!(matches!(
            err.current_context(),
            AuthError::InvalidCanonicalHost(host) if host == "x@evil.com"
        ));
    }

    /// A non-string stored value (e.g. a JSON number) is the same class of
    /// corrupt-configuration event as a malformed string and must fail
    /// closed identically -- silently disabling pinning here would be the
    /// opposite (fail-open) posture from the malformed-string case above.
    #[tokio::test]
    async fn canonical_host_non_string_value_is_rejected() {
        let db = setup_migrated_db().await;
        upsert_global_setting_raw(&db, "oauth.canonical_host", serde_json::json!(12345))
            .await
            .expect("insert canonical_host");

        let err = compute_pinned_redirect(&db, "https://observed.example.com")
            .await
            .expect_err("non-string canonical_host must fail closed, not silently disable pinning");

        assert!(matches!(
            err.current_context(),
            AuthError::InvalidCanonicalHost(detail) if detail.contains("number")
        ));
    }
}

/// Direct-call tests for `create_oidc_exchange_and_redirect`'s final
/// redirect construction -- the last link in the pin-at-authorize,
/// replay-at-callback chain. `return_origin` here is always a value already
/// validated (or explicitly absent) upstream; these tests pin the two
/// outcomes so a future refactor cannot silently swap in a header-derived
/// origin.
#[cfg(all(test, feature = "db-sqlite", feature = "oidc"))]
mod exchange_redirect_tests {
    #![expect(
        clippy::expect_used,
        reason = "test code: panics on failure are acceptable"
    )]

    use super::create_oidc_exchange_and_redirect;
    use crate::test_harness::TestApp;
    use axum::response::Response;
    use http::{StatusCode, header};

    fn location(resp: &Response) -> String {
        resp.headers()
            .get(header::LOCATION)
            .expect("expected Location header")
            .to_str()
            .expect("Location header is not valid UTF-8")
            .to_string()
    }

    /// Critical regression pin: when `return_origin` is `None` (canonical
    /// host was unset at authorize time, the flow-snapshot sentinel for
    /// "never validated"), the exchange redirect MUST stay relative.
    /// Substituting a header-derived origin here would reopen the exact
    /// Host-header-injection vector this task closes.
    #[tokio::test]
    async fn exchange_redirect_is_relative_when_return_origin_none() {
        let app = TestApp::new().await;
        let user_id = uuid::Uuid::now_v7();
        let provider_id = uuid::Uuid::now_v7();

        let resp = create_oidc_exchange_and_redirect(&app.state, user_id, provider_id, None).await;

        assert_eq!(resp.status(), StatusCode::SEE_OTHER);
        let loc = location(&resp);
        assert!(
            loc.starts_with("/login?oidc_code="),
            "expected relative redirect, got: {loc}"
        );
    }

    #[tokio::test]
    async fn exchange_redirect_is_absolute_to_return_origin() {
        let app = TestApp::new().await;
        let user_id = uuid::Uuid::now_v7();
        let provider_id = uuid::Uuid::now_v7();

        let resp = create_oidc_exchange_and_redirect(
            &app.state,
            user_id,
            provider_id,
            Some("https://alias.example.com"),
        )
        .await;

        assert_eq!(resp.status(), StatusCode::SEE_OTHER);
        let loc = location(&resp);
        assert!(
            loc.starts_with("https://alias.example.com/login?oidc_code="),
            "expected absolute redirect to return_origin, got: {loc}"
        );
    }
}

#[cfg(all(test, feature = "db-sqlite", feature = "oidc"))]
mod audit_tests {
    #![expect(
        clippy::expect_used,
        reason = "test code: panics on failure are acceptable"
    )]
    #![expect(clippy::panic, reason = "test code: panics on failure are acceptable")]

    use crate::auth::oidc_state::PendingAccountLinkParams;
    use crate::auth::oidc_state::PendingOidcRegistrationParams;
    use crate::auth::password;
    use crate::test_harness::TestApp;
    use axum::body::Body;
    use axum::http::Request;
    use http::header;
    use openidconnect::{Nonce, PkceCodeChallenge};
    use sea_orm::{
        ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait, QueryFilter,
        QueryOrder, Set,
    };
    use time::OffsetDateTime;
    use tower::ServiceExt;
    use uptrakit_shared_db::entity::prelude::{PendingOidcFlow, Role, User, UserRole};
    use uptrakit_shared_db::entity::{
        audit_log, oidc_provider, pending_oidc_flow, role, session, user, user_role,
    };
    use uptrakit_shared_types::MaskedEmail;

    const ACTION_AUTH_OIDC_EXCHANGE: uptrakit_audit_log::RegisteredAuditAction =
        uptrakit_audit_log::AuditActionType::AUTH_OIDC_EXCHANGE;
    const ACTION_AUTH_OIDC_LINK: uptrakit_audit_log::RegisteredAuditAction =
        uptrakit_audit_log::AuditActionType::AUTH_OIDC_LINK;

    async fn tenant_audit_row_for_action(
        db: &sea_orm::DatabaseConnection,
        action_type: uptrakit_audit_log::RegisteredAuditAction,
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

    async fn insert_active_oidc_provider(
        db: &sea_orm::DatabaseConnection,
        tenant_id: uuid::Uuid,
        name: &str,
        slug: &str,
    ) -> uuid::Uuid {
        insert_active_oidc_provider_with_issuer(
            db,
            tenant_id,
            name,
            slug,
            "https://issuer.example.test",
            false,
        )
        .await
    }

    /// Sibling of [`insert_active_oidc_provider`] for tests that drive a real
    /// discovery round-trip against a `httpmock` `MockServer`: lets the
    /// caller point `issuer_url` at the mock server's base URL and enable
    /// `allow_private_network_issuers` (required since the mock listens on
    /// 127.0.0.1, which the default SSRF-safe resolver blocks — see
    /// `crate::oidc_http_client::OidcHttpClient::new`).
    async fn insert_active_oidc_provider_with_issuer(
        db: &sea_orm::DatabaseConnection,
        tenant_id: uuid::Uuid,
        name: &str,
        slug: &str,
        issuer_url: &str,
        allow_private_network_issuers: bool,
    ) -> uuid::Uuid {
        let provider_id = uuid::Uuid::now_v7();
        let now = OffsetDateTime::now_utc();
        oidc_provider::ActiveModel {
            id: Set(provider_id),
            tenant_id: Set(tenant_id),
            name: Set(name.to_string()),
            slug: Set(slug.to_string()),
            logo_url: Set(None),
            issuer_url: Set(issuer_url.to_string()),
            client_id: Set("client-id".to_string()),
            client_secret: Set(uptrakit_crypto::EncryptedString::new(
                "client-secret".to_string(),
                "uptrakit:oidc_providers:client_secret",
            )
            .expect("encrypt client secret")),
            scopes: Set("openid email profile".to_string()),
            auto_create_users: Set(true),
            allow_private_network_issuers: Set(allow_private_network_issuers),
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

    /// Mount a minimal OIDC discovery document (plus an empty JWKS) on
    /// `server`, matching what `build_oidc_client` (via
    /// `CoreProviderMetadata::discover_async`) requires: `issuer` must equal
    /// the mock server's own base URL exactly, and the JWKS referenced by
    /// `jwks_uri` is fetched as a second round-trip.
    fn mount_oidc_discovery(server: &httpmock::MockServer) {
        let issuer = server.base_url();
        server.mock(|when, then| {
            when.method(httpmock::Method::GET)
                .path("/.well-known/openid-configuration");
            then.status(200)
                .header("content-type", "application/json")
                .json_body(serde_json::json!({
                    "issuer": issuer,
                    "authorization_endpoint": format!("{issuer}/authorize"),
                    "token_endpoint": format!("{issuer}/token"),
                    "jwks_uri": format!("{issuer}/jwks"),
                    "response_types_supported": ["code"],
                    "subject_types_supported": ["public"],
                    "id_token_signing_alg_values_supported": ["RS256"],
                }));
        });
        server.mock(|when, then| {
            when.method(httpmock::Method::GET).path("/jwks");
            then.status(200)
                .header("content-type", "application/json")
                .json_body(serde_json::json!({ "keys": [] }));
        });
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

    /// End-to-end (HTTP-level) coverage for the `compute_pinned_redirect`
    /// error arm added to `oidc_authorize`: a malformed
    /// `oauth.canonical_host` value must fail closed (500 +
    /// `canonical_settings_read_failed` audit event), not silently fall back
    /// to the observed request origin or an unpinned redirect_uri.
    #[tokio::test]
    async fn oidc_authorize_invalid_canonical_host_writes_failed_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();
        let provider_id = insert_active_oidc_provider(
            &app.db,
            app.state.default_tenant_id,
            "OIDC Invalid Canonical Host",
            "oidc-invalid-canonical-host",
        )
        .await;

        uptrakit_shared_db::raw_settings::upsert_global_setting_raw(
            &app.db,
            "oauth.canonical_host",
            serde_json::json!("x@evil.example.test"),
        )
        .await
        .expect("insert malformed canonical_host");

        let response = client
            .get(&format!("/api/v1/auth/oidc/{provider_id}/authorize"))
            .header("Host", "localhost")
            .send()
            .await;
        assert_eq!(response.status(), http::StatusCode::INTERNAL_SERVER_ERROR);

        let row = tenant_audit_row_for_action(
            &app.db,
            uptrakit_audit_log::AuditActionType::AUTH_OIDC_AUTHORIZE,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Failed.as_str()
        );
        let provider_id_str = provider_id.to_string();
        assert_eq!(row.target_id.as_deref(), Some(provider_id_str.as_str()));
        let details = row.details_json.expect("audit details");
        assert_eq!(
            details["reason_code"],
            serde_json::json!("canonical_settings_read_failed")
        );
    }

    /// The security-critical join at the end of `oidc_authorize`: the
    /// `redirect_uri` and `return_origin` handed to `oidc_flow_store.insert`
    /// must be exactly what `compute_pinned_redirect` computed, not the raw
    /// observed base URL. Drives a real authorize request against a mocked
    /// OIDC discovery document and reads the persisted `pending_oidc_flows`
    /// row back to verify it.
    #[tokio::test]
    async fn authorize_pins_redirect_and_snapshots_return_origin() {
        let app = TestApp::new().await;
        let client = app.client();

        let server = httpmock::MockServer::start_async().await;
        mount_oidc_discovery(&server);

        let provider_id = insert_active_oidc_provider_with_issuer(
            &app.db,
            app.state.default_tenant_id,
            "OIDC Redirect Pinning",
            "oidc-redirect-pinning",
            &server.base_url(),
            true,
        )
        .await;

        // Canonical host equals the request's own observed host, so
        // `return_origin` should be `Some("https://localhost")`.
        uptrakit_shared_db::raw_settings::upsert_global_setting_raw(
            &app.db,
            "oauth.canonical_host",
            serde_json::json!("localhost"),
        )
        .await
        .expect("insert canonical_host");

        let response = client
            .get(&format!("/api/v1/auth/oidc/{provider_id}/authorize"))
            .header("Host", "localhost")
            .send()
            .await;
        assert_eq!(
            response.status(),
            http::StatusCode::OK,
            "authorize should succeed against the mocked discovery document"
        );

        let flow = PendingOidcFlow::find()
            .filter(pending_oidc_flow::Column::ProviderId.eq(provider_id))
            .one(&app.db)
            .await
            .expect("query pending_oidc_flows")
            .expect("expected a pending_oidc_flows row for this provider");

        assert_eq!(
            flow.redirect_uri, "https://localhost/api/v1/auth/oidc/callback",
            "redirect_uri must be the canonical-host-pinned callback URL"
        );
        assert_eq!(
            flow.return_origin, "https://localhost",
            "return_origin must be the canonical origin (equal to the observed origin here)"
        );
    }

    /// Companion to `authorize_pins_redirect_and_snapshots_return_origin`:
    /// when the observed request origin is neither the canonical origin nor
    /// listed in `oauth.accepted_audience_hosts`, the persisted
    /// `return_origin` must fall back to the CANONICAL origin, never the
    /// observed one -- that fallback is the actual security control this
    /// task exists to cover.
    #[tokio::test]
    async fn authorize_unlisted_origin_falls_back_to_canonical() {
        let app = TestApp::new().await;
        let client = app.client();

        let server = httpmock::MockServer::start_async().await;
        mount_oidc_discovery(&server);

        let provider_id = insert_active_oidc_provider_with_issuer(
            &app.db,
            app.state.default_tenant_id,
            "OIDC Redirect Pinning Unlisted",
            "oidc-redirect-pinning-unlisted",
            &server.base_url(),
            true,
        )
        .await;

        // Canonical host differs from the observed request host ("localhost"
        // below), and "localhost" is not in the accepted-audience list.
        uptrakit_shared_db::raw_settings::upsert_global_setting_raw(
            &app.db,
            "oauth.canonical_host",
            serde_json::json!("canonical.example.test"),
        )
        .await
        .expect("insert canonical_host");

        let response = client
            .get(&format!("/api/v1/auth/oidc/{provider_id}/authorize"))
            .header("Host", "localhost")
            .send()
            .await;
        assert_eq!(
            response.status(),
            http::StatusCode::OK,
            "authorize should succeed against the mocked discovery document"
        );

        let flow = PendingOidcFlow::find()
            .filter(pending_oidc_flow::Column::ProviderId.eq(provider_id))
            .one(&app.db)
            .await
            .expect("query pending_oidc_flows")
            .expect("expected a pending_oidc_flows row for this provider");

        assert_eq!(
            flow.redirect_uri, "https://canonical.example.test/api/v1/auth/oidc/callback",
            "redirect_uri always pins to canonical"
        );
        assert_eq!(
            flow.return_origin, "https://canonical.example.test",
            "return_origin must fall back to the canonical origin, not the observed one"
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

    /// Stage 1 no longer re-derives `redirect_uri` from request headers (B3
    /// pinning: it replays the flow snapshot instead), so a missing Host
    /// header can no longer be a stage-1 failure mode. The next reachable
    /// stage-1 failure is OIDC discovery against the seeded provider's
    /// unreachable `issuer_url` — this test retargets to that failure while
    /// keeping the original assertion this test exists for: provider target
    /// identity survives stage-1 failures in the audit event.
    #[tokio::test]
    async fn oidc_callback_stage1_discovery_failure_keeps_provider_target_in_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();
        let provider_id = insert_active_oidc_provider(
            &app.db,
            app.state.default_tenant_id,
            "OIDC Exchange Mint Failure",
            "oidc-exchange-mint-failure",
        )
        .await;
        let csrf_state = "pending-stage1-state";
        let (_pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();
        let nonce = Nonce::new_random();

        app.state
            .oidc
            .oidc_flow_store
            .insert(
                csrf_state.to_string(),
                provider_id,
                &pkce_verifier,
                &nonce,
                crate::auth::oidc_state::FlowSnapshot {
                    redirect_uri: "https://test.example.com/api/v1/auth/oidc/callback".into(),
                    return_origin: Some("https://test.example.com".into()),
                },
            )
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
            uptrakit_audit_log::AuditOutcome::Failed.as_str()
        );
        assert_eq!(row.target_type.as_deref(), Some("oidc_provider"));
        let provider_id_str = provider_id.to_string();
        assert_eq!(row.target_id.as_deref(), Some(provider_id_str.as_str()));
        let details = row.details_json.expect("audit details");
        assert_eq!(
            details["reason_code"],
            serde_json::json!("oidc_discovery_failed")
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
    async fn oidc_exchange_rejects_deactivated_user() {
        let app = TestApp::new().await;
        let client = app.client();
        let email = "oidc-deactivated@test.local";
        let (register_status, _register_body): (http::StatusCode, serde_json::Value) = client
            .post_json(
                "/api/v1/auth/register",
                &serde_json::json!({
                    "email": email,
                    "password": "password123",
                    "first_name": "Oidc",
                    "last_name": "Deact",
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

        // Deactivate the user.
        let mut active: user::ActiveModel = User::find_by_id(user_id)
            .one(&app.db)
            .await
            .expect("query user")
            .expect("user exists")
            .into();
        active.is_active = Set(false);
        active.deactivated_at = Set(Some(OffsetDateTime::now_utc()));
        active.update(&app.db).await.expect("deactivate user");

        let provider_id = insert_active_oidc_provider(
            &app.db,
            app.state.default_tenant_id,
            "OIDC Deactivated",
            "oidc-deactivated",
        )
        .await;
        let exchange_code = "oidc-deactivated-code";
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
        assert_eq!(status, http::StatusCode::FORBIDDEN);

        // No OIDC session row was persisted for the deactivated user (the
        // password-auth session created by registration is expected to exist).
        let oidc_sessions = session::Entity::find()
            .filter(session::Column::AuthMethod.eq("oidc"))
            .all(&app.db)
            .await
            .expect("query sessions");
        assert!(
            oidc_sessions.is_empty(),
            "no OIDC session must be minted for a deactivated user"
        );
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
    async fn oidc_exchange_empty_code_writes_validation_failed_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();

        let (status, _body): (http::StatusCode, serde_json::Value) = client
            .post_json(
                "/api/v1/auth/oidc/exchange",
                &serde_json::json!({ "code": "" }),
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
        assert_eq!(details["reason_code"], serde_json::json!("invalid_request"));
    }

    #[tokio::test]
    async fn oidc_exchange_load_failure_writes_failed_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();

        crate::test_harness::fixtures::drop_table(&app.db, "pending_oidc_token_exchanges").await;

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

        crate::test_harness::fixtures::drop_table(&app.db, "sessions").await;

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
            .create_access_token(uuid::Uuid::now_v7(), "oidc", Some(provider_id), None)
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

        crate::test_harness::fixtures::drop_table(&app.db, "user_oidc_links").await;

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

        #[expect(
            clippy::disallowed_methods,
            reason = "test-only schema sabotage: CREATE TRIGGER injects a forced DB-level side effect to exercise the error path"
        )]
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

    async fn break_owner_role_assignment(db: &sea_orm::DatabaseConnection) {
        use sea_orm::sea_query::Expr;
        Role::update_many()
            .col_expr(
                role::Column::Name,
                Expr::value("system_administrator_renamed"),
            )
            .filter(role::Column::Name.eq("system_administrator"))
            .exec(db)
            .await
            .expect("rename role");
    }

    async fn role_names_for_user(
        db: &sea_orm::DatabaseConnection,
        tenant_id: uuid::Uuid,
        user_id: uuid::Uuid,
    ) -> Vec<String> {
        let role_ids: Vec<uuid::Uuid> = UserRole::find()
            .filter(user_role::Column::TenantId.eq(tenant_id))
            .filter(user_role::Column::UserId.eq(user_id))
            .all(db)
            .await
            .expect("user_role rows")
            .into_iter()
            .map(|r| r.role_id)
            .collect();
        let mut names: Vec<String> = Role::find()
            .filter(role::Column::Id.is_in(role_ids))
            .all(db)
            .await
            .expect("role rows")
            .into_iter()
            .map(|r| r.name)
            .collect();
        names.sort();
        names
    }

    async fn insert_txn_user(txn: &sea_orm::DatabaseTransaction, email: &str) -> uuid::Uuid {
        let user_id = uuid::Uuid::now_v7();
        let now = OffsetDateTime::now_utc();
        user::ActiveModel {
            id: Set(user_id),
            email: Set(MaskedEmail::new(email.to_string())),
            first_name: Set("Oidc".to_string()),
            last_name: Set("First".to_string()),
            password_hash: Set(None),
            is_active: Set(true),
            deactivated_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(txn)
        .await
        .expect("insert user in txn");
        user_id
    }

    /// Spec test 2 (callback path): owner-role failure inside handle_new_user →
    /// Err(redirect), transaction dropped → nothing committed, snapshot untouched.
    /// RED pre-fix: the hand-rolled copy logged the failure and returned Ok, so
    /// the caller committed a first user with zero roles + closed registration.
    #[tokio::test]
    async fn oidc_callback_new_user_rolls_back_on_owner_role_failure() {
        let app = TestApp::new().await;
        let provider_id = insert_active_oidc_provider(
            &app.db,
            app.state.default_tenant_id,
            "First User Rollback",
            "first-user-rollback",
        )
        .await;
        let provider = oidc_provider::Entity::find_by_id(provider_id)
            .one(&app.db)
            .await
            .expect("provider query")
            .expect("provider row");
        let mode_before = app.state.settings.registration().mode;

        break_owner_role_assignment(&app.db).await;

        let txn = uptrakit_shared_db::access_grants::begin_guarded(&app.db)
            .await
            .expect("begin (guarded)");
        let user_id = insert_txn_user(&txn, "oidc-rollback@test.local").await;
        if super::handle_new_user(&app.state, &txn, user_id, &provider, &serde_json::json!({}))
            .await
            .is_ok()
        {
            panic!("handle_new_user must fail when owner-role assignment fails");
        }
        drop(txn); // what execute_oidc_resolution does on Err: rollback

        assert_eq!(
            User::find().count(&app.db).await.expect("count"),
            0,
            "no user row may survive the rollback"
        );
        assert_eq!(app.state.settings.registration().mode, mode_before);
    }

    /// Spec test 5 (+ helper-publish split): first OIDC user's pre-assigned
    /// default role is cleared, exactly the owner set remains, and the snapshot
    /// is NOT published by handle_new_user (the NewUser commit owner does that).
    #[tokio::test]
    async fn oidc_callback_first_user_clears_preassigned_default_role() {
        use crate::auth::registration::RegistrationMode;

        let app = TestApp::new().await;
        let provider_id = insert_active_oidc_provider(
            &app.db,
            app.state.default_tenant_id,
            "First User Clear",
            "first-user-clear",
        )
        .await;
        let provider = oidc_provider::Entity::find_by_id(provider_id)
            .one(&app.db)
            .await
            .expect("provider query")
            .expect("provider row");

        let txn = uptrakit_shared_db::access_grants::begin_guarded(&app.db)
            .await
            .expect("begin (guarded)");
        let user_id = insert_txn_user(&txn, "oidc-clear@test.local").await;
        // Stand-in for resolve_oidc_user's best-effort default role (the legacy
        // "user" role no longer exists; viewer overlaps the owner set, proving
        // the Clear step ran — Keep would PK-conflict here).
        super::super::auth::assign_viewer_role(&txn, app.state.default_tenant_id, user_id)
            .await
            .expect("pre-assign default role");
        let (returned_id, reg, sync_outcome) = match super::handle_new_user(
            &app.state,
            &txn,
            user_id,
            &provider,
            &serde_json::json!({}),
        )
        .await
        {
            Ok(v) => v,
            Err(_) => panic!("handle_new_user must succeed"),
        };
        txn.commit().await.expect("commit");

        assert_eq!(returned_id, user_id);
        assert!(
            reg.is_some(),
            "first user must yield a registration snapshot"
        );
        assert_eq!(
            sync_outcome,
            super::RoleSyncOutcome::NoChange,
            "provider has no role_claim_path configured — sync must no-op"
        );
        let mut expected: Vec<String> = [
            "viewer",
            "operator",
            "service_manager",
            "software_manager",
            "host_manager",
            "settings_manager",
            "command_manager",
            "system_administrator",
        ]
        .iter()
        .map(|s| (*s).to_string())
        .collect();
        expected.sort();
        assert_eq!(
            role_names_for_user(&app.db, app.state.default_tenant_id, user_id).await,
            expected,
            "exactly the owner role set — pre-assigned default cleared, no duplicates"
        );
        assert_ne!(
            app.state.settings.registration().mode,
            RegistrationMode::Closed,
            "handle_new_user must not publish; the commit owner does"
        );
    }

    /// The owner set of roles, sorted the same way `role_names_for_user`
    /// sorts its result (used by more than one guard test below).
    fn owner_role_names_sorted() -> Vec<String> {
        let mut expected: Vec<String> = [
            "viewer",
            "operator",
            "service_manager",
            "software_manager",
            "host_manager",
            "settings_manager",
            "command_manager",
            "system_administrator",
        ]
        .iter()
        .map(|s| (*s).to_string())
        .collect();
        expected.sort();
        expected
    }

    /// M1.6a guard test 1 (inverts the former pin test
    /// `oidc_callback_role_mapping_provider_overrides_owner_roles`): a
    /// role-mapping provider claims the first (and only) user's roles are
    /// `["admin"]` -> local `operator`. Applying that mapped replace would
    /// strip `settings_manager` (tenant `access:manage`) and
    /// `system_administrator` (`system.access:manage`) — the sole covering
    /// holders of both planes for this tenant. The lockout guard must skip
    /// the write; the login still succeeds and the owner set survives
    /// untouched.
    #[tokio::test]
    async fn oidc_role_mapping_cannot_strip_last_access_manage_holder() {
        let app = TestApp::new().await;
        let provider_id = insert_active_oidc_provider(
            &app.db,
            app.state.default_tenant_id,
            "First User Mapped",
            "first-user-mapped",
        )
        .await;
        // Configure role mapping: claim "admin" → local role "operator".
        // (ActiveModelTrait is already imported at audit_tests module level — do
        // not re-import it here; only IntoActiveModel is new.)
        {
            use sea_orm::IntoActiveModel as _;
            let provider_row = oidc_provider::Entity::find_by_id(provider_id)
                .one(&app.db)
                .await
                .expect("provider query")
                .expect("provider row");
            let mut active = provider_row.into_active_model();
            active.role_claim_path = Set(Some("roles".to_string()));
            active.role_mapping = Set(oidc_provider::RoleMapping(std::collections::HashMap::from(
                [("admin".to_string(), "operator".to_string())],
            )));
            active.update(&app.db).await.expect("update provider");
        }
        let provider = oidc_provider::Entity::find_by_id(provider_id)
            .one(&app.db)
            .await
            .expect("provider query")
            .expect("provider row");

        let txn = uptrakit_shared_db::access_grants::begin_guarded(&app.db)
            .await
            .expect("begin (guarded)");
        let user_id = insert_txn_user(&txn, "oidc-mapped@test.local").await;
        let claims = serde_json::json!({ "roles": ["admin"] });
        let (_, _, sync_outcome) = match super::handle_new_user(
            &app.state, &txn, user_id, &provider, &claims,
        )
        .await
        {
            Ok(v) => v,
            Err(_) => panic!(
                "handle_new_user must succeed — the guard skips the write, it never fails the login"
            ),
        };
        txn.commit().await.expect("commit");

        assert_eq!(
            role_names_for_user(&app.db, app.state.default_tenant_id, user_id).await,
            owner_role_names_sorted(),
            "the covering-shrink mapped replace must be skipped — the owner set survives untouched"
        );
        assert_eq!(
            sync_outcome,
            super::RoleSyncOutcome::SkippedLockout {
                attempted_role_names: vec!["operator".to_string()]
            }
        );
    }

    /// M1.6a guard test 2: a pure shrink that does NOT cover-strip
    /// `access:manage` still applies normally — the guard only blocks
    /// lockouts, not ordinary syncs. A second (non-first) user carries the
    /// default `viewer` role only; the same mapped-replace-to-`operator`
    /// provider is permitted because the tenant's first user still holds
    /// `settings_manager`.
    #[tokio::test]
    async fn oidc_role_mapping_pure_shrink_without_lockout_still_applies() {
        let app = TestApp::new().await;
        let provider_id = insert_active_oidc_provider(
            &app.db,
            app.state.default_tenant_id,
            "Pure Shrink",
            "pure-shrink",
        )
        .await;
        {
            use sea_orm::IntoActiveModel as _;
            let provider_row = oidc_provider::Entity::find_by_id(provider_id)
                .one(&app.db)
                .await
                .expect("provider query")
                .expect("provider row");
            let mut active = provider_row.into_active_model();
            active.role_claim_path = Set(Some("roles".to_string()));
            active.role_mapping = Set(oidc_provider::RoleMapping(std::collections::HashMap::from(
                [("admin".to_string(), "operator".to_string())],
            )));
            active.update(&app.db).await.expect("update provider");
        }
        let provider = oidc_provider::Entity::find_by_id(provider_id)
            .one(&app.db)
            .await
            .expect("provider query")
            .expect("provider row");

        // First user (owns the tenant's covering access:manage holder) so
        // the second user's shrink below is NOT the last covering holder.
        let txn = uptrakit_shared_db::access_grants::begin_guarded(&app.db)
            .await
            .expect("begin (guarded)");
        let first_user_id = insert_txn_user(&txn, "oidc-owner@test.local").await;
        super::handle_new_user(
            &app.state,
            &txn,
            first_user_id,
            &provider,
            &serde_json::json!({}),
        )
        .await
        .expect("first user setup must succeed");
        txn.commit().await.expect("commit first user");

        let txn = uptrakit_shared_db::access_grants::begin_guarded(&app.db)
            .await
            .expect("begin (guarded)");
        let user_id = insert_txn_user(&txn, "oidc-shrink@test.local").await;
        super::super::auth::assign_viewer_role(&txn, app.state.default_tenant_id, user_id)
            .await
            .expect("pre-assign default role");
        let claims = serde_json::json!({ "roles": ["admin"] });
        let sync_outcome = super::sync_oidc_roles(
            &txn,
            app.state.default_tenant_id,
            app.state.default_tenant_id,
            user_id,
            &provider,
            &claims,
        )
        .await
        .expect("sync must succeed — not a lockout");
        txn.commit().await.expect("commit second user's sync");

        assert_eq!(sync_outcome, super::RoleSyncOutcome::Applied);
        assert_eq!(
            role_names_for_user(&app.db, app.state.default_tenant_id, user_id).await,
            vec!["operator".to_string()],
            "non-covering shrink must still apply the mapped replace"
        );
    }

    /// M1.6a guard test 3: the `SkippedLockout` outcome from guard test 1,
    /// driven through the same post-commit path production code uses
    /// (`handle_role_sync_outcome`), emits the
    /// `user_role.sync_lockout_prevented` Event naming the provider and the
    /// attempted role set — the operator's only signal, since the login
    /// itself succeeded.
    #[tokio::test]
    async fn oidc_role_mapping_lockout_emits_prevented_audit_event() {
        let app = TestApp::new().await;
        let provider_id = insert_active_oidc_provider(
            &app.db,
            app.state.default_tenant_id,
            "Lockout Audit",
            "lockout-audit",
        )
        .await;
        {
            use sea_orm::IntoActiveModel as _;
            let provider_row = oidc_provider::Entity::find_by_id(provider_id)
                .one(&app.db)
                .await
                .expect("provider query")
                .expect("provider row");
            let mut active = provider_row.into_active_model();
            active.role_claim_path = Set(Some("roles".to_string()));
            active.role_mapping = Set(oidc_provider::RoleMapping(std::collections::HashMap::from(
                [("admin".to_string(), "operator".to_string())],
            )));
            active.update(&app.db).await.expect("update provider");
        }
        let provider = oidc_provider::Entity::find_by_id(provider_id)
            .one(&app.db)
            .await
            .expect("provider query")
            .expect("provider row");

        let txn = uptrakit_shared_db::access_grants::begin_guarded(&app.db)
            .await
            .expect("begin (guarded)");
        let user_id = insert_txn_user(&txn, "oidc-lockout-audit@test.local").await;
        let claims = serde_json::json!({ "roles": ["admin"] });
        let (_, _, sync_outcome) =
            match super::handle_new_user(&app.state, &txn, user_id, &provider, &claims).await {
                Ok(v) => v,
                Err(_) => panic!("handle_new_user must succeed"),
            };
        txn.commit().await.expect("commit");
        super::handle_role_sync_outcome(&app.state, user_id, provider.name.as_str(), sync_outcome)
            .await;

        let row = tenant_audit_row_for_action(
            &app.db,
            uptrakit_audit_log::AuditActionType::USER_ROLE_SYNC_LOCKOUT_PREVENTED,
        )
        .await;
        assert_eq!(row.target_id.as_deref(), Some(user_id.to_string().as_str()));
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
        let details = row.details_json.expect("details_json present");
        assert_eq!(details["provider"], serde_json::json!("Lockout Audit"));
        assert_eq!(details["attempted_roles"], serde_json::json!(["operator"]));
    }

    /// M1.6a guard test 4: a login whose mapped role set already equals the
    /// user's current role set returns `NoChange` without writing — the
    /// steady-state login path never takes the sentinel lock. Pinned via the
    /// `user_role.assigned_at` timestamp staying byte-for-byte unchanged.
    #[tokio::test]
    async fn oidc_role_mapping_noop_sync_takes_no_lock() {
        let app = TestApp::new().await;
        let provider_id = insert_active_oidc_provider(
            &app.db,
            app.state.default_tenant_id,
            "Noop Sync",
            "noop-sync",
        )
        .await;
        {
            use sea_orm::IntoActiveModel as _;
            let provider_row = oidc_provider::Entity::find_by_id(provider_id)
                .one(&app.db)
                .await
                .expect("provider query")
                .expect("provider row");
            let mut active = provider_row.into_active_model();
            active.role_claim_path = Set(Some("roles".to_string()));
            active.role_mapping = Set(oidc_provider::RoleMapping(std::collections::HashMap::from(
                [("admin".to_string(), "operator".to_string())],
            )));
            active.update(&app.db).await.expect("update provider");
        }
        let provider = oidc_provider::Entity::find_by_id(provider_id)
            .one(&app.db)
            .await
            .expect("provider query")
            .expect("provider row");

        let operator_role_id = Role::find()
            .filter(role::Column::Name.eq("operator"))
            .one(&app.db)
            .await
            .expect("role query")
            .expect("operator role exists")
            .id;

        let txn = uptrakit_shared_db::access_grants::begin_guarded(&app.db)
            .await
            .expect("begin (guarded)");
        let user_id = insert_txn_user(&txn, "oidc-noop@test.local").await;
        let assigned_at_before = OffsetDateTime::UNIX_EPOCH + time::Duration::days(1);
        user_role::ActiveModel {
            tenant_id: Set(app.state.default_tenant_id),
            user_id: Set(user_id),
            role_id: Set(operator_role_id),
            assigned_at: Set(assigned_at_before),
        }
        .insert(&txn)
        .await
        .expect("pre-assign operator role");
        let claims = serde_json::json!({ "roles": ["admin"] });
        let sync_outcome = super::sync_oidc_roles(
            &txn,
            app.state.default_tenant_id,
            app.state.default_tenant_id,
            user_id,
            &provider,
            &claims,
        )
        .await
        .expect("sync must succeed");
        txn.commit().await.expect("commit");

        assert_eq!(sync_outcome, super::RoleSyncOutcome::NoChange);
        let row = UserRole::find()
            .filter(user_role::Column::TenantId.eq(app.state.default_tenant_id))
            .filter(user_role::Column::UserId.eq(user_id))
            .one(&app.db)
            .await
            .expect("user_role query")
            .expect("user_role row survives");
        assert_eq!(
            row.assigned_at, assigned_at_before,
            "no-op sync must not touch the existing row's assigned_at"
        );
    }

    /// M1.6a guard test 5: `sync_oidc_roles`'s write phase runs inside a
    /// SAVEPOINT specifically so a mid-write failure can be rolled back
    /// without forcing the whole outer (login) transaction to abort —
    /// see the rustdoc above `sync_oidc_roles`. This pins that atomicity
    /// guarantee against a regression (e.g. a future swap to relying on
    /// `DatabaseTransaction`'s Drop-queued rollback instead of the explicit
    /// `sp.rollback()` call) actually holding.
    ///
    /// A same-tenant delete-then-insert-mismatch failure inside the
    /// savepoint is not reachable via pure data setup: the roles the insert
    /// loop writes are the exact rows a single batched `SELECT` already
    /// fetched earlier in the SAME function, on the SAME connection, inside
    /// the SAME `Immediate`-mode transaction that holds SQLite's write lock
    /// for its whole duration — nothing can invalidate those ids between the
    /// fetch and the insert. So the failure here is induced via a `tenant_id`
    /// that has no row in `tenants` (an FK violation on the `user_roles`
    /// INSERT), scoped away from the user's real, pre-existing role
    /// assignment under the real tenant. That still exercises the exact
    /// `Err(e) => sp.rollback()` branch and proves the two things a
    /// Drop-based regression would break: no orphaned row survives the
    /// failed write, and collateral (real-tenant) data is untouched.
    #[tokio::test]
    async fn oidc_role_mapping_savepoint_rolls_back_on_write_failure() {
        let app = TestApp::new().await;
        let provider_id = insert_active_oidc_provider(
            &app.db,
            app.state.default_tenant_id,
            "Savepoint Rollback",
            "savepoint-rollback",
        )
        .await;
        {
            use sea_orm::IntoActiveModel as _;
            let provider_row = oidc_provider::Entity::find_by_id(provider_id)
                .one(&app.db)
                .await
                .expect("provider query")
                .expect("provider row");
            let mut active = provider_row.into_active_model();
            active.role_claim_path = Set(Some("roles".to_string()));
            active.role_mapping = Set(oidc_provider::RoleMapping(std::collections::HashMap::from(
                [("admin".to_string(), "operator".to_string())],
            )));
            active.update(&app.db).await.expect("update provider");
        }
        let provider = oidc_provider::Entity::find_by_id(provider_id)
            .one(&app.db)
            .await
            .expect("provider query")
            .expect("provider row");

        let viewer_role_id = Role::find()
            .filter(role::Column::Name.eq("viewer"))
            .one(&app.db)
            .await
            .expect("role query")
            .expect("viewer role exists")
            .id;

        // Set up the user's real, pre-existing role assignment under the
        // real tenant, in its own committed transaction.
        let txn = uptrakit_shared_db::access_grants::begin_guarded(&app.db)
            .await
            .expect("begin (guarded)");
        let user_id = insert_txn_user(&txn, "oidc-savepoint@test.local").await;
        let pre_existing_assigned_at = OffsetDateTime::UNIX_EPOCH + time::Duration::days(1);
        user_role::ActiveModel {
            tenant_id: Set(app.state.default_tenant_id),
            user_id: Set(user_id),
            role_id: Set(viewer_role_id),
            assigned_at: Set(pre_existing_assigned_at),
        }
        .insert(&txn)
        .await
        .expect("pre-assign viewer role under the real tenant");
        txn.commit().await.expect("commit pre-existing assignment");

        // Sync scoped to a tenant_id with no row in `tenants` -- the
        // savepoint's write phase gets past the DELETE (matches nothing for
        // this tenant/user pair) but the INSERT violates the `user_roles.
        // tenant_id -> tenants.id` foreign key, forcing `write_result` to
        // Err and driving the explicit `sp.rollback()` branch.
        let bogus_tenant_id = uuid::Uuid::now_v7();
        let txn = uptrakit_shared_db::access_grants::begin_guarded(&app.db)
            .await
            .expect("begin (guarded)");
        let claims = serde_json::json!({ "roles": ["admin"] });
        let sync_result = super::sync_oidc_roles(
            &txn,
            bogus_tenant_id,
            app.state.default_tenant_id,
            user_id,
            &provider,
            &claims,
        )
        .await;
        assert!(
            sync_result.is_err(),
            "the FK-violating insert must surface as Err, not silently succeed"
        );
        // Mirror the real caller contract (`let _ = sync_oidc_roles(...)`):
        // a sync error does not abort the outer transaction.
        txn.commit()
            .await
            .expect("outer transaction commit must still succeed after the savepoint rollback");

        let bogus_tenant_rows = UserRole::find()
            .filter(user_role::Column::TenantId.eq(bogus_tenant_id))
            .filter(user_role::Column::UserId.eq(user_id))
            .all(&app.db)
            .await
            .expect("bogus-tenant user_role query");
        assert!(
            bogus_tenant_rows.is_empty(),
            "the savepoint rollback must leave no orphaned partial write behind"
        );

        let real_tenant_row = UserRole::find()
            .filter(user_role::Column::TenantId.eq(app.state.default_tenant_id))
            .filter(user_role::Column::UserId.eq(user_id))
            .one(&app.db)
            .await
            .expect("real-tenant user_role query")
            .expect("the pre-existing role assignment must survive intact");
        assert_eq!(real_tenant_row.role_id, viewer_role_id);
        assert_eq!(
            real_tenant_row.assigned_at, pre_existing_assigned_at,
            "the pre-existing assignment must be untouched by the failed, unrelated-tenant sync"
        );
    }

    /// Spec test 4 (OIDC half) + post-commit publish coverage for the
    /// oidc_complete_registration publish site: happy-path first user via the
    /// route → 200, exactly the owner role set, snapshot Closed AFTER commit.
    /// (The execute_oidc_resolution NewUser-arm publish site is not reachable
    /// route-level without a mock IdP; it is covered by the final-verification
    /// grep that every set_registration call sits after its commit.)
    #[tokio::test]
    async fn oidc_complete_registration_first_user_publishes_closed_after_commit() {
        use crate::auth::registration::RegistrationMode;

        let app = TestApp::new().await;
        let client = app.client();
        let provider_id = insert_active_oidc_provider(
            &app.db,
            app.state.default_tenant_id,
            "Complete Registration Happy",
            "complete-registration-happy",
        )
        .await;

        let registration_code = "happy-first-user";
        app.state
            .oidc
            .oidc_registration_store
            .insert(PendingOidcRegistrationParams {
                registration_code: registration_code.to_string(),
                provider_id,
                oidc_subject: "happy-subject".to_string(),
                email: "happy@test.local".to_string(),
                first_name: Some("Happy".to_string()),
                last_name: Some("First".to_string()),
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

        let first = User::find()
            .one(&app.db)
            .await
            .expect("query")
            .expect("first user committed");
        let mut expected: Vec<String> = [
            "viewer",
            "operator",
            "service_manager",
            "software_manager",
            "host_manager",
            "settings_manager",
            "command_manager",
            "system_administrator",
        ]
        .iter()
        .map(|s| (*s).to_string())
        .collect();
        expected.sort();
        assert_eq!(
            role_names_for_user(&app.db, app.state.default_tenant_id, first.id).await,
            expected
        );
        assert_eq!(
            app.state.settings.registration().mode,
            RegistrationMode::Closed,
            "snapshot published Closed after the route's commit"
        );
    }

    /// Spec test 2 (complete-registration path, the fourth bug instance):
    /// owner-role failure → 500 via the route, nothing committed, snapshot
    /// untouched. RED pre-fix: the swallow committed a viewer-only first user.
    #[tokio::test]
    async fn oidc_complete_registration_rolls_back_first_user_on_owner_role_failure() {
        let app = TestApp::new().await;
        let client = app.client();
        let provider_id = insert_active_oidc_provider(
            &app.db,
            app.state.default_tenant_id,
            "Complete Registration Rollback",
            "complete-registration-rollback",
        )
        .await;
        let mode_before = app.state.settings.registration().mode;

        break_owner_role_assignment(&app.db).await;

        let registration_code = "rollback-first-user";
        app.state
            .oidc
            .oidc_registration_store
            .insert(PendingOidcRegistrationParams {
                registration_code: registration_code.to_string(),
                provider_id,
                oidc_subject: "rollback-subject".to_string(),
                email: "rollback@test.local".to_string(),
                first_name: Some("Roll".to_string()),
                last_name: Some("Back".to_string()),
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
        assert_eq!(status, http::StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(
            User::find().count(&app.db).await.expect("count"),
            0,
            "transaction must roll back: no user row committed"
        );
        assert_eq!(app.state.settings.registration().mode, mode_before);
    }

    /// M16a-plan3 Task 2: a tenant-scoped role whose name is claimed by a
    /// role-mapping value must never shadow the matching global role —
    /// `sync_oidc_roles`'s `.is_in(local_role_names)` lookup must stay
    /// scoped to `tenant_id IS NULL` rows. Deterministic pre-fix (unlike
    /// the `.one()` sites): both the global and shadow "operator" rows
    /// match the unscoped `.is_in()`/`.all()` query, so the shadow is
    /// visibly assigned alongside the global row.
    #[tokio::test]
    async fn sync_oidc_roles_ignores_tenant_shadow() {
        let app = TestApp::new().await;
        let provider_id = insert_active_oidc_provider(
            &app.db,
            app.state.default_tenant_id,
            "Role Shadow",
            "role-shadow",
        )
        .await;
        {
            use sea_orm::IntoActiveModel as _;
            let provider_row = oidc_provider::Entity::find_by_id(provider_id)
                .one(&app.db)
                .await
                .expect("provider query")
                .expect("provider row");
            let mut active = provider_row.into_active_model();
            active.role_claim_path = Set(Some("roles".to_string()));
            active.role_mapping = Set(oidc_provider::RoleMapping(std::collections::HashMap::from(
                [("admin".to_string(), "operator".to_string())],
            )));
            active.update(&app.db).await.expect("update provider");
        }
        let provider = oidc_provider::Entity::find_by_id(provider_id)
            .one(&app.db)
            .await
            .expect("provider query")
            .expect("provider row");

        let global_operator_id = Role::find()
            .filter(role::Column::Name.eq("operator"))
            .filter(role::Column::TenantId.is_null())
            .one(&app.db)
            .await
            .expect("role query")
            .expect("global operator role exists")
            .id;
        let shadow_operator_id = crate::test_harness::fixtures::insert_shadow_role(
            &app.db,
            app.state.default_tenant_id,
            "operator",
        )
        .await;

        // First user (owns the tenant's covering access:manage holder) so
        // the second user's sync below is not the last covering holder.
        let txn = uptrakit_shared_db::access_grants::begin_guarded(&app.db)
            .await
            .expect("begin (guarded)");
        let first_user_id = insert_txn_user(&txn, "oidc-role-shadow-owner@test.local").await;
        super::handle_new_user(
            &app.state,
            &txn,
            first_user_id,
            &provider,
            &serde_json::json!({}),
        )
        .await
        .expect("first user setup must succeed");
        txn.commit().await.expect("commit first user");

        let txn = uptrakit_shared_db::access_grants::begin_guarded(&app.db)
            .await
            .expect("begin (guarded)");
        let user_id = insert_txn_user(&txn, "oidc-role-shadow@test.local").await;
        let claims = serde_json::json!({ "roles": ["admin"] });
        let sync_outcome = super::sync_oidc_roles(
            &txn,
            app.state.default_tenant_id,
            app.state.default_tenant_id,
            user_id,
            &provider,
            &claims,
        )
        .await
        .expect("sync must succeed — not a lockout");
        txn.commit().await.expect("commit sync");

        assert_eq!(sync_outcome, super::RoleSyncOutcome::Applied);
        let assigned_role_ids: Vec<uuid::Uuid> = UserRole::find()
            .filter(user_role::Column::TenantId.eq(app.state.default_tenant_id))
            .filter(user_role::Column::UserId.eq(user_id))
            .all(&app.db)
            .await
            .expect("user_role query")
            .into_iter()
            .map(|r| r.role_id)
            .collect();
        assert_eq!(
            assigned_role_ids,
            vec![global_operator_id],
            "sync_oidc_roles must assign only the global operator role"
        );
        assert!(
            !assigned_role_ids.contains(&shadow_operator_id),
            "the tenant shadow role must never be assigned by sync_oidc_roles"
        );
    }

    /// M16a-plan3 Task 2: a tenant-scoped role named "user" must never
    /// shadow a global built-in — `resolve_oidc_user`'s best-effort
    /// default-role lookup must stay scoped to `tenant_id IS NULL` rows.
    /// The legacy "user" role no longer exists among the built-in roles
    /// (see `m20260310_000002_granular_permissions.rs`'s `OLD_ROLES`
    /// migration), so a tenant-created role named "user" is the ONLY name
    /// match — pre-fix, the unscoped lookup finds and assigns it;
    /// post-fix, `resolve_oidc_user` must stay inert (assign nothing).
    #[tokio::test]
    async fn resolve_oidc_user_stays_inert_for_tenant_role_named_user() {
        let app = TestApp::new().await;
        let provider_id = insert_active_oidc_provider(
            &app.db,
            app.state.default_tenant_id,
            "User Shadow",
            "user-shadow",
        )
        .await;
        let provider = oidc_provider::Entity::find_by_id(provider_id)
            .one(&app.db)
            .await
            .expect("provider query")
            .expect("provider row");

        crate::test_harness::fixtures::insert_shadow_role(
            &app.db,
            app.state.default_tenant_id,
            "user",
        )
        .await;

        let resolution = super::resolve_oidc_user(super::OidcUserParams {
            db: &app.db,
            tenant_id: app.state.default_tenant_id,
            provider_id: provider.id,
            oidc_subject: "user-shadow-subject",
            email: "user-shadow@test.local",
            first_name: Some("Shadow"),
            last_name: Some("Target"),
            auto_create: true,
            email_verified: Some(true),
        })
        .await
        .expect("resolve_oidc_user must succeed");

        let user_id = match resolution {
            super::OidcUserResolution::NewUser(id) => id,
            _ => panic!("expected NewUser resolution"),
        };

        let role_count = UserRole::find()
            .filter(user_role::Column::TenantId.eq(app.state.default_tenant_id))
            .filter(user_role::Column::UserId.eq(user_id))
            .count(&app.db)
            .await
            .expect("count user_role rows");
        assert_eq!(
            role_count, 0,
            "resolve_oidc_user must stay inert — no global \"user\" role \
             exists, so the tenant shadow must never be picked up as a \
             best-effort default"
        );
    }
}
