# controller-core Phase 1 — Crate Scaffold + Core State Moves

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement
> this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Create `uptrakit-controller-core` crate and move `DbState`, `Settings`, `AuthState`, and `WorkloadClaimRegistry` from `web-api` into it,
leaving re-export shims so all existing `crate::` paths inside `web-api` continue to compile unchanged.

**Architecture:** New crate at `crates/ui/controller-core/` (picked up automatically by workspace glob `crates/ui/*`). All moved types keep their
public API; constructors become `pub` where previously `pub(crate)`. Re-export shims (`pub use uptrakit_controller_core::…::*`) in the web-api
source files maintain backward compatibility for the rest of the phase. Shims are removed in Phase 2.

**Tech Stack:** Rust 2024 edition; `parking_lot` for locks; `sea-orm` for DB types; `uptrakit-web-api-auth` for auth sub-types
(JwtManager, DeviceFlowStore, etc.); no `axum`, no `uptrakit-web-api`, no `uptrakit-mcp`.

**Standards binding:** `#[non_exhaustive]` on all public structs per coding-standards.md §Struct Extensibility. `parking_lot::Mutex`/`RwLock` only.
`#[expect(..., reason = "...")]` for suppression. `pub(crate)` constructors promoted to `pub` when crossing crate boundaries.

---

## Task 1: Create controller-core crate skeleton

**Files:**

- Create: `crates/ui/controller-core/Cargo.toml`
- Create: `crates/ui/controller-core/src/lib.rs`
- Modify: `Cargo.toml` (root workspace, add to `[workspace.dependencies]`)
- Modify: `release-plz.toml` (add package entry + changelog_include)

- [ ] **Step 1: Create `crates/ui/controller-core/Cargo.toml`**

```toml
[package]
name = "uptrakit-controller-core"
description = "Pure business-logic state types — zero web-api/mcp dependency"
edition.workspace = true
license.workspace = true
authors.workspace = true
repository.workspace = true
version = "0.0.1"
publish.workspace = true

[dependencies]
uptrakit-web-api-auth                   = { workspace = true }
uptrakit-web-api-queries                = { workspace = true }
uptrakit-shared-types                   = { workspace = true }
uptrakit-shared-db                      = { workspace = true }
uptrakit-audit-log                      = { workspace = true }
uptrakit-plugin-infrastructure-registry = { workspace = true }
uptrakit-wire                           = { workspace = true }
uptrakit-service-connections            = { workspace = true }
async-trait  = { workspace = true }
sea-orm      = { workspace = true }
serde_json   = { workspace = true }
time         = { workspace = true }
uuid         = { workspace = true }
tokio        = { workspace = true }
tokio-util   = { workspace = true }
parking_lot  = { workspace = true }
rootcause    = { workspace = true }
tracing      = { workspace = true }

[lints]
workspace = true
```

- [ ] **Step 2: Create `crates/ui/controller-core/src/lib.rs`**

```rust
//! `uptrakit-controller-core` — pure business-logic state types.
//!
//! **Invariant**: this crate must never import `uptrakit-web-api`, `uptrakit-mcp`,
//! or any crate that depends on them (axum, utoipa, etc.). Enforced by the
//! absence of those path deps in `Cargo.toml`. Any contributor adding a dep
//! that pulls in axum must stop and reconsider the design.

pub mod auth;
pub mod connections;
pub mod db;
pub mod notification;
pub mod settings;
pub mod update;
pub mod workload_claims;
pub mod audit;
```

Create all stub module files to make the crate compile:

```bash
mkdir -p crates/ui/controller-core/src/auth
mkdir -p crates/ui/controller-core/src/settings
mkdir -p crates/ui/controller-core/src/update

touch crates/ui/controller-core/src/auth/mod.rs
touch crates/ui/controller-core/src/auth/jwt.rs
touch crates/ui/controller-core/src/auth/denylist.rs
touch crates/ui/controller-core/src/auth/device_flow.rs
touch crates/ui/controller-core/src/auth/rate_limit.rs
touch crates/ui/controller-core/src/auth/api_token.rs
touch crates/ui/controller-core/src/connections.rs
touch crates/ui/controller-core/src/db.rs
touch crates/ui/controller-core/src/notification.rs
touch crates/ui/controller-core/src/settings/mod.rs
touch crates/ui/controller-core/src/update/mod.rs
touch crates/ui/controller-core/src/update/controller.rs
touch crates/ui/controller-core/src/workload_claims.rs
touch crates/ui/controller-core/src/audit.rs
```

- [ ] **Step 3: Add to `[workspace.dependencies]` in root `Cargo.toml`**

Find the `[workspace.dependencies]` section and add:

```toml
uptrakit-controller-core = { path = "crates/ui/controller-core" }
```

- [ ] **Step 4: Add to `release-plz.toml`**

Add after the last `[[package]]` entry that has `release = false`:

```toml
[[package]]
name = "uptrakit-controller-core"
release = false
```

Then in the `uptrakit-controller` `[[package]]` entry, add `"uptrakit-controller-core"` to its `changelog_include` array.
Same for `uptrakit-controller-standalone`. Both arrays are near lines 260 and 306.

- [ ] **Step 5: Run initial compile check**

```bash
cargo check --all-features 2>&1 | head -30
```

Expected: empty crate compiles cleanly. If module stubs are truly empty, some `unused import` warnings may appear when you
add deps later — that is fine at this stage.

- [ ] **Step 6: Commit**

```bash
git commit --only crates/ui/controller-core/ Cargo.toml release-plz.toml \
  -m "feat(controller-core): create crate scaffold with empty module stubs"
```

---

## Task 2: Move `DbState` to `controller-core/src/db.rs`

**Files:**

- Modify: `crates/ui/controller-core/src/db.rs`
- Modify: `crates/ui/web-api/src/app_state.rs` (replace definition with re-export)
- Modify: `crates/ui/web-api/Cargo.toml` (add controller-core dep)

`DbState` is currently defined at `crates/ui/web-api/src/app_state.rs:354–365`. Its `new()` is `pub(crate)` — promote it to `pub` when moving.

- [ ] **Step 1: Write `crates/ui/controller-core/src/db.rs`**

```rust
use sea_orm::DatabaseConnection;

/// Opaque newtype wrapping the database connection pool.
///
/// `#[non_exhaustive]`: additional metadata fields may be added (e.g. read replica).
/// External crates must use `DbState::new()`.
#[non_exhaustive]
#[derive(Clone)]
pub struct DbState(DatabaseConnection);

impl DbState {
    pub fn new(db: DatabaseConnection) -> Self {
        Self(db)
    }

    pub fn db(&self) -> &DatabaseConnection {
        &self.0
    }
}
```

- [ ] **Step 2: Add `uptrakit-controller-core` dep to `crates/ui/web-api/Cargo.toml`**

In the `[dependencies]` section, add:

```toml
uptrakit-controller-core = { workspace = true }
```

- [ ] **Step 3: Replace `DbState` definition in `crates/ui/web-api/src/app_state.rs`**

Find and replace the block from line 354 (`pub struct DbState(DatabaseConnection);`) through `}` at line 365 with:

```rust
pub use uptrakit_controller_core::db::DbState;
```

Also add the import at the top of the use block in app_state.rs (if not already covered by the re-export):

The existing `use sea_orm::DatabaseConnection;` in app_state.rs can stay — it's used by `AppStateBuilder`.

- [ ] **Step 4: Verify compile**

```bash
cargo check --all-features 2>&1 | grep -E "^error" | head -20
```

Expected: no errors.

- [ ] **Step 5: Commit**

```bash
git commit --only crates/ui/controller-core/src/db.rs crates/ui/web-api/src/app_state.rs crates/ui/web-api/Cargo.toml \
  -m "refactor(controller-core): move DbState from web-api"
```

---

## Task 3: Move `Settings` to `controller-core/src/settings/mod.rs`

**Files:**

- Modify: `crates/ui/controller-core/src/settings/mod.rs` (full content from web-api)
- Modify: `crates/ui/web-api/src/settings.rs` (replace with re-export shim)

`web-api/src/settings.rs` is 685 lines. Copy the entire content, then update the `crate::` imports.

- [ ] **Step 1: Copy settings.rs content to controller-core**

Copy the entire content of `crates/ui/web-api/src/settings.rs` into `crates/ui/controller-core/src/settings/mod.rs`.

- [ ] **Step 2: Update `crate::` imports in `controller-core/src/settings/mod.rs`**

The file uses these web-api internal paths — change each to its canonical source:

| Old (crate::)                                          | New (fully-qualified)                                                  |
| ------------------------------------------------------ | ---------------------------------------------------------------------- |
| `crate::SettingKey`                                    | `uptrakit_web_api_auth::SettingKey`                                    |
| `crate::auth::authentication::AuthenticationSettings`  | `uptrakit_web_api_auth::auth::authentication::AuthenticationSettings`  |
| `crate::auth::registration::RegistrationSettings`      | `uptrakit_web_api_auth::auth::registration::RegistrationSettings`      |
| `crate::settings_store::{RawSettings, RawSettingsExt}` | `uptrakit_web_api_auth::settings_store::{RawSettings, RawSettingsExt}` |
| `crate::auth` (any remaining)                          | `uptrakit_web_api_auth::auth`                                          |

The `uptrakit_web_api_types::MaskedUrl` and `uptrakit_plugin_infrastructure_registry::all_descriptors()` references are already
fully-qualified and need no change.

- [ ] **Step 3: Replace `crates/ui/web-api/src/settings.rs` with a re-export shim**

```rust
// Re-export shim — removed in Phase 2 once all internal callers are updated.
pub use uptrakit_controller_core::settings::*;
```

- [ ] **Step 4: Verify compile**

```bash
cargo check --all-features 2>&1 | grep -E "^error" | head -20
```

Expected: no errors. If any `use crate::settings::SomeName` in web-api breaks because of privacy, make the specific item
`pub` in controller-core's settings module.

- [ ] **Step 5: Commit**

```bash
git commit --only crates/ui/controller-core/src/settings/mod.rs crates/ui/web-api/src/settings.rs \
  -m "refactor(controller-core): move Settings and snapshots from web-api"
```

---

## Task 4: Move `AuthState` to `controller-core/src/auth/mod.rs`

**Files:**

- Modify: `crates/ui/controller-core/src/auth/mod.rs`
- Modify: `crates/ui/web-api/src/app_state.rs` (add AuthState re-export)

`AuthState` is at `app_state.rs:386–395`. Its fields use types from `uptrakit-web-api-auth`. Also move `AuthenticatedUser` and `AuthFailure`
from `middleware/require_auth.rs` — they're needed by `authenticate_api_token` (Phase 2) and have no web-api-specific deps.

- [ ] **Step 1: Write `crates/ui/controller-core/src/auth/mod.rs`**

```rust
pub mod api_token;
pub mod jwt;
pub mod denylist;
pub mod device_flow;
pub mod rate_limit;

pub use uptrakit_web_api_auth::auth::{AuthError, AuthMethod, permissions::Permission};
pub use uptrakit_web_api_auth::auth::jwt::JwtManager;
pub use uptrakit_web_api_auth::auth::token_denylist::TokenDenylist;
pub use uptrakit_web_api_auth::auth::device_flow::DeviceFlowStore;
pub use uptrakit_web_api_auth::auth::rate_limit::RateLimitStore;

/// Struct holding the result of a successful authentication attempt.
///
/// Moved from `web-api/src/middleware/require_auth.rs` — contains no HTTP/Axum types.
/// `#[non_exhaustive]`: OAuth 2.1 will add fields (e.g. scope, sub claim).
#[non_exhaustive]
#[derive(Clone, Debug)]
pub struct AuthenticatedUser {
    pub user_id: uuid::Uuid,
    pub auth_method: AuthMethod,
    pub permissions: Vec<Permission>,
    pub jti: Option<String>,
}

impl AuthenticatedUser {
    pub fn has_permission(&self, perm: Permission) -> bool {
        self.permissions.contains(&perm)
    }

    pub fn audit_actor(
        &self,
        api_token_id: Option<AuthenticatedApiTokenId>,
    ) -> (uptrakit_audit_log::AuditActorType, Option<uuid::Uuid>) {
        use uptrakit_audit_log::AuditActorType;
        match self.auth_method {
            AuthMethod::ApiToken => (
                AuditActorType::ApiToken,
                api_token_id.map(|t| t.0),
            ),
            _ => (AuditActorType::User, Some(self.user_id)),
        }
    }
}

/// Newtype for an authenticated API token ID — preserves type safety at audit boundaries.
#[derive(Clone, Copy, Debug)]
pub struct AuthenticatedApiTokenId(pub uuid::Uuid);

/// Failure variants returned by authentication helpers.
///
/// `#[non_exhaustive]`: OAuth 2.1 will add new rejection cases.
#[non_exhaustive]
#[derive(Debug)]
pub enum AuthFailure {
    InvalidApiToken,
    UserNotFound,
    UserDeactivated,
    InvalidOrExpiredToken,
    InvalidTokenSubject,
    TokenRevoked,
    InvalidOidcSessionMissingProvider,
    InternalError,
}

impl AuthFailure {
    /// Returns a short reason code suitable for audit log `details`, if applicable.
    pub fn api_token_reason_code(&self) -> Option<&'static str> {
        match self {
            Self::InvalidApiToken => Some("invalid_or_revoked_api_token"),
            Self::UserNotFound => Some("user_not_found"),
            Self::UserDeactivated => Some("user_deactivated"),
            Self::InternalError => Some("internal_error"),
            _ => None,
        }
    }
}

/// Authentication state held in `AppState` / `McpState`.
///
/// `#[non_exhaustive]`: OAuth 2.1 will add fields (e.g. OIDC provider registry).
#[non_exhaustive]
#[derive(Clone)]
pub struct AuthState {
    pub jwt: std::sync::Arc<JwtManager>,
    pub device_flow_store: DeviceFlowStore,
    pub rate_limit_store: RateLimitStore,
    pub token_denylist: std::sync::Arc<TokenDenylist>,
}
```

- [ ] **Step 2: Add re-export shim in `crates/ui/web-api/src/app_state.rs`**

Replace the `AuthState` struct definition (lines 386–395) with:

```rust
pub use uptrakit_controller_core::auth::{AuthState, AuthenticatedUser, AuthenticatedApiTokenId, AuthFailure};
```

- [ ] **Step 3: Update `require_auth.rs` imports**

In `crates/ui/web-api/src/middleware/require_auth.rs`, replace the local definitions of `AuthenticatedUser`,
`AuthenticatedApiTokenId`, `AuthFailure` with imports from controller-core:

```rust
use uptrakit_controller_core::auth::{
    AuthenticatedUser, AuthenticatedApiTokenId, AuthFailure,
};
```

Remove the now-redundant struct definitions from `require_auth.rs`.

- [ ] **Step 4: Verify compile**

```bash
cargo check --all-features 2>&1 | grep -E "^error" | head -20
```

Expected: no errors. Fix any visibility issue (make helper methods `pub`).

- [ ] **Step 5: Commit**

```bash
git commit --only crates/ui/controller-core/src/auth/ crates/ui/web-api/src/app_state.rs crates/ui/web-api/src/middleware/require_auth.rs \
  -m "refactor(controller-core): move AuthState, AuthenticatedUser, AuthFailure"
```

---

## Task 5: Move `WorkloadClaimRegistry` to `controller-core/src/workload_claims.rs`

**Files:**

- Modify: `crates/ui/controller-core/src/workload_claims.rs` (full content from web-api)
- Modify: `crates/ui/web-api/src/workload_claims.rs` (replace with re-export shim)

`web-api/src/workload_claims.rs` is 845 lines. It uses only `std::collections`, `time`, `uuid` — no web-api-specific imports. Safe to move as-is.

- [ ] **Step 1: Copy entire content of `crates/ui/web-api/src/workload_claims.rs` into `crates/ui/controller-core/src/workload_claims.rs`**

Verify no `crate::` references in the file:

```bash
grep -n "crate::" crates/ui/web-api/src/workload_claims.rs
```

Expected: no output. If any appear, replace them with their absolute path equivalents.

- [ ] **Step 2: Replace `crates/ui/web-api/src/workload_claims.rs` with a re-export shim**

```rust
// Re-export shim — removed in Phase 2.
pub use uptrakit_controller_core::workload_claims::*;
```

- [ ] **Step 3: Verify compile**

```bash
cargo check --all-features 2>&1 | grep -E "^error" | head -20
```

Expected: no errors.

- [ ] **Step 4: Commit**

```bash
git commit --only crates/ui/controller-core/src/workload_claims.rs crates/ui/web-api/src/workload_claims.rs \
  -m "refactor(controller-core): move WorkloadClaimRegistry from web-api"
```

---

## Task 6: Phase 1 CI quality gate

**Files:** None modified — this task only verifies.

- [ ] **Step 1: Format**

```bash
cargo fmt --all
```

Expected: no diff.

- [ ] **Step 2: Check without default features**

```bash
cargo check --no-default-features --features db-sqlite 2>&1 | grep -E "^error"
```

Expected: no output.

- [ ] **Step 3: Check all features**

```bash
cargo check --all-features 2>&1 | grep -E "^error"
```

Expected: no output.

- [ ] **Step 4: Clippy (both feature sets)**

```bash
cargo clippy --all-targets --no-default-features --features db-sqlite 2>&1 | grep -E "^error"
cargo clippy --all-targets --all-features 2>&1 | grep -E "^error"
```

Expected: no errors. Fix any clippy errors before proceeding.

- [ ] **Step 5: Tests**

```bash
cargo test --all-features 2>&1 | tail -20
```

Expected: all tests pass.

- [ ] **Step 6: Dependency audit**

```bash
cargo deny check
```

Expected: no violations.

- [ ] **Step 7: Commit any formatting/lint fixes**

```bash
# Commit only the files that fmt/clippy actually changed:
git commit --only crates/ui/controller-core/ crates/ui/web-api/ \
  -m "chore: apply fmt/clippy fixes after Phase 1 moves"
```

---

## Self-Review

**Spec coverage:**

- [x] Create `uptrakit-controller-core` crate with zero web-api/mcp imports (Task 1)
- [x] Move `DbState` (Task 2)
- [x] Move `Settings` + snapshots (Task 3)
- [x] Move `AuthState` + auth value types (Task 4)
- [x] Move `WorkloadClaimRegistry` (Task 5)
- [x] Add `release-plz.toml` entry (Task 1, Step 4)
- [x] Re-export shims keep existing `crate::settings::…` paths valid (Tasks 3–5)
- [ ] `ServiceConnectionRegistry` — Phase 2 (connections.rs is a stub here)
- [ ] `authenticate_api_token` / `emit_api_token_auth_audit` extraction — Phase 2
- [ ] `NotificationState` move — Phase 2
- [ ] `UpdateDispatcher` trait — Phase 3
- [ ] MCP decoupling — Phase 4

**Type consistency:** `AuthenticatedUser.audit_actor()` helper matches the `authenticated_user_audit_actor` free function that web-api still
exposes. Verify both call the same underlying logic after Task 4.

**Idiom audit:** All struct moves keep `#[non_exhaustive]` where appropriate. No `unwrap()` introduced. Constructor promotion from
`pub(crate)` to `pub` is required at crate boundaries — covered in Task 2.
