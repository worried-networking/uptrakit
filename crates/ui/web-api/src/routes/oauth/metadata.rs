//! RFC 8414 §3 authorization server metadata endpoint.

use std::sync::Arc;

use axum::Extension;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Json, Response};
use uptrakit_web_api_types::oauth::OAuthAuthorizationServerMetadata;

use crate::AppState;
use crate::extract::ExternalBaseUrl;
use crate::routes::oauth::helpers::resolve_external_base_url;

const DEVICE_CODE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";

/// RFC 8414 §3 authorization server metadata (device-grant-only subset).
#[utoipa::path(
    get,
    path = "/.well-known/oauth-authorization-server",
    responses(
        (status = 200, description = "Discovery metadata", body = OAuthAuthorizationServerMetadata)
    ),
    tag = "OAuth"
)]
#[tracing::instrument(skip_all)]
pub async fn metadata(
    State(_state): State<Arc<AppState>>,
    external_base_url: Option<Extension<ExternalBaseUrl>>,
    headers: HeaderMap,
) -> Response {
    let base = resolve_external_base_url(external_base_url, &headers);
    let body = OAuthAuthorizationServerMetadata {
        issuer: base.clone(),
        device_authorization_endpoint: format!("{base}/api/v1/oauth/device_authorization"),
        token_endpoint: format!("{base}/api/v1/oauth/token"),
        grant_types_supported: vec![DEVICE_CODE_GRANT.to_string()],
        response_types_supported: vec![],
        token_endpoint_auth_methods_supported: vec!["none".into()],
        code_challenge_methods_supported: vec![],
    };
    (StatusCode::OK, Json(body)).into_response()
}
