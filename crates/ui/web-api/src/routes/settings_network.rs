use crate::AppState;
use crate::SettingKey;
use crate::error_response::error_response;
use crate::extract::Validated;
use crate::middleware::permission::CanManageGlobalSettings;
use crate::middleware::require_auth::{
    AuthenticatedApiTokenId, AuthenticatedUser, authenticated_user_audit_actor,
};
use crate::settings_store::upsert_global_setting;
use axum::{
    Extension, Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use ipnet::IpNet;
use sea_orm::ConnectionTrait;
use std::net::SocketAddr;
use std::sync::Arc;

pub use uptrakit_web_api_types::settings_network::{
    NetworkSettingsResponse, UpdateNetworkSettingsRequest,
};

fn emit_network_settings_audit(
    state: &AppState,
    user: &AuthenticatedUser,
    api_token_id: Option<AuthenticatedApiTokenId>,
    outcome: uptrakit_audit_log::AuditOutcome,
    details: serde_json::Value,
) {
    let (actor_type, actor_id) = authenticated_user_audit_actor(user, api_token_id);

    if let Ok(entry) = uptrakit_audit_log::AuditEntry::builder(
        uptrakit_audit_log::AuditActionType::GLOBAL_SETTING_UPDATE,
    )
    .system_scope()
    .actor(actor_type, actor_id)
    .target(
        "global_setting",
        "network".to_string(),
        Some("network".to_string()),
    )
    .outcome(outcome)
    .details(details)
    .build()
    {
        state.audit_emitter.emit_best_effort(entry);
    }
}

fn network_settings_audit_details(
    requested_keys: &[&str],
    regenerate_cert_requested: bool,
    extra: serde_json::Value,
) -> serde_json::Value {
    let mut details = serde_json::json!({
        "setting_area": "network",
        "requested_keys": requested_keys,
        "regenerate_cert_requested": regenerate_cert_requested,
    });

    if let Some(map) = details.as_object_mut()
        && let Some(extra_map) = extra.as_object()
    {
        for (key, value) in extra_map {
            map.insert(key.clone(), value.clone());
        }
    }

    details
}

struct NetworkSettingsUpdateError {
    response: Response,
    outcome: uptrakit_audit_log::AuditOutcome,
    details: serde_json::Value,
}

impl NetworkSettingsUpdateError {
    fn validation(details: serde_json::Value, response: Response) -> Self {
        Self {
            response,
            outcome: uptrakit_audit_log::AuditOutcome::ValidationFailed,
            details,
        }
    }

    fn failed(details: serde_json::Value, response: Response) -> Self {
        Self {
            response,
            outcome: uptrakit_audit_log::AuditOutcome::Failed,
            details,
        }
    }
}

/// Persist a single global setting to the database, returning an error response
/// on failure. The `setting_name` is used only for the error log message.
async fn persist_setting(
    db: &impl ConnectionTrait,
    key: SettingKey,
    value: serde_json::Value,
    setting_name: &str,
) -> Result<(), Response> {
    upsert_global_setting(db, key, value).await.map_err(|e| {
        tracing::error!(error = ?e, setting_name = setting_name, "Failed to save setting");
        error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
    })
}

/// Convert an empty string to `None`, preserving non-empty values.
fn empty_to_none(s: &str) -> Option<String> {
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}

/// Serialize an `Option<String>` to a JSON value (`null` when `None`).
fn option_to_json(value: &Option<String>) -> serde_json::Value {
    match value {
        Some(v) => serde_json::json!(v),
        None => serde_json::Value::Null,
    }
}

/// Parse a list of proxy strings into validated `IpNet` values.
fn parse_trusted_proxies(proxies: &[String]) -> Result<Vec<IpNet>, String> {
    let mut parsed = Vec::with_capacity(proxies.len());
    for s in proxies {
        let net = s
            .parse::<IpNet>()
            .or_else(|_| s.parse::<std::net::IpAddr>().map(IpNet::from))
            .map_err(|_| format!("invalid IP or CIDR: {s}"))?;
        parsed.push(net);
    }
    Ok(parsed)
}

/// Validate a PKI address URL, returning `None` for empty strings or the
/// normalized (trailing-slash-stripped) URL on success.
fn validate_pki_addr(url_str: &str) -> Result<Option<String>, String> {
    if url_str.is_empty() {
        return Ok(None);
    }
    let url = url_str
        .parse::<url::Url>()
        .map_err(|e| format!("invalid PKI address URL: {e}"))?;
    match url.scheme() {
        "http" | "https" => {}
        other => {
            return Err(format!(
                "unsupported URL scheme: {other} (expected http or https)"
            ));
        }
    }
    Ok(Some(url_str.trim_end_matches('/').to_string()))
}

/// Get network settings
#[utoipa::path(
    get,
    path = "/api/v1/global-settings/network",
    responses(
        (status = 200, description = "Network settings", body = NetworkSettingsResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Global Settings",
    extensions(("x-required-permission" = json!("manage_global_settings"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn get_network_settings(
    State(state): State<Arc<AppState>>,
    CanManageGlobalSettings(_user): CanManageGlobalSettings,
) -> Response {
    let network = state.settings.network();
    let response = NetworkSettingsResponse {
        trusted_proxies: network
            .trusted_proxies
            .iter()
            .map(|n| n.to_string())
            .collect(),
        real_ip_header: network.real_ip_header,
        sans: network.sans,
        https_addr: network.https_addr.to_string(),
        forwarded_client_cert_info_header: network.forwarded_client_cert_info_header,
        forwarded_client_cert_pem_header: network.forwarded_client_cert_pem_header,
        pki_addr: network.pki_addr,
        pki_addr_warning: None,
        cert_regenerated: None,
    };
    (StatusCode::OK, Json(response)).into_response()
}

/// Update network settings
#[utoipa::path(
    put,
    path = "/api/v1/global-settings/network",
    request_body = UpdateNetworkSettingsRequest,
    responses(
        (status = 200, description = "Settings updated", body = NetworkSettingsResponse),
        (status = 400, description = "Invalid values"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Global Settings",
    extensions(("x-required-permission" = json!("manage_global_settings"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn update_network_settings(
    State(state): State<Arc<AppState>>,
    CanManageGlobalSettings(user): CanManageGlobalSettings,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Validated(req): Validated<UpdateNetworkSettingsRequest>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);
    let mut requested_keys = Vec::new();
    if req.trusted_proxies.is_some() {
        requested_keys.push(SettingKey::TrustedProxies.as_str());
    }
    if req.real_ip_header.is_some() {
        requested_keys.push(SettingKey::RealIpHeader.as_str());
    }
    if req.sans.is_some() {
        requested_keys.push(SettingKey::Sans.as_str());
    }
    if req.https_addr.is_some() {
        requested_keys.push(SettingKey::HttpsAddr.as_str());
    }
    if req.forwarded_client_cert_info_header.is_some() {
        requested_keys.push(SettingKey::ForwardedClientCertInfoHeader.as_str());
    }
    if req.forwarded_client_cert_pem_header.is_some() {
        requested_keys.push(SettingKey::ForwardedClientCertPemHeader.as_str());
    }
    if req.pki_addr.is_some() {
        requested_keys.push(SettingKey::PkiAddr.as_str());
    }
    let regenerate_cert_requested = req.regenerate_cert == Some(true);

    match update_network_settings_inner(&state, req).await {
        Ok(resp) => {
            if !requested_keys.is_empty() {
                emit_network_settings_audit(
                    &state,
                    &user,
                    api_token_id,
                    uptrakit_audit_log::AuditOutcome::Success,
                    network_settings_audit_details(
                        &requested_keys,
                        regenerate_cert_requested,
                        serde_json::json!({}),
                    ),
                );
            }
            resp
        }
        Err(error) => {
            if !requested_keys.is_empty() {
                emit_network_settings_audit(
                    &state,
                    &user,
                    api_token_id,
                    error.outcome,
                    network_settings_audit_details(
                        &requested_keys,
                        regenerate_cert_requested,
                        error.details,
                    ),
                );
            }
            error.response
        }
    }
}

async fn update_network_settings_inner(
    state: &AppState,
    req: UpdateNetworkSettingsRequest,
) -> Result<Response, NetworkSettingsUpdateError> {
    let db = state.db();

    // Validate and apply trusted proxies (runtime-changeable)
    if let Some(ref proxies) = req.trusted_proxies {
        let parsed = parse_trusted_proxies(proxies).map_err(|msg| {
            NetworkSettingsUpdateError::validation(
                serde_json::json!({
                    "reason_code": "trusted_proxies_invalid",
                    "setting_key": SettingKey::TrustedProxies.as_str(),
                    "provided_trusted_proxies": proxies,
                    "validation_error": msg,
                }),
                error_response(StatusCode::BAD_REQUEST, msg),
            )
        })?;
        let json_val = serde_json::json!(parsed.iter().map(|n| n.to_string()).collect::<Vec<_>>());
        persist_setting(db, SettingKey::TrustedProxies, json_val, "trusted_proxies")
            .await
            .map_err(|response| {
                NetworkSettingsUpdateError::failed(
                    serde_json::json!({
                        "reason_code": "trusted_proxies_upsert_failed",
                        "setting_key": SettingKey::TrustedProxies.as_str(),
                    }),
                    response,
                )
            })?;
        state.settings.set_trusted_proxies(parsed).await;
    }

    // Validate and apply real_ip_header (runtime-changeable)
    if let Some(ref header) = req.real_ip_header {
        persist_setting(
            db,
            SettingKey::RealIpHeader,
            serde_json::json!(header),
            "real_ip_header",
        )
        .await
        .map_err(|response| {
            NetworkSettingsUpdateError::failed(
                serde_json::json!({
                    "reason_code": "real_ip_header_upsert_failed",
                    "setting_key": SettingKey::RealIpHeader.as_str(),
                }),
                response,
            )
        })?;
        state.settings.set_real_ip_header(header.clone()).await;
    }

    // Validate and apply sans (runtime-changeable)
    let sans_updated = req.sans.is_some();
    if let Some(ref sans) = req.sans {
        persist_setting(db, SettingKey::Sans, serde_json::json!(sans), "sans")
            .await
            .map_err(|response| {
                NetworkSettingsUpdateError::failed(
                    serde_json::json!({
                        "reason_code": "sans_upsert_failed",
                        "setting_key": SettingKey::Sans.as_str(),
                    }),
                    response,
                )
            })?;
        state.settings.set_sans(sans.clone()).await;
    }

    // Validate and apply forwarded_client_cert_info_header (runtime-changeable)
    if let Some(ref header) = req.forwarded_client_cert_info_header {
        let value = empty_to_none(header);
        persist_setting(
            db,
            SettingKey::ForwardedClientCertInfoHeader,
            option_to_json(&value),
            "forwarded_client_cert_info_header",
        )
        .await
        .map_err(|response| {
            NetworkSettingsUpdateError::failed(
                serde_json::json!({
                    "reason_code": "forwarded_client_cert_info_header_upsert_failed",
                    "setting_key": SettingKey::ForwardedClientCertInfoHeader.as_str(),
                }),
                response,
            )
        })?;
        state
            .settings
            .set_forwarded_client_cert_info_header(value)
            .await;
    }

    // Validate and apply forwarded_client_cert_pem_header (runtime-changeable)
    if let Some(ref header) = req.forwarded_client_cert_pem_header {
        let value = empty_to_none(header);
        persist_setting(
            db,
            SettingKey::ForwardedClientCertPemHeader,
            option_to_json(&value),
            "forwarded_client_cert_pem_header",
        )
        .await
        .map_err(|response| {
            NetworkSettingsUpdateError::failed(
                serde_json::json!({
                    "reason_code": "forwarded_client_cert_pem_header_upsert_failed",
                    "setting_key": SettingKey::ForwardedClientCertPemHeader.as_str(),
                }),
                response,
            )
        })?;
        state
            .settings
            .set_forwarded_client_cert_pem_header(value)
            .await;
    }

    // Validate and apply pki_addr (requires CA rotation to fully take effect)
    let pki_addr_changed = if let Some(ref url_str) = req.pki_addr {
        let value = validate_pki_addr(url_str).map_err(|msg| {
            NetworkSettingsUpdateError::validation(
                serde_json::json!({
                    "reason_code": "pki_addr_invalid",
                    "setting_key": SettingKey::PkiAddr.as_str(),
                    "provided_pki_addr": url_str,
                    "validation_error": msg,
                }),
                error_response(StatusCode::BAD_REQUEST, msg),
            )
        })?;
        let changed = state.settings.pki_addr() != value;
        persist_setting(db, SettingKey::PkiAddr, option_to_json(&value), "pki_addr")
            .await
            .map_err(|response| {
                NetworkSettingsUpdateError::failed(
                    serde_json::json!({
                        "reason_code": "pki_addr_upsert_failed",
                        "setting_key": SettingKey::PkiAddr.as_str(),
                    }),
                    response,
                )
            })?;
        state.settings.set_pki_addr(value).await;
        changed
    } else {
        false
    };

    // Validate and apply https_addr (requires restart -- save to DB only)
    if let Some(ref addr_str) = req.https_addr {
        let addr: SocketAddr = addr_str.parse().map_err(|_| {
            NetworkSettingsUpdateError::validation(
                serde_json::json!({
                    "reason_code": "https_addr_invalid",
                    "setting_key": SettingKey::HttpsAddr.as_str(),
                    "provided_https_addr": addr_str,
                }),
                error_response(
                    StatusCode::BAD_REQUEST,
                    format!("invalid HTTPS address: {addr_str}"),
                ),
            )
        })?;
        persist_setting(
            db,
            SettingKey::HttpsAddr,
            serde_json::json!(addr.to_string()),
            "https_addr",
        )
        .await
        .map_err(|response| {
            NetworkSettingsUpdateError::failed(
                serde_json::json!({
                    "reason_code": "https_addr_upsert_failed",
                    "setting_key": SettingKey::HttpsAddr.as_str(),
                }),
                response,
            )
        })?;
        state.settings.set_https_addr(addr).await;
    }

    // Optionally regenerate server certificate when SANs were updated
    let cert_regenerated = if sans_updated && req.regenerate_cert == Some(true) {
        match super::server_cert::renew_server_certificate_inner(state).await {
            Ok(_) => {
                tracing::info!(
                    "server certificate regenerated after SAN update via network settings API"
                );
                Some(true)
            }
            Err(e) => {
                tracing::error!(error = %e, "server certificate regeneration failed after SAN update");
                return Err(NetworkSettingsUpdateError::failed(
                    serde_json::json!({
                        "reason_code": "server_certificate_regeneration_failed",
                        "setting_key": SettingKey::Sans.as_str(),
                    }),
                    error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "SANs updated but server certificate regeneration failed",
                    ),
                ));
            }
        }
    } else {
        None
    };

    let network = state.settings.network();
    let warning = if pki_addr_changed {
        Some(
            "Changing the PKI address requires CA rotation. All agent certificates will need \
             to be renewed. Call POST /api/v1/global-settings/ca/rotate to apply the change."
                .to_string(),
        )
    } else {
        None
    };
    let response = NetworkSettingsResponse {
        trusted_proxies: network
            .trusted_proxies
            .iter()
            .map(|n| n.to_string())
            .collect(),
        real_ip_header: network.real_ip_header,
        sans: network.sans,
        https_addr: network.https_addr.to_string(),
        forwarded_client_cert_info_header: network.forwarded_client_cert_info_header,
        forwarded_client_cert_pem_header: network.forwarded_client_cert_pem_header,
        pki_addr: network.pki_addr,
        pki_addr_warning: warning,
        cert_regenerated,
    };
    Ok((StatusCode::OK, Json(response)).into_response())
}

#[cfg(all(test, feature = "db-sqlite"))]
mod tests {
    use super::*;
    use crate::auth::AuthMethod;
    use crate::auth::permissions::Permission;
    use crate::middleware::permission::CanManageGlobalSettings;
    use crate::middleware::require_auth::AuthenticatedUser;
    use crate::test_harness::{build_test_state, insert_default_tenant, setup_migrated_db};
    use sea_orm::{
        ColumnTrait, ConnectOptions, Database, DatabaseConnection, EntityTrait, PaginatorTrait,
        QueryFilter, QueryOrder,
    };
    use uptrakit_shared_db::entity::system_audit_log;

    async fn latest_global_setting_update_audit_row(
        db: &DatabaseConnection,
    ) -> system_audit_log::Model {
        for _ in 0..50 {
            if let Some(row) = system_audit_log::Entity::find()
                .filter(
                    system_audit_log::Column::ActionType
                        .eq(uptrakit_audit_log::AuditActionType::GLOBAL_SETTING_UPDATE),
                )
                .order_by_desc(system_audit_log::Column::OccurredAt)
                .one(db)
                .await
                .expect("query system audit rows")
            {
                return row;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        panic!("expected global setting update system audit row");
    }

    async fn wait_for_system_audit_rows(db: &DatabaseConnection, expected: u64) {
        for _ in 0..50 {
            let count = system_audit_log::Entity::find()
                .count(db)
                .await
                .expect("count system audit rows");
            if count == expected {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        panic!("expected {expected} system audit rows");
    }

    #[tokio::test]
    async fn update_network_settings_validation_failure_writes_global_setting_update_audit_event() {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        let (state, _jwt) = build_test_state(db.clone(), tenant_id).await;

        let response = update_network_settings(
            State(Arc::clone(&state)),
            CanManageGlobalSettings::new(AuthenticatedUser {
                user_id: uuid::Uuid::now_v7(),
                auth_method: AuthMethod::Password,
                permissions: vec![Permission::ManageGlobalSettings],
            }),
            None,
            Validated(UpdateNetworkSettingsRequest {
                trusted_proxies: Some(vec!["not-an-ip".to_string()]),
                real_ip_header: None,
                sans: None,
                https_addr: None,
                forwarded_client_cert_info_header: None,
                forwarded_client_cert_pem_header: None,
                pki_addr: None,
                regenerate_cert: None,
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        wait_for_system_audit_rows(&db, 1).await;

        let row = latest_global_setting_update_audit_row(&db).await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::ValidationFailed.as_str()
        );
        assert_eq!(row.target_type.as_deref(), Some("global_setting"));
        assert_eq!(row.target_id.as_deref(), Some("network"));

        let details = row.details_json.expect("details");
        assert_eq!(details["setting_area"], serde_json::json!("network"));
        assert_eq!(
            details["reason_code"],
            serde_json::json!("trusted_proxies_invalid")
        );
        assert_eq!(
            details["setting_key"],
            serde_json::json!(SettingKey::TrustedProxies.as_str())
        );
    }

    #[tokio::test]
    async fn update_network_settings_persistence_failure_writes_global_setting_update_audit_event()
    {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        let (mut state, _jwt) = build_test_state(db.clone(), tenant_id).await;

        let raw_db = Database::connect(ConnectOptions::new("sqlite::memory:".to_string()))
            .await
            .expect("raw sqlite db");
        Arc::make_mut(&mut state).db = crate::app_state::DbState::new(raw_db);

        let response = update_network_settings(
            State(Arc::clone(&state)),
            CanManageGlobalSettings::new(AuthenticatedUser {
                user_id: uuid::Uuid::now_v7(),
                auth_method: AuthMethod::Password,
                permissions: vec![Permission::ManageGlobalSettings],
            }),
            None,
            Validated(UpdateNetworkSettingsRequest {
                trusted_proxies: None,
                real_ip_header: Some("x-forwarded-for".to_string()),
                sans: None,
                https_addr: None,
                forwarded_client_cert_info_header: None,
                forwarded_client_cert_pem_header: None,
                pki_addr: None,
                regenerate_cert: None,
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        wait_for_system_audit_rows(&db, 1).await;

        let row = latest_global_setting_update_audit_row(&db).await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Failed.as_str()
        );
        assert_eq!(row.target_type.as_deref(), Some("global_setting"));
        assert_eq!(row.target_id.as_deref(), Some("network"));

        let details = row.details_json.expect("details");
        assert_eq!(details["setting_area"], serde_json::json!("network"));
        assert_eq!(
            details["reason_code"],
            serde_json::json!("real_ip_header_upsert_failed")
        );
        assert_eq!(
            details["setting_key"],
            serde_json::json!(SettingKey::RealIpHeader.as_str())
        );
    }
}
