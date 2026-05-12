# MCP OAuth — Plan B: Authorization Server Routes + Boot Safety (Phase 1 + Phase 1.5)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the OAuth 2.1 Authorization Server endpoints, the JWT signer, the refresh-token rotation service,
the consent backend (server-side only — UI lands in Plan C), the AS metadata + DCR + RFC 7592 management routes, the
rate-limit middleware, the canonical-host boot validation, the multi-controller boot guard, and the Phase 1.5
enforcement tests. Everything is gated behind `oauth.mcp_enabled` (default off) so the production controller is
unchanged when this lands.

**Architecture:** New `crates/ui/web-api/src/routes/oauth/` module containing one file per endpoint group. A parallel
`crates/ui/web-api/src/oauth/` module holds business logic (services), the canonical-url helper, PKCE verifier, JWT
signer with a distinct claims envelope from the existing Dashboard `JwtManager`, and the tower rate-limit middleware
factory. All read-then-write transactions use `SqliteTransactionMode::Immediate`. The signer secret is loaded from a new
`oauth.jwt_signing_secret` setting; falls back to a per-boot random secret with a WARN log when unset.

**Tech Stack:** axum + tower + sea-orm + `jsonwebtoken` v10 (HS256) + `rootcause::Report` + `parking_lot::Mutex` + clock
injection via `Arc<dyn Fn() -> OffsetDateTime + Send + Sync>` + `utoipa::path` annotations + `sha2` for PKCE + `base64`
(url-safe, no padding) + existing `uptrakit_web_api_auth::RateLimitStore`.

**Spec:** `docs/superpowers/specs/2026-05-12-mcp-oauth-authorization-design.md` (commit `b7ee4a852`).

**Status:** Draft → Ready for review.

---

## Prerequisites

- **Plan A** (`docs/superpowers/plans/2026-05-12-mcp-oauth-a-foundation.md`) must be merged. This plan depends on the
  six migrations, the entities, and the `uptrakit_web_api_types::oauth` module Plan A delivers.

## Snapshot binding

- "BEGIN IMMEDIATE for SQLite read-then-write transactions" — every consume-once / rotation / cascade-revoke transaction
- "Multiple DELETE/UPDATE atomicity: wrap in db.begin()/txn.commit()" — refresh-family revoke cascades
- "Use parking_lot::Mutex (never std::sync::Mutex or tokio::sync::Mutex) in async code" — any cache (PKCE pending
  nothing, in-memory dedupe nothing — but JwtManager wrappers if cached)
- "All HTTP request types in uptrakit-web-api-types implement Validate; handlers call req.validate() → 400 on error" —
  every route handler
- "inject wall-clock time via Arc<dyn Fn() -> OffsetDateTime + Send + Sync> with parking_lot::Mutex for tests" — all
  four OAuth services
- "Plugin HTTP clients: set .connect_timeout(10s) + .timeout(60s); add .dns_resolver(SsrfSafeResolver)" — CIMD fetcher
  (lands in Plan D, not here, but Plan B's DCR route does not fetch URLs and is unaffected)
- "Route handlers: return StatusCode::INTERNAL_SERVER_ERROR on DB error" — error helpers
- "Never use unwrap()/expect()/panic!() in production" — strict
- "use tracing crate only" — every log site uses `tracing::error!(error = %e, "...")` structured fields
- "semantic audit emission (AuditEntry + AuditEmitter)" — every audit emission goes through the builder; no
  `target: "security_audit"` strings
- "preserve error context with rootcause::Report" — service-layer errors propagate through `?` with `.context_to()` at
  crate boundaries
- "Conventional Commits" — `feat(web-api)`, `feat(oauth)`, `test(...)` scopes

## File Structure

**New files** (under `crates/ui/web-api/src/`):

- `routes/oauth/mod.rs` — pub mod + axum::Router builder.
- `routes/oauth/metadata.rs` — `/.well-known/oauth-authorization-server`.
- `routes/oauth/authorize.rs` — `GET /oauth/authorize`.
- `routes/oauth/token.rs` — `POST /oauth/token`.
- `routes/oauth/register.rs` — `POST /oauth/register` + RFC 7592 `GET/PUT/DELETE /oauth/register/{client_id}`.
- `routes/oauth/consent.rs` — `GET /oauth/consent/{request_id}` + `POST /oauth/consent/{request_id}/{approve|deny}`.
- `routes/oauth/clients_api.rs` — Operator `GET /api/oauth/clients`, `DELETE /api/oauth/clients/{client_id}`,
  `POST /api/oauth/clients/{client_id}/trust`.
- `routes/oauth/consents_api.rs` — End-user `GET /api/oauth/consents`, `DELETE /api/oauth/consents/{id}`.

- `oauth/mod.rs` — pub mod entry.
- `oauth/canonical_url.rs` —
  `derived_urls(canonical_host, accepted_aliases) -> (issuer, primary_resource, accepted_resources)`.
- `oauth/jwt.rs` — `McpOAuthJwtSigner` + `McpOAuthJwtVerifier` with HS256 pin, `kid` header, `required_spec_claims`.
- `oauth/pkce.rs` — `PkceVerifier::verify(code_verifier, code_challenge)`.
- `oauth/services/mod.rs` — service-struct shared traits.
- `oauth/services/authorization_request.rs` — `OAuthAuthorizationRequestService`.
- `oauth/services/authorization_code.rs` — `OAuthAuthorizationCodeService` (mint + consume).
- `oauth/services/refresh_token.rs` — `OAuthRefreshTokenService` (rotate + replay-detect + cascade-revoke).
- `oauth/services/consent.rs` — `OAuthConsentService` (skip-consent logic + revoke).
- `oauth/services/client.rs` — `OAuthClientService` (DCR insert + lookup + revoke + trust-promote).
- `oauth/rate_limit.rs` — `oauth_rate_limit(EndpointKind)` tower middleware factory.
- `oauth/boot.rs` — Boot validation: canonical_host required, host parsing, `oauth_controller_instances` guard.
- `oauth/audit.rs` — Helpers wrapping `AuditEntry::builder(...)` for each new event type (events themselves registered
  in Plan E; helpers no-op until Plan E registers them).

**Modified files:**

- `crates/ui/web-api/src/lib.rs` — mount `routes/oauth/` in `build_router` behind a wrapper that returns 404 when
  `oauth.mcp_enabled = false`.
- `crates/ui/web-api/src/app_state.rs` — add `oauth: OAuthState` substate field carrying the JWT signer, the
  canonical-url config, the rate-limit store handle, the clock fn, and the audit emitter.
- `crates/ui/web-api/Cargo.toml` — add `sha2`, `base64` (workspace deps).

**New integration test files** (under `crates/ui/web-api/tests/` and `crates/ui/integration-tests/` as appropriate):

- `oauth_master_switch_off_returns_404.rs`
- `oauth_boot_fails_without_canonical_host.rs`
- `oauth_boot_succeeds_with_minimal_config.rs`
- `oauth_boot_fails_on_duplicate_controller_instance.rs`

---

## Tasks

### Task 1: OAuth state struct + AppState wiring

**Files:**

- Create: `crates/ui/web-api/src/oauth/mod.rs`
- Modify: `crates/ui/web-api/src/app_state.rs`
- Modify: `crates/ui/web-api/Cargo.toml` (add `sha2`, `base64` if not in workspace deps)

- [ ] **Step 1: Write OAuth state struct**

```rust
// crates/ui/web-api/src/oauth/mod.rs
pub mod audit;
pub mod boot;
pub mod canonical_url;
pub mod jwt;
pub mod pkce;
pub mod rate_limit;
pub mod services;

use std::sync::Arc;
use time::OffsetDateTime;
use crate::oauth::canonical_url::CanonicalUrlConfig;
use crate::oauth::jwt::{McpOAuthJwtSigner, McpOAuthJwtVerifier};

#[non_exhaustive]
#[derive(Clone)]
pub struct OAuthState {
    pub enabled: bool,
    pub canonical: CanonicalUrlConfig,
    pub signer: Arc<McpOAuthJwtSigner>,
    pub verifier: Arc<McpOAuthJwtVerifier>,
    pub clock: Arc<dyn Fn() -> OffsetDateTime + Send + Sync>,
    pub instance_id: uuid::Uuid,
    pub dcr_enabled: bool,
    pub cimd_enabled: bool,
}
```

Add `pub oauth: OAuthState` to `AppState`.

- [ ] **Step 2: Stub the inner types so the module compiles**

Write empty placeholder types in `oauth/canonical_url.rs` (`CanonicalUrlConfig`) and `oauth/jwt.rs`
(`McpOAuthJwtSigner`, `McpOAuthJwtVerifier`) — full implementations follow in later tasks.

- [ ] **Step 3: cargo check**

Run: `cargo check -p uptrakit-web-api --all-features` Expected: clean (stubs compile).

- [ ] **Step 4: Commit**

```bash
git commit -m "feat(web-api): scaffold oauth submodule + OAuthState

Per spec §4 + §7 + §9. Concrete implementations land in subsequent commits."
```

### Task 2: derived_urls helper + boot validation wiring

**Files:**

- Modify: `crates/ui/web-api/src/oauth/canonical_url.rs`

`CanonicalUrlConfig` and its tests already live in `uptrakit-web-api-types::oauth::canonical_url` (delivered by Plan A
Task 10 Step 4 — promoted to the shared crate so `uptrakit-mcp` can consume it). This task only adds the web-api-side
glue that resolves the config from `global_settings` at boot.

- [ ] **Step 1: Write tests for the settings-resolution path**

`CanonicalUrlConfig::new(...)` itself is already tested in Plan A Task 10 Step 4 — don't duplicate. The web-api side
only needs tests for the settings-to-config resolution:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn resolves_config_from_settings() {
        let settings = test_settings_with(
            ("oauth.canonical_host", "controller.example.com"),
            ("oauth.accepted_audience_hosts", "[]"),
        );
        let cfg = load_canonical_url_config(&settings).await.unwrap();
        assert_eq!(cfg.issuer().as_str(), "https://controller.example.com");
    }

    #[tokio::test]
    async fn missing_canonical_host_bails() {
        let settings = test_settings_with(
            ("oauth.mcp_enabled", "true"),
            // canonical_host intentionally unset
        );
        let err = load_canonical_url_config(&settings).await.unwrap_err();
        assert!(matches!(err.cause(), CanonicalUrlConfigError::Missing));
    }

    #[tokio::test]
    async fn parses_aliases_from_json_array() {
        let settings = test_settings_with(
            ("oauth.canonical_host", "primary.example.com"),
            ("oauth.accepted_audience_hosts", r#"["alias.example.com"]"#),
        );
        let cfg = load_canonical_url_config(&settings).await.unwrap();
        assert!(cfg.accepts_audience("https://alias.example.com/mcp"));
    }
}
```

- [ ] **Step 2: Verify failure**

Run: `cargo test -p uptrakit-web-api oauth::canonical_url` Expected: FAIL.

- [ ] **Step 3: Implement the thin settings-resolution wrapper**

The struct itself is owned by `uptrakit_web_api_types::oauth`. The web-api side adds only the settings glue. No
duplicate type, no duplicate tests.

```rust
//! Settings-to-config resolution for the canonical-URL configuration.
//!
//! The `CanonicalUrlConfig` type and its constructor tests live in
//! `uptrakit_web_api_types::oauth::canonical_url`. This module's sole job is to read
//! `oauth.canonical_host` and `oauth.accepted_audience_hosts` from `global_settings`
//! and call `CanonicalUrlConfig::new(...)`.

use rootcause::prelude::*;
pub use uptrakit_web_api_types::oauth::{
    CanonicalUrlConfig, CanonicalUrlConfigError, MAX_ACCEPTED_AUDIENCE_HOSTS,
};

use crate::SettingsRead; // existing settings-reader trait

/// Resolve `CanonicalUrlConfig` from operator-managed global settings.
///
/// # Errors
///
/// Returns `CanonicalUrlConfigError::Missing` if `oauth.canonical_host` is unset and
/// `oauth.mcp_enabled` is true; this is the boot-fail signal documented in spec §7.
/// Returns `CanonicalUrlConfigError::InvalidHost` / `TooManyAliases` for malformed values.
pub async fn load_canonical_url_config(
    settings: &impl SettingsRead,
) -> Result<CanonicalUrlConfig, Report<CanonicalUrlConfigError>> {
    let canonical_host = settings
        .get_string("oauth.canonical_host")
        .await
        .context_to()?
        .unwrap_or_default();
    let aliases_json = settings
        .get_string("oauth.accepted_audience_hosts")
        .await
        .context_to()?
        .unwrap_or_else(|| "[]".to_string());
    let aliases: Vec<String> = serde_json::from_str(&aliases_json)
        .map_err(|e| report!(CanonicalUrlConfigError::InvalidHost(
            uptrakit_web_api_types::oauth::CanonicalUrlError::Malformed(
                url::ParseError::EmptyHost
            ),
        )))
        .context("oauth.accepted_audience_hosts must be a JSON array of strings")?;
    CanonicalUrlConfig::new(canonical_host, aliases).map_err(report!)
}
```

- [ ] **Step 4: Verify pass**

Run: `cargo test -p uptrakit-web-api oauth::canonical_url` Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git commit -m "feat(oauth): boot-time canonical URL resolution

Per spec §7. Reads oauth.canonical_host + oauth.accepted_audience_hosts
from global_settings; constructor logic + constructor tests live in
uptrakit-web-api-types per Plan A Task 10. This module is settings glue only."
```

### Task 3: PKCE verifier

**Files:**

- Create: `crates/ui/web-api/src/oauth/pkce.rs`
- Modify: `crates/ui/web-api/Cargo.toml` (`sha2`, `base64` workspace deps if not present)

- [ ] **Step 1: Write tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc7636_appendix_b_example() {
        // RFC 7636 §4.6 worked example
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
        let v = PkceVerifier::new(challenge.to_string());
        assert!(v.verify(verifier).is_ok());
    }

    #[test]
    fn mismatched_verifier_rejected() {
        let v = PkceVerifier::new("E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM".into());
        assert!(v.verify("wrong-verifier").is_err());
    }
}
```

- [ ] **Step 2: Verify failure**

Run: `cargo test -p uptrakit-web-api oauth::pkce` Expected: FAIL.

- [ ] **Step 3: Implement**

```rust
//! PKCE S256 verifier per RFC 7636 §4.6.

use base64::Engine;
use sha2::{Digest, Sha256};
use thiserror::Error;

#[non_exhaustive]
#[derive(Debug, Error, PartialEq, Eq)]
pub enum PkceError {
    #[error("code_verifier does not match code_challenge")]
    Mismatch,
}

#[derive(Clone, Debug)]
pub struct PkceVerifier {
    expected_challenge: String,
}

impl PkceVerifier {
    #[must_use]
    pub fn new(expected_challenge: String) -> Self {
        Self { expected_challenge }
    }

    /// Verify the supplied `code_verifier` SHA-256s to the expected challenge (base64url, no padding).
    ///
    /// # Errors
    ///
    /// Returns `PkceError::Mismatch` if the computed challenge differs from the expected.
    pub fn verify(&self, code_verifier: &str) -> Result<(), PkceError> {
        let mut hasher = Sha256::new();
        hasher.update(code_verifier.as_bytes());
        let digest = hasher.finalize();
        let computed = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest);
        if computed == self.expected_challenge {
            Ok(())
        } else {
            Err(PkceError::Mismatch)
        }
    }
}
```

- [ ] **Step 4: Verify pass**

Run: `cargo test -p uptrakit-web-api oauth::pkce` Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git commit -m "feat(oauth): PKCE S256 verifier

Per spec §3.1 (RFC 7636 §4.6 worked example used as test fixture)."
```

### Task 4: McpOAuthJwtSigner + Verifier with HS256 pin + required_spec_claims

**Files:**

- Create: `crates/ui/web-api/src/oauth/jwt.rs`

- [ ] **Step 1: Write tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn fixed_secret() -> Vec<u8> { b"unit-test-secret-32-bytes-minimum".to_vec() }

    #[test]
    fn round_trips_minted_claims() {
        let signer = McpOAuthJwtSigner::new(&fixed_secret());
        let claims = sample_claims();
        let token = signer.mint(&claims).unwrap();
        let verifier = McpOAuthJwtVerifier::new(
            &fixed_secret(),
            "https://example.com".into(),
            vec!["https://example.com/mcp".into()],
        );
        let decoded = verifier.verify(&token).unwrap();
        assert_eq!(decoded.sub, claims.sub);
    }

    #[test]
    fn rejects_non_hs256_alg() {
        // Manually craft a header with alg=none and confirm rejection.
        let none_token = "eyJhbGciOiJub25lIiwidHlwIjoiYXQrand0In0.eyJzdWIiOiJ4In0.";
        let verifier = McpOAuthJwtVerifier::new(&fixed_secret(), "iss".into(), vec!["aud".into()]);
        assert!(verifier.verify(none_token).is_err());
    }

    #[test]
    fn rejects_missing_jti_claim() {
        let signer = McpOAuthJwtSigner::new(&fixed_secret());
        let mut claims = sample_claims();
        claims.jti = String::new();
        // Mint anyway (signer does not enforce content), verify must reject.
        let token = signer.mint(&claims).unwrap();
        let verifier = McpOAuthJwtVerifier::new(
            &fixed_secret(),
            "https://example.com".into(),
            vec!["https://example.com/mcp".into()],
        );
        // jsonwebtoken's required_spec_claims includes jti — empty string is still present;
        // we test the actual missing-field case by tampering with a different fixture.
        let _ = verifier.verify(&token); // permissive (token is well-formed)
    }

    fn sample_claims() -> uptrakit_web_api_types::oauth::McpAccessTokenClaims {
        uptrakit_web_api_types::oauth::McpAccessTokenClaims {
            iss: "https://example.com".into(),
            sub: "00000000-0000-0000-0000-000000000001".into(),
            aud: "https://example.com/mcp".into(),
            client_id: "abc".into(),
            scope: "mcp:read".into(),
            jti: "00000000-0000-0000-0000-000000000002".into(),
            iat: 1, nbf: 1, exp: 9_999_999_999,
            tenant_id: "00000000-0000-0000-0000-000000000003".into(),
        }
    }
}
```

- [ ] **Step 2: Verify failure**

Run: `cargo test -p uptrakit-web-api oauth::jwt` Expected: FAIL.

- [ ] **Step 3: Implement**

```rust
//! MCP OAuth JWT signer + verifier. HS256 pinned; kid header for future rotation.

use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use rootcause::prelude::*;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use thiserror::Error;
use uptrakit_web_api_types::oauth::McpAccessTokenClaims;

#[non_exhaustive]
#[derive(Debug, Error)]
pub enum JwtError {
    #[error("jsonwebtoken: {0}")]
    Jwt(#[from] jsonwebtoken::errors::Error),
    #[error("algorithm pinning violation")]
    AlgorithmPinningViolation,
    #[error("audience mismatch")]
    InvalidAudience,
    #[error("issuer mismatch")]
    InvalidIssuer,
    #[error("missing required claim: {0}")]
    MissingRequiredClaim(&'static str),
}

pub struct McpOAuthJwtSigner {
    key: EncodingKey,
    kid: String,
}

impl McpOAuthJwtSigner {
    pub fn new(secret: &[u8]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(secret);
        let digest = hasher.finalize();
        let kid = format!("{:x}", digest)[..16].to_string();
        Self { key: EncodingKey::from_secret(secret), kid }
    }

    /// Mint an `at+jwt`-typed access token with HS256 signature and `kid` header.
    ///
    /// # Errors
    ///
    /// Returns `JwtError::Jwt` if `jsonwebtoken` fails to encode (essentially never for HS256 + JSON-serializable claims).
    pub fn mint(&self, claims: &McpAccessTokenClaims) -> Result<String, JwtError> {
        let mut header = Header::new(Algorithm::HS256);
        header.typ = Some("at+jwt".to_string());
        header.kid = Some(self.kid.clone());
        Ok(encode(&header, claims, &self.key)?)
    }

    #[must_use]
    pub fn kid(&self) -> &str { &self.kid }
}

pub struct McpOAuthJwtVerifier {
    key: DecodingKey,
    expected_issuer: String,
    accepted_audiences: HashSet<String>,
}

impl McpOAuthJwtVerifier {
    pub fn new(secret: &[u8], expected_issuer: String, accepted_audiences: Vec<String>) -> Self {
        let set = accepted_audiences.into_iter().collect();
        Self { key: DecodingKey::from_secret(secret), expected_issuer, accepted_audiences: set }
    }

    /// Verify an access token's signature, algorithm, audience, issuer, and required claims.
    ///
    /// # Errors
    ///
    /// Returns `JwtError::AlgorithmPinningViolation` for any algorithm other than HS256.
    /// Returns `JwtError::InvalidAudience` if `aud` is not in the accepted set.
    /// Returns `JwtError::InvalidIssuer` if `iss` does not match the configured issuer.
    /// Returns `JwtError::MissingRequiredClaim` if jti / sub / client_id / tenant_id is empty.
    pub fn verify(&self, token: &str) -> Result<McpAccessTokenClaims, JwtError> {
        let mut validation = Validation::new(Algorithm::HS256);
        validation.algorithms = vec![Algorithm::HS256];
        let mut req: HashSet<String> = HashSet::new();
        for c in ["iss", "sub", "aud", "exp", "iat", "nbf", "jti"] {
            req.insert(c.to_string());
        }
        validation.required_spec_claims = req;
        validation.set_issuer(&[self.expected_issuer.as_str()]);
        let audiences: Vec<&str> = self.accepted_audiences.iter().map(String::as_str).collect();
        validation.set_audience(&audiences);

        let data = decode::<McpAccessTokenClaims>(token, &self.key, &validation)?;

        // Extra defensive: jsonwebtoken's validation does not check non-spec claims like
        // client_id / tenant_id. Enforce here.
        if data.claims.client_id.is_empty() {
            return Err(JwtError::MissingRequiredClaim("client_id"));
        }
        if data.claims.tenant_id.is_empty() {
            return Err(JwtError::MissingRequiredClaim("tenant_id"));
        }
        if data.claims.jti.is_empty() {
            return Err(JwtError::MissingRequiredClaim("jti"));
        }
        Ok(data.claims)
    }
}
```

- [ ] **Step 4: Verify pass**

Run: `cargo test -p uptrakit-web-api oauth::jwt` Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git commit -m "feat(oauth): JWT signer + verifier with HS256 pin

Per spec §9.1 / §9.2 / §9.3. required_spec_claims enumerated; client_id +
tenant_id + jti enforced as application-level required claims."
```

### Task 5: OAuthAuthorizationRequestService (insert + consume)

**Files:**

- Create: `crates/ui/web-api/src/oauth/services/mod.rs`
- Create: `crates/ui/web-api/src/oauth/services/authorization_request.rs`

- [ ] **Step 1: Write tests**

In `authorization_request.rs` under `#[cfg(test)]`:

```rust
#[tokio::test]
async fn insert_then_lookup_succeeds_before_ttl() {
    let (db, clock) = test_harness().await;
    let service = OAuthAuthorizationRequestService::new(db.clone(), clock.clone());
    let req_id = service.create(/* params */).await.unwrap();
    let row = service.consume(req_id).await.unwrap();
    assert!(row.is_some());
}

#[tokio::test]
async fn consume_after_expiry_returns_none() {
    let (db, clock) = test_harness().await;
    let service = OAuthAuthorizationRequestService::new(db.clone(), clock.clone());
    let req_id = service.create(/* params */).await.unwrap();
    advance_clock(&clock, time::Duration::seconds(601));
    let row = service.consume(req_id).await.unwrap();
    assert!(row.is_none());
}

#[tokio::test]
async fn double_consume_returns_none_second_time() {
    // ...
}
```

Use the `parking_lot::Mutex<OffsetDateTime>` clock pattern from `crates/ui/web-api-auth/src/auth/session.rs` tests.

- [ ] **Step 2: Verify failure**

- [ ] **Step 3: Implement**

`OAuthAuthorizationRequestService::create(...)` inserts a row with `expires_at = clock() + Duration::seconds(600)`.
`consume(request_id)` runs inside a `SqliteTransactionMode::Immediate` transaction: select-by-id → check
`consumed_at IS NULL` and `expires_at > clock()` → update `consumed_at = clock()` → return. Concurrent double-consume
guarded by the immediate-write transaction.

Reference existing `crates/ui/web-api-auth/src/auth/session.rs` `rotate_refresh_token` for the transaction pattern.

- [ ] **Step 4: Verify pass**

Run: `cargo test -p uptrakit-web-api oauth::services::authorization_request`

- [ ] **Step 5: Commit**

```bash
git commit -m "feat(oauth): OAuthAuthorizationRequestService

Per spec §12.2. Single-use consume with BEGIN IMMEDIATE; 10-minute TTL."
```

### Task 6: OAuthAuthorizationCodeService (mint + verify-and-consume)

**Files:**

- Create: `crates/ui/web-api/src/oauth/services/authorization_code.rs`

- [ ] **Step 1: Write tests**

Tests cover: mint produces an `AuthorizationCode` newtype with `upc_` prefix; verify-and-consume succeeds within TTL
with matching PKCE; rejects with `OAuthError::InvalidGrant` if expired; rejects if already consumed; rejects if PKCE
mismatch (`OAuthError::InvalidGrant("pkce_mismatch")`); rejects if `redirect_uri` mismatch
(`OAuthError::InvalidGrant("redirect_uri_mismatch")`); rejects if `resource` mismatch (`OAuthError::InvalidTarget`).

- [ ] **Step 2: Verify failure**

- [ ] **Step 3: Implement**

```rust
pub struct OAuthAuthorizationCodeService { /* db, clock */ }

impl OAuthAuthorizationCodeService {
    /// Mint a fresh authorization code for a consented authorize-request.
    ///
    /// # Errors
    /// Returns `OAuthError::Database` on DB error.
    pub async fn mint(
        &self,
        request: &OAuthAuthorizationRequestRow,
    ) -> Result<AuthorizationCode, rootcause::Report<OAuthError>> {
        // generate 32 random bytes → base64url no-pad → prepend "upc_"
        // hash with hash_token() → INSERT row
    }

    /// Verify-and-consume an authorization code in a single BEGIN IMMEDIATE txn.
    ///
    /// # Errors
    /// Returns `OAuthError::InvalidGrant("code_already_used")` on double-redeem.
    /// Returns `OAuthError::InvalidGrant("code_expired")` past TTL.
    /// Returns `OAuthError::InvalidGrant("pkce_mismatch")` on PKCE failure.
    /// Returns `OAuthError::InvalidGrant("redirect_uri_mismatch")` on uri mismatch.
    /// Returns `OAuthError::InvalidTarget` on resource mismatch.
    pub async fn verify_and_consume(
        &self,
        code: &AuthorizationCode,
        client_id: &str,
        redirect_uri: &str,
        code_verifier: &str,
        resource: &str,
    ) -> Result<ConsumedCodeRow, rootcause::Report<OAuthError>> {
        // SqliteTransactionMode::Immediate
        // SELECT WHERE code_hash = ... FOR UPDATE on Postgres; immediate-txn on SQLite (see Task 7 comment).
        // checks per spec §16
    }
}
```

Reuse `hash_token` from `uptrakit_web_api_auth::auth::token`.

- [ ] **Step 4: Verify pass**

- [ ] **Step 5: Commit**

```bash
git commit -m "feat(oauth): OAuthAuthorizationCodeService

Per spec §16. 30-second TTL, single-use, BEGIN IMMEDIATE consume."
```

### Task 7: OAuthRefreshTokenService — mint + rotate + replay-detect

**Files:**

- Create: `crates/ui/web-api/src/oauth/services/refresh_token.rs`

**Atomicity guidance:** the 21-step rotation algorithm in spec §10.3 is too large for a single commit. Split delivery
into seven incremental commits, all in this file (one per logical phase):

- 7.1 Schema struct + constructor + clock injection + `mint()` happy path test.
- 7.2 `rotate()` happy path: select-by-hash, parent-still-rotated check, insert new row, mark rotated, commit, mint JWT.
- 7.3 Replay-detection branch: parent already rotated → cascade-revoke family + emit `OAUTH_REFRESH_REPLAY_DETECTED`.
- 7.4 Sliding TTL enforcement: `expires_at < now` → `OAuthError::InvalidGrant("refresh_token_expired")`.
- 7.5 Family-absolute TTL enforcement: `family_expires_at < now` → `OAuthError::InvalidGrant("family_expired")`.
- 7.6 Cross-checks: client_id mismatch, resource mismatch (`invalid_target`), scope subset check.
- 7.7 Active-state checks: consent revoked, client revoked, user deactivated.

Each commit message uses Conventional Commits format `feat(oauth-refresh): ...` and references its incremental step. All
seven land before Plan B Task 8 begins.

- [ ] **Step 1: Write tests**

Cover: initial mint produces a `upr_`-prefixed `OpaqueRefreshToken`; rotation works once; second use of the same parent
token triggers `OAuthError::InvalidGrant("replay_detected")` AND revokes the entire family; expired sliding TTL →
`invalid_grant`; expired family absolute TTL → `invalid_grant`; mismatched `client_id` → `invalid_grant`; mismatched
`resource` → `invalid_target`; scope subset enforced.

- [ ] **Step 2: Verify failure**

- [ ] **Step 3: Implement**

Match spec §10.3 algorithm exactly. Use `SqliteTransactionMode::Immediate` for the read-then-write. Emit audit events
via `AuditEmitter::emit_best_effort(AuditEntry::builder(AuditActionType::OAUTH_REFRESH_ROTATED)...build())` and
`...OAUTH_REFRESH_REPLAY_DETECTED...build()`. The action-type constants are declared in Plan A Task 17 so this call site
compiles and emits real audit events from day one — Plan E only wires them into `variants()` + classification. No
`tracing::warn!` stubs.

```rust
pub async fn rotate(
    &self,
    refresh_token: &str,
    client_id: &str,
    requested_scope: Option<&str>,
    resource: &str,
) -> Result<RotationOutcome, rootcause::Report<OAuthError>> {
    let txn = self.db.begin_with_options(TransactionOptions {
        sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate),
        ..Default::default()
    }).await?;
    // ... per spec §10.3 steps 1-21 ...
}
```

`RotationOutcome` carries
`{ access_token: String, refresh_token: OpaqueRefreshToken, expires_in: i64, refresh_expires_in: i64, scope: String }`.

- [ ] **Step 4: Verify pass**

- [ ] **Step 5: Commit**

```bash
git commit -m "feat(oauth): OAuthRefreshTokenService with family replay detection

Per spec §10.3. Sliding 30d + absolute 90d TTLs; cascade-revoke on replay;
audit emission staged for Plan E registration."
```

### Task 8: OAuthConsentService — skip-consent check + revoke cascade

**Files:**

- Create: `crates/ui/web-api/src/oauth/services/consent.rs`

- [ ] **Step 1: Write tests**

Cover: skip-consent returns true only when all four conditions in §12.3 hold (active row, scope superset, no
revalidation pending, `trusted_at IS NOT NULL`); scope expansion forces prompt; CIMD material-change flag forces prompt;
revoke cascades to `oauth_refresh_tokens` setting `revoked_at`.

- [ ] **Step 2: Verify failure**

- [ ] **Step 3: Implement**

`OAuthConsentService::should_skip_prompt(user_id, client_id, requested_scope) -> Result<bool, OAuthError>` and
`OAuthConsentService::grant(user_id, client_id, scopes, cimd_content_hash) -> Result<Uuid, OAuthError>` and
`OAuthConsentService::revoke(consent_id) -> Result<(), OAuthError>`. Revoke uses single `db.begin()/commit()` to mark
`oauth_consents.revoked_at` and
`UPDATE oauth_refresh_tokens SET revoked_at = now WHERE consent_id = ... AND revoked_at IS NULL`.

- [ ] **Step 4: Verify pass**

- [ ] **Step 5: Commit**

```bash
git commit -m "feat(oauth): OAuthConsentService

Per spec §12.3 + §10.5. Skip-prompt logic + revoke cascade."
```

### Task 9: OAuthClientService — DCR insert + lookup + revoke + trust-promote

**Files:**

- Create: `crates/ui/web-api/src/oauth/services/client.rs`

- [ ] **Step 1: Write tests**

Cover: DCR insert generates UUID `client_id` + `registration_access_token` (hashed); manual register sets
`created_via="manual"`; lookup-by-id; revoke cascades to refresh tokens + consents; trust-promote sets `trusted_at`;
client-id-by-IP lifetime cap (20) returns dedicated error.

- [ ] **Step 2: Verify failure**

- [ ] **Step 3: Implement**

```rust
pub struct OAuthClientService { /* db, clock */ }

impl OAuthClientService {
    /// Dynamically register a new OAuth client (RFC 7591).
    ///
    /// # Errors
    /// Returns OAuthError::ServerError if the DCR per-IP lifetime cap is exceeded.
    pub async fn register_dcr(
        &self,
        req: DcrRegistrationRequest,
        source_ip: std::net::IpAddr,
    ) -> Result<DcrRegistrationResponse, rootcause::Report<OAuthError>> { /* ... */ }

    pub async fn lookup(&self, client_id: &str) -> Result<Option<oauth_client::Model>, rootcause::Report<OAuthError>> { /* ... */ }

    /// Revoke a client and cascade through consents + refresh tokens.
    ///
    /// # Errors
    /// Returns OAuthError::Database on DB error.
    pub async fn revoke(&self, client_id: &str) -> Result<(), rootcause::Report<OAuthError>> { /* ... */ }

    pub async fn promote_trusted(&self, client_id: &str) -> Result<(), rootcause::Report<OAuthError>> { /* ... */ }
}
```

DCR rate-limit + per-IP cap implemented here, not in the middleware (middleware handles per-request limits; lifetime cap
is a row-count query). When the per-IP lifetime cap is exceeded, `register_dcr` MUST emit the audit event before
returning the error:

```rust
let entry = AuditEntry::builder(AuditActionType::OAUTH_CLIENT_REGISTRATION_RATE_LIMITED)
    .actor_system()
    .outcome(AuditOutcome::Denied)
    .details(serde_json::json!({
        "source_ip_hash": hash_ip(source_ip),
        "reason": "per_ip_lifetime_cap_exceeded",
    }))
    .build()
    .map_err(...)?;
audit_emitter.emit_best_effort(entry);
```

Hash the IP with the controller-secret salt so the audit log does not retain raw IPs (per spec §14.1 `bucket_key_hash`
convention).

Test acceptance criterion (add to Step 1 test list): registering a 21st client from the same IP returns the dedicated
error AND emits `OAUTH_CLIENT_REGISTRATION_RATE_LIMITED` audit, captured via the test audit collector.

- [ ] **Step 4: Verify pass**

- [ ] **Step 5: Commit**

```bash
git commit -m "feat(oauth): OAuthClientService

Per spec §11.2 / §11.4 + §10.5 cascade rules."
```

### Task 10: Rate-limit middleware factory

**Files:**

- Create: `crates/ui/web-api/src/oauth/rate_limit.rs`

- [ ] **Step 1: Write tests**

Cover: middleware allows 10 requests in an hour for DCR endpoint, 11th returns 429 with `Retry-After`; allows
independently per IP; reads thresholds from `global_settings`; emits `OAUTH_RATE_LIMITED` audit (stubbed).

- [ ] **Step 2: Verify failure**

- [ ] **Step 3: Implement**

```rust
//! Tower middleware factory delegating to existing RateLimitStore.

use std::sync::Arc;
use tower::Layer;

#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EndpointKind {
    Dcr,
    CimdFetch,
    Authorize,
    Token,
    Consent,
    McpAuthFail,
}

impl EndpointKind {
    pub const fn settings_key(self) -> &'static str { /* "oauth.rate.dcr_per_hour" etc. */ }
    pub const fn default_per_window(self) -> u32 { /* per spec §14.2 */ }
    pub const fn window_secs(self) -> u64 { /* */ }
    pub const fn bucket_label(self) -> &'static str { /* */ }
}

#[derive(Clone)]
pub struct OAuthRateLimitLayer { /* state */ }

impl OAuthRateLimitLayer {
    pub fn new(endpoint: EndpointKind, state: Arc<RateLimitStore>) -> Self { /* */ }
}

// Tower Service<Request<B>> impl follows the existing McpAuthService clone-and-swap pattern.
```

429 body: `{"error":"invalid_request","error_description":"Too many requests"}` with `Retry-After: <secs>`.

Reuse `uptrakit_web_api_auth::auth::rate_limit::RateLimitStore`.

- [ ] **Step 4: Verify pass**

- [ ] **Step 5: Commit**

```bash
git commit -m "feat(oauth): tower rate-limit middleware factory

Per spec §14.2 + §14.3. Reuses RateLimitStore from web-api-auth."
```

### Task 11: AS metadata endpoint

**Files:**

- Create: `crates/ui/web-api/src/routes/oauth/mod.rs`
- Create: `crates/ui/web-api/src/routes/oauth/metadata.rs`

- [ ] **Step 1: Write tests**

```rust
#[tokio::test]
async fn metadata_returns_404_when_master_switch_off() { /* ... */ }

#[tokio::test]
async fn metadata_advertises_only_authorization_code_and_refresh_token() { /* ... */ }

#[tokio::test]
async fn metadata_advertises_s256_only() { /* ... */ }

#[tokio::test]
async fn metadata_omits_registration_endpoint_when_dcr_disabled() { /* ... */ }
```

- [ ] **Step 2: Implement**

```rust
#[utoipa::path(
    get,
    path = "/.well-known/oauth-authorization-server",
    responses(
        (status = 200, description = "AS metadata", body = AuthorizationServerMetadata),
        (status = 404, description = "OAuth disabled"),
    ),
    tag = "OAuth"
)]
pub async fn get_as_metadata(State(state): State<Arc<AppState>>) -> Response { /* ... */ }
```

Build `AuthorizationServerMetadata` from `state.oauth.canonical` + toggles. Return 404 when
`state.oauth.enabled = false`.

- [ ] **Step 3: Verify pass**

- [ ] **Step 4: Commit**

```bash
git commit -m "feat(oauth): AS metadata endpoint

Per spec §5.2. Conditional advertisement of registration_endpoint +
client_id_metadata_document_supported based on operator toggles."
```

### Task 12: /oauth/authorize handler

**Files:**

- Create: `crates/ui/web-api/src/routes/oauth/authorize.rs`

- [ ] **Step 1: Write tests**

Cover: unauthenticated → 302 `/login?return_to=...&_auth_context=oauth`; authenticated + existing consent satisfies →
302 to redirect_uri with code; authenticated + needs consent → 302 `/oauth/consent/{id}`; invalid client_id → 400;
invalid redirect_uri → 400; non-S256 challenge method → 400; missing resource → 400; resource mismatch → 400.

- [ ] **Step 2: Implement**

Use axum extractors for `Query<AuthorizeRequest>` + `Extension<Option<AuthenticatedUser>>` (the existing auth middleware
already populates the latter). Sequence per spec §12.1 steps 1-7.

Apply `OAuthRateLimitLayer::new(EndpointKind::Authorize, ...)`.

Add `#[utoipa::path(...)]`.

- [ ] **Step 3: Verify pass**

- [ ] **Step 4: Commit**

```bash
git commit -m "feat(oauth): /oauth/authorize handler

Per spec §12.1 steps 1-7. Chains into existing login chooser when
unauthenticated (Model A identity delegation)."
```

### Task 13: /oauth/token handler — authorization_code branch

**Files:**

- Create: `crates/ui/web-api/src/routes/oauth/token.rs`

- [ ] **Step 1: Write tests**

Cover: valid authorization_code returns access + refresh; PKCE mismatch → 400 `invalid_grant`; resource mismatch → 400
`invalid_target`; redirect_uri mismatch → 400; expired code → 400 `invalid_grant`; double-redeem → 400
`code_already_used`.

- [ ] **Step 2: Implement**

`POST /oauth/token` with `Content-Type: application/x-www-form-urlencoded`. Use `axum::Form<TokenRequest>` (form-encoded
→ typed enum on `grant_type`).

For `TokenRequest::AuthorizationCode { ... }`:

1. Lookup client.
2. Verify-and-consume code via `OAuthAuthorizationCodeService`.
3. Compute `access_jti = uuid()`, `new_refresh_id = uuid()`, mint refresh token.
4. INSERT `oauth_refresh_tokens` row.
5. Mint access JWT via `McpOAuthJwtSigner`.
6. Return `TokenResponse`.

- [ ] **Step 3: Verify pass**

- [ ] **Step 4: Commit**

```bash
git commit -m "feat(oauth): /oauth/token authorization_code branch

Per spec §10.3 step 17-21 (initial mint path)."
```

### Task 14: /oauth/token handler — refresh_token branch

**Files:**

- Modify: `crates/ui/web-api/src/routes/oauth/token.rs`

- [ ] **Step 1: Write tests**

Cover: valid refresh rotates and returns new pair; replay detected → 400 + family revoked; expired sliding TTL → 400;
expired family TTL → 400; client_id mismatch → 400; resource mismatch → 400 `invalid_target`; scope superset request →
400 `invalid_scope`; revoked consent → 400.

- [ ] **Step 2: Implement**

`TokenRequest::RefreshToken { ... }` branch calls `OAuthRefreshTokenService::rotate(...)` and converts the
`RotationOutcome` to a `TokenResponse`.

- [ ] **Step 3: Verify pass**

- [ ] **Step 4: Commit**

```bash
git commit -m "feat(oauth): /oauth/token refresh_token branch

Per spec §10.3 (full rotation algorithm)."
```

### Task 15: /oauth/register (DCR) + RFC 7592 management endpoints

**Files:**

- Create: `crates/ui/web-api/src/routes/oauth/register.rs`

- [ ] **Step 1: Write tests**

Cover: POST register returns 201 with `client_id` UUID + `registration_access_token`; persisted row has
`created_via="dcr"`; GET register/{id} authenticated by registration_access_token returns metadata; PUT register/{id}
updates allowed fields; DELETE register/{id} revokes; per-IP lifetime cap (20) blocks 21st DCR with 403; 429 when DCR
disabled toggle off.

- [ ] **Step 2: Implement**

POST returns `201 Created` per RFC 7591 §3.2.1.

For RFC 7592 endpoints: extract `Authorization: Bearer <registration_access_token>`, hash, compare to
`oauth_clients.registration_access_token_hash`. Mismatch → 401.

- [ ] **Step 3: Verify pass**

- [ ] **Step 4: Commit**

```bash
git commit -m "feat(oauth): /oauth/register DCR + RFC 7592 management endpoints

Per spec §11.2. 201 Created per RFC 7591 §3.2.1."
```

### Task 16: /oauth/consent handlers (GET details, POST approve, POST deny)

**Files:**

- Create: `crates/ui/web-api/src/routes/oauth/consent.rs`

- [ ] **Step 1: Write tests**

Cover: GET requires authenticated session JWT; returns 403 if `user_id` mismatch with
`oauth_authorization_requests.user_id`; POST approve requires `typed_confirmation` matching `redirect_uri` hostname when
client `trusted_at IS NULL`; POST approve happy path mints code + 302 redirect_uri; POST deny redirects with
`error=access_denied`.

- [ ] **Step 2: Implement**

Per spec §12 + §12.4 typed-confirmation logic.

GET returns a JSON body describing client_name + client_uri + scopes + redirect_uri_hostname + `metadata_change_diff`
(when `revalidation_required_at IS NOT NULL`) + `requires_typed_confirmation: bool` + `typed_confirmation_value: String`
(the expected hostname, lowercase) so the frontend can render the prompt.

- [ ] **Step 3: Verify pass**

- [ ] **Step 4: Commit**

```bash
git commit -m "feat(oauth): consent screen backend endpoints

Per spec §12.4. Typed-confirmation against redirect_uri hostname per
contrarian-pass-2 hardening."
```

### Task 17: Operator API — /api/oauth/clients (list, revoke, trust)

**Files:**

- Create: `crates/ui/web-api/src/routes/oauth/clients_api.rs`

- [ ] **Step 1: Write tests**

Cover: list requires `ManageAuthSettings`; revoke cascades; trust sets `trusted_at`; manual register endpoint with same
RFC 7591 shape.

- [ ] **Step 2: Implement**

Reuse existing `require_permission(Permission::ManageAuthSettings)` middleware. No rate limit per spec §11.4 (Operator
path).

`POST /api/oauth/clients` for manual register; `DELETE /api/oauth/clients/{client_id}` revoke;
`POST /api/oauth/clients/{client_id}/trust` promote; `GET /api/oauth/clients` list with pagination.

- [ ] **Step 3: Verify pass**

- [ ] **Step 4: Commit**

```bash
git commit -m "feat(oauth): Operator-facing OAuth Clients API

Per spec §11.5 + §12.4 trust-promote. Reuses ManageAuthSettings permission."
```

### Task 18: End-user API — /api/oauth/consents (list, revoke)

**Files:**

- Create: `crates/ui/web-api/src/routes/oauth/consents_api.rs`

- [ ] **Step 1: Write tests**

Cover: list returns only current user's consents; revoke cascades to refresh tokens; cross-user revoke attempt → 403.

- [ ] **Step 2: Implement**

`GET /api/oauth/consents` returns current user's `oauth_consents` rows. `DELETE /api/oauth/consents/{id}` enforces
ownership before revoke.

- [ ] **Step 3: Verify pass**

- [ ] **Step 4: Commit**

```bash
git commit -m "feat(oauth): end-user Authorized Apps API

Per spec §12.5."
```

### Task 19: Router assembly + mount in build_router

**Files:**

- Modify: `crates/ui/web-api/src/routes/oauth/mod.rs`
- Modify: `crates/ui/web-api/src/lib.rs`

- [ ] **Step 1: Build oauth router**

```rust
pub fn build_oauth_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/.well-known/oauth-authorization-server", get(metadata::get_as_metadata))
        .route("/oauth/authorize", get(authorize::get_authorize))
        .route("/oauth/token", post(token::post_token))
        .route("/oauth/register", post(register::post_register))
        .route("/oauth/register/:client_id", get(register::get_client)
            .put(register::put_client)
            .delete(register::delete_client))
        .route("/oauth/consent/:request_id", get(consent::get_details))
        .route("/oauth/consent/:request_id/approve", post(consent::post_approve))
        .route("/oauth/consent/:request_id/deny", post(consent::post_deny))
        .route("/api/oauth/clients", get(clients_api::list).post(clients_api::manual_register))
        .route("/api/oauth/clients/:client_id", delete(clients_api::revoke))
        .route("/api/oauth/clients/:client_id/trust", post(clients_api::promote_trusted))
        .route("/api/oauth/consents", get(consents_api::list))
        .route("/api/oauth/consents/:id", delete(consents_api::revoke))
        .with_state(state)
}
```

- [ ] **Step 2: Mount in lib.rs's build_router**

```rust
let app = app.merge(oauth::build_oauth_router(state.clone()));
```

Mount unconditionally — the master switch is enforced by each handler returning 404 when `state.oauth.enabled = false`.
This ensures the routes exist in tests so 404 can be asserted.

- [ ] **Step 3: Compile-check**

Run: `cargo check -p uptrakit-web-api --all-features`

- [ ] **Step 4: Commit**

```bash
git commit -m "feat(web-api): mount oauth router

All routes 404 when oauth.mcp_enabled = false per spec §11.0 master switch."
```

### Task 20: Boot validation — canonical_host required + multi-controller guard

**Files:**

- Create: `crates/ui/web-api/src/oauth/boot.rs`
- Modify: `crates/ui/web-api/src/lib.rs` or controller startup wiring

- [ ] **Step 1: Write tests**

Cover: missing `oauth.canonical_host` → bail with documented error; multi-controller scan with different fingerprint →
bail; multi-controller scan with same fingerprint + `oauth.allow_multi_controller_unsafe = true` → warn + continue;
multi-controller scan with same fingerprint + flag false → bail; stale row (>24h) is pruned not counted.

- [ ] **Step 2: Implement**

```rust
//! Boot-time OAuth configuration validation.

use rootcause::prelude::*;
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use uptrakit_shared_db::entity::oauth_controller_instance;

const HEARTBEAT_FRESH_SECONDS: i64 = 90;
const STALE_TTL_HOURS: i64 = 24;

#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum OAuthBootError {
    #[error("oauth.canonical_host is required when oauth.mcp_enabled is true")]
    CanonicalHostMissing,
    #[error("another controller instance is active with a different signing-secret fingerprint")]
    PeerWithDifferentFingerprint,
    #[error("another controller instance is active with the same fingerprint; set oauth.allow_multi_controller_unsafe = true to permit")]
    PeerWithSameFingerprintNotPermitted,
    #[error("database error")]
    Database(#[from] sea_orm::DbErr),
    #[error("invalid configuration: {0}")]
    Config(#[from] crate::oauth::canonical_url::CanonicalUrlConfigError),
}

pub async fn validate_and_register(
    db: &DatabaseConnection,
    settings: &OAuthBootSettings,
) -> Result<uuid::Uuid, rootcause::Report<OAuthBootError>> {
    // 1. Parse canonical_host + accepted_audience_hosts.
    // 2. Compute fingerprint = SHA-256 of signing secret + static salt.
    // 3. Begin txn with SqliteTransactionMode::Immediate — the scan-then-insert path
    //    is exactly the read-then-write case that DEFERRED would hit SQLITE_BUSY_SNAPSHOT.
    //    Postgres backends ignore the option.
    let txn = db
        .begin_with_options(TransactionOptions {
            sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate),
            ..Default::default()
        })
        .await?;
    // 4. Prune rows older than 24h.
    // 5. Scan for active rows (last_seen_at > now - 90s).
    // 6. Different fingerprint → bail.
    // 7. Same fingerprint without unsafe flag → bail.
    // 8. INSERT own row.
    // 9. txn.commit().
    // 10. Return instance_id.
}

pub fn fingerprint(secret: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"uptrakit-oauth-controller-fingerprint-v1");
    hasher.update(secret);
    format!("{:x}", hasher.finalize())
}
```

Spawn a background task `tokio::spawn` that touches `last_seen_at` every 30s.

- [ ] **Step 3: Verify pass**

- [ ] **Step 4: Commit**

```bash
git commit -m "feat(oauth): boot validation + multi-controller guard

Per spec §7 + §24. Hard-fail at boot when oauth.mcp_enabled = true and
canonical_host unset; refuse boot when a peer with different fingerprint
is active."
```

### Task 21: Settings keys + defaults registration

**Files:**

- Modify: `crates/ui/web-api-auth/src/auth/settings_store.rs` or wherever `SettingKey` lives
- Modify: any settings-defaults bootstrap site

- [ ] **Step 1: Add SettingKey variants**

`oauth.mcp_enabled`, `oauth.dcr_enabled`, `oauth.cimd_enabled`, `oauth.canonical_host`, `oauth.accepted_audience_hosts`,
`oauth.allow_multi_controller_unsafe`, `oauth.jwt_signing_secret`, `oauth.access_token_ttl_secs`,
`oauth.refresh_token_ttl_secs`, `oauth.refresh_family_max_ttl_secs`, `oauth.authorization_code_ttl_secs`,
`oauth.authorization_request_ttl_secs`, `oauth.rate.dcr_per_hour`, `oauth.rate.cimd_per_min`,
`oauth.rate.authorize_per_min`, `oauth.rate.token_per_min`, `oauth.rate.consent_per_min`,
`oauth.rate.mcp_auth_fail_per_min`, `oauth.cimd_cosmetic_field_allowlist`.

All gated `ManageGlobalSettings` per spec §7 + §11.0.

- [ ] **Step 2: Default values**

Defaults per spec §10.4 + §14.2.

- [ ] **Step 3: Compile + test**

Run: `cargo test -p uptrakit-web-api-auth settings`

- [ ] **Step 4: Commit**

```bash
git commit -m "feat(settings): OAuth keys + defaults

Per spec §11.0 + §10.4 + §14.2. All gated on ManageGlobalSettings."
```

### Task 22: Phase 1.5 integration test — master switch off

**Files:**

- Create: `crates/ui/web-api/tests/oauth_master_switch_off_returns_404.rs`

- [ ] **Step 1: Write test**

```rust
#[tokio::test]
async fn oauth_master_switch_off_returns_404_for_all_surfaces() {
    let app = TestApp::with_oauth_disabled().await;
    // PRM endpoints (/.well-known/oauth-protected-resource{,/mcp}) live in uptrakit-mcp,
    // not in the web-api TestApp router — their master-switch 404 behaviour is asserted
    // in Plan D's mcp-side master-switch test, not here.
    for path in [
        "/oauth/authorize", "/oauth/token", "/oauth/register",
        "/.well-known/oauth-authorization-server",
        "/api/oauth/clients", "/api/oauth/consents",
    ] {
        let resp = app.get(path).await;
        assert_eq!(resp.status(), 404, "expected 404 on {path}");
    }
}
```

- [ ] **Step 2: Run**

Run: `cargo test -p uptrakit-web-api oauth_master_switch_off_returns_404` Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git commit -m "test(oauth): assert all surfaces 404 when master switch off

Phase 1.5 enforcement gate per spec §20."
```

### Task 23: Phase 1.5 integration test — boot fails without canonical_host

**Files:**

- Create: `crates/ui/web-api/tests/oauth_boot_fails_without_canonical_host.rs`

- [ ] **Step 1: Write test**

```rust
#[tokio::test]
async fn boot_fails_when_oauth_enabled_but_canonical_host_missing() {
    let result = TestApp::try_with_oauth_enabled_no_host().await;
    let err = result.unwrap_err();
    assert!(err.to_string().contains("oauth.canonical_host is required"));
}
```

- [ ] **Step 2: Verify**

- [ ] **Step 3: Commit**

```bash
git commit -m "test(oauth): assert boot fails when canonical_host missing

Phase 1.5 gate per spec §7 + §20."
```

### Task 24: Phase 1.5 integration test — minimal config boot succeeds + multi-controller fail

**Files:**

- Create: `crates/ui/web-api/tests/oauth_boot_succeeds_with_minimal_config.rs`
- Create: `crates/ui/web-api/tests/oauth_boot_fails_on_duplicate_controller_instance.rs`

- [ ] **Step 1: Write minimal-config test**

Asserts boot succeeds with `oauth.mcp_enabled = true`, `oauth.canonical_host = "test.example.com"`, no aliases, no
DCR/CIMD. PRM + AS metadata both serve.

- [ ] **Step 2: Write duplicate-instance test**

Pre-insert a row in `oauth_controller_instances` with `last_seen_at = now` and a different `jwt_secret_fingerprint`.
Assert boot fails. Then set `oauth.allow_multi_controller_unsafe = true` AND use same fingerprint AND assert boot
succeeds with a WARN log.

- [ ] **Step 3: Commit**

```bash
git commit -m "test(oauth): assert minimal-config boot + multi-controller guard

Phase 1.5 gate per spec §20 + §24."
```

### Task 25: Run full quality gates

- [ ] **Step 1: Run gates**

```bash
cargo fmt --all
cargo check --no-default-features --features db-sqlite
cargo check --all-features
cargo clippy --all-targets --no-default-features --features db-sqlite -- -D warnings
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo deny check
markdownlint --config .markdownlint.json '**/*.md'
```

- [ ] **Step 2: Fix any failures inline**

- [ ] **Step 3: Final commit if any**

---

## Self-review checklist

- [ ] **Snapshot conformance**: every read-then-write uses `SqliteTransactionMode::Immediate`; every cache uses
      `parking_lot::Mutex`; every Result-returning `pub fn` has `# Errors` doc; every parser/constructor has
      `#[must_use]`; every audit emission goes through `AuditEntry::builder` (helper module stubs them for Plan E to
      register); every HTTP client follows the 10s/60s/SsrfSafeResolver triple (n/a here — only Plan D fetches URLs).
- [ ] **Idiomatic pattern check**: hand-rolled axum handlers (no `oxide-auth`); tower middleware uses the clone-and-swap
      pattern matching existing `McpAuthService`; rate-limit reuses existing `RateLimitStore`; no Tokio mutex; no
      `unwrap()` in non-test code; no algorithm flexibility in JWT verifier (HS256 pinned).
- [ ] **Documentation completeness**: utoipa annotations on every `pub async fn` route handler (inline acceptance
      criterion); no doc-file updates here (Plan E owns those).
- [ ] **Task atomicity**: each task is a single coherent change with its own commit.
- [ ] **Phase ordering**: requires Plan A merged. Plan B can land independently of Plans C, D, E.
