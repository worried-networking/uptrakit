# OAuth E2E Test Coverage Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add e2e test coverage for Device OAuth (full HTTP chain) and MCP OAuth (in-process
auth-code + PKCE round-trip with JWT claim verification), plus Playwright tests for the `/device`
approval page.

**Architecture:** Three independent work streams — two new Rust integration-test files inside
`crates/ui/web-api/src/integration_tests/`, one new Playwright spec in
`frontend/tests/e2e/`. A shared prerequisite extracts the `insert_oauth_client` fixture from a
private test helper in `consent.rs` to `test_harness/fixtures.rs` so both Rust tests can use it.

**Tech Stack:** Rust (tokio, axum, sea-orm, jsonwebtoken), TestApp + TestClient harness, Playwright +
TypeScript.

---

## File Map

| Action | Path                                                                    | Purpose                                                              |
| ------ | ----------------------------------------------------------------------- | -------------------------------------------------------------------- |
| Modify | `crates/ui/web-api/src/test_harness/fixtures.rs`                        | Add `insert_oauth_client` as `pub(crate)`                            |
| Modify | `crates/ui/web-api/src/routes/oauth/consent.rs`                         | Remove private `insert_oauth_client`, update 5 call sites            |
| Modify | `crates/ui/web-api/src/integration_tests/mod.rs`                        | Add `mod device_auth_http_roundtrip;` and `mod oauth_mcp_roundtrip;` |
| Create | `crates/ui/web-api/src/integration_tests/device_auth_http_roundtrip.rs` | 2 device-flow tests (approve + deny → poll)                          |
| Create | `crates/ui/web-api/src/integration_tests/oauth_mcp_roundtrip.rs`        | 2 MCP OAuth tests (roundtrip + deny-consent)                         |
| Create | `frontend/tests/e2e/device-approval.spec.ts`                            | 5 Playwright tests for `/device` page                                |

---

## Task 1: Extract `insert_oauth_client` to `test_harness/fixtures.rs`

**Files:**

- Modify: `crates/ui/web-api/src/test_harness/fixtures.rs`
- Modify: `crates/ui/web-api/src/routes/oauth/consent.rs:578-617` (function definition)
- Modify: `crates/ui/web-api/src/routes/oauth/consent.rs:656,692,720,743,787` (5 call sites)

- [ ] **Step 1.1: Add imports to `fixtures.rs`**

  Open `crates/ui/web-api/src/test_harness/fixtures.rs`. Line 13 has:

  ```rust
  use uptrakit_shared_db::entity::{host, permission, role, role_permission, service, service_host};
  ```

  Extend the braced list to include `oauth_client` (alphabetical order — after `mqtt_lease`, before `permission`):

  ```rust
  use uptrakit_shared_db::entity::{host, oauth_client, permission, role, role_permission, service, service_host};
  ```

- [ ] **Step 1.2: Add `insert_oauth_client` to `fixtures.rs`**

  Append this function to the end of `crates/ui/web-api/src/test_harness/fixtures.rs`:

  ```rust
  /// Insert an `oauth_client` row and return its `client_id`.
  ///
  /// When `trusted` is `true`, `trusted_at` is set to the current timestamp.
  pub(crate) async fn insert_oauth_client(
      db: &DatabaseConnection,
      redirect_uri: &str,
      trusted: bool,
  ) -> String {
      let now = time::OffsetDateTime::now_utc();
      let client_id = format!("test-consent-client-{}", uuid::Uuid::now_v7());
      let redirect_uris_json =
          serde_json::to_string(&vec![redirect_uri]).expect("serialize redirect_uris");

      oauth_client::ActiveModel {
          id: Set(client_id.clone()),
          client_name: Set("Consent Test Client".to_string()),
          client_uri: Set(Some("https://example.com".to_string())),
          logo_uri: Set(None),
          redirect_uris: Set(redirect_uris_json),
          default_scope: Set("mcp:read".to_string()),
          grant_types: Set("authorization_code".to_string()),
          response_types: Set("code".to_string()),
          token_endpoint_auth_method: Set("none".to_string()),
          client_secret_hash: Set(None),
          registration_access_token_hash: Set(None),
          created_via: Set("test".to_string()),
          created_at: Set(now),
          last_used_at: Set(None),
          revoked_at: Set(None),
          metadata_cached_at: Set(None),
          metadata_etag: Set(None),
          metadata_content_hash: Set(None),
          metadata_raw: Set(None),
          metadata_parse_error: Set(None),
          metadata_parse_error_at: Set(None),
          trusted_at: Set(if trusted { Some(now) } else { None }),
      }
      .insert(db)
      .await
      .expect("insert oauth_client")
      .id
  }
  ```

  Note: the existing function in `consent.rs` returns `client_id` via a local variable; here we
  return `.id` from the inserted model directly — same value, simpler.

- [ ] **Step 1.3: Skip compile check**

  Do NOT run a compile check here. `consent.rs` still has its private `insert_oauth_client`
  function, so the code will report a duplicate-definition error. Proceed directly to Step 1.4.

- [ ] **Step 1.4: Remove private `insert_oauth_client` from `consent.rs`**

  In `crates/ui/web-api/src/routes/oauth/consent.rs`, delete the private function at line 575–617
  (the block that starts with `/// Insert an 'oauth_client' row` and ends with the closing `}`
  after `.expect("insert oauth_client")`).

- [ ] **Step 1.5: Update 5 call sites in `consent.rs`**

  The 5 call sites at lines 656, 692, 720, 743, 787 all look like:

  ```rust
  let client_id = insert_oauth_client(&app.db, TEST_REDIRECT_URI, false).await;
  ```

  Replace each one with:

  ```rust
  let client_id = crate::test_harness::fixtures::insert_oauth_client(&app.db, TEST_REDIRECT_URI, false).await;
  ```

  (Line 743 uses `true` — keep that argument.)

- [ ] **Step 1.6: Verify it compiles**

  ```bash
  cargo check --no-default-features --features db-sqlite -p uptrakit-web-api 2>&1 | head -30
  ```

  Expected: clean output (no errors).

- [ ] **Step 1.7: Run existing consent tests to verify no regression**

  ```bash
  cargo test --no-default-features --features db-sqlite -p uptrakit-web-api -- integration_tests::consent 2>&1 | tail -20
  ```

  Expected: all previously-passing tests still pass.

- [ ] **Step 1.8: Commit**

  ```bash
  git add crates/ui/web-api/src/test_harness/fixtures.rs \
          crates/ui/web-api/src/routes/oauth/consent.rs
  git commit -m "refactor(test): move insert_oauth_client to test_harness::fixtures

  Extracts the private oauth_client fixture helper from consent.rs tests
  into test_harness/fixtures.rs as pub(crate), ready for reuse in the
  upcoming oauth_mcp_roundtrip integration test.

  Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
  ```

---

## Task 2: Device flow HTTP round-trip tests

**Files:**

- Create: `crates/ui/web-api/src/integration_tests/device_auth_http_roundtrip.rs`
- Modify: `crates/ui/web-api/src/integration_tests/mod.rs`

### Step 2.1: Add mod declaration

- [ ] Open `crates/ui/web-api/src/integration_tests/mod.rs`. Add one line in alphabetical order
      (`device_auth_h` sorts before `device_auth_o`, so insert **before** `mod device_auth_oauth;`,
      making it the second line of the file after `mod auth_flow;`):

  ```rust
  mod device_auth_http_roundtrip;
  ```

### Step 2.2: Create the test file

- [ ] Create `crates/ui/web-api/src/integration_tests/device_auth_http_roundtrip.rs` with this
      exact content:

  ```rust
  #![expect(
      clippy::expect_used,
      reason = "test helper functions are not covered by allow-expect-in-tests"
  )]

  use http::StatusCode;
  use secrecy::ExposeSecret;
  use uptrakit_web_api_types::auth::UserResponse;
  use uptrakit_web_api_types::oauth::{
      DeviceAuthorizationResponse, OAuthErrorCode, OAuthErrorResponse, OAuthTokenResponse,
  };

  use crate::test_harness::TestApp;
  use crate::test_harness::fixtures::register_user;

  const CLIENT_ID: &str = "uptrakit-cli";
  const DEVICE_CODE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";

  /// Percent-encode a string for inclusion in an `application/x-www-form-urlencoded` body.
  /// The `:` and `/` in the device_code grant URN must be encoded as %3A and %2F.
  fn urlencoded(s: &str) -> String {
      s.chars()
          .flat_map(|c| match c {
              'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => vec![c],
              ':' => vec!['%', '3', 'A'],
              '/' => vec!['%', '2', 'F'],
              _ => {
                  let encoded = format!("%{:02X}", c as u32);
                  encoded.chars().collect()
              }
          })
          .collect()
  }

  // ── Test 1: full approve → token → API ─────────────────────────────────────

  #[tokio::test]
  async fn device_flow_full_http_chain_token_works_at_api() {
      let app = TestApp::new().await;
      let client = app.client();

      // Start device flow.
      let (status, flow): (_, DeviceAuthorizationResponse) = client
          .post_form(
              "/api/v1/oauth/device_authorization",
              &format!("client_id={CLIENT_ID}"),
          )
          .send_json()
          .await;
      assert_eq!(status, StatusCode::OK);
      let user_code = flow.user_code.clone();
      let device_code = flow.device_code.clone();

      // Register user via HTTP, capture JWT and expected email.
      let (reg_status, auth) =
          register_user(&client, "approve-device@example.com", "TestPassword1!").await;
      assert_eq!(reg_status, StatusCode::CREATED);
      let user_jwt = auth.access_token.expose_secret().to_string();
      let expected_email = auth.user.email.clone();

      // Approve via POST /api/v1/auth/device/approve (no internal state injection).
      let approve_status = client
          .post_json(
              "/api/v1/auth/device/approve",
              &serde_json::json!({ "user_code": user_code }),
          )
          .bearer(&user_jwt)
          .send_status()
          .await;
      assert_eq!(approve_status, StatusCode::OK);

      // Poll for token.
      let form = format!(
          "grant_type={}&device_code={}&client_id={CLIENT_ID}",
          urlencoded(DEVICE_CODE_GRANT),
          urlencoded(&device_code),
      );
      let (token_status, token): (_, OAuthTokenResponse) = client
          .post_form("/api/v1/oauth/token", &form)
          .send_json()
          .await;
      assert_eq!(token_status, StatusCode::OK);
      assert_eq!(token.token_type, "Bearer");
      assert!(
          token.access_token.starts_with("upk_"),
          "access_token must start with 'upk_', got: {}",
          token.access_token,
      );

      // Use the device token at a standard authenticated endpoint.
      let (me_status, me): (_, UserResponse) = client
          .get("/api/v1/auth/me")
          .bearer(&token.access_token)
          .send_json()
          .await;
      assert_eq!(me_status, StatusCode::OK);
      assert_eq!(me.email, expected_email);
  }

  // ── Test 2: deny → poll → access_denied ────────────────────────────────────

  #[tokio::test]
  async fn device_flow_deny_via_http_returns_access_denied_on_poll() {
      let app = TestApp::new().await;
      let client = app.client();

      // Start device flow.
      let (status, flow): (_, DeviceAuthorizationResponse) = client
          .post_form(
              "/api/v1/oauth/device_authorization",
              &format!("client_id={CLIENT_ID}"),
          )
          .send_json()
          .await;
      assert_eq!(status, StatusCode::OK);

      // Register user via HTTP.
      let (reg_status, auth) =
          register_user(&client, "deny-device@example.com", "TestPassword1!").await;
      assert_eq!(reg_status, StatusCode::CREATED);
      let user_jwt = auth.access_token.expose_secret().to_string();

      // Deny via POST /api/v1/auth/device/deny.
      let deny_status = client
          .post_json(
              "/api/v1/auth/device/deny",
              &serde_json::json!({ "user_code": flow.user_code }),
          )
          .bearer(&user_jwt)
          .send_status()
          .await;
      assert_eq!(deny_status, StatusCode::OK);

      // Poll — must return access_denied per RFC 8628.
      let form = format!(
          "grant_type={}&device_code={}&client_id={CLIENT_ID}",
          urlencoded(DEVICE_CODE_GRANT),
          urlencoded(&flow.device_code),
      );
      let (token_status, err): (_, OAuthErrorResponse) = client
          .post_form("/api/v1/oauth/token", &form)
          .send_json()
          .await;
      assert_eq!(token_status, StatusCode::BAD_REQUEST);
      assert_eq!(err.error, OAuthErrorCode::AccessDenied);
  }
  ```

### Step 2.3: Run the new tests

- [ ] Run both new tests:

  ```bash
  cargo test --no-default-features --features db-sqlite -p uptrakit-web-api \
    -- integration_tests::device_auth_http_roundtrip 2>&1 | tail -20
  ```

  Expected:

  ```text
  test integration_tests::device_auth_http_roundtrip::device_flow_full_http_chain_token_works_at_api ... ok
  test integration_tests::device_auth_http_roundtrip::device_flow_deny_via_http_returns_access_denied_on_poll ... ok
  ```

### Step 2.4: Run clippy

- [ ] ```bash
      cargo clippy --all-targets --no-default-features --features db-sqlite -p uptrakit-web-api 2>&1 | grep "^error" | head -20
      ```

  Expected: no errors.

### Step 2.5: Commit

- [ ] Run:

  ```bash
  git add crates/ui/web-api/src/integration_tests/device_auth_http_roundtrip.rs \
          crates/ui/web-api/src/integration_tests/mod.rs
  git commit -m "test(device-auth): add full HTTP round-trip integration tests"
  ```

---

## Task 3: MCP OAuth in-process PKCE round-trip tests

**Files:**

- Create: `crates/ui/web-api/src/integration_tests/oauth_mcp_roundtrip.rs`
- Modify: `crates/ui/web-api/src/integration_tests/mod.rs`

Depends on: Task 1 (uses `crate::test_harness::fixtures::insert_oauth_client`).

### Background

- The `GET /oauth/authorize` endpoint redirects (302) to `/oauth/consent/<request_id>`.
  `TestClient::send()` returns the raw response without following redirects — read the `Location`
  header to get `request_id`.
- The `POST /oauth/consent/{id}/approve` endpoint requires the optional-auth middleware to be
  layered onto the router (same pattern as `consent.rs` tests). Define it locally in this file.
- The signer and verifier must share the same secret (`TEST_SECRET`) so `verify()` succeeds on
  a real minted token.
- The clock from `OAuthState::disabled()` is already `Arc::new(OffsetDateTime::now_utc)` — no
  override needed. Do NOT replace it with a fixed past timestamp; that causes `ar_svc.consume()`
  to return 410 Gone.
- The `authenticate_jwt` function is `pub(crate)` — the import path is valid from any module in
  this crate.
- Deny endpoint takes no body — use `client.post_empty(...)`.

### Step 3.1: Add mod declaration

- [ ] In `crates/ui/web-api/src/integration_tests/mod.rs`, add after `mod oauth_master_switch_off;`
  and before `mod oidc_callback;` (alphabetical: `oauth_mcp` sorts after `oauth_master`, before
  `oidc`):

  ```rust
  mod oauth_mcp_roundtrip;
  ```

### Step 3.2: Create the test file

- [ ] Create `crates/ui/web-api/src/integration_tests/oauth_mcp_roundtrip.rs`:

  ```rust
  #![expect(
      clippy::expect_used,
      reason = "test helper functions are not covered by allow-expect-in-tests"
  )]

  use std::sync::Arc;

  use http::StatusCode;
  use secrecy::ExposeSecret;
  use uptrakit_web_api_types::oauth::OAuthTokenResponse;

  use crate::oauth::OAuthState;
  use crate::oauth::canonical_url::CanonicalUrlConfig;
  use crate::oauth::jwt::{McpOAuthJwtSigner, McpOAuthJwtVerifier};
  use crate::router::build_router;
  use crate::test_harness::fixtures::{insert_oauth_client, register_user};
  use crate::test_harness::http_client::TestClient;
  use crate::test_harness::{build_test_state, insert_default_tenant, setup_migrated_db};

  // RFC 7636 §4.6 test vectors.
  const CODE_VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
  const CODE_CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
  const REDIRECT_URI: &str = "https://localhost/callback";
  const RESOURCE: &str = "https://controller.example.com/mcp";
  // Secret shared between signer (token endpoint) and verifier (test assertion).
  const TEST_SECRET: &[u8] = b"mcp-roundtrip-test-secret-32b!!";

  // ── Setup ────────────────────────────────────────────────────────────────────

  /// Optional-auth middleware: if a valid Bearer token is present, inject
  /// `AuthenticatedUser` into request extensions. Required so that the
  /// `GET /oauth/authorize` handler can identify the caller.
  async fn optional_auth_middleware(
      axum::extract::State(state): axum::extract::State<Arc<crate::AppState>>,
      mut req: axum::extract::Request,
      next: axum::middleware::Next,
  ) -> axum::response::Response {
      use crate::middleware::require_auth::authenticate_jwt;

      let token = req
          .headers()
          .get(axum::http::header::AUTHORIZATION)
          .and_then(|v| v.to_str().ok())
          .and_then(|v| v.strip_prefix("Bearer "))
          .map(str::to_owned);

      if let Some(token) = token
          && let Ok((user, _setup_required)) = authenticate_jwt(&state, &token).await
      {
          req.extensions_mut().insert(user);
      }

      next.run(req).await
  }

  /// Build a [`TestClient`] backed by a router with MCP OAuth enabled and the
  /// optional-auth middleware wired in, plus the DB connection for fixtures.
  async fn setup_client() -> (TestClient, sea_orm::DatabaseConnection) {
      let db = setup_migrated_db().await;
      let tenant_id = insert_default_tenant(&db).await;
      let (state, _jwt) = build_test_state(db.clone(), tenant_id).await;

      // Use disabled() as a base so the struct literal never breaks when new fields are added.
      let mut oauth = OAuthState::disabled();
      oauth.enabled = true;
      oauth.canonical = CanonicalUrlConfig::new("controller.example.com".to_string(), vec![])
          .expect("test canonical url");
      oauth.signer = Arc::new(McpOAuthJwtSigner::new(TEST_SECRET));
      oauth.verifier = Arc::new(McpOAuthJwtVerifier::new(
          TEST_SECRET,
          "https://controller.example.com".to_string(),
          vec![RESOURCE.to_string()],
      ));

      let mut patched = (*state).clone();
      patched.oauth = oauth;
      let state = Arc::new(patched);
      let router = build_router(Arc::clone(&state)).layer(
          axum::middleware::from_fn_with_state(Arc::clone(&state), optional_auth_middleware),
      );

      (TestClient::new(router), db)
  }

  // ── Helpers ──────────────────────────────────────────────────────────────────

  fn urlencoded(s: &str) -> String {
      s.chars()
          .flat_map(|c| match c {
              'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => vec![c],
              ':' => vec!['%', '3', 'A'],
              '/' => vec!['%', '2', 'F'],
              _ => {
                  let encoded = format!("%{:02X}", c as u32);
                  encoded.chars().collect()
              }
          })
          .collect()
  }

  // ── Test 1: full auth-code + PKCE round-trip ─────────────────────────────────

  #[tokio::test]
  async fn mcp_oauth_auth_code_pkce_roundtrip_token_claims_valid() {
      let (client, db) = setup_client().await;

      // Insert a trusted oauth_client; trusted clients skip the typed confirmation gate.
      let oauth_client_id = insert_oauth_client(&db, REDIRECT_URI, true).await;

      // Register a user via HTTP to get a user JWT and user_id.
      let (reg_status, auth) =
          register_user(&client, "mcp-roundtrip@example.com", "TestPassword1!").await;
      assert_eq!(reg_status, StatusCode::CREATED);
      let user_jwt = auth.access_token.expose_secret().to_string();
      let user_id = auth.user.id;

      // Step 1 — GET /oauth/authorize → 302 to /oauth/consent/<request_id>.
      let authorize_url = format!(
          "/oauth/authorize?response_type=code&client_id={}&redirect_uri={}&scope=mcp%3Aread&state=test-state-123&code_challenge={}&code_challenge_method=S256&resource={}",
          urlencoded(&oauth_client_id),
          urlencoded(REDIRECT_URI),
          CODE_CHALLENGE,
          urlencoded(RESOURCE),
      );
      let authorize_resp = client.get(&authorize_url).bearer(&user_jwt).send().await;
      assert_eq!(authorize_resp.status(), StatusCode::FOUND);
      let location = authorize_resp
          .headers()
          .get("location")
          .expect("Location header missing")
          .to_str()
          .expect("Location is not ASCII");
      let request_id = location
          .strip_prefix("/oauth/consent/")
          .expect("Location must start with /oauth/consent/")
          .split('?')
          .next()
          .expect("split always yields at least one element");

      // Step 2 — POST /oauth/consent/<request_id>/approve → JSON with redirect_to containing code.
      let (approve_status, approve_body): (_, serde_json::Value) = client
          .post_json(
              &format!("/oauth/consent/{request_id}/approve"),
              &serde_json::json!({}),
          )
          .bearer(&user_jwt)
          .send_json()
          .await;
      assert_eq!(approve_status, StatusCode::OK);
      let redirect_to = approve_body["redirect_to"]
          .as_str()
          .expect("redirect_to must be a string");
      assert!(
          redirect_to.contains("code="),
          "redirect_to must contain code=, got: {redirect_to}",
      );

      // Extract the authorization code from the redirect_to URL query string.
      let auth_code = redirect_to
          .split('?')
          .nth(1)
          .and_then(|q| q.split('&').find_map(|p| p.strip_prefix("code=")))
          .expect("code query param in redirect_to");

      // Step 3 — POST /oauth/token → JWT access token.
      let token_form = format!(
          "grant_type=authorization_code&code={}&redirect_uri={}&client_id={}&code_verifier={}&resource={}",
          urlencoded(auth_code),
          urlencoded(REDIRECT_URI),
          urlencoded(&oauth_client_id),
          CODE_VERIFIER,
          urlencoded(RESOURCE),
      );
      let (token_status, token): (_, OAuthTokenResponse) = client
          .post_form("/oauth/token", &token_form)
          .send_json()
          .await;
      assert_eq!(token_status, StatusCode::OK);
      assert_eq!(token.token_type, "Bearer");

      // Step 4 — Verify JWT claims with McpOAuthJwtVerifier using the same secret.
      let verifier = McpOAuthJwtVerifier::new(
          TEST_SECRET,
          "https://controller.example.com".to_string(),
          vec![RESOURCE.to_string()],
      );
      let claims = verifier
          .verify(&token.access_token)
          .expect("JWT must verify with matching secret and issuer");
      assert_eq!(claims.iss, "https://controller.example.com");
      assert_eq!(claims.aud, RESOURCE);
      assert_eq!(claims.sub, user_id.to_string());
      assert_eq!(claims.scope, "mcp:read");
      assert!(
          claims.exp > time::OffsetDateTime::now_utc().unix_timestamp(),
          "token must not be expired",
      );
  }

  // ── Test 2: deny consent → access_denied redirect ────────────────────────────

  #[tokio::test]
  async fn mcp_oauth_deny_consent_yields_access_denied_redirect() {
      let (client, db) = setup_client().await;

      let oauth_client_id = insert_oauth_client(&db, REDIRECT_URI, true).await;

      let (reg_status, auth) =
          register_user(&client, "mcp-deny@example.com", "TestPassword1!").await;
      assert_eq!(reg_status, StatusCode::CREATED);
      let user_jwt = auth.access_token.expose_secret().to_string();

      // GET /oauth/authorize → 302 → extract request_id.
      let authorize_url = format!(
          "/oauth/authorize?response_type=code&client_id={}&redirect_uri={}&scope=mcp%3Aread&state=test-state-123&code_challenge={}&code_challenge_method=S256&resource={}",
          urlencoded(&oauth_client_id),
          urlencoded(REDIRECT_URI),
          CODE_CHALLENGE,
          urlencoded(RESOURCE),
      );
      let authorize_resp = client.get(&authorize_url).bearer(&user_jwt).send().await;
      assert_eq!(authorize_resp.status(), StatusCode::FOUND);
      let location = authorize_resp
          .headers()
          .get("location")
          .expect("Location header missing")
          .to_str()
          .expect("Location is not ASCII");
      let request_id = location
          .strip_prefix("/oauth/consent/")
          .expect("Location must start with /oauth/consent/")
          .split('?')
          .next()
          .expect("split always yields at least one element");

      // POST /oauth/consent/<request_id>/deny (no body).
      let (deny_status, deny_body): (_, serde_json::Value) = client
          .post_empty(&format!("/oauth/consent/{request_id}/deny"))
          .bearer(&user_jwt)
          .send_json()
          .await;
      assert_eq!(deny_status, StatusCode::OK);
      let redirect_to = deny_body["redirect_to"]
          .as_str()
          .expect("redirect_to must be a string");
      assert!(
          redirect_to.contains("error=access_denied"),
          "redirect_to must contain error=access_denied, got: {redirect_to}",
      );
      assert!(
          redirect_to.contains("state=test-state-123"),
          "redirect_to must preserve state param, got: {redirect_to}",
      );
  }
  ```

### Step 3.3: Run the new tests

- [ ] ```bash
      cargo test --no-default-features --features db-sqlite -p uptrakit-web-api \
        -- integration_tests::oauth_mcp_roundtrip 2>&1 | tail -20
      ```

  Expected:

  ```text
  test integration_tests::oauth_mcp_roundtrip::mcp_oauth_auth_code_pkce_roundtrip_token_claims_valid ... ok
  test integration_tests::oauth_mcp_roundtrip::mcp_oauth_deny_consent_yields_access_denied_redirect ... ok
  ```

### Step 3.4: Run full backend test suite

- [ ] ```bash
      cargo test --all-features 2>&1 | tail -30
      ```

  Expected: all tests pass.

### Step 3.5: Run clippy

- [ ] ```bash
      cargo clippy --all-targets --all-features 2>&1 | grep "^error" | head -20
      ```

  Expected: no errors.

### Step 3.6: Commit

- [ ] Run:

  ```bash
  git add crates/ui/web-api/src/integration_tests/oauth_mcp_roundtrip.rs \
          crates/ui/web-api/src/integration_tests/mod.rs
  git commit -m "test(oauth): add MCP OAuth in-process auth-code + PKCE round-trip tests"
  ```

---

## Task 4: Playwright tests for `/device` approval page

**Files:**

- Create: `frontend/tests/e2e/device-approval.spec.ts`

### Background

All API calls are mocked via `page.route()`. Glob patterns do NOT match query strings
(`**/path?query` silently fails to match) — omit query strings and filter by method inline.
The mock for the lookup endpoint uses `route.request().method() === 'GET'` to distinguish it
from other request types to the same path.

The `/device` page auto-triggers a lookup when:

1. All 8 code characters are filled (`codeValid = true`)
2. The user is logged in (`isLoggedIn = true`)
3. `lookupPhase === 'idle'`

For tests that don't need a lookup (unauthenticated), no lookup mock is needed.

### Step 4.1: Create the spec file

- [ ] Create `frontend/tests/e2e/device-approval.spec.ts`:

  ```typescript
  import { expect, test } from "@playwright/test";
  import type { Page } from "@playwright/test";

  // ---------------------------------------------------------------------------
  // Selectors
  // ---------------------------------------------------------------------------

  const CONSENT_PROMPT = '[data-ui="consent-prompt"]';
  const APPROVE_BUTTON = 'button:has-text("Approve")';
  const DENY_BUTTON = 'button:has-text("Deny")';
  const SUCCESS_CALLOUT = '[data-ui="callout"][data-tone="success"]';
  const DENIED_CALLOUT = '[data-ui="callout"][data-tone="warning"]';
  const ERROR_CALLOUT = '[data-ui="callout"][data-tone="danger"]';

  // ---------------------------------------------------------------------------
  // Session helpers — same pattern as oauth-consent.spec.ts
  // ---------------------------------------------------------------------------

  async function mockAuthenticatedSession(page: Page) {
    await page.route("**/api/v1/auth/refresh", (route) =>
      route.fulfill({
        status: 200,
        json: {
          access_token: "test-access-token",
          refresh_token: "test-refresh-token",
        },
      }),
    );
    await page.route("**/api/v1/auth/me", (route) =>
      route.fulfill({
        status: 200,
        json: {
          id: "00000000-0000-0000-0000-000000000001",
          email: "user@example.com",
          first_name: "Test",
          last_name: "User",
          permissions: [],
        },
      }),
    );
    await page.route("**/api/v1/system/alerts", (route) =>
      route.fulfill({ json: { alerts: [] } }),
    );
  }

  // ---------------------------------------------------------------------------
  // Device-specific mock helpers
  // ---------------------------------------------------------------------------

  async function mockDeviceLookupSuccess(page: Page) {
    await page.route("**/api/v1/auth/device/lookup", (route) => {
      if (route.request().method() === "GET") {
        route.fulfill({
          status: 200,
          json: {
            client_name: "uptrakit CLI",
            expires_at: "2099-01-01T00:00:00Z",
          },
        });
      } else {
        route.fallback();
      }
    });
  }

  async function mockDeviceLookup404(page: Page) {
    await page.route("**/api/v1/auth/device/lookup", (route) => {
      if (route.request().method() === "GET") {
        route.fulfill({ status: 404, json: { error: "not found" } });
      } else {
        route.fallback();
      }
    });
  }

  async function mockDeviceApprove(page: Page) {
    await page.route("**/api/v1/auth/device/approve", (route) => {
      if (route.request().method() === "POST") {
        route.fulfill({ status: 200, json: { message: "approved" } });
      } else {
        route.fallback();
      }
    });
  }

  async function mockDeviceDeny(page: Page) {
    await page.route("**/api/v1/auth/device/deny", (route) => {
      if (route.request().method() === "POST") {
        route.fulfill({ status: 200, json: { message: "denied" } });
      } else {
        route.fallback();
      }
    });
  }

  // ---------------------------------------------------------------------------
  // Tests
  // ---------------------------------------------------------------------------

  test.describe("device approval page", () => {
    test("pre-filled code triggers lookup and shows consent prompt", async ({
      page,
    }) => {
      await mockAuthenticatedSession(page);
      await mockDeviceLookupSuccess(page);

      await page.goto("/device?user_code=BCDF-GHJK");
      await page.waitForSelector(CONSENT_PROMPT);

      await expect(page.locator(CONSENT_PROMPT)).toBeVisible();
      await expect(page.locator(APPROVE_BUTTON)).toBeEnabled();
      await expect(page.locator(DENY_BUTTON)).toBeEnabled();
    });

    test("approve calls approve endpoint and shows success callout", async ({
      page,
    }) => {
      await mockAuthenticatedSession(page);
      await mockDeviceLookupSuccess(page);
      await mockDeviceApprove(page);

      await page.goto("/device?user_code=BCDF-GHJK");
      await page.waitForSelector(CONSENT_PROMPT);

      const approveResponse = page.waitForResponse(
        "**/api/v1/auth/device/approve",
      );
      await page.locator(APPROVE_BUTTON).click();
      await approveResponse;

      await expect(page.locator(SUCCESS_CALLOUT)).toBeVisible();
    });

    test("deny calls deny endpoint and shows denied callout", async ({
      page,
    }) => {
      await mockAuthenticatedSession(page);
      await mockDeviceLookupSuccess(page);
      await mockDeviceDeny(page);

      await page.goto("/device?user_code=BCDF-GHJK");
      await page.waitForSelector(CONSENT_PROMPT);

      const denyResponse = page.waitForResponse("**/api/v1/auth/device/deny");
      await page.locator(DENY_BUTTON).click();
      await denyResponse;

      await expect(page.locator(DENIED_CALLOUT)).toBeVisible();
    });

    test("invalid user_code shows error callout", async ({ page }) => {
      await mockAuthenticatedSession(page);
      await mockDeviceLookup404(page);

      await page.goto("/device?user_code=BCDF-GHJK");
      await page.waitForResponse("**/api/v1/auth/device/lookup");

      await expect(page.locator(ERROR_CALLOUT)).toBeVisible();
      await expect(page.locator(CONSENT_PROMPT)).not.toBeVisible();
    });

    test("unauthenticated user sees login prompt", async ({ page }) => {
      // Refresh returns 401 → auth module clears user → isLoggedIn = false.
      await page.route("**/api/v1/auth/refresh", (route) =>
        route.fulfill({ status: 401, json: {} }),
      );
      await page.route("**/api/v1/system/alerts", (route) =>
        route.fulfill({ json: { alerts: [] } }),
      );

      await page.goto("/device?user_code=BCDF-GHJK");

      await expect(page.locator('a[href*="/login"]')).toBeVisible();
      await expect(page.locator(CONSENT_PROMPT)).not.toBeVisible();
    });
  });
  ```

### Step 4.2: Run format check

- [ ] ```bash
      cd frontend && npx prettier --write tests/e2e/device-approval.spec.ts && cd ..
      ```

  Expected: file may be rewritten with aligned tabs. Re-check the content is unchanged in meaning.

### Step 4.3: Run lint

- [ ] ```bash
      cd frontend && npm run lint 2>&1 | tail -20 && cd ..
      ```

  Expected: no errors for the new file.

### Step 4.4: Run the Playwright tests

- [ ] ```bash
      cd frontend && npm run test:e2e -- --grep "device approval" 2>&1 | tail -40 && cd ..
      ```

  Expected: all 5 tests pass:

  ```text
  ✓ device approval page > pre-filled code triggers lookup and shows consent prompt
  ✓ device approval page > approve calls approve endpoint and shows success callout
  ✓ device approval page > deny calls deny endpoint and shows denied callout
  ✓ device approval page > invalid user_code shows error callout
  ✓ device approval page > unauthenticated user sees login prompt
  ```

### Step 4.5: Commit

- [ ] Run:

  ```bash
  git add frontend/tests/e2e/device-approval.spec.ts
  git commit -m "test(device-ui): add Playwright tests for /device approval page"
  ```

---

## Task 5: Final quality gate

- [ ] **Step 5.1: Full Rust check + test + deny**

  ```bash
  cargo fmt --all && \
  cargo check --no-default-features --features db-sqlite && \
  cargo check --all-features && \
  cargo clippy --all-targets --no-default-features --features db-sqlite 2>&1 | grep "^error" && \
  cargo clippy --all-targets --all-features 2>&1 | grep "^error" && \
  cargo test --all-features 2>&1 | tail -30 && \
  cargo deny check
  ```

  Expected: `cargo test` shows all tests pass; `cargo deny check` exits clean; both clippy
  commands produce no `error:` lines.

- [ ] **Step 5.2: Full frontend quality gate**

  ```bash
  cd frontend && npm run lint && npm run format:check && npm run test:e2e && cd ..
  ```

  Expected: all steps pass without errors.
