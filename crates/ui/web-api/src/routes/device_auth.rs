use std::convert::Infallible;
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde::Deserialize;
use uptrakit_shared_types::DeviceAuthStatus;
use uptrakit_web_api_types::SecretString;
use uptrakit_web_api_types::device_auth::{DeviceAuthAuthorizedSse, DeviceAuthExpiredSse};

use crate::AppState;
use crate::api_error::ApiError;
use crate::auth::api_token::ApiTokenService;
use crate::auth::device_flow::DeviceFlowStatus;
use crate::auth::token::hash_token;
use crate::device_flow_broadcaster::DeviceFlowEvent;
use crate::error_response::error_response;
use crate::extract::ApiTokenSvc;
use crate::middleware::permission::CanViewServices;
use crate::middleware::require_auth::{
    AuthenticatedApiTokenId, AuthenticatedUser, authenticated_user_audit_actor,
};

pub use uptrakit_web_api_types::device_auth::{
    DeviceAuthApproveRequest, DeviceAuthApproveResponse, DeviceAuthPollRequest,
    DeviceAuthPollResponse, DeviceAuthStartRequest, DeviceAuthStartResponse,
};

fn emit_device_auth_decision_audit(
    state: &AppState,
    user: &AuthenticatedUser,
    api_token_id: Option<AuthenticatedApiTokenId>,
    action_type: &'static str,
    device_flow_id: String,
    outcome: uptrakit_audit_log::AuditOutcome,
    details: serde_json::Value,
) {
    let (actor_type, actor_id) = authenticated_user_audit_actor(user, api_token_id);

    let entry = uptrakit_audit_log::AuditEntry::builder(action_type)
        .tenant_scope(state.default_tenant_id)
        .actor(actor_type, actor_id)
        .target("device_flow", device_flow_id, None)
        .outcome(outcome)
        .details(details)
        .build();

    if let Ok(entry) = entry {
        state.audit_emitter.emit_best_effort(entry);
    }
}

fn emit_device_auth_system_audit(
    state: &AppState,
    action_type: uptrakit_audit_log::AuditActionType,
    device_flow_id: Option<String>,
    outcome: uptrakit_audit_log::AuditOutcome,
    details: serde_json::Value,
) {
    let mut builder = uptrakit_audit_log::AuditEntry::builder(action_type)
        .tenant_scope(state.default_tenant_id)
        .actor_system()
        .outcome(outcome)
        .details(details);

    if let Some(device_flow_id) = device_flow_id {
        builder = builder.target("device_flow", device_flow_id, None);
    }

    if let Ok(entry) = builder.build() {
        state.audit_emitter.emit_best_effort(entry);
    }
}

fn classify_device_auth_poll_status_error(
    error: &rootcause::Report<crate::auth::device_flow::DeviceFlowError>,
) -> (uptrakit_audit_log::AuditOutcome, &'static str) {
    match error.current_context() {
        crate::auth::device_flow::DeviceFlowError::NotFound
        | crate::auth::device_flow::DeviceFlowError::AlreadyAuthorized => (
            uptrakit_audit_log::AuditOutcome::Denied,
            "device_flow_not_found",
        ),
        crate::auth::device_flow::DeviceFlowError::TokenGeneration(_)
        | crate::auth::device_flow::DeviceFlowError::Database(_) => (
            uptrakit_audit_log::AuditOutcome::Failed,
            "device_flow_status_lookup_failed",
        ),
    }
}

fn classify_device_auth_poll_consume_error(
    error: &rootcause::Report<crate::auth::device_flow::DeviceFlowError>,
) -> (uptrakit_audit_log::AuditOutcome, &'static str) {
    match error.current_context() {
        crate::auth::device_flow::DeviceFlowError::NotFound
        | crate::auth::device_flow::DeviceFlowError::AlreadyAuthorized => (
            uptrakit_audit_log::AuditOutcome::Denied,
            "device_flow_not_found",
        ),
        crate::auth::device_flow::DeviceFlowError::TokenGeneration(_)
        | crate::auth::device_flow::DeviceFlowError::Database(_) => (
            uptrakit_audit_log::AuditOutcome::Failed,
            "device_flow_consume_failed",
        ),
    }
}

fn classify_device_auth_approval_error(
    error: &rootcause::Report<crate::auth::device_flow::DeviceFlowError>,
) -> (&'static str, uptrakit_audit_log::AuditOutcome, &'static str) {
    match error.current_context() {
        crate::auth::device_flow::DeviceFlowError::NotFound => (
            uptrakit_audit_log::AuditActionType::AUTH_DEVICE_DENY,
            uptrakit_audit_log::AuditOutcome::Denied,
            "device_flow_not_found",
        ),
        crate::auth::device_flow::DeviceFlowError::AlreadyAuthorized => (
            uptrakit_audit_log::AuditActionType::AUTH_DEVICE_DENY,
            uptrakit_audit_log::AuditOutcome::Denied,
            "device_flow_already_authorized",
        ),
        crate::auth::device_flow::DeviceFlowError::TokenGeneration(_) => (
            uptrakit_audit_log::AuditActionType::AUTH_DEVICE_APPROVE,
            uptrakit_audit_log::AuditOutcome::Failed,
            "device_flow_token_generation_error",
        ),
        crate::auth::device_flow::DeviceFlowError::Database(_) => (
            uptrakit_audit_log::AuditActionType::AUTH_DEVICE_APPROVE,
            uptrakit_audit_log::AuditOutcome::Failed,
            "device_flow_database_error",
        ),
    }
}

/// Start a device authorization flow
#[utoipa::path(
    post,
    path = "/api/v1/auth/device",
    request_body = DeviceAuthStartRequest,
    responses(
        (status = 200, description = "Device flow started", body = DeviceAuthStartResponse),
        (status = 500, description = "Internal error")
    ),
    tag = "Authentication"
)]
#[tracing::instrument(skip_all)]
pub async fn device_auth_start(
    State(state): State<Arc<AppState>>,
    external_base_url: Option<axum::Extension<crate::extract::ExternalBaseUrl>>,
    headers: HeaderMap,
    Json(req): Json<DeviceAuthStartRequest>,
) -> Response {
    let has_client_name = req.client_name.is_some();
    let (device_code, user_code) = match state.auth.device_flow_store.create(req.client_name).await
    {
        Ok(result) => result,
        Err(e) => {
            tracing::error!("Failed to create device flow: {e}");
            emit_device_auth_system_audit(
                &state,
                uptrakit_audit_log::AuditActionType::AUTH_DEVICE_START.into(),
                None,
                uptrakit_audit_log::AuditOutcome::Failed,
                serde_json::json!({ "reason_code": "device_flow_create_failed" }),
            );
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // Create a broadcaster channel for SSE subscribers.
    let device_code_hash = hash_token(&device_code);
    state
        .broadcast
        .device_flow_broadcaster
        .create_channel(&device_code_hash)
        .await;

    emit_device_auth_system_audit(
        &state,
        uptrakit_audit_log::AuditActionType::AUTH_DEVICE_START.into(),
        Some(device_code_hash.clone()),
        uptrakit_audit_log::AuditOutcome::Success,
        serde_json::json!({ "has_client_name": has_client_name }),
    );

    // Derive verification URL: prefer ExternalBaseUrl, then Origin, then Host
    let host = external_base_url
        .map(|axum::Extension(u)| u.0)
        .or_else(|| {
            headers
                .get("origin")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string())
        })
        .or_else(|| {
            headers
                .get("host")
                .and_then(|v| v.to_str().ok())
                .map(|h| format!("https://{h}"))
        })
        .unwrap_or_default();

    let verification_url = format!("{host}/device?code={user_code}");

    let response = DeviceAuthStartResponse {
        device_code,
        user_code,
        verification_url,
        expires_in: 600,
        interval: 5,
    };

    (StatusCode::OK, Json(response)).into_response()
}

/// Poll for device authorization status
#[utoipa::path(
    post,
    path = "/api/v1/auth/device/poll",
    request_body = DeviceAuthPollRequest,
    responses(
        (status = 200, description = "Device flow status", body = DeviceAuthPollResponse),
        (status = 404, description = "Device flow not found"),
        (status = 429, description = "Polling too fast")
    ),
    tag = "Authentication"
)]
#[tracing::instrument(skip_all)]
pub async fn device_auth_poll(
    State(state): State<Arc<AppState>>,
    api_token_svc: ApiTokenSvc,
    Json(req): Json<DeviceAuthPollRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let device_flow_id = hash_token(&req.device_code);
    let status = match state
        .auth
        .device_flow_store
        .get_status(&req.device_code)
        .await
    {
        Ok(status) => status,
        Err(error) => {
            let (outcome, reason_code) = classify_device_auth_poll_status_error(&error);
            emit_device_auth_system_audit(
                &state,
                uptrakit_audit_log::AuditActionType::AUTH_DEVICE_POLL.into(),
                Some(device_flow_id),
                outcome,
                serde_json::json!({ "reason_code": reason_code }),
            );
            return Err(error.into());
        }
    };

    match status {
        DeviceFlowStatus::Pending => Ok((
            StatusCode::OK,
            Json(DeviceAuthPollResponse {
                status: DeviceAuthStatus::Pending,
                token: None,
                token_name: None,
            }),
        )
            .into_response()),
        DeviceFlowStatus::Expired => Ok((
            StatusCode::OK,
            Json(DeviceAuthPollResponse {
                status: DeviceAuthStatus::Expired,
                token: None,
                token_name: None,
            }),
        )
            .into_response()),
        DeviceFlowStatus::Authorized { .. } => {
            // Consume the flow (one-time use)
            let (user_id, client_name) =
                match state.auth.device_flow_store.consume(&req.device_code).await {
                    Ok(result) => result,
                    Err(error) => {
                        let (outcome, reason_code) =
                            classify_device_auth_poll_consume_error(&error);
                        emit_device_auth_system_audit(
                            &state,
                            uptrakit_audit_log::AuditActionType::AUTH_DEVICE_POLL.into(),
                            Some(device_flow_id),
                            outcome,
                            serde_json::json!({ "reason_code": reason_code }),
                        );
                        return Err(error.into());
                    }
                };

            let token_name = client_name.unwrap_or_else(|| "cli-device-auth".into());

            // Create an API token for the user
            match api_token_svc.create_token(user_id, &token_name).await {
                Ok(created) => {
                    emit_device_auth_system_audit(
                        &state,
                        uptrakit_audit_log::AuditActionType::AUTH_DEVICE_POLL.into(),
                        Some(device_flow_id),
                        uptrakit_audit_log::AuditOutcome::Success,
                        serde_json::json!({}),
                    );
                    Ok((
                        StatusCode::OK,
                        Json(DeviceAuthPollResponse {
                            status: DeviceAuthStatus::Authorized,
                            token: Some(SecretString::new(created.plaintext_token)),
                            token_name: Some(token_name),
                        }),
                    )
                        .into_response())
                }
                Err(e) => {
                    tracing::error!("Failed to create API token for device flow: {e:?}");
                    emit_device_auth_system_audit(
                        &state,
                        uptrakit_audit_log::AuditActionType::AUTH_DEVICE_POLL.into(),
                        Some(device_flow_id),
                        uptrakit_audit_log::AuditOutcome::Failed,
                        serde_json::json!({ "reason_code": "api_token_create_failed" }),
                    );
                    Ok(error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Internal server error",
                    ))
                }
            }
        }
    }
}

/// Approve a device authorization (authenticated)
#[utoipa::path(
    post,
    path = "/api/v1/auth/device/approve",
    request_body = DeviceAuthApproveRequest,
    responses(
        (status = 200, description = "Device authorized", body = DeviceAuthApproveResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Device flow not found"),
        (status = 409, description = "Already authorized")
    ),
    tag = "Authentication",
    extensions(("x-required-permission" = json!("view_services"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn device_auth_approve(
    State(state): State<Arc<AppState>>,
    CanViewServices(auth_user): CanViewServices,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Json(req): Json<DeviceAuthApproveRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let api_token_id = api_token_id.map(|value| value.0);
    let normalized = req.user_code.replace('-', "").to_uppercase();
    let device_flow_id = hash_token(&normalized);

    if let Err(error) = state
        .auth
        .device_flow_store
        .approve(&normalized, auth_user.user_id)
        .await
    {
        let (action_type, outcome, reason_code) = classify_device_auth_approval_error(&error);
        emit_device_auth_decision_audit(
            &state,
            &auth_user,
            api_token_id,
            action_type,
            device_flow_id,
            outcome,
            serde_json::json!({ "reason_code": reason_code }),
        );
        return Err(error.into());
    }

    // Notify SSE subscribers that the flow was approved.
    if let Ok(hash) = state
        .auth
        .device_flow_store
        .get_device_code_hash_by_user_code(&normalized)
        .await
    {
        state
            .broadcast
            .device_flow_broadcaster
            .notify_status_changed(&hash)
            .await;
    }

    emit_device_auth_decision_audit(
        &state,
        &auth_user,
        api_token_id,
        uptrakit_audit_log::AuditActionType::AUTH_DEVICE_APPROVE,
        device_flow_id,
        uptrakit_audit_log::AuditOutcome::Success,
        serde_json::json!({}),
    );

    Ok((
        StatusCode::OK,
        Json(DeviceAuthApproveResponse {
            message: "Device authorized".into(),
        }),
    )
        .into_response())
}

/// Query parameters for the device auth SSE stream.
#[derive(Debug, Deserialize)]
pub struct DeviceAuthStreamQuery {
    pub device_code: String,
}

/// SSE stream for device authorization status.
///
/// Unauthenticated endpoint (same as poll). The CLI connects here instead of
/// polling and receives the token immediately when the user approves in the
/// browser.
///
/// # Events
///
/// - `authorized` — contains the API token and token name.
/// - `expired` — the device flow expired before approval.
#[tracing::instrument(skip_all)]
pub async fn device_auth_stream(
    State(state): State<Arc<AppState>>,
    api_token_svc: ApiTokenSvc,
    Query(query): Query<DeviceAuthStreamQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let device_code = query.device_code;
    let device_code_hash = hash_token(&device_code);
    let shutdown_token = state.shutdown_token.clone();

    // Subscribe to the broadcaster for live notifications.
    let broadcast_rx = state
        .broadcast
        .device_flow_broadcaster
        .subscribe(&device_code_hash)
        .await;

    // Check current status in the DB.
    let current_status = state
        .auth
        .device_flow_store
        .get_status(&device_code)
        .await?;

    let stream = async_stream::stream! {
        match current_status {
            DeviceFlowStatus::Authorized { .. } => {
                // Already authorized — consume and yield token immediately.
                if let Some(event) = consume_and_yield(&state, &api_token_svc, &device_code).await {
                    yield Ok::<_, Infallible>(event);
                }
                state.broadcast.device_flow_broadcaster.remove_channel(&device_code_hash).await;
                return;
            }
            DeviceFlowStatus::Expired => {
                let payload = DeviceAuthExpiredSse {
                    message: "Device flow expired".to_string(),
                };
                if let Ok(json) = serde_json::to_string(&payload) {
                    yield Ok::<_, Infallible>(Event::default().event("expired").data(json));
                }
                state.broadcast.device_flow_broadcaster.remove_channel(&device_code_hash).await;
                return;
            }
            DeviceFlowStatus::Pending => {
                // Fall through to wait on broadcast channel.
            }
        }

        // Wait for broadcast events.
        if let Some(mut rx) = broadcast_rx {
            let timeout = tokio::time::sleep(std::time::Duration::from_secs(600));
            tokio::pin!(timeout);

            loop {
                tokio::select! {
                    ev = rx.recv() => {
                        match ev {
                            Ok(DeviceFlowEvent::StatusChanged) => {
                                if let Some(event) = consume_and_yield(&state, &api_token_svc, &device_code).await {
                                    yield Ok::<_, Infallible>(event);
                                }
                                state.broadcast.device_flow_broadcaster.remove_channel(&device_code_hash).await;
                                return;
                            }
                            Ok(DeviceFlowEvent::Expired) => {
                                let payload = DeviceAuthExpiredSse {
                                    message: "Device flow expired".to_string(),
                                };
                                if let Ok(json) = serde_json::to_string(&payload) {
                                    yield Ok::<_, Infallible>(Event::default().event("expired").data(json));
                                }
                                state.broadcast.device_flow_broadcaster.remove_channel(&device_code_hash).await;
                                return;
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                                continue;
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                                return;
                            }
                        }
                    }
                    _ = &mut timeout => {
                        let payload = DeviceAuthExpiredSse {
                            message: "Device flow expired".to_string(),
                        };
                        if let Ok(json) = serde_json::to_string(&payload) {
                            yield Ok::<_, Infallible>(Event::default().event("expired").data(json));
                        }
                        state.broadcast.device_flow_broadcaster.remove_channel(&device_code_hash).await;
                        return;
                    }
                    _ = shutdown_token.cancelled() => {
                        state.broadcast.device_flow_broadcaster.remove_channel(&device_code_hash).await;
                        return;
                    }
                }
            }
        }
    };

    Ok(Sse::new(stream)
        .keep_alive(KeepAlive::default().interval(std::time::Duration::from_secs(15)))
        .into_response())
}

/// Consume the device flow and return an SSE `authorized` event with the token.
async fn consume_and_yield(
    state: &AppState,
    api_svc: &ApiTokenService,
    device_code: &str,
) -> Option<Event> {
    let (user_id, client_name) = match state.auth.device_flow_store.consume(device_code).await {
        Ok(result) => result,
        Err(e) => {
            tracing::error!("Device flow consume failed during SSE: {e}");
            return None;
        }
    };

    let token_name = client_name.unwrap_or_else(|| "cli-device-auth".into());
    match api_svc.create_token(user_id, &token_name).await {
        Ok(created) => {
            let payload = DeviceAuthAuthorizedSse {
                token: SecretString::new(created.plaintext_token),
                token_name,
            };
            serde_json::to_string(&payload)
                .ok()
                .map(|json| Event::default().event("authorized").data(json))
        }
        Err(e) => {
            tracing::error!("Failed to create API token during SSE: {e:?}");
            None
        }
    }
}

#[cfg(all(test, feature = "db-sqlite"))]
mod tests {
    use super::*;

    use sea_orm::{
        ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, Set,
    };
    use uptrakit_shared_db::entity::{audit_log, user};
    use uptrakit_shared_types::MaskedEmail;
    use uptrakit_web_api_types::device_auth::{DeviceAuthPollResponse, DeviceAuthStartResponse};

    async fn latest_tenant_audit_row_for_action(
        db: &sea_orm::DatabaseConnection,
        action_type: &str,
    ) -> audit_log::Model {
        for _ in 0..50 {
            if let Some(row) = audit_log::Entity::find()
                .filter(audit_log::Column::ActionType.eq(action_type))
                .order_by_desc(audit_log::Column::OccurredAt)
                .one(db)
                .await
                .expect("query audit rows by action")
            {
                return row;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        panic!("expected tenant audit row for action {action_type}");
    }

    async fn insert_user(db: &sea_orm::DatabaseConnection) -> uuid::Uuid {
        let now = time::OffsetDateTime::now_utc();
        let user = user::ActiveModel {
            id: Set(uuid::Uuid::now_v7()),
            email: Set(MaskedEmail::new("device-auth-test@example.com")),
            first_name: Set("Device".to_string()),
            last_name: Set("Tester".to_string()),
            password_hash: Set(None),
            is_active: Set(true),
            deactivated_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        };

        user.insert(db).await.expect("insert test user").id
    }

    #[tokio::test]
    async fn device_auth_start_success_writes_audit_event() {
        let app = crate::test_harness::TestApp::new().await;
        let client = app.client();

        let (status, _): (axum::http::StatusCode, DeviceAuthStartResponse) = client
            .post_json(
                "/api/v1/auth/device",
                &DeviceAuthStartRequest {
                    client_name: Some("upk-cli".to_string()),
                },
            )
            .send_json()
            .await;

        assert_eq!(status, StatusCode::OK);

        let row = latest_tenant_audit_row_for_action(
            &app.db,
            uptrakit_audit_log::AuditActionType::AUTH_DEVICE_START,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::System.as_str()
        );
        assert_eq!(row.target_type.as_deref(), Some("device_flow"));
        let details = row.details_json.expect("details");
        assert_eq!(details["has_client_name"], serde_json::json!(true));
    }

    #[tokio::test]
    async fn device_auth_start_failure_writes_failed_audit_event() {
        let app = crate::test_harness::TestApp::new().await;
        let client = app.client();

        app.db
            .execute_unprepared("DROP TABLE pending_device_flows")
            .await
            .expect("drop pending_device_flow table");

        let status = client
            .post_json(
                "/api/v1/auth/device",
                &DeviceAuthStartRequest { client_name: None },
            )
            .send_status()
            .await;

        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);

        let row = latest_tenant_audit_row_for_action(
            &app.db,
            uptrakit_audit_log::AuditActionType::AUTH_DEVICE_START,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Failed.as_str()
        );
        let details = row.details_json.expect("details");
        assert_eq!(
            details["reason_code"],
            serde_json::json!("device_flow_create_failed")
        );
    }

    #[tokio::test]
    async fn device_auth_poll_authorized_success_writes_audit_event() {
        let app = crate::test_harness::TestApp::new().await;
        let client = app.client();
        let user_id = insert_user(&app.db).await;

        let (device_code, user_code) = app
            .state
            .auth
            .device_flow_store
            .create(Some("upk-cli".to_string()))
            .await
            .expect("create device flow");
        app.state
            .auth
            .device_flow_store
            .approve(&user_code, user_id)
            .await
            .expect("approve device flow");

        let (status, body): (axum::http::StatusCode, DeviceAuthPollResponse) = client
            .post_json(
                "/api/v1/auth/device/poll",
                &DeviceAuthPollRequest { device_code },
            )
            .send_json()
            .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.status, DeviceAuthStatus::Authorized);
        assert!(body.token.is_some());

        let row = latest_tenant_audit_row_for_action(
            &app.db,
            uptrakit_audit_log::AuditActionType::AUTH_DEVICE_POLL,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::System.as_str()
        );
    }

    #[tokio::test]
    async fn device_auth_poll_not_found_writes_denied_audit_event() {
        let app = crate::test_harness::TestApp::new().await;
        let client = app.client();

        let status = client
            .post_json(
                "/api/v1/auth/device/poll",
                &DeviceAuthPollRequest {
                    device_code: "does-not-exist".to_string(),
                },
            )
            .send_status()
            .await;

        assert_eq!(status, StatusCode::NOT_FOUND);

        let row = latest_tenant_audit_row_for_action(
            &app.db,
            uptrakit_audit_log::AuditActionType::AUTH_DEVICE_POLL,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
        let details = row.details_json.expect("details");
        assert_eq!(
            details["reason_code"],
            serde_json::json!("device_flow_not_found")
        );
    }

    #[tokio::test]
    async fn device_auth_poll_api_token_create_failure_writes_failed_audit_event() {
        let app = crate::test_harness::TestApp::new().await;
        let client = app.client();

        let (device_code, user_code) = app
            .state
            .auth
            .device_flow_store
            .create(None)
            .await
            .expect("create device flow");
        app.state
            .auth
            .device_flow_store
            .approve(&user_code, uuid::Uuid::now_v7())
            .await
            .expect("approve device flow");

        let status = client
            .post_json(
                "/api/v1/auth/device/poll",
                &DeviceAuthPollRequest { device_code },
            )
            .send_status()
            .await;

        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);

        let row = latest_tenant_audit_row_for_action(
            &app.db,
            uptrakit_audit_log::AuditActionType::AUTH_DEVICE_POLL,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Failed.as_str()
        );
        let details = row.details_json.expect("details");
        assert_eq!(
            details["reason_code"],
            serde_json::json!("api_token_create_failed")
        );
    }
}
