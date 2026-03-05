//! HTTP handler for `GET /api/v1/global-settings`.
//!
//! Returns all infrastructure-scoped settings in a single response, avoiding
//! the need for the global-settings UI page to issue multiple parallel requests.

use std::sync::Arc;

use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};

use crate::AppState;
use crate::middleware::permission::CanManageGlobalSettings;
use uptrakit_web_api_types::settings_combined::GlobalSettingsCombinedResponse;
use uptrakit_web_api_types::settings_mqtt::MqttLimitResponse;
use uptrakit_web_api_types::settings_nats::NatsSettingsResponse;
use uptrakit_web_api_types::settings_network::NetworkSettingsResponse;

/// Get all global settings
///
/// Returns network settings, MQTT client limit, and (when NATS support is
/// compiled in) the NATS URL configuration in a single response. Requires the
/// `manage_global_settings` permission.
///
/// System service enrollment tokens are managed via the dedicated
/// `/api/v1/system-enrollment-tokens` endpoints.
#[utoipa::path(
    get,
    path = "/api/v1/global-settings",
    responses(
        (status = 200, description = "Global settings", body = GlobalSettingsCombinedResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Global Settings",
    extensions(("x-required-permission" = json!("manage_global_settings"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn get_global_combined_settings(
    State(state): State<Arc<AppState>>,
    CanManageGlobalSettings(_user): CanManageGlobalSettings,
) -> Response {
    let network = state.settings.network();
    let network_response = NetworkSettingsResponse {
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

    let mqtt_limit = MqttLimitResponse {
        max_clients_per_tenant: state.settings.mqtt_max_clients_per_tenant(),
    };

    #[cfg(feature = "nats")]
    let nats = {
        let nats_url = state.settings.nats_url();
        let has_url = nats_url.is_some();
        Some(NatsSettingsResponse {
            url: nats_url,
            has_url,
        })
    };
    #[cfg(not(feature = "nats"))]
    let nats: Option<NatsSettingsResponse> = None;

    let response = GlobalSettingsCombinedResponse {
        network: network_response,
        mqtt_limit,
        nats,
    };

    (StatusCode::OK, Json(response)).into_response()
}
