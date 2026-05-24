# MCP `get_current_user` Integration Tests Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add in-process integration tests that exercise the full
`McpAuthLayer → StreamableHttpService → McpRequestContext → DB` pipeline for
`get_current_user`, and fix the broken Docker e2e test that sends `tools/call`
without an `initialize` handshake.

**Architecture:** Each test spins up a real TCP listener, wires `McpState` backed
by an in-memory SQLite database, and drives it via `reqwest` through the full MCP
stateful-SSE handshake (`initialize` → `notifications/initialized` → `tools/call`).
The Docker e2e test is fixed by replacing its bare `tools/call` request with the
correct three-step session flow.

**Tech Stack:** Rust, axum, rmcp, reqwest, sea-orm (SQLite), tokio, jsonwebtoken

---

## File map

| File | Action |
| --- | --- |
| `crates/ui/mcp/Cargo.toml` | Add `db-sqlite` feature + dev-deps |
| `crates/ui/mcp/tests/get_current_user_mcp.rs` | **Create** — test file (all new) |
| `crates/core/integration-tests/tests/oauth_end_to_end.rs` | Fix broken step 9 |

---

## Task 1: Add `db-sqlite` feature and dev-dependencies to `uptrakit-mcp`

**Files:**

- Modify: `crates/ui/mcp/Cargo.toml`

- [ ] **Step 1: Add the feature and dev-deps**

  Open `crates/ui/mcp/Cargo.toml`. Add the following after the existing
  `[dependencies]` block (and before `[lints]`):

  ```toml
  [features]
  db-sqlite = ["uptrakit-shared-db/db-sqlite", "sea-orm/sqlx-sqlite"]

  [dev-dependencies]
  uptrakit-web-api-auth  = { workspace = true }
  uptrakit-shared-types  = { workspace = true }
  uptrakit-crypto        = { workspace = true, features = ["testing"] }
  jsonwebtoken           = { workspace = true, features = ["aws_lc_rs"] }
  reqwest                = { workspace = true }
  time                   = { workspace = true }
  tokio                  = { workspace = true, features = ["rt-multi-thread", "macros"] }
  ```

  **Note:** `jsonwebtoken` is already in `[dev-dependencies]` — remove the old
  line and keep only the new one (with `features = ["aws_lc_rs"]`).

- [ ] **Step 2: Verify the crate compiles with the new feature**

  ```bash
  cargo check -p uptrakit-mcp --features db-sqlite
  ```

  Expected: clean compile, zero errors.

- [ ] **Step 3: Commit**

  ```bash
  git add crates/ui/mcp/Cargo.toml
  git commit -m "chore(mcp): add db-sqlite feature + integration test dev-deps"
  ```

---

## Task 2: Create test file skeleton with `McpTestApp` harness

**Files:**

- Create: `crates/ui/mcp/tests/get_current_user_mcp.rs`

- [ ] **Step 1: Write the file skeleton with imports and `McpTestApp`**

  Create `crates/ui/mcp/tests/get_current_user_mcp.rs` with the following content:

  ```rust
  #![cfg(all(test, feature = "db-sqlite"))]

  use std::net::SocketAddr;
  use std::sync::Arc;

  use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
  use reqwest::Client;
  use sea_orm::{
      ActiveModelTrait, ColumnTrait, ConnectOptions, Database, DatabaseConnection,
      EntityTrait, QueryFilter, Set,
  };
  use serde_json::{Value, json};
  use time::OffsetDateTime;
  use tokio::net::TcpListener;
  use tokio_util::sync::CancellationToken;
  use uuid::Uuid;

  use uptrakit_audit_log::{
      AuditEmitter, AuditLogBackend, AuditLogDispatcher, DatabaseBackend, NoopBackend,
  };
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

  // ── Constants ────────────────────────────────────────────────────────────────

  /// JWT secret used by McpTestApp::new_with_oauth and mint_test_jwt.
  const TEST_JWT_SECRET: &[u8] = b"mcp-integration-test-secret-32b!";
  /// JWT issuer used in oauth tests.
  const TEST_ISSUER: &str = "https://controller.test";
  /// JWT audience used in oauth tests — must match what McpState is configured with.
  const TEST_AUD: &str = "https://controller.test/mcp";
  /// Secret for the internal JwtManager (session tokens, unrelated to MCP OAuth).
  const INTERNAL_JWT_SECRET: &[u8] = b"mcp-test-internal-jwt-secret-32b";

  // ── McpTestApp ───────────────────────────────────────────────────────────────

  struct McpTestApp {
      /// Address the axum server is bound to.
      addr: SocketAddr,
      /// Database handle for fixture queries.
      db: DatabaseConnection,
      /// The tenant seeded during setup.
      tenant_id: Uuid,
      /// Cancel token — cancel() on Drop shuts down axum::serve.
      cancel: CancellationToken,
  }

  impl McpTestApp {
      /// Spin up a test server with API-token auth only (oauth_enabled = false).
      async fn new() -> Self {
          Self::build(false, None).await
      }

      /// Spin up a test server with OAuth JWT auth enabled.
      ///
      /// Uses TEST_JWT_SECRET / TEST_ISSUER / TEST_AUD — callers must mint tokens
      /// with mint_test_jwt() to match.
      async fn new_with_oauth() -> Self {
          let verifier = McpOAuthJwtVerifier::new(
              TEST_JWT_SECRET,
              TEST_ISSUER.to_string(),
              vec![TEST_AUD.to_string()],
          );
          Self::build(true, Some(Arc::new(verifier))).await
      }

      async fn build(
          oauth_enabled: bool,
          oauth_verifier: Option<Arc<McpOAuthJwtVerifier>>,
      ) -> Self {
          // Enable plaintext mode so EncryptedString fields (token hash, audit log)
          // work without a real master key in the test environment.
          uptrakit_crypto::enable_plaintext_mode();

          let opt = ConnectOptions::new("sqlite::memory:");
          let db = Database::connect(opt).await.expect("connect to sqlite");
          run_migrations(&db).await.expect("run migrations");

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
          // Set SANs so rmcp host-header validation is exercised rather than
          // bypassed (empty allowed_hosts → allow-all in rmcp).
          // The portless "127.0.0.1" entry matches Host: 127.0.0.1:<any-port>.
          settings.set_sans(vec!["127.0.0.1".to_string()]).await;

          let audit_db_backend: Arc<dyn AuditLogBackend> =
              Arc::new(DatabaseBackend::new(db.clone()));
          let audit_emitter = AuditEmitter::with_backends(
              AuditLogDispatcher::new(Arc::clone(&audit_db_backend)),
              Arc::clone(&audit_db_backend),
              Arc::new(NoopBackend),
          );

          let cancel = CancellationToken::new();

          let state = McpState::new(
              DbState::new(db.clone()),
              AuthState::new(
                  Arc::clone(&jwt),
                  DeviceFlowStore::new(db.clone()),
                  RateLimitStore::new(db.clone()),
                  Arc::new(TokenDenylist::new()),
              ),
              settings,
              tenant_id,
              Uuid::nil(), // controller_id — unused in these tests
              audit_emitter,
              cancel.child_token(), // shutdown_token for McpState internal use
              Arc::new(NoopUpdateDispatcher),
              oauth_enabled,
              oauth_verifier,
              None, // oauth_canonical
          );

          let listener = TcpListener::bind("127.0.0.1:0")
              .await
              .expect("bind TCP listener");
          let addr = listener.local_addr().expect("local_addr");

          let router = build_mcp_router(state);
          let ct = cancel.clone();
          tokio::spawn(async move {
              axum::serve(listener, router)
                  .with_graceful_shutdown(ct.cancelled_owned())
                  .await
                  .expect("axum::serve");
          });

          Self { addr, db, tenant_id, cancel }
      }
  }

  impl Drop for McpTestApp {
      fn drop(&mut self) {
          // cancel() is sync (atomic flag flip) — safe from Drop.
          self.cancel.cancel();
      }
  }

  // ── Helper: insert tenant ────────────────────────────────────────────────────

  #[expect(
      clippy::unwrap_used,
      reason = "test fixture — panic on setup failure"
  )]
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
  ```

- [ ] **Step 2: Verify the skeleton compiles**

  ```bash
  cargo check -p uptrakit-mcp --features db-sqlite --tests
  ```

  Expected: zero errors. Fix any import issues before continuing.

- [ ] **Step 3: Commit the skeleton**

  ```bash
  git add crates/ui/mcp/tests/get_current_user_mcp.rs
  git commit -m "test(mcp): scaffold McpTestApp harness for get_current_user integration tests"
  ```

---

## Task 3: Add fixture helpers

**Files:**

- Modify: `crates/ui/mcp/tests/get_current_user_mcp.rs`

- [ ] **Step 1: Add `insert_user`, `link_user_to_access_mcp_role`, and `create_api_token`**

  Append the following three functions to the test file (after `insert_tenant`):

  ```rust
  // ── Fixture helpers ──────────────────────────────────────────────────────────

  /// Insert an active user row; returns the new user_id.
  ///
  /// The user has no role by default — call link_user_to_access_mcp_role to
  /// grant AccessMcp.
  #[expect(
      clippy::unwrap_used,
      reason = "test fixture — panic on setup failure"
  )]
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

  /// Link a user to the "viewer" built-in role (which has the access_mcp
  /// permission via migration m20260424_000001_access_mcp_permission).
  ///
  /// Do NOT use the "owner" role — it was removed in
  /// m20260310_000002_granular_permissions.
  #[expect(
      clippy::unwrap_used,
      reason = "test fixture — panic on setup failure"
  )]
  async fn link_user_to_access_mcp_role(
      db: &DatabaseConnection,
      tenant_id: Uuid,
      user_id: Uuid,
  ) {
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
  }

  /// Create a upk_-prefixed API token for a user; returns the plaintext token.
  ///
  /// The plaintext token is only available at creation time.
  #[expect(
      clippy::unwrap_used,
      reason = "test fixture — panic on setup failure"
  )]
  async fn create_api_token(db: &DatabaseConnection, user_id: Uuid) -> String {
      let service = ApiTokenService::new(db.clone());
      let created = service
          .create_token(user_id, "integration-test-token")
          .await
          .unwrap();
      created.plaintext_token
  }
  ```

- [ ] **Step 2: Verify compilation**

  ```bash
  cargo check -p uptrakit-mcp --features db-sqlite --tests
  ```

  Expected: zero errors.

- [ ] **Step 3: Commit**

  ```bash
  git add crates/ui/mcp/tests/get_current_user_mcp.rs
  git commit -m "test(mcp): add fixture helpers for user/role/token insertion"
  ```

---

## Task 4: Add protocol helpers (`McpSession` + `extract_sse_result`)

**Files:**

- Modify: `crates/ui/mcp/tests/get_current_user_mcp.rs`

- [ ] **Step 1: Add `extract_sse_result`**

  Append after the fixture helpers:

  ```rust
  // ── Protocol helpers ─────────────────────────────────────────────────────────

  /// Parse the first non-empty data line from an SSE body.
  ///
  /// SSE streams in stateful mode begin with a priming event (`data:` with an
  /// empty payload) followed by the actual JSON response (`data: {...}`).
  /// This function skips empty data chunks and returns the first non-empty one.
  ///
  /// Panics with a descriptive message if no non-empty data line is found.
  #[expect(
      clippy::panic,
      reason = "test helper — explicit failure message on SSE parse error"
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
  ```

- [ ] **Step 2: Add `McpSession`**

  Append after `extract_sse_result`:

  ```rust
  struct McpSession {
      client: Client,
      /// Full URL prefix, e.g. "http://127.0.0.1:54321/mcp"
      base: String,
      session_id: String,
      /// Bearer token stored so call_tool() doesn't need it as a parameter.
      bearer: String,
  }

  impl McpSession {
      /// Perform the MCP initialize + notifications/initialized handshake.
      ///
      /// Returns an McpSession whose session_id can be used in subsequent calls.
      #[expect(
          clippy::unwrap_used,
          reason = "test helper — panic on protocol failure"
      )]
      async fn initialize(addr: SocketAddr, bearer: &str) -> Self {
          let client = Client::new();
          let base = format!("http://{addr}/mcp");

          // Step 1: POST initialize (no session ID yet)
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

          // Step 2: Extract session ID from response header
          let session_id = init_resp
              .headers()
              .get("Mcp-Session-Id")
              .expect("Mcp-Session-Id header must be present")
              .to_str()
              .unwrap()
              .to_string();

          // Step 3: POST notifications/initialized (acknowledge the session)
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

          Self { client, base, session_id, bearer: bearer.to_string() }
      }

      /// Call a tool and return the parsed SSE result JSON.
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
  ```

- [ ] **Step 3: Verify compilation**

  ```bash
  cargo check -p uptrakit-mcp --features db-sqlite --tests
  ```

  Expected: zero errors.

- [ ] **Step 4: Commit**

  ```bash
  git add crates/ui/mcp/tests/get_current_user_mcp.rs
  git commit -m "test(mcp): add McpSession protocol helper and extract_sse_result"
  ```

---

## Task 5: Test 1 — API token happy path

**Files:**

- Modify: `crates/ui/mcp/tests/get_current_user_mcp.rs`

- [ ] **Step 1: Append the test**

  ```rust
  // ── Tests ────────────────────────────────────────────────────────────────────

  #[tokio::test]
  async fn api_token_get_current_user_succeeds() {
      let app = McpTestApp::new().await;
      let user_id = insert_user(&app.db, "owner@mcp.test").await;
      link_user_to_access_mcp_role(&app.db, app.tenant_id, user_id).await;
      let token = create_api_token(&app.db, user_id).await;

      let session = McpSession::initialize(app.addr, &token).await;
      let result = session.call_tool("get_current_user", json!({})).await;

      // The MCP result structure: result.result.content[0].text is a JSON string.
      let text = result["result"]["content"][0]["text"]
          .as_str()
          .expect("content[0].text must be a string");
      let parsed: serde_json::Value =
          serde_json::from_str(text).expect("text must be valid JSON");

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
      assert!(
          parsed["permissions"]
              .as_array()
              .unwrap()
              .iter()
              .any(|p| p.as_str() == Some("access_mcp")),
          "permissions must contain access_mcp"
      );
  }
  ```

- [ ] **Step 2: Run the test**

  ```bash
  cargo test -p uptrakit-mcp --features db-sqlite api_token_get_current_user_succeeds -- --nocapture
  ```

  Expected:

  ```text
  test api_token_get_current_user_succeeds ... ok
  ```

  If it fails, common issues:
  - `enable_plaintext_mode()` not called before DB setup — check `McpTestApp::build`
  - Host header rejected (400) — the `set_sans` call must complete before `build_mcp_router`
  - SSE parse failure — run with `--nocapture` and print `text` to inspect the raw body

- [ ] **Step 3: Commit**

  ```bash
  git add crates/ui/mcp/tests/get_current_user_mcp.rs
  git commit -m "test(mcp): add api_token_get_current_user_succeeds integration test"
  ```

---

## Task 6: Test 2 — OAuth JWT happy path

**Files:**

- Modify: `crates/ui/mcp/tests/get_current_user_mcp.rs`

- [ ] **Step 1: Add the JWT minting helper (before the tests block)**

  Append after the `McpSession` impl:

  ```rust
  // ── JWT minting ──────────────────────────────────────────────────────────────

  /// Mint a test JWT for the given user and tenant.
  ///
  /// Mirrors the pattern in oauth_rs_audience_binding.rs.
  /// client_id and jti must be non-empty UUID strings (verified by McpOAuthJwtVerifier).
  #[expect(
      clippy::unwrap_used,
      reason = "test helper — unwrap on infallible token encoding"
  )]
  fn mint_test_jwt(user_id: Uuid, tenant_id: Uuid) -> String {
      let claims = McpAccessTokenClaims::new(
          TEST_ISSUER.to_string(),            // iss
          user_id.to_string(),                // sub
          TEST_AUD.to_string(),               // aud
          Uuid::new_v4().to_string(),         // client_id (non-empty UUID string)
          "mcp:read".to_string(),             // scope
          Uuid::new_v4().to_string(),         // jti (non-empty UUID string)
          1,                                  // iat
          1,                                  // nbf
          9_999_999_999,                      // exp (far future)
          tenant_id.to_string(),              // tenant_id
      );

      let mut header = Header::new(Algorithm::HS256);
      header.typ = Some("at+jwt".to_string());
      encode(&header, &claims, &EncodingKey::from_secret(TEST_JWT_SECRET)).unwrap()
  }
  ```

- [ ] **Step 2: Append the OAuth JWT test**

  ```rust
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
      let parsed: serde_json::Value =
          serde_json::from_str(text).expect("text must be valid JSON");

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
  ```

- [ ] **Step 3: Run the test**

  ```bash
  cargo test -p uptrakit-mcp --features db-sqlite oauth_jwt_get_current_user_succeeds -- --nocapture
  ```

  Expected:

  ```text
  test oauth_jwt_get_current_user_succeeds ... ok
  ```

  If you get HTTP 401 on initialize, check that `new_with_oauth()` sets
  `oauth_enabled: true` and the verifier is built with the same secret/issuer/aud
  as `mint_test_jwt()`.

- [ ] **Step 4: Commit**

  ```bash
  git add crates/ui/mcp/tests/get_current_user_mcp.rs
  git commit -m "test(mcp): add oauth_jwt_get_current_user_succeeds integration test"
  ```

---

## Task 7: Test 3 — Missing `AccessMcp` returns 403

**Files:**

- Modify: `crates/ui/mcp/tests/get_current_user_mcp.rs`

- [ ] **Step 1: Append the 403 test**

  ```rust
  #[tokio::test]
  async fn missing_access_mcp_permission_returns_403() {
      let app = McpTestApp::new().await;
      // Create a user with NO role link — no AccessMcp permission.
      let user_id = insert_user(&app.db, "unpriv@mcp.test").await;
      let token = create_api_token(&app.db, user_id).await;

      // McpAuthLayer returns 403 before the MCP protocol layer — no session
      // initialization needed, just send any valid MCP request body.
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
  }
  ```

- [ ] **Step 2: Run all three tests**

  ```bash
  cargo test -p uptrakit-mcp --features db-sqlite -- --nocapture
  ```

  Expected output:

  ```text
  test api_token_get_current_user_succeeds ... ok
  test oauth_jwt_get_current_user_succeeds ... ok
  test missing_access_mcp_permission_returns_403 ... ok
  ```

- [ ] **Step 3: Run clippy to confirm no lint issues**

  ```bash
  cargo clippy -p uptrakit-mcp --features db-sqlite --tests -- -D warnings
  ```

  Expected: zero warnings/errors.

- [ ] **Step 4: Commit**

  ```bash
  git add crates/ui/mcp/tests/get_current_user_mcp.rs
  git commit -m "test(mcp): add missing_access_mcp_permission_returns_403 integration test"
  ```

---

## Task 8: Fix broken Docker e2e test

**Files:**

- Modify: `crates/core/integration-tests/tests/oauth_end_to_end.rs`

- [ ] **Step 1: Understand the current broken code**

  Open `crates/core/integration-tests/tests/oauth_end_to_end.rs` and find the
  comment `// Step 9 — Call MCP with the OAuth access token`. Currently it sends
  a `tools/call` request without an `initialize` handshake. In stateful SSE mode
  (the production default), this returns HTTP 422 (`unexpected_message` — no
  session exists).

  The current broken block (around line 221–245):

  ```rust
  // Step 9 — Call MCP with the OAuth access token; assert HTTP 200.
  let mcp_resp = http
      .post(format!("https://127.0.0.1:{port}/mcp"))
      .header("Authorization", format!("Bearer {access_token}"))
      .header("Content-Type", "application/json")
      .json(&serde_json::json!({
          "jsonrpc": "2.0",
          "id": 1,
          "method": "tools/call",
          "params": {
              "name": "get_current_user",
              "arguments": {}
          }
      }))
      .send()
      .await
      .expect("POST /mcp with OAuth bearer");
  assert_eq!(
      mcp_resp.status().as_u16(),
      200,
      "MCP tool call with OAuth token must return 200, got: {}",
      mcp_resp.status()
  );
  ```

- [ ] **Step 2: Replace step 9 with the correct 3-step flow**

  Replace the entire `// Step 9` block with:

  ```rust
      // -----------------------------------------------------------------------
      // Step 9a — Initialize the MCP session.
      // -----------------------------------------------------------------------
      let init_resp = http
          .post(format!("https://127.0.0.1:{port}/mcp"))
          .header("Authorization", format!("Bearer {access_token}"))
          .header("Content-Type", "application/json")
          .header("Accept", "application/json, text/event-stream")
          .body(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"mcp-e2e-test","version":"1.0"}}}"#)
          .send()
          .await
          .expect("POST /mcp initialize");
      assert_eq!(
          init_resp.status().as_u16(),
          200,
          "initialize must return 200, got: {}",
          init_resp.status()
      );
      let session_id = init_resp
          .headers()
          .get("Mcp-Session-Id")
          .expect("Mcp-Session-Id header must be present after initialize")
          .to_str()
          .expect("session ID must be valid UTF-8")
          .to_string();

      // -----------------------------------------------------------------------
      // Step 9b — Acknowledge the session.
      // -----------------------------------------------------------------------
      let notif_resp = http
          .post(format!("https://127.0.0.1:{port}/mcp"))
          .header("Authorization", format!("Bearer {access_token}"))
          .header("Content-Type", "application/json")
          .header("Mcp-Session-Id", &session_id)
          .body(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)
          .send()
          .await
          .expect("POST /mcp notifications/initialized");
      assert_eq!(
          notif_resp.status().as_u16(),
          202,
          "notifications/initialized must return 202, got: {}",
          notif_resp.status()
      );

      // -----------------------------------------------------------------------
      // Step 9c — Call the tool; assert HTTP 200 and verify result content.
      // -----------------------------------------------------------------------
      let mcp_resp = http
          .post(format!("https://127.0.0.1:{port}/mcp"))
          .header("Authorization", format!("Bearer {access_token}"))
          .header("Content-Type", "application/json")
          .header("Accept", "application/json, text/event-stream")
          .header("Mcp-Session-Id", &session_id)
          .header("Mcp-Protocol-Version", "2025-03-26")
          .json(&serde_json::json!({
              "jsonrpc": "2.0",
              "id": 2,
              "method": "tools/call",
              "params": {
                  "name": "get_current_user",
                  "arguments": {}
              }
          }))
          .send()
          .await
          .expect("POST /mcp tools/call with OAuth bearer");
      assert_eq!(
          mcp_resp.status().as_u16(),
          200,
          "MCP tool call with OAuth token must return 200, got: {}",
          mcp_resp.status()
      );

      // Step 9d — Verify the SSE body contains user identity.
      let body_text = mcp_resp.text().await.expect("read tools/call body");
      assert!(
          body_text.contains("email"),
          "tools/call result must contain 'email' field; body: {body_text}"
      );
  ```

- [ ] **Step 3: Verify the file compiles**

  The Docker integration tests are `#[ignore]` by default (they need a running
  Docker environment). Only check compilation:

  ```bash
  cargo check -p uptrakit-integration-tests
  ```

  Expected: zero errors.

- [ ] **Step 4: Commit**

  ```bash
  git add crates/core/integration-tests/tests/oauth_end_to_end.rs
  git commit -m "fix(integration-tests): initialize MCP session before tools/call in OAuth e2e test"
  ```

---

## Task 9: Run quality gate and clean up

- [ ] **Step 1: Run the full in-process test suite**

  ```bash
  cargo test --all-features 2>&1 | tail -20
  ```

  Expected: all tests pass, including the three new ones.

- [ ] **Step 2: Run clippy across the workspace**

  ```bash
  cargo clippy --all-targets --all-features -- -D warnings
  ```

  Expected: zero warnings.

- [ ] **Step 3: Verify markdownlint passes**

  ```bash
  npx markdownlint-cli '**/*.md' --config .markdownlint.json
  ```

  Expected: no errors.
