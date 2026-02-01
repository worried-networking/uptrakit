use crate::AppState;
use crate::SettingKey;
use crate::auth::permissions::Permission;
use crate::middleware::require_auth::AuthenticatedUser;
use crate::settings_store::upsert_setting;
use axum::{
    Extension, Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use ipnet::IpNet;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::Arc;
use utoipa::ToSchema;

#[derive(Serialize, ToSchema)]
pub struct NetworkSettingsResponse {
    pub trusted_proxies: Vec<String>,
    pub real_ip_header: String,
    pub extra_sans: Vec<String>,
    pub https_addr: String,
    pub forwarded_client_cert_info_header: Option<String>,
    pub forwarded_client_cert_pem_header: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct UpdateNetworkSettingsRequest {
    pub trusted_proxies: Option<Vec<String>>,
    pub real_ip_header: Option<String>,
    pub extra_sans: Option<Vec<String>>,
    pub https_addr: Option<String>,
    /// Header name for structured client certificate info (e.g. `X-Forwarded-Tls-Client-Cert-Info`).
    /// Empty string disables.
    pub forwarded_client_cert_info_header: Option<String>,
    /// Header name for PEM-encoded client certificate (e.g. `X-Forwarded-Tls-Client-Cert`).
    /// Empty string disables.
    pub forwarded_client_cert_pem_header: Option<String>,
}

/// Get network settings
#[utoipa::path(
    get,
    path = "/api/v1/settings/network",
    responses(
        (status = 200, description = "Network settings", body = NetworkSettingsResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Settings",
    security(("bearer_token" = []))
)]
pub async fn get_network_settings(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUser>,
) -> Response {
    if !user.has_permission(Permission::ViewSettings) {
        return (StatusCode::FORBIDDEN, "Insufficient permissions").into_response();
    }

    let network = state.settings.network().await;
    let response = NetworkSettingsResponse {
        trusted_proxies: network
            .trusted_proxies
            .iter()
            .map(|n| n.to_string())
            .collect(),
        real_ip_header: network.real_ip_header,
        extra_sans: network.extra_sans,
        https_addr: network.https_addr.to_string(),
        forwarded_client_cert_info_header: network.forwarded_client_cert_info_header,
        forwarded_client_cert_pem_header: network.forwarded_client_cert_pem_header,
    };
    (StatusCode::OK, Json(response)).into_response()
}

/// Update network settings
#[utoipa::path(
    put,
    path = "/api/v1/settings/network",
    request_body = UpdateNetworkSettingsRequest,
    responses(
        (status = 200, description = "Settings updated", body = NetworkSettingsResponse),
        (status = 400, description = "Invalid values"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Settings",
    security(("bearer_token" = []))
)]
pub async fn update_network_settings(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUser>,
    Json(req): Json<UpdateNetworkSettingsRequest>,
) -> Response {
    if !user.has_permission(Permission::ManageSettings) {
        return (StatusCode::FORBIDDEN, "Insufficient permissions").into_response();
    }

    // Validate and apply trusted proxies (runtime-changeable)
    if let Some(ref proxies) = req.trusted_proxies {
        let mut parsed = Vec::with_capacity(proxies.len());
        for s in proxies {
            match s.parse::<IpNet>() {
                Ok(net) => parsed.push(net),
                Err(_) => {
                    // Try bare IP
                    match s.parse::<std::net::IpAddr>() {
                        Ok(ip) => parsed.push(IpNet::from(ip)),
                        Err(_) => {
                            return (StatusCode::BAD_REQUEST, format!("invalid IP or CIDR: {s}"))
                                .into_response();
                        }
                    }
                }
            }
        }
        let json_val = serde_json::json!(parsed.iter().map(|n| n.to_string()).collect::<Vec<_>>());
        if let Err(e) = upsert_setting(&state.db, SettingKey::TrustedProxies, json_val).await {
            tracing::error!("Failed to save trusted_proxies: {e:?}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
        state.settings.set_trusted_proxies(parsed).await;
    }

    // Validate and apply real_ip_header (runtime-changeable)
    if let Some(ref header) = req.real_ip_header {
        if let Err(e) = upsert_setting(
            &state.db,
            SettingKey::RealIpHeader,
            serde_json::json!(header),
        )
        .await
        {
            tracing::error!("Failed to save real_ip_header: {e:?}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
        state.settings.set_real_ip_header(header.clone()).await;
    }

    // Validate and apply extra_sans (runtime-changeable)
    if let Some(ref sans) = req.extra_sans {
        if let Err(e) =
            upsert_setting(&state.db, SettingKey::ExtraSans, serde_json::json!(sans)).await
        {
            tracing::error!("Failed to save extra_sans: {e:?}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
        state.settings.set_extra_sans(sans.clone()).await;
    }

    // Validate and apply forwarded_client_cert_info_header (runtime-changeable)
    if let Some(ref header) = req.forwarded_client_cert_info_header {
        let value = if header.is_empty() {
            None
        } else {
            Some(header.clone())
        };
        let json_val = match &value {
            Some(v) => serde_json::json!(v),
            None => serde_json::Value::Null,
        };
        if let Err(e) = upsert_setting(
            &state.db,
            SettingKey::ForwardedClientCertInfoHeader,
            json_val,
        )
        .await
        {
            tracing::error!("Failed to save forwarded_client_cert_info_header: {e:?}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
        state
            .settings
            .set_forwarded_client_cert_info_header(value)
            .await;
    }

    // Validate and apply forwarded_client_cert_pem_header (runtime-changeable)
    if let Some(ref header) = req.forwarded_client_cert_pem_header {
        let value = if header.is_empty() {
            None
        } else {
            Some(header.clone())
        };
        let json_val = match &value {
            Some(v) => serde_json::json!(v),
            None => serde_json::Value::Null,
        };
        if let Err(e) = upsert_setting(
            &state.db,
            SettingKey::ForwardedClientCertPemHeader,
            json_val,
        )
        .await
        {
            tracing::error!("Failed to save forwarded_client_cert_pem_header: {e:?}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
        state
            .settings
            .set_forwarded_client_cert_pem_header(value)
            .await;
    }

    // Validate and apply https_addr (requires restart — save to DB only)
    if let Some(ref addr_str) = req.https_addr {
        let addr: SocketAddr = match addr_str.parse() {
            Ok(a) => a,
            Err(_) => {
                return (
                    StatusCode::BAD_REQUEST,
                    format!("invalid HTTPS address: {addr_str}"),
                )
                    .into_response();
            }
        };
        if let Err(e) = upsert_setting(
            &state.db,
            SettingKey::HttpsAddr,
            serde_json::json!(addr.to_string()),
        )
        .await
        {
            tracing::error!("Failed to save https_addr: {e:?}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
        state.settings.set_https_addr(addr).await;
    }

    let network = state.settings.network().await;
    let response = NetworkSettingsResponse {
        trusted_proxies: network
            .trusted_proxies
            .iter()
            .map(|n| n.to_string())
            .collect(),
        real_ip_header: network.real_ip_header,
        extra_sans: network.extra_sans,
        https_addr: network.https_addr.to_string(),
        forwarded_client_cert_info_header: network.forwarded_client_cert_info_header,
        forwarded_client_cert_pem_header: network.forwarded_client_cert_pem_header,
    };
    (StatusCode::OK, Json(response)).into_response()
}
