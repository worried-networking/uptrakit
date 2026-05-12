//! RFC 8414 §3 Authorization Server metadata for MCP OAuth.
//!
//! Returns 404 when `oauth.mcp_enabled = false`.

use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use uptrakit_web_api_types::oauth::AuthorizationServerMetadata;

use crate::AppState;

/// RFC 8414 §2 Authorization Server Metadata.
///
/// Returns 404 when `oauth.mcp_enabled = false`.
#[utoipa::path(
    get,
    path = "/.well-known/oauth-authorization-server",
    responses(
        (status = 200, description = "AS metadata", body = AuthorizationServerMetadata),
        (status = 404, description = "OAuth disabled"),
    ),
    tag = "OAuth"
)]
pub async fn get_as_metadata(State(state): State<Arc<AppState>>) -> Response {
    if !state.oauth.enabled {
        return StatusCode::NOT_FOUND.into_response();
    }
    let canonical = &state.oauth.canonical;
    let issuer = canonical.issuer().as_str().to_string();
    let body = AuthorizationServerMetadata::new(
        issuer.clone(),
        format!("{issuer}/oauth/authorize"),
        format!("{issuer}/oauth/token"),
        if state.oauth.dcr_enabled {
            Some(format!("{issuer}/oauth/register"))
        } else {
            None
        },
        vec!["mcp:read".to_string(), "mcp:write".to_string()],
        vec!["code".to_string()],
        vec![
            "authorization_code".to_string(),
            "refresh_token".to_string(),
        ],
        vec!["S256".to_string()],
        vec!["none".to_string()],
        state.oauth.cimd_enabled,
        None,
    );
    (StatusCode::OK, Json(body)).into_response()
}

#[cfg(all(test, feature = "db-sqlite"))]
mod tests {
    use super::*;
    use http::StatusCode;
    use uptrakit_web_api_types::oauth::AuthorizationServerMetadata;

    use crate::oauth::OAuthState;
    use crate::router::build_router;
    use crate::test_harness::{build_test_state, insert_default_tenant, setup_migrated_db};

    fn enabled_oauth_state(dcr: bool, cimd: bool) -> OAuthState {
        use crate::oauth::canonical_url::CanonicalUrlConfig;
        use crate::oauth::jwt::{McpOAuthJwtSigner, McpOAuthJwtVerifier};
        use time::OffsetDateTime;

        let canonical = CanonicalUrlConfig::new("controller.example.com".to_string(), vec![])
            .expect("test canonical url");
        OAuthState {
            enabled: true,
            canonical,
            signer: Arc::new(McpOAuthJwtSigner::new(b"test-secret-not-used")),
            verifier: Arc::new(McpOAuthJwtVerifier::new(
                b"test-secret-not-used",
                "https://controller.example.com".into(),
                vec![],
            )),
            clock: Arc::new(OffsetDateTime::now_utc),
            instance_id: uuid::Uuid::nil(),
            dcr_enabled: dcr,
            cimd_enabled: cimd,
        }
    }

    async fn app_with_oauth(enabled: bool) -> axum::Router {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        let (state, _jwt) = build_test_state(db, tenant_id).await;
        let mut patched = (*state).clone();
        patched.oauth = if enabled {
            enabled_oauth_state(false, false)
        } else {
            OAuthState::disabled()
        };
        build_router(Arc::new(patched))
    }

    async fn app_with_oauth_state(oauth: OAuthState) -> axum::Router {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        let (state, _jwt) = build_test_state(db, tenant_id).await;
        let mut patched = (*state).clone();
        patched.oauth = oauth;
        build_router(Arc::new(patched))
    }

    #[tokio::test]
    async fn metadata_returns_404_when_master_switch_off() {
        use axum::body::Body;
        use http::Request;
        use tower::ServiceExt;

        let router = app_with_oauth(false).await;
        let req = Request::builder()
            .method("GET")
            .uri("/.well-known/oauth-authorization-server")
            .body(Body::empty())
            .expect("build request");
        let response = router.oneshot(req).await.expect("call router");
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn metadata_advertises_only_authorization_code_and_refresh_token() {
        use axum::body::Body;
        use http::Request;
        use http_body_util::BodyExt;
        use tower::ServiceExt;

        let router = app_with_oauth(true).await;
        let req = Request::builder()
            .method("GET")
            .uri("/.well-known/oauth-authorization-server")
            .body(Body::empty())
            .expect("build request");
        let response = router.oneshot(req).await.expect("call router");
        assert_eq!(response.status(), StatusCode::OK);

        let body = response
            .into_body()
            .collect()
            .await
            .expect("collect body")
            .to_bytes();
        let meta: AuthorizationServerMetadata =
            serde_json::from_slice(&body).expect("parse metadata");

        let grants = &meta.grant_types_supported;
        assert!(grants.contains(&"authorization_code".to_string()));
        assert!(grants.contains(&"refresh_token".to_string()));
        assert_eq!(grants.len(), 2, "only authorization_code and refresh_token");
    }

    #[tokio::test]
    async fn metadata_advertises_s256_only() {
        use axum::body::Body;
        use http::Request;
        use http_body_util::BodyExt;
        use tower::ServiceExt;

        let router = app_with_oauth(true).await;
        let req = Request::builder()
            .method("GET")
            .uri("/.well-known/oauth-authorization-server")
            .body(Body::empty())
            .expect("build request");
        let response = router.oneshot(req).await.expect("call router");
        assert_eq!(response.status(), StatusCode::OK);

        let body = response
            .into_body()
            .collect()
            .await
            .expect("collect body")
            .to_bytes();
        let meta: AuthorizationServerMetadata =
            serde_json::from_slice(&body).expect("parse metadata");

        assert_eq!(
            meta.code_challenge_methods_supported,
            vec!["S256".to_string()],
            "only S256 must be advertised"
        );
    }

    #[tokio::test]
    async fn metadata_omits_registration_endpoint_when_dcr_disabled() {
        use axum::body::Body;
        use http::Request;
        use http_body_util::BodyExt;
        use tower::ServiceExt;

        let router = app_with_oauth_state(enabled_oauth_state(false, false)).await;
        let req = Request::builder()
            .method("GET")
            .uri("/.well-known/oauth-authorization-server")
            .body(Body::empty())
            .expect("build request");
        let response = router.oneshot(req).await.expect("call router");
        assert_eq!(response.status(), StatusCode::OK);

        let body = response
            .into_body()
            .collect()
            .await
            .expect("collect body")
            .to_bytes();
        let meta: AuthorizationServerMetadata =
            serde_json::from_slice(&body).expect("parse metadata");

        assert!(
            meta.registration_endpoint.is_none(),
            "registration_endpoint must be absent when dcr_enabled=false"
        );
    }

    #[tokio::test]
    async fn metadata_includes_registration_endpoint_when_dcr_enabled() {
        use axum::body::Body;
        use http::Request;
        use http_body_util::BodyExt;
        use tower::ServiceExt;

        let router = app_with_oauth_state(enabled_oauth_state(true, false)).await;
        let req = Request::builder()
            .method("GET")
            .uri("/.well-known/oauth-authorization-server")
            .body(Body::empty())
            .expect("build request");
        let response = router.oneshot(req).await.expect("call router");
        assert_eq!(response.status(), StatusCode::OK);

        let body = response
            .into_body()
            .collect()
            .await
            .expect("collect body")
            .to_bytes();
        let meta: AuthorizationServerMetadata =
            serde_json::from_slice(&body).expect("parse metadata");

        let reg_ep = meta
            .registration_endpoint
            .expect("registration_endpoint present");
        assert!(
            reg_ep.ends_with("/oauth/register"),
            "registration_endpoint must end with /oauth/register"
        );
    }
}
