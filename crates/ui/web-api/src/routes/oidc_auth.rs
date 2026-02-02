use crate::AppState;
use crate::auth::authentication::{
    OidcUserResolution, extract_mapped_roles, resolve_oidc_user, sync_oidc_roles,
};
use crate::auth::password;
use crate::auth::session::SessionService;
use crate::auth::token::{generate_secure_token, generate_uuid};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Redirect, Response},
};
use openidconnect::{
    AuthenticationFlow, AuthorizationCode, ClientId, ClientSecret, CsrfToken, IssuerUrl, Nonce,
    PkceCodeChallenge, RedirectUrl, Scope, TokenResponse,
    core::{CoreClient, CoreProviderMetadata, CoreResponseType},
    reqwest,
};
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};
use serde::Deserialize;
use std::sync::Arc;
use time::OffsetDateTime;
use uptrakit_shared_db::entity::prelude::*;
use uptrakit_shared_db::entity::{oidc_provider, user_oidc_link};

pub use super::auth::AuthResponse;
pub use uptrakit_web_api_types::oidc_auth::{
    AuthMethodsResponse, OidcAuthorizeResponse, OidcExchangeRequest, OidcLinkRequest,
    OidcProviderInfo,
};

#[derive(Deserialize)]
pub struct OidcCallbackParams {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
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
pub async fn auth_methods(State(state): State<Arc<AppState>>) -> Response {
    let auth_settings = state.settings.authentication().await;

    let providers = OidcProvider::find()
        .filter(oidc_provider::Column::TenantId.eq(state.default_tenant_id))
        .filter(oidc_provider::Column::IsActive.eq(true))
        .filter(oidc_provider::Column::DeletedAt.is_null())
        .all(&state.db)
        .await
        .unwrap_or_default();

    let oidc_providers: Vec<OidcProviderInfo> = providers
        .into_iter()
        .map(|p| OidcProviderInfo {
            id: p.id.to_string(),
            name: p.name,
            slug: p.slug,
            logo_url: p.logo_url,
        })
        .collect();

    let response = AuthMethodsResponse {
        password: auth_settings.password_auth_enabled,
        oidc_providers,
    };

    (StatusCode::OK, Json(response)).into_response()
}

/// Start OIDC authorization flow (public)
#[utoipa::path(
    get,
    path = "/api/v1/auth/oidc/{provider_id}/authorize",
    params(("provider_id" = String, Path, description = "OIDC Provider ID")),
    responses(
        (status = 200, description = "Authorization URL", body = OidcAuthorizeResponse),
        (status = 404, description = "Provider not found or inactive")
    ),
    tag = "Authentication"
)]
pub async fn oidc_authorize(
    State(state): State<Arc<AppState>>,
    Path(provider_id): Path<String>,
    external_base_url: Option<Extension<crate::extract::ExternalBaseUrl>>,
    headers: HeaderMap,
) -> Response {
    let base_url = external_base_url
        .map(|Extension(u)| u.0)
        .or_else(|| base_url_from_headers(&headers));
    let base_url = match base_url {
        Some(url) => url,
        None => return (StatusCode::BAD_REQUEST, "Missing Host header").into_response(),
    };

    let redirect_url = match RedirectUrl::new(format!("{base_url}/api/v1/auth/oidc/callback")) {
        Ok(url) => url,
        Err(e) => {
            tracing::error!("Invalid OIDC redirect URL: {e}");
            return (StatusCode::BAD_REQUEST, "Invalid redirect URL").into_response();
        }
    };

    let provider_uuid = match uuid::Uuid::parse_str(&provider_id) {
        Ok(id) => id,
        Err(_) => return (StatusCode::BAD_REQUEST, "Invalid provider ID").into_response(),
    };

    let provider = match find_active_provider(&state.db, state.default_tenant_id, provider_uuid)
        .await
    {
        Some(p) => p,
        None => return (StatusCode::NOT_FOUND, "Provider not found or inactive").into_response(),
    };

    // Build OIDC client via discovery
    let issuer_url = match IssuerUrl::new(provider.issuer_url.clone()) {
        Ok(u) => u,
        Err(e) => {
            tracing::error!("Invalid issuer URL for provider {}: {e}", provider.slug);
            return (StatusCode::BAD_GATEWAY, "Invalid OIDC issuer URL").into_response();
        }
    };
    let http_client = reqwest::Client::default();
    let provider_metadata =
        match CoreProviderMetadata::discover_async(issuer_url, &http_client).await {
            Ok(m) => m,
            Err(e) => {
                tracing::error!("OIDC discovery failed for provider {}: {e}", provider.slug);
                return (StatusCode::BAD_GATEWAY, "OIDC provider discovery failed").into_response();
            }
        };
    let client = CoreClient::from_provider_metadata(
        provider_metadata,
        ClientId::new(provider.client_id.clone()),
        Some(ClientSecret::new(provider.client_secret.clone())),
    );
    let client = client.set_redirect_uri(redirect_url);

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
        .oidc_flow_store
        .insert(
            csrf_state.secret().clone(),
            provider_uuid,
            &pkce_verifier,
            &nonce,
        )
        .await
    {
        tracing::error!("Failed to store OIDC flow: {e:?}");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    let response = OidcAuthorizeResponse {
        authorize_url: auth_url.to_string(),
    };

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
pub async fn oidc_callback(
    State(state): State<Arc<AppState>>,
    Query(params): Query<OidcCallbackParams>,
    external_base_url: Option<Extension<crate::extract::ExternalBaseUrl>>,
    headers: HeaderMap,
) -> Response {
    // Handle error from provider
    if params.error.is_some() {
        return Redirect::to("/login?error=oidc_denied").into_response();
    }

    let (code, csrf_state) = match (params.code, params.state) {
        (Some(c), Some(s)) => (c, s),
        _ => return Redirect::to("/login?error=oidc_missing_params").into_response(),
    };

    // Retrieve pending flow from database
    let flow = match state.oidc_flow_store.take(&csrf_state).await {
        Ok(Some(f)) => f,
        Ok(None) => return Redirect::to("/login?error=oidc_state_expired").into_response(),
        Err(e) => {
            tracing::error!("Failed to retrieve OIDC flow: {e:?}");
            return Redirect::to("/login?error=oidc_internal_error").into_response();
        }
    };

    // Load provider
    let provider =
        match find_active_provider(&state.db, state.default_tenant_id, flow.provider_id).await {
            Some(p) => p,
            None => return Redirect::to("/login?error=oidc_provider_gone").into_response(),
        };

    let base_url = external_base_url
        .map(|Extension(u)| u.0)
        .or_else(|| base_url_from_headers(&headers));
    let base_url = match base_url {
        Some(url) => url,
        None => return Redirect::to("/login?error=oidc_missing_host").into_response(),
    };
    let redirect_url = match RedirectUrl::new(format!("{base_url}/api/v1/auth/oidc/callback")) {
        Ok(url) => url,
        Err(e) => {
            tracing::error!("Invalid OIDC redirect URL during callback: {e}");
            return Redirect::to("/login?error=oidc_invalid_redirect").into_response();
        }
    };

    // Build OIDC client
    let issuer_url = match IssuerUrl::new(provider.issuer_url.clone()) {
        Ok(u) => u,
        Err(e) => {
            tracing::error!("Invalid issuer URL during callback: {e}");
            return Redirect::to("/login?error=oidc_discovery_failed").into_response();
        }
    };
    let http_client = reqwest::Client::default();
    let provider_metadata =
        match CoreProviderMetadata::discover_async(issuer_url, &http_client).await {
            Ok(m) => m,
            Err(e) => {
                tracing::error!("OIDC discovery failed during callback: {e}");
                return Redirect::to("/login?error=oidc_discovery_failed").into_response();
            }
        };
    let client = CoreClient::from_provider_metadata(
        provider_metadata,
        ClientId::new(provider.client_id.clone()),
        Some(ClientSecret::new(provider.client_secret.clone())),
    );
    let client = client.set_redirect_uri(redirect_url.clone());

    // Exchange code for tokens
    let token_request = match client.exchange_code(AuthorizationCode::new(code)) {
        Ok(req) => req,
        Err(e) => {
            tracing::error!("OIDC token endpoint not configured: {e}");
            return Redirect::to("/login?error=oidc_token_exchange_failed").into_response();
        }
    };
    let token_response = match token_request
        .set_redirect_uri(std::borrow::Cow::Owned(redirect_url))
        .set_pkce_verifier(flow.pkce_verifier)
        .request_async(&http_client)
        .await
    {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("OIDC token exchange failed: {e}");
            return Redirect::to("/login?error=oidc_token_exchange_failed").into_response();
        }
    };

    // Extract ID token and validate
    let id_token = match token_response.id_token() {
        Some(t) => t,
        None => return Redirect::to("/login?error=oidc_no_id_token").into_response(),
    };

    let id_token_verifier = client.id_token_verifier();
    let claims = match id_token.claims(&id_token_verifier, &flow.nonce) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("OIDC ID token validation failed: {e}");
            return Redirect::to("/login?error=oidc_token_validation_failed").into_response();
        }
    };

    // Extract standard claims
    let sub = claims.subject().to_string();
    let email = claims.email().map(|e| e.to_string()).unwrap_or_default();
    let first_name = claims
        .given_name()
        .and_then(|n| n.get(None))
        .map(|n| n.to_string());
    let last_name = claims
        .family_name()
        .and_then(|n| n.get(None))
        .map(|n| n.to_string());

    if email.is_empty() {
        return Redirect::to("/login?error=oidc_no_email").into_response();
    }

    // Get additional claims as JSON for role mapping
    let additional_claims = serde_json::to_value(claims.additional_claims()).unwrap_or_default();

    // Resolve user
    let resolution = match resolve_oidc_user(
        &state.db,
        state.default_tenant_id,
        flow.provider_id,
        &sub,
        &email,
        first_name.as_deref(),
        last_name.as_deref(),
        provider.auto_create_users,
    )
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("OIDC user resolution failed: {e:?}");
            return Redirect::to("/login?error=oidc_internal_error").into_response();
        }
    };

    match resolution {
        OidcUserResolution::LinkedUser(user_id) => {
            // Sync roles and create session
            let _ = sync_oidc_roles(
                &state.db,
                state.default_tenant_id,
                user_id,
                &provider,
                &additional_claims,
            )
            .await;
            create_oidc_exchange_and_redirect(&state, user_id, flow.provider_id).await
        }
        OidcUserResolution::NewUser(user_id) => {
            let _ = sync_oidc_roles(
                &state.db,
                state.default_tenant_id,
                user_id,
                &provider,
                &additional_claims,
            )
            .await;
            create_oidc_exchange_and_redirect(&state, user_id, flow.provider_id).await
        }
        OidcUserResolution::AutoLink { user_id } => {
            // Auto-link and create session
            let link = user_oidc_link::ActiveModel {
                id: Set(generate_uuid()),
                user_id: Set(user_id),
                provider_id: Set(flow.provider_id),
                oidc_subject: Set(sub),
                linked_at: Set(OffsetDateTime::now_utc()),
            };
            if let Err(e) = link.insert(&state.db).await {
                tracing::error!("Failed to auto-link OIDC account: {e}");
                return Redirect::to("/login?error=oidc_link_failed").into_response();
            }
            let _ = sync_oidc_roles(
                &state.db,
                state.default_tenant_id,
                user_id,
                &provider,
                &additional_claims,
            )
            .await;
            create_oidc_exchange_and_redirect(&state, user_id, flow.provider_id).await
        }
        OidcUserResolution::LinkViaPasswordRequired { user_id } => {
            // Store pending link and redirect to frontend
            let mapped_roles = extract_mapped_roles(&provider, &additional_claims);
            let link_token_value =
                generate_secure_token().unwrap_or_else(|_| generate_uuid().to_string());
            let link_token = match store_pending_link(
                &state,
                crate::auth::oidc_state::PendingAccountLinkParams {
                    token: link_token_value,
                    provider_id: flow.provider_id,
                    oidc_subject: sub,
                    email: email.clone(),
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
                    tracing::error!("Failed to store pending link: {e:?}");
                    return Redirect::to("/login?error=oidc_internal_error").into_response();
                }
            };
            let encoded_email = urlencoding::encode(&email);
            Redirect::to(&format!(
                "/login?link_required=true&link_token={link_token}&email={encoded_email}"
            ))
            .into_response()
        }
        OidcUserResolution::LinkViaOidcRequired {
            user_id,
            existing_provider_id,
        } => {
            let mapped_roles = extract_mapped_roles(&provider, &additional_claims);
            let link_token_value =
                generate_secure_token().unwrap_or_else(|_| generate_uuid().to_string());
            let link_token = match store_pending_link(
                &state,
                crate::auth::oidc_state::PendingAccountLinkParams {
                    token: link_token_value,
                    provider_id: flow.provider_id,
                    oidc_subject: sub,
                    email: email.clone(),
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
                    tracing::error!("Failed to store pending link: {e:?}");
                    return Redirect::to("/login?error=oidc_internal_error").into_response();
                }
            };
            let encoded_email = urlencoding::encode(&email);
            Redirect::to(&format!(
                "/login?link_required=true&link_token={link_token}&email={encoded_email}&link_provider_id={existing_provider_id}"
            ))
            .into_response()
        }
        OidcUserResolution::NotAllowed => {
            Redirect::to("/login?error=oidc_no_account").into_response()
        }
        OidcUserResolution::Deactivated => {
            Redirect::to("/login?error=account_deactivated").into_response()
        }
    }
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
pub async fn oidc_exchange(
    State(state): State<Arc<AppState>>,
    Json(req): Json<OidcExchangeRequest>,
) -> Response {
    let pending = match state.oidc_token_exchange_store.take(&req.code).await {
        Ok(Some(p)) => p,
        Ok(None) => {
            return (StatusCode::BAD_REQUEST, "Invalid or expired exchange code").into_response();
        }
        Err(e) => {
            tracing::error!("Failed to retrieve OIDC exchange: {e:?}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    // Create refresh token
    let session_service = SessionService::new(state.db.clone());
    let refresh_token = match session_service
        .create_refresh_token(
            pending.user_id,
            AuthMethod::Oidc {
                provider_id: pending.provider_id,
            },
            None,
            None,
        )
        .await
    {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("Failed to create refresh token during OIDC exchange: {e:?}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    // Get user info
    let user = match User::find_by_id(pending.user_id).one(&state.db).await {
        Ok(Some(u)) => u,
        _ => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };

    let permissions =
        super::auth::get_user_permissions(&state.db, state.default_tenant_id, pending.user_id)
            .await
            .unwrap_or_default();

    // Create JWT access token
    let access_token = match state.jwt.create_access_token(
        pending.user_id,
        &permissions,
        "oidc",
        Some(pending.provider_id),
    ) {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("Failed to create access token during OIDC exchange: {e:?}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let response = AuthResponse {
        access_token,
        refresh_token,
        expires_in: state.jwt.expires_in(),
        token_type: "Bearer".to_string(),
        user: super::auth::UserResponse {
            id: user.id.to_string(),
            email: user.email,
            first_name: user.first_name,
            last_name: user.last_name,
            permissions,
        },
    };

    (StatusCode::OK, Json(response)).into_response()
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
pub async fn oidc_link(
    State(state): State<Arc<AppState>>,
    req: axum::extract::Request,
) -> Response {
    // Parse the body manually since we also need headers
    let (parts, body) = req.into_parts();
    let bytes = match axum::body::to_bytes(body, 1024 * 16).await {
        Ok(b) => b,
        Err(_) => return (StatusCode::BAD_REQUEST, "Invalid request body").into_response(),
    };
    let link_req: OidcLinkRequest = match serde_json::from_slice(&bytes) {
        Ok(r) => r,
        Err(_) => return (StatusCode::BAD_REQUEST, "Invalid JSON").into_response(),
    };

    // Retrieve pending link from database
    let pending = match state.account_link_store.take(&link_req.link_token).await {
        Ok(Some(p)) => p,
        Ok(None) => {
            return (StatusCode::BAD_REQUEST, "Link token not found or expired").into_response();
        }
        Err(e) => {
            tracing::error!("Failed to retrieve pending link: {e:?}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    // Verify ownership
    let verified = if let Some(ref pwd) = link_req.password {
        // Password verification
        let user = match User::find_by_id(pending.user_id).one(&state.db).await {
            Ok(Some(u)) => u,
            _ => return (StatusCode::UNAUTHORIZED, "User not found").into_response(),
        };
        let hash = match user.password_hash.as_ref() {
            Some(h) => h,
            None => return (StatusCode::UNAUTHORIZED, "User has no password").into_response(),
        };
        matches!(password::verify_password(pwd, hash), Ok(true))
    } else {
        // Bearer token verification (OIDC-to-OIDC linking) — now JWT-based
        let bearer = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .map(|s| s.to_string());

        if let Some(token) = bearer {
            match state.jwt.decode_access_token(&token) {
                Ok(claims) => uuid::Uuid::parse_str(&claims.sub)
                    .map(|uid| uid == pending.user_id)
                    .unwrap_or(false),
                Err(_) => false,
            }
        } else {
            false
        }
    };

    if !verified {
        return (StatusCode::UNAUTHORIZED, "Verification failed").into_response();
    }

    // Create the link
    let link = user_oidc_link::ActiveModel {
        id: Set(generate_uuid()),
        user_id: Set(pending.user_id),
        provider_id: Set(pending.provider_id),
        oidc_subject: Set(pending.oidc_subject),
        linked_at: Set(OffsetDateTime::now_utc()),
    };

    if let Err(e) = link.insert(&state.db).await {
        tracing::error!("Failed to create OIDC link: {e}");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    // Sync roles if we have mapped roles
    if !pending.mapped_roles.is_empty() {
        // Load provider for role sync
        if let Some(provider) =
            find_active_provider(&state.db, state.default_tenant_id, pending.provider_id).await
        {
            // Build a minimal claims object just for role sync
            // We already have the mapped roles, so we build fake claims matching the mapping
            let mut fake_claims = serde_json::Map::new();
            if let Some(ref path) = provider.role_claim_path {
                let reverse_mapped: Vec<String> = pending
                    .mapped_roles
                    .iter()
                    .filter_map(|local_name| {
                        provider
                            .role_mapping
                            .0
                            .iter()
                            .find(|(_, v)| v.as_str() == local_name)
                            .map(|(k, _)| k.clone())
                    })
                    .collect();
                // Set at the first path segment for simplicity
                let first_segment = path.split('.').next().unwrap_or(path);
                fake_claims.insert(
                    first_segment.to_string(),
                    serde_json::Value::Array(
                        reverse_mapped
                            .into_iter()
                            .map(serde_json::Value::String)
                            .collect(),
                    ),
                );
            }
            let _ = sync_oidc_roles(
                &state.db,
                state.default_tenant_id,
                pending.user_id,
                &provider,
                &serde_json::Value::Object(fake_claims),
            )
            .await;
        }
    }

    // Create refresh token
    let session_service = SessionService::new(state.db.clone());
    let refresh_token = match session_service
        .create_refresh_token(
            pending.user_id,
            AuthMethod::Oidc {
                provider_id: pending.provider_id,
            },
            None,
            None,
        )
        .await
    {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("Failed to create refresh token: {e:?}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    // Get user info for response
    let user = match User::find_by_id(pending.user_id).one(&state.db).await {
        Ok(Some(u)) => u,
        _ => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };

    let permissions =
        super::auth::get_user_permissions(&state.db, state.default_tenant_id, pending.user_id)
            .await
            .unwrap_or_default();

    // Create JWT access token
    let access_token = match state.jwt.create_access_token(
        pending.user_id,
        &permissions,
        "oidc",
        Some(pending.provider_id),
    ) {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("Failed to create access token: {e:?}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let response = AuthResponse {
        access_token,
        refresh_token,
        expires_in: state.jwt.expires_in(),
        token_type: "Bearer".to_string(),
        user: super::auth::UserResponse {
            id: user.id.to_string(),
            email: user.email,
            first_name: user.first_name,
            last_name: user.last_name,
            permissions,
        },
    };

    (StatusCode::OK, Json(response)).into_response()
}

// Helper functions

async fn find_active_provider(
    db: &DatabaseConnection,
    tenant_id: uuid::Uuid,
    id: uuid::Uuid,
) -> Option<oidc_provider::Model> {
    OidcProvider::find_by_id(id)
        .filter(oidc_provider::Column::TenantId.eq(tenant_id))
        .filter(oidc_provider::Column::IsActive.eq(true))
        .filter(oidc_provider::Column::DeletedAt.is_null())
        .one(db)
        .await
        .ok()
        .flatten()
}

/// Store only (user_id, provider_id) in the database and redirect with exchange code.
/// Token creation is deferred to the `oidc_exchange` endpoint.
async fn create_oidc_exchange_and_redirect(
    state: &AppState,
    user_id: uuid::Uuid,
    provider_id: uuid::Uuid,
) -> Response {
    // Generate exchange code
    let exchange_code = generate_secure_token().unwrap_or_else(|_| generate_uuid().to_string());

    if let Err(e) = state
        .oidc_token_exchange_store
        .insert(exchange_code.clone(), user_id, provider_id)
        .await
    {
        tracing::error!("Failed to store OIDC exchange: {e:?}");
        return Redirect::to("/login?error=oidc_session_failed").into_response();
    }

    Redirect::to(&format!("/login?oidc_code={exchange_code}")).into_response()
}

async fn store_pending_link(
    state: &AppState,
    params: crate::auth::oidc_state::PendingAccountLinkParams,
) -> std::result::Result<String, rootcause::Report<crate::auth::oidc_state::OidcStoreError>> {
    let link_token = params.token.clone();
    state.account_link_store.insert(params).await?;
    Ok(link_token)
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
    use super::base_url_from_headers;
    use axum::http::{HeaderMap, HeaderValue};

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
}
