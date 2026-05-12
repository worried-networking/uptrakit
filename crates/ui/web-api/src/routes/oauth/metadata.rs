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
    let body = OAuthAuthorizationServerMetadata::new(
        base.clone(),
        format!("{base}/api/v1/oauth/device_authorization"),
        format!("{base}/api/v1/oauth/token"),
        vec![DEVICE_CODE_GRANT.to_string()],
        vec![],
        vec!["none".into()],
        vec![],
    );
    (StatusCode::OK, Json(body)).into_response()
}
