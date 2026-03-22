use crate::AppState;
use crate::SettingKey;
use crate::error_response::error_response;
use crate::extract::Validated;
use crate::middleware::permission::CanManageGlobalSettings;
use crate::settings_store::upsert_global_setting;
use axum::{
    Json,
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
    CanManageGlobalSettings(_user): CanManageGlobalSettings,
    Validated(req): Validated<UpdateNetworkSettingsRequest>,
) -> Response {
    match update_network_settings_inner(&state, req).await {
        Ok(resp) | Err(resp) => resp,
    }
}

async fn update_network_settings_inner(
    state: &AppState,
    req: UpdateNetworkSettingsRequest,
) -> Result<Response, Response> {
    let db = state.db();

    // Validate and apply trusted proxies (runtime-changeable)
    if let Some(ref proxies) = req.trusted_proxies {
        let parsed = parse_trusted_proxies(proxies)
            .map_err(|msg| error_response(StatusCode::BAD_REQUEST, msg))?;
        let json_val = serde_json::json!(parsed.iter().map(|n| n.to_string()).collect::<Vec<_>>());
        persist_setting(db, SettingKey::TrustedProxies, json_val, "trusted_proxies").await?;
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
        .await?;
        state.settings.set_real_ip_header(header.clone()).await;
    }

    // Validate and apply sans (runtime-changeable)
    let sans_updated = req.sans.is_some();
    if let Some(ref sans) = req.sans {
        persist_setting(db, SettingKey::Sans, serde_json::json!(sans), "sans").await?;
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
        .await?;
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
        .await?;
        state
            .settings
            .set_forwarded_client_cert_pem_header(value)
            .await;
    }

    // Validate and apply pki_addr (requires CA rotation to fully take effect)
    let pki_addr_changed = if let Some(ref url_str) = req.pki_addr {
        let value = validate_pki_addr(url_str)
            .map_err(|msg| error_response(StatusCode::BAD_REQUEST, msg))?;
        let changed = state.settings.pki_addr() != value;
        persist_setting(db, SettingKey::PkiAddr, option_to_json(&value), "pki_addr").await?;
        state.settings.set_pki_addr(value).await;
        changed
    } else {
        false
    };

    // Validate and apply https_addr (requires restart -- save to DB only)
    if let Some(ref addr_str) = req.https_addr {
        let addr: SocketAddr = addr_str.parse().map_err(|_| {
            error_response(
                StatusCode::BAD_REQUEST,
                format!("invalid HTTPS address: {addr_str}"),
            )
        })?;
        persist_setting(
            db,
            SettingKey::HttpsAddr,
            serde_json::json!(addr.to_string()),
            "https_addr",
        )
        .await?;
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
                return Err(error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "SANs updated but server certificate regeneration failed",
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
