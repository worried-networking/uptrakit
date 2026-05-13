//! Route handlers for MFA verification and email OTP dispatch.
//!
//! Endpoints:
//! - `POST /api/v1/auth/mfa/verify` — verify TOTP, email OTP, or recovery code
//! - `POST /api/v1/auth/mfa/email`  — generate and send an email OTP for the challenge
#![expect(
    clippy::let_underscore_must_use,
    reason = "fire-and-forget cleanup sends on error paths intentionally drop results"
)]

use crate::AppState;
use crate::auth::AuthMethod;
use crate::auth::mfa_challenge::{
    consume_challenge, consume_recovery_code, find_matching_recovery_code, generate_email_otp,
    hash_email_otp, load_valid_challenge, record_failed_attempt, store_email_otp_hash,
    verify_email_otp,
};
use crate::auth::refresh_cookie::set_refresh_token_cookie;
use crate::auth::totp::verify_totp_code;
use crate::error_response::error_response;
use crate::extract::{SessionSvc, Validated};
use crate::middleware::require_auth::get_user_permissions;
use axum::{
    Json,
    extract::State,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter, Set,
    SqliteTransactionMode, TransactionOptions, TransactionTrait,
};
use std::sync::Arc;
use uptrakit_shared_db::entity::prelude::*;
use uptrakit_shared_db::entity::user_totp;
use uptrakit_web_api_types::SecretString;
use uptrakit_web_api_types::auth::AuthResponse;
pub use uptrakit_web_api_types::auth::UserResponse;
use uptrakit_web_api_types::mfa::{MfaEmailRequest, MfaMethod, MfaVerifyRequest};

// ── Audit helpers ────────────────────────────────────────────────────────────

fn emit_mfa_audit(
    state: &AppState,
    action: uptrakit_audit_log::RegisteredAuditAction,
    user_id: uuid::Uuid,
    outcome: uptrakit_audit_log::AuditOutcome,
    reason_code: Option<&str>,
    method: Option<&str>,
) {
    let mut details = serde_json::Map::new();
    if let Some(method) = method {
        details.insert("method".to_string(), serde_json::json!(method));
    }
    if let Some(reason_code) = reason_code {
        details.insert("reason_code".to_string(), serde_json::json!(reason_code));
    }

    let builder = uptrakit_audit_log::AuditEntry::builder(action)
        .tenant_scope(state.default_tenant_id)
        .actor(uptrakit_audit_log::AuditActorType::User, Some(user_id))
        .outcome(outcome)
        .target("user", user_id.to_string(), None)
        .details(serde_json::Value::Object(details));

    if let Ok(entry) = builder.build() {
        state.audit_emitter.emit_best_effort(entry);
    }
}

// ── build_full_session ───────────────────────────────────────────────────────

/// Build a full authenticated session for `user_id` after MFA is verified.
///
/// Loads permissions, creates a refresh token (password auth method), creates
/// an access JWT, sets the refresh-token cookie, and returns an [`AuthResponse`].
/// Returns an error response if any step fails.
pub(crate) async fn build_full_session(
    state: &AppState,
    session_svc: &SessionSvc,
    user: &uptrakit_shared_db::entity::user::Model,
) -> Result<Response, Response> {
    let permissions = match get_user_permissions(state.db(), state.default_tenant_id, user.id).await
    {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(
                "Failed to get user permissions during MFA session build: {:?}",
                e
            );
            vec![]
        }
    };

    let refresh_token = match session_svc
        .create_refresh_token(user.id, AuthMethod::Password, None, None)
        .await
    {
        Ok(token) => token,
        Err(e) => {
            tracing::error!("Failed to create refresh token after MFA: {:?}", e);
            return Err(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error",
            ));
        }
    };

    let access_token =
        match state
            .auth
            .jwt
            .create_access_token(user.id, &permissions, "password", None, None)
        {
            Ok(token) => token,
            Err(e) => {
                tracing::error!("Failed to create access token after MFA: {:?}", e);
                return Err(error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error",
                ));
            }
        };

    let cookie = set_refresh_token_cookie(&refresh_token);

    let has_pending_email_change = uptrakit_shared_db::entity::prelude::EmailChangeRequest::find()
        .filter(uptrakit_shared_db::entity::email_change_request::Column::UserId.eq(user.id))
        .filter(
            uptrakit_shared_db::entity::email_change_request::Column::ExpiresAt
                .gt(time::OffsetDateTime::now_utc()),
        )
        .one(state.db())
        .await
        .unwrap_or(None)
        .is_some();

    let response = AuthResponse {
        access_token: SecretString::new(access_token),
        refresh_token: SecretString::new(refresh_token),
        expires_in: state.auth.jwt.expires_in(),
        token_type: "Bearer".to_string(),
        user: UserResponse {
            id: user.id,
            email: user.email.expose_email().to_string(),
            first_name: user.first_name.clone(),
            last_name: user.last_name.clone(),
            permissions,
            has_pending_email_change,
        },
    };

    Ok((
        StatusCode::OK,
        [(header::SET_COOKIE, cookie)],
        Json(response),
    )
        .into_response())
}

// ── POST /api/v1/auth/mfa/verify ─────────────────────────────────────────────

/// Verify an MFA code (TOTP, email OTP, or recovery code) and issue a session.
///
/// The challenge identified by `mfa_token` must be unconsumed, unexpired, and
/// have remaining attempts. On success the challenge is consumed and a full
/// authenticated session is returned.
#[utoipa::path(
    post,
    path = "/api/v1/auth/mfa/verify",
    request_body = uptrakit_web_api_types::mfa::MfaVerifyRequest,
    responses(
        (status = 200, description = "MFA verified — session issued", body = crate::routes::auth::AuthResponse),
        (status = 401, description = "Invalid or expired MFA token / wrong code"),
        (status = 429, description = "Too many failed attempts")
    ),
    tag = "Authentication"
)]
#[tracing::instrument(skip_all)]
pub async fn mfa_verify(
    State(state): State<Arc<AppState>>,
    session_svc: SessionSvc,
    Validated(req): Validated<MfaVerifyRequest>,
) -> Response {
    // ── BEGIN IMMEDIATE transaction (read-then-write: load + update challenge) ──
    let txn = match state
        .db()
        .begin_with_options(TransactionOptions {
            sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate),
            ..Default::default()
        })
        .await
    {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("Failed to begin MFA verify transaction: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // Load and validate the challenge inside the transaction.
    let challenge = match load_valid_challenge(&txn, &req.mfa_token).await {
        Ok(c) => c,
        Err(e) => {
            let _ = txn.rollback().await;
            use crate::auth::AuthError;
            let (status, msg) = match e.current_context() {
                AuthError::MfaChallengeNotFound => {
                    (StatusCode::UNAUTHORIZED, "Invalid or expired MFA token")
                }
                AuthError::MfaChallengeExpired => {
                    (StatusCode::UNAUTHORIZED, "MFA token has expired")
                }
                AuthError::MfaChallengeExhausted => {
                    (StatusCode::UNAUTHORIZED, "Too many failed attempts")
                }
                _ => (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error"),
            };
            return error_response(status, msg);
        }
    };

    let user_id = challenge.user_id;

    // Verify the provided code against the selected method.
    let (verified, is_recovery) = match &req.method {
        MfaMethod::Totp => {
            // Load the active TOTP record for the user (inside the transaction).
            let totp_row = match UserTotp::find()
                .filter(user_totp::Column::UserId.eq(user_id))
                .filter(user_totp::Column::IsActive.eq(true))
                .one(&txn)
                .await
            {
                Ok(Some(row)) => row,
                Ok(None) => {
                    let _ = txn.rollback().await;
                    return error_response(StatusCode::UNAUTHORIZED, "TOTP not enrolled");
                }
                Err(e) => {
                    tracing::error!("Failed to load TOTP row: {e}");
                    let _ = txn.rollback().await;
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Internal server error",
                    );
                }
            };

            let secret = totp_row.secret.expose_secret().to_string();

            let step = verify_totp_code(&secret, &req.code);

            // Anti-replay: reject if the matched step was already used.
            let valid = match step {
                None => false,
                Some(step) => {
                    if let Some(last) = totp_row.last_used_step {
                        if step <= last {
                            false // replay
                        } else {
                            // Persist new last_used_step atomically.
                            let mut active = totp_row.clone().into_active_model();
                            active.last_used_step = Set(Some(step));
                            if let Err(e) = active.update(&txn).await {
                                tracing::error!("Failed to update last_used_step: {e}");
                                let _ = txn.rollback().await;
                                return error_response(
                                    StatusCode::INTERNAL_SERVER_ERROR,
                                    "Internal server error",
                                );
                            }
                            true
                        }
                    } else {
                        // First use — just record the step.
                        let mut active = totp_row.clone().into_active_model();
                        active.last_used_step = Set(Some(step));
                        if let Err(e) = active.update(&txn).await {
                            tracing::error!("Failed to update last_used_step: {e}");
                            let _ = txn.rollback().await;
                            return error_response(
                                StatusCode::INTERNAL_SERVER_ERROR,
                                "Internal server error",
                            );
                        }
                        true
                    }
                }
            };

            (valid, false)
        }

        MfaMethod::Email => {
            let hash = match challenge.email_code_hash.as_deref() {
                Some(h) => h.to_string(),
                None => {
                    let _ = txn.rollback().await;
                    return error_response(
                        StatusCode::UNAUTHORIZED,
                        "No email OTP issued for this challenge",
                    );
                }
            };

            // Argon2 is CPU-intensive — off the async executor.
            let code = req.code.clone();
            let valid =
                match tokio::task::spawn_blocking(move || verify_email_otp(&code, &hash)).await {
                    Ok(Ok(v)) => v,
                    Ok(Err(e)) => {
                        tracing::error!("Email OTP verify error: {:?}", e);
                        let _ = txn.rollback().await;
                        return error_response(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "Internal server error",
                        );
                    }
                    Err(e) => {
                        tracing::error!("spawn_blocking panicked: {:?}", e);
                        let _ = txn.rollback().await;
                        return error_response(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "Internal server error",
                        );
                    }
                };
            (valid, false)
        }

        MfaMethod::RecoveryCode => {
            // Load all unused recovery codes for the user.
            let codes = match UserRecoveryCode::find()
                .filter(uptrakit_shared_db::entity::user_recovery_code::Column::UserId.eq(user_id))
                .all(&txn)
                .await
            {
                Ok(c) => c,
                Err(e) => {
                    tracing::error!("Failed to load recovery codes: {e}");
                    let _ = txn.rollback().await;
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Internal server error",
                    );
                }
            };

            let plaintext = req.code.clone();
            let matched_id = match tokio::task::spawn_blocking(move || {
                find_matching_recovery_code(&codes, &plaintext)
            })
            .await
            {
                Ok(id) => id,
                Err(e) => {
                    tracing::error!(
                        "spawn_blocking panicked during recovery code check: {:?}",
                        e
                    );
                    let _ = txn.rollback().await;
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Internal server error",
                    );
                }
            };

            if let Some(code_id) = matched_id {
                // Consume (mark used) the recovery code atomically.
                if let Err(e) = consume_recovery_code(&txn, code_id).await {
                    tracing::error!("Failed to consume recovery code: {:?}", e);
                    let _ = txn.rollback().await;
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Internal server error",
                    );
                }
                (true, true)
            } else {
                (false, false)
            }
        }

        MfaMethod::Other(_) | _ => {
            tracing::warn!(method = %req.method, "unsupported MFA method in verify request");
            let _ = txn.rollback().await;
            return error_response(StatusCode::BAD_REQUEST, "Unsupported MFA method");
        }
    };

    if !verified {
        // Record the failed attempt — may exhaust the challenge.
        let exhausted = match record_failed_attempt(&txn, &challenge).await {
            Ok(ex) => ex,
            Err(e) => {
                tracing::error!("Failed to record failed MFA attempt: {:?}", e);
                let _ = txn.rollback().await;
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        };

        if let Err(e) = txn.commit().await {
            tracing::error!("Failed to commit MFA failure: {e}");
        }

        if exhausted {
            emit_mfa_audit(
                &state,
                uptrakit_audit_log::AuditActionType::AUTH_MFA_CHALLENGE_EXHAUSTED,
                user_id,
                uptrakit_audit_log::AuditOutcome::Denied,
                Some("too_many_attempts"),
                Some(req.method.as_str()),
            );
            return error_response(StatusCode::UNAUTHORIZED, "Too many failed attempts");
        }

        emit_mfa_audit(
            &state,
            uptrakit_audit_log::AuditActionType::AUTH_MFA_FAILED,
            user_id,
            uptrakit_audit_log::AuditOutcome::Denied,
            Some("invalid_code"),
            Some(req.method.as_str()),
        );
        return error_response(StatusCode::UNAUTHORIZED, "Invalid MFA code");
    }

    // Success — consume the challenge and commit.
    if let Err(e) = consume_challenge(&txn, &challenge).await {
        tracing::error!("Failed to consume MFA challenge: {:?}", e);
        let _ = txn.rollback().await;
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    if let Err(e) = txn.commit().await {
        tracing::error!("Failed to commit MFA success: {e}");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    // Load the user row (outside the transaction — challenge is already consumed).
    let user = match User::find_by_id(user_id).one(state.db()).await {
        Ok(Some(u)) => u,
        Ok(None) => {
            return error_response(StatusCode::UNAUTHORIZED, "User not found");
        }
        Err(e) => {
            tracing::error!("Failed to load user after MFA: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    if !user.is_active {
        return error_response(StatusCode::UNAUTHORIZED, "Invalid credentials");
    }

    // Emit audit events.
    if is_recovery {
        emit_mfa_audit(
            &state,
            uptrakit_audit_log::AuditActionType::AUTH_MFA_RECOVERY_USED,
            user_id,
            uptrakit_audit_log::AuditOutcome::Success,
            None,
            Some(req.method.as_str()),
        );
    }

    emit_mfa_audit(
        &state,
        uptrakit_audit_log::AuditActionType::AUTH_MFA_VERIFIED,
        user_id,
        uptrakit_audit_log::AuditOutcome::Success,
        None,
        Some(req.method.as_str()),
    );

    // Build and return the full session.
    match build_full_session(&state, &session_svc, &user).await {
        Ok(response) => response,
        Err(error_resp) => error_resp,
    }
}

// ── POST /api/v1/auth/mfa/email ───────────────────────────────────────────────

/// Generate and send an email OTP for the given MFA challenge.
///
/// Loads the challenge (read-only), generates a 6-digit OTP, hashes it with
/// Argon2id in `spawn_blocking`, stores the hash in the challenge row, and
/// sends the OTP via the transactional email plugin.
#[utoipa::path(
    post,
    path = "/api/v1/auth/mfa/email",
    request_body = uptrakit_web_api_types::mfa::MfaEmailRequest,
    responses(
        (status = 204, description = "Email OTP sent (or silently ignored if email delivery is unavailable)"),
        (status = 401, description = "Invalid or expired MFA token")
    ),
    tag = "Authentication"
)]
#[tracing::instrument(skip_all)]
pub async fn mfa_send_email(
    State(state): State<Arc<AppState>>,
    Validated(req): Validated<MfaEmailRequest>,
) -> Response {
    use crate::auth::token::hash_token;
    use uptrakit_plugin_infrastructure_registry::TransactionalEmailError;
    use uptrakit_shared_db::entity::mfa_challenge;

    // Load challenge (read-only — no transaction needed here; the subsequent
    // store_email_otp_hash write is a single UPDATE, not read-then-conditional-write).
    let token_hash = hash_token(&req.mfa_token);
    let now = time::OffsetDateTime::now_utc();

    let challenge = match MfaChallenge::find()
        .filter(mfa_challenge::Column::TokenHash.eq(&token_hash))
        .one(state.db())
        .await
    {
        Ok(Some(c)) => c,
        Ok(None) => {
            return error_response(StatusCode::UNAUTHORIZED, "Invalid or expired MFA token");
        }
        Err(e) => {
            tracing::error!("Failed to load MFA challenge for email send: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    if challenge.consumed_at.is_some() || now >= challenge.expires_at {
        return error_response(StatusCode::UNAUTHORIZED, "Invalid or expired MFA token");
    }

    // Load the user's email address.
    let user = match User::find_by_id(challenge.user_id).one(state.db()).await {
        Ok(Some(u)) => u,
        Ok(None) => {
            return error_response(StatusCode::UNAUTHORIZED, "User not found");
        }
        Err(e) => {
            tracing::error!("Failed to load user for MFA email send: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let email_addr = user.email.expose_email().to_string();

    // Generate OTP and hash it in spawn_blocking.
    let otp_code = generate_email_otp();
    let otp_for_hash = otp_code.clone();

    let otp_hash = match tokio::task::spawn_blocking(move || hash_email_otp(&otp_for_hash)).await {
        Ok(Ok(h)) => h,
        Ok(Err(e)) => {
            tracing::error!("Failed to hash email OTP: {:?}", e);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
        Err(e) => {
            tracing::error!("spawn_blocking panicked hashing OTP: {:?}", e);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // Persist the hash.
    if let Err(e) = store_email_otp_hash(state.db(), &challenge, otp_hash).await {
        tracing::error!("Failed to store email OTP hash: {:?}", e);
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    // Build and send the email.
    let tenant_db =
        uptrakit_web_api_queries::TenantDb::new(state.db().clone(), state.default_tenant_id);

    let body_plain = format!(
        "Your Uptrakit verification code is: {otp_code}\n\n\
        This code expires when your login session expires (5 minutes). \
        Do not share this code with anyone.",
    );
    let body_html = format!(
        "<p>Your Uptrakit verification code is: <strong>{otp_code}</strong></p>\
        <p>This code expires when your login session expires (5 minutes). \
        Do not share this code with anyone.</p>",
    );

    match state
        .plugin
        .plugin_ops
        .send_transactional_email(
            &tenant_db,
            &email_addr,
            "Your Uptrakit verification code",
            &body_plain,
            &body_html,
        )
        .await
    {
        Ok(()) => {}
        Err(TransactionalEmailError::NotConfigured) => {
            return error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "Email delivery not configured",
            );
        }
        Err(TransactionalEmailError::DeliveryFailed(_)) => {
            return error_response(StatusCode::SERVICE_UNAVAILABLE, "Email delivery failed");
        }
        Err(e) => {
            tracing::warn!(?e, "unhandled TransactionalEmailError sending MFA OTP");
            return error_response(StatusCode::SERVICE_UNAVAILABLE, "Email delivery failed");
        }
    }

    StatusCode::OK.into_response()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    #![expect(
        clippy::expect_used,
        reason = "test code: panics on failure are acceptable"
    )]
    #![expect(clippy::panic, reason = "test code: panics on failure are acceptable")]
    #![expect(
        clippy::assertions_on_result_states,
        reason = "test assertions use is_ok/is_err pattern"
    )]

    use super::*;
    use crate::ServiceCredentialSources;
    use crate::auth::mfa_challenge::create_mfa_challenge;
    use crate::auth::password;
    use crate::auth::session::SessionService;
    use crate::auth::totp::generate_totp_secret;
    use sea_orm::{ConnectOptions, Database, DatabaseConnection, EntityTrait};
    use time::OffsetDateTime;
    use uptrakit_shared_db::entity::{tenant, user};
    use uptrakit_shared_types::MaskedEmail;

    async fn setup_test_db() -> DatabaseConnection {
        let opt = ConnectOptions::new("sqlite::memory:".to_owned());
        let db = Database::connect(opt).await.expect("test db");
        uptrakit_shared_db::migration::run_migrations(&db)
            .await
            .expect("migrations");
        db
    }

    async fn insert_test_user(db: &DatabaseConnection) -> uuid::Uuid {
        let now = OffsetDateTime::now_utc();
        let hash = password::hash_password("correct-horse-battery-staple").expect("hash");
        let user_id = uuid::Uuid::now_v7();
        user::ActiveModel {
            id: Set(user_id),
            email: Set(MaskedEmail::new("mfa-test@example.com")),
            first_name: Set("MFA".to_string()),
            last_name: Set("Test".to_string()),
            password_hash: Set(Some(hash)),
            is_active: Set(true),
            deactivated_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(db)
        .await
        .expect("insert user");
        user_id
    }

    async fn test_state(db: DatabaseConnection) -> Arc<AppState> {
        use crate::auth::registration::{RegistrationMode, RegistrationSettings};
        use crate::cert_signer::{AgentCertSigner, CertSignerError, SignedCertBundle};
        use crate::settings::Settings;

        struct NoopCertSigner;
        #[async_trait::async_trait]
        impl AgentCertSigner for NoopCertSigner {
            async fn sign_agent_csr(
                &self,
                _: &str,
                _: &uuid::Uuid,
                _: time::Duration,
            ) -> std::result::Result<SignedCertBundle, rootcause::Report<CertSignerError>>
            {
                Err(rootcause::report!(CertSignerError::Signing(
                    "noop".to_string(),
                )))
            }

            fn active_ca_fingerprint(&self) -> String {
                "0".repeat(64)
            }
        }

        let ca_pem = "-----BEGIN CERTIFICATE-----\ntest\n-----END CERTIFICATE-----\n";
        let snapshot_data = crate::ca_snapshot::CaPublicSnapshot {
            active_cert_pem: ca_pem.to_string(),
            active_fingerprint: "0".repeat(64),
            previous_cert_pem: None,
            previous_fingerprint: None,
            trusted_cas: vec![crate::ca_snapshot::TrustedCaPublic {
                cert_pem: ca_pem.to_string(),
                fingerprint: "0".repeat(64),
                not_after: time::OffsetDateTime::now_utc() + time::Duration::days(365),
            }],
            trusted_ca_cns: Vec::new(),
            bundle_pem: ca_pem.to_string(),
            bundle_hash: "0".repeat(64),
            managed: true,
            active_not_after: time::OffsetDateTime::now_utc() + time::Duration::days(365),
            pki_addr: None,
        };
        let (_ca_tx, ca_rx) = tokio::sync::watch::channel(snapshot_data);
        let ca_key_store: crate::CaKeyStoreRef =
            Arc::new(tokio::sync::RwLock::new(crate::ca_snapshot::CaKeyStore {
                active_key_pem: zeroize::Zeroizing::new(String::new()),
                previous_key_pem: None,
                trusted_ca_keys: vec![],
            }));

        let rustls_cfg = {
            let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
            let key_pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
            let cert = rcgen::CertificateParams::new(vec!["localhost".into()])
                .unwrap()
                .self_signed(&key_pair)
                .unwrap();
            let server_config = rustls::ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(
                    vec![rustls::pki_types::CertificateDer::from(cert.der().to_vec())],
                    rustls::pki_types::PrivateKeyDer::try_from(key_pair.serialize_der()).unwrap(),
                )
                .unwrap();
            axum_server::tls_rustls::RustlsConfig::from_config(Arc::new(server_config))
        };

        let notification_service = crate::notification_service::NotificationService::new(
            crate::service_connections::ServiceConnectionRegistry::new(),
            uuid::Uuid::nil(),
        );

        let settings = Settings::new(
            RegistrationSettings {
                mode: RegistrationMode::Open,
                token_hash: None,
                require_token_for_oidc: false,
            },
            168,
        );

        let plugin_ops: Arc<dyn uptrakit_plugin_infrastructure_registry::PluginOps> = Arc::new(
            uptrakit_plugin_infrastructure_registry::build_catalog(
                &uptrakit_plugin_infrastructure_registry::CatalogConfig::default(),
                uptrakit_plugin_infrastructure_registry::InstancePluginStates::all_disabled(),
            )
            .expect("default catalog should build"),
        );

        let notification_dispatcher = crate::notifications::dispatcher::NotificationDispatcher::new(
            db.clone(),
            Arc::clone(&plugin_ops),
            "https://localhost".to_string(),
        );

        let default_tenant_id = tenant::Entity::find()
            .one(&db)
            .await
            .expect("query default tenant")
            .expect("default tenant row")
            .id;

        let (_, config_rx_for_auth) = uptrakit_config_reload::RuntimeConfigChannels::from_runtime(
            &uptrakit_config_reload::RuntimeConfig::default(),
        );

        Arc::new(AppState {
            db: crate::app_state::DbState::new(db.clone()),
            cert: crate::app_state::CertState {
                ca_snapshot: ca_rx,
                ca_key_store,
                revocation_notify: Arc::new(tokio::sync::Notify::const_new()),
                crl_pem_cache: Arc::new(parking_lot::RwLock::new(String::new())),
                ca_rotation_trigger: Arc::new(tokio::sync::Notify::const_new()),
            },
            auth: crate::app_state::AuthState::new(
                Arc::new(crate::auth::jwt::JwtManager::from_secret(
                    b"test-mfa-secret",
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
            default_tenant_id,
            settings,
            cert_signer: Arc::new(NoopCertSigner),
            service_connections: crate::service_connections::ServiceConnectionRegistry::new(),
            controller_id: uuid::Uuid::nil(),
            plugin: crate::app_state::PluginState::new(
                plugin_ops,
                Arc::new(crate::global_providers::GlobalProviders::new(db.clone())),
            ),
            credential_sources: ServiceCredentialSources::default(),
            shutdown_token: Default::default(),
            embedded_service_notifier: None,
            audit_log_filter: uptrakit_audit_log::AuditFilter::default(),
            audit_log_dispatcher: uptrakit_audit_log::AuditLogDispatcher::new(Arc::new(
                uptrakit_audit_log::DatabaseBackend::new(db.clone()),
            )),
            audit_emitter: uptrakit_audit_log::AuditEmitter::new(
                uptrakit_audit_log::AuditLogDispatcher::new(Arc::new(
                    uptrakit_audit_log::DatabaseBackend::new(db.clone()),
                )),
            ),
            surface_proxy_deps: crate::app_state::SurfaceProxyDeps::new(
                Arc::new(crate::surface_registry::SurfaceRegistry::new(
                    crate::surface_registry::SurfaceRegistryConfig::default(),
                )),
                Arc::new(crate::surface_proxy::SurfaceProxy::new()),
            ),
            config_test_proxy: Arc::new(crate::config_test_proxy::ConfigTestProxy::new()),
            workload_claim_registry: Arc::new(crate::workload_claims::WorkloadClaimRegistry::new()),
            server: crate::app_state::ServerState::new(
                std::path::PathBuf::from("/tmp/test-pki"),
                rustls_cfg,
            ),
            reject_dangerous_commands: false,
            #[cfg(feature = "interactive")]
            interactive_sessions: crate::interactive_sessions::InteractiveSessionRegistry::new(),
            update_dispatcher: Arc::new(uptrakit_controller_core::update::NoopUpdateDispatcher),
            instance_plugin_snapshot: Arc::new(arc_swap::ArcSwap::from_pointee(
                uptrakit_web_api_queries::instance_plugin_settings::InstancePluginSnapshot::empty(),
            )),
            coordinator_handle: {
                let (tx, _) = tokio::sync::mpsc::unbounded_channel();
                uptrakit_config_reload::ReloadCoordinator::new(vec![], tx).1
            },
            settings_version_cache: uptrakit_config_reload::SettingsVersionCache::new(),
            db_config_rx: config_rx_for_auth.db,
            network_config_rx: config_rx_for_auth.network,
            nats_config_rx: config_rx_for_auth.nats,
            tls_config_rx: config_rx_for_auth.tls,
            audit_config_rx: config_rx_for_auth.audit,
            log_config_rx: config_rx_for_auth.log,
            master_key_config_rx: config_rx_for_auth.master_key,
            embedded_services_config_rx: config_rx_for_auth.embedded_services,
            zeroconf_config_rx: config_rx_for_auth.zeroconf,
            oauth: crate::oauth::OAuthState::disabled(),
        })
    }

    // ── Helper: enroll TOTP for user ─────────────────────────────────────────

    async fn enroll_totp(db: &DatabaseConnection, user_id: uuid::Uuid) -> String {
        use uptrakit_crypto::EncryptedString;
        use uptrakit_shared_db::entity::user_totp;

        uptrakit_crypto::enable_plaintext_mode();

        let secret = generate_totp_secret();
        let enc_secret =
            EncryptedString::new(secret.clone(), "uptrakit:user_totp:secret").expect("encrypt");

        user_totp::ActiveModel {
            id: Set(uuid::Uuid::now_v7()),
            user_id: Set(user_id),
            secret: Set(enc_secret),
            is_active: Set(true),
            enrolled_at: Set(Some(OffsetDateTime::now_utc())),
            last_used_step: Set(None),
            created_at: Set(OffsetDateTime::now_utc()),
        }
        .insert(db)
        .await
        .expect("insert user_totp");

        secret
    }

    // ── mfa_verify: valid TOTP ───────────────────────────────────────────────

    #[tokio::test]
    async fn mfa_verify_valid_totp_returns_200_with_auth_response() {
        uptrakit_crypto::enable_plaintext_mode();

        let db = setup_test_db().await;
        let user_id = insert_test_user(&db).await;
        let state = test_state(db.clone()).await;

        let secret = enroll_totp(&db, user_id).await;
        let mfa_token = create_mfa_challenge(&db, user_id).await.expect("challenge");

        // Generate a current TOTP code.
        let code = crate::auth::totp::generate_totp_code(&secret)
            .expect("should generate current TOTP code");

        let session_svc = crate::extract::SessionSvc::new(SessionService::new(db.clone()));

        let response = mfa_verify(
            State(state),
            session_svc,
            Validated(MfaVerifyRequest {
                mfa_token,
                code,
                method: MfaMethod::Totp,
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
    }

    // ── mfa_verify: wrong TOTP code → 401, attempt_count incremented ─────────

    #[tokio::test]
    async fn mfa_verify_wrong_totp_returns_401_and_increments_attempt_count() {
        uptrakit_crypto::enable_plaintext_mode();

        let db = setup_test_db().await;
        let user_id = insert_test_user(&db).await;
        let state = test_state(db.clone()).await;

        let _secret = enroll_totp(&db, user_id).await;
        let mfa_token = create_mfa_challenge(&db, user_id).await.expect("challenge");

        let session_svc = crate::extract::SessionSvc::new(SessionService::new(db.clone()));

        let response = mfa_verify(
            State(state),
            session_svc,
            Validated(MfaVerifyRequest {
                mfa_token: mfa_token.clone(),
                code: "000000".to_string(),
                method: MfaMethod::Totp,
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        // Check attempt_count was incremented.
        use uptrakit_shared_db::entity::mfa_challenge;
        let token_hash = crate::auth::token::hash_token(&mfa_token);
        let row = MfaChallenge::find()
            .filter(mfa_challenge::Column::TokenHash.eq(&token_hash))
            .one(&db)
            .await
            .expect("query")
            .expect("row");
        assert_eq!(row.attempt_count, 1);
    }

    // ── mfa_verify: expired token → 401 ──────────────────────────────────────

    #[tokio::test]
    async fn mfa_verify_expired_token_returns_401() {
        uptrakit_crypto::enable_plaintext_mode();

        let db = setup_test_db().await;
        let user_id = insert_test_user(&db).await;
        let state = test_state(db.clone()).await;

        let _secret = enroll_totp(&db, user_id).await;

        // Create a challenge and immediately expire it.
        let mfa_token = create_mfa_challenge(&db, user_id).await.expect("challenge");

        // Force-expire by setting expires_at to the past.
        use sea_orm::EntityTrait;
        let token_hash = crate::auth::token::hash_token(&mfa_token);
        use uptrakit_shared_db::entity::mfa_challenge;
        let row = MfaChallenge::find()
            .filter(mfa_challenge::Column::TokenHash.eq(&token_hash))
            .one(&db)
            .await
            .expect("query")
            .expect("row");
        let mut active = row.clone().into_active_model();
        active.expires_at = Set(OffsetDateTime::now_utc() - time::Duration::seconds(10));
        active.update(&db).await.expect("expire challenge");

        let session_svc = crate::extract::SessionSvc::new(SessionService::new(db.clone()));

        let response = mfa_verify(
            State(state),
            session_svc,
            Validated(MfaVerifyRequest {
                mfa_token,
                code: "123456".to_string(),
                method: MfaMethod::Totp,
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    // ── mfa_verify: already consumed → 401 ───────────────────────────────────

    #[tokio::test]
    async fn mfa_verify_already_consumed_returns_401() {
        uptrakit_crypto::enable_plaintext_mode();

        let db = setup_test_db().await;
        let user_id = insert_test_user(&db).await;
        let state = test_state(db.clone()).await;

        let _secret = enroll_totp(&db, user_id).await;

        let mfa_token = create_mfa_challenge(&db, user_id).await.expect("challenge");

        // Mark the challenge as already consumed.
        let token_hash = crate::auth::token::hash_token(&mfa_token);
        use uptrakit_shared_db::entity::mfa_challenge;
        let row = MfaChallenge::find()
            .filter(mfa_challenge::Column::TokenHash.eq(&token_hash))
            .one(&db)
            .await
            .expect("query")
            .expect("row");
        let mut active = row.clone().into_active_model();
        active.consumed_at = Set(Some(OffsetDateTime::now_utc()));
        active.update(&db).await.expect("consume challenge");

        let session_svc = crate::extract::SessionSvc::new(SessionService::new(db.clone()));

        let response = mfa_verify(
            State(state),
            session_svc,
            Validated(MfaVerifyRequest {
                mfa_token,
                code: "123456".to_string(),
                method: MfaMethod::Totp,
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    // ── mfa_send_email: valid → 200, email_code_hash set ─────────────────────

    #[tokio::test]
    async fn mfa_send_email_sets_email_code_hash() {
        let db = setup_test_db().await;
        let user_id = insert_test_user(&db).await;
        let state = test_state(db.clone()).await;

        let mfa_token = create_mfa_challenge(&db, user_id).await.expect("challenge");

        let response = mfa_send_email(
            State(state),
            Validated(MfaEmailRequest {
                mfa_token: mfa_token.clone(),
            }),
        )
        .await;

        // With no email plugin configured this will return 503 — but the hash
        // must have been written to the DB before sending was attempted.
        // We accept 200 (if somehow configured) or 503 (no plugin).
        assert!(
            response.status() == StatusCode::OK
                || response.status() == StatusCode::SERVICE_UNAVAILABLE
        );

        let token_hash = crate::auth::token::hash_token(&mfa_token);
        use uptrakit_shared_db::entity::mfa_challenge;
        let row = MfaChallenge::find()
            .filter(mfa_challenge::Column::TokenHash.eq(&token_hash))
            .one(&db)
            .await
            .expect("query")
            .expect("row");

        // The hash must be set regardless of whether the email was delivered.
        assert!(row.email_code_hash.is_some(), "email_code_hash must be set");
    }
}
