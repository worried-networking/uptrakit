use crate::AppState;
use crate::SettingKey;
use crate::auth::permissions::Permission;
use crate::error_response::error_response;
use crate::middleware::require_auth::AuthenticatedUser;
use crate::settings_store::upsert_setting;
use axum::{
    Extension, Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use ipnet::IpNet;
use std::net::SocketAddr;
use std::sync::Arc;

pub use uptrakit_web_api_types::settings_network::{
    NetworkSettingsResponse, UpdateNetworkSettingsRequest,
};
use uptrakit_web_api_types::validation::Validate;

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
    if !user.has_permission(Permission::ManageGlobalSettings) {
        return error_response(StatusCode::FORBIDDEN, "Insufficient permissions");
    }

    let network = state.settings.network();
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
        pki_addr: network.pki_addr,
        pki_addr_warning: None,
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
    if !user.has_permission(Permission::ManageGlobalSettings) {
        return error_response(StatusCode::FORBIDDEN, "Insufficient permissions");
    }

    if let Err(e) = req.validate() {
        return error_response(StatusCode::BAD_REQUEST, e.to_string());
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
                            return error_response(
                                StatusCode::BAD_REQUEST,
                                format!("invalid IP or CIDR: {s}"),
                            );
                        }
                    }
                }
            }
        }
        let json_val = serde_json::json!(parsed.iter().map(|n| n.to_string()).collect::<Vec<_>>());
        if let Err(e) = upsert_setting(
            &state.db,
            state.default_tenant_id,
            SettingKey::TrustedProxies,
            json_val,
        )
        .await
        {
            tracing::error!("Failed to save trusted_proxies: {e:?}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
        state.settings.set_trusted_proxies(parsed).await;
    }

    // Validate and apply real_ip_header (runtime-changeable)
    if let Some(ref header) = req.real_ip_header {
        if let Err(e) = upsert_setting(
            &state.db,
            state.default_tenant_id,
            SettingKey::RealIpHeader,
            serde_json::json!(header),
        )
        .await
        {
            tracing::error!("Failed to save real_ip_header: {e:?}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
        state.settings.set_real_ip_header(header.clone()).await;
    }

    // Validate and apply extra_sans (runtime-changeable)
    if let Some(ref sans) = req.extra_sans {
        if let Err(e) = upsert_setting(
            &state.db,
            state.default_tenant_id,
            SettingKey::ExtraSans,
            serde_json::json!(sans),
        )
        .await
        {
            tracing::error!("Failed to save extra_sans: {e:?}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
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
            state.default_tenant_id,
            SettingKey::ForwardedClientCertInfoHeader,
            json_val,
        )
        .await
        {
            tracing::error!("Failed to save forwarded_client_cert_info_header: {e:?}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
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
            state.default_tenant_id,
            SettingKey::ForwardedClientCertPemHeader,
            json_val,
        )
        .await
        {
            tracing::error!("Failed to save forwarded_client_cert_pem_header: {e:?}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
        state
            .settings
            .set_forwarded_client_cert_pem_header(value)
            .await;
    }

    // Track whether pki_addr changed for the warning
    let mut pki_addr_changed = false;

    // Validate and apply pki_addr (requires CA rotation to fully take effect)
    if let Some(ref url_str) = req.pki_addr {
        let value = if url_str.is_empty() {
            None
        } else {
            // Validate URL format
            match url_str.parse::<url::Url>() {
                Ok(url) => match url.scheme() {
                    "http" | "https" => {}
                    other => {
                        return error_response(
                            StatusCode::BAD_REQUEST,
                            format!("unsupported URL scheme: {other} (expected http or https)"),
                        );
                    }
                },
                Err(e) => {
                    return error_response(
                        StatusCode::BAD_REQUEST,
                        format!("invalid PKI address URL: {e}"),
                    );
                }
            }
            Some(url_str.trim_end_matches('/').to_string())
        };

        // Check if the value actually changed
        let current = state.settings.pki_addr();
        if current != value {
            pki_addr_changed = true;
        }

        let json_val = match &value {
            Some(v) => serde_json::json!(v),
            None => serde_json::Value::Null,
        };
        if let Err(e) = upsert_setting(
            &state.db,
            state.default_tenant_id,
            SettingKey::PkiAddr,
            json_val,
        )
        .await
        {
            tracing::error!("Failed to save pki_addr: {e:?}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
        state.settings.set_pki_addr(value).await;
    }

    // Validate and apply https_addr (requires restart — save to DB only)
    if let Some(ref addr_str) = req.https_addr {
        let addr: SocketAddr = match addr_str.parse() {
            Ok(a) => a,
            Err(_) => {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    format!("invalid HTTPS address: {addr_str}"),
                );
            }
        };
        if let Err(e) = upsert_setting(
            &state.db,
            state.default_tenant_id,
            SettingKey::HttpsAddr,
            serde_json::json!(addr.to_string()),
        )
        .await
        {
            tracing::error!("Failed to save https_addr: {e:?}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
        state.settings.set_https_addr(addr).await;
    }

    let network = state.settings.network();
    let warning = if pki_addr_changed {
        Some(
            "Changing the PKI address requires CA rotation. All agent certificates will need \
             to be renewed. Call POST /api/v1/settings/rotate-ca to apply the change."
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
        extra_sans: network.extra_sans,
        https_addr: network.https_addr.to_string(),
        forwarded_client_cert_info_header: network.forwarded_client_cert_info_header,
        forwarded_client_cert_pem_header: network.forwarded_client_cert_pem_header,
        pki_addr: network.pki_addr,
        pki_addr_warning: warning,
    };
    (StatusCode::OK, Json(response)).into_response()
}
