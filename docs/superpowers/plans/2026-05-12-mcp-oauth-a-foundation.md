# MCP OAuth — Plan A: Foundation (Phase 0)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land the database migrations, sea-orm entities, and shared OAuth wire types that every other plan in this
series depends on. No behavior change in production — the OAuth master switch ships disabled.

**Architecture:** Add six sea-orm migrations under `crates/shared/db/src/migration/`, generate matching entities under
`crates/shared/db/src/entity/`, and add a new `oauth` module to `uptrakit-web-api-types` carrying every typed wire enum,
newtype, request/response struct, and error type the AS and RS will consume. All public types are `#[non_exhaustive]`;
every wire-facing enum follows the project's `Other(String)` catch-all pattern.

**Tech Stack:** Rust 2024 + sea-orm migrations + `uuid` + `time::OffsetDateTime` + `serde` + `url` crate +
`uptrakit-shared-macros::impl_report_conversion!` + `rootcause::Report`.

**Spec:** `docs/superpowers/specs/2026-05-12-mcp-oauth-authorization-design.md` (commit `b7ee4a852`).

**Status:** Draft → Ready for review.

---

## Prerequisites

None. This plan is the prerequisite for Plans B, C, D, E.

## Snapshot binding

Tasks in this plan exercise the following Binding Rules from `.superpowers/standards-snapshot.md`:

- "#[non_exhaustive] on all extensible public enums and structs" — every public OAuth type
- "Wire-safe enums must have Other(String) catch-all; use `From<String>` + infallible Deserialize" — McpScope,
  OAuthGrantType, ResponseType, TokenEndpointAuthMethod, CodeChallengeMethod
- "Typed enums for internal write-path discriminators (ActorType, BatchType)" — EndpointKind (no `Other(String)`,
  `Copy`)
- "prefer typed enums or newtypes over raw String mode flags" — CanonicalResourceUrl, OpaqueRefreshToken,
  AuthorizationCode newtypes
- "prefer typed request/response/config structs over serde_json::Value" — every Request/Response struct
- "preserve error context with rootcause::Report, report!, bail!, .context_to()" — OAuthError
- "use rootcause::prelude and thiserror; ReportConversion via impl_report_conversion!" — error glue at boundaries
- "All HTTP request types in uptrakit-web-api-types implement Validate" — every \*Request struct
- "#[must_use] on pure constructors, parsers, validation predicates" — CanonicalResourceUrl::parse, ScopeSet::from_str,
  PkceVerifier::new
- "public functions returning Result must include # Errors section" — every fallible parser
- "derive Copy for small C-like enums" — OAuthGrantType, ResponseType, CodeChallengeMethod, TokenEndpointAuthMethod (no
  `Other(String)`)
- Conventional Commits: `feat(db)`, `feat(web-api-types)`, `test(...)` scopes; small granular commits

---

## File Structure

**New migrations** (under `crates/shared/db/src/migration/`):

- `m20260512_000001_oauth_clients.rs`
- `m20260512_000002_oauth_consents.rs`
- `m20260512_000003_oauth_authorization_requests.rs`
- `m20260512_000004_oauth_authorization_codes.rs`
- `m20260512_000005_oauth_refresh_tokens.rs`
- `m20260512_000006_oauth_controller_instances.rs`
- Update `crates/shared/db/src/migration/mod.rs` to list them in order.

**New entities** (under `crates/shared/db/src/entity/`):

- `oauth_client.rs`
- `oauth_consent.rs`
- `oauth_authorization_request.rs`
- `oauth_authorization_code.rs`
- `oauth_refresh_token.rs`
- `oauth_controller_instance.rs`
- Update `crates/shared/db/src/entity/mod.rs` (`pub mod` + `prelude` re-export).

**New shared types module** (under `crates/shared/web-api-types/src/`):

- `oauth/mod.rs` — module root, public re-exports.
- `oauth/scope.rs` — `McpScope` enum.
- `oauth/grant_type.rs` — `OAuthGrantType`, `ResponseType`, `CodeChallengeMethod`, `TokenEndpointAuthMethod`.
- `oauth/canonical_url.rs` — `CanonicalResourceUrl` newtype + `CanonicalUrlError`.
- `oauth/tokens.rs` — `OpaqueRefreshToken`, `AuthorizationCode` newtypes; `McpAccessTokenClaims`.
- `oauth/error.rs` — `OAuthError` enum with RFC 6749 variants.
- `oauth/requests.rs` — `AuthorizeRequest`, `TokenRequest`, `DcrRegistrationRequest`, `ConsentDecision`.
- `oauth/responses.rs` — `DcrRegistrationResponse`, `TokenResponse`, `ProtectedResourceMetadata`,
  `AuthorizationServerMetadata`.
- `oauth/metadata.rs` — DCR / CIMD shared metadata structs.
- Update `crates/shared/web-api-types/src/lib.rs` (`pub mod oauth`).

**New shared types module** (under `crates/ui/mcp/src/context.rs` — additive edit):

- Extend `McpRequestContext` with `auth_method: McpAuthMethod` field.
- Add `McpAuthMethod` enum (`ApiToken`, `OAuth { client_id, jti, scopes }`).

## FK ON DELETE convention (applies to every migration in this plan)

The codebase convention is explicit `.on_delete(ForeignKeyAction::...)` on every FK (see
`crates/shared/db/src/migration/m20260209_000001_initial.rs`). Per-relation choice:

| Table                          | Column       | References                                | ON DELETE  | Rationale                                                             |
| ------------------------------ | ------------ | ----------------------------------------- | ---------- | --------------------------------------------------------------------- |
| `oauth_consents`               | `user_id`    | `users.id`                                | `Cascade`  | Account deletion revokes all grants                                   |
| `oauth_consents`               | `client_id`  | `oauth_clients.id`                        | `Restrict` | Preserve audit trail; explicit revoke before drop                     |
| `oauth_authorization_requests` | `user_id`    | `users.id`                                | `Cascade`  | Drop in-flight requests when user deleted                             |
| `oauth_authorization_requests` | `client_id`  | `oauth_clients.id`                        | `Cascade`  | In-flight is ephemeral, no audit value                                |
| `oauth_authorization_codes`    | `request_id` | `oauth_authorization_requests.request_id` | `Cascade`  | Mirror parent                                                         |
| `oauth_authorization_codes`    | `client_id`  | `oauth_clients.id`                        | `Cascade`  | Ephemeral; mirror parent                                              |
| `oauth_authorization_codes`    | `user_id`    | `users.id`                                | `Cascade`  | Drop in-flight on user delete                                         |
| `oauth_refresh_tokens`         | `user_id`    | `users.id`                                | `Cascade`  | Drop all sessions when user deleted                                   |
| `oauth_refresh_tokens`         | `client_id`  | `oauth_clients.id`                        | `Restrict` | Preserve audit chain; client revoke flows go through application code |
| `oauth_refresh_tokens`         | `consent_id` | `oauth_consents.id`                       | `Cascade`  | Consent owns refresh-token lifecycle                                  |

Every migration task below uses these actions exactly. SeaORM's default (no `.on_delete()`) is `NoAction`, which is
silently wrong for almost every row here. Specify the action explicitly per row.

---

## Tasks

### Task 1: oauth_clients migration

**Files:**

- Create: `crates/shared/db/src/migration/m20260512_000001_oauth_clients.rs`
- Modify: `crates/shared/db/src/migration/mod.rs` (register migration in `Migrator::migrations()`)

- [ ] **Step 1: Write the migration**

Reference an existing migration for shape (e.g., `m20260309_000003_host_tags.rs`).

Create `crates/shared/db/src/migration/m20260512_000001_oauth_clients.rs` with `up()` that builds the table per spec
§11.1, including columns: `id TEXT PK`, `client_name TEXT NOT NULL`, `client_uri TEXT NULL`, `logo_uri TEXT NULL`,
`redirect_uris TEXT NOT NULL` (JSON), `default_scope TEXT NOT NULL`, `grant_types TEXT NOT NULL` (JSON),
`response_types TEXT NOT NULL` (JSON), `token_endpoint_auth_method TEXT NOT NULL`, `client_secret_hash TEXT NULL`,
`registration_access_token_hash TEXT NULL`, `created_via TEXT NOT NULL`, `created_at TIMESTAMP NOT NULL`,
`last_used_at TIMESTAMP NULL`, `revoked_at TIMESTAMP NULL`, `metadata_cached_at TIMESTAMP NULL`,
`metadata_etag TEXT NULL`, `metadata_content_hash TEXT NULL`, `metadata_raw TEXT NULL`,
`metadata_parse_error TEXT NULL`, `metadata_parse_error_at TIMESTAMP NULL`, `trusted_at TIMESTAMP NULL`. Add partial
index on `revoked_at IS NULL`. Implement `down()` to drop the table.

- [ ] **Step 2: Register migration**

In `crates/shared/db/src/migration/mod.rs`, add `mod m20260512_000001_oauth_clients;` and append
`Box::new(m20260512_000001_oauth_clients::Migration)` to the `migrations()` vector in chronological order.

- [ ] **Step 3: Test the migration round-trips**

Run: `cargo test -p uptrakit-shared-db migration::m20260512_000001` Expected: PASS — table created, dropped, recreated
cleanly on `up/down/up`.

- [ ] **Step 4: Commit**

```bash
git add crates/shared/db/src/migration/m20260512_000001_oauth_clients.rs crates/shared/db/src/migration/mod.rs
git commit -m "feat(db): add oauth_clients table

Per docs/superpowers/specs/2026-05-12-mcp-oauth-authorization-design.md §11.1.
Master switch oauth.mcp_enabled defaults to false; this table is dormant
until later phases land."
```

### Task 2: oauth_consents migration

**Files:**

- Create: `crates/shared/db/src/migration/m20260512_000002_oauth_consents.rs`
- Modify: `crates/shared/db/src/migration/mod.rs`

- [ ] **Step 1: Write the migration**

Columns per spec §12.3: `id UUID PK`, `user_id UUID NOT NULL` FK `users`, `client_id TEXT NOT NULL` FK `oauth_clients`,
`scopes TEXT NOT NULL` (JSON), `cimd_content_hash_at_grant TEXT NULL`, `revalidation_required_at TIMESTAMP NULL`,
`granted_at TIMESTAMP NOT NULL`, `revoked_at TIMESTAMP NULL`. Add partial unique index
`(user_id, client_id) WHERE revoked_at IS NULL`.

- [ ] **Step 2: Register migration in mod.rs**

- [ ] **Step 3: Test round-trip**

Run: `cargo test -p uptrakit-shared-db migration::m20260512_000002` Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/shared/db/src/migration/m20260512_000002_oauth_consents.rs crates/shared/db/src/migration/mod.rs
git commit -m "feat(db): add oauth_consents table

Per spec §12.3. Includes cimd_content_hash_at_grant for material-change
re-consent detection per §11.3."
```

### Task 3: oauth_authorization_requests migration

**Files:**

- Create: `crates/shared/db/src/migration/m20260512_000003_oauth_authorization_requests.rs`
- Modify: `crates/shared/db/src/migration/mod.rs`

- [ ] **Step 1: Write the migration**

Columns per spec §12.2: `request_id UUID PK`, `client_id TEXT NOT NULL` FK `oauth_clients`, `user_id UUID NOT NULL` FK
`users`, `redirect_uri TEXT NOT NULL`, `scope TEXT NOT NULL`, `state TEXT NOT NULL`, `code_challenge TEXT NOT NULL`,
`code_challenge_method TEXT NOT NULL`, `resource TEXT NOT NULL`, `created_at TIMESTAMP NOT NULL`,
`expires_at TIMESTAMP NOT NULL`, `consumed_at TIMESTAMP NULL`. Partial index on `consumed_at IS NULL`.

- [ ] **Step 2: Register migration**
- [ ] **Step 3: Test round-trip**
- [ ] **Step 4: Commit**

```bash
git commit -m "feat(db): add oauth_authorization_requests table

Per spec §12.2. Server-side in-flight state for the consent flow."
```

### Task 4: oauth_authorization_codes migration

**Files:**

- Create: `crates/shared/db/src/migration/m20260512_000004_oauth_authorization_codes.rs`
- Modify: `crates/shared/db/src/migration/mod.rs`

- [ ] **Step 1: Write the migration**

Columns per spec §16: `id UUID PK`, `code_hash TEXT NOT NULL UNIQUE`, `request_id UUID NOT NULL` FK
`oauth_authorization_requests`, `client_id TEXT NOT NULL` FK `oauth_clients`, `user_id UUID NOT NULL` FK `users`,
`redirect_uri TEXT NOT NULL`, `scope TEXT NOT NULL`, `code_challenge TEXT NOT NULL`,
`code_challenge_method TEXT NOT NULL`, `resource TEXT NOT NULL`, `issued_at TIMESTAMP NOT NULL`,
`expires_at TIMESTAMP NOT NULL`, `consumed_at TIMESTAMP NULL`. Partial index on `code_hash WHERE consumed_at IS NULL`.

- [ ] **Step 2: Register migration**
- [ ] **Step 3: Test round-trip**
- [ ] **Step 4: Commit**

```bash
git commit -m "feat(db): add oauth_authorization_codes table

Per spec §16. 30-second TTL single-use authorization codes."
```

### Task 5: oauth_refresh_tokens migration

**Files:**

- Create: `crates/shared/db/src/migration/m20260512_000005_oauth_refresh_tokens.rs`
- Modify: `crates/shared/db/src/migration/mod.rs`

- [ ] **Step 1: Write the migration**

Columns per spec §10.1: `id UUID PK`, `family_id UUID NOT NULL`, `parent_id UUID NULL`,
`token_hash TEXT NOT NULL UNIQUE`, `client_id TEXT NOT NULL` FK `oauth_clients`, `user_id UUID NOT NULL` FK `users`,
`consent_id UUID NOT NULL` FK `oauth_consents`, `scope TEXT NOT NULL`, `resource TEXT NOT NULL`,
`issued_at TIMESTAMP NOT NULL`, `expires_at TIMESTAMP NOT NULL`, `family_expires_at TIMESTAMP NOT NULL`,
`rotated_at TIMESTAMP NULL`, `revoked_at TIMESTAMP NULL`. Indexes: `(token_hash)`, `(family_id, rotated_at)`, partial
`(user_id, client_id) WHERE revoked_at IS NULL`, `(consent_id)`.

- [ ] **Step 2: Register migration**
- [ ] **Step 3: Test round-trip**
- [ ] **Step 4: Commit**

```bash
git commit -m "feat(db): add oauth_refresh_tokens table

Per spec §10.1. Schema supports family-replay detection and sliding+absolute TTLs."
```

### Task 6: oauth_controller_instances migration

**Files:**

- Create: `crates/shared/db/src/migration/m20260512_000006_oauth_controller_instances.rs`
- Modify: `crates/shared/db/src/migration/mod.rs`

- [ ] **Step 1: Write the migration**

Per spec §24: `instance_id UUID PK`, `jwt_secret_fingerprint TEXT NOT NULL`, `started_at TIMESTAMP NOT NULL`,
`last_seen_at TIMESTAMP NOT NULL`. Index on `last_seen_at`.

- [ ] **Step 2: Register migration**
- [ ] **Step 3: Test round-trip**
- [ ] **Step 4: Commit**

```bash
git commit -m "feat(db): add oauth_controller_instances table

Per spec §24. Enables multi-controller boot guard."
```

### Task 7: Generate sea-orm entities for all six tables

**Files:**

- Create: `crates/shared/db/src/entity/oauth_client.rs`
- Create: `crates/shared/db/src/entity/oauth_consent.rs`
- Create: `crates/shared/db/src/entity/oauth_authorization_request.rs`
- Create: `crates/shared/db/src/entity/oauth_authorization_code.rs`
- Create: `crates/shared/db/src/entity/oauth_refresh_token.rs`
- Create: `crates/shared/db/src/entity/oauth_controller_instance.rs`
- Modify: `crates/shared/db/src/entity/mod.rs`

- [ ] **Step 1: Write entity files**

For each entity file follow the existing pattern in `crates/shared/db/src/entity/oidc_provider.rs`:

- `#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]`
- `Model` struct with column types matching the migration (TIMESTAMP → `OffsetDateTime`, JSON columns →
  `serde_json::Value` or typed struct via `column_type = "Json"`, TEXT → `String`).
- Empty `Relation` enum or FK relations as needed.
- `ActiveModelBehavior` impl.

For `oauth_client`: `id` is `String` (not `Uuid` — CIMD client_ids are URLs).

- [ ] **Step 2: Register entities in mod.rs**

```rust
pub mod oauth_authorization_code;
pub mod oauth_authorization_request;
pub mod oauth_client;
pub mod oauth_consent;
pub mod oauth_controller_instance;
pub mod oauth_refresh_token;

pub mod prelude {
    // ... existing re-exports ...
    pub use super::oauth_authorization_code::Entity as OauthAuthorizationCode;
    pub use super::oauth_authorization_request::Entity as OauthAuthorizationRequest;
    pub use super::oauth_client::Entity as OauthClient;
    pub use super::oauth_consent::Entity as OauthConsent;
    pub use super::oauth_controller_instance::Entity as OauthControllerInstance;
    pub use super::oauth_refresh_token::Entity as OauthRefreshToken;
}
```

- [ ] **Step 3: Compile-check**

Run: `cargo check -p uptrakit-shared-db --all-features` Expected: clean compile.

- [ ] **Step 4: Commit**

```bash
git commit -m "feat(db): add sea-orm entities for OAuth tables

Six entities matching m20260512_000001..000006 migrations."
```

### Task 8: McpScope enum in web-api-types

**Files:**

- Create: `crates/shared/web-api-types/src/oauth/mod.rs`
- Create: `crates/shared/web-api-types/src/oauth/scope.rs`
- Modify: `crates/shared/web-api-types/src/lib.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/shared/web-api-types/src/oauth/scope.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_variants_round_trip_via_as_str() {
        for v in McpScope::KNOWN_VARIANTS {
            let s = v.as_str();
            let parsed = McpScope::from(s.to_string());
            assert_eq!(*v, parsed);
        }
    }

    #[test]
    fn unknown_scope_round_trips_via_other() {
        let s = "mcp:custom:future";
        let scope = McpScope::from(s.to_string());
        assert_eq!(scope, McpScope::Other(s.to_string()));
        assert_eq!(scope.as_str(), s);
    }

    #[test]
    fn deserialize_infallible_for_unknown_string() {
        let json = r#""mcp:future_scope""#;
        let scope: McpScope = serde_json::from_str(json).unwrap();
        assert!(matches!(scope, McpScope::Other(_)));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p uptrakit-web-api-types oauth::scope` Expected: FAIL (type does not exist).

- [ ] **Step 3: Implement McpScope**

Mirror `crates/shared/wire/src/lib.rs` `EnrollmentStatus` pattern. Write
`crates/shared/web-api-types/src/oauth/scope.rs`:

```rust
//! OAuth scope enum for MCP. Wire-safe per crates/shared/wire/src/lib.rs convention.

use std::fmt;
use std::str::FromStr;

#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum McpScope {
    Read,
    Write,
    Other(String),
}

impl McpScope {
    pub const KNOWN_VARIANTS: &'static [McpScope] = &[McpScope::Read, McpScope::Write];

    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            McpScope::Read => "mcp:read",
            McpScope::Write => "mcp:write",
            McpScope::Other(s) => s.as_str(),
        }
    }
}

impl fmt::Display for McpScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<String> for McpScope {
    fn from(s: String) -> Self {
        match s.as_str() {
            "mcp:read" => McpScope::Read,
            "mcp:write" => McpScope::Write,
            _ => McpScope::Other(s),
        }
    }
}

impl FromStr for McpScope {
    type Err = std::convert::Infallible;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(McpScope::from(s.to_string()))
    }
}

impl serde::Serialize for McpScope {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

impl<'de> serde::Deserialize<'de> for McpScope {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Ok(McpScope::from(s))
    }
}
```

Create `crates/shared/web-api-types/src/oauth/mod.rs`:

```rust
//! OAuth 2.1 wire types shared between uptrakit-web-api (AS) and uptrakit-mcp (RS).

pub mod scope;

pub use scope::McpScope;
```

In `lib.rs` add `pub mod oauth;`.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p uptrakit-web-api-types oauth::scope` Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/shared/web-api-types/src/oauth/mod.rs crates/shared/web-api-types/src/oauth/scope.rs crates/shared/web-api-types/src/lib.rs
git commit -m "feat(web-api-types): add McpScope wire-safe enum

Per spec §8.1. Other(String) catch-all + KNOWN_VARIANTS const array
follow crates/shared/wire/src/lib.rs convention."
```

### Task 9: OAuth AS-internal enums (grant types, response types, code challenge method, token-endpoint auth method)

**Files:**

- Create: `crates/shared/web-api-types/src/oauth/grant_type.rs`
- Modify: `crates/shared/web-api-types/src/oauth/mod.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grant_type_serializes_as_oauth_strings() {
        assert_eq!(
            serde_json::to_string(&OAuthGrantType::AuthorizationCode).unwrap(),
            r#""authorization_code""#
        );
        assert_eq!(
            serde_json::to_string(&OAuthGrantType::RefreshToken).unwrap(),
            r#""refresh_token""#
        );
    }

    #[test]
    fn code_challenge_method_only_s256() {
        let s = serde_json::to_string(&CodeChallengeMethod::S256).unwrap();
        assert_eq!(s, r#""S256""#);
    }
}
```

- [ ] **Step 2: Verify failure**

Run: `cargo test -p uptrakit-web-api-types oauth::grant_type` Expected: FAIL.

- [ ] **Step 3: Implement**

These enums are AS-internal (advertised in AS metadata JSON but the values are fixed in the OAuth 2.1 spec —
`Other(String)` not needed for v1). `Copy` per snapshot rule for C-like enums:

```rust
//! AS-internal typed enums (RFC 6749 / RFC 8414 vocabulary). Copy because no Other(String).

use serde::{Deserialize, Serialize};

#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OAuthGrantType {
    AuthorizationCode,
    RefreshToken,
}

impl OAuthGrantType {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            OAuthGrantType::AuthorizationCode => "authorization_code",
            OAuthGrantType::RefreshToken => "refresh_token",
        }
    }
}

#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponseType {
    Code,
}

#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CodeChallengeMethod {
    S256,
}

#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenEndpointAuthMethod {
    None,
    ClientSecretBasic,
}
```

Re-export from `oauth/mod.rs`.

- [ ] **Step 4: Verify pass**

Run: `cargo test -p uptrakit-web-api-types oauth::grant_type` Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git commit -m "feat(web-api-types): add OAuth AS-internal enums

OAuthGrantType, ResponseType, CodeChallengeMethod, TokenEndpointAuthMethod.
All #[non_exhaustive] + Copy per the typed-enum-for-internal-discriminator rule."
```

### Task 10: CanonicalResourceUrl newtype

**Files:**

- Create: `crates/shared/web-api-types/src/oauth/canonical_url.rs`
- Modify: `crates/shared/web-api-types/src/oauth/mod.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_host() {
        let u = CanonicalResourceUrl::parse("https://controller.example.com/mcp").unwrap();
        assert_eq!(u.as_str(), "https://controller.example.com/mcp");
    }

    #[test]
    fn rejects_fragment() {
        let err = CanonicalResourceUrl::parse("https://controller.example.com/mcp#x").unwrap_err();
        assert!(matches!(err, CanonicalUrlError::Fragment));
    }

    #[test]
    fn rejects_query() {
        let err = CanonicalResourceUrl::parse("https://controller.example.com/mcp?x=1").unwrap_err();
        assert!(matches!(err, CanonicalUrlError::QueryString));
    }

    #[test]
    fn rejects_http_scheme() {
        let err = CanonicalResourceUrl::parse("http://controller.example.com/mcp").unwrap_err();
        assert!(matches!(err, CanonicalUrlError::InsecureScheme));
    }

    #[test]
    fn rejects_trailing_slash() {
        let err = CanonicalResourceUrl::parse("https://controller.example.com/mcp/").unwrap_err();
        assert!(matches!(err, CanonicalUrlError::TrailingSlash));
    }

    #[test]
    fn lowercases_host() {
        let u = CanonicalResourceUrl::parse("https://Controller.Example.Com/mcp").unwrap();
        assert_eq!(u.as_str(), "https://controller.example.com/mcp");
    }
}
```

- [ ] **Step 2: Verify failure**

Run: `cargo test -p uptrakit-web-api-types oauth::canonical_url` Expected: FAIL.

- [ ] **Step 3: Implement**

```rust
//! Canonical resource URL newtype. Single source of truth for RFC 8707 audience binding.

use thiserror::Error;
use url::Url;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CanonicalResourceUrl(Url);

#[non_exhaustive]
#[derive(Debug, Error, PartialEq, Eq)]
pub enum CanonicalUrlError {
    #[error("URL is malformed: {0}")]
    Malformed(#[from] url::ParseError),
    #[error("URL must use https scheme")]
    InsecureScheme,
    #[error("URL must not contain a fragment")]
    Fragment,
    #[error("URL must not contain a query string")]
    QueryString,
    #[error("URL must not have a trailing slash (use bare-root form)")]
    TrailingSlash,
}

impl CanonicalResourceUrl {
    /// Parse and normalize a canonical URL string.
    ///
    /// # Errors
    ///
    /// Returns `CanonicalUrlError::Malformed` if the URL fails RFC 3986 parsing.
    /// Returns `CanonicalUrlError::InsecureScheme` if the scheme is not `https`.
    /// Returns `CanonicalUrlError::Fragment` if the URL contains a fragment.
    /// Returns `CanonicalUrlError::QueryString` if the URL contains a query.
    /// Returns `CanonicalUrlError::TrailingSlash` if the URL has a trailing
    /// slash on a non-root path.
    #[must_use = "parsing returns a canonicalised URL; callers must persist or compare it"]
    pub fn parse(s: &str) -> Result<Self, CanonicalUrlError> {
        let mut url = Url::parse(s)?;
        if url.scheme() != "https" {
            return Err(CanonicalUrlError::InsecureScheme);
        }
        if url.fragment().is_some() {
            return Err(CanonicalUrlError::Fragment);
        }
        if url.query().is_some() {
            return Err(CanonicalUrlError::QueryString);
        }
        let path = url.path();
        if path.len() > 1 && path.ends_with('/') {
            return Err(CanonicalUrlError::TrailingSlash);
        }
        // `url` already lowercases host.
        Ok(Self(url))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    #[must_use]
    pub fn into_url(self) -> Url {
        self.0
    }
}
```

Re-export from `oauth/mod.rs`. Add `url = { workspace = true }` to `crates/shared/web-api-types/Cargo.toml` if not
present.

- [ ] **Step 4: Add CanonicalUrlConfig (primary host + accepted aliases)**

Plan D (`uptrakit-mcp` Resource Server) needs the full config — primary host plus the accepted-audience aliases — not
just a single URL. Promote the config struct into the same module so both the AS and the RS can consume it without
`uptrakit-mcp → uptrakit-web-api` coupling:

```rust
// crates/shared/web-api-types/src/oauth/canonical_url.rs (append)

pub const MAX_ACCEPTED_AUDIENCE_HOSTS: usize = 5;

#[derive(Clone, Debug)]
pub struct CanonicalUrlConfig {
    issuer: CanonicalResourceUrl,
    primary_resource: CanonicalResourceUrl,
    accepted_resources: Vec<CanonicalResourceUrl>,
}

#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum CanonicalUrlConfigError {
    #[error("oauth.canonical_host is required when oauth.mcp_enabled is true")]
    Missing,
    #[error("oauth.accepted_audience_hosts exceeds cap of {MAX_ACCEPTED_AUDIENCE_HOSTS}")]
    TooManyAliases,
    #[error("canonical host invalid: {0}")]
    InvalidHost(#[from] CanonicalUrlError),
}

impl CanonicalUrlConfig {
    /// Build a `CanonicalUrlConfig` from operator-supplied hostnames.
    ///
    /// # Errors
    /// Returns `Missing` if `canonical_host` is empty.
    /// Returns `TooManyAliases` if more than `MAX_ACCEPTED_AUDIENCE_HOSTS` aliases supplied.
    /// Returns `InvalidHost` for any malformed host (scheme, path, fragment, query).
    pub fn new(
        canonical_host: String,
        accepted_aliases: Vec<String>,
    ) -> Result<Self, CanonicalUrlConfigError> {
        if canonical_host.is_empty() {
            return Err(CanonicalUrlConfigError::Missing);
        }
        if accepted_aliases.len() > MAX_ACCEPTED_AUDIENCE_HOSTS {
            return Err(CanonicalUrlConfigError::TooManyAliases);
        }
        let issuer = CanonicalResourceUrl::parse(&format!("https://{canonical_host}"))?;
        let primary_resource =
            CanonicalResourceUrl::parse(&format!("https://{canonical_host}/mcp"))?;
        let mut accepted_resources = vec![primary_resource.clone()];
        for alias in accepted_aliases {
            let r = CanonicalResourceUrl::parse(&format!("https://{alias}/mcp"))?;
            if accepted_resources.iter().any(|p| p == &r) {
                continue;
            }
            accepted_resources.push(r);
        }
        Ok(Self { issuer, primary_resource, accepted_resources })
    }

    #[must_use]
    pub fn issuer(&self) -> &CanonicalResourceUrl { &self.issuer }

    #[must_use]
    pub fn primary_resource(&self) -> &CanonicalResourceUrl { &self.primary_resource }

    #[must_use]
    pub fn accepts_audience(&self, aud: &str) -> bool {
        self.accepted_resources.iter().any(|r| r.as_str() == aud)
    }
}
```

Add `thiserror = { workspace = true }` to `crates/shared/web-api-types/Cargo.toml` if not present.

Add tests covering: derives issuer and resource from host; rejects >5 aliases; accepts token with alias audience.

- [ ] **Step 5: Verify pass**

Run: `cargo test -p uptrakit-web-api-types oauth::canonical_url` Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git commit -m "feat(web-api-types): add CanonicalResourceUrl + CanonicalUrlConfig

Per spec §7. Single source of truth for RFC 8707 audience binding; parser
and constructor are #[must_use] and include # Errors docs. Config lives
in the shared crate so uptrakit-mcp can consume it without depending on
uptrakit-web-api (per Plan D prerequisite note)."
```

### Task 11: OpaqueRefreshToken + AuthorizationCode newtypes + McpAccessTokenClaims

**Files:**

- Create: `crates/shared/web-api-types/src/oauth/tokens.rs`
- Modify: `crates/shared/web-api-types/src/oauth/mod.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opaque_refresh_token_must_use_upr_prefix() {
        assert!(OpaqueRefreshToken::parse("upr_abc").is_ok());
        assert!(matches!(
            OpaqueRefreshToken::parse("upk_abc"),
            Err(TokenParseError::WrongPrefix { .. })
        ));
    }

    #[test]
    fn authorization_code_must_use_upc_prefix() {
        assert!(AuthorizationCode::parse("upc_abc").is_ok());
        assert!(matches!(
            AuthorizationCode::parse("upr_abc"),
            Err(TokenParseError::WrongPrefix { .. })
        ));
    }

    #[test]
    fn access_claims_round_trip_json() {
        let claims = McpAccessTokenClaims {
            iss: "https://example.com".into(),
            sub: "00000000-0000-0000-0000-000000000001".into(),
            aud: "https://example.com/mcp".into(),
            client_id: "abc".into(),
            scope: "mcp:read mcp:write".into(),
            jti: "00000000-0000-0000-0000-000000000002".into(),
            iat: 1_715_520_000,
            nbf: 1_715_520_000,
            exp: 1_715_520_900,
            tenant_id: "00000000-0000-0000-0000-000000000003".into(),
        };
        let json = serde_json::to_string(&claims).unwrap();
        let back: McpAccessTokenClaims = serde_json::from_str(&json).unwrap();
        assert_eq!(claims, back);
    }
}
```

- [ ] **Step 2: Verify failure**

Run: `cargo test -p uptrakit-web-api-types oauth::tokens` Expected: FAIL.

- [ ] **Step 3: Implement**

```rust
//! Token-shape newtypes + access-token claims envelope.

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[non_exhaustive]
#[derive(Debug, Error, PartialEq, Eq)]
pub enum TokenParseError {
    #[error("token must begin with {expected:?}")]
    WrongPrefix { expected: &'static str },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpaqueRefreshToken(String);

impl OpaqueRefreshToken {
    /// Parse a `upr_`-prefixed refresh token string.
    ///
    /// # Errors
    ///
    /// Returns `TokenParseError::WrongPrefix` if the string does not begin with `upr_`.
    #[must_use = "parsed refresh token must be either hashed for storage or returned to client"]
    pub fn parse(s: &str) -> Result<Self, TokenParseError> {
        if !s.starts_with("upr_") {
            return Err(TokenParseError::WrongPrefix { expected: "upr_" });
        }
        Ok(Self(s.to_string()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthorizationCode(String);

impl AuthorizationCode {
    /// Parse a `upc_`-prefixed authorization code string.
    ///
    /// # Errors
    ///
    /// Returns `TokenParseError::WrongPrefix` if the string does not begin with `upc_`.
    #[must_use]
    pub fn parse(s: &str) -> Result<Self, TokenParseError> {
        if !s.starts_with("upc_") {
            return Err(TokenParseError::WrongPrefix { expected: "upc_" });
        }
        Ok(Self(s.to_string()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// MCP OAuth access token claims envelope per spec §9.1.
///
/// `typ: "at+jwt"` per RFC 9068 lives in the JWT header, not these claims.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpAccessTokenClaims {
    pub iss: String,
    pub sub: String,
    pub aud: String,
    pub client_id: String,
    pub scope: String,
    pub jti: String,
    pub iat: i64,
    pub nbf: i64,
    pub exp: i64,
    pub tenant_id: String,
}
```

- [ ] **Step 4: Verify pass**

Run: `cargo test -p uptrakit-web-api-types oauth::tokens` Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git commit -m "feat(web-api-types): add token newtypes and McpAccessTokenClaims

Per spec §9.1, §10.2, §16. Newtypes carry # Errors docs and #[must_use]
markers; claims struct uses #[non_exhaustive] for forward compat."
```

### Task 12: OAuthError enum + impl_report_conversion! glue

**Files:**

- Create: `crates/shared/web-api-types/src/oauth/error.rs`
- Modify: `crates/shared/web-api-types/src/oauth/mod.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc6749_codes_serialize_snake_case() {
        let code = OAuthError::InvalidGrant.error_code();
        assert_eq!(code, "invalid_grant");
        let code = OAuthError::InvalidTarget.error_code();
        assert_eq!(code, "invalid_target");
    }

    #[test]
    fn server_error_is_500_otherwise_400() {
        assert_eq!(OAuthError::InvalidGrant.http_status(), 400);
        assert_eq!(OAuthError::InvalidClient.http_status(), 401);
        assert_eq!(OAuthError::ServerError.http_status(), 500);
        assert_eq!(OAuthError::TemporarilyUnavailable.http_status(), 503);
    }
}
```

- [ ] **Step 2: Verify failure**

Run: `cargo test -p uptrakit-web-api-types oauth::error` Expected: FAIL.

- [ ] **Step 3: Implement**

```rust
//! Typed OAuth error enum per RFC 6749 §5.2 + RFC 8707 §2.

use thiserror::Error;
use uptrakit_shared_macros::impl_report_conversion;
use rootcause::prelude::*;

#[non_exhaustive]
#[derive(Debug, Error, PartialEq, Eq, Clone)]
pub enum OAuthError {
    #[error("invalid_request: {0}")]
    InvalidRequest(String),
    #[error("invalid_client")]
    InvalidClient,
    #[error("invalid_grant: {0}")]
    InvalidGrant(&'static str),
    #[error("unauthorized_client")]
    UnauthorizedClient,
    #[error("unsupported_grant_type")]
    UnsupportedGrantType,
    #[error("invalid_scope")]
    InvalidScope,
    #[error("invalid_target")]
    InvalidTarget,
    #[error("access_denied")]
    AccessDenied,
    #[error("server_error")]
    ServerError,
    #[error("temporarily_unavailable")]
    TemporarilyUnavailable,
    #[error("insufficient_scope")]
    InsufficientScope,
    // Note: NO #[from] here — `impl_report_conversion!` below generates the conversion.
    // Adding both #[from] and the macro produces duplicate `From<DbErr>` impls.
    #[error("database error: {0}")]
    Database(sea_orm::DbErr),
}

impl OAuthError {
    // Cannot be `const fn` — pattern-matching tuple variants holding `String` is
    // not const-evaluable in Rust 2024 stable.
    #[must_use]
    pub fn error_code(&self) -> &'static str {
        match self {
            OAuthError::InvalidRequest(_) => "invalid_request",
            OAuthError::InvalidClient => "invalid_client",
            OAuthError::InvalidGrant(_) => "invalid_grant",
            OAuthError::UnauthorizedClient => "unauthorized_client",
            OAuthError::UnsupportedGrantType => "unsupported_grant_type",
            OAuthError::InvalidScope => "invalid_scope",
            OAuthError::InvalidTarget => "invalid_target",
            OAuthError::AccessDenied => "access_denied",
            OAuthError::ServerError => "server_error",
            OAuthError::TemporarilyUnavailable => "temporarily_unavailable",
            OAuthError::InsufficientScope => "insufficient_scope",
            OAuthError::Database(_) => "server_error",
        }
    }

    #[must_use]
    pub fn http_status(&self) -> u16 {
        match self {
            OAuthError::InvalidRequest(_)
            | OAuthError::InvalidGrant(_)
            | OAuthError::UnsupportedGrantType
            | OAuthError::InvalidScope
            | OAuthError::InvalidTarget => 400,
            OAuthError::InvalidClient | OAuthError::UnauthorizedClient => 401,
            OAuthError::AccessDenied | OAuthError::InsufficientScope => 403,
            OAuthError::ServerError | OAuthError::Database(_) => 500,
            OAuthError::TemporarilyUnavailable => 503,
        }
    }
}

impl_report_conversion!(sea_orm::DbErr => OAuthError::Database);
```

Add `sea-orm`, `rootcause`, `uptrakit-shared-macros`, `thiserror` to `web-api-types/Cargo.toml` if not already.

- [ ] **Step 4: Verify pass**

Run: `cargo test -p uptrakit-web-api-types oauth::error` Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git commit -m "feat(web-api-types): add OAuthError typed enum

RFC 6749 §5.2 + RFC 8707 §2 error codes mapped through #[non_exhaustive]
typed enum. impl_report_conversion! bridges sea_orm::DbErr per
docs/development/error-handling.md."
```

### Task 13: AuthorizeRequest, TokenRequest, ConsentDecision request structs + Validate impls

**Files:**

- Create: `crates/shared/web-api-types/src/oauth/requests.rs`
- Modify: `crates/shared/web-api-types/src/oauth/mod.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::Validate;

    #[test]
    fn authorize_request_validates_response_type() {
        let req = AuthorizeRequest {
            response_type: "token".into(), // wrong — only "code" allowed
            client_id: "x".into(),
            redirect_uri: "https://x/cb".into(),
            scope: "mcp:read".into(),
            state: "s".into(),
            code_challenge: "c".into(),
            code_challenge_method: "S256".into(),
            resource: "https://x/mcp".into(),
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn authorize_request_requires_s256() {
        let mut req = valid_authorize();
        req.code_challenge_method = "plain".into();
        assert!(req.validate().is_err());
    }

    fn valid_authorize() -> AuthorizeRequest {
        AuthorizeRequest {
            response_type: "code".into(),
            client_id: "x".into(),
            redirect_uri: "https://x/cb".into(),
            scope: "mcp:read".into(),
            state: "s".into(),
            code_challenge: "c".into(),
            code_challenge_method: "S256".into(),
            resource: "https://x/mcp".into(),
        }
    }
}
```

- [ ] **Step 2: Verify failure**

Run: `cargo test -p uptrakit-web-api-types oauth::requests` Expected: FAIL.

- [ ] **Step 3: Implement**

```rust
//! HTTP request types for OAuth AS endpoints.

use serde::{Deserialize, Serialize};

use crate::Validate;

use super::error::OAuthError;

#[non_exhaustive]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuthorizeRequest {
    pub response_type: String,
    pub client_id: String,
    pub redirect_uri: String,
    pub scope: String,
    pub state: String,
    pub code_challenge: String,
    pub code_challenge_method: String,
    pub resource: String,
}

impl Validate for AuthorizeRequest {
    type Error = OAuthError;
    fn validate(&self) -> Result<(), Self::Error> {
        if self.response_type != "code" {
            return Err(OAuthError::InvalidRequest("response_type must be 'code'".into()));
        }
        if self.code_challenge_method != "S256" {
            return Err(OAuthError::InvalidRequest("code_challenge_method must be 'S256'".into()));
        }
        if self.code_challenge.is_empty() {
            return Err(OAuthError::InvalidRequest("code_challenge is required (PKCE)".into()));
        }
        if self.state.is_empty() {
            return Err(OAuthError::InvalidRequest("state is required".into()));
        }
        if self.resource.is_empty() {
            return Err(OAuthError::InvalidTarget);
        }
        Ok(())
    }
}

#[non_exhaustive]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "grant_type", rename_all = "snake_case")]
pub enum TokenRequest {
    AuthorizationCode {
        code: String,
        redirect_uri: String,
        client_id: String,
        code_verifier: String,
        resource: String,
    },
    RefreshToken {
        refresh_token: String,
        client_id: String,
        #[serde(default)]
        scope: Option<String>,
        resource: String,
    },
}

impl Validate for TokenRequest {
    type Error = OAuthError;
    fn validate(&self) -> Result<(), Self::Error> {
        match self {
            TokenRequest::AuthorizationCode { code, code_verifier, resource, .. } => {
                if code.is_empty() || code_verifier.is_empty() {
                    return Err(OAuthError::InvalidRequest("code and code_verifier required".into()));
                }
                if resource.is_empty() {
                    return Err(OAuthError::InvalidTarget);
                }
                Ok(())
            }
            TokenRequest::RefreshToken { refresh_token, resource, .. } => {
                if refresh_token.is_empty() {
                    return Err(OAuthError::InvalidRequest("refresh_token required".into()));
                }
                if resource.is_empty() {
                    return Err(OAuthError::InvalidTarget);
                }
                Ok(())
            }
        }
    }
}

#[non_exhaustive]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConsentDecision {
    /// Hostname the user typed for unverified-client confirmation.
    /// Required when the client's `trusted_at` is null.
    pub typed_confirmation: Option<String>,
}

impl Validate for ConsentDecision {
    type Error = OAuthError;
    fn validate(&self) -> Result<(), Self::Error> {
        // Field-level validation only; cross-state ownership + typed-confirmation
        // comparison happens server-side.
        Ok(())
    }
}
```

- [ ] **Step 4: Verify pass**

Run: `cargo test -p uptrakit-web-api-types oauth::requests` Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git commit -m "feat(web-api-types): add AS request types with Validate impls

AuthorizeRequest, TokenRequest, ConsentDecision per spec §5.1, §10.3, §12.1.
TokenRequest is internally tagged on grant_type so axum's serde extractor
picks the correct variant from the form-encoded body."
```

### Task 14: DCR request/response + AS metadata + PRM metadata structs

**Files:**

- Create: `crates/shared/web-api-types/src/oauth/responses.rs`
- Create: `crates/shared/web-api-types/src/oauth/metadata.rs`
- Modify: `crates/shared/web-api-types/src/oauth/mod.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn as_metadata_includes_required_fields() {
        let meta = AuthorizationServerMetadata {
            issuer: "https://controller.example.com".into(),
            authorization_endpoint: "https://controller.example.com/oauth/authorize".into(),
            token_endpoint: "https://controller.example.com/oauth/token".into(),
            registration_endpoint: Some("https://controller.example.com/oauth/register".into()),
            scopes_supported: vec!["mcp:read".into(), "mcp:write".into()],
            response_types_supported: vec!["code".into()],
            grant_types_supported: vec!["authorization_code".into(), "refresh_token".into()],
            code_challenge_methods_supported: vec!["S256".into()],
            token_endpoint_auth_methods_supported: vec!["none".into(), "client_secret_basic".into()],
            client_id_metadata_document_supported: true,
            service_documentation: Some("https://controller.example.com/docs/oauth".into()),
        };
        let json = serde_json::to_value(&meta).unwrap();
        assert_eq!(json["code_challenge_methods_supported"], serde_json::json!(["S256"]));
    }

    #[test]
    fn prm_includes_authorization_servers_array() {
        let prm = ProtectedResourceMetadata {
            resource: "https://controller.example.com/mcp".into(),
            authorization_servers: vec!["https://controller.example.com".into()],
            scopes_supported: vec!["mcp:read".into(), "mcp:write".into()],
            bearer_methods_supported: vec!["header".into()],
            resource_documentation: Some("https://controller.example.com/docs/mcp".into()),
        };
        let json = serde_json::to_value(&prm).unwrap();
        assert!(json["authorization_servers"].is_array());
    }
}
```

- [ ] **Step 2: Verify failure**

Run: `cargo test -p uptrakit-web-api-types oauth` Expected: FAIL.

- [ ] **Step 3: Implement responses + metadata**

```rust
// crates/shared/web-api-types/src/oauth/responses.rs

use serde::{Deserialize, Serialize};

#[non_exhaustive]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: i64,
    pub refresh_token: Option<String>,
    pub refresh_expires_in: Option<i64>,
    pub scope: String,
}

#[non_exhaustive]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DcrRegistrationRequest {
    pub client_name: String,
    pub client_uri: Option<String>,
    pub logo_uri: Option<String>,
    pub redirect_uris: Vec<String>,
    pub grant_types: Vec<String>,
    pub response_types: Vec<String>,
    pub token_endpoint_auth_method: String,
    #[serde(default)]
    pub scope: Option<String>,
}

#[non_exhaustive]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DcrRegistrationResponse {
    pub client_id: String,
    pub client_id_issued_at: i64,
    pub registration_access_token: String,
    pub registration_client_uri: String,
    pub client_name: String,
    pub client_uri: Option<String>,
    pub logo_uri: Option<String>,
    pub redirect_uris: Vec<String>,
    pub grant_types: Vec<String>,
    pub response_types: Vec<String>,
    pub token_endpoint_auth_method: String,
    pub scope: String,
}
```

```rust
// crates/shared/web-api-types/src/oauth/metadata.rs

use serde::{Deserialize, Serialize};

#[non_exhaustive]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuthorizationServerMetadata {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registration_endpoint: Option<String>,
    pub scopes_supported: Vec<String>,
    pub response_types_supported: Vec<String>,
    pub grant_types_supported: Vec<String>,
    pub code_challenge_methods_supported: Vec<String>,
    pub token_endpoint_auth_methods_supported: Vec<String>,
    pub client_id_metadata_document_supported: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_documentation: Option<String>,
}

#[non_exhaustive]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProtectedResourceMetadata {
    pub resource: String,
    pub authorization_servers: Vec<String>,
    pub scopes_supported: Vec<String>,
    pub bearer_methods_supported: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_documentation: Option<String>,
}
```

Add Validate impl for `DcrRegistrationRequest` rejecting empty `redirect_uris`, unknown `grant_types`, unknown
`response_types`, and `token_endpoint_auth_method` outside `{"none", "client_secret_basic"}`.

- [ ] **Step 4: Verify pass**

Run: `cargo test -p uptrakit-web-api-types oauth` Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git commit -m "feat(web-api-types): add OAuth response + metadata types

TokenResponse, DcrRegistrationRequest/Response, AuthorizationServerMetadata,
ProtectedResourceMetadata. All #[non_exhaustive] per spec §5.2, §6.4, §11.2."
```

### Task 15: Extend McpRequestContext with auth_method + McpAuthMethod enum (additive)

**Files:**

- Modify: `crates/ui/mcp/src/context.rs`

- [ ] **Step 1: Read existing McpRequestContext**

Confirm current shape at `crates/ui/mcp/src/context.rs` (already `#[non_exhaustive]`).

- [ ] **Step 2: Add McpAuthMethod enum + field**

Edit `crates/ui/mcp/src/context.rs`:

```rust
use uptrakit_web_api_types::oauth::McpScope;

#[non_exhaustive]
#[derive(Clone, Debug)]
pub enum McpAuthMethod {
    ApiToken,
    OAuth {
        client_id: String,
        jti: uuid::Uuid,
        scopes: Vec<McpScope>,
    },
}

#[non_exhaustive]
#[derive(Clone, Debug)]
pub struct McpRequestContext {
    pub user_id: Uuid,
    pub token_id: Uuid,
    pub tenant_id: Uuid,
    pub permissions: Vec<Permission>,
    pub auth_method: McpAuthMethod,   // NEW
}

impl McpRequestContext {
    #[must_use]
    pub fn new(
        user_id: Uuid,
        token_id: Uuid,
        tenant_id: Uuid,
        permissions: Vec<Permission>,
        auth_method: McpAuthMethod,    // NEW arg
    ) -> Self { /* ... */ }
}
```

Update the existing `validate_api_token_for_mcp` to pass `McpAuthMethod::ApiToken`.

Add `uptrakit-web-api-types` to `crates/ui/mcp/Cargo.toml` dependencies (it already is — confirm).

- [ ] **Step 3: Update call sites in `crates/ui/mcp/src/auth.rs`**

Pass `McpAuthMethod::ApiToken` everywhere `McpRequestContext::new` is called.

- [ ] **Step 4: Run tests**

Run: `cargo test -p uptrakit-mcp` Expected: PASS — additive change only.

- [ ] **Step 5: Commit**

```bash
git commit -m "feat(mcp): add auth_method field to McpRequestContext

Per spec §6.2. McpAuthMethod is additive (#[non_exhaustive]); existing
API-token path passes ApiToken variant. OAuth variant lands in Plan D."
```

### Task 16: Re-export everything from oauth/mod.rs

**Files:**

- Modify: `crates/shared/web-api-types/src/oauth/mod.rs`

- [ ] **Step 1: Write final mod.rs**

```rust
//! OAuth 2.1 wire types shared between uptrakit-web-api (AS) and uptrakit-mcp (RS).

pub mod canonical_url;
pub mod error;
pub mod grant_type;
pub mod metadata;
pub mod requests;
pub mod responses;
pub mod scope;
pub mod tokens;

pub use canonical_url::{CanonicalResourceUrl, CanonicalUrlConfig, CanonicalUrlConfigError, CanonicalUrlError};
pub use error::OAuthError;
pub use grant_type::{
    CodeChallengeMethod, OAuthGrantType, ResponseType, TokenEndpointAuthMethod,
};
pub use metadata::{AuthorizationServerMetadata, ProtectedResourceMetadata};
pub use requests::{AuthorizeRequest, ConsentDecision, TokenRequest};
pub use responses::{DcrRegistrationRequest, DcrRegistrationResponse, TokenResponse};
pub use scope::McpScope;
pub use tokens::{AuthorizationCode, McpAccessTokenClaims, OpaqueRefreshToken, TokenParseError};

/// MCP Authorization spec revision this implementation targets. Emitted by the PRM endpoint
/// as `x-uptrakit-mcp-auth-spec-revision` per spec §23.1 so downstream tooling can correlate
/// behavior with the spec revision.
pub const MCP_AUTH_SPEC_REVISION: &str = "2025-11-25";
```

- [ ] **Step 2: Commit**

```bash
git commit -m "chore(web-api-types): public re-exports for oauth module

Includes MCP_AUTH_SPEC_REVISION constant per spec §23.1 and the
CanonicalUrlConfig promotion that uptrakit-mcp consumes."
```

### Task 17: Register OAuth audit action-type constants

**Files:**

- Modify: `crates/shared/audit-log/src/action_type.rs`

These are pure-data constant declarations (no behavior). Declaring them here lets Plans B and D emit real
`AuditEntry::builder(...)` calls from day one rather than relying on `tracing::warn!` stubs that have to be rewritten
later. Plan E Task 1 will register the constants in the `variants()` array and add a stable-string round-trip test; this
task only adds the constants themselves.

- [ ] **Step 1: Write the constants**

Follow the existing `AUTH_API_TOKEN_AUTHENTICATE` pattern in `action_type.rs`. Add inside the existing
`impl AuditActionType { ... }` block:

```rust
pub const OAUTH_AUTHORIZE_REQUEST: RegisteredAuditAction =
    RegisteredAuditAction::new("oauth.authorize_request");
pub const OAUTH_TOKEN_ISSUED: RegisteredAuditAction =
    RegisteredAuditAction::new("oauth.token_issued");
pub const OAUTH_TOKEN_REJECTED: RegisteredAuditAction =
    RegisteredAuditAction::new("oauth.token_rejected");
pub const OAUTH_REFRESH_ROTATED: RegisteredAuditAction =
    RegisteredAuditAction::new("oauth.refresh_rotated");
pub const OAUTH_REFRESH_REPLAY_DETECTED: RegisteredAuditAction =
    RegisteredAuditAction::new("oauth.refresh_replay_detected");
pub const OAUTH_CLIENT_REGISTERED: RegisteredAuditAction =
    RegisteredAuditAction::new("oauth.client_registered");
pub const OAUTH_CLIENT_FIRST_USE: RegisteredAuditAction =
    RegisteredAuditAction::new("oauth.client_first_use");
pub const OAUTH_CLIENT_METADATA_REFRESHED: RegisteredAuditAction =
    RegisteredAuditAction::new("oauth.client_metadata_refreshed");
pub const OAUTH_CLIENT_METADATA_CHANGED_MATERIALLY: RegisteredAuditAction =
    RegisteredAuditAction::new("oauth.client_metadata_changed_materially");
pub const OAUTH_CLIENT_TRUSTED: RegisteredAuditAction =
    RegisteredAuditAction::new("oauth.client_trusted");
pub const OAUTH_CLIENT_REVOKED: RegisteredAuditAction =
    RegisteredAuditAction::new("oauth.client_revoked");
pub const OAUTH_CLIENT_REGISTRATION_RATE_LIMITED: RegisteredAuditAction =
    RegisteredAuditAction::new("oauth.client_registration_rate_limited");
pub const OAUTH_CONFIG_AUDIENCE_HOSTS_CHANGED: RegisteredAuditAction =
    RegisteredAuditAction::new("oauth.config_audience_hosts_changed");
pub const OAUTH_CIMD_PARSE_FAILED: RegisteredAuditAction =
    RegisteredAuditAction::new("oauth.cimd_parse_failed");
pub const OAUTH_CONSENT_GRANT: RegisteredAuditAction =
    RegisteredAuditAction::new("oauth.consent_grant");
pub const OAUTH_CONSENT_DENY: RegisteredAuditAction =
    RegisteredAuditAction::new("oauth.consent_deny");
pub const OAUTH_CONSENT_REVOKE: RegisteredAuditAction =
    RegisteredAuditAction::new("oauth.consent_revoke");
pub const OAUTH_RATE_LIMITED: RegisteredAuditAction =
    RegisteredAuditAction::new("oauth.rate_limited");
pub const MCP_OAUTH_AUTHENTICATE: RegisteredAuditAction =
    RegisteredAuditAction::new("mcp.oauth_authenticate");
```

Do NOT add them to `variants()` here — Plan E Task 1 does that, together with the classification logic in Plan E Task 2.

- [ ] **Step 2: Compile-check**

Run: `cargo check -p uptrakit-audit-log` Expected: clean (constants are pure data; no `variants()` change yet).

- [ ] **Step 3: Commit**

```bash
git commit -m "feat(audit-log): declare OAuth action-type constants

Per spec §14.1. Constants declared in Plan A so Plans B/D emit real
AuditEntry::builder() calls from day one. variants() registration
+ classification land in Plan E."
```

### Task 18: Run full quality gates

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

Expected: every command exits 0.

- [ ] **Step 2: Resolve any clippy or test failures inline (no warnings suppressed)**

If clippy complains about new code, fix the root cause — do not add `#[allow]`. Per snapshot rule, prefer
`#[expect(..., reason = "...")]` over `#[allow]` only when an exception is genuinely required.

- [ ] **Step 3: Commit any gate-driven cleanups**

If new commits result, prefix with `chore(oauth): satisfy ... gate`.

### Task 19: Final commit message check + push

- [ ] **Step 1: Verify commit log**

```bash
git log --oneline --since="this morning" | head -20
```

Confirm every commit uses Conventional Commits format `type(scope): subject` per `docs/development/commit-messages.md`.

- [ ] **Step 2: Push to feature branch**

Defer to the merge-orchestration spec; do not push to main.

---

## Self-review checklist

- [ ] **Snapshot conformance**: every type carries `#[non_exhaustive]`; wire enums have `Other(String)` +
      `KNOWN_VARIANTS` + infallible `Deserialize`; AS-internal enums are `Copy` with no catch-all; every `pub fn`
      returning `Result` has `# Errors` doc; every parser/constructor has `#[must_use]`; `impl_report_conversion!` used
      at boundaries.
- [ ] **Idiomatic pattern check**: no Tokio mutex (no async locks introduced this plan); migrations use SeaORM `Schema`
      builder, not raw SQL; no `unwrap()` in non-test code.
- [ ] **Documentation completeness**: this plan is foundation only; doc updates land in Plan E. No doc updates required
      here beyond inline rustdocs (already required by snapshot rule).
- [ ] **Task atomicity**: each task is a single coherent change with its own commit.
- [ ] **Phase ordering**: this plan is the prerequisite for Plans B, C, D, E. Merge before any of them begins.
