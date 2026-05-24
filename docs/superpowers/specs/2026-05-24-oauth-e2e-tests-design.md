# OAuth E2E Test Coverage — Spec

- **Date:** 2026-05-24
- **Status:** Approved

## Overview

Add complete end-to-end test coverage for two OAuth flows:

1. **Device Authorization Grant (RFC 8628)** — full HTTP chain via `POST /api/v1/auth/device/approve`,
   verified by calling an authenticated API endpoint with the minted token.
2. **MCP OAuth 2.1 Authorization Code + PKCE** — full in-process round-trip from `GET /oauth/authorize`
   through token exchange, verified by JWT claim assertion and `McpOAuthJwtVerifier` acceptance.
3. **Device Approval UI** — Playwright tests for the `/device` page (mocked backend).

Real MCP endpoint coverage (`POST /mcp` with OAuth JWT) is already provided by the existing Docker
system test `oauth_end_to_end_mcp_rs_round_trip` and requires no changes.

---

## Existing Coverage (not duplicated)

### Backend — in-process

| File                                           | Coverage                                                                                                                                                                                                                                                                                                                                        |
| ---------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `integration_tests/device_auth_oauth.rs`       | 15 tests: RFC 8628 protocol shape, error codes, slow_down, access_denied. **Approval via internal state injection only** — `device_flow_store.approve()`. HTTP approve endpoint untested. Deny endpoint tested via HTTP (401, 200, 404, 403 shape tests) but **no test chains deny → `POST /oauth/token` → assert `access_denied` poll error**. |
| `integration_tests/oauth_boot_validation.rs`   | Boot validation: missing host, minimal config, multi-controller fingerprint guard                                                                                                                                                                                                                                                               |
| `integration_tests/oauth_master_switch_off.rs` | All OAuth surfaces return 404 when `oauth.mcp_enabled = false`                                                                                                                                                                                                                                                                                  |
| `routes/oauth/consent.rs` (inline)             | GET consent details (200, 401, 403); POST approve (redirects with code); POST deny (redirects with access_denied); OAuth disabled 404                                                                                                                                                                                                           |
| `routes/oauth/authorize.rs` (inline)           | Unauthenticated → 302 to login; authenticated → 302 to consent; redirect_uri mismatch; wrong client_id; skip_prompt on existing consent                                                                                                                                                                                                         |

### Backend — Docker system tests

| File                                          | Coverage                                                                                                                                                                                                                                                                          |
| --------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `integration-tests/tests/oauth_end_to_end.rs` | MCP OAuth 2.1 auth-code + PKCE round-trip ending at `POST /mcp` (`get_current_user`, `mcp:read`). **Bypasses the real consent handler** via `POST /oauth/test/auto-approve/{request_id}` test-utils backdoor — the real `POST /oauth/consent/{id}/approve` path is not exercised. |

### MCP crate — unit tests

| File                                     | Coverage                                                                                  |
| ---------------------------------------- | ----------------------------------------------------------------------------------------- |
| `mcp/tests/oauth_prefix_dispatch.rs`     | JWT verifier: valid, wrong signature, expired, wrong issuer, garbage token, missing token |
| `mcp/tests/oauth_rs_audience_binding.rs` | Wrong/correct audience binding                                                            |

### Frontend — Playwright (mocked)

| File                                     | Coverage                                                                                                                     |
| ---------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------- |
| `tests/e2e/oauth-consent.spec.ts`        | Consent page: trusted/unverified client, approve redirect, deny redirect, localhost warning, DCR warning, scope descriptions |
| `tests/e2e/oauth-login-redirect.spec.ts` | Login → OAuth authorize redirect loop fix                                                                                    |

---

## New Artifacts

### 1. `crates/ui/web-api/src/integration_tests/device_auth_http_roundtrip.rs`

Full device flow through HTTP endpoints only — no internal state injection.

**Infrastructure:** `TestApp` + `TestClient`. Follows patterns in `device_auth_oauth.rs`.

#### Test: `device_flow_full_http_chain_token_works_at_api`

```text
POST /api/v1/oauth/device_authorization  (client_id=uptrakit-cli)
  → assert 200, capture user_code + device_code

Register user via /api/v1/auth/register, capture JWT

POST /api/v1/auth/device/approve
  Authorization: Bearer <user_jwt>
  Body: { "user_code": "<user_code>" }
  → assert 200

POST /api/v1/oauth/token  (form-encoded, values percent-encoded)
  grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Adevice_code
  device_code=<device_code percent-encoded>
  client_id=uptrakit-cli
  → assert 200
  → assert token_type = "Bearer"
  → assert access_token starts with "upk_"

Use the urlencoded() helper from device_auth_oauth.rs (or equivalent percent-encoding) —
the `:` and `/` in the grant_type urn must be encoded as `%3A` and `%2F` in the form body.

GET /api/v1/auth/me
  Authorization: Bearer <access_token>
  → assert 200
  → assert response.email matches registered user
```

Verifies: `POST /api/v1/auth/device/approve` (zero existing HTTP coverage), `upk_` token usable on standard authenticated endpoints.

#### Test: `device_flow_deny_via_http_returns_access_denied_on_poll`

```text
POST /api/v1/oauth/device_authorization  (client_id=uptrakit-cli)
  → assert 200, capture user_code + device_code

Register user via /api/v1/auth/register, capture JWT

POST /api/v1/auth/device/deny
  Authorization: Bearer <user_jwt>
  Body: { "user_code": "<user_code>" }
  → assert 200

POST /api/v1/oauth/token  (form-encoded, values percent-encoded)
  grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Adevice_code
  device_code=<device_code percent-encoded>
  client_id=uptrakit-cli
  → assert HTTP 400
  → assert error = "access_denied"
```

Verifies: full HTTP chain `POST /api/v1/auth/device/deny` → `POST /oauth/token` poll →
RFC 8628 `access_denied` error code. No existing test exercises this end-to-end sequence.

**Constraints:**

- No `tokio::time::sleep` or wall-clock waits.
- No `start_paused` (DB-backed test — SQLx pool timers auto-fire).
- No `app.state.auth.device_flow_store` access.
- Both test files need `#![expect(clippy::expect_used, reason = "...")]` at module level
  (same as `device_auth_oauth.rs` lines 1–4).

---

### 2. `crates/ui/web-api/src/integration_tests/oauth_mcp_roundtrip.rs`

Full MCP OAuth authorization-code + PKCE round-trip in-process, using the **real**
`POST /oauth/consent/{id}/approve` handler (not the test-utils backdoor used by the Docker test).
Terminates at JWT claim assertion — no `/mcp` call (MCP router is not part of `build_router()`;
`/mcp` endpoint coverage owned by the existing Docker test). Primary value: first test that
exercises the real consent→token chain **and** asserts specific JWT claim values (`iss`, `aud`,
`sub`, `scope`) on a real minted token — catching issuer/audience misconfiguration that HTTP-200
checks at the MCP layer cannot.

**Infrastructure:** Borrows setup steps from `ConsentTestApp` in `routes/oauth/consent.rs` but uses
`TestClient::new(router)` (not `router.oneshot()` as that module does):

- `setup_migrated_db()` + `insert_default_tenant()` + `build_test_state()`.
- Patch `AppState.oauth` with `OAuthState { enabled: true, ... }` — same secret + canonical URL used
  for both signing and verification. Use `Arc::new(OffsetDateTime::now_utc)` as the clock (same as
  `enabled_oauth_state` in `consent.rs`) — a fixed past timestamp causes `ar_svc.consume()` to
  treat the authorization request as expired and return 410 Gone.
- Add optional-auth middleware so `GET /oauth/authorize` sees the user JWT.
- `TestClient::new(patched_router)` — same constructor used in `oauth_enabled_client()` helper.

These files live in `integration_tests/` which is gated at module level by
`#[cfg(all(test, feature = "db-sqlite"))]` in `lib.rs` — no per-file `#[cfg]` needed.

Both new Rust files must include at module level:

```rust
#![expect(
    clippy::expect_used,
    reason = "test helper functions are not covered by allow-expect-in-tests"
)]
```

Matches the pattern in `device_auth_oauth.rs` lines 1–4. Without it, `cargo clippy --all-features`
fails on `.expect()` calls inside test helpers (workspace `clippy::all = "deny"`).

**DB fixtures:**

`insert_oauth_client(db, redirect_uri, trusted: bool) -> String` — inserts an `oauth_client` row,
returns `client_id`. Trusted = `trusted_at` set to now. This helper already exists as a private
function in `routes/oauth/consent.rs` `mod tests` (line 578); **move** it to
`test_harness/fixtures.rs` as `pub(crate)` and update all five call sites in `consent.rs` (lines
656, 692, 720, 743, 787) to use `crate::test_harness::fixtures::insert_oauth_client`. Do not
duplicate the function.

**PKCE constants:**

```text
CODE_VERIFIER  = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"
CODE_CHALLENGE = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"  // base64url(SHA256(CODE_VERIFIER))
REDIRECT_URI   = "https://localhost/callback"
RESOURCE       = "https://controller.example.com/mcp"
```

These match the canonical RFC 7636 §4.6 test vectors confirmed in `crates/ui/web-api/src/oauth/pkce.rs`.
The verifier is the random string the client holds; the challenge is its SHA-256/base64url hash sent
to the AS.

#### Test: `mcp_oauth_auth_code_pkce_roundtrip_token_claims_valid`

```text
Insert trusted oauth_client (scope=mcp:read, redirect_uri=REDIRECT_URI)
Register user → mint user JWT (all permissions)

GET /oauth/authorize
  ?response_type=code
  &client_id=<client_id>
  &redirect_uri=REDIRECT_URI
  &scope=mcp:read
  &state=test-state-123
  &code_challenge=CODE_CHALLENGE
  &code_challenge_method=S256
  &resource=RESOURCE
  Authorization: Bearer <user_jwt>
  → assert 302
  → assert Location = /oauth/consent/<request_id>

POST /oauth/consent/<request_id>/approve
  Authorization: Bearer <user_jwt>
  Body: {}
  → assert 200
  → assert redirect_to contains "code="

Extract authorization_code from redirect_to query param

POST /oauth/token  (form-encoded)
  grant_type=authorization_code
  code=<authorization_code>
  redirect_uri=REDIRECT_URI
  client_id=<client_id>
  code_verifier=CODE_VERIFIER
  resource=RESOURCE
  → assert 200
  → assert token_type = "Bearer"

Decode JWT (no signature check at this step — just claims)
  → assert iss = "https://controller.example.com"
  → assert aud contains RESOURCE
  → assert sub = user_id (UUID string)
  → assert scope = "mcp:read"
  → assert exp > now

McpOAuthJwtVerifier::new(secret, iss.to_string(), vec![RESOURCE.to_string()]).verify(&access_token)
  → assert Ok(claims)
  → assert claims.sub = user_id
```

#### Test: `mcp_oauth_deny_consent_yields_access_denied_redirect`

```text
Insert trusted oauth_client
Register user → mint user JWT

GET /oauth/authorize  (same params as above)
  → assert 302, extract request_id from Location

POST /oauth/consent/<request_id>/deny
  Authorization: Bearer <user_jwt>
  → assert 200
  → assert redirect_to contains "error=access_denied"
  → assert redirect_to contains "state=test-state-123"
```

**Constraints:**

- Test secret must match between `McpOAuthJwtSigner` (used by token endpoint) and `McpOAuthJwtVerifier`
  (used in assertion). Use a fixed test constant, e.g. `b"mcp-roundtrip-test-secret-32b!!"`.
- Canonical URL must match the `iss` claim and the verifier's `issuer` argument.
- `resource` param must match the verifier's accepted audience list.
- No wall-clock sleeps. No `start_paused`.

---

### 3. `frontend/tests/e2e/device-approval.spec.ts`

Playwright tests for the `/device` page. All API calls mocked. Follows patterns from `oauth-consent.spec.ts`.

**Shared session mock** (reuse `mockAuthenticatedSession` pattern):

```typescript
// Mocks: /api/v1/auth/refresh → 200 with tokens
//        /api/v1/auth/me → 200 with user object
//        /api/v1/system/alerts → 200 { alerts: [] }
```

**Per-test mocks:**

| Mock           | Glob pattern                    | Method filter | Response                                                              |
| -------------- | ------------------------------- | ------------- | --------------------------------------------------------------------- |
| Lookup success | `**/api/v1/auth/device/lookup`  | `GET` only    | `{ client_name: "uptrakit CLI", expires_at: "2099-01-01T00:00:00Z" }` |
| Lookup 404     | `**/api/v1/auth/device/lookup`  | `GET` only    | HTTP 404                                                              |
| Approve        | `**/api/v1/auth/device/approve` | `POST` only   | HTTP 200                                                              |
| Deny           | `**/api/v1/auth/device/deny`    | `POST` only   | HTTP 200                                                              |

Playwright `page.route()` glob patterns do not match query strings — `**` does not expand across
`?`. Omit the query string from the glob; filter by method inline (pattern from
`oauth-consent.spec.ts` line 52: `if (route.request().method() === 'GET')`). The lookup URL check
(`user_code` value) is optional since tests use a fixed code value and the mock intercepts all
GET requests to the lookup path.

**Selectors (derive from existing component markup):**

```typescript
const CONSENT_PROMPT = '[data-ui="consent-prompt"]';
const APPROVE_BUTTON = 'button:has-text("Approve")';
const DENY_BUTTON = 'button:has-text("Deny")';
const SUCCESS_CALLOUT = '[data-ui="callout"][data-tone="success"]';
const DENIED_CALLOUT = '[data-ui="callout"][data-tone="warning"]';
const ERROR_CALLOUT = '[data-ui="callout"][data-tone="danger"]';
```

#### Tests (5)

**`pre-filled code triggers lookup and shows consent prompt`**

- Navigate to `/device?user_code=BCDF-GHJK`
- Authenticated session + lookup mock (200)
- Assert: `CONSENT_PROMPT` visible, contains "uptrakit CLI"
- Assert: `APPROVE_BUTTON` and `DENY_BUTTON` enabled

**`approve calls approve endpoint and shows success callout`**

- Navigate to `/device?user_code=BCDF-GHJK`
- Authenticated session + lookup mock (200) + approve mock (200)
- Click `APPROVE_BUTTON`
- Wait for approve response
- Assert: `SUCCESS_CALLOUT` visible

**`deny calls deny endpoint and shows denied callout`**

- Navigate to `/device?user_code=BCDF-GHJK`
- Authenticated session + lookup mock (200) + deny mock (200)
- Click `DENY_BUTTON`
- Wait for deny response
- Assert: `DENIED_CALLOUT` visible

**`invalid user_code shows error callout`**

- Navigate to `/device?user_code=BCDF-GHJK`
- Authenticated session + lookup mock (404)
- Assert: `ERROR_CALLOUT` visible
- Assert: `CONSENT_PROMPT` not visible

**`unauthenticated user sees login prompt`**

- Navigate to `/device?user_code=BCDF-GHJK`
- No session mocks (unauthenticated)
- Assert: login link (`a[href*="/login"]`) visible
- Assert: `CONSENT_PROMPT` not visible

**Constraints:**

- No snapshot tests.
- Prettier config: `useTabs: true`, `singleQuote: true`, `trailingComma: 'none'`, `printWidth: 120`.
- TypeScript strict mode.
- Selectors must be stable across light/dark/mobile viewports (no color-dependent selectors).

---

## Out of Scope

- `mcp:write` scope verification (deferred)
- Docker-level device flow system test (in-process HTTP chain deemed sufficient)
- Keyboard UX tests for device code entry
- Refresh token flow (explicitly deferred per ADR 0009 Seam 1)
- Snapshot/visual parity tests for `/device` page

---

## Quality Gates

```bash
# Backend
cargo fmt --all
cargo check --no-default-features --features db-sqlite
cargo check --all-features
cargo clippy --all-targets --no-default-features --features db-sqlite
cargo clippy --all-targets --all-features
cargo test --all-features
cargo deny check

# Frontend
cd frontend && npm run lint && npm run format:check && npm run test:e2e
```

---

## Documentation Impact

No externally observable behavior changes. No API surface additions. No doc updates required — these are test-only files.

---

## Files Created

| File                                                                    | Type                                                                                                                         |
| ----------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------- |
| `crates/ui/web-api/src/integration_tests/device_auth_http_roundtrip.rs` | New                                                                                                                          |
| `crates/ui/web-api/src/integration_tests/oauth_mcp_roundtrip.rs`        | New                                                                                                                          |
| `crates/ui/web-api/src/integration_tests/mod.rs`                        | Modified (add two `mod` declarations)                                                                                        |
| `crates/ui/web-api/src/test_harness/fixtures.rs`                        | Modified (move `insert_oauth_client` from `consent.rs`, expose as `pub(crate)`)                                              |
| `crates/ui/web-api/src/routes/oauth/consent.rs`                         | Modified (remove private `insert_oauth_client`, update 5 call sites to `crate::test_harness::fixtures::insert_oauth_client`) |
| `frontend/tests/e2e/device-approval.spec.ts`                            | New                                                                                                                          |
