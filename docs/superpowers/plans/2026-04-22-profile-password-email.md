# Profile, Password & Email Management Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add self-service profile editing (display name), password change, and email change with email confirmation to the Settings > Profile page.

**Architecture:** New DB migration + entity for pending email-change requests. Backend handlers
added to existing `users.rs` and `auth.rs` route files. Token denylist extended with a JTI
allowlist so the current access token survives "invalidate all other sessions". Frontend adds
three new `SectionCard` sections to `/profile` plus a new `/auth/email-change/confirm` route
using `PublicEntryShell`.

**Tech Stack:** Rust (SeaORM, Axum, argon2), sea-orm-migration, uptrakit-crypto, SvelteKit 5, Tailwind v4, PublicEntryShell

---

## File Map

| Action | File |
| --- | --- |
| Create | `crates/shared/db/src/migration/m20260422_000001_email_change_request.rs` |
| Modify | `crates/shared/db/src/migration/mod.rs` |
| Create | `crates/shared/db/src/entity/email_change_request.rs` |
| Modify | `crates/shared/db/src/entity/mod.rs` |
| Modify | `crates/shared/db/src/entity/prelude.rs` |
| Modify | `crates/shared/db/src/entity/user.rs` |
| Create | `crates/shared/web-api-types/src/profile.rs` |
| Modify | `crates/shared/web-api-types/src/lib.rs` |
| Modify | `crates/shared/web-api-types/src/auth.rs` (`UserResponse`) |
| Modify | `crates/ui/web-api-auth/src/auth/token_denylist.rs` |
| Modify | `crates/ui/web-api-auth/src/auth/session.rs` |
| Modify | `crates/ui/web-api/src/middleware/require_auth.rs` |
| Modify | `crates/plugins/notifications/core/src/error.rs` |
| Modify | `crates/plugins/notifications/email/src/plugin.rs` |
| Modify | `crates/ui/web-api/src/routes/users.rs` |
| Modify | `crates/ui/web-api/src/app_state.rs` |
| Modify | `crates/ui/web-api/src/routes/auth.rs` |
| Modify | `crates/shared/scheduler-engine/src/executors/auth_cleanup.rs` |
| Modify | `frontend/src/lib/types.ts` |
| Modify | `frontend/src/lib/auth.svelte.ts` |
| Modify | `frontend/src/lib/api.ts` |
| Modify | `frontend/src/routes/profile/+page.svelte` |
| Create | `frontend/src/routes/auth/email-change/confirm/+page.svelte` |

---

## Task 1: DB Migration — email_change_requests table

**Files:**

- Create: `crates/shared/db/src/migration/m20260422_000001_email_change_request.rs`
- Modify: `crates/shared/db/src/migration/mod.rs`

- [ ] **Step 1: Write the migration**

```rust
// crates/shared/db/src/migration/m20260422_000001_email_change_request.rs
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub(super) struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Alias::new("email_change_requests"))
                    .col(
                        ColumnDef::new(Alias::new("id"))
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(Alias::new("user_id"))
                            .uuid()
                            .not_null()
                            .unique_key(),
                    )
                    .col(ColumnDef::new(Alias::new("new_email")).text().not_null())
                    .col(ColumnDef::new(Alias::new("token_hash")).string().not_null())
                    .col(
                        ColumnDef::new(Alias::new("expires_at"))
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Alias::new("created_at"))
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .from(
                                Alias::new("email_change_requests"),
                                Alias::new("user_id"),
                            )
                            .to(Alias::new("users"), Alias::new("id"))
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .table(Alias::new("email_change_requests"))
                    .name("idx_email_change_requests_token_hash")
                    .col(Alias::new("token_hash"))
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .table(Alias::new("email_change_requests"))
                    .name("idx_email_change_requests_expires_at")
                    .col(Alias::new("expires_at"))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(
                Table::drop()
                    .table(Alias::new("email_change_requests"))
                    .if_exists()
                    .to_owned(),
            )
            .await
    }
}
```

- [ ] **Step 2: Register in mod.rs**

In `crates/shared/db/src/migration/mod.rs`, add after the last `mod` declaration at line 64:

```rust
mod m20260422_000001_email_change_request;
```

And append to the migrations `vec![]` after the last `Box::new(m20260417_000001_semantic_audit_logs::Migration)` entry:

```rust
Box::new(m20260422_000001_email_change_request::Migration),
```

- [ ] **Step 3: Run migration test**

```bash
cargo test -p uptrakit-shared-db migrations_run_on_empty_sqlite --all-features -- --nocapture
```

Expected: PASS (the migration runs without error).

- [ ] **Step 4: Commit**

```bash
git add crates/shared/db/src/migration/m20260422_000001_email_change_request.rs \
        crates/shared/db/src/migration/mod.rs
git commit -m "feat(db): add email_change_requests migration"
```

---

## Task 2: DB Entity — email_change_request

**Files:**

- Create: `crates/shared/db/src/entity/email_change_request.rs`
- Modify: `crates/shared/db/src/entity/mod.rs`
- Modify: `crates/shared/db/src/entity/prelude.rs`
- Modify: `crates/shared/db/src/entity/user.rs`

- [ ] **Step 1: Create entity**

```rust
// crates/shared/db/src/entity/email_change_request.rs
use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "email_change_requests")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    #[sea_orm(unique)]
    pub user_id: Uuid,
    pub new_email: uptrakit_crypto::EncryptedString,
    pub token_hash: String,
    pub expires_at: OffsetDateTime,
    pub created_at: OffsetDateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::user::Entity",
        from = "Column::UserId",
        to = "super::user::Column::Id"
    )]
    User,
}

impl Related<super::user::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::User.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
```

- [ ] **Step 2: Register in mod.rs**

In `crates/shared/db/src/entity/mod.rs`, add after `pub mod user;`:

```rust
pub mod email_change_request;
```

- [ ] **Step 3: Export from prelude.rs**

In `crates/shared/db/src/entity/prelude.rs`, add after the `pub use super::data_encryption_key` line:

```rust
pub use super::email_change_request::{
    Entity as EmailChangeRequest, Model as EmailChangeRequestModel,
};
```

- [ ] **Step 4: Add HasMany relation on user.rs**

In `crates/shared/db/src/entity/user.rs`, add to the `Relation` enum:

```rust
#[sea_orm(has_many = "super::email_change_request::Entity")]
EmailChangeRequests,
```

And add the `Related` impl:

```rust
impl Related<super::email_change_request::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::EmailChangeRequests.def()
    }
}
```

- [ ] **Step 5: Verify compilation**

```bash
cargo check -p uptrakit-shared-db --all-features
```

Expected: PASS, no errors.

- [ ] **Step 6: Commit**

```bash
git add crates/shared/db/src/entity/email_change_request.rs \
        crates/shared/db/src/entity/mod.rs \
        crates/shared/db/src/entity/prelude.rs \
        crates/shared/db/src/entity/user.rs
git commit -m "feat(db): add EmailChangeRequest entity"
```

---

## Task 3: Type Definitions — profile request/response types

**Files:**

- Create: `crates/shared/web-api-types/src/profile.rs`
- Modify: `crates/shared/web-api-types/src/lib.rs`
- Modify: `crates/shared/web-api-types/src/auth.rs`

- [ ] **Step 1: Create profile.rs**

```rust
// crates/shared/web-api-types/src/profile.rs
use serde::{Deserialize, Serialize};
use uptrakit_shared_types::SecretString;

use crate::validation::{Validate, ValidationError};

#[derive(Serialize, Deserialize)]
pub struct UpdateProfileRequest {
    pub first_name: String,
    pub last_name: String,
}

impl Validate for UpdateProfileRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.first_name.is_empty() {
            return Err(ValidationError {
                field: "first_name",
                message: "first_name must not be empty".to_string(),
            });
        }
        if self.first_name.len() > 100 {
            return Err(ValidationError {
                field: "first_name",
                message: "first_name must not exceed 100 characters".to_string(),
            });
        }
        if self.last_name.len() > 100 {
            return Err(ValidationError {
                field: "last_name",
                message: "last_name must not exceed 100 characters".to_string(),
            });
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
pub struct InitiateEmailChangeRequest {
    pub new_email: String,
    pub password: SecretString,
}

impl Validate for InitiateEmailChangeRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.new_email.len() > 254 {
            return Err(ValidationError {
                field: "new_email",
                message: "new_email must not exceed 254 characters".to_string(),
            });
        }
        if !self.new_email.contains('@') {
            return Err(ValidationError {
                field: "new_email",
                message: "new_email must contain '@'".to_string(),
            });
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
pub struct ChangePasswordRequest {
    pub current_password: SecretString,
    pub new_password: SecretString,
}

impl Validate for ChangePasswordRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        let len = self.new_password.expose_secret().len();
        if len < 8 {
            return Err(ValidationError {
                field: "new_password",
                message: "new_password must be at least 8 characters".to_string(),
            });
        }
        if len > 1024 {
            return Err(ValidationError {
                field: "new_password",
                message: "new_password must not exceed 1024 characters".to_string(),
            });
        }
        Ok(())
    }
}
```

- [ ] **Step 2: Write tests for validation**

In a `#[cfg(test)]` block at the bottom of `profile.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_profile_empty_first_name_fails() {
        let req = UpdateProfileRequest {
            first_name: "".to_string(),
            last_name: "Doe".to_string(),
        };
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "first_name");
    }

    #[test]
    fn update_profile_valid() {
        let req = UpdateProfileRequest {
            first_name: "Jane".to_string(),
            last_name: "Doe".to_string(),
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn initiate_email_change_missing_at_fails() {
        let req = InitiateEmailChangeRequest {
            new_email: "notanemail".to_string(),
            password: SecretString::new("hunter2"),
        };
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "new_email");
    }

    #[test]
    fn change_password_too_short_fails() {
        let req = ChangePasswordRequest {
            current_password: SecretString::new("oldpass"),
            new_password: SecretString::new("short"),
        };
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "new_password");
    }

    #[test]
    fn change_password_valid() {
        let req = ChangePasswordRequest {
            current_password: SecretString::new("oldpassword"),
            new_password: SecretString::new("newpassword123"),
        };
        assert!(req.validate().is_ok());
    }
}
```

- [ ] **Step 3: Register profile.rs in lib.rs**

In `crates/shared/web-api-types/src/lib.rs`, add (alphabetically after `permissions`):

```rust
pub mod profile;
```

- [ ] **Step 4: Add `has_pending_email_change` to `UserResponse`**

In `crates/shared/web-api-types/src/auth.rs`, change `UserResponse` to:

```rust
#[derive(Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UserResponse {
    pub id: Uuid,
    pub email: String,
    pub first_name: String,
    pub last_name: String,
    pub permissions: Vec<Permission>,
    pub has_pending_email_change: bool,
}
```

- [ ] **Step 5: Fix the existing round-trip test in lib.rs**

The `user_response_round_trip` test in `crates/shared/web-api-types/src/lib.rs` constructs
`UserResponse` directly. It will fail to compile. Add `has_pending_email_change: false` to both
test instances there.

- [ ] **Step 6: Run tests**

```bash
cargo test -p uptrakit-web-api-types --all-features
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/shared/web-api-types/src/profile.rs \
        crates/shared/web-api-types/src/lib.rs \
        crates/shared/web-api-types/src/auth.rs
git commit -m "feat(types): add profile request types + has_pending_email_change to UserResponse"
```

---

## Task 4: Token Denylist — `deny_user_except` with JTI allowlist

**Files:**

- Modify: `crates/ui/web-api-auth/src/auth/token_denylist.rs`

- [ ] **Step 1: Write failing test**

Add to the `#[cfg(test)]` block in `token_denylist.rs`:

```rust
#[tokio::test]
async fn deny_user_except_keeps_allowlisted_jti_valid() {
    let denylist = TokenDenylist::new();
    let user_id = Uuid::from_bytes([20; 16]);
    let now = time::OffsetDateTime::now_utc().unix_timestamp();

    // Allow current JTI, deny all others issued before now.
    denylist
        .deny_user_except(user_id, "current-jti", now + 900, now, now + 900)
        .await;

    // Current JTI must pass even though iat < iat_cutoff.
    assert!(!denylist.is_denied("current-jti", &user_id, now - 1).await);

    // Any other JTI issued before cutoff must be denied.
    assert!(denylist.is_denied("old-jti", &user_id, now - 1).await);

    // Token issued at or after cutoff must be allowed regardless.
    assert!(!denylist.is_denied("new-jti", &user_id, now).await);
}

#[tokio::test]
async fn purge_expired_removes_expired_allowlist_entries() {
    let denylist = TokenDenylist::new();
    let user_id = Uuid::from_bytes([21; 16]);
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    let past = now - 100;

    // Add an already-expired allowlist entry.
    denylist
        .deny_user_except(user_id, "expired-allowed-jti", past, now - 200, past)
        .await;

    denylist.purge_expired().await;

    // After purge, the allowlist entry is gone. The user deny entry (if any) determines behavior.
    // Since iat_cutoff = now - 200, the expired entry itself: purge_after was `past` so it's pruned.
    // After purge, we now rely on user_entries alone. The user deny cutoff = now - 200, which is in
    // the past — and its purge_after = past too, so that entry is also gone.
    // The previously-allowlisted JTI must now be allowed (no user_entries block it).
    assert!(!denylist.is_denied("expired-allowed-jti", &user_id, now - 300).await);
}
```

- [ ] **Step 2: Run to verify they fail**

```bash
cargo test -p uptrakit-web-api-auth deny_user_except -- --nocapture 2>&1 | head -30
```

Expected: compile error (method does not exist) or test not found.

- [ ] **Step 3: Add `jti_allowlist` to `DenylistInner` and implement `deny_user_except`**

Change `DenylistInner`:

```rust
struct DenylistInner {
    /// JTI → expiry timestamp (unix seconds). Token is denied until it would
    /// have expired anyway.
    jti_entries: HashMap<String, i64>,
    /// user_id → revocation entry. All tokens for this user with
    /// `iat < entry.iat_cutoff` are denied.
    user_entries: HashMap<Uuid, UserDenyEntry>,
    /// JTI → expiry timestamp. These tokens are explicitly allowed even if the
    /// user-level denylist would otherwise deny them (e.g. after password change,
    /// keep the initiating request's token valid).
    jti_allowlist: HashMap<String, i64>,
}
```

Update `TokenDenylist::new()` and `new_with_db()` to initialise `jti_allowlist: HashMap::new()`.

Add the method after `deny_user_remote`:

```rust
/// Deny all tokens for a user issued before `iat_cutoff`, but explicitly
/// allow the token identified by `jti` (which expires at `jti_exp`).
///
/// Used after password or email change to invalidate all other access tokens
/// while keeping the caller's current token alive.
pub async fn deny_user_except(
    &self,
    user_id: Uuid,
    jti: &str,
    jti_exp: i64,
    iat_cutoff: i64,
    purge_after: i64,
) {
    self.deny_user(user_id, iat_cutoff, purge_after).await;
    self.inner
        .write()
        .await
        .jti_allowlist
        .insert(jti.to_string(), jti_exp);
}
```

Update `is_denied` to check the allowlist before user_entries:

```rust
pub async fn is_denied(&self, jti: &str, user_id: &Uuid, iat: i64) -> bool {
    let inner = self.inner.read().await;

    // Check JTI-level denial
    if inner.jti_entries.contains_key(jti) {
        return true;
    }

    // Check allowlist — explicitly permitted JTIs bypass user-level denial
    if inner.jti_allowlist.contains_key(jti) {
        return false;
    }

    // Check user-level denial
    if let Some(entry) = inner.user_entries.get(user_id)
        && iat < entry.iat_cutoff
    {
        return true;
    }

    false
}
```

Update `purge_expired` to also purge the allowlist:

```rust
pub async fn purge_expired(&self) {
    let now = time::OffsetDateTime::now_utc().unix_timestamp();

    {
        let mut inner = self.inner.write().await;
        inner.jti_entries.retain(|_, exp| *exp > now);
        inner.user_entries.retain(|_, entry| entry.purge_after > now);
        inner.jti_allowlist.retain(|_, exp| *exp > now);
    }

    // ... existing DB cleanup unchanged
}
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p uptrakit-web-api-auth token_denylist --all-features
```

Expected: all token_denylist tests PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/ui/web-api-auth/src/auth/token_denylist.rs
git commit -m "feat(auth): add jti_allowlist and deny_user_except to TokenDenylist"
```

---

## Task 5: AuthenticatedUser — add `jti` field

**Files:**

- Modify: `crates/ui/web-api/src/middleware/require_auth.rs`

- [ ] **Step 1: Write failing test**

Add at the end of the `#[cfg(test)]` block in `require_auth.rs`:

```rust
#[tokio::test]
async fn authenticate_jwt_sets_jti_on_authenticated_user() {
    let db = test_db().await;
    let state = test_state(db).await;

    let user_id = generate_uuid();
    let permissions = vec![];
    let jwt_token = state
        .auth
        .jwt
        .create_access_token(user_id, &permissions, "password", None)
        .unwrap();

    let auth_user = authenticate_jwt(&state, &jwt_token).await.unwrap();

    assert!(auth_user.jti.is_some(), "jti must be set for JWT auth");
    assert!(!auth_user.jti.unwrap().is_empty());
}
```

- [ ] **Step 2: Run to verify it fails**

```bash
cargo test -p uptrakit-web-api authenticate_jwt_sets_jti -- --nocapture 2>&1 | head -20
```

Expected: compile error (no field `jti` on `AuthenticatedUser`).

- [ ] **Step 3: Add `jti` to `AuthenticatedUser` and set it in `authenticate_jwt`**

Change `AuthenticatedUser`:

```rust
#[derive(Clone, Debug)]
pub struct AuthenticatedUser {
    pub user_id: uuid::Uuid,
    pub auth_method: AuthMethod,
    pub permissions: Vec<Permission>,
    /// JTI of the JWT access token, if authenticated via JWT (None for API token auth).
    pub jti: Option<String>,
}
```

In `authenticate_api_token`, set `jti: None` in the `AuthenticatedUser` literal:

```rust
Ok((
    AuthenticatedUser {
        user_id,
        auth_method: AuthMethod::ApiToken,
        permissions,
        jti: None,
    },
    token_id,
))
```

In `authenticate_jwt`, set `jti: Some(claims.jti.clone())`:

```rust
Ok(AuthenticatedUser {
    user_id,
    auth_method,
    permissions: claims.permissions,
    jti: Some(claims.jti),
})
```

- [ ] **Step 4: Fix compile errors**

Search for any other places that construct `AuthenticatedUser` and add `jti: None` or `jti: Some(...)`. Run `cargo check` to find all sites:

```bash
cargo check -p uptrakit-web-api --all-features 2>&1 | grep "missing field"
```

- [ ] **Step 5: Run tests**

```bash
cargo test -p uptrakit-web-api require_auth --all-features
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/ui/web-api/src/middleware/require_auth.rs
git commit -m "feat(auth): add jti field to AuthenticatedUser"
```

---

## Task 6: SessionService — `delete_user_sessions_except`

**Files:**

- Modify: `crates/ui/web-api-auth/src/auth/session.rs`

- [ ] **Step 1: Write failing test**

Add to `#[cfg(test)] mod tests` in `session.rs`:

```rust
#[tokio::test]
async fn test_delete_user_sessions_except_keeps_specified_session() {
    let db = setup_test_db().await;
    let service = SessionService::new(db.clone());

    let user = User::find().one(&db).await.unwrap().unwrap();

    // Create two sessions.
    let token_a = service
        .create_refresh_token(user.id, AuthMethod::Password, None, None)
        .await
        .unwrap();
    let token_b = service
        .create_refresh_token(user.id, AuthMethod::Password, None, None)
        .await
        .unwrap();

    // Find session A's id.
    let hash_a = hash_token(&token_a);
    let session_a = Session::find()
        .filter(session::Column::RefreshTokenHash.eq(hash_a))
        .one(&db)
        .await
        .unwrap()
        .unwrap();

    // Delete all except session A.
    service
        .delete_user_sessions_except(user.id, session_a.id)
        .await
        .unwrap();

    // Session A must survive.
    assert!(service.verify_refresh_token(&token_a).await.is_ok());
    // Session B must be gone.
    assert!(service.verify_refresh_token(&token_b).await.is_err());
}
```

- [ ] **Step 2: Run to verify failure**

```bash
cargo test -p uptrakit-web-api-auth test_delete_user_sessions_except -- --nocapture 2>&1 | head -20
```

Expected: compile error (method not found).

- [ ] **Step 3: Implement `delete_user_sessions_except` on `SessionService`**

In `session.rs`, add after `delete_user_sessions`:

```rust
/// Delete all sessions for a user except the one with the given session ID.
///
/// Used on password change to revoke all other sessions while keeping the
/// caller's current session alive.
pub async fn delete_user_sessions_except(
    &self,
    user_id: uuid::Uuid,
    except_session_id: uuid::Uuid,
) -> Result<()> {
    Session::delete_many()
        .filter(session::Column::UserId.eq(user_id))
        .filter(session::Column::Id.ne(except_session_id))
        .exec(&self.db)
        .await
        .context_to()?;

    Ok(())
}
```

- [ ] **Step 4: Add to `SessionOps` trait**

In the `SessionOps` trait definition, add:

```rust
async fn delete_user_sessions_except(
    &self,
    user_id: uuid::Uuid,
    except_session_id: uuid::Uuid,
) -> Result<()>;
```

- [ ] **Step 5: Add `SessionOps` impl for `SessionService`**

In the `impl SessionOps for SessionService` block, add:

```rust
async fn delete_user_sessions_except(
    &self,
    user_id: uuid::Uuid,
    except_session_id: uuid::Uuid,
) -> Result<()> {
    SessionService::delete_user_sessions_except(self, user_id, except_session_id).await
}
```

- [ ] **Step 6: Add to `MockSessionOps`**

In `mod controller_di_tests`, add to `impl SessionOps for MockSessionOps`:

```rust
async fn delete_user_sessions_except(
    &self,
    _user_id: uuid::Uuid,
    _except_session_id: uuid::Uuid,
) -> Result<()> {
    Ok(())
}
```

- [ ] **Step 7: Run tests**

```bash
cargo test -p uptrakit-web-api-auth session --all-features
```

Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add crates/ui/web-api-auth/src/auth/session.rs
git commit -m "feat(auth): add delete_user_sessions_except to SessionService + SessionOps"
```

---

## Task 7: NotificationPluginError — `SmtpNotConfigured` variant

**Files:**

- Modify: `crates/plugins/notifications/core/src/error.rs`
- Modify: `crates/plugins/notifications/email/src/plugin.rs`

- [ ] **Step 1: Add variant to `NotificationPluginError`**

In `crates/plugins/notifications/core/src/error.rs`, add after `InvalidConfig`:

```rust
/// SMTP is not configured; no host has been provided.
#[error("SMTP is not configured")]
SmtpNotConfigured,
```

- [ ] **Step 2: Update email plugin to use new variant**

In `crates/plugins/notifications/email/src/plugin.rs`, find the place that returns
`InvalidConfig("smtp_host must not be empty")` or similar when SMTP host is missing.
Change it to:

```rust
return Err(report!(NotificationPluginError::SmtpNotConfigured));
```

- [ ] **Step 3: Check for exhaustive matches on `NotificationPluginError`**

Since the enum is `#[non_exhaustive]`, external crates use wildcard arms. No internal match sites need updating unless they exist inside the core crate:

```bash
cargo check -p uptrakit-plugin-notifications-core --all-features 2>&1 | grep error
cargo check -p uptrakit-plugin-notifications-email --all-features 2>&1 | grep error
```

- [ ] **Step 4: Run notification plugin tests**

```bash
cargo test -p uptrakit-plugin-notifications-core --all-features
cargo test -p uptrakit-plugin-notifications-email --all-features
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/plugins/notifications/core/src/error.rs \
        crates/plugins/notifications/email/src/plugin.rs
git commit -m "feat(notifications): add SmtpNotConfigured error variant"
```

---

## Task 8: Backend — `update_profile` handler

**Files:**

- Modify: `crates/ui/web-api/src/routes/users.rs`

The handler lives at `PUT /api/v1/users/{id}/profile`. It allows any authenticated user to
update their own `first_name` and `last_name` only. A user cannot update other users via this
endpoint (use the admin endpoint for that).

- [ ] **Step 1: Write the handler**

Add to `users.rs`, before the closing of the module:

```rust
pub async fn update_profile(
    State(state): State<Arc<AppState>>,
    axum::Extension(auth_user): axum::Extension<AuthenticatedUser>,
    Path(user_id): Path<Uuid>,
    Json(req): Json<uptrakit_web_api_types::profile::UpdateProfileRequest>,
) -> Response {
    use uptrakit_web_api_types::validation::Validate;

    if auth_user.user_id != user_id {
        return error_response(StatusCode::FORBIDDEN, "Cannot update another user's profile");
    }

    if let Err(e) = req.validate() {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            &format!("{}: {}", e.field, e.message),
        );
    }

    let now = OffsetDateTime::now_utc();

    let model = match User::find_by_id(user_id).one(state.db()).await {
        Ok(Some(m)) => m,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "User not found"),
        Err(e) => {
            tracing::error!(error = %e, "failed to load user");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let mut active: uptrakit_shared_db::entity::user::ActiveModel = model.into();
    active.first_name = Set(req.first_name);
    active.last_name = Set(req.last_name);
    active.updated_at = Set(now);

    if let Err(e) = active.update(state.db()).await {
        tracing::error!(error = %e, "failed to update user profile");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    StatusCode::NO_CONTENT.into_response()
}
```

- [ ] **Step 2: Register route**

Find where users routes are registered (likely in `crates/ui/web-api/src/routes/mod.rs` or the main router). Add:

```rust
.route("/users/{id}/profile", put(users::update_profile))
```

Run `cargo check` to find the exact location:

```bash
cargo check -p uptrakit-web-api --all-features 2>&1 | head -20
```

Look for existing `.route("/users/...` patterns in the router setup.

- [ ] **Step 3: Run check**

```bash
cargo check -p uptrakit-web-api --all-features
```

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/ui/web-api/src/routes/users.rs
git commit -m "feat(api): add update_profile handler"
```

---

## Task 9: Backend — `initiate_email_change` handler

**Files:**

- Modify: `crates/ui/web-api/src/routes/users.rs`

`POST /api/v1/users/{id}/email` — verifies current password, upserts an `email_change_request` row, sends a confirmation email, returns 202.

- [ ] **Step 1: Add `callback_base_url` to `AppState`**

In `crates/ui/web-api/src/app_state.rs`, add a new public field to the `AppState` struct:

```rust
pub callback_base_url: String,
```

Then find all `AppState { ... }` construction sites (main.rs or builder) and thread the same value already passed to `NotificationDispatcher::new()`:

```bash
grep -rn "AppState {" crates/ui/web-api/src/ --include="*.rs" | head -10
```

- [ ] **Step 2: Write the handler**

```rust
pub async fn initiate_email_change(
    State(state): State<Arc<AppState>>,
    axum::Extension(auth_user): axum::Extension<AuthenticatedUser>,
    Path(user_id): Path<Uuid>,
    Json(req): Json<uptrakit_web_api_types::profile::InitiateEmailChangeRequest>,
) -> Response {
    use uptrakit_shared_db::entity::{email_change_request, prelude::*};
    use uptrakit_web_api_types::validation::Validate;
    use crate::auth::AuthMethod;

    if auth_user.user_id != user_id {
        return error_response(StatusCode::FORBIDDEN, "Cannot change another user's email");
    }
    // Only password-based users can change email via this flow.
    if !matches!(auth_user.auth_method, AuthMethod::Password) {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Email change is only available for password-based accounts",
        );
    }

    if let Err(e) = req.validate() {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            &format!("{}: {}", e.field, e.message),
        );
    }

    let user = match User::find_by_id(user_id).one(state.db()).await {
        Ok(Some(u)) => u,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "User not found"),
        Err(e) => {
            tracing::error!(error = %e, "failed to load user");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // Verify current password.
    let hash = match &user.password_hash {
        Some(h) => h.expose_secret().to_string(),
        None => return error_response(StatusCode::UNPROCESSABLE_ENTITY, "No password set"),
    };
    match crate::auth::password::verify_password(req.password.expose_secret(), &hash) {
        Ok(true) => {}
        Ok(false) => return error_response(StatusCode::UNAUTHORIZED, "Current password is incorrect"),
        Err(e) => {
            tracing::error!(error = %e, "failed to verify password");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    }

    // Check new email is not already taken.
    use sea_orm::{ColumnTrait, QueryFilter};
    let existing = User::find()
        .filter(uptrakit_shared_db::entity::user::Column::Email.eq(
            uptrakit_shared_types::MaskedEmail::new(req.new_email.as_str()),
        ))
        .one(state.db())
        .await;
    if let Ok(Some(_)) = existing {
        return error_response(StatusCode::CONFLICT, "Email address is already in use");
    }

    // Generate confirm token and store request (upsert via delete + insert).
    let raw_token = match crate::auth::token::generate_secure_token() {
        Ok(t) => t,
        Err(e) => {
            tracing::error!(error = %e, "failed to generate secure token");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };
    let token_hash = crate::auth::token::hash_token(&raw_token);

    let now = OffsetDateTime::now_utc();
    let expires_at = now + time::Duration::hours(24);

    let encrypted_email = match uptrakit_crypto::EncryptedString::new(
        req.new_email.clone(),
        "uptrakit:email_change_requests:new_email",
    ) {
        Ok(e) => e,
        Err(e) => {
            tracing::error!(error = %e, "failed to encrypt new email");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // Delete any existing request for this user, then insert fresh one.
    let txn = match state.db().begin().await {
        Ok(t) => t,
        Err(e) => {
            tracing::error!(error = %e, "failed to begin transaction");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    use sea_orm::TransactionTrait;
    let _ = EmailChangeRequest::delete_many()
        .filter(email_change_request::Column::UserId.eq(user_id))
        .exec(&txn)
        .await;

    let record = email_change_request::ActiveModel {
        id: Set(Uuid::now_v7()),
        user_id: Set(user_id),
        new_email: Set(encrypted_email),
        token_hash: Set(token_hash),
        expires_at: Set(expires_at),
        created_at: Set(now),
    };

    if let Err(e) = record.insert(&txn).await {
        tracing::error!(error = %e, "failed to insert email change request");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    if let Err(e) = txn.commit().await {
        tracing::error!(error = %e, "failed to commit email change request");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    // Send confirmation email.
    let confirm_url = format!(
        "{}/auth/email-change/confirm?token={}",
        state.callback_base_url,
        raw_token,
    );
    send_email_change_confirmation(
        &state,
        state.default_tenant_id,
        req.new_email.as_str(),
        &confirm_url,
    )
    .await;

    StatusCode::ACCEPTED.into_response()
}

async fn send_email_change_confirmation(
    state: &AppState,
    tenant_id: Uuid,
    to_address: &str,
    confirm_url: &str,
) {
    use uptrakit_plugin_infrastructure_registry::{DeliveryMessage, MessageAction, escape_html};

    let Some(transport) = state
        .plugin_ops
        .transport(&uptrakit_shared_types::PluginTypeId::new("email"))
    else {
        tracing::warn!("email notification transport not available; skipping confirmation email");
        return;
    };

    let settings_bag = crate::notifications::dispatcher::build_settings_bag(
        state.db(),
        tenant_id,
    )
    .await;

    let config = serde_json::json!({ "to_addresses": [to_address] });

    let title = "Confirm your email change".to_string();
    let body = format!(
        "Click the link below to confirm your new email address. This link expires in 24 hours.\n\n{}",
        confirm_url,
    );
    let body_html = format!(
        "<p>Click the link below to confirm your new email address. This link expires in 24 hours.</p>\
        <p><a href=\"{url}\">{url}</a></p>",
        url = escape_html(confirm_url),
    );

    let message = DeliveryMessage::new(
        title,
        body,
        Some(body_html),
        serde_json::Value::Null,
        vec![],
    );

    if let Err(e) = transport.deliver(&config, &settings_bag, &message).await {
        tracing::warn!(error = %e, %to_address, "failed to send email change confirmation");
    }
}
```

- [ ] **Step 3: Register route**

```rust
.route("/users/{id}/email", post(users::initiate_email_change))
```

- [ ] **Step 4: Run check**

```bash
cargo check -p uptrakit-web-api --all-features 2>&1 | grep error
```

Fix any compile errors (method names, field names on `AppState`).

- [ ] **Step 5: Commit**

```bash
git add crates/ui/web-api/src/routes/users.rs
git commit -m "feat(api): add initiate_email_change handler"
```

---

## Task 10: Backend — `cancel_email_change` and `confirm_email_change` handlers

**Files:**

- Modify: `crates/ui/web-api/src/routes/users.rs`
- Modify: `crates/ui/web-api/src/routes/auth.rs`

- [ ] **Step 1: Write `cancel_email_change` in `users.rs`**

`DELETE /api/v1/users/{id}/email` — deletes the pending request:

```rust
pub async fn cancel_email_change(
    State(state): State<Arc<AppState>>,
    axum::Extension(auth_user): axum::Extension<AuthenticatedUser>,
    Path(user_id): Path<Uuid>,
) -> Response {
    use uptrakit_shared_db::entity::{email_change_request, prelude::*};
    use sea_orm::{ColumnTrait, QueryFilter};

    if auth_user.user_id != user_id {
        return error_response(StatusCode::FORBIDDEN, "Cannot cancel another user's email change");
    }

    let result = EmailChangeRequest::delete_many()
        .filter(email_change_request::Column::UserId.eq(user_id))
        .exec(state.db())
        .await;

    match result {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "failed to cancel email change");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}
```

- [ ] **Step 2: Write `confirm_email_change` in `auth.rs`**

`GET /api/v1/auth/email-change/confirm?token=<token>` — public endpoint (no auth required). Validates token, applies email change, invalidates sessions:

```rust
pub async fn confirm_email_change(
    State(state): State<Arc<AppState>>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    use uptrakit_shared_db::entity::{email_change_request, prelude::*};
    use sea_orm::{ColumnTrait, QueryFilter, TransactionTrait, ActiveModelTrait, Set};

    let raw_token = match params.get("token") {
        Some(t) => t.clone(),
        None => return error_response(StatusCode::BAD_REQUEST, "Missing token"),
    };

    let token_hash = crate::auth::token::hash_token(&raw_token);
    let now = time::OffsetDateTime::now_utc();

    let request_row = match EmailChangeRequest::find()
        .filter(email_change_request::Column::TokenHash.eq(&token_hash))
        .one(state.db())
        .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "Invalid or expired token"),
        Err(e) => {
            tracing::error!(error = %e, "failed to look up email change request");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    if now >= request_row.expires_at {
        return error_response(StatusCode::GONE, "Token has expired");
    }

    let user_id = request_row.user_id;
    let new_email_plain = request_row.new_email.expose_secret().to_string();

    // Apply email change + delete request atomically.
    let txn = match state.db().begin().await {
        Ok(t) => t,
        Err(e) => {
            tracing::error!(error = %e, "failed to begin transaction");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let user = match User::find_by_id(user_id).one(&txn).await {
        Ok(Some(u)) => u,
        _ => return error_response(StatusCode::NOT_FOUND, "User not found"),
    };

    let mut active: uptrakit_shared_db::entity::user::ActiveModel = user.into();
    active.email = Set(uptrakit_shared_types::MaskedEmail::new(&new_email_plain));
    active.updated_at = Set(now);
    if let Err(e) = active.update(&txn).await {
        tracing::error!(error = %e, "failed to update user email");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    if let Err(e) = EmailChangeRequest::delete_by_id(request_row.id)
        .exec(&txn)
        .await
    {
        tracing::error!(error = %e, "failed to delete email change request");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    if let Err(e) = txn.commit().await {
        tracing::error!(error = %e, "failed to commit email change");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    // Invalidate all sessions (user must log in again with new email).
    let session_service = crate::auth::session::SessionService::new(state.db().clone());
    if let Err(e) = session_service.delete_user_sessions(user_id).await {
        tracing::warn!(error = %e, "failed to delete sessions after email change");
    }

    let now_ts = now.unix_timestamp();
    state
        .auth
        .token_denylist
        .deny_user(user_id, now_ts, now_ts + crate::auth::jwt::ACCESS_TOKEN_EXPIRY_SECS as i64)
        .await;

    (StatusCode::OK, Json(serde_json::json!({ "message": "Email confirmed successfully" }))).into_response()
}
```

- [ ] **Step 3: Register routes**

```rust
// In users router:
.route("/users/{id}/email", delete(users::cancel_email_change))

// In auth router (public, no require_auth middleware):
.route("/auth/email-change/confirm", get(auth::confirm_email_change))
```

- [ ] **Step 4: Run check**

```bash
cargo check -p uptrakit-web-api --all-features 2>&1 | grep "^error"
```

Fix any compile errors.

- [ ] **Step 5: Commit**

```bash
git add crates/ui/web-api/src/routes/users.rs \
        crates/ui/web-api/src/routes/auth.rs
git commit -m "feat(api): add cancel_email_change and confirm_email_change handlers"
```

---

## Task 11: Backend — `change_password` handler

**Files:**

- Modify: `crates/ui/web-api/src/routes/users.rs`

`PUT /api/v1/users/{id}/password` — verifies current password, sets new hash, invalidates all other sessions and access tokens.

- [ ] **Step 1: Find the refresh-token cookie name and extraction pattern**

```bash
grep -r "refresh_token\|REFRESH_TOKEN_COOKIE\|CookieJar" crates/ui/web-api/src/routes/auth.rs --include="*.rs" | head -20
```

Note the cookie name and extraction method used in the logout handler.

- [ ] **Step 2: Write the handler**

```rust
pub async fn change_password(
    State(state): State<Arc<AppState>>,
    axum::Extension(auth_user): axum::Extension<AuthenticatedUser>,
    Path(user_id): Path<Uuid>,
    headers: axum::http::HeaderMap,
    Json(req): Json<uptrakit_web_api_types::profile::ChangePasswordRequest>,
) -> Response {
    use uptrakit_web_api_types::validation::Validate;
    use crate::auth::AuthMethod;

    if auth_user.user_id != user_id {
        return error_response(StatusCode::FORBIDDEN, "Cannot change another user's password");
    }
    if !matches!(auth_user.auth_method, AuthMethod::Password) {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Password change is only available for password-based accounts",
        );
    }

    if let Err(e) = req.validate() {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            &format!("{}: {}", e.field, e.message),
        );
    }

    let user = match User::find_by_id(user_id).one(state.db()).await {
        Ok(Some(u)) => u,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "User not found"),
        Err(e) => {
            tracing::error!(error = %e, "failed to load user");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let hash = match &user.password_hash {
        Some(h) => h.expose_secret().to_string(),
        None => return error_response(StatusCode::UNPROCESSABLE_ENTITY, "No password set"),
    };

    match crate::auth::password::verify_password(req.current_password.expose_secret(), &hash) {
        Ok(true) => {}
        Ok(false) => return error_response(StatusCode::UNAUTHORIZED, "Current password is incorrect"),
        Err(e) => {
            tracing::error!(error = %e, "failed to verify password");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    }

    let new_hash = match crate::auth::password::hash_password(req.new_password.expose_secret()) {
        Ok(h) => h,
        Err(e) => {
            tracing::error!(error = %e, "failed to hash new password");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let now = OffsetDateTime::now_utc();
    let mut active: uptrakit_shared_db::entity::user::ActiveModel = user.into();
    active.password_hash = Set(Some(new_hash));
    active.updated_at = Set(now);

    if let Err(e) = active.update(state.db()).await {
        tracing::error!(error = %e, "failed to update password");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    // Extract current refresh token from cookie to identify the current session.
    let session_service = crate::auth::session::SessionService::new(state.db().clone());
    let refresh_token_opt = extract_refresh_token_from_headers(&headers);

    if let Some(refresh_token) = refresh_token_opt {
        let token_hash = crate::auth::token::hash_token(&refresh_token);
        use sea_orm::ColumnTrait;
        let session = uptrakit_shared_db::entity::prelude::Session::find()
            .filter(uptrakit_shared_db::entity::session::Column::RefreshTokenHash.eq(token_hash))
            .one(state.db())
            .await
            .ok()
            .flatten();

        if let Some(session) = session {
            let _ = session_service
                .delete_user_sessions_except(user_id, session.id)
                .await;
        } else {
            let _ = session_service.delete_user_sessions(user_id).await;
        }
    } else {
        let _ = session_service.delete_user_sessions(user_id).await;
    }

    // Deny all other access tokens; keep current JTI alive.
    let now_ts = now.unix_timestamp();
    if let Some(jti) = &auth_user.jti {
        state
            .auth
            .token_denylist
            .deny_user_except(
                user_id,
                jti,
                now_ts + crate::auth::jwt::ACCESS_TOKEN_EXPIRY_SECS,
                now_ts,
                now_ts + crate::auth::jwt::ACCESS_TOKEN_EXPIRY_SECS,
            )
            .await;
    } else {
        state
            .auth
            .token_denylist
            .deny_user(user_id, now_ts, now_ts + crate::auth::jwt::ACCESS_TOKEN_EXPIRY_SECS as i64)
            .await;
    }

    StatusCode::NO_CONTENT.into_response()
}

fn extract_refresh_token_from_headers(headers: &axum::http::HeaderMap) -> Option<String> {
    // Matches the same cookie extraction used by the logout/refresh handlers.
    let cookie_header = headers.get(axum::http::header::COOKIE)?.to_str().ok()?;
    for part in cookie_header.split(';') {
        let part = part.trim();
        if let Some(value) = part.strip_prefix("refresh_token=") {
            return Some(value.to_string());
        }
    }
    None
}
```

> **Note:** Verify the actual cookie name (`refresh_token`) used in the auth handlers:
>
> ```bash
> grep -r "refresh_token\|COOKIE_NAME" crates/ui/web-api/src/routes/auth.rs | head -5
> ```
>
> Also verify `hash_password` and `verify_password` function paths:
>
> ```bash
> grep -r "pub fn hash_password\|pub fn verify_password" crates/ui/web-api-auth/src/ --include="*.rs"
> ```

- [ ] **Step 3: Register route**

```rust
.route("/users/{id}/password", put(users::change_password))
```

- [ ] **Step 4: Run check**

```bash
cargo check -p uptrakit-web-api --all-features 2>&1 | grep "^error"
```

- [ ] **Step 5: Commit**

```bash
git add crates/ui/web-api/src/routes/users.rs
git commit -m "feat(api): add change_password handler with session invalidation"
```

---

## Task 12: Backend — update `me` endpoint with `has_pending_email_change`

**Files:**

- Modify: `crates/ui/web-api/src/routes/auth.rs`

- [ ] **Step 1: Update the `me` handler**

In `auth.rs`, the `me` handler at line ~1523 currently builds `UserResponse { id, email, first_name, last_name, permissions }`. Update it to:

```rust
// After loading the user and permissions, check for pending email change:
use sea_orm::{ColumnTrait, QueryFilter};
let has_pending_email_change = uptrakit_shared_db::entity::prelude::EmailChangeRequest::find()
    .filter(
        uptrakit_shared_db::entity::email_change_request::Column::UserId
            .eq(user.id),
    )
    .filter(
        uptrakit_shared_db::entity::email_change_request::Column::ExpiresAt
            .gt(time::OffsetDateTime::now_utc()),
    )
    .one(state.db())
    .await
    .unwrap_or(None)
    .is_some();

let response = UserResponse {
    id: user.id,
    email: user.email.expose_email().to_string(),
    first_name: user.first_name,
    last_name: user.last_name,
    permissions,
    has_pending_email_change,
};
```

Also update any other places in `auth.rs` that construct `UserResponse` (login, register,
refresh, oidc callbacks) to add `has_pending_email_change: false` (they don't need the DB
check since a freshly-logged-in user cannot have a pending request):

```bash
grep -n "UserResponse {" crates/ui/web-api/src/routes/auth.rs
```

Add `has_pending_email_change: false` to each struct literal that isn't the `me` handler.

- [ ] **Step 2: Run check**

```bash
cargo check -p uptrakit-web-api --all-features 2>&1 | grep "^error"
```

- [ ] **Step 3: Commit**

```bash
git add crates/ui/web-api/src/routes/auth.rs
git commit -m "feat(api): add has_pending_email_change to me endpoint"
```

---

## Task 13: Auth Cleanup — expire email_change_requests

**Files:**

- Modify: `crates/shared/scheduler-engine/src/executors/auth_cleanup.rs`

- [ ] **Step 1: Write failing test**

Add to `#[cfg(test)] mod tests` in `auth_cleanup.rs`:

```rust
#[tokio::test]
async fn deletes_expired_email_change_requests() {
    use uptrakit_shared_db::entity::{email_change_request, prelude::*};
    use sea_orm::{ActiveModelTrait, Set};
    use uptrakit_shared_types::MaskedEmail;
    use uptrakit_shared_db::entity::user;

    let db = setup_db().await;
    let now = OffsetDateTime::now_utc();
    let past = now - time::Duration::hours(25);
    let future = now + time::Duration::hours(24);

    // Insert a test user (required by FK).
    let user_id = uuid::Uuid::now_v7();
    let user2_id = uuid::Uuid::now_v7();
    user::ActiveModel {
        id: Set(user_id),
        email: Set(MaskedEmail::new("test-ecr1@example.com")),
        first_name: Set("Test".to_string()),
        last_name: Set("User".to_string()),
        password_hash: Set(None),
        is_active: Set(true),
        deactivated_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&db)
    .await
    .expect("insert user 1");

    user::ActiveModel {
        id: Set(user2_id),
        email: Set(MaskedEmail::new("test-ecr2@example.com")),
        first_name: Set("Test2".to_string()),
        last_name: Set("User2".to_string()),
        password_hash: Set(None),
        is_active: Set(true),
        deactivated_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&db)
    .await
    .expect("insert user 2");

    // Insert an expired request.
    email_change_request::ActiveModel {
        id: Set(uuid::Uuid::now_v7()),
        user_id: Set(user_id),
        new_email: Set(uptrakit_crypto::EncryptedString::plaintext_for_test("expired@example.com".to_string())),
        token_hash: Set("expired-hash".to_string()),
        expires_at: Set(past),
        created_at: Set(past),
    }
    .insert(&db)
    .await
    .expect("insert expired request");

    // Insert a fresh request.
    email_change_request::ActiveModel {
        id: Set(uuid::Uuid::now_v7()),
        user_id: Set(user2_id),
        new_email: Set(uptrakit_crypto::EncryptedString::plaintext_for_test("fresh@example.com".to_string())),
        token_hash: Set("fresh-hash".to_string()),
        expires_at: Set(future),
        created_at: Set(now),
    }
    .insert(&db)
    .await
    .expect("insert fresh request");

    let executor = AuthCleanupExecutor::new(db.clone());
    let task = make_task(&db);
    executor.execute(&task).await.expect("execute should succeed");

    use sea_orm::EntityTrait;
    let remaining = EmailChangeRequest::find().all(&db).await.expect("query");
    assert_eq!(remaining.len(), 1, "only fresh request should remain");
    assert_eq!(remaining[0].token_hash, "fresh-hash");
}
```

- [ ] **Step 2: Run to verify failure**

```bash
cargo test -p uptrakit-scheduler-engine deletes_expired_email_change_requests -- --nocapture 2>&1 | head -30
```

Expected: compile error (EmailChangeRequest not in scope).

- [ ] **Step 3: Add cleanup to `AuthCleanupExecutor::execute`**

In `auth_cleanup.rs`, add after `ApiRateLimit::delete_many()...` and before `txn.commit()`:

```rust
use uptrakit_shared_db::entity::{email_change_request, prelude::EmailChangeRequest};

EmailChangeRequest::delete_many()
    .filter(email_change_request::Column::ExpiresAt.lt(now))
    .exec(&txn)
    .await
    .context_to()?;
```

Add `uptrakit-crypto = { workspace = true, features = ["testing"] }` to `[dev-dependencies]`
in `crates/shared/scheduler-engine/Cargo.toml` — `EncryptedString::plaintext_for_test`
requires the `testing` feature.

Also add `uptrakit_crypto::enable_plaintext_mode();` as the first line of `setup_db()` in
`auth_cleanup.rs` — without this call, `EncryptedString::plaintext_for_test` panics at runtime.

- [ ] **Step 4: Run tests**

```bash
cargo test -p uptrakit-scheduler-engine --all-features
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/shared/scheduler-engine/src/executors/auth_cleanup.rs
git commit -m "feat(scheduler): expire email_change_requests in AuthCleanupExecutor"
```

---

## Task 14: Frontend — types, JWT decoder, API helpers

**Files:**

- Modify: `frontend/src/lib/types.ts`
- Modify: `frontend/src/lib/auth.svelte.ts`
- Modify: `frontend/src/lib/api.ts`

- [ ] **Step 1: Update `User` interface in `types.ts`**

Find the `export interface User` block (line ~64) and add `has_pending_email_change`:

```typescript
export interface User {
  id: string;
  email: string;
  first_name: string;
  last_name: string;
  permissions: Permission[];
  has_pending_email_change: boolean;
}
```

- [ ] **Step 2: Add `parseJwt` and `getAuthMethod` to `auth.svelte.ts`**

Add after the imports at the top of `auth.svelte.ts`:

```typescript
/** Decode a JWT payload without verification (client-side only). */
function parseJwt(token: string): Record<string, unknown> {
  try {
    const base64 = token.split('.')[1].replace(/-/g, '+').replace(/_/g, '/');
    return JSON.parse(atob(base64)) as Record<string, unknown>;
  } catch {
    return {};
  }
}

/** Returns the auth_method claim from the current access token, or null. */
export function getAuthMethod(): string | null {
  const token = getAccessToken();
  if (!token) return null;
  const claims = parseJwt(token);
  return typeof claims['auth_method'] === 'string' ? claims['auth_method'] : null;
}
```

- [ ] **Step 3: Add profile API helpers to `api.ts`**

Add at the end of `api.ts`:

```typescript
// Profile
export interface UpdateProfileRequest {
  first_name: string;
  last_name: string;
}

export interface InitiateEmailChangeRequest {
  new_email: string;
  password: string;
}

export interface ChangePasswordRequest {
  current_password: string;
  new_password: string;
}

export function updateProfile(userId: string, data: UpdateProfileRequest): Promise<void> {
  return requestVoid(`/users/${encodeURIComponent(userId)}/profile`, {
    method: 'PUT',
    body: JSON.stringify(data)
  });
}

export function initiateEmailChange(userId: string, data: InitiateEmailChangeRequest): Promise<void> {
  return requestVoid(`/users/${encodeURIComponent(userId)}/email`, {
    method: 'POST',
    body: JSON.stringify(data)
  });
}

export function cancelEmailChange(userId: string): Promise<void> {
  return requestVoid(`/users/${encodeURIComponent(userId)}/email`, {
    method: 'DELETE'
  });
}

export function changePassword(userId: string, data: ChangePasswordRequest): Promise<void> {
  return requestVoid(`/users/${encodeURIComponent(userId)}/password`, {
    method: 'PUT',
    body: JSON.stringify(data)
  });
}

export function confirmEmailChange(token: string): Promise<{ message: string }> {
  return request<{ message: string }>(
    `/auth/email-change/confirm?token=${encodeURIComponent(token)}`
  );
}
```

- [ ] **Step 4: Run frontend type check**

```bash
cd frontend && npm run check 2>&1 | head -40
```

Expected: PASS (or only pre-existing errors).

- [ ] **Step 5: Commit**

```bash
git add frontend/src/lib/types.ts \
        frontend/src/lib/auth.svelte.ts \
        frontend/src/lib/api.ts
git commit -m "feat(frontend): add profile types, JWT auth_method helper, and API functions"
```

---

## Task 15: Frontend — Profile Details section

**Files:**

- Modify: `frontend/src/routes/profile/+page.svelte`

- [ ] **Step 1: Add Profile Details section**

At the top of `<script lang="ts">`, add imports:

```typescript
import { getAuthMethod } from '$lib/auth.svelte';
import { updateProfile, initiateEmailChange, cancelEmailChange, changePassword } from '$lib/api';
import type { UpdateProfileRequest, InitiateEmailChangeRequest, ChangePasswordRequest } from '$lib/api';
```

Add state variables for the profile section (`user` is already declared at line 21 of the file — do not redeclare it):

```typescript
const authMethod = $derived(getAuthMethod());

// Profile details form
let firstName = $state('');
let lastName = $state('');
let profileSaving = $state(false);

$effect(() => {
  if (user) {
    firstName = user.first_name;
    lastName = user.last_name;
  }
});

async function handleSaveProfile() {
  if (!user) return;
  profileSaving = true;
  try {
    await updateProfile(user.id, { first_name: firstName, last_name: lastName });
    showSuccess('Profile updated');
  } catch (e) {
    showError(e instanceof Error ? e.message : 'Failed to update profile');
  } finally {
    profileSaving = false;
  }
}
```

In the template, add the Profile Details section before the existing API tokens section:

```svelte
<SectionCard title="Profile" data-ui="profile-details-section">
  <FormFieldRow label="First name">
    <Input bind:value={firstName} placeholder="First name" />
  </FormFieldRow>
  <FormFieldRow label="Last name">
    <Input bind:value={lastName} placeholder="Last name" />
  </FormFieldRow>
  <FormFieldRow label="Email">
    <Input value={user?.email ?? ''} disabled />
    {#if authMethod === 'password'}
      <Button variant="secondary" size="sm" onclick={() => (showChangeEmail = true)}>
        Change email
      </Button>
    {/if}
  </FormFieldRow>
  <div class="flex justify-end">
    <Button variant="primary" loading={profileSaving} onclick={handleSaveProfile}>
      Save
    </Button>
  </div>
</SectionCard>
```

- [ ] **Step 2: Verify no TypeScript errors in the section**

```bash
cd frontend && npm run check 2>&1 | grep "profile"
```

Expected: no errors relating to the profile section.

- [ ] **Step 3: Commit**

```bash
git add frontend/src/routes/profile/+page.svelte
git commit -m "feat(frontend): add Profile Details section to profile page"
```

---

## Task 16: Frontend — Change Email section

**Files:**

- Modify: `frontend/src/routes/profile/+page.svelte`

- [ ] **Step 1: Add state and handlers for email change**

```typescript
let showChangeEmail = $state(false);
let newEmail = $state('');
let emailPassword = $state('');
let emailChanging = $state(false);
let emailChangeSuccess = $state(false);

async function handleInitiateEmailChange() {
  if (!user) return;
  emailChanging = true;
  try {
    await initiateEmailChange(user.id, { new_email: newEmail, password: emailPassword });
    emailChangeSuccess = true;
    newEmail = '';
    emailPassword = '';
  } catch (e) {
    showError(e instanceof Error ? e.message : 'Failed to initiate email change');
  } finally {
    emailChanging = false;
  }
}

async function handleCancelEmailChange() {
  if (!user) return;
  try {
    await cancelEmailChange(user.id);
    showSuccess('Email change cancelled');
    // Refresh user data to update has_pending_email_change
    // (trigger re-fetch of /me or update local state)
  } catch (e) {
    showError(e instanceof Error ? e.message : 'Failed to cancel email change');
  }
}
```

- [ ] **Step 2: Add Change Email section template**

```svelte
{#if authMethod === 'password'}
  <SectionCard data-ui="change-email-section">
    {#if emailChangeSuccess}
      <Callout tone="success">
        A confirmation link has been sent to your new address. Check your inbox and click
        the link to complete the change.
      </Callout>
    {:else if user?.has_pending_email_change}
      <Callout tone="info">
        A confirmation email has been sent. Check your inbox. If you did not request this
        change, you can cancel it.
      </Callout>
      <div class="flex justify-end">
        <Button variant="ghost" onclick={handleCancelEmailChange}>Cancel email change</Button>
      </div>
    {:else if showChangeEmail}
      <FormFieldRow label="New email">
        <Input
          type="email"
          bind:value={newEmail}
          placeholder="new@example.com"
          data-ui="new-email-input"
        />
      </FormFieldRow>
      <FormFieldRow label="Current password">
        <Input
          type="password"
          bind:value={emailPassword}
          placeholder="Enter your password"
          data-ui="email-change-password-input"
        />
      </FormFieldRow>
      <div class="flex justify-end gap-2">
        <Button variant="ghost" onclick={() => (showChangeEmail = false)}>Cancel</Button>
        <Button
          variant="primary"
          loading={emailChanging}
          onclick={handleInitiateEmailChange}
        >
          Send confirmation email
        </Button>
      </div>
    {/if}
  </SectionCard>
{/if}
```

- [ ] **Step 3: Run check**

```bash
cd frontend && npm run check 2>&1 | grep "error"
```

- [ ] **Step 4: Commit**

```bash
git add frontend/src/routes/profile/+page.svelte
git commit -m "feat(frontend): add Change Email section to profile page"
```

---

## Task 17: Frontend — Change Password section

**Files:**

- Modify: `frontend/src/routes/profile/+page.svelte`

- [ ] **Step 1: Add password change state and handler**

```typescript
let currentPassword = $state('');
let newPassword = $state('');
let confirmPassword = $state('');
let passwordSaving = $state(false);
let confirmPasswordError = $state('');

async function handleChangePassword() {
  if (!user) return;
  if (newPassword !== confirmPassword) {
    confirmPasswordError = 'Passwords do not match';
    return;
  }
  confirmPasswordError = '';
  passwordSaving = true;
  try {
    await changePassword(user.id, {
      current_password: currentPassword,
      new_password: newPassword
    });
    showSuccess('Password changed');
    currentPassword = '';
    newPassword = '';
    confirmPassword = '';
  } catch (e) {
    showError(e instanceof Error ? e.message : 'Failed to change password');
  } finally {
    passwordSaving = false;
  }
}
```

- [ ] **Step 2: Add Change Password section template**

Only shown for password-auth users:

```svelte
{#if authMethod === 'password'}
  <SectionCard title="Change password" data-ui="change-password-section">
    <FormFieldRow label="Current password">
      <Input
        type="password"
        bind:value={currentPassword}
        placeholder="Current password"
        data-ui="current-password-input"
      />
    </FormFieldRow>
    <FormFieldRow label="New password">
      <Input
        type="password"
        bind:value={newPassword}
        placeholder="At least 8 characters"
        data-ui="new-password-input"
      />
    </FormFieldRow>
    <FormFieldRow label="Confirm new password">
      <Input
        type="password"
        bind:value={confirmPassword}
        placeholder="Repeat new password"
        data-ui="confirm-password-input"
      />
      {#if confirmPasswordError}
        <p class="text-sm text-(--color-danger)">{confirmPasswordError}</p>
      {/if}
    </FormFieldRow>
    <div class="flex justify-end">
      <Button variant="primary" loading={passwordSaving} onclick={handleChangePassword}>
        Change password
      </Button>
    </div>
  </SectionCard>
{/if}
```

- [ ] **Step 3: Run check**

```bash
cd frontend && npm run check 2>&1 | grep "error"
```

Expected: no errors.

- [ ] **Step 4: Commit**

```bash
git add frontend/src/routes/profile/+page.svelte
git commit -m "feat(frontend): add Change Password section to profile page"
```

---

## Task 18: Frontend — Email confirm route

**Files:**

- Create: `frontend/src/routes/auth/email-change/confirm/+page.svelte`

- [ ] **Step 1: Create the route directory and file**

```bash
mkdir -p frontend/src/routes/auth/email-change/confirm
```

- [ ] **Step 2: Write the page**

```svelte
<script lang="ts">
  import { onMount } from 'svelte';
  import { page } from '$app/state';
  import { goto } from '$app/navigation';
  import { confirmEmailChange } from '$lib/api';
  import { Callout } from '$lib/components/ui';
  import Button from '$lib/components/Button.svelte';
  import { PUBLIC_ENTRY_FORM_CLASS } from '$lib/components/ui/PublicEntryShell.svelte';
  import PublicEntryShell from '$lib/components/ui/PublicEntryShell.svelte';

  type State = 'loading' | 'success' | 'error';
  let state: State = $state('loading');
  let errorMessage = $state('');

  onMount(async () => {
    const token = page.url.searchParams.get('token');
    if (!token) {
      errorMessage = 'Invalid confirmation link.';
      state = 'error';
      return;
    }
    try {
      await confirmEmailChange(token);
      state = 'success';
    } catch (e) {
      errorMessage =
        e instanceof Error ? e.message : 'Failed to confirm email change.';
      state = 'error';
    }
  });
</script>

<PublicEntryShell title="Confirm email change">
  {#snippet children()}
    <div class={PUBLIC_ENTRY_FORM_CLASS}>
      {#if state === 'loading'}
        <Button variant="primary" loading={true} disabled>Confirming…</Button>
      {:else if state === 'success'}
        <Callout tone="success">
          Your email address has been updated. Please log in with your new address.
        </Callout>
        <Button variant="primary" onclick={() => goto('/login')}>Go to login</Button>
      {:else}
        <Callout tone="danger">{errorMessage}</Callout>
        <Button variant="secondary" onclick={() => goto('/profile')}>Back to profile</Button>
      {/if}
    </div>
  {/snippet}
</PublicEntryShell>
```

- [ ] **Step 3: Run check and build**

```bash
cd frontend && npm run check 2>&1 | grep "error"
cd frontend && npm run build 2>&1 | tail -20
```

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add frontend/src/routes/auth/email-change/confirm/+page.svelte
git commit -m "feat(frontend): add email change confirmation route"
```

---

## Task 19: Full quality gate

- [ ] **Step 1: Rust formatting and lints**

```bash
cargo fmt --all
cargo check --no-default-features --features db-sqlite
cargo check --all-features
cargo clippy --all-targets --no-default-features --features db-sqlite -- -D warnings
cargo clippy --all-targets --all-features -- -D warnings
```

Fix any warnings as errors.

- [ ] **Step 2: Run all Rust tests**

```bash
cargo test --all-features 2>&1 | tail -40
```

Expected: all tests pass.

- [ ] **Step 3: Frontend quality gate**

```bash
cd frontend && npm run lint && npm run format:check && npm run check && npm run test && npm run build
```

Expected: all pass.

- [ ] **Step 4: Markdown lint**

```bash
markdownlint --config .markdownlint.json 'docs/**/*.md'
```

- [ ] **Step 5: Final commit (if any fixups)**

```bash
git add -p   # stage only fixup changes
git commit -m "fix: quality gate fixups for profile management"
```

---

## Spec Coverage Check

| Spec requirement | Task |
| --- | --- |
| DB table for pending email change | Task 1 |
| SeaORM entity + user relation | Task 2 |
| Request/response types + validation | Task 3 |
| JTI allowlist on denylist (deny_user_except) | Task 4 |
| `jti` on `AuthenticatedUser` | Task 5 |
| `delete_user_sessions_except` | Task 6 |
| `SmtpNotConfigured` error variant | Task 7 |
| `update_profile` handler | Task 8 |
| `initiate_email_change` handler + email send | Task 9 |
| `cancel_email_change` handler | Task 10 |
| `confirm_email_change` handler | Task 10 |
| `change_password` handler + session invalidation | Task 11 |
| `has_pending_email_change` on me endpoint | Task 12 |
| Expire email_change_requests in scheduler | Task 13 |
| Frontend User type + JWT auth_method + API helpers | Task 14 |
| Profile Details section (name, read-only email, Change email button) | Task 15 |
| Change Email section (pending/no-pending states, success Callout) | Task 16 |
| Change Password section (3 fields, confirm match, hint) | Task 17 |
| Confirm email route with PublicEntryShell | Task 18 |
