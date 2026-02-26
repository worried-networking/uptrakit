mod connection;
pub(crate) mod handler;
mod protocol;

pub(crate) use handler::trigger_discovery_for_agent_host;
use protocol::{ConnectionType, ServiceWsError, ServiceWsResult};

use std::net::IpAddr;
use std::sync::Arc;

use axum::Extension;
use axum::extract::State;
use axum::extract::WebSocketUpgrade;
use axum::extract::ws::WebSocket;
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

use rootcause::prelude::*;
use uptrakit_internal_wire::{IncomingSeq, OutgoingSeq};
use uptrakit_shared_db::entity::service as service_entity;

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
pub async fn service_ws(
    State(state): State<Arc<AppState>>,
    identity: Option<Extension<ServiceIdentity>>,
    client_ip: Option<Extension<ClientIp>>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    use crate::auth::rate_limit::RateLimitOutcome;

    // Per-IP connection rate limit: 30 attempts per 60 seconds.
    // Fail-closed on DB error to prevent bypass under load.
    if let Some(Extension(ClientIp(ref ip))) = client_ip {
        let key = format!("ws_connect:{ip}");
        match state.rate_limit_store.check_rate_limit(&key, 30, 60).await {
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
        // Try unified lookup: find any non-deactivated service by secret hash.
        match lookup_by_secret(state.db(), &secret).await {
            Ok(service) => {
                tracing::info!(
                    service_id = %service.id,
                    "enrolled service WS upgrade (bearer)"
                );
                ConnectionType::Enrolled(service.id)
            }
            Err(e) => {
                // Per-IP auth failure rate limit: 10 failures per 300 seconds.
                if let Some(Extension(ClientIp(ref ip))) = client_ip {
                    let fail_key = format!("ws_auth_fail:{ip}");
                    match state
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
/// All services (agent and MQTT) use deterministic SHA-256 hashing for enrollment
/// secrets. This allows a single indexed DB query instead of iterating rows.
async fn lookup_by_secret(
    db: &sea_orm::DatabaseConnection,
    secret: &str,
) -> ServiceWsResult<service_entity::Model> {
    let secret_hash = crate::auth::token::hash_token(secret);
    uptrakit_shared_db::entity::prelude::Service::find()
        .filter(service_entity::Column::EnrollmentSecretHash.eq(&secret_hash))
        .filter(service_entity::Column::DeactivatedAt.is_null())
        .one(db)
        .await
        .context_to::<ServiceWsError>()?
        .ok_or_else(|| report!(ServiceWsError::InvalidSecret))
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
        ConnectionType::Enrolled(service_id) => {
            connection::handle_enrolled(
                socket,
                state,
                service_id,
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

#[cfg(test)]
mod tests {
    use super::protocol::{
        MessageRateLimiter, ServiceWsError, WS_MESSAGE_RATE_WINDOW, deserialize_service_msg,
        record_service_activity,
    };
    use sea_orm::{
        ActiveModelTrait, ConnectOptions, ConnectionTrait, Database, DatabaseConnection,
        EntityTrait, Set,
    };
    use uptrakit_internal_wire::IncomingSeq;
    use uptrakit_shared_db::entity::service as service_entity;

    #[test]
    fn deserialize_unknown_type_returns_none() {
        let mut in_seq = IncomingSeq::new();
        let json = r#"{"seq":1,"type":"future_message","data":{"foo":"bar"}}"#;
        let result = deserialize_service_msg(&mut in_seq, json);
        assert!(
            matches!(result, Ok(None)),
            "unknown type should return Ok(None)"
        );
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
        let json = r#"{"seq":2,"type":"ping","service_ts":12345}"#;
        let result = deserialize_service_msg(&mut in_seq, json);
        assert!(
            matches!(result, Err(ref e) if matches!(e.current_context(), ServiceWsError::SequenceValidation(_))),
            "sequence mismatch should return Err(SequenceValidation)"
        );
    }

    #[test]
    fn message_rate_limiter_enforces_window() {
        let mut limiter = MessageRateLimiter::new(WS_MESSAGE_RATE_WINDOW, 2);
        assert!(limiter.allow());
        assert!(limiter.allow());
        assert!(!limiter.allow());

        let reset_time = std::time::Instant::now() - WS_MESSAGE_RATE_WINDOW;
        limiter.set_window_start(reset_time);
        assert!(limiter.allow());
    }

    async fn setup_test_db() -> DatabaseConnection {
        let opt = ConnectOptions::new("sqlite::memory:".to_owned());
        let db = Database::connect(opt).await.expect("test db");
        db.execute_unprepared(
            "CREATE TABLE services (
                id TEXT PRIMARY KEY,
                tenant_id TEXT NOT NULL,
                capabilities TEXT NOT NULL DEFAULT '[]',
                hostname TEXT NOT NULL,
                friendly_name TEXT NOT NULL,
                ip_address TEXT,
                status TEXT NOT NULL,
                enrollment_secret_hash TEXT NOT NULL UNIQUE,
                client_version TEXT,
                last_seen_at INTEGER,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                deactivated_at INTEGER,
                ping_interval_seconds INTEGER
            )",
        )
        .await
        .expect("create services");
        db
    }

    async fn insert_service(db: &DatabaseConnection, ip_address: Option<&str>) -> uuid::Uuid {
        let id = uuid::Uuid::now_v7();
        let now = time::OffsetDateTime::now_utc();
        service_entity::ActiveModel {
            id: Set(id),
            tenant_id: Set(uuid::Uuid::now_v7()),
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
        }
        .insert(db)
        .await
        .expect("insert service");
        id
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
}
