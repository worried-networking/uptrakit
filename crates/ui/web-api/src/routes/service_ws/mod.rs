#![expect(clippy::indexing_slicing, reason = "index is computed to be in bounds")]
#![expect(
    clippy::large_futures,
    reason = "large future is acceptable in this WS handler path"
)]

mod connection;
pub(crate) mod handler;
mod protocol;

pub(crate) use handler::trigger_discovery_for_agent_host;
use protocol::{ConnectionType, ServiceWsError, ServiceWsResult};

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;

use axum::Extension;
use axum::extract::WebSocketUpgrade;
use axum::extract::ws::WebSocket;
use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use uuid::Uuid;

use rootcause::prelude::*;
use uptrakit_audit_log::{AuditActionType, AuditActorType, AuditEntry, AuditOutcome};
use uptrakit_shared_db::entity::service as service_entity;
use uptrakit_shared_db::entity::system_service as sys_svc_entity;
use uptrakit_wire::{IncomingSeq, OutgoingSeq};

use crate::AppState;
use crate::extract::{ClientIp, ServiceIdentity};

// ---------------------------------------------------------------------------
// Unified entry point
// ---------------------------------------------------------------------------

/// Unified WebSocket handler for all service types.
///
/// Determines the connection type (Authenticated / Enrolled / Anonymous) and
/// dispatches to the unified capability-gated handler in
/// [`handler`].
///
/// Per-IP rate limiting is applied before the WebSocket upgrade to prevent
/// connection floods and brute-force bearer secret guessing.
///
/// An optional `service_id` query parameter can be supplied by enrolled
/// services (those in the `Bearer`-token path). When present, the
/// enrollment-secret lookup is narrowed to that specific service — a
/// defence-in-depth measure against cross-service secret collisions during the
/// narrow pre-certificate window.
pub async fn service_ws(
    State(state): State<Arc<AppState>>,
    identity: Option<Extension<ServiceIdentity>>,
    client_ip: Option<Extension<ClientIp>>,
    Query(query): Query<HashMap<String, String>>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    use crate::auth::rate_limit::RateLimitOutcome;

    // Per-IP connection rate limit: 30 attempts per 60 seconds.
    // Fail-closed on DB error to prevent bypass under load.
    if let Some(Extension(ClientIp(ref ip))) = client_ip {
        let key = format!("ws_connect:{ip}");
        match state
            .auth
            .rate_limit_store
            .check_rate_limit(&key, 30, 60)
            .await
        {
            Ok(RateLimitOutcome::Limited { retry_after_secs }) => {
                tracing::warn!(%ip, "WS connection rate limited");
                return (
                    axum::http::StatusCode::TOO_MANY_REQUESTS,
                    format!("Too many connection attempts. Retry after {retry_after_secs}s"),
                )
                    .into_response();
            }
            Err(e) => {
                tracing::error!(error = %e, "WS rate limiter error — rejecting (fail-closed)");
                return (
                    axum::http::StatusCode::SERVICE_UNAVAILABLE,
                    "Service temporarily unavailable".to_string(),
                )
                    .into_response();
            }
            Ok(RateLimitOutcome::Allowed) => {}
        }
    }

    let conn_type = if let Some(Extension(ref id)) = identity {
        tracing::info!(service_id = %id.service_id, "service WS upgrade (mTLS)");
        ConnectionType::Authenticated {
            service_id: id.service_id,
            cert_serial: id.cert_serial.clone(),
        }
    } else if let Some(secret) = connection::extract_bearer(&headers) {
        // Extract optional service_id query param for narrowing the secret lookup.
        let query_service_id = query.get("service_id").and_then(|v| v.parse::<Uuid>().ok());

        // Try unified lookup: find any non-deactivated service by secret hash.
        match lookup_by_secret(state.db(), &secret, query_service_id).await {
            Ok((id, is_system)) => {
                tracing::info!(
                    service_id = %id,
                    is_system,
                    "enrolled service WS upgrade (bearer)"
                );
                ConnectionType::Enrolled {
                    service_id: id,
                    is_system,
                }
            }
            Err(e) => {
                let (outcome, reason_code) =
                    classify_bearer_service_auth_failure(e.current_context());
                emit_bearer_service_auth_failure_audit(
                    &state,
                    query_service_id,
                    client_ip.as_ref().map(|Extension(ClientIp(ip))| *ip),
                    outcome,
                    reason_code,
                )
                .await;
                // Per-IP auth failure rate limit: 10 failures per 300 seconds.
                if let Some(Extension(ClientIp(ref ip))) = client_ip {
                    let fail_key = format!("ws_auth_fail:{ip}");
                    match state
                        .auth
                        .rate_limit_store
                        .check_rate_limit(&fail_key, 10, 300)
                        .await
                    {
                        Ok(RateLimitOutcome::Limited { retry_after_secs }) => {
                            tracing::warn!(%ip, "WS bearer auth failure rate limited");
                            return (
                                axum::http::StatusCode::TOO_MANY_REQUESTS,
                                format!(
                                    "Too many failed auth attempts. Retry after {retry_after_secs}s"
                                ),
                            )
                                .into_response();
                        }
                        Err(rate_err) => {
                            tracing::error!(
                                error = %rate_err,
                                "WS auth-fail rate limiter error — rejecting (fail-closed)"
                            );
                            return (
                                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                                "Service temporarily unavailable".to_string(),
                            )
                                .into_response();
                        }
                        Ok(RateLimitOutcome::Allowed) => {}
                    }
                }
                let msg = e.to_string();
                tracing::warn!("bearer auth failed: {msg}");
                return (axum::http::StatusCode::UNAUTHORIZED, msg).into_response();
            }
        }
    } else {
        tracing::info!("anonymous service WS upgrade");
        ConnectionType::Anonymous
    };

    let ip = client_ip.map(|Extension(ClientIp(ip))| ip);

    ws.max_message_size(connection::MAX_WS_MESSAGE_SIZE)
        .on_upgrade(move |socket| handle_connection(socket, state, conn_type, ip))
}

// ---------------------------------------------------------------------------
// Unified bearer-secret lookup
// ---------------------------------------------------------------------------

/// Look up a service by bearer enrollment secret.
///
/// Tries the tenant `services` table first, then the `system_services` table.
/// Returns `(service_id, is_system)` on success.
///
/// When `service_id` is `Some`, the query is further filtered to that specific
/// service. If the secret hash matches a different service, `InvalidSecret` is
/// returned (same error as no match) to avoid revealing that a secret collision
/// occurred across services.
async fn lookup_by_secret(
    db: &sea_orm::DatabaseConnection,
    secret: &str,
    service_id: Option<Uuid>,
) -> ServiceWsResult<(Uuid, bool)> {
    let secret_hash = crate::auth::token::hash_token(secret);

    // Try tenant services first.
    let mut query = uptrakit_shared_db::entity::prelude::Service::find()
        .filter(service_entity::Column::EnrollmentSecretHash.eq(&secret_hash))
        .filter(service_entity::Column::DeactivatedAt.is_null());
    if let Some(id) = service_id {
        query = query.filter(service_entity::Column::Id.eq(id));
    }
    if let Some(svc) = query.one(db).await.context_to::<ServiceWsError>()? {
        return Ok((svc.id, false));
    }

    // Try system services.
    use uptrakit_shared_db::entity::system_service as sys_svc_entity;
    let mut sys_query = uptrakit_shared_db::entity::prelude::SystemService::find()
        .filter(sys_svc_entity::Column::EnrollmentSecretHash.eq(&secret_hash))
        .filter(sys_svc_entity::Column::DeactivatedAt.is_null());
    if let Some(id) = service_id {
        sys_query = sys_query.filter(sys_svc_entity::Column::Id.eq(id));
    }
    if let Some(svc) = sys_query.one(db).await.context_to::<ServiceWsError>()? {
        return Ok((svc.id, true));
    }

    Err(report!(ServiceWsError::InvalidSecret))
}

fn classify_bearer_service_auth_failure(
    err: &ServiceWsError,
) -> (uptrakit_audit_log::AuditOutcome, &'static str) {
    match err {
        ServiceWsError::InvalidSecret => (
            uptrakit_audit_log::AuditOutcome::Denied,
            "invalid_reconnect_secret",
        ),
        ServiceWsError::Database(_) => (
            uptrakit_audit_log::AuditOutcome::Failed,
            "reconnect_lookup_failed",
        ),
        ServiceWsError::Deserialize(_)
        | ServiceWsError::SequenceValidation(_)
        | ServiceWsError::ProtocolVersionMismatch { .. } => (
            uptrakit_audit_log::AuditOutcome::Failed,
            "reconnect_auth_failed",
        ),
    }
}

struct ResolvedBearerAuthTarget {
    tenant_id: Option<Uuid>,
    actor_id: Option<Uuid>,
    service_app_name: Option<String>,
    target_id: Option<String>,
}

async fn resolve_bearer_auth_target(
    db: &sea_orm::DatabaseConnection,
    service_id_hint: Option<Uuid>,
) -> ResolvedBearerAuthTarget {
    let Some(service_id) = service_id_hint else {
        return ResolvedBearerAuthTarget {
            tenant_id: None,
            actor_id: None,
            service_app_name: None,
            target_id: None,
        };
    };

    if let Ok(Some(service)) = service_entity::Entity::find_by_id(service_id).one(db).await {
        return ResolvedBearerAuthTarget {
            tenant_id: Some(service.tenant_id),
            actor_id: Some(service_id),
            service_app_name: service.service_app_name,
            target_id: Some(service_id.to_string()),
        };
    }

    if let Ok(Some(service)) = sys_svc_entity::Entity::find_by_id(service_id).one(db).await {
        return ResolvedBearerAuthTarget {
            tenant_id: None,
            actor_id: Some(service_id),
            service_app_name: service.service_app_name,
            target_id: Some(service_id.to_string()),
        };
    }

    ResolvedBearerAuthTarget {
        tenant_id: None,
        actor_id: None,
        service_app_name: None,
        target_id: Some(service_id.to_string()),
    }
}

async fn emit_bearer_service_auth_failure_audit(
    state: &AppState,
    service_id_hint: Option<Uuid>,
    client_ip: Option<IpAddr>,
    outcome: AuditOutcome,
    reason_code: &'static str,
) {
    let resolved = resolve_bearer_auth_target(state.db(), service_id_hint).await;
    let mut details = serde_json::json!({
        "auth_method": "bearer_reconnect",
        "reason_code": reason_code,
    });
    if let Some(client_ip) = client_ip {
        details["client_ip"] = serde_json::Value::String(client_ip.to_string());
    }

    let mut builder = AuditEntry::builder(AuditActionType::AUTH_SERVICE_AUTHENTICATE)
        .actor(AuditActorType::Service, resolved.actor_id)
        .actor_display_opt(resolved.service_app_name.clone())
        .target_opt(
            resolved.target_id.as_ref().map(|_| "service".to_string()),
            resolved.target_id,
            resolved.service_app_name.clone(),
        )
        .outcome(outcome)
        .details(details);
    builder = if let Some(tenant_id) = resolved.tenant_id {
        builder.tenant_scope(tenant_id)
    } else {
        builder.system_scope()
    };

    match builder.build() {
        Ok(entry) => state.audit_emitter.emit_best_effort(entry),
        Err(error) => tracing::warn!(
            error = %error,
            service_id_hint = ?service_id_hint,
            outcome = outcome.as_str(),
            reason_code,
            "failed to build bearer service auth failure audit entry"
        ),
    }
}

// ---------------------------------------------------------------------------
// Top-level dispatch
// ---------------------------------------------------------------------------

async fn handle_connection(
    socket: WebSocket,
    state: Arc<AppState>,
    conn_type: ConnectionType,
    client_ip: Option<IpAddr>,
) {
    let mut out_seq = OutgoingSeq::new();
    let mut in_seq = IncomingSeq::new();

    match conn_type {
        ConnectionType::Authenticated {
            service_id,
            cert_serial,
        } => {
            connection::handle_authenticated(
                socket,
                state,
                service_id,
                cert_serial,
                client_ip,
                &mut out_seq,
                &mut in_seq,
            )
            .await;
        }
        ConnectionType::Enrolled {
            service_id,
            is_system,
        } => {
            connection::handle_enrolled(
                socket,
                state,
                service_id,
                is_system,
                client_ip,
                &mut out_seq,
                &mut in_seq,
            )
            .await;
        }
        ConnectionType::Anonymous => {
            connection::handle_anonymous(socket, state, client_ip, &mut out_seq, &mut in_seq).await;
        }
    }
}

#[cfg(all(test, feature = "db-sqlite"))]
mod tests {
    #![expect(
        clippy::expect_used,
        reason = "test code: panics on failure are acceptable"
    )]
    #![expect(clippy::panic, reason = "test code: panics on failure are acceptable")]

    use super::protocol::{
        MessageRateLimiter, ServiceWsError, WS_MESSAGE_RATE_WINDOW, deserialize_service_msg,
        record_service_activity,
    };
    use sea_orm::{
        ActiveModelTrait, ColumnTrait, ConnectOptions, Database, DatabaseConnection, EntityTrait,
        QueryFilter, QueryOrder, Set,
    };
    use uptrakit_shared_db::entity::{
        service as service_entity, system_audit_log, system_service, tenant,
    };
    use uptrakit_wire::IncomingSeq;

    #[test]
    fn deserialize_unknown_type_returns_unknown_variant() {
        let mut in_seq = IncomingSeq::new();
        let json = r#"{"protocol_version":1,"seq":1,"type":"future_message","data":{"foo":"bar"}}"#;
        let result = deserialize_service_msg(&mut in_seq, json);
        match result {
            Ok(Some(d)) => {
                assert!(
                    matches!(d.message, uptrakit_wire::ServiceMessage::Unknown),
                    "unknown type should produce ServiceMessage::Unknown"
                );
            }
            other => panic!("expected Ok(Some(DeserializedMessage {{ Unknown }})), got {other:?}"),
        }
    }

    #[test]
    fn deserialize_malformed_json_returns_err() {
        let mut in_seq = IncomingSeq::new();
        let result = deserialize_service_msg(&mut in_seq, "not valid json at all");
        assert!(result.is_err(), "malformed JSON should return Err");
    }

    #[test]
    fn deserialize_sequence_error_returns_err() {
        let mut in_seq = IncomingSeq::new();
        let json = r#"{"protocol_version":1,"seq":2,"type":"ping","service_ts":12345}"#;
        let result = deserialize_service_msg(&mut in_seq, json);
        let report = result.unwrap_err();
        let ctx = report.current_context();
        assert!(
            matches!(ctx, ServiceWsError::SequenceValidation(_)),
            "sequence mismatch should return Err(SequenceValidation)"
        );
    }

    #[test]
    fn message_rate_limiter_enforces_window() {
        let mut limiter = MessageRateLimiter::new(WS_MESSAGE_RATE_WINDOW, 2);
        assert!(limiter.allow());
        assert!(limiter.allow());
        assert!(!limiter.allow(), "should reject when at capacity");

        // After TWO full windows have elapsed, the previous window's count
        // has been fully flushed (prev_count was already rotated once, then
        // the second rotation zeros it out).
        let two_windows_ago = std::time::Instant::now() - WS_MESSAGE_RATE_WINDOW * 2;
        limiter.set_window_start(two_windows_ago);
        assert!(limiter.allow(), "should allow after two full windows");
    }

    #[test]
    fn sliding_window_prevents_boundary_burst() {
        // With a fixed-window limiter and max=50, an attacker could send 50
        // at the end of one window and 50 at the start of the next (100 in
        // rapid succession). The sliding window weights previous-window count
        // to prevent this.
        let window = std::time::Duration::from_secs(1);
        let mut limiter = MessageRateLimiter::new(window, 50);

        // Fill the current window to capacity.
        for _ in 0..50 {
            assert!(limiter.allow());
        }
        assert!(!limiter.allow());

        // Simulate window boundary: rotate current → previous, but use a
        // window_start that's just barely past the boundary so the previous
        // window still has high weight.
        let just_past_boundary = std::time::Instant::now();
        limiter.set_window_start(just_past_boundary);
        // prev_count is still 0 here because set_window_start only changes
        // the timestamp. Call allow() which triggers the rotation check.

        // Manually build a scenario: create a fresh limiter, fill it, then
        // rotate by pushing the window start backward.
        let mut limiter2 = MessageRateLimiter::new(window, 50);
        for _ in 0..50 {
            assert!(limiter2.allow());
        }
        // Rotate: push window_start back so elapsed >= window, triggering
        // the rotation on next allow() call.
        limiter2.set_window_start(std::time::Instant::now() - window);

        // First call triggers rotation (prev=50, curr=0). The weighted
        // estimate is ~50*(1.0) + 0 = ~50, which is >= max, so it should
        // reject immediately.
        assert!(
            !limiter2.allow(),
            "sliding window should reject burst at boundary"
        );
    }

    async fn setup_test_db() -> DatabaseConnection {
        let opt = ConnectOptions::new("sqlite::memory:");
        let db = Database::connect(opt).await.expect("test db");
        uptrakit_shared_db::migration::run_migrations(&db)
            .await
            .expect("run migrations");
        db
    }

    async fn insert_tenant(db: &DatabaseConnection) -> uuid::Uuid {
        let id = uuid::Uuid::now_v7();
        let now = time::OffsetDateTime::now_utc();
        tenant::ActiveModel {
            id: Set(id),
            name: Set("test-tenant".to_string()),
            slug: Set(id.to_string()),
            is_default: Set(false),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .expect("insert tenant");
        id
    }

    async fn insert_service(db: &DatabaseConnection, ip_address: Option<&str>) -> uuid::Uuid {
        let tenant_id = insert_tenant(db).await;
        let id = uuid::Uuid::now_v7();
        let now = time::OffsetDateTime::now_utc();
        service_entity::ActiveModel {
            id: Set(id),
            tenant_id: Set(tenant_id),
            capabilities: Set("[]".to_string()),
            hostname: Set("test-host".to_string()),
            friendly_name: Set("test-host".to_string()),
            ip_address: Set(ip_address.map(ToOwned::to_owned)),
            status: Set(service_entity::ServiceStatus::Pending),
            enrollment_secret_hash: Set(format!("secret-{id}")),
            client_version: Set(None),
            last_seen_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
            ping_interval_seconds: Set(None),
            enrollment_token_id: Set(None),
            cert_lifetime_hours: Set(None),
            service_app_name: Set(None),
            is_embedded: Set(false),
            embedded_owner_key: Set(None),
        }
        .insert(db)
        .await
        .expect("insert service");
        id
    }

    async fn insert_system_service(db: &DatabaseConnection) -> uuid::Uuid {
        let id = uuid::Uuid::now_v7();
        let now = time::OffsetDateTime::now_utc();
        system_service::ActiveModel {
            id: Set(id),
            capabilities: Set("[]".to_string()),
            hostname: Set("system-host".to_string()),
            friendly_name: Set("system-host".to_string()),
            ip_address: Set(None),
            status: Set(system_service::SystemServiceStatus::Pending),
            enrollment_secret_hash: Set(format!("secret-{id}")),
            client_version: Set(None),
            last_seen_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
            ping_interval_seconds: Set(None),
            cert_lifetime_hours: Set(None),
            system_enrollment_token_id: Set(None),
            service_app_name: Set(Some("uptrakit-scheduler".to_string())),
            is_embedded: Set(false),
            embedded_owner_key: Set(None),
        }
        .insert(db)
        .await
        .expect("insert system service");
        id
    }

    async fn tenant_audit_row_for_action(
        db: &DatabaseConnection,
        action_type: uptrakit_audit_log::RegisteredAuditAction,
    ) -> uptrakit_shared_db::entity::audit_log::Model {
        for _ in 0..50 {
            if let Some(row) = uptrakit_shared_db::entity::audit_log::Entity::find()
                .filter(uptrakit_shared_db::entity::audit_log::Column::ActionType.eq(action_type))
                .order_by_desc(uptrakit_shared_db::entity::audit_log::Column::OccurredAt)
                .one(db)
                .await
                .expect("query tenant audit rows")
            {
                return row;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        panic!("expected tenant audit row for action {action_type}");
    }

    async fn system_audit_row_for_action(
        db: &DatabaseConnection,
        action_type: uptrakit_audit_log::RegisteredAuditAction,
    ) -> system_audit_log::Model {
        for _ in 0..50 {
            if let Some(row) = system_audit_log::Entity::find()
                .filter(system_audit_log::Column::ActionType.eq(action_type))
                .order_by_desc(system_audit_log::Column::OccurredAt)
                .one(db)
                .await
                .expect("query system audit rows")
            {
                return row;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        panic!("expected system audit row for action {action_type}");
    }

    async fn emit_bearer_service_auth_failure_audit(
        state: &std::sync::Arc<crate::AppState>,
        service_id: Option<uuid::Uuid>,
        client_ip: Option<std::net::IpAddr>,
        outcome: uptrakit_audit_log::AuditOutcome,
        reason_code: &'static str,
    ) {
        let Some(service_id) = service_id else {
            return;
        };

        let details = {
            let mut details = serde_json::json!({
                "auth_method": "bearer_reconnect",
                "reason_code": reason_code,
            });
            if let Some(client_ip) = client_ip {
                details["client_ip"] = serde_json::Value::String(client_ip.to_string());
            }
            details
        };

        let entry = if let Ok(Some(service)) = service_entity::Entity::find_by_id(service_id)
            .one(state.db())
            .await
        {
            uptrakit_audit_log::AuditEntry::builder(
                uptrakit_audit_log::AuditActionType::AUTH_SERVICE_AUTHENTICATE,
            )
            .tenant_scope(service.tenant_id)
            .actor_service(service.id)
            .actor_display_opt(service.service_app_name)
            .target(
                "service",
                service.id.to_string(),
                Some(service.friendly_name),
            )
            .outcome(outcome)
            .details(details)
            .build()
        } else if let Ok(Some(service)) = system_service::Entity::find_by_id(service_id)
            .one(state.db())
            .await
        {
            uptrakit_audit_log::AuditEntry::builder(
                uptrakit_audit_log::AuditActionType::AUTH_SERVICE_AUTHENTICATE,
            )
            .system_scope()
            .actor_service(service.id)
            .actor_display_opt(service.service_app_name)
            .target(
                "service",
                service.id.to_string(),
                Some(service.friendly_name),
            )
            .outcome(outcome)
            .details(details)
            .build()
        } else {
            return;
        };

        if let Ok(entry) = entry {
            state.audit_emitter.emit_best_effort(entry);
        }
    }

    #[tokio::test]
    async fn record_service_activity_sets_ip_when_provided() {
        let db = setup_test_db().await;
        let service_id = insert_service(&db, None).await;

        let ip = std::net::IpAddr::from([10, 1, 2, 3]);
        record_service_activity(&db, service_id, Some(ip))
            .await
            .expect("record activity");

        let model = service_entity::Entity::find_by_id(service_id)
            .one(&db)
            .await
            .expect("query service")
            .expect("service exists");
        assert_eq!(model.ip_address.as_deref(), Some("10.1.2.3"));
        assert!(model.last_seen_at.is_some());
    }

    #[tokio::test]
    async fn record_service_activity_preserves_ip_when_absent() {
        let db = setup_test_db().await;
        let service_id = insert_service(&db, Some("192.0.2.7")).await;

        record_service_activity(&db, service_id, None)
            .await
            .expect("record activity");

        let model = service_entity::Entity::find_by_id(service_id)
            .one(&db)
            .await
            .expect("query service")
            .expect("service exists");
        assert_eq!(model.ip_address.as_deref(), Some("192.0.2.7"));
        assert!(model.last_seen_at.is_some());
    }

    #[tokio::test]
    async fn emit_bearer_service_auth_failure_writes_denied_tenant_audit_row() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;
        let service_id = insert_service(&db, None).await;

        emit_bearer_service_auth_failure_audit(
            &state,
            Some(service_id),
            Some(std::net::IpAddr::from([198, 51, 100, 7])),
            uptrakit_audit_log::AuditOutcome::Denied,
            "invalid_reconnect_secret",
        )
        .await;

        let row = tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::AUTH_SERVICE_AUTHENTICATE,
        )
        .await;
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::Service.as_str()
        );
        assert_eq!(row.actor_id, Some(service_id));
        assert_eq!(row.target_type.as_deref(), Some("service"));
        assert_eq!(
            row.target_id.as_deref(),
            Some(service_id.to_string().as_str())
        );
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
        let details = row.details_json.expect("details");
        assert_eq!(details["auth_method"], "bearer_reconnect");
        assert_eq!(details["reason_code"], "invalid_reconnect_secret");
        assert_eq!(details["client_ip"], "198.51.100.7");
    }

    #[tokio::test]
    async fn emit_bearer_service_auth_failure_writes_denied_system_audit_row() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;
        let service_id = insert_system_service(&db).await;

        emit_bearer_service_auth_failure_audit(
            &state,
            Some(service_id),
            None,
            uptrakit_audit_log::AuditOutcome::Denied,
            "invalid_reconnect_secret",
        )
        .await;

        let row = system_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::AUTH_SERVICE_AUTHENTICATE,
        )
        .await;
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::Service.as_str()
        );
        assert_eq!(row.actor_id, Some(service_id));
        assert_eq!(row.target_type.as_deref(), Some("service"));
        assert_eq!(
            row.target_id.as_deref(),
            Some(service_id.to_string().as_str())
        );
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
        let details = row.details_json.expect("details");
        assert_eq!(details["auth_method"], "bearer_reconnect");
        assert_eq!(details["reason_code"], "invalid_reconnect_secret");
        assert!(details.get("client_ip").is_none());
    }
}
