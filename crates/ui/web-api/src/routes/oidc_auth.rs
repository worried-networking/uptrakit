use crate::AppState;
use crate::auth::authentication::{
    OidcUserParams, OidcUserResolution, extract_mapped_roles, resolve_oidc_user, sync_oidc_roles,
};
use crate::auth::password;
use crate::auth::refresh_cookie::set_refresh_token_cookie;
use crate::auth::session::SessionService;
use crate::auth::token::{generate_secure_token, generate_uuid};
use crate::error_response::error_response;
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Redirect, Response},
};
use openidconnect::{
    AuthenticationFlow, AuthorizationCode, ClientId, ClientSecret, CsrfToken,
    EndpointMaybeSet, EndpointNotSet, EndpointSet,
    IssuerUrl, Nonce, PkceCodeChallenge, PkceCodeVerifier, RedirectUrl, Scope, TokenResponse,
    core::{CoreClient, CoreProviderMetadata, CoreResponseType},
    reqwest,
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait, QueryFilter, Set,
    TransactionTrait,
};
use serde::Deserialize;
use std::sync::Arc;
use time::OffsetDateTime;
use uptrakit_shared_db::entity::prelude::*;
use uptrakit_shared_db::entity::{oidc_provider, user_oidc_link, user_role};
use uptrakit_shared_types::MaskedEmail;

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
        None => return error_response(StatusCode::BAD_REQUEST, "Missing Host header"),
    };

    let redirect_url = match RedirectUrl::new(format!("{base_url}/api/v1/auth/oidc/callback")) {
        Ok(url) => url,
        Err(e) => {
            tracing::error!("Invalid OIDC redirect URL: {e}");
            return error_response(StatusCode::BAD_REQUEST, "Invalid redirect URL");
        }
    };

    let provider =
        match find_active_provider(state.db(), state.default_tenant_id, provider_id).await {
            Some(p) => p,
            None => return error_response(StatusCode::NOT_FOUND, "Provider not found or inactive"),
        };

    // Build OIDC client via discovery
    let client = match build_oidc_client(&provider, redirect_url).await {
        Some(c) => c,
        None => return error_response(StatusCode::BAD_GATEWAY, "OIDC provider unavailable"),
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
        .oidc_flow_store
        .insert(
            csrf_state.secret().clone(),
            provider_id,
            &pkce_verifier,
            &nonce,
        )
        .await
    {
        tracing::error!("Failed to store OIDC flow: {e:?}");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
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
        match find_active_provider(state.db(), state.default_tenant_id, flow.provider_id).await {
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

    // Build OIDC client via discovery
    let client = match build_oidc_client(&provider, redirect_url.clone()).await {
        Some(c) => c,
        None => return Redirect::to("/login?error=oidc_discovery_failed").into_response(),
    };

    // Exchange code for tokens and extract claims
    let ExtractedOidcClaims {
        sub,
        email,
        email_verified,
        first_name,
        last_name,
        additional_claims,
    } = match exchange_code_for_claims(
        &client,
        code,
        flow.pkce_verifier,
        flow.nonce,
        redirect_url,
    )
    .await
    {
        Ok(c) => c,
        Err(response) => return response,
    };

    // Pre-check: if registration mode is Invite and auto_create is enabled,
    // check whether this would create a new user requiring a registration token.
    let reg_settings = state.settings.registration();
    if reg_settings.mode == RegistrationMode::Invite && provider.auto_create_users {
        // Check if an OIDC link already exists for this subject
        let has_link = UserOidcLink::find()
            .filter(user_oidc_link::Column::ProviderId.eq(flow.provider_id))
            .filter(user_oidc_link::Column::OidcSubject.eq(&sub))
            .count(state.db())
            .await
            .unwrap_or(1)
            > 0;

        if !has_link {
            // Check if a user with this email already exists
            let has_user = User::find()
                .filter(uptrakit_shared_db::entity::user::Column::Email.eq(&email))
                .count(state.db())
                .await
                .unwrap_or(1)
                > 0;

            if !has_user {
                // This would be a brand-new user — check if token is required
                let is_first_user = User::find()
                    .count(state.db())
                    .await
                    .map(|c| c == 0)
                    .unwrap_or(false);

                if reg_settings.needs_token_for_oidc(is_first_user) {
                    // Store pending registration and redirect to token input form
                    let mapped_roles = extract_mapped_roles(&provider, &additional_claims);
                    let code =
                        generate_secure_token().unwrap_or_else(|_| generate_uuid().to_string());

                    if let Err(e) = state
                        .oidc_registration_store
                        .insert(crate::auth::oidc_state::PendingOidcRegistrationParams {
                            registration_code: code.clone(),
                            provider_id: flow.provider_id,
                            oidc_subject: sub.clone(),
                            email: email.clone(),
                            first_name: first_name.clone(),
                            last_name: last_name.clone(),
                            mapped_roles,
                        })
                        .await
                    {
                        tracing::error!("Failed to store pending OIDC registration: {e:?}");
                        return Redirect::to("/login?error=oidc_internal_error").into_response();
                    }

                    return Redirect::to(&format!(
                        "/login?registration_token_required=true&registration_code={code}"
                    ))
                    .into_response();
                }
            }
        }
    }

    // Resolve user inside a transaction to prevent the race where two concurrent
    // OIDC callbacks both see user_count == 1 and both get the owner role.
    let txn = match state.db().begin().await {
        Ok(txn) => txn,
        Err(e) => {
            tracing::error!("Failed to start OIDC callback transaction: {e}");
            return Redirect::to("/login?error=oidc_internal_error").into_response();
        }
    };

    let resolution = match resolve_oidc_user(OidcUserParams {
        db: &txn,
        tenant_id: state.default_tenant_id,
        provider_id: flow.provider_id,
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
            tracing::error!("OIDC user resolution failed: {e:?}");
            return Redirect::to("/login?error=oidc_internal_error").into_response();
        }
    };

    match resolution {
        OidcUserResolution::LinkedUser(user_id) => {
            // Defense-in-depth: verify user is still active before creating session
            match User::find_by_id(user_id).one(&txn).await {
                Ok(Some(user)) if !user.is_active => {
                    return Redirect::to("/login?error=account_deactivated").into_response();
                }
                Ok(None) => {
                    return Redirect::to("/login?error=oidc_internal_error").into_response();
                }
                Err(e) => {
                    tracing::error!("failed to load user for OIDC login: {e:?}");
                    return Redirect::to("/login?error=oidc_internal_error").into_response();
                }
                _ => {}
            }

            // Sync roles and create session
            let _ = sync_oidc_roles(
                &txn,
                state.default_tenant_id,
                user_id,
                &provider,
                &additional_claims,
            )
            .await;

            if let Err(e) = txn.commit().await {
                tracing::error!("Failed to commit OIDC callback transaction: {e}");
                return Redirect::to("/login?error=oidc_internal_error").into_response();
            }
            create_oidc_exchange_and_redirect(&state, user_id, flow.provider_id).await
        }
        OidcUserResolution::NewUser(user_id) => {
            // Atomically check if this is the first user (threshold 1 because the
            // user was just created by resolve_oidc_user) and handle owner role +
            // initial setup inside the same transaction.
            let user_count = match User::find().count(&txn).await {
                Ok(n) => n,
                Err(e) => {
                    tracing::error!("Failed to count users during OIDC registration: {e}");
                    return Redirect::to("/login?error=oidc_internal_error").into_response();
                }
            };
            if user_count == 1 {
                // Delete the default 'user' role assigned by resolve_oidc_user
                let _ = UserRole::delete_many()
                    .filter(user_role::Column::TenantId.eq(state.default_tenant_id))
                    .filter(user_role::Column::UserId.eq(user_id))
                    .exec(&txn)
                    .await;

                // Assign owner role
                if let Err(e) =
                    super::auth::assign_owner_role(&txn, state.default_tenant_id, user_id).await
                {
                    tracing::error!("Failed to assign owner role to first OIDC user: {e:?}");
                }

                // Complete initial setup (close registration, remove token)
                let mut reg = state.settings.registration();
                if let Err(e) = reg
                    .complete_initial_setup(&txn, state.default_tenant_id)
                    .await
                {
                    tracing::error!("Failed to complete initial setup for first OIDC user: {e:?}");
                }
                state.settings.set_registration(reg).await;

                tracing::info!("first user registered via OIDC, assigned owner role");
            }

            // Sync roles
            let _ = sync_oidc_roles(
                &txn,
                state.default_tenant_id,
                user_id,
                &provider,
                &additional_claims,
            )
            .await;

            if let Err(e) = txn.commit().await {
                tracing::error!("Failed to commit OIDC callback transaction: {e}");
                return Redirect::to("/login?error=oidc_internal_error").into_response();
            }
            create_oidc_exchange_and_redirect(&state, user_id, flow.provider_id).await
        }
        OidcUserResolution::LinkViaPasswordRequired { user_id } => {
            // No DB writes needed in this branch, just drop the transaction
            drop(txn);

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
            let encoded_email =
                percent_encoding::utf8_percent_encode(&email, percent_encoding::NON_ALPHANUMERIC);
            // Suppress Referer on the link-token redirect so the token URL is not
            // forwarded to any third-party resource loaded by the login page.
            let mut link_headers = HeaderMap::new();
            link_headers.insert(
                header::REFERRER_POLICY,
                HeaderValue::from_static("no-referrer"),
            );
            (
                link_headers,
                Redirect::to(&format!(
                    "/login?link_required=true&link_token={link_token}&email={encoded_email}"
                )),
            )
                .into_response()
        }
        OidcUserResolution::LinkViaOidcRequired {
            user_id,
            existing_provider_id,
        } => {
            // No DB writes needed in this branch, just drop the transaction
            drop(txn);

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
            let encoded_email =
                percent_encoding::utf8_percent_encode(&email, percent_encoding::NON_ALPHANUMERIC);
            // Suppress Referer on the link-token redirect so the token URL is not
            // forwarded to any third-party resource loaded by the login page.
            let mut link_headers = HeaderMap::new();
            link_headers.insert(
                header::REFERRER_POLICY,
                HeaderValue::from_static("no-referrer"),
            );
            (
                link_headers,
                Redirect::to(&format!(
                    "/login?link_required=true&link_token={link_token}&email={encoded_email}&link_provider_id={existing_provider_id}"
                )),
            )
                .into_response()
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
            return error_response(StatusCode::BAD_REQUEST, "Invalid or expired exchange code");
        }
        Err(e) => {
            tracing::error!("Failed to retrieve OIDC exchange: {e:?}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    mint_oidc_auth_response(&state, pending.user_id, pending.provider_id).await
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
pub async fn oidc_complete_registration(
    State(state): State<Arc<AppState>>,
    Json(req): Json<OidcCompleteRegistrationRequest>,
) -> Response {
    // 1. Atomically consume the pending registration so the code is one-time use
    let pending = match state
        .oidc_registration_store
        .take(req.registration_code.expose_secret())
        .await
    {
        Ok(Some(p)) => p,
        Ok(None) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "Invalid or expired registration code",
            );
        }
        Err(e) => {
            tracing::error!("Failed to consume pending OIDC registration: {e:?}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // 2. Validate the registration token after consuming the entry
    let reg_settings = state.settings.registration();
    if let Err(e) = reg_settings.validate(Some(req.registration_token.expose_secret())) {
        return e.into_response();
    }

    // 3. Wrap user creation + first-user check + role assignment in a transaction
    // to prevent the race where two concurrent registrations both see count == 0.
    let txn = match state.db().begin().await {
        Ok(txn) => txn,
        Err(e) => {
            tracing::error!("Failed to start OIDC complete-registration transaction: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // 4. Race condition guard: verify user still doesn't exist
    let user_exists = User::find()
        .filter(uptrakit_shared_db::entity::user::Column::Email.eq(&pending.email))
        .count(&txn)
        .await
        .unwrap_or(1)
        > 0;

    if user_exists {
        return error_response(
            StatusCode::CONFLICT,
            "A user with this email already exists",
        );
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
        tracing::error!("Failed to create user during OIDC registration: {e}");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
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
        tracing::error!("Failed to create OIDC link during registration: {e}");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
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
            tracing::error!(
                "Failed to handle first-user setup for OIDC complete-registration: {e:?}"
            );
            false
        }
    };

    if is_first_user {
        tracing::info!("first user registered via OIDC complete-registration, assigned owner role");
    } else {
        // Assign default user role
        if let Err(e) = super::auth::assign_user_role(&txn, state.default_tenant_id, user_id).await
        {
            tracing::error!("Failed to assign default role during OIDC registration: {e:?}");
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
        tracing::error!("Failed to commit OIDC complete-registration transaction: {e}");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    // 9. Create session + JWT
    mint_oidc_auth_response(&state, user_id, pending.provider_id).await
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
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid request body"),
    };
    let link_req: OidcLinkRequest = match serde_json::from_slice(&bytes) {
        Ok(r) => r,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid JSON"),
    };

    // Retrieve pending link from database
    let pending = match state
        .account_link_store
        .take(link_req.link_token.expose_secret())
        .await
    {
        Ok(Some(p)) => p,
        Ok(None) => {
            return error_response(StatusCode::BAD_REQUEST, "Link token not found or expired");
        }
        Err(e) => {
            tracing::error!("Failed to retrieve pending link: {e:?}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // Verify ownership
    let verified = if let Some(ref pwd) = link_req.password {
        if let Some(message) = password::validate_password_length(pwd.expose_secret()) {
            return error_response(StatusCode::BAD_REQUEST, message);
        }
        // Password verification
        let user = match User::find_by_id(pending.user_id).one(state.db()).await {
            Ok(Some(u)) => u,
            _ => return error_response(StatusCode::UNAUTHORIZED, "User not found"),
        };
        let hash = match user.password_hash.as_ref() {
            Some(h) => h,
            None => return error_response(StatusCode::UNAUTHORIZED, "User has no password"),
        };
        matches!(
            password::verify_password(pwd.expose_secret(), hash.expose_secret()),
            Ok(true)
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
        return error_response(StatusCode::UNAUTHORIZED, "Verification failed");
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
        tracing::error!("Failed to create OIDC link: {e}");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
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

    mint_oidc_auth_response(&state, pending.user_id, pending.provider_id).await
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
) -> Option<DiscoveredCoreClient> {
    let issuer_url = IssuerUrl::new(provider.issuer_url.clone())
        .map_err(|e| tracing::error!("Invalid OIDC issuer URL for provider {}: {e}", provider.id))
        .ok()?;
    let http_client = reqwest::Client::default();
    let provider_metadata = CoreProviderMetadata::discover_async(issuer_url, &http_client)
        .await
        .map_err(|e| {
            tracing::error!(
                "OIDC provider discovery failed for provider {}: {e}",
                provider.id
            );
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
) -> Result<ExtractedOidcClaims, Response> {
    let http_client = reqwest::Client::default();
    let token_request = client
        .exchange_code(AuthorizationCode::new(code))
        .map_err(|e| {
            tracing::error!("OIDC token endpoint not configured: {e}");
            Redirect::to("/login?error=oidc_token_exchange_failed").into_response()
        })?;
    let token_response = token_request
        .set_redirect_uri(std::borrow::Cow::Owned(redirect_url))
        .set_pkce_verifier(pkce_verifier)
        .request_async(&http_client)
        .await
        .map_err(|e| {
            tracing::error!("OIDC token exchange failed: {e}");
            Redirect::to("/login?error=oidc_token_exchange_failed").into_response()
        })?;

    let id_token = token_response
        .id_token()
        .ok_or_else(|| Redirect::to("/login?error=oidc_no_id_token").into_response())?;

    let id_token_verifier = client.id_token_verifier();
    let claims = id_token
        .claims(&id_token_verifier, &nonce)
        .map_err(|e| {
            tracing::error!("OIDC ID token validation failed: {e}");
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

    let additional_claims =
        serde_json::to_value(claims.additional_claims()).unwrap_or_default();

    Ok(ExtractedOidcClaims {
        sub,
        email,
        email_verified,
        first_name,
        last_name,
        additional_claims,
    })
}

/// Build a synthetic `serde_json::Value` that re-maps stored `mapped_roles`
/// back to the provider's original role-claim keys, suitable for passing to
/// [`sync_oidc_roles`].
///
/// This is needed in flows where the original OIDC token is no longer
/// available (e.g., deferred registration completion or account linking).
fn build_fake_claims_for_sync(
    provider: &oidc_provider::Model,
    mapped_roles: &[String],
) -> serde_json::Value {
    let mut fake_claims = serde_json::Map::new();
    if let Some(ref path) = provider.role_claim_path {
        let reverse_mapped: Vec<String> = mapped_roles
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
    serde_json::Value::Object(fake_claims)
}

/// Create an OIDC refresh token, access token, and return a complete
/// [`AuthResponse`].
///
/// This is the shared session-creation step used by [`oidc_exchange`],
/// [`oidc_complete_registration`], and [`oidc_link`] after any provider-
/// specific work (user creation, linking, role sync) has been committed.
async fn mint_oidc_auth_response(state: &AppState, user_id: Uuid, provider_id: Uuid) -> Response {
    let session_service = SessionService::new(state.db().clone());
    let refresh_token = match session_service
        .create_refresh_token(
            user_id,
            AuthMethod::Oidc { provider_id },
            None,
            None,
        )
        .await
    {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("Failed to create OIDC refresh token: {e:?}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let user = match User::find_by_id(user_id).one(state.db()).await {
        Ok(Some(u)) => u,
        _ => return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error"),
    };

    let permissions =
        super::auth::get_user_permissions(state.db(), state.default_tenant_id, user_id)
            .await
            .unwrap_or_default();

    let access_token = match state
        .jwt
        .create_access_token(user_id, &permissions, "oidc", Some(provider_id))
    {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("Failed to create OIDC access token: {e:?}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let cookie = set_refresh_token_cookie(&refresh_token);
    let response = AuthResponse {
        access_token: SecretString::new(access_token),
        refresh_token: SecretString::new(refresh_token),
        expires_in: state.jwt.expires_in(),
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

async fn find_active_provider(
    db: &impl ConnectionTrait,
    tenant_id: uuid::Uuid,
    id: uuid::Uuid,
) -> Option<oidc_provider::Model> {
    OidcProvider::find_by_id(id)
        .filter(oidc_provider::Column::TenantId.eq(tenant_id))
        .filter(oidc_provider::Column::IsActive.eq(true))
        .filter(oidc_provider::Column::DeactivatedAt.is_null())
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
