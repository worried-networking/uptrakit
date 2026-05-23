# MCP OAuth Verification — Plan C: Consent Bypass + E2E Test

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a `POST /oauth/test/auto-approve/{request_id}` endpoint that lets integration tests
skip the consent UI redirect chain, then complete steps 3–9 of `oauth_end_to_end_mcp_rs_round_trip`.

**Architecture:** The handler lives in `test_utils.rs` behind `#[cfg(feature = "test-utils")]`
(compile-time gate) and `test_utils_allowed()` (runtime gate). It delegates to the same services as
`approve_consent` but skips CSRF and rate-limit checks. Returns JSON
`{ "redirect_uri": "…?code=…&state=…" }`. `ApiClient` gains two helper methods. The E2E test uses
`wait_for_generation` + `force_reexec` to confirm OAuth boots live before proceeding.

**Prerequisite:** Plan A must be merged first — the E2E test cannot pass unless OAuth boots.

**Tech Stack:** Rust 2024, Axum 0.8 (`{param}` path syntax), SeaORM, serde_json, reqwest, sha2,
base64.

---

## File Map

| Action | Path                                                        |
| ------ | ----------------------------------------------------------- |
| Modify | `crates/ui/web-api/src/routes/test_utils.rs`                |
| Modify | `crates/ui/web-api/src/router.rs`                           |
| Modify | `crates/core/integration-tests/tests/helpers/api_client.rs` |
| Modify | `crates/core/integration-tests/tests/oauth_end_to_end.rs`   |

---

## Task 1: Add `oauth_auto_approve_consent` handler to `test_utils.rs`

**Files:**

- Modify: `crates/ui/web-api/src/routes/test_utils.rs`

The whole file is gated `#![cfg(feature = "test-utils")]` (line 6). The handler follows the same
structure as `approve_consent` in `consent.rs` but omits the typed-confirmation check and
rate-limiting — test clients are always trusted via the bearer-token auth.

- [ ] **Step 1: Add handler imports**

Add the following `use` statements at the top of `test_utils.rs` (after the existing imports, before
the first function):

```rust
use axum::Json;
use uuid::Uuid;
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};

use crate::oauth::services::authorization_code::{
    MintAuthorizationCode, OAuthAuthorizationCodeService,
};
use crate::oauth::services::authorization_request::OAuthAuthorizationRequestService;
use crate::oauth::services::consent::OAuthConsentService;
```

Note: `sea_orm::EntityTrait` and `AuthenticatedUser` are NOT imported — neither is needed.
`helpers::percent_encode` is `pub(super)` (private to `routes::oauth`) and unreachable from
`test_utils.rs`. Use `percent_encoding` directly instead (it is already a workspace dependency).

- [ ] **Step 2: Write failing test (unit-level smoke check)**

There is no in-process unit test for handlers — they are tested via integration tests (Task 4).
Verify the function signature compiles:

```bash
cargo check --all-features -p uptrakit-web-api 2>&1 | grep "error\[" | head -5
```

Expected: compile errors because the function doesn't exist yet.

- [ ] **Step 3: Add the handler**

Append to `crates/ui/web-api/src/routes/test_utils.rs`:

```rust
/// Approve an OAuth consent request without going through the browser UI.
///
/// Looks up the pending authorization request by `request_id`, grants consent on behalf
/// of whichever user initiated the authorization (their `user_id` is on the request row),
/// issues an authorization code, and returns JSON:
/// `{ "redirect_uri": "https://...?code=<code>&state=<state>" }`.
///
/// No ownership check — this endpoint is already double-gated by the compile-time
/// `#[cfg(feature = "test-utils")]` file attribute and the `test_utils_allowed()` runtime check.
///
/// Returns 404 when `UPTRAKIT_TEST_UTILS_ENABLED != "true"`.
/// Returns 404 when OAuth is disabled or the authorization request does not exist / already consumed.
pub(crate) async fn oauth_auto_approve_consent(
    State(state): State<Arc<AppState>>,
    Path(request_id): Path<Uuid>,
) -> Response {
    if !test_utils_allowed() {
        return StatusCode::NOT_FOUND.into_response();
    }
    if !state.oauth.enabled {
        return StatusCode::NOT_FOUND.into_response();
    }

    // Consume the authorization request (marks it used, returns row).
    // The `user_id` on the row is the user who drove GET /oauth/authorize.
    let ar_svc =
        OAuthAuthorizationRequestService::new(state.db().clone(), Arc::clone(&state.oauth.clock));
    let row = match ar_svc.consume(request_id).await {
        Ok(Some(r)) => r,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "auto-approve: failed to consume authorization request");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    // Grant consent on behalf of the user who initiated the authorization.
    let consent_svc =
        OAuthConsentService::new(state.db().clone(), Arc::clone(&state.oauth.clock));
    if let Err(e) = consent_svc
        .grant(row.user_id, &row.client_id, &row.scope, None)
        .await
    {
        tracing::error!(error = %e, "auto-approve: failed to grant consent");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    // Mint authorization code.
    let code_svc =
        OAuthAuthorizationCodeService::new(state.db().clone(), Arc::clone(&state.oauth.clock));
    let code = match code_svc
        .mint(MintAuthorizationCode {
            request_id: row.request_id,
            client_id: row.client_id.clone(),
            user_id: row.user_id,
            redirect_uri: row.redirect_uri.clone(),
            scope: row.scope.clone(),
            code_challenge: row.code_challenge.clone(),
            code_challenge_method: row.code_challenge_method.clone(),
            resource: row.resource.clone(),
        })
        .await
    {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, "auto-approve: failed to mint authorization code");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let sep = if row.redirect_uri.contains('?') { '&' } else { '?' };
    let redirect_uri = format!(
        "{}{}code={}&state={}",
        row.redirect_uri,
        sep,
        utf8_percent_encode(code.as_str(), NON_ALPHANUMERIC),
        utf8_percent_encode(&row.state, NON_ALPHANUMERIC),
    );

    (StatusCode::OK, Json(serde_json::json!({ "redirect_uri": redirect_uri }))).into_response()
}
```

- [ ] **Step 4: Compile check**

```bash
cargo check --all-features -p uptrakit-web-api 2>&1 | tail -5
```

Expected: clean compile.

- [ ] **Step 5: Commit**

```bash
git add crates/ui/web-api/src/routes/test_utils.rs
git commit -m "feat(test-utils): add oauth_auto_approve_consent handler — skip consent UI in E2E tests"
```

---

## Task 2: Mount the route in `router.rs`

**Files:**

- Modify: `crates/ui/web-api/src/router.rs`

- [ ] **Step 1: Add route to the `#[cfg(feature = "test-utils")]` block**

Find the existing test-utils block (around line 994–1008):

```rust
#[cfg(feature = "test-utils")]
{
    router = router.route(
        "/api/v1/test/services/{id}/request-renewal",
        axum::routing::post(crate::routes::test_utils::request_service_renewal),
    );
    router = router.route(
        "/api/v1/test/services/{id}/disconnect",
        axum::routing::post(crate::routes::test_utils::disconnect_service),
    );
    router = router.route(
        "/test/force-reexec",
        axum::routing::post(crate::routes::test_utils::force_reexec),
    );
}
```

Add inside the same block:

```rust
    router = router.route(
        "/oauth/test/auto-approve/{request_id}",
        axum::routing::post(crate::routes::test_utils::oauth_auto_approve_consent),
    );
```

Note: `{request_id}` — Axum 0.8 path syntax, NOT `:request_id`.

The route requires a valid bearer token because `AuthenticatedUser` is an `Extension` extracted by
the optional-auth middleware that runs across all routes.

- [ ] **Step 2: Compile check**

```bash
cargo check --all-features -p uptrakit-web-api 2>&1 | tail -5
```

Expected: clean compile.

- [ ] **Step 3: Commit**

```bash
git add crates/ui/web-api/src/router.rs
git commit -m "feat(test-utils/router): mount /oauth/test/auto-approve/{request_id} route"
```

---

## Task 3: Add `update_oauth_settings` and `auto_approve_consent` to `ApiClient`

**Files:**

- Modify: `crates/core/integration-tests/tests/helpers/api_client.rs`

Note: `api_client.rs` has `#![expect(clippy::expect_used, clippy::panic, reason = "...")]` at the
top — panics and `.expect()` are acceptable in this helper.

- [ ] **Step 1: Add `api_token` field to `ApiClient` struct**

`ApiClient` stores the token only inside the `UptrakitClient` wrapper — the raw string is not
accessible from raw `reqwest` callers. Add `api_token: Option<String>` to the struct and populate it
at login time.

Locate the `ApiClient` struct definition (search for `pub(crate) struct ApiClient`) and add the
field:

```rust
pub(crate) struct ApiClient {
    base_url: String,
    client: Option<UptrakitClient>,
    pki_base_url: Option<String>,
    api_token: Option<String>,  // ← add
}
```

If a `new` constructor or `Default` impl exists, initialise `api_token: None` there too.

Then find `register_and_login_with_token` (around line 220–250). It calls
`auth_resp.access_token.expose_secret().to_string()` to build the `UptrakitClient`. Capture the
token before assigning it:

```rust
let token = auth_resp.access_token.expose_secret().to_string();
self.client = Some(UptrakitClient::with_token(&self.base_url, &token, true));
self.api_token = Some(token);
```

- [ ] **Step 2: Add `update_oauth_settings` helper**

The PUT endpoint uses the `IfMatch` extractor which returns 428 when the header is absent and does
strict ETag string comparison — `If-Match: *` wildcard is NOT supported. The helper must GET first
to capture the current ETag (`W/"settings-v0"` format), then PUT with that exact value.

````rust
/// PUT /api/v1/global-settings/oauth — enable OAuth for E2E tests.
///
/// Sets `canonical_host` (triggers auto-enable of `mcp_enabled`) and sets `dcr_enabled = true`
/// so Dynamic Client Registration works. Both must be set before the E2E DCR step.
///
/// `dcr_enabled` is seeded to `false` by `seed_oauth_defaults`; the seed is NOT removed by Plan A
/// because DCR should remain explicitly opt-in in production. The test enables it here.
///
/// First GETs current settings to read the ETag, then PUTs with `If-Match: <etag>`.
/// The `IfMatch` extractor does strict string comparison; wildcards are not supported.
///
/// Requires a valid API token (call `register_and_login_with_token` first).
/// Panics if any request fails or the server returns a non-2xx status.
pub(crate) async fn update_oauth_settings(&self, canonical_host: &str) {
    let token = self
        .api_token
        .as_deref()
        .expect("must call register_and_login_with_token first");
    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .expect("build reqwest client");

    // Step 1: GET current settings to capture the ETag.
    let get_resp = client
        .get(format!("{}/api/v1/global-settings/oauth", self.base_url))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("GET /api/v1/global-settings/oauth");
    assert!(
        get_resp.status().is_success(),
        "GET oauth settings: expected 2xx, got {}",
        get_resp.status()
    );
    let etag = get_resp
        .headers()
        .get("etag")
        .and_then(|v| v.to_str().ok())
        .expect("ETag header on GET oauth settings")
        .to_string();

    // Step 2: PUT with the captured ETag — IfMatch extractor rejects wildcard *.
    // Include dcr_enabled = true: the seed sets it false; the E2E test needs DCR to work.
    let put_resp = client
        .put(format!("{}/api/v1/global-settings/oauth", self.base_url))
        .header("Authorization", format!("Bearer {token}"))
        .header("If-Match", &etag)
        .json(&serde_json::json!({
            "canonical_host": canonical_host,
            "dcr_enabled": true
        }))
        .send()
        .await
        .expect("PUT /api/v1/global-settings/oauth");
    assert!(
        put_resp.status().is_success(),
        "update_oauth_settings: expected 2xx, got {}",
        put_resp.status()
    );
}

- [ ] **Step 3: Add `auto_approve_consent` helper**

```rust
/// POST /oauth/test/auto-approve/{request_id}
///
/// Approves the pending OAuth consent request, returning the authorization code
/// extracted from the `code=` query parameter of the returned `redirect_uri`.
///
/// Panics if the request fails, returns non-2xx, or `code=` is absent in the URI.
pub(crate) async fn auto_approve_consent(&self, request_id: &str) -> String {
    let token = self
        .api_token
        .as_deref()
        .expect("must call register_and_login_with_token first");
    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .expect("build reqwest client");
    let resp = client
        .post(format!(
            "{}/oauth/test/auto-approve/{request_id}",
            self.base_url
        ))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("POST /oauth/test/auto-approve");
    assert!(
        resp.status().is_success(),
        "auto_approve_consent: expected 2xx, got {}",
        resp.status()
    );
    let body: serde_json::Value = resp.json().await.expect("parse auto-approve response");
    let redirect_uri = body["redirect_uri"]
        .as_str()
        .expect("redirect_uri in response");
    // Extract code= from redirect_uri query string.
    url::Url::parse(redirect_uri)
        .expect("parse redirect_uri")
        .query_pairs()
        .find(|(k, _)| k == "code")
        .map(|(_, v)| v.into_owned())
        .expect("code= parameter in redirect_uri")
}
````

If `url` crate is not a dependency of `uptrakit-integration-tests`, use string splitting instead:

```rust
redirect_uri
    .split('?')
    .nth(1)
    .and_then(|q| {
        q.split('&')
            .find(|p| p.starts_with("code="))
            .map(|p| p.trim_start_matches("code=").to_owned())
    })
    .expect("code= parameter in redirect_uri")
```

- [ ] **Step 4: Compile check**

```bash
cargo check --all-features -p uptrakit-integration-tests 2>&1 | tail -10
```

Expected: clean compile.

- [ ] **Step 5: Commit**

```bash
git add crates/core/integration-tests/tests/helpers/api_client.rs
git commit -m "feat(integration-tests): add update_oauth_settings and auto_approve_consent to ApiClient"
```

---

## Task 4: Complete steps 3–9 of `oauth_end_to_end_mcp_rs_round_trip`

**Files:**

- Modify: `crates/core/integration-tests/tests/oauth_end_to_end.rs`

- [ ] **Step 1: Read current test body**

Read lines 85–252 of `oauth_end_to_end.rs` to understand exactly what is already there and what the
`eprintln!("...skipped...")` block replaces.

The test already has:

- Steps 1–2: `ControllerContainer::start`, `register_and_login_with_token` ✓
- `generate_pkce_pair()` call saving to `(_code_verifier, _code_challenge)` ✓
- Steps 5–9: all in comments / eprintln stub

- [ ] **Step 2: Build the raw reqwest client**

Uncomment / add the raw reqwest client that was in the comment block:

```rust
let http = reqwest::Client::builder()
    .danger_accept_invalid_certs(true)
    .connect_timeout(Duration::from_secs(10))
    .timeout(Duration::from_secs(60))
    .redirect(reqwest::redirect::Policy::none())
    .build()
    .expect("build reqwest client");
```

- [ ] **Step 3: Implement Step 3 — enable OAuth via `update_oauth_settings`**

Replace the "INFRASTRUCTURE GAP" comment block for Step 3 with:

```rust
// Step 3 — Enable the MCP OAuth server via canonical_host auto-enable.
// No mcp_enabled field needed: absent row + canonical_host set → auto-enabled.
api_client
    .update_oauth_settings(&format!("127.0.0.1:{port}"))
    .await;
```

- [ ] **Step 4: Implement Step 4 — reexec and wait for new generation**

```rust
// Step 4 — Force reexec so OAuth boots live, then wait for the new generation.
// Read current_gen BEFORE reexec so we know what to wait for.
let current_gen: u64 = {
    let healthz_resp = http
        .get(format!("https://127.0.0.1:{port}/healthz"))
        .send()
        .await
        .expect("GET /healthz");
    healthz_resp
        .headers()
        .get("x-reexec-generation")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
};
api_client.force_reexec().await;
api_client
    .wait_for_generation(current_gen + 1, Duration::from_secs(30))
    .await;
```

- [ ] **Step 5: Implement Step 5 — DCR registration**

```rust
// Step 5 — Register an OAuth client via Dynamic Client Registration.
let dcr_resp = http
    .post(format!("https://127.0.0.1:{port}/oauth/register"))
    .json(&serde_json::json!({
        "client_name": "e2e-test-client",
        "redirect_uris": ["https://localhost/callback"],
        "grant_types": ["authorization_code"],
        "response_types": ["code"],
        "token_endpoint_auth_method": "none",
        "scope": "mcp:read"
    }))
    .send()
    .await
    .expect("POST /oauth/register");
assert_eq!(
    dcr_resp.status().as_u16(),
    201,
    "DCR must return 201, got: {}",
    dcr_resp.status()
);
let dcr_body: serde_json::Value = dcr_resp.json().await.expect("parse DCR response");
let client_id = dcr_body["client_id"]
    .as_str()
    .expect("client_id in DCR response")
    .to_string();
```

- [ ] **Step 6: Implement Step 6 — GET /oauth/authorize with PKCE**

Remove the `let (_code_verifier, _code_challenge) = generate_pkce_pair();` line (the underscored
bindings) and replace with active bindings:

```rust
let (code_verifier, code_challenge) = generate_pkce_pair();
```

Then add the authorize request:

```rust
// Step 6 — Drive GET /oauth/authorize with PKCE.
// MUST include the API token so the optional_auth middleware resolves the user;
// without it the authorize endpoint returns a login redirect, not a consent redirect.
let api_token = api_client
    .api_token
    .as_deref()
    .expect("api_token populated after register_and_login_with_token");
let auth_resp = http
    .get(format!("https://127.0.0.1:{port}/oauth/authorize"))
    .header("Authorization", format!("Bearer {api_token}"))
    .query(&[
        ("response_type", "code"),
        ("client_id", &client_id),
        ("redirect_uri", "https://localhost/callback"),
        ("scope", "mcp:read"),
        ("state", "test-state-e2e"),
        ("code_challenge", &code_challenge),
        ("code_challenge_method", "S256"),
        ("resource", &format!("https://127.0.0.1:{port}/mcp")),
    ])
    .send()
    .await
    .expect("GET /oauth/authorize");

let location = auth_resp
    .headers()
    .get("location")
    .and_then(|v| v.to_str().ok())
    .expect("authorize must 302 to /oauth/consent/<id>")
    .to_string();
assert!(
    location.starts_with("/oauth/consent/"),
    "expected /oauth/consent/<id>, got: {location}"
);
assert!(
    !location.contains("code="),
    "consent must not be pre-granted — revoke existing grants or use a fresh client"
);

let request_id = location
    .trim_start_matches("/oauth/consent/")
    .to_string();
```

- [ ] **Step 7: Implement Step 7 — auto-approve consent**

```rust
// Step 7 — Bypass consent UI via test-utils endpoint.
let code = api_client.auto_approve_consent(&request_id).await;
```

- [ ] **Step 8: Implement Step 8 — token exchange**

```rust
// Step 8 — Exchange the authorization code for an access token.
let token_resp = http
    .post(format!("https://127.0.0.1:{port}/oauth/token"))
    .form(&[
        ("grant_type", "authorization_code"),
        ("code", code.as_str()),
        ("redirect_uri", "https://localhost/callback"),
        ("client_id", client_id.as_str()),
        ("code_verifier", code_verifier.as_str()),
        ("resource", &format!("https://127.0.0.1:{port}/mcp")),
    ])
    .send()
    .await
    .expect("POST /oauth/token");
assert_eq!(
    token_resp.status().as_u16(),
    200,
    "token exchange must return 200, got: {}",
    token_resp.status()
);
let token_body: serde_json::Value = token_resp.json().await.expect("parse token response");
let access_token = token_body["access_token"]
    .as_str()
    .expect("access_token in token response")
    .to_string();
```

- [ ] **Step 9: Implement Step 9 — authenticated MCP call**

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

- [ ] **Step 10: Remove `#[ignore]` and the `eprintln!` stub**

- Remove the `#[ignore = "..."]` attribute from the test function.
- Remove the `eprintln!("oauth_end_to_end_mcp_rs_round_trip: Steps 5–9 skipped...")` line.
- Remove the `#![expect(dead_code, reason = "...")]` attribute now that all helpers are used.

- [ ] **Step 11: Compile check**

```bash
cargo check --all-features -p uptrakit-integration-tests 2>&1 | tail -10
```

Expected: clean compile.

- [ ] **Step 12: Commit**

```bash
git add crates/core/integration-tests/tests/oauth_end_to_end.rs
git commit -m "feat(e2e): complete oauth_end_to_end_mcp_rs_round_trip steps 3-9"
```

---

## Quality Gates

Plan A must be complete (and Docker image rebuilt) before this plan's tests can pass.

```bash
# Compile and static analysis (no Docker required)
cargo check --all-features
cargo clippy --all-targets --all-features

# Full E2E test (requires uptrakit-test:latest Docker image)
# Build the image first if it's out of date:
docker build -f docker/Dockerfile.test -t uptrakit-test:latest .

# Run the E2E test:
cargo test -p uptrakit-integration-tests -- oauth_end_to_end_mcp_rs_round_trip --nocapture
```
