<!-- markdownlint-disable MD013 -->

# RFC 8628 Device Auth — Plan 1 (Backend) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Refactor the uptrakit backend so the CLI device-authentication surface is strictly RFC 8628 + RFC 8414 compliant on the wire, with the data model and audit changes that support it.

**Architecture:** New `/api/v1/oauth/{device_authorization,token}` and `/.well-known/oauth-authorization-server` routes replace the legacy `/api/v1/auth/device{,/poll,/stream}` triplet. The existing `pending_device_flows` table gains `last_polled_at`, `interval`, `scope`, and `denied_by` columns; the `status` enum widens to include `denied`. The `web-api-auth/device_flow.rs` store gains `poll` (folding the old `consume`), `deny`, plus `validate_client_id` and `issue_access_token` future-migration seams. The SSE broadcaster is deleted. ADR 0009 records the wire-compliance decision and the four named seams.

**Tech Stack:** Rust + Axum + SeaORM + tokio + `rootcause::Report`. Wire enums via the `wire_safe_enum!` macro from `uptrakit_shared_macros`. SQLite read-then-write transactions use `BEGIN IMMEDIATE`. Idiomatic per `.superpowers/standards-snapshot.md`.

**Spec:** `docs/superpowers/specs/2026-05-12-rfc8628-device-auth-design.md` (commit `2ab437436`).

**Dependencies:** None. Plan 2 (client integration) depends on this plan landing first.

---

## File map

### Create

- `crates/shared/db/src/migration/m20260512_000001_device_flow_rfc8628.rs` — new migration.
- `crates/shared/web-api-types/src/oauth.rs` — new module: OAuth request/response/error types + `OAuthErrorCode` wire enum.
- `crates/ui/web-api/src/routes/oauth/mod.rs` — module wiring.
- `crates/ui/web-api/src/routes/oauth/device_authorization.rs` — RFC 8628 §3.1 handler.
- `crates/ui/web-api/src/routes/oauth/token.rs` — RFC 6749 §3.2 / RFC 8628 §3.4 dispatcher handler.
- `crates/ui/web-api/src/routes/oauth/metadata.rs` — RFC 8414 §3 discovery handler.
- `docs/adr/0009-oauth-2-device-flow-rfc-compliance.md` — new ADR.

### Modify

- `crates/shared/db/src/migration/mod.rs` — register new migration.
- `crates/shared/db/src/entity/pending_device_flow.rs` — add 4 fields.
- `crates/shared/types/src/device_auth_status.rs` — add `Denied` variant + tests.
- `crates/shared/web-api-types/src/lib.rs` — export new `oauth` module; delete dead `DeviceAuth*Start/Poll` types from `device_auth.rs`.
- `crates/shared/web-api-types/src/device_auth.rs` — remove `DeviceAuthStartRequest/Response`, `DeviceAuthPollRequest/Response`, SSE payload types; add `DeviceAuthDenyRequest/Response`, `DeviceAuthLookupQuery/Response`.
- `crates/ui/web-api-auth/src/auth/device_flow.rs` — rewrite store: add `poll`, `deny`, `validate_client_id`, `issue_access_token`; absorb `consume` into `poll`; delete `get_device_code_hash_by_user_code`.
- `crates/ui/web-api/src/routes/device_auth.rs` — delete `device_auth_start`, `device_auth_poll`, `device_auth_stream`; keep `device_auth_approve` (drop broadcaster call); add `device_auth_deny`, `device_auth_lookup`.
- `crates/ui/web-api/src/router.rs` — mount new routes; unmount deleted ones.
- `crates/ui/web-api/src/middleware/rate_limit.rs` — swap entries in `RATE_LIMITS`; rename / update tests.
- `crates/ui/web-api/src/app_state.rs` — remove `device_flow_broadcaster` field from `BroadcastState` and builder.
- `crates/ui/web-api/src/lib.rs` — drop `pub mod device_flow_broadcaster;` and the test_harness wiring.

### Delete

- `crates/ui/web-api/src/device_flow_broadcaster.rs` (entire file).

---

## Conventions referenced throughout

All citations point to `.superpowers/standards-snapshot.md` unless otherwise noted.

- **Wire-safe enums:** mandatory `wire_safe_enum!` macro (per `docs/development/coding-standards.md` §"Wire-Safe Other(String) Catch-All — Required implementation"). Never hand-write `Serialize`/`Deserialize`/`rename_all` for a wire enum.
- **SeaORM migrations:** chronological file names `m<YYYYMMDD>_<NNNNNN>_<slug>.rs`; register in `migration/mod.rs`.
- **SeaORM transactions for read-then-write:** `db.begin_with_options(TransactionOptions { sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate), ..Default::default() })` (project Binding Rule).
- **Axum extractors:** `Form<T>` for form-urlencoded, `Json<T>` for JSON, `Query<T>` for query strings.
- **`Validate` trait:** every HTTP request type implements it; handlers call `req.validate()` and return `invalid_request` (OAuth) or existing `BadRequest` (UI internal).
- **Errors:** `rootcause::Report` + `report!()` / `bail!()`. No `.unwrap()` in production. `#[expect(lint, reason = "...")]` instead of `#[allow]` (reason mandatory).
- **Audit:** existing constants `AUTH_DEVICE_START`, `AUTH_DEVICE_POLL`, `AUTH_DEVICE_APPROVE`, `AUTH_DEVICE_DENY` (already in `crates/shared/audit-log/src/action_type.rs`).
- **Tests touching time:** inject `now: OffsetDateTime` as a function parameter; never combine `#[tokio::test(start_paused = true)]` with SeaORM SQLite tests.
- **OpenAPI:** every new route handler carries `#[utoipa::path(...)]`.

Commit style: Conventional Commits. Scope examples used below: `feat(web-api)`, `feat(web-api-auth)`, `feat(web-api-types)`, `feat(shared-db)`, `feat(shared-types)`, `chore(web-api)`, `docs(adr)`.

---

## Task 1: Add `Denied` variant to `DeviceAuthStatus`

**Files:**

- Modify: `crates/shared/types/src/device_auth_status.rs`

Adds the new `Denied` variant first because every downstream layer (entity, store, routes, types) needs it. Existing rule from the snapshot: `#[non_exhaustive]` is already on the enum, so adding a variant is type-additive — but every test array in this file enumerates the variants explicitly and will silently miss `Denied` unless updated.

- [ ] **Step 1: Read the file to confirm structure**

Run: `cat crates/shared/types/src/device_auth_status.rs | head -120`
Expected: variants `Pending`, `Authorized`, `Expired`; three test arrays at lines 78, 91, 102.

- [ ] **Step 2: Add `Denied` variant + match arms**

Edit `crates/shared/types/src/device_auth_status.rs` — add `Denied` to the enum, `as_str`, `Display` (via `as_str`), and `FromStr`:

```rust
pub enum DeviceAuthStatus {
    #[cfg_attr(feature = "sea-orm", sea_orm(string_value = "pending"))]
    Pending,
    #[cfg_attr(feature = "sea-orm", sea_orm(string_value = "authorized"))]
    Authorized,
    #[cfg_attr(feature = "sea-orm", sea_orm(string_value = "denied"))]
    Denied,
    #[cfg_attr(feature = "sea-orm", sea_orm(string_value = "expired"))]
    Expired,
}

impl DeviceAuthStatus {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Authorized => "authorized",
            Self::Denied => "denied",
            Self::Expired => "expired",
        }
    }
}

impl FromStr for DeviceAuthStatus {
    type Err = ParseDeviceAuthStatusError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pending" => Ok(Self::Pending),
            "authorized" => Ok(Self::Authorized),
            "denied" => Ok(Self::Denied),
            "expired" => Ok(Self::Expired),
            _ => Err(ParseDeviceAuthStatusError),
        }
    }
}
```

- [ ] **Step 3: Update the three test arrays to include `Denied`**

In `mod tests`, replace the three `for variant in [...]` arrays (at the existing line locations of `serde_round_trip`, `display_matches_as_str`, `from_str_round_trip`) with:

```rust
for variant in [
    DeviceAuthStatus::Pending,
    DeviceAuthStatus::Authorized,
    DeviceAuthStatus::Denied,
    DeviceAuthStatus::Expired,
] {
    // (existing body unchanged)
}
```

Also extend the `serde_values` test with the new variant:

```rust
assert_eq!(
    serde_json::to_string(&DeviceAuthStatus::Denied).unwrap(),
    r#""denied""#
);
```

- [ ] **Step 4: Add a denied-string assertion test**

Append to `mod tests`:

```rust
#[test]
fn denied_variant_string_value_is_denied() {
    assert_eq!(DeviceAuthStatus::Denied.as_str(), "denied");
    assert_eq!(format!("{}", DeviceAuthStatus::Denied), "denied");
    assert_eq!("denied".parse::<DeviceAuthStatus>().unwrap(), DeviceAuthStatus::Denied);
}
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p uptrakit-shared-types device_auth_status -- --nocapture`
Expected: all `device_auth_status::tests::*` pass.

- [ ] **Step 6: Commit**

```bash
git add crates/shared/types/src/device_auth_status.rs
git commit -m "feat(shared-types): add DeviceAuthStatus::Denied variant for RFC 8628 access_denied"
```

---

## Task 2: Migration `m20260512_000001_device_flow_rfc8628`

**Files:**

- Create: `crates/shared/db/src/migration/m20260512_000001_device_flow_rfc8628.rs`
- Modify: `crates/shared/db/src/migration/mod.rs`

Adds `last_polled_at`, `interval` (NOT NULL DEFAULT 5), `scope`, and `denied_by` columns to `pending_device_flows`. The `status` column is `TEXT` and accepts the new `'denied'` literal without a schema change. Pattern matches the project's existing migration style (see `m20260510_000001_instance_plugin_setting.rs`).

- [ ] **Step 1: Write the migration**

Create `crates/shared/db/src/migration/m20260512_000001_device_flow_rfc8628.rs`:

```rust
use sea_orm_migration::prelude::*;

pub(super) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260512_000001_device_flow_rfc8628"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(PendingDeviceFlows::Table)
                    .add_column(
                        ColumnDef::new(PendingDeviceFlows::LastPolledAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .add_column(
                        ColumnDef::new(PendingDeviceFlows::Interval)
                            .integer()
                            .not_null()
                            .default(5),
                    )
                    .add_column(
                        ColumnDef::new(PendingDeviceFlows::Scope)
                            .text()
                            .null(),
                    )
                    .add_column(
                        ColumnDef::new(PendingDeviceFlows::DeniedBy)
                            .uuid()
                            .null(),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(PendingDeviceFlows::Table)
                    .drop_column(PendingDeviceFlows::DeniedBy)
                    .drop_column(PendingDeviceFlows::Scope)
                    .drop_column(PendingDeviceFlows::Interval)
                    .drop_column(PendingDeviceFlows::LastPolledAt)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum PendingDeviceFlows {
    Table,
    LastPolledAt,
    Interval,
    Scope,
    DeniedBy,
}
```

- [ ] **Step 2: Register in `mod.rs`**

In `crates/shared/db/src/migration/mod.rs`:

1. Add `mod m20260512_000001_device_flow_rfc8628;` alongside the other `mod` lines at the top.
2. Append `Box::new(m20260512_000001_device_flow_rfc8628::Migration),` to the `migrations()` `Vec` after the existing `m20260510_000001_instance_plugin_setting` entry.

- [ ] **Step 3: Run sqlite check**

Run: `cargo check -p uptrakit-shared-db --no-default-features --features db-sqlite`
Expected: clean compile.

- [ ] **Step 4: Verify migration list compiles and runs**

Run: `cargo test -p uptrakit-shared-db --features db-sqlite -- migration --nocapture 2>&1 | head -40`
Expected: all migration tests pass (each one runs the full migration chain).

- [ ] **Step 5: Commit**

```bash
git add crates/shared/db/src/migration/m20260512_000001_device_flow_rfc8628.rs \
        crates/shared/db/src/migration/mod.rs
git commit -m "feat(shared-db): add device flow columns for RFC 8628 (interval, scope, denied_by, last_polled_at)"
```

---

## Task 3: Entity update — `pending_device_flow`

**Files:**

- Modify: `crates/shared/db/src/entity/pending_device_flow.rs`

Adds four new SeaORM model fields matching the migration. The Rustdoc clarifies the `user_id` vs `denied_by` mutual-exclusion invariant so future readers don't conflate them.

- [ ] **Step 1: Add the four fields with Rustdoc**

Replace `crates/shared/db/src/entity/pending_device_flow.rs` with:

```rust
use sea_orm::entity::prelude::*;
use time::OffsetDateTime;
use uptrakit_shared_types::DeviceAuthStatus;

/// A pending device-authorization flow (RFC 8628 §3.1).
///
/// Status transitions:
/// - `Pending` (initial) → `Authorized` (via `approve`) → row consumed by `poll`.
/// - `Pending` → `Denied` (via `deny`).
/// - `Pending` → `Expired` (background sweeper after `expires_at`).
///
/// Invariant: at most one of `user_id` (approver) and `denied_by` (denier) is `Some`.
/// A row in `Authorized` status has `user_id = Some(...)` and `denied_by = None`;
/// a row in `Denied` status has `user_id = None` and `denied_by = Some(...)`.
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "pending_device_flows")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    #[sea_orm(unique, column_type = "Text")]
    pub device_code_hash: String,
    #[sea_orm(unique, column_type = "Text")]
    pub user_code: String,
    pub status: DeviceAuthStatus,
    /// User who approved this flow. `Some` only when `status = Authorized`.
    pub user_id: Option<Uuid>,
    /// User who denied this flow. `Some` only when `status = Denied`.
    pub denied_by: Option<Uuid>,
    pub client_name: Option<String>,
    /// Requested OAuth `scope` parameter (RFC 8628 §3.1). Echoed on token response,
    /// not yet enforced (Seam 2 in the spec's Future Migrations section).
    pub scope: Option<String>,
    /// Current polling interval in seconds. Initialised to 5; bumped by 5 each
    /// time a `slow_down` is returned to the caller.
    pub interval: i32,
    /// Timestamp of the most recent poll request. `None` until the first poll.
    pub last_polled_at: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
    pub expires_at: OffsetDateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
```

- [ ] **Step 2: Build sqlite + full feature flags**

Run: `cargo check -p uptrakit-shared-db --no-default-features --features db-sqlite && cargo check -p uptrakit-shared-db --all-features`
Expected: clean compile for both.

- [ ] **Step 3: Commit**

```bash
git add crates/shared/db/src/entity/pending_device_flow.rs
git commit -m "feat(shared-db): add device-flow entity fields for interval/scope/denied_by/last_polled_at"
```

---

## Task 4: New `oauth.rs` module in `web-api-types`

**Files:**

- Create: `crates/shared/web-api-types/src/oauth.rs`
- Modify: `crates/shared/web-api-types/src/lib.rs`

Defines the new wire-facing types: `OAuthErrorCode` via `wire_safe_enum!`, request/response structs, `Validate` impls. The OAuth error response shape follows RFC 6749 §5.2 (with the uptrakit-extension `interval` field for `slow_down`). The request types do **not** carry struct-level `#[serde(rename_all)]` — RFC field names are already snake_case and `grant_type` must round-trip the URI literal without serde transformation.

- [ ] **Step 1: Write the new module**

Create `crates/shared/web-api-types/src/oauth.rs`:

```rust
//! RFC 8628 (Device Authorization Grant) + RFC 8414 (Authorization Server
//! Metadata) request/response types.
//!
//! See `docs/superpowers/specs/2026-05-12-rfc8628-device-auth-design.md`.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uptrakit_shared_macros::wire_safe_enum;

use crate::Validate;

// --- Error codes --------------------------------------------------------

wire_safe_enum! {
    /// OAuth 2.0 error codes per RFC 6749 §5.2 and RFC 8628 §3.5.
    ///
    /// Wire-safe via `Other(String)` so the CLI tolerates new codes added
    /// by a newer server.
    #[derive(Clone, Debug, PartialEq, Eq, Hash)]
    #[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
    pub enum OAuthErrorCode {
        AuthorizationPending => "authorization_pending",
        SlowDown             => "slow_down",
        AccessDenied         => "access_denied",
        ExpiredToken         => "expired_token",
        InvalidRequest       => "invalid_request",
        InvalidClient        => "invalid_client",
        InvalidGrant         => "invalid_grant",
        UnsupportedGrantType => "unsupported_grant_type",
    }
    parse_error = ParseOAuthErrorCodeError("invalid OAuth 2.0 error code");
}

// --- Device-authorization request / response ---------------------------

/// RFC 8628 §3.1 device-authorization request. Form-urlencoded body.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DeviceAuthorizationRequest {
    /// Public client identifier. Must match the server's configured constant.
    pub client_id: String,
    /// Optional space-separated scope list (RFC 6749 §3.3). Stored on the flow
    /// row, echoed on the token response, not yet enforced.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    /// Uptrakit extension: free-form audit label, e.g. `cli-laptop-2026-05-12`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_name: Option<String>,
}

impl Validate for DeviceAuthorizationRequest {
    fn validate(&self) -> Result<(), String> {
        if self.client_id.trim().is_empty() {
            return Err("client_id is required".into());
        }
        if let Some(scope) = &self.scope {
            if scope.trim().is_empty() {
                return Err("scope must be non-empty when present".into());
            }
        }
        Ok(())
    }
}

/// RFC 8628 §3.2 device-authorization response.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DeviceAuthorizationResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: String,
    pub expires_in: u64,
    pub interval: i32,
}

// --- Token request / response ------------------------------------------

/// RFC 6749 §3.2 / RFC 8628 §3.4 token endpoint request. Form-urlencoded.
///
/// `grant_type` is intentionally `String` — the device-code grant value is the
/// literal URI `urn:ietf:params:oauth:grant-type:device_code`; the handler
/// matches the raw string and returns `unsupported_grant_type` for any other
/// value.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct OAuthTokenRequest {
    pub grant_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
}

impl Validate for OAuthTokenRequest {
    fn validate(&self) -> Result<(), String> {
        if self.grant_type.trim().is_empty() {
            return Err("grant_type is required".into());
        }
        Ok(())
    }
}

/// RFC 6749 §5.1 success token response.
///
/// `expires_in`, `refresh_token`, and `scope` are `Option` + `skip_serializing_if`
/// so they are omitted (not serialised as `null`) when unset. Today the server
/// always omits all three; the fields exist on the wire type so a future
/// migration to short-lived bearer + refresh tokens is purely additive (Seam 1).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct OAuthTokenResponse {
    pub access_token: String,
    pub token_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_in: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
}

/// RFC 6749 §5.2 error response, with the uptrakit `interval` extension used
/// by `slow_down` (RFC 8628 §3.5).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct OAuthErrorResponse {
    pub error: OAuthErrorCode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_description: Option<String>,
    /// Server-recommended polling interval (seconds). Only set on `slow_down`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interval: Option<i32>,
}

// --- Discovery metadata ------------------------------------------------

/// RFC 8414 §3 authorization server metadata (device-grant-only subset).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct OAuthAuthorizationServerMetadata {
    pub issuer: String,
    pub device_authorization_endpoint: String,
    pub token_endpoint: String,
    pub grant_types_supported: Vec<String>,
    pub response_types_supported: Vec<String>,
    pub token_endpoint_auth_methods_supported: Vec<String>,
    pub code_challenge_methods_supported: Vec<String>,
}

// --- UI-internal: deny + lookup ---------------------------------------

/// Request body for `POST /api/v1/auth/device/deny` (UI-internal).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DeviceAuthDenyRequest {
    pub user_code: String,
}

impl Validate for DeviceAuthDenyRequest {
    fn validate(&self) -> Result<(), String> {
        if self.user_code.trim().is_empty() {
            return Err("user_code is required".into());
        }
        Ok(())
    }
}

/// Response body for `POST /api/v1/auth/device/deny`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DeviceAuthDenyResponse {
    pub message: String,
}

/// Query string for `GET /api/v1/auth/device/lookup` (UI-internal).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::IntoParams))]
pub struct DeviceAuthLookupQuery {
    pub user_code: String,
}

impl Validate for DeviceAuthLookupQuery {
    fn validate(&self) -> Result<(), String> {
        if self.user_code.trim().is_empty() {
            return Err("user_code is required".into());
        }
        Ok(())
    }
}

/// Response body for `GET /api/v1/auth/device/lookup`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DeviceAuthLookupResponse {
    pub client_name: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub expires_at: OffsetDateTime,
}

// --- Tests --------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const KNOWN_VARIANTS: &[&str] = &[
        "authorization_pending",
        "slow_down",
        "access_denied",
        "expired_token",
        "invalid_request",
        "invalid_client",
        "invalid_grant",
        "unsupported_grant_type",
    ];

    #[test]
    fn oauth_error_code_known_variants_round_trip() {
        for wire in KNOWN_VARIANTS {
            let value = OAuthErrorCode::from((*wire).to_string());
            assert_eq!(value.as_str(), *wire);
            let json = serde_json::to_string(&value).expect("serialize");
            assert_eq!(json, format!("\"{wire}\""));
            let parsed: OAuthErrorCode = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(parsed, value);
        }
    }

    #[test]
    fn oauth_error_code_unknown_deserializes_to_other() {
        let json = "\"temporarily_unavailable\"";
        let value: OAuthErrorCode = serde_json::from_str(json).expect("deserialize");
        assert_eq!(value, OAuthErrorCode::Other("temporarily_unavailable".into()));
        // Round-trip preserves the inner string.
        assert_eq!(serde_json::to_string(&value).expect("serialize"), json);
    }

    #[test]
    fn validate_rejects_empty_client_id() {
        let req = DeviceAuthorizationRequest {
            client_id: "".into(),
            scope: None,
            client_name: None,
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn validate_rejects_empty_grant_type() {
        let req = OAuthTokenRequest {
            grant_type: "".into(),
            device_code: None,
            client_id: None,
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn token_response_omits_optional_fields() {
        let resp = OAuthTokenResponse {
            access_token: "abc".into(),
            token_type: "Bearer".into(),
            expires_in: None,
            refresh_token: None,
            scope: None,
        };
        let json = serde_json::to_value(&resp).expect("serialize");
        assert_eq!(json["access_token"], "abc");
        assert_eq!(json["token_type"], "Bearer");
        assert!(json.get("expires_in").is_none(), "expires_in must be omitted");
        assert!(json.get("refresh_token").is_none(), "refresh_token must be omitted");
        assert!(json.get("scope").is_none(), "scope must be omitted");
    }

    #[test]
    fn error_response_with_slow_down_interval() {
        let resp = OAuthErrorResponse {
            error: OAuthErrorCode::SlowDown,
            error_description: None,
            interval: Some(10),
        };
        let json = serde_json::to_value(&resp).expect("serialize");
        assert_eq!(json["error"], "slow_down");
        assert_eq!(json["interval"], 10);
        assert!(json.get("error_description").is_none());
    }
}
```

- [ ] **Step 2: Export the module + drop dead types**

In `crates/shared/web-api-types/src/lib.rs`, add `pub mod oauth;` next to the existing `pub mod device_auth;`.

In `crates/shared/web-api-types/src/device_auth.rs`, replace the file body with the surviving types only:

```rust
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::Validate;

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DeviceAuthApproveRequest {
    pub user_code: String,
}

impl Validate for DeviceAuthApproveRequest {
    fn validate(&self) -> Result<(), String> {
        if self.user_code.trim().is_empty() {
            return Err("user_code is required".into());
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DeviceAuthApproveResponse {
    pub message: String,
}
```

(The deleted types: `DeviceAuthStartRequest/Response`, `DeviceAuthPollRequest/Response`, `DeviceAuthAuthorizedSse`, `DeviceAuthExpiredSse`.)

Also update `crates/shared/web-api-types/src/lib.rs` to remove any re-exports of the deleted types if they exist there, and remove any test-module usages such as the existing `pub use uptrakit_shared_types::DeviceAuthStatus` references that mentioned `Pending`/`Authorized` for the deleted poll-response type — keep `DeviceAuthStatus` re-export (still used elsewhere) but delete the now-dead helper tests.

- [ ] **Step 3: Build + test**

Run: `cargo check -p uptrakit-web-api-types --all-features && cargo test -p uptrakit-web-api-types --all-features -- oauth`
Expected: clean compile; all `oauth::tests::*` pass.

- [ ] **Step 4: Commit**

```bash
git add crates/shared/web-api-types/src/oauth.rs \
        crates/shared/web-api-types/src/device_auth.rs \
        crates/shared/web-api-types/src/lib.rs
git commit -m "feat(web-api-types): add OAuth 2.0 + RFC 8628 wire types via wire_safe_enum! macro"
```

---

## Task 5: Rewrite `device_flow.rs` store (Part A — model + helpers)

**Files:**

- Modify: `crates/ui/web-api-auth/src/auth/device_flow.rs`

This task expands the `pending_device_flow` ActiveModel sites to set the new columns at creation, adds the constants and the two named seams (`validate_client_id`, `issue_access_token`), plus the `apply_scope_to_token` no-op stub. The `poll` and `deny` methods land in Task 6 so this commit stays reviewable.

- [ ] **Step 1: Add imports and constants**

In `crates/ui/web-api-auth/src/auth/device_flow.rs`, add to the top of the file (after the existing imports):

```rust
use sea_orm::{TransactionTrait, TransactionOptions};
use sea_orm::SqliteTransactionMode;
use uptrakit_web_api_types::oauth::OAuthErrorCode;
```

(Keep the existing imports.) Add new constants after `USER_CODE_ALPHABET`:

```rust
/// Hardcoded OAuth public-client identifier for the CLI. Future migration
/// (Seam 3 in the spec): replace this constant with a lookup against an
/// `oauth_clients` allowlist table.
pub const CLIENT_ID: &str = "uptrakit-cli";

/// Default polling interval (seconds) returned to clients on flow creation.
pub const POLL_INTERVAL_SECONDS: i32 = 5;

/// How many seconds to add to `interval` each time a `slow_down` is returned
/// (per RFC 8628 §3.5 client-side bump rule).
pub const POLL_INTERVAL_BUMP_SECONDS: i32 = 5;
```

- [ ] **Step 2: Add the two named seams + scope stub**

Append to the file (after `DeviceFlowStore` impl, before the `generate_user_code` fn):

```rust
/// Validate the OAuth `client_id` form parameter.
///
/// **Seam 3** — future migration replaces this function with a DB lookup
/// against an `oauth_clients` allowlist. The call sites in the routes layer
/// stay unchanged.
#[must_use]
pub fn validate_client_id(client_id: &str) -> Result<(), OAuthErrorCode> {
    if client_id == CLIENT_ID {
        Ok(())
    } else {
        Err(OAuthErrorCode::InvalidClient)
    }
}

/// Apply the requested `scope` parameter to a freshly-minted token.
///
/// **Seam 2** — today this is a no-op stub: scopes are recorded on the flow
/// row and echoed in audit, but no Permission narrowing happens. A future
/// migration replaces this body with a real scope→Permission map.
pub fn apply_scope_to_token(_token_id: uuid::Uuid, _scope: Option<&str>) {
    // intentional no-op
}
```

For `issue_access_token`, we will route it through the existing API-token mint code. Look at the existing `consume` consumer in `routes/device_auth.rs` for the call shape. Add this function next to the seam helpers above:

```rust
/// Mint a long-lived API access token for the given user.
///
/// **Seam 1** — future migration replaces this single function with
/// short-lived bearer + refresh-token issuance. Callers receive a
/// `SecretString` today; the future signature returns a `TokenPair`.
#[must_use = "minted token must be returned to the caller"]
pub async fn issue_access_token(
    db: &DatabaseConnection,
    user_id: Uuid,
    token_name: String,
) -> Result<SecretString> {
    // Delegate to the existing API-token issuance pipeline. The exact call is
    // resolved by reading `crates/ui/web-api-auth/src/auth/api_token.rs` —
    // typically `ApiTokenStore::create(db, user_id, token_name).await`.
    // Until refactored to a free function, the caller can construct the store
    // and invoke `create()` directly; this wrapper exists so future
    // refresh-token support has exactly one call site to update.
    use crate::auth::api_token::ApiTokenStore;
    let store = ApiTokenStore::new(db.clone());
    store
        .create(user_id, token_name)
        .await
        .map(|created| created.raw_token)
}
```

If the existing API-token API differs slightly (different return shape, named `mint` instead of `create`, etc.), adapt this wrapper to match — read `api_token.rs` first and use the actual function. The point is one wrapping function; the body is the swap-point.

Add `use uptrakit_shared_types::SecretString;` near the top imports if not already present.

- [ ] **Step 3: Expand the `create` method to write the new columns**

Find the `create()` method (line ~58). Replace the `ActiveModel` literal to initialise the four new columns:

```rust
let model = pending_device_flow::ActiveModel {
    id: Set(id),
    device_code_hash: Set(device_code_hash),
    user_code: Set(raw_user_code),
    status: Set(DeviceAuthStatus::Pending),
    user_id: Set(None),
    denied_by: Set(None),
    client_name: Set(client_name),
    scope: Set(scope),
    interval: Set(POLL_INTERVAL_SECONDS),
    last_polled_at: Set(None),
    created_at: Set(now),
    expires_at: Set(expires_at),
};
```

Update the signature to accept the new `scope` parameter:

```rust
pub async fn create(
    &self,
    client_name: Option<String>,
    scope: Option<String>,
) -> Result<(String, String)> { /* ... unchanged body otherwise ... */ }
```

The existing call site in the old `device_auth_start` handler will be deleted in Task 8; the new `oauth/device_authorization.rs` handler (Task 8) passes `scope` through.

Update every existing test in `mod tests` that calls `store.create(...)` to pass `None` for the new `scope` parameter — `grep -n "store.create" crates/ui/web-api-auth/src/auth/device_flow.rs` lists them. Example update: `store.create(Some("test-client".into()), None).await`.

- [ ] **Step 4: Run existing tests**

Run: `cargo test -p uptrakit-web-api-auth device_flow -- --nocapture`
Expected: every pre-existing `device_flow` test still passes; nothing depends on the deleted methods yet.

- [ ] **Step 5: Commit**

```bash
git add crates/ui/web-api-auth/src/auth/device_flow.rs
git commit -m "feat(web-api-auth): add OAuth seam helpers + scope/interval to device flow create"
```

---

## Task 6: Rewrite `device_flow.rs` store (Part B — `poll` + `deny`)

**Files:**

- Modify: `crates/ui/web-api-auth/src/auth/device_flow.rs`

Replaces `get_status` + `consume` + `get_device_code_hash_by_user_code` with the unified `poll(now)` method (BEGIN IMMEDIATE; six branches per the spec) plus the new `deny` method. The previous `consume` logic moves into `poll` step 4.

- [ ] **Step 1: Add the `PollOutcome` enum**

After the existing `DeviceFlowStatus` enum, add:

```rust
/// Outcome of a single `poll()` call. Internal — the route layer maps these
/// onto RFC 8628 §3.5 wire codes.
///
/// Not `#[non_exhaustive]`: this type is crate-private.
#[derive(Debug)]
pub enum PollOutcome {
    /// Flow is approved; token has been minted.
    Authorized { token: SecretString, token_name: String },
    /// Flow is still pending; client should poll again after `interval` seconds.
    Pending,
    /// Client polled too fast; bumped `interval` is returned to it.
    SlowDown { bumped_interval: i32 },
    /// Operator denied this flow.
    Denied,
    /// Flow has expired.
    Expired,
    /// Device code is unknown (route layer collapses this into `expired_token`).
    Unknown,
    /// Device code is malformed (route layer maps to `invalid_grant`).
    MalformedDeviceCode,
}
```

- [ ] **Step 2: Implement `poll`**

Inside `impl DeviceFlowStore`, add:

```rust
/// Poll a device flow. RFC 8628 §3.4–§3.5.
///
/// All branches run inside a single `BEGIN IMMEDIATE` SQLite transaction
/// (per CLAUDE.md "SQLite Transaction Rules": read-then-write must use
/// Immediate to avoid `SQLITE_BUSY_SNAPSHOT`). On Postgres this is a no-op
/// per SeaORM.
pub async fn poll(&self, device_code: &str, now: OffsetDateTime) -> Result<PollOutcome> {
    if device_code.is_empty() {
        return Ok(PollOutcome::MalformedDeviceCode);
    }
    let hash = hash_token(device_code);

    let txn = self
        .db
        .begin_with_config(
            Some(sea_orm::IsolationLevel::Serializable),
            Some(sea_orm::AccessMode::ReadWrite),
        )
        .await
        .context_to()?;
    // SeaORM exposes per-driver hints through TransactionOptions in some
    // versions; if `begin_with_options` is available in this revision, prefer:
    //   let txn = self.db.begin_with_options(TransactionOptions {
    //       sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate),
    //       ..Default::default()
    //   }).await.context_to()?;
    // Adapt at impl time based on the version pinned in Cargo.toml.

    let flow_opt = PendingDeviceFlow::find()
        .filter(pending_device_flow::Column::DeviceCodeHash.eq(&hash))
        .one(&txn)
        .await
        .context_to()?;

    let Some(flow) = flow_opt else {
        txn.commit().await.context_to()?;
        return Ok(PollOutcome::Unknown);
    };

    if flow.expires_at <= now {
        txn.commit().await.context_to()?;
        return Ok(PollOutcome::Expired);
    }

    match flow.status {
        DeviceAuthStatus::Authorized => {
            let user_id = flow
                .user_id
                .ok_or_else(|| report!(DeviceFlowError::NotFound))?;
            let token_name = flow.client_name.clone().unwrap_or_else(|| "cli".into());

            // Atomic conditional delete; matches the old `consume` HA-safe pattern.
            let result = PendingDeviceFlow::delete_many()
                .filter(pending_device_flow::Column::Id.eq(flow.id))
                .filter(
                    pending_device_flow::Column::Status
                        .eq(DeviceAuthStatus::Authorized.as_str()),
                )
                .exec(&txn)
                .await
                .context_to()?;
            if result.rows_affected == 0 {
                txn.commit().await.context_to()?;
                return Ok(PollOutcome::Unknown);
            }

            let token =
                issue_access_token(&self.db, user_id, token_name.clone()).await?;
            txn.commit().await.context_to()?;
            Ok(PollOutcome::Authorized { token, token_name })
        }
        DeviceAuthStatus::Denied => {
            txn.commit().await.context_to()?;
            Ok(PollOutcome::Denied)
        }
        DeviceAuthStatus::Expired => {
            txn.commit().await.context_to()?;
            Ok(PollOutcome::Expired)
        }
        DeviceAuthStatus::Pending => {
            let interval = flow.interval;
            let too_fast = matches!(flow.last_polled_at, Some(prev) if (now - prev).whole_seconds() < interval as i64);

            if too_fast {
                let bumped = interval.saturating_add(POLL_INTERVAL_BUMP_SECONDS);
                PendingDeviceFlow::update_many()
                    .col_expr(
                        pending_device_flow::Column::Interval,
                        sea_orm::sea_query::Expr::value(bumped),
                    )
                    .col_expr(
                        pending_device_flow::Column::LastPolledAt,
                        sea_orm::sea_query::Expr::value(now),
                    )
                    .filter(pending_device_flow::Column::Id.eq(flow.id))
                    .exec(&txn)
                    .await
                    .context_to()?;
                txn.commit().await.context_to()?;
                Ok(PollOutcome::SlowDown {
                    bumped_interval: bumped,
                })
            } else {
                PendingDeviceFlow::update_many()
                    .col_expr(
                        pending_device_flow::Column::LastPolledAt,
                        sea_orm::sea_query::Expr::value(now),
                    )
                    .filter(pending_device_flow::Column::Id.eq(flow.id))
                    .exec(&txn)
                    .await
                    .context_to()?;
                txn.commit().await.context_to()?;
                Ok(PollOutcome::Pending)
            }
        }
        _ => {
            tracing::warn!(status = ?flow.status, "device flow has unexpected status");
            txn.commit().await.context_to()?;
            Ok(PollOutcome::Unknown)
        }
    }
}
```

> **Note on `begin_with_options`.** Check the SeaORM version pinned in the workspace `Cargo.toml`. If `begin_with_options(TransactionOptions { sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate), .. })` is available, use it (matches CLAUDE.md guidance verbatim). Otherwise use `begin_with_config` as shown and pin the SQLite-specific pragma via `txn.execute_unprepared("PRAGMA ...")` before the read. The point is: every read-then-write under `poll` must serialise against concurrent writers; the existing project standard is `BEGIN IMMEDIATE`.

- [ ] **Step 3: Implement `deny`**

Inside `impl DeviceFlowStore`, add:

```rust
/// Deny a pending device flow. RFC 8628 access_denied path.
///
/// Atomic update: `status = 'pending' → 'denied'`, stored on `denied_by`.
/// Concurrent approve/deny races resolve at this CAS; the loser returns
/// `DeviceFlowError::AlreadyAuthorized` (same shape as `approve` losers).
pub async fn deny(&self, user_code: &str, denied_by: Uuid) -> Result<()> {
    let normalized = user_code.replace('-', "").to_uppercase();
    let now = OffsetDateTime::now_utc();

    let flow = PendingDeviceFlow::find()
        .filter(pending_device_flow::Column::UserCode.eq(&normalized))
        .one(&self.db)
        .await
        .context_to()?
        .ok_or_else(|| report!(DeviceFlowError::NotFound))?;

    if flow.expires_at <= now {
        bail!(DeviceFlowError::NotFound);
    }
    if flow.status != DeviceAuthStatus::Pending {
        bail!(DeviceFlowError::AlreadyAuthorized);
    }

    let result = PendingDeviceFlow::update_many()
        .col_expr(
            pending_device_flow::Column::Status,
            sea_orm::sea_query::Expr::value(DeviceAuthStatus::Denied.as_str()),
        )
        .col_expr(
            pending_device_flow::Column::DeniedBy,
            sea_orm::sea_query::Expr::value(denied_by),
        )
        .filter(pending_device_flow::Column::Id.eq(flow.id))
        .filter(pending_device_flow::Column::Status.eq(DeviceAuthStatus::Pending.as_str()))
        .filter(pending_device_flow::Column::ExpiresAt.gt(now))
        .exec(&self.db)
        .await
        .context_to()?;
    if result.rows_affected == 0 {
        bail!(DeviceFlowError::AlreadyAuthorized);
    }
    Ok(())
}
```

- [ ] **Step 4: Delete the dead methods**

Delete the entire `consume` method (was lines 173–205) and the entire `get_device_code_hash_by_user_code` method (was lines 211–221). Delete the old `get_status` method (was lines 86–109) — `poll` returns the equivalent information through `PollOutcome` plus the `lookup` route handles the UI-facing case.

If any of the existing tests in `mod tests` directly call `consume`/`get_status` (they do: `test_consume_one_time_use`, `test_consume_pending_fails`, `test_status_pending`, `test_approve_and_status`, `test_cleanup_expired`, `test_expired_flow_returns_expired_status`, `test_not_found`), rewrite them against `poll`. Examples of the rewrites land in Task 7; for this step, just `#[ignore]` the call sites that are about to be replaced so the file compiles:

```rust
#[tokio::test]
#[ignore = "replaced by poll-based tests in task 7"]
async fn test_status_pending() {}
```

This minimises commit size; Task 7 replaces the ignored stubs with the real new test set.

- [ ] **Step 5: Compile and confirm**

Run: `cargo check -p uptrakit-web-api-auth --all-features && cargo test -p uptrakit-web-api-auth device_flow -- --nocapture`
Expected: clean compile; the ignored tests are skipped; the remaining tests (e.g. `test_approve_already_authorized`, `test_create_flow`, `test_approve_normalizes_code`, `test_user_code_format`) still pass.

- [ ] **Step 6: Commit**

```bash
git add crates/ui/web-api-auth/src/auth/device_flow.rs
git commit -m "feat(web-api-auth): add poll() and deny() under BEGIN IMMEDIATE, absorb consume"
```

---

## Task 7: New store-level tests for `poll` + `deny`

**Files:**

- Modify: `crates/ui/web-api-auth/src/auth/device_flow.rs` (test module only).

Adds the exact test names locked by the spec's "Testing plan" section. All tests inject `now` as a parameter to `poll`; none use `tokio::time::advance`.

- [ ] **Step 1: Replace the test module's contents**

Replace the ignored stubs added in Task 6 with the real tests:

```rust
#[tokio::test]
async fn slow_down_when_polled_too_fast() {
    let db = test_db().await;
    let store = DeviceFlowStore::new(db);
    let (device_code, _) = store.create(None, None).await.unwrap();
    let t0 = OffsetDateTime::now_utc();

    let first = store.poll(&device_code, t0).await.unwrap();
    assert!(matches!(first, PollOutcome::Pending));

    let t1 = t0 + time::Duration::seconds(2);
    let second = store.poll(&device_code, t1).await.unwrap();
    assert!(matches!(second, PollOutcome::SlowDown { .. }));
}

#[tokio::test]
async fn slow_down_returns_bumped_interval() {
    let db = test_db().await;
    let store = DeviceFlowStore::new(db);
    let (device_code, _) = store.create(None, None).await.unwrap();
    let t0 = OffsetDateTime::now_utc();
    let _ = store.poll(&device_code, t0).await.unwrap();

    let t1 = t0 + time::Duration::seconds(1);
    let outcome = store.poll(&device_code, t1).await.unwrap();
    assert!(
        matches!(outcome, PollOutcome::SlowDown { bumped_interval } if bumped_interval == 10),
        "expected bumped_interval = 10, got {outcome:?}"
    );

    // Another fast poll — should bump again to 15.
    let t2 = t1 + time::Duration::seconds(1);
    let outcome = store.poll(&device_code, t2).await.unwrap();
    assert!(
        matches!(outcome, PollOutcome::SlowDown { bumped_interval } if bumped_interval == 15),
        "expected bumped_interval = 15, got {outcome:?}"
    );
}

#[tokio::test]
async fn last_polled_at_updates_on_each_poll() {
    let db = test_db().await;
    let store = DeviceFlowStore::new(db);
    let (device_code, _) = store.create(None, None).await.unwrap();
    let t0 = OffsetDateTime::now_utc();
    store.poll(&device_code, t0).await.unwrap();
    let t1 = t0 + time::Duration::seconds(60);
    store.poll(&device_code, t1).await.unwrap();
    // Inspect via a direct entity read.
    let hash = hash_token(&device_code);
    let flow = PendingDeviceFlow::find()
        .filter(pending_device_flow::Column::DeviceCodeHash.eq(&hash))
        .one(&store.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(flow.last_polled_at, Some(t1));
}

#[tokio::test]
async fn unknown_device_code_returns_expired_token() {
    let db = test_db().await;
    let store = DeviceFlowStore::new(db);
    let outcome = store
        .poll("not-a-known-code", OffsetDateTime::now_utc())
        .await
        .unwrap();
    assert!(matches!(outcome, PollOutcome::Unknown));
}

#[tokio::test]
async fn malformed_device_code_returns_invalid_grant() {
    let db = test_db().await;
    let store = DeviceFlowStore::new(db);
    let outcome = store.poll("", OffsetDateTime::now_utc()).await.unwrap();
    assert!(matches!(outcome, PollOutcome::MalformedDeviceCode));
}

#[tokio::test]
async fn deny_marks_flow_denied_and_sets_denied_by() {
    let db = test_db().await;
    let store = DeviceFlowStore::new(db);
    let (_device_code, user_code) = store.create(None, None).await.unwrap();
    let denier = Uuid::now_v7();

    store.deny(&user_code, denier).await.unwrap();

    let normalized = user_code.replace('-', "").to_uppercase();
    let flow = PendingDeviceFlow::find()
        .filter(pending_device_flow::Column::UserCode.eq(&normalized))
        .one(&store.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(flow.status, DeviceAuthStatus::Denied);
    assert_eq!(flow.denied_by, Some(denier));
    assert_eq!(flow.user_id, None);
}

#[tokio::test]
async fn poll_after_deny_returns_access_denied() {
    let db = test_db().await;
    let store = DeviceFlowStore::new(db);
    let (device_code, user_code) = store.create(None, None).await.unwrap();
    let denier = Uuid::now_v7();
    store.deny(&user_code, denier).await.unwrap();

    let outcome = store
        .poll(&device_code, OffsetDateTime::now_utc())
        .await
        .unwrap();
    assert!(matches!(outcome, PollOutcome::Denied));
}

#[tokio::test]
async fn concurrent_poll_does_not_double_consume() {
    let db = test_db().await;
    let store = DeviceFlowStore::new(db);
    let (device_code, user_code) = store.create(None, None).await.unwrap();
    let user_id = Uuid::now_v7();
    store.approve(&user_code, user_id).await.unwrap();
    let now = OffsetDateTime::now_utc();

    let store2 = store.clone();
    let dc2 = device_code.clone();
    let (a, b) = tokio::join!(
        store.poll(&device_code, now),
        store2.poll(&dc2, now),
    );

    let outcomes = [a.unwrap(), b.unwrap()];
    let authorized_count = outcomes
        .iter()
        .filter(|o| matches!(o, PollOutcome::Authorized { .. }))
        .count();
    let unknown_count = outcomes
        .iter()
        .filter(|o| matches!(o, PollOutcome::Unknown))
        .count();
    assert_eq!(authorized_count, 1, "exactly one authorized: {outcomes:?}");
    assert_eq!(unknown_count, 1, "exactly one unknown: {outcomes:?}");
}

#[tokio::test]
async fn concurrent_approve_and_deny_resolves_atomically() {
    let db = test_db().await;
    let store = DeviceFlowStore::new(db);
    let (_device_code, user_code) = store.create(None, None).await.unwrap();
    let approver = Uuid::now_v7();
    let denier = Uuid::now_v7();

    let s1 = store.clone();
    let s2 = store.clone();
    let uc1 = user_code.clone();
    let uc2 = user_code.clone();
    let (a, b) = tokio::join!(
        s1.approve(&uc1, approver),
        s2.deny(&uc2, denier),
    );

    let winners = [a.is_ok(), b.is_ok()];
    assert_eq!(winners.iter().filter(|x| **x).count(), 1, "exactly one wins");
}
```

- [ ] **Step 2: Run the new tests**

Run: `cargo test -p uptrakit-web-api-auth device_flow -- --nocapture`
Expected: all new tests pass.

- [ ] **Step 3: Commit**

```bash
git add crates/ui/web-api-auth/src/auth/device_flow.rs
git commit -m "test(web-api-auth): cover poll/deny under BEGIN IMMEDIATE + concurrency"
```

---

## Task 8: New OAuth routes module

**Files:**

- Create: `crates/ui/web-api/src/routes/oauth/mod.rs`
- Create: `crates/ui/web-api/src/routes/oauth/device_authorization.rs`
- Create: `crates/ui/web-api/src/routes/oauth/token.rs`
- Create: `crates/ui/web-api/src/routes/oauth/metadata.rs`
- Modify: `crates/ui/web-api/src/routes/mod.rs`

Three new handlers; all carry `#[utoipa::path]`. Audit emission uses the existing constants. The `token.rs` handler parses `grant_type` as a `String` and dispatches manually so the serde-derive rejection path can't mask `unsupported_grant_type`.

Read the existing `crates/ui/web-api/src/routes/device_auth.rs` (specifically `device_auth_start` lines 90-170 and `device_auth_poll` lines 173-280) before writing — that's the reference for: how to derive `verification_uri`/`verification_uri_complete` from the `ExternalBaseUrl` extractor, how to emit `AUTH_DEVICE_START` audit, error-response helper invocation. The new handlers carry the same patterns but with the new wire-typed bodies.

- [ ] **Step 1: Create `mod.rs`**

Create `crates/ui/web-api/src/routes/oauth/mod.rs`:

```rust
//! OAuth 2.0 endpoints (RFC 8628 device grant + RFC 8414 metadata).
//!
//! See `docs/superpowers/specs/2026-05-12-rfc8628-device-auth-design.md`.

pub mod device_authorization;
pub mod metadata;
pub mod token;
```

- [ ] **Step 2: Create `device_authorization.rs`**

Create `crates/ui/web-api/src/routes/oauth/device_authorization.rs` with a handler that:

1. Extracts `Form<DeviceAuthorizationRequest>`.
2. Calls `req.validate()` → on Err returns 400 + `OAuthErrorCode::InvalidRequest` (with `error_description = msg`).
3. Calls `validate_client_id(&req.client_id)` → on Err returns 400 + `OAuthErrorCode::InvalidClient`.
4. Calls `state.auth.device_flow_store.create(req.client_name.clone(), req.scope.clone())`.
5. Computes `verification_uri` and `verification_uri_complete` from the same `ExternalBaseUrl` chain `device_auth_start` uses today.
6. Emits `AUTH_DEVICE_START` audit (system actor) with `details.has_client_name`, `details.scope`.
7. Returns HTTP 200 + JSON `DeviceAuthorizationResponse { device_code, user_code, verification_uri, verification_uri_complete, expires_in: 600, interval: 5 }`.

Use this handler skeleton as the template. The audit-emission and external-URL-resolution helpers (`emit_device_auth_start_audit`, `resolve_external_base_url`) port over verbatim from the existing `device_auth_start` handler in `crates/ui/web-api/src/routes/device_auth.rs:90-170` — read that handler before writing this one and lift the helpers into a shared submodule rather than re-implementing them.

```rust
use std::sync::Arc;

use axum::Extension;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Json, Response};
use axum::Form;
use uptrakit_web_api_types::oauth::{
    DeviceAuthorizationRequest, DeviceAuthorizationResponse, OAuthErrorCode,
};
use uptrakit_web_api_types::Validate;
use uptrakit_web_api_auth::auth::device_flow::validate_client_id;

use crate::app_state::AppState;
use crate::error_response::oauth_error_response;
use crate::extract::ExternalBaseUrl;

const DEVICE_CODE_TTL_SECONDS: u64 = 600;
const DEFAULT_INTERVAL: i32 = 5;

/// RFC 8628 §3.1 device-authorization request.
#[utoipa::path(
    post,
    path = "/api/v1/oauth/device_authorization",
    request_body(content = DeviceAuthorizationRequest, content_type = "application/x-www-form-urlencoded"),
    responses(
        (status = 200, description = "Device authorization started", body = DeviceAuthorizationResponse),
        (status = 400, description = "Invalid request, invalid_client, or invalid_scope")
    ),
    tag = "OAuth"
)]
#[tracing::instrument(skip_all)]
pub async fn device_authorization(
    State(state): State<Arc<AppState>>,
    external_base_url: Option<Extension<ExternalBaseUrl>>,
    headers: HeaderMap,
    Form(req): Form<DeviceAuthorizationRequest>,
) -> Response {
    if let Err(msg) = req.validate() {
        return oauth_error_response(StatusCode::BAD_REQUEST, OAuthErrorCode::InvalidRequest, Some(msg), None);
    }
    if let Err(code) = validate_client_id(&req.client_id) {
        return oauth_error_response(StatusCode::BAD_REQUEST, code, None, None);
    }

    let (device_code, user_code) = match state
        .auth
        .device_flow_store
        .create(req.client_name.clone(), req.scope.clone())
        .await
    {
        Ok(pair) => pair,
        Err(e) => {
            tracing::error!("device flow create failed: {e}");
            return oauth_error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                OAuthErrorCode::InvalidRequest,
                Some("internal error".into()),
                None,
            );
        }
    };

    // Resolve external base URL — match the chain used in the existing
    // device_auth_start handler (ExternalBaseUrl extractor, then Origin,
    // then Host header).
    let base = resolve_external_base_url(external_base_url, &headers);
    let verification_uri = format!("{base}/device");
    let verification_uri_complete = format!("{base}/device?user_code={user_code}");

    // Audit (reuse the existing system-audit helper from device_auth.rs —
    // either move it into a shared module or inline the same builder call here).
    emit_device_auth_start_audit(
        &state,
        &device_code,
        req.client_name.is_some(),
        req.scope.as_deref(),
    );

    let body = DeviceAuthorizationResponse {
        device_code,
        user_code,
        verification_uri,
        verification_uri_complete,
        expires_in: DEVICE_CODE_TTL_SECONDS,
        interval: DEFAULT_INTERVAL,
    };
    (StatusCode::OK, Json(body)).into_response()
}
```

For the helpers (`resolve_external_base_url`, `emit_device_auth_start_audit`, `oauth_error_response`), copy-paste the equivalent logic from the existing `routes/device_auth.rs` (start handler) and rename. Place `oauth_error_response` in `crates/ui/web-api/src/error_response.rs` next to the existing `error_response` fn:

```rust
pub fn oauth_error_response(
    status: http::StatusCode,
    error: uptrakit_web_api_types::oauth::OAuthErrorCode,
    description: Option<String>,
    interval: Option<i32>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let body = uptrakit_web_api_types::oauth::OAuthErrorResponse {
        error,
        error_description: description,
        interval,
    };
    (status, axum::Json(body)).into_response()
}
```

- [ ] **Step 3: Create `token.rs`**

Create `crates/ui/web-api/src/routes/oauth/token.rs`:

```rust
use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use axum::Form;
use time::OffsetDateTime;
use uptrakit_web_api_auth::auth::device_flow::{validate_client_id, PollOutcome};
use uptrakit_web_api_types::oauth::{
    OAuthErrorCode, OAuthTokenRequest, OAuthTokenResponse,
};
use uptrakit_web_api_types::Validate;

use crate::app_state::AppState;
use crate::error_response::oauth_error_response;

const DEVICE_CODE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";

/// RFC 6749 §3.2 / RFC 8628 §3.4 token endpoint.
#[utoipa::path(
    post,
    path = "/api/v1/oauth/token",
    request_body(content = OAuthTokenRequest, content_type = "application/x-www-form-urlencoded"),
    responses(
        (status = 200, description = "Token granted", body = OAuthTokenResponse),
        (status = 400, description = "OAuth error per RFC 6749 §5.2 / RFC 8628 §3.5")
    ),
    tag = "OAuth"
)]
#[tracing::instrument(skip_all)]
pub async fn token(
    State(state): State<Arc<AppState>>,
    Form(req): Form<OAuthTokenRequest>,
) -> Response {
    if let Err(msg) = req.validate() {
        return oauth_error_response(StatusCode::BAD_REQUEST, OAuthErrorCode::InvalidRequest, Some(msg), None);
    }

    match req.grant_type.as_str() {
        DEVICE_CODE_GRANT => device_code_grant(state, req).await,
        _ => oauth_error_response(
            StatusCode::BAD_REQUEST,
            OAuthErrorCode::UnsupportedGrantType,
            None,
            None,
        ),
    }
}

async fn device_code_grant(state: Arc<AppState>, req: OAuthTokenRequest) -> Response {
    let device_code = match &req.device_code {
        Some(s) if !s.trim().is_empty() => s.clone(),
        _ => {
            return oauth_error_response(
                StatusCode::BAD_REQUEST,
                OAuthErrorCode::InvalidRequest,
                Some("device_code is required".into()),
                None,
            );
        }
    };

    let client_id = match &req.client_id {
        Some(s) if !s.trim().is_empty() => s,
        _ => {
            return oauth_error_response(
                StatusCode::BAD_REQUEST,
                OAuthErrorCode::InvalidRequest,
                Some("client_id is required".into()),
                None,
            );
        }
    };
    if let Err(code) = validate_client_id(client_id) {
        return oauth_error_response(StatusCode::BAD_REQUEST, code, None, None);
    }

    let outcome = match state
        .auth
        .device_flow_store
        .poll(&device_code, OffsetDateTime::now_utc())
        .await
    {
        Ok(o) => o,
        Err(e) => {
            tracing::error!("device flow poll failed: {e}");
            return oauth_error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                OAuthErrorCode::InvalidRequest,
                Some("internal error".into()),
                None,
            );
        }
    };

    emit_poll_audit(&state, &device_code, &outcome);

    match outcome {
        PollOutcome::Authorized { token, .. } => {
            let body = OAuthTokenResponse {
                access_token: token.expose_secret().to_string(),
                token_type: "Bearer".into(),
                expires_in: None,
                refresh_token: None,
                scope: None,
            };
            (StatusCode::OK, Json(body)).into_response()
        }
        PollOutcome::Pending => oauth_error_response(
            StatusCode::BAD_REQUEST,
            OAuthErrorCode::AuthorizationPending,
            None,
            None,
        ),
        PollOutcome::SlowDown { bumped_interval } => oauth_error_response(
            StatusCode::BAD_REQUEST,
            OAuthErrorCode::SlowDown,
            None,
            Some(bumped_interval),
        ),
        PollOutcome::Denied => oauth_error_response(
            StatusCode::BAD_REQUEST,
            OAuthErrorCode::AccessDenied,
            None,
            None,
        ),
        PollOutcome::Expired | PollOutcome::Unknown => oauth_error_response(
            StatusCode::BAD_REQUEST,
            OAuthErrorCode::ExpiredToken,
            None,
            None,
        ),
        PollOutcome::MalformedDeviceCode => oauth_error_response(
            StatusCode::BAD_REQUEST,
            OAuthErrorCode::InvalidGrant,
            None,
            None,
        ),
    }
}

fn emit_poll_audit(state: &Arc<AppState>, device_code: &str, outcome: &PollOutcome) {
    use uptrakit_audit_log::{AuditActionType, AuditOutcome};
    // Inline a system-audit emission matching the existing
    // emit_device_auth_system_audit helper in routes/device_auth.rs.
    // Map outcomes:
    //   Authorized -> AuditOutcome::Success
    //   SlowDown   -> AuditOutcome::Failed (details: { reason_code: "slow_down" })
    //   Denied     -> AuditOutcome::Denied
    //   _          -> AuditOutcome::Failed
    // ... (see the device_auth.rs helper for the exact builder call)
}
```

(`emit_poll_audit` is a thin wrapper around the existing `emit_device_auth_system_audit` helper; either lift the helper into a shared module under `crates/ui/web-api/src/routes/device_audit_helpers.rs` or re-implement inline matching it exactly. Both options are acceptable — prefer lifting if it can be done as a 10-line refactor.)

- [ ] **Step 4: Create `metadata.rs`**

Create `crates/ui/web-api/src/routes/oauth/metadata.rs`:

```rust
use std::sync::Arc;

use axum::Extension;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Json, Response};
use uptrakit_web_api_types::oauth::OAuthAuthorizationServerMetadata;

use crate::app_state::AppState;
use crate::extract::ExternalBaseUrl;

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
    let body = OAuthAuthorizationServerMetadata {
        issuer: base.clone(),
        device_authorization_endpoint: format!("{base}/api/v1/oauth/device_authorization"),
        token_endpoint: format!("{base}/api/v1/oauth/token"),
        grant_types_supported: vec![DEVICE_CODE_GRANT.to_string()],
        response_types_supported: vec![],
        token_endpoint_auth_methods_supported: vec!["none".into()],
        code_challenge_methods_supported: vec![],
    };
    (StatusCode::OK, Json(body)).into_response()
}

// re-use the same resolve_external_base_url helper as device_authorization.rs
```

(Lift `resolve_external_base_url` into a shared helper inside the oauth module if more than one file uses it.)

- [ ] **Step 5: Wire into `routes/mod.rs`**

Edit `crates/ui/web-api/src/routes/mod.rs` and add:

```rust
pub mod oauth;
```

- [ ] **Step 6: Compile**

Run: `cargo check -p uptrakit-web-api --all-features`
Expected: clean compile. (Routes are not yet mounted in the router — that's Task 11.)

- [ ] **Step 7: Commit**

```bash
git add crates/ui/web-api/src/routes/oauth/ \
        crates/ui/web-api/src/routes/mod.rs \
        crates/ui/web-api/src/error_response.rs
git commit -m "feat(web-api): add OAuth 2.0 routes (device_authorization, token, metadata)"
```

---

## Task 9: Add `/auth/device/deny` and `/auth/device/lookup` routes

**Files:**

- Modify: `crates/ui/web-api/src/routes/device_auth.rs`

`device_auth_deny` is a near-clone of `device_auth_approve` calling the new `deny()` store method. `device_auth_lookup` is a new `Query<DeviceAuthLookupQuery>` handler.

- [ ] **Step 1: Add the `deny` handler**

In `crates/ui/web-api/src/routes/device_auth.rs`, add (next to `device_auth_approve`):

```rust
/// Operator denies a pending device authorization.
#[utoipa::path(
    post,
    path = "/api/v1/auth/device/deny",
    request_body = DeviceAuthDenyRequest,
    responses(
        (status = 200, description = "Device denied", body = DeviceAuthDenyResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Device flow not found"),
        (status = 409, description = "Already authorized or denied"),
    ),
    tag = "Authentication",
    extensions(("x-required-permission" = json!("view_services"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn device_auth_deny(
    State(state): State<Arc<AppState>>,
    CanViewServices(auth_user): CanViewServices,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Json(req): Json<DeviceAuthDenyRequest>,
) -> Result<impl IntoResponse, ApiError> {
    if let Err(msg) = req.validate() {
        return Err(ApiError::BadRequest(msg));
    }
    let api_token_id = api_token_id.map(|e| e.0);
    let normalized = req.user_code.replace('-', "").to_uppercase();
    let device_flow_id = hash_token(&normalized);

    if let Err(error) = state
        .auth
        .device_flow_store
        .deny(&normalized, auth_user.user_id)
        .await
    {
        let (action_type, outcome, reason_code) = error.current_context().approval_classification();
        emit_device_auth_decision_audit(
            &state,
            &auth_user,
            api_token_id,
            action_type,
            device_flow_id,
            outcome,
            serde_json::json!({ "reason_code": reason_code, "decision": "deny" }),
        );
        return Err(error.into());
    }

    emit_device_auth_decision_audit(
        &state,
        &auth_user,
        api_token_id,
        uptrakit_audit_log::AuditActionType::AUTH_DEVICE_DENY,
        device_flow_id,
        uptrakit_audit_log::AuditOutcome::Success,
        serde_json::json!({ "decision": "deny" }),
    );
    Ok(Json(DeviceAuthDenyResponse {
        message: "Device authorization denied.".into(),
    }))
}
```

- [ ] **Step 2: Add the `lookup` handler**

```rust
/// Look up `client_name` + `expires_at` for a pending device flow by user_code.
#[utoipa::path(
    get,
    path = "/api/v1/auth/device/lookup",
    params(DeviceAuthLookupQuery),
    responses(
        (status = 200, description = "Lookup ok", body = DeviceAuthLookupResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Not found"),
    ),
    tag = "Authentication",
    extensions(("x-required-permission" = json!("view_services"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn device_auth_lookup(
    State(state): State<Arc<AppState>>,
    _auth_user: CanViewServices,
    Query(query): Query<DeviceAuthLookupQuery>,
) -> Result<Json<DeviceAuthLookupResponse>, ApiError> {
    if let Err(msg) = query.validate() {
        return Err(ApiError::BadRequest(msg));
    }
    let normalized = query.user_code.replace('-', "").to_uppercase();
    let flow = state
        .auth
        .device_flow_store
        .lookup_by_user_code(&normalized)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(|| ApiError::NotFound("device flow not found".into()))?;

    Ok(Json(DeviceAuthLookupResponse {
        client_name: flow.client_name,
        expires_at: flow.expires_at,
    }))
}
```

`lookup_by_user_code` is a new read-only method on `DeviceFlowStore`. Add it to `device_flow.rs` alongside the other methods:

```rust
pub async fn lookup_by_user_code(
    &self,
    user_code: &str,
) -> Result<Option<pending_device_flow::Model>> {
    let normalized = user_code.replace('-', "").to_uppercase();
    let flow = PendingDeviceFlow::find()
        .filter(pending_device_flow::Column::UserCode.eq(&normalized))
        .one(&self.db)
        .await
        .context_to()?;
    Ok(flow)
}
```

Add the necessary imports at the top of `device_auth.rs`:

```rust
use axum::extract::Query;
use uptrakit_web_api_types::device_auth::{
    DeviceAuthDenyRequest, DeviceAuthDenyResponse,
};
use uptrakit_web_api_types::oauth::{DeviceAuthLookupQuery, DeviceAuthLookupResponse};
```

- [ ] **Step 3: Compile**

Run: `cargo check -p uptrakit-web-api --all-features`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add crates/ui/web-api/src/routes/device_auth.rs \
        crates/ui/web-api-auth/src/auth/device_flow.rs
git commit -m "feat(web-api): add /auth/device/{deny,lookup} routes + lookup store method"
```

---

## Task 10: Delete legacy device-auth routes + SSE broadcaster

**Files:**

- Modify: `crates/ui/web-api/src/routes/device_auth.rs`
- Delete: `crates/ui/web-api/src/device_flow_broadcaster.rs`
- Modify: `crates/ui/web-api/src/app_state.rs`
- Modify: `crates/ui/web-api/src/lib.rs`

The compilation failures from this delete force every consumer to update.

- [ ] **Step 1: Delete the dead handlers in `routes/device_auth.rs`**

Delete `device_auth_start`, `device_auth_poll`, `device_auth_stream`, and any helpers exclusively used by them. Also delete the `notify_status_changed` call inside `device_auth_approve` (lines ~328-334 in the existing file) and the `create_channel` call inside the now-deleted `device_auth_start`.

Keep: `device_auth_approve`, `device_auth_deny` (Task 9), `device_auth_lookup` (Task 9), `emit_device_auth_decision_audit`, `emit_device_auth_system_audit`.

Delete imports that are now unused (`DeviceFlowEvent`, `DeviceAuthStartRequest`, `DeviceAuthStartResponse`, `DeviceAuthPollRequest`, `DeviceAuthPollResponse`, etc.).

- [ ] **Step 2: Delete the broadcaster file**

```bash
git rm crates/ui/web-api/src/device_flow_broadcaster.rs
```

- [ ] **Step 3: Remove the field from `BroadcastState`**

In `crates/ui/web-api/src/app_state.rs`:

- Delete line 110 (`pub device_flow_broadcaster: crate::device_flow_broadcaster::DeviceFlowBroadcaster,`).
- Delete line 344 (`device_flow_broadcaster: Option<...>,`).
- Delete line 401 (`device_flow_broadcaster: None,`).
- Delete line 801 (`device_flow_broadcaster: self.device_flow_broadcaster.unwrap_or_default(),`).
- Delete any `with_device_flow_broadcaster` builder method.

- [ ] **Step 4: Remove the module declaration and test-harness wiring**

In `crates/ui/web-api/src/lib.rs`:

- Delete `pub mod device_flow_broadcaster;` (line 11).
- Delete the `device_flow_broadcaster` field initialisation inside `BroadcastState { ... }` literals (lines 226-227 area).

Search for any remaining references and remove them:

```bash
grep -rn "device_flow_broadcaster\|DeviceFlowBroadcaster" crates/ui/web-api/ --include='*.rs'
```

Expected: no matches after cleanup. If `test_harness/mod.rs` has its own construction, drop the field there too.

- [ ] **Step 5: Compile**

Run: `cargo check -p uptrakit-web-api --all-features`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add -A crates/ui/web-api/
git commit -m "chore(web-api): delete legacy device_auth routes + SSE broadcaster (replaced by /oauth/*)"
```

---

## Task 11: Router wiring

**Files:**

- Modify: `crates/ui/web-api/src/router.rs`

Mount the new routes; unmount the deleted ones. `/.well-known/oauth-authorization-server` mounts outside `/api/v1` (RFC 8615 reserves the `/.well-known` prefix).

- [ ] **Step 1: Inspect the router**

Run: `grep -n "device" crates/ui/web-api/src/router.rs | head -20`
Note every line that references the old `/api/v1/auth/device*` paths.

- [ ] **Step 2: Replace the old mounts with the new ones**

Where the old code looked like:

```rust
.route("/api/v1/auth/device", post(device_auth_start))
.route("/api/v1/auth/device/poll", post(device_auth_poll))
.route("/api/v1/auth/device/stream", get(device_auth_stream))
.route("/api/v1/auth/device/approve", post(device_auth_approve))
```

Replace with:

```rust
.route(
    "/api/v1/oauth/device_authorization",
    post(crate::routes::oauth::device_authorization::device_authorization),
)
.route(
    "/api/v1/oauth/token",
    post(crate::routes::oauth::token::token),
)
.route("/api/v1/auth/device/approve", post(device_auth_approve))
.route("/api/v1/auth/device/deny", post(device_auth_deny))
.route("/api/v1/auth/device/lookup", get(device_auth_lookup))
```

Mount the discovery endpoint outside `/api/v1` (i.e. at the top-level `Router` before any nesting that adds the `/api/v1` prefix):

```rust
.route(
    "/.well-known/oauth-authorization-server",
    get(crate::routes::oauth::metadata::metadata),
)
```

Adapt the exact `Router` builder shape to match the existing file. If routes are split across separate `Router::new()` blocks per logical area, place the new OAuth routes in their own block (e.g. `oauth_router()`) and `.merge(...)` it into the main router.

- [ ] **Step 3: Compile**

Run: `cargo check -p uptrakit-web-api --all-features && cargo build -p uptrakit-web-api --all-features`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add crates/ui/web-api/src/router.rs
git commit -m "feat(web-api): mount /oauth/* + /.well-known/oauth-authorization-server routes"
```

---

## Task 12: Rate-limit middleware update

**Files:**

- Modify: `crates/ui/web-api/src/middleware/rate_limit.rs`

Swap the `RATE_LIMITS` entries to the new paths and rename / update the existing tests.

- [ ] **Step 1: Update the `RATE_LIMITS` map**

In `crates/ui/web-api/src/middleware/rate_limit.rs`, replace the existing device-related entries (the two starting at lines 50 and 58):

```rust
(
    "/api/v1/oauth/device_authorization",
    EndpointRateLimit {
        max_requests: 10,
        window_secs: 60,
        fail_closed: true,
    },
),
(
    "/api/v1/oauth/token",
    EndpointRateLimit {
        max_requests: 60,
        window_secs: 60,
        fail_closed: true,
    },
),
(
    "/api/v1/auth/device/approve",
    EndpointRateLimit {
        max_requests: 5,
        window_secs: 60,
        fail_closed: true,
    },
),
(
    "/api/v1/auth/device/deny",
    EndpointRateLimit {
        max_requests: 5,
        window_secs: 60,
        fail_closed: true,
    },
),
(
    "/api/v1/auth/device/lookup",
    EndpointRateLimit {
        max_requests: 60,
        window_secs: 60,
        fail_closed: true,
    },
),
```

(Delete the `/api/v1/auth/device` and `/api/v1/auth/device/poll` entries; keep `/approve` as before.)

- [ ] **Step 2: Update tests**

Rewrite the three existing tests in `mod tests`:

```rust
#[test]
fn rate_limited_paths_list() {
    let mut expected = vec![
        "/api/v1/auth/login",
        "/api/v1/auth/register",
        "/api/v1/auth/refresh",
        "/api/v1/oauth/device_authorization",
        "/api/v1/oauth/token",
        "/api/v1/auth/device/approve",
        "/api/v1/auth/device/deny",
        "/api/v1/auth/device/lookup",
    ];
    if cfg!(feature = "oidc") {
        expected.extend_from_slice(&[
            "/api/v1/auth/oidc/exchange",
            "/api/v1/auth/oidc/link",
            "/api/v1/auth/oidc/complete-registration",
        ]);
    }

    for path in &expected {
        assert!(
            RATE_LIMITS.contains_key(path),
            "expected {path} to be rate-limited"
        );
    }

    assert_eq!(
        RATE_LIMITS.len(),
        expected.len(),
        "unexpected extra rate-limited paths"
    );
}

#[test]
fn non_rate_limited_paths() {
    let paths = [
        "/api/v1/auth/logout",
        "/api/v1/auth/me",
        "/healthz",
        "/api/v1/services",
        "/.well-known/oauth-authorization-server",
    ];
    for path in &paths {
        assert!(
            !RATE_LIMITS.contains_key(path),
            "{path} should not be rate-limited"
        );
    }
}

#[test]
fn oauth_token_has_higher_limit() {
    let token_limit = RATE_LIMITS
        .get("/api/v1/oauth/token")
        .expect("token limit");
    let login_limit = RATE_LIMITS.get("/api/v1/auth/login").expect("login limit");
    assert!(
        token_limit.max_requests > login_limit.max_requests,
        "token endpoint must be more permissive than login: token={} login={}",
        token_limit.max_requests,
        login_limit.max_requests,
    );
}
```

- [ ] **Step 3: Run middleware tests**

Run: `cargo test -p uptrakit-web-api middleware::rate_limit -- --nocapture`
Expected: three tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/ui/web-api/src/middleware/rate_limit.rs
git commit -m "feat(web-api): retune rate-limit map for /oauth/* + /auth/device/{deny,lookup}"
```

---

## Task 13: Route-layer tests

**Files:**

- Modify: `crates/ui/web-api/src/routes/oauth/device_authorization.rs` (append `mod tests`).
- Modify: `crates/ui/web-api/src/routes/oauth/token.rs` (append `mod tests`).
- Modify: `crates/ui/web-api/src/routes/oauth/metadata.rs` (append `mod tests`).
- Modify: `crates/ui/web-api/src/routes/device_auth.rs` (append the new deny/lookup tests next to the existing approve tests).

Tests follow the existing `crates/ui/web-api/src/routes/device_auth.rs` test harness shape (look at `device_auth_start_success_writes_audit_event` around line 567 — it shows how to build a test `AppState` and assert on audit emissions).

- [ ] **Step 1: Tests for `oauth/token.rs`**

Add to `crates/ui/web-api/src/routes/oauth/token.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_harness::TestAppState;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    async fn make_router(state: TestAppState) -> axum::Router {
        axum::Router::new()
            .route("/api/v1/oauth/token", axum::routing::post(token))
            .with_state(state.app_state())
    }

    fn form_body(items: &[(&str, &str)]) -> Body {
        Body::from(serde_urlencoded::to_string(items).expect("form encode"))
    }

    #[tokio::test]
    async fn unsupported_grant_type_response() {
        let state = TestAppState::new().await;
        let app = make_router(state).await;
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/oauth/token")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(form_body(&[
                        ("grant_type", "client_credentials"),
                        ("client_id", "uptrakit-cli"),
                    ]))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), 400);
        let body: serde_json::Value = read_json(resp).await;
        assert_eq!(body["error"], "unsupported_grant_type");
    }

    #[tokio::test]
    async fn invalid_grant_when_device_code_unknown() {
        let state = TestAppState::new().await;
        let app = make_router(state).await;
        let resp = app.oneshot(make_token_req(&[
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ("device_code", "never-existed-deadbeef"),
            ("client_id", "uptrakit-cli"),
        ])).await.unwrap();
        assert_eq!(resp.status(), 400);
        let body: serde_json::Value = read_json(resp).await;
        assert_eq!(body["error"], "expired_token");
    }

    #[tokio::test]
    async fn invalid_request_when_missing_fields() {
        let state = TestAppState::new().await;
        let app = make_router(state).await;
        let resp = app.oneshot(make_token_req(&[
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
        ])).await.unwrap();
        assert_eq!(resp.status(), 400);
        let body: serde_json::Value = read_json(resp).await;
        assert_eq!(body["error"], "invalid_request");
        assert!(body["error_description"].as_str().is_some());
    }

    #[tokio::test]
    async fn invalid_client_when_client_id_mismatches() {
        let state = TestAppState::new().await;
        let (device_code, _) = state.app_state().auth.device_flow_store
            .create(None, None).await.unwrap();
        let app = make_router(state).await;
        let resp = app.oneshot(make_token_req(&[
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ("device_code", &device_code),
            ("client_id", "wrong-client"),
        ])).await.unwrap();
        assert_eq!(resp.status(), 400);
        let body: serde_json::Value = read_json(resp).await;
        assert_eq!(body["error"], "invalid_client");
    }

    #[tokio::test]
    async fn authorization_pending_400() {
        let state = TestAppState::new().await;
        let (device_code, _) = state.app_state().auth.device_flow_store
            .create(None, None).await.unwrap();
        let app = make_router(state).await;
        let resp = app.oneshot(make_token_req(&[
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ("device_code", &device_code),
            ("client_id", "uptrakit-cli"),
        ])).await.unwrap();
        assert_eq!(resp.status(), 400);
        let body: serde_json::Value = read_json(resp).await;
        assert_eq!(body["error"], "authorization_pending");
    }

    #[tokio::test]
    async fn slow_down_400_with_interval() {
        let state = TestAppState::new().await;
        let (device_code, _) = state.app_state().auth.device_flow_store
            .create(None, None).await.unwrap();
        let app = make_router(state).await;
        // First poll → pending.
        let _ = app.clone().oneshot(make_token_req(&[
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ("device_code", &device_code),
            ("client_id", "uptrakit-cli"),
        ])).await.unwrap();
        // Second poll immediately → slow_down.
        let resp = app.oneshot(make_token_req(&[
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ("device_code", &device_code),
            ("client_id", "uptrakit-cli"),
        ])).await.unwrap();
        assert_eq!(resp.status(), 400);
        let body: serde_json::Value = read_json(resp).await;
        assert_eq!(body["error"], "slow_down");
        assert_eq!(body["interval"], 10);
    }

    #[tokio::test]
    async fn access_denied_400() {
        let state = TestAppState::new().await;
        let (device_code, user_code) = state.app_state().auth.device_flow_store
            .create(None, None).await.unwrap();
        let denier = uuid::Uuid::now_v7();
        state.app_state().auth.device_flow_store.deny(&user_code, denier).await.unwrap();

        let app = make_router(state).await;
        let resp = app.oneshot(make_token_req(&[
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ("device_code", &device_code),
            ("client_id", "uptrakit-cli"),
        ])).await.unwrap();
        assert_eq!(resp.status(), 400);
        let body: serde_json::Value = read_json(resp).await;
        assert_eq!(body["error"], "access_denied");
    }

    #[tokio::test]
    async fn expired_token_400() {
        // create + expire_flow helper from device_flow.rs (or wait + sleep test).
        // Approach: create a flow, then directly set expires_at to a past time
        // via the entity update, then poll.
        // ... same pattern as test_expired_flow_returns_expired_status.
    }

    #[tokio::test]
    async fn success_returns_bearer_token() {
        let state = TestAppState::new().await;
        let (device_code, user_code) = state.app_state().auth.device_flow_store
            .create(Some("test-client".into()), None).await.unwrap();
        let approver = uuid::Uuid::now_v7();
        state.app_state().auth.device_flow_store.approve(&user_code, approver).await.unwrap();
        let app = make_router(state).await;
        let resp = app.oneshot(make_token_req(&[
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ("device_code", &device_code),
            ("client_id", "uptrakit-cli"),
        ])).await.unwrap();
        assert_eq!(resp.status(), 200);
        let body: serde_json::Value = read_json(resp).await;
        assert_eq!(body["token_type"], "Bearer");
        assert!(body["access_token"].as_str().unwrap_or_default().len() > 8);
        assert!(body.get("expires_in").is_none());
        assert!(body.get("refresh_token").is_none());
        assert!(body.get("scope").is_none());
    }

    #[tokio::test]
    async fn audit_records_slow_down_outcome() {
        // Mirror the existing pattern in routes/device_auth.rs:
        // - Provide a recording audit emitter via TestAppState.
        // - Drive a second poll within `interval`.
        // - Assert AUTH_DEVICE_POLL appears with details.reason_code == "slow_down".
    }

    // Helper functions:
    fn make_token_req(items: &[(&str, &str)]) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri("/api/v1/oauth/token")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(form_body(items))
            .unwrap()
    }
    async fn read_json(resp: axum::response::Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }
}
```

(The exact `TestAppState` shape lives in `crates/ui/web-api/src/test_harness/mod.rs` — read it before writing this test and adapt as needed.)

- [ ] **Step 2: Tests for `oauth/device_authorization.rs`**

Append to `oauth/device_authorization.rs`:

```rust
#[cfg(test)]
mod tests {
    // success_response_shape_matches_rfc:
    //   POST with grant_type/client_id=uptrakit-cli and assert response JSON has
    //   exactly: device_code, user_code, verification_uri,
    //   verification_uri_complete, expires_in, interval.
    // client_id_mismatch_returns_invalid_client.
    // client_name_extension_field_persists_to_audit.
    // verification_uri_complete_contains_user_code (string-suffix check).
    // external_base_url_resolution_unchanged: send X-Forwarded-* and assert
    //   verification_uri honours it.
    // ... (mirror the token.rs structure for the harness)
}
```

- [ ] **Step 3: Tests for `oauth/metadata.rs`**

Append:

```rust
#[cfg(test)]
mod tests {
    // discovery_doc_lists_device_grant_endpoints: full JSON shape match.
    // discovery_doc_no_auth_required: unauthenticated request → 200.
}
```

- [ ] **Step 4: Tests for `device_auth.rs` deny + lookup**

Append next to the existing tests:

```rust
#[cfg(test)]
mod deny_lookup_tests {
    // deny_requires_permission: no auth → 401; CanViewServices missing → 403.
    // deny_emits_audit_event: assert AUTH_DEVICE_DENY with outcome Success.
    // deny_unknown_user_code_returns_not_found.
    // lookup_returns_client_name_and_expiry.
    // lookup_unknown_user_code_returns_404.
    // (mirror the existing test harness shape used by approve tests).
}
```

- [ ] **Step 5: Run the full backend test pass**

Run: `cargo test -p uptrakit-web-api --all-features`
Expected: every new test passes.

- [ ] **Step 6: Commit**

```bash
git add crates/ui/web-api/src/routes/oauth/ crates/ui/web-api/src/routes/device_auth.rs
git commit -m "test(web-api): cover OAuth routes + device deny/lookup with shape and audit assertions"
```

---

## Task 14: ADR 0009

**Files:**

- Create: `docs/adr/0009-oauth-2-device-flow-rfc-compliance.md`

Architectural Decision Record covering: strict RFC 8628 + 8414 wire compliance, minimum-viable token issuance, hard break, four named seams.

- [ ] **Step 1: Write the ADR**

Create `docs/adr/0009-oauth-2-device-flow-rfc-compliance.md`:

```markdown
# 0009 — OAuth 2.0 Device Authorization Grant: strict RFC compliance, minimum-viable issuance

- Status: Accepted
- Date: 2026-05-12
- Spec: `docs/superpowers/specs/2026-05-12-rfc8628-device-auth-design.md`

## Context

uptrakit's CLI uses an OAuth-flavoured device authorization flow today. The wire
shape diverges from RFC 8628 in several ways: poll responses use HTTP 200 with a
custom `status` enum plus HTTP 404 instead of the RFC's HTTP 400 + JSON `error`
field; the start endpoint returns `verification_url` instead of
`verification_uri`; there is no `verification_uri_complete`; no per-flow
`slow_down` cadence enforcement; no operator-driven `access_denied` path; no
`/.well-known` discovery document. The product is self-hosted, single-tenant,
with one known CLI consumer.

## Decision

Refactor the wire to strict RFC 8628 + RFC 8414 conformance in a single hard
break:

- Replace `/api/v1/auth/device{,/poll,/stream}` with the standard OAuth
  endpoints: `POST /api/v1/oauth/device_authorization`, `POST /api/v1/oauth/token`
  (a `grant_type` dispatcher), and `GET /.well-known/oauth-authorization-server`.
- Adopt RFC 6749 §5.1 / §5.2 response shapes: success returns `access_token`/
  `token_type: "Bearer"`; failure returns HTTP 400 with `{"error": <code>}`.
- Add per-flow `slow_down` cadence enforcement and an explicit Operator-driven
  `access_denied` path.
- Drop the SSE stream; clients poll plain at `interval` cadence.
- Keep today's minimum-viable token issuance: indefinite API tokens, no refresh
  token, no scope enforcement, single hardcoded `client_id = "uptrakit-cli"`.
  These deliberate omissions are paired with four named extension seams so future
  features land as targeted refactors rather than redesigns.

## Consequences

Positive:

- Any conformant RFC 8628 client works end-to-end without uptrakit-specific
  knowledge.
- `slow_down` is a per-flow protocol-correct signal; the IP rate limit no longer
  collides with well-behaved clients on shared NAT.
- The `access_denied` path gives Operators a phishing/mis-direction defence —
  active denial instead of "user closes tab and waits for expiry".
- The token endpoint dispatcher is the single integration point for future
  OAuth grants (refresh, password, client credentials).

Negative / accepted trade-offs:

- Hard break: backend + CLI + frontend ship together. There is no
  cross-version-compatible interim.
- Indefinite-lifetime tokens land alongside the new endpoints. A future
  refresh-token migration will require operators to rotate; this is documented
  as Seam 1.

### Four named seams (extension points)

The implementation deliberately localises each anticipated future feature to a
single named function:

1. **Token issuance — Seam 1.** `crates/ui/web-api-auth/src/auth/device_flow.rs`,
   `issue_access_token`. Today mints an indefinite API token. Future: returns
   `TokenPair { access_token, expires_in, refresh_token }` and the
   `OAuthTokenResponse` fields stop being `None`. The `refresh_token` grant arm
   slots into the `/api/v1/oauth/token` dispatcher.

2. **Scope enforcement — Seam 2.** `crates/ui/web-api-auth/src/auth/device_flow.rs`,
   `apply_scope_to_token`. Today a no-op stub. Future: parses the `scope`
   string (RFC 6749 §3.3), maps each scope to a `Permission` subset, and
   attaches the narrowed permission set to the minted token. The `scope`
   parameter is already persisted on `pending_device_flows.scope` and echoed in
   audit; no other call sites change.

3. **Client registry — Seam 3.** `crates/ui/web-api-auth/src/auth/device_flow.rs`,
   `validate_client_id` + `CLIENT_ID` constant. Today validates an exact
   match. Future: replaces the function body with an `oauth_clients` table
   lookup; the constant is deleted. No route handler changes.

4. **Long-poll — Seam 4.** `crates/ui/web-api/src/routes/oauth/token.rs`,
   `device_code_grant` arm. Today returns the current outcome immediately.
   Future: an opt-in `wait` form parameter (capped ≤30s, below typical
   reverse-proxy idle timeouts). When present and the outcome would be
   `authorization_pending`, the handler awaits a `tokio::sync::Notify` keyed
   by `device_code` up to the cap, then re-evaluates. RFC-compliant clients
   that omit `wait` see the existing behaviour.

## Notes

- `CONTEXT.md` is unchanged: RFC 8628 vocabulary is OAuth standard, not
  uptrakit-specific. The existing reservation of the noun "device" for this
  flow continues to hold.
- The implementation plan that lands these changes is split into two PRs to
  keep review tractable: backend (this plan), then client (CLI + frontend +
  openapi-client).
```

- [ ] **Step 2: Lint markdown**

Run: `npx prettier --write docs/adr/0009-oauth-2-device-flow-rfc-compliance.md && npx markdownlint --config .markdownlint.json docs/adr/0009-oauth-2-device-flow-rfc-compliance.md`
Expected: no errors.

- [ ] **Step 3: Commit**

```bash
git add docs/adr/0009-oauth-2-device-flow-rfc-compliance.md
git commit -m "docs(adr): 0009 — OAuth 2.0 device flow RFC compliance + four named seams"
```

---

## Task 15: Quality gates

**Files:**

- None (verification only).

Plan 1 is complete only when every gate passes. Run the full chain.

- [ ] **Step 1: Format + lint + check**

Run, sequentially:

```bash
cargo fmt --all
cargo check --no-default-features --features db-sqlite
cargo check --all-features
cargo clippy --all-targets --no-default-features --features db-sqlite
cargo clippy --all-targets --all-features
```

Expected: each command exits 0 with no warnings.

- [ ] **Step 2: Test**

Run:

```bash
cargo test --all-features
```

Expected: green across the workspace, including the new `device_flow`, `oauth::*`, and `routes::device_auth::*` tests.

- [ ] **Step 3: Dependency / license audit**

Run: `cargo deny check`
Expected: clean. (No new crate dependencies were introduced by this plan; if `cargo deny` reports a violation, fix it before proceeding.)

- [ ] **Step 4: Markdown lint**

Run: `markdownlint --config .markdownlint.json '**/*.md'` (or the workspace's `npx markdownlint ...` equivalent).
Expected: clean.

- [ ] **Step 5: Final state check**

Run: `git log --oneline -20`
Expected: a coherent commit graph through Tasks 1–14, all on the feature branch, with no uncommitted changes outside the spec scope.

- [ ] **Step 6: Plan-completion note**

Plan 1 is complete. Proceed to Plan 2 (CLI + frontend + openapi-client) — see `docs/superpowers/plans/2026-05-12-rfc8628-device-auth-2-client.md`. Plan 2 depends on the routes mounted in Task 11 being live.
