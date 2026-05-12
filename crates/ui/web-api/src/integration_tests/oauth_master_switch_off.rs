// Tests that all OAuth surfaces return 404 when oauth.mcp_enabled = false.

use http::StatusCode;

use crate::test_harness::TestApp;
use crate::test_harness::fixtures::register_and_get_token;

/// All MCP OAuth surfaces must return 404 when `oauth.mcp_enabled = false`.
///
/// `TestApp::new()` wires up `OAuthState::disabled()` by default, so no
/// special setup is needed.
///
/// Note on form extraction: Axum deserialises `Query<T>` / `Form<T>` *before*
/// calling the handler, so routes with required parameters receive a 400
/// (parse error) if the params are absent — before the `oauth.enabled` guard
/// can fire.  To reach the guard we must supply well-formed parameters.
#[tokio::test]
async fn master_switch_off_returns_404_for_all_surfaces() {
    // TestApp::new() creates with OAuth disabled by default (OAuthState::disabled()).
    let app = TestApp::new().await;
    let client = app.client();

    // GET /.well-known/oauth-authorization-server — no params needed.
    let status = client
        .get("/.well-known/oauth-authorization-server")
        .send_status()
        .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "expected 404 for /.well-known/oauth-authorization-server",
    );

    // GET /oauth/authorize — Query<AuthorizeRequest> extraction requires all
    // fields; supply a well-formed (but semantically invalid) request so the
    // extractor succeeds and the enabled guard fires.
    let authorize_qs = concat!(
        "response_type=code",
        "&client_id=test-client",
        "&redirect_uri=https%3A%2F%2Fexample.com%2Fcb",
        "&scope=mcp%3Aread",
        "&state=teststate",
        "&code_challenge=abc123",
        "&code_challenge_method=S256",
        "&resource=https%3A%2F%2Fexample.com%2Fmcp",
    );
    let status = client
        .get(&format!("/oauth/authorize?{authorize_qs}"))
        .send_status()
        .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "expected 404 for /oauth/authorize"
    );

    // POST /oauth/token — Form<TokenRequest> extraction requires a valid body.
    let token_body = concat!(
        "grant_type=authorization_code",
        "&code=testcode",
        "&redirect_uri=https%3A%2F%2Fexample.com%2Fcb",
        "&client_id=test-client",
        "&code_verifier=testverifier",
        "&resource=https%3A%2F%2Fexample.com%2Fmcp",
    );
    let status = client
        .post_form("/oauth/token", token_body)
        .send_status()
        .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "expected 404 for /oauth/token"
    );

    // POST /oauth/register — Json<DcrRegistrationRequest> extraction requires
    // all non-optional fields; supply a well-formed body so the extractor
    // succeeds and the enabled guard fires.
    let status = client
        .post_json(
            "/oauth/register",
            &serde_json::json!({
                "client_name": "test",
                "redirect_uris": ["https://example.com/cb"],
                "grant_types": ["authorization_code"],
                "response_types": ["code"],
                "token_endpoint_auth_method": "none",
            }),
        )
        .send_status()
        .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "expected 404 for /oauth/register"
    );

    // Authenticated operator surfaces — register a user first.
    let token = register_and_get_token(&client).await;
    for path in ["/api/oauth/clients", "/api/oauth/consents"] {
        let status = client.get(path).bearer(&token).send_status().await;
        assert_eq!(status, StatusCode::NOT_FOUND, "expected 404 for {path}");
    }
}
