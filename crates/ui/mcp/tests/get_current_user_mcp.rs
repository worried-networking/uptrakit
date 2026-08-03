#![cfg(all(test, feature = "db-sqlite"))]

use std::net::SocketAddr;
use std::sync::Arc;

use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use reqwest::Client;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectOptions, Database, DatabaseConnection, EntityTrait,
    QueryFilter, Set,
};
use serde_json::{Value, json};
use time::OffsetDateTime;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use uptrakit_audit_log::{
    AuditEmitter, AuditLogBackend, AuditLogDispatcher, DatabaseBackend, NoopBackend,
};
use uptrakit_controller_core::access::AccessEngine;
use uptrakit_controller_core::auth::{
    AuthState, DeviceFlowStore, JwtManager, RateLimitStore, TokenDenylist,
};
use uptrakit_controller_core::db::DbState;
use uptrakit_controller_core::settings::Settings;
use uptrakit_controller_core::update::NoopUpdateDispatcher;
use uptrakit_mcp::build_mcp_router;
use uptrakit_mcp::state::McpState;
use uptrakit_shared_db::entity::{role, tenant, user, user_role};
use uptrakit_shared_db::migration::run_migrations;
use uptrakit_shared_types::MaskedEmail;
use uptrakit_web_api_auth::auth::api_token::ApiTokenService;
use uptrakit_web_api_auth::auth::registration::{RegistrationMode, RegistrationSettings};
use uptrakit_web_api_types::oauth::{McpAccessTokenClaims, McpOAuthJwtVerifier};

// ── Constants ─────────────────────────────────────────────────────────────────

const TEST_JWT_SECRET: &[u8] = b"mcp-integration-test-secret-32b!";
const TEST_ISSUER: &str = "https://controller.test";
const TEST_AUD: &str = "https://controller.test/mcp";
const INTERNAL_JWT_SECRET: &[u8] = b"mcp-test-internal-jwt-secret-32b";

// ── McpTestApp ────────────────────────────────────────────────────────────────

struct McpTestApp {
    addr: SocketAddr,
    db: DatabaseConnection,
    tenant_id: Uuid,
    cancel: CancellationToken,
    engine: Arc<AccessEngine>,
}

impl McpTestApp {
    async fn new() -> Self {
        Self::build(false, None).await
    }

    async fn new_with_oauth() -> Self {
        let verifier = McpOAuthJwtVerifier::new(
            TEST_JWT_SECRET,
            TEST_ISSUER.to_string(),
            vec![TEST_AUD.to_string()],
        );
        Self::build(true, Some(Arc::new(verifier))).await
    }

    #[expect(clippy::unwrap_used, reason = "test harness — panic on setup failure")]
    async fn build(oauth_enabled: bool, oauth_verifier: Option<Arc<McpOAuthJwtVerifier>>) -> Self {
        // Required so EncryptedString fields (token hash, audit log) work without
        // a real master key in the test process.
        uptrakit_crypto::enable_plaintext_mode();

        let opt = ConnectOptions::new("sqlite::memory:");
        let db = Database::connect(opt).await.unwrap();
        run_migrations(&db).await.unwrap();

        let tenant_id = insert_tenant(&db).await;

        let jwt = Arc::new(JwtManager::from_secret(INTERNAL_JWT_SECRET));

        let settings = Settings::new(
            RegistrationSettings {
                mode: RegistrationMode::Open,
                token_hash: None,
                require_token_for_oidc: false,
            },
            168,
        );
        // Set SANs so rmcp host-header validation is exercised (empty → allow-all).
        // Portless "127.0.0.1" matches Host: 127.0.0.1:<any-port> per rmcp rules.
        settings.set_sans(vec!["127.0.0.1".to_string()]).await;

        let audit_db_backend: Arc<dyn AuditLogBackend> = Arc::new(DatabaseBackend::new(db.clone()));
        let audit_emitter = AuditEmitter::with_backends(
            AuditLogDispatcher::new(Arc::clone(&audit_db_backend)),
            Arc::clone(&audit_db_backend),
            Arc::new(NoopBackend),
        );

        let cancel = CancellationToken::new();

        let engine = Arc::new(AccessEngine::new(db.clone()));

        let state = McpState::new(
            DbState::new(db.clone()),
            AuthState::new(
                Arc::clone(&jwt),
                DeviceFlowStore::new(db.clone()),
                RateLimitStore::new(db.clone()),
                Arc::new(TokenDenylist::new()),
            ),
            Arc::clone(&engine),
            settings,
            tenant_id,
            Uuid::nil(),
            audit_emitter,
            cancel.child_token(),
            Arc::new(NoopUpdateDispatcher),
            oauth_enabled,
            oauth_verifier,
            None,
        );

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let router = build_mcp_router(state);
        let ct = cancel.clone();
        tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(ct.cancelled_owned())
                .await
                .unwrap();
        });

        Self {
            addr,
            db,
            tenant_id,
            cancel,
            engine,
        }
    }
}

impl Drop for McpTestApp {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

// ── Fixture helpers ───────────────────────────────────────────────────────────

#[expect(clippy::unwrap_used, reason = "test fixture — panic on setup failure")]
async fn insert_tenant(db: &DatabaseConnection) -> Uuid {
    let id = Uuid::now_v7();
    let now = OffsetDateTime::now_utc();
    tenant::ActiveModel {
        id: Set(id),
        name: Set("test-tenant".to_string()),
        slug: Set(id.to_string()),
        is_default: Set(true),
        created_at: Set(now),
        updated_at: Set(now),
        deactivated_at: Set(None),
    }
    .insert(db)
    .await
    .unwrap();
    id
}

#[expect(clippy::unwrap_used, reason = "test fixture — panic on setup failure")]
async fn insert_user(db: &DatabaseConnection, email: &str) -> Uuid {
    let user_id = Uuid::now_v7();
    let now = OffsetDateTime::now_utc();
    user::ActiveModel {
        id: Set(user_id),
        email: Set(MaskedEmail::new(email)),
        first_name: Set("Test".to_string()),
        last_name: Set("User".to_string()),
        password_hash: Set(None),
        is_active: Set(true),
        deactivated_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(db)
    .await
    .unwrap();
    user_id
}

/// Link user to the "viewer" built-in role, which has `access_mcp` via
/// migration m20260424_000001_access_mcp_permission. Do NOT use "owner"
/// (removed in m20260310_000002_granular_permissions).
///
/// Returns the viewer role's `Uuid` so callers can look up its seeded
/// `mcp:use` access grant via `uptrakit_shared_db::access_grants`.
#[expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test fixture — panic on setup failure"
)]
async fn link_user_to_access_mcp_role(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    user_id: Uuid,
) -> Uuid {
    let viewer_role = role::Entity::find()
        .filter(role::Column::Name.eq("viewer"))
        .one(db)
        .await
        .unwrap()
        .expect("viewer role must exist after migrations");

    let now = OffsetDateTime::now_utc();
    user_role::ActiveModel {
        tenant_id: Set(tenant_id),
        user_id: Set(user_id),
        role_id: Set(viewer_role.id),
        assigned_at: Set(now),
    }
    .insert(db)
    .await
    .unwrap();

    viewer_role.id
}

/// Link user to the "operator" built-in role, whose seed grant
/// (`m20260728_000002_seed_access_grants`) carries `updates:trigger` — unlike
/// viewer's `*:read`, which does not cover a `:trigger` verb.
#[expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test fixture — panic on setup failure"
)]
async fn link_user_to_operator_role(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    user_id: Uuid,
) -> Uuid {
    let operator_role = role::Entity::find()
        .filter(role::Column::Name.eq("operator"))
        .one(db)
        .await
        .unwrap()
        .expect("operator role must exist after migrations");

    let now = OffsetDateTime::now_utc();
    user_role::ActiveModel {
        tenant_id: Set(tenant_id),
        user_id: Set(user_id),
        role_id: Set(operator_role.id),
        assigned_at: Set(now),
    }
    .insert(db)
    .await
    .unwrap();

    operator_role.id
}

#[expect(clippy::unwrap_used, reason = "test fixture — panic on setup failure")]
async fn create_api_token(db: &DatabaseConnection, user_id: Uuid) -> String {
    let service = ApiTokenService::new(db.clone());
    service
        .create_token(user_id, "integration-test-token")
        .await
        .unwrap()
        .plaintext_token
}

// ── Protocol helpers ──────────────────────────────────────────────────────────

/// Parse the first non-empty data line from an SSE body.
///
/// Stateful-mode POST responses begin with a priming event (`data:` with empty
/// payload) then emit the actual JSON-RPC response. Skips empty chunks.
#[expect(
    clippy::panic,
    reason = "test helper — explicit failure on SSE parse error"
)]
fn extract_sse_result(body: &str) -> Value {
    for chunk in body.split("\n\n") {
        for line in chunk.lines() {
            let data = line.strip_prefix("data:").map(str::trim).unwrap_or("");
            if !data.is_empty() {
                return serde_json::from_str(data)
                    .unwrap_or_else(|e| panic!("SSE data is not valid JSON: {e}\ndata: {data}"));
            }
        }
    }
    panic!("no non-empty SSE data line found in body:\n{body}")
}

struct McpSession {
    client: Client,
    base: String,
    session_id: String,
    bearer: String,
}

impl McpSession {
    #[expect(
        clippy::unwrap_used,
        clippy::expect_used,
        reason = "test helper — panic on protocol failure"
    )]
    async fn initialize(addr: SocketAddr, bearer: &str) -> Self {
        let client = Client::new();
        let base = format!("http://{addr}/mcp");

        let init_resp = client
            .post(&base)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .header("Authorization", format!("Bearer {bearer}"))
            .body(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}"#)
            .send()
            .await
            .unwrap();

        assert_eq!(
            init_resp.status().as_u16(),
            200,
            "initialize must return 200"
        );

        let session_id = init_resp
            .headers()
            .get("Mcp-Session-Id")
            .expect("Mcp-Session-Id header must be present")
            .to_str()
            .unwrap()
            .to_string();

        let notif_resp = client
            .post(&base)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .header("Authorization", format!("Bearer {bearer}"))
            .header("Mcp-Session-Id", &session_id)
            .body(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)
            .send()
            .await
            .unwrap();

        assert_eq!(
            notif_resp.status().as_u16(),
            202,
            "notifications/initialized must return 202"
        );

        Self {
            client,
            base,
            session_id,
            bearer: bearer.to_string(),
        }
    }

    #[expect(
        clippy::unwrap_used,
        reason = "test helper — panic on protocol failure"
    )]
    async fn call_tool(&self, name: &str, args: Value) -> Value {
        let body = json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": { "name": name, "arguments": args }
        });

        let resp = self
            .client
            .post(&self.base)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .header("Authorization", format!("Bearer {}", self.bearer))
            .header("Mcp-Session-Id", &self.session_id)
            .header("Mcp-Protocol-Version", "2025-03-26")
            .json(&body)
            .send()
            .await
            .unwrap();

        assert_eq!(resp.status().as_u16(), 200, "tools/call must return 200");
        let text = resp.text().await.unwrap();
        extract_sse_result(&text)
    }
}

// ── JWT minting ───────────────────────────────────────────────────────────────

#[expect(
    clippy::unwrap_used,
    reason = "test helper — unwrap on infallible token encoding"
)]
fn mint_test_jwt(user_id: Uuid, tenant_id: Uuid) -> String {
    let claims = McpAccessTokenClaims::new(
        TEST_ISSUER.to_string(),
        user_id.to_string(),
        TEST_AUD.to_string(),
        Uuid::new_v4().to_string(), // client_id — non-empty UUID string
        "mcp:read".to_string(),
        Uuid::new_v4().to_string(), // jti — non-empty UUID string
        1,
        1,
        9_999_999_999,
        tenant_id.to_string(),
    );

    let mut header = Header::new(Algorithm::HS256);
    header.typ = Some("at+jwt".to_string());
    encode(&header, &claims, &EncodingKey::from_secret(TEST_JWT_SECRET)).unwrap()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn api_token_get_current_user_succeeds() {
    let app = McpTestApp::new().await;
    let user_id = insert_user(&app.db, "owner@mcp.test").await;
    link_user_to_access_mcp_role(&app.db, app.tenant_id, user_id).await;
    let token = create_api_token(&app.db, user_id).await;

    let session = McpSession::initialize(app.addr, &token).await;
    let result = session.call_tool("get_current_user", json!({})).await;

    let text = result["result"]["content"][0]["text"]
        .as_str()
        .expect("content[0].text must be a string");
    let parsed: Value = serde_json::from_str(text).expect("text must be valid JSON");

    assert_eq!(
        parsed["email"].as_str().unwrap(),
        "owner@mcp.test",
        "email must match"
    );
    assert_eq!(
        parsed["user_id"].as_str().unwrap(),
        user_id.to_string(),
        "user_id must match"
    );
}

#[tokio::test]
async fn oauth_jwt_get_current_user_succeeds() {
    let app = McpTestApp::new_with_oauth().await;
    let user_id = insert_user(&app.db, "oauth@mcp.test").await;
    link_user_to_access_mcp_role(&app.db, app.tenant_id, user_id).await;
    let token = mint_test_jwt(user_id, app.tenant_id);

    let session = McpSession::initialize(app.addr, &token).await;
    let result = session.call_tool("get_current_user", json!({})).await;

    let text = result["result"]["content"][0]["text"]
        .as_str()
        .expect("content[0].text must be a string");
    let parsed: Value = serde_json::from_str(text).expect("text must be valid JSON");

    assert_eq!(
        parsed["email"].as_str().unwrap(),
        "oauth@mcp.test",
        "email must match"
    );
    assert_eq!(
        parsed["user_id"].as_str().unwrap(),
        user_id.to_string(),
        "user_id must match"
    );
}

#[tokio::test]
async fn missing_access_mcp_permission_returns_403() {
    let app = McpTestApp::new().await;
    let user_id = insert_user(&app.db, "unpriv@mcp.test").await;
    let token = create_api_token(&app.db, user_id).await;

    // McpAuthLayer returns 403 before the MCP protocol layer — no session
    // initialization needed.
    let client = Client::new();
    let resp = client
        .post(format!("http://{}/mcp", app.addr))
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .header("Authorization", format!("Bearer {token}"))
        .body(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}"#)
        .send()
        .await
        .expect("send request");

    assert_eq!(
        resp.status().as_u16(),
        403,
        "missing AccessMcp must return 403"
    );
    // Confirm the 403 came from McpAuthLayer, not the MCP protocol layer.
    // The MCP protocol layer would have set Mcp-Session-Id before returning;
    // McpAuthLayer short-circuits before it.
    assert!(
        resp.headers().get("Mcp-Session-Id").is_none(),
        "McpAuthLayer must not set Mcp-Session-Id on 403"
    );
}

/// Engine-gate discriminator: legacy role_permissions still grant
/// access_mcp, but the engine grant row is gone -> 403 only once the
/// gate reads the engine.
#[tokio::test]
async fn api_token_denied_when_mcp_use_grant_removed() {
    let app = McpTestApp::new().await;
    let user_id = insert_user(&app.db, "no-grant@example.com").await;
    let viewer_role_id = link_user_to_access_mcp_role(&app.db, app.tenant_id, user_id).await;
    let token = create_api_token(&app.db, user_id).await;

    // access_grants is engine-owned: reach it only via the public
    // choke-point module, never the sealed entity.
    let load = uptrakit_shared_db::access_grants::load_grants_for_principal(
        &app.db,
        app.tenant_id,
        user_id,
        &[viewer_role_id],
    )
    .await
    .expect("load grants");
    let mcp_grant = load
        .grants
        .iter()
        .find(|grant| grant.patterns.iter().any(|p| p.to_string() == "mcp:use"))
        .expect("mcp:use backfill grant must exist after migrations");
    uptrakit_shared_db::access_grants::delete_grant(&app.db, mcp_grant.id)
        .await
        .expect("delete grant");
    // Not strictly needed on a cold cache, but makes the test assert the
    // intended semantic instead of depending on cache-warming order.
    app.engine.invalidate_subjects(&[user_id], &[]);

    let client = Client::new();
    let resp = client
        .post(format!("http://{}/mcp", app.addr))
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .header("Authorization", format!("Bearer {token}"))
        .body(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}"#)
        .send()
        .await
        .expect("send request");

    assert_eq!(
        resp.status().as_u16(),
        403,
        "removed mcp:use grant must return 403"
    );
    assert!(
        resp.headers().get("Mcp-Session-Id").is_none(),
        "McpAuthLayer must not set Mcp-Session-Id on 403"
    );
}

/// Same discriminator as `api_token_denied_when_mcp_use_grant_removed`, but
/// for the OAuth JWT path.
#[tokio::test]
async fn oauth_denied_when_mcp_use_grant_removed() {
    let app = McpTestApp::new_with_oauth().await;
    let user_id = insert_user(&app.db, "no-grant-oauth@example.com").await;
    let viewer_role_id = link_user_to_access_mcp_role(&app.db, app.tenant_id, user_id).await;
    let token = mint_test_jwt(user_id, app.tenant_id);

    let load = uptrakit_shared_db::access_grants::load_grants_for_principal(
        &app.db,
        app.tenant_id,
        user_id,
        &[viewer_role_id],
    )
    .await
    .expect("load grants");
    let mcp_grant = load
        .grants
        .iter()
        .find(|grant| grant.patterns.iter().any(|p| p.to_string() == "mcp:use"))
        .expect("mcp:use backfill grant must exist after migrations");
    uptrakit_shared_db::access_grants::delete_grant(&app.db, mcp_grant.id)
        .await
        .expect("delete grant");
    app.engine.invalidate_subjects(&[user_id], &[]);

    let client = Client::new();
    let resp = client
        .post(format!("http://{}/mcp", app.addr))
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .header("Authorization", format!("Bearer {token}"))
        .body(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}"#)
        .send()
        .await
        .expect("send request");

    assert_eq!(
        resp.status().as_u16(),
        403,
        "removed mcp:use grant must return 403"
    );
    assert!(
        resp.headers().get("Mcp-Session-Id").is_none(),
        "McpAuthLayer must not set Mcp-Session-Id on 403"
    );
}

/// Allow-path: viewer role intact with its seeded `mcp:use` grant ->
/// the engine gate admits the request on both auth paths.
#[tokio::test]
async fn api_token_allowed_when_mcp_use_grant_present() {
    let app = McpTestApp::new().await;
    let user_id = insert_user(&app.db, "with-grant-api@example.com").await;
    link_user_to_access_mcp_role(&app.db, app.tenant_id, user_id).await;
    let token = create_api_token(&app.db, user_id).await;

    let client = Client::new();
    let resp = client
        .post(format!("http://{}/mcp", app.addr))
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .header("Authorization", format!("Bearer {token}"))
        .body(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}"#)
        .send()
        .await
        .expect("send request");

    assert_eq!(
        resp.status().as_u16(),
        200,
        "intact mcp:use grant must allow the request"
    );
}

#[tokio::test]
async fn oauth_allowed_when_mcp_use_grant_present() {
    let app = McpTestApp::new_with_oauth().await;
    let user_id = insert_user(&app.db, "with-grant-oauth@example.com").await;
    link_user_to_access_mcp_role(&app.db, app.tenant_id, user_id).await;
    let token = mint_test_jwt(user_id, app.tenant_id);

    let client = Client::new();
    let resp = client
        .post(format!("http://{}/mcp", app.addr))
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .header("Authorization", format!("Bearer {token}"))
        .body(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}"#)
        .send()
        .await
        .expect("send request");

    assert_eq!(
        resp.status().as_u16(),
        200,
        "intact mcp:use grant must allow the request"
    );
}

/// Proves in-process cache coherence on the MCP path: a successful call,
/// then the user's role link is removed and the engine cache invalidated,
/// then the next call must be denied immediately (no cache staleness
/// window). The NATS cross-process invalidation leg is covered elsewhere
/// and is not this test's claim.
#[tokio::test]
async fn api_token_denied_immediately_after_role_removed_and_invalidated() {
    let app = McpTestApp::new().await;
    let user_id = insert_user(&app.db, "revoke@example.com").await;
    link_user_to_access_mcp_role(&app.db, app.tenant_id, user_id).await;
    let token = create_api_token(&app.db, user_id).await;

    let client = Client::new();
    let init_body = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}"#;

    let first = client
        .post(format!("http://{}/mcp", app.addr))
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .header("Authorization", format!("Bearer {token}"))
        .body(init_body)
        .send()
        .await
        .expect("send first request");
    assert_eq!(first.status().as_u16(), 200, "first call must succeed");

    user_role::Entity::delete_many()
        .filter(user_role::Column::UserId.eq(user_id))
        .exec(&app.db)
        .await
        .expect("delete user_roles");
    app.engine.invalidate_subjects(&[user_id], &[]);

    let second = client
        .post(format!("http://{}/mcp", app.addr))
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .header("Authorization", format!("Bearer {token}"))
        .body(init_body)
        .send()
        .await
        .expect("send second request");
    assert_eq!(
        second.status().as_u16(),
        403,
        "role removal + invalidation must deny the very next call"
    );
}

// ── require_tool_auth: per-tool catalog action enforcement ─────────────────────

/// Viewer's seeded `*:read` grant does not cover `updates:trigger` — the tool
/// must be denied by `require_tool_auth`'s engine check, not merely by scope.
#[tokio::test]
async fn trigger_update_denied_for_viewer_only() {
    let app = McpTestApp::new().await;
    let user_id = insert_user(&app.db, "viewer-only@mcp.test").await;
    link_user_to_access_mcp_role(&app.db, app.tenant_id, user_id).await;
    let token = create_api_token(&app.db, user_id).await;

    let session = McpSession::initialize(app.addr, &token).await;
    let result = session
        .call_tool(
            "trigger_update",
            json!({
                "host_id": Uuid::nil().to_string(),
                "software_item_id": Uuid::nil().to_string(),
                "to_version": "1.0.0",
            }),
        )
        .await;

    let message = result["error"]["message"]
        .as_str()
        .expect("error.message must be a string for a permission-denied tool call");
    assert!(
        message.contains("permission denied: updates:trigger required"),
        "expected permission-denied message, got: {message}"
    );
}

/// Viewer + operator: operator's seed grant carries `updates:trigger`, so the
/// engine check passes and the call reaches `NoopUpdateDispatcher`, which
/// unconditionally fails with `UpdateDispatchError::Internal` regardless of
/// input — proving the gate was passed without needing a working dispatcher.
#[tokio::test]
async fn trigger_update_proceeds_past_gate_for_viewer_and_operator() {
    let app = McpTestApp::new().await;
    let user_id = insert_user(&app.db, "viewer-operator@mcp.test").await;
    link_user_to_access_mcp_role(&app.db, app.tenant_id, user_id).await;
    link_user_to_operator_role(&app.db, app.tenant_id, user_id).await;
    let token = create_api_token(&app.db, user_id).await;

    let session = McpSession::initialize(app.addr, &token).await;
    let result = session
        .call_tool(
            "trigger_update",
            json!({
                "host_id": Uuid::nil().to_string(),
                "software_item_id": Uuid::nil().to_string(),
                "to_version": "1.0.0",
            }),
        )
        .await;

    let message = result["error"]["message"]
        .as_str()
        .expect("error.message must be a string for a dispatcher-failed tool call");
    assert!(
        !message.contains("permission denied"),
        "call must pass the permission gate, got: {message}"
    );
    assert!(
        message.contains("trigger_update failed: internal error"),
        "expected NoopUpdateDispatcher's Internal error to surface, got: {message}"
    );
}

/// Viewer's `*:read` grant covers `software:read`, so `list_update_history`
/// (a read-only tool) is allowed for a viewer-only user.
#[tokio::test]
async fn list_update_history_allowed_for_viewer() {
    let app = McpTestApp::new().await;
    let user_id = insert_user(&app.db, "viewer-history@mcp.test").await;
    link_user_to_access_mcp_role(&app.db, app.tenant_id, user_id).await;
    let token = create_api_token(&app.db, user_id).await;

    let session = McpSession::initialize(app.addr, &token).await;
    let result = session.call_tool("list_update_history", json!({})).await;

    assert!(
        result.get("error").is_none(),
        "list_update_history must not error for a viewer, got: {result:?}"
    );
    let text = result["result"]["content"][0]["text"]
        .as_str()
        .expect("content[0].text must be a string");
    let parsed: Value = serde_json::from_str(text).expect("text must be valid JSON");
    assert!(
        parsed["items"].as_array().is_some(),
        "expected a paginated items array, got: {parsed:?}"
    );
}
