use crate::SettingKey;
use crate::auth::Result;
#[cfg(feature = "oidc")]
use crate::auth::error::AuthError;
use crate::settings_store::{RawSettings, RawSettingsExt, upsert_setting};
#[cfg(feature = "oidc")]
use rootcause::prelude::*;
use sea_orm::ConnectionTrait;
#[cfg(feature = "oidc")]
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set};
#[cfg(feature = "oidc")]
use time::OffsetDateTime;
#[cfg(feature = "oidc")]
use uptrakit_shared_db::begin_immediate;
#[cfg(feature = "oidc")]
use uptrakit_shared_db::entity::{
    oidc_provider, prelude::*, role, user, user_oidc_link, user_role,
};
#[cfg(feature = "oidc")]
use uptrakit_shared_types::MaskedEmail;

/// Global authentication settings (password auth toggle and 2FA requirement).
/// OIDC config lives in the `oidc_providers` table.
#[derive(Clone, Debug)]
pub struct AuthenticationSettings {
    pub password_auth_enabled: bool,
    pub two_factor_required: bool,
}

impl Default for AuthenticationSettings {
    fn default() -> Self {
        Self {
            password_auth_enabled: true,
            two_factor_required: false,
        }
    }
}

impl AuthenticationSettings {
    /// Build from pre-fetched settings map. No DB access required.
    pub fn from_raw(raw: &RawSettings) -> Self {
        let password_auth_enabled = raw
            .get_setting(SettingKey::PasswordAuthEnabled)
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let two_factor_required = raw
            .get_setting(SettingKey::TwoFactorRequired)
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        Self {
            password_auth_enabled,
            two_factor_required,
        }
    }

    pub async fn save(&self, db: &impl ConnectionTrait, tenant_id: uuid::Uuid) -> Result<()> {
        upsert_setting(
            db,
            tenant_id,
            SettingKey::PasswordAuthEnabled,
            serde_json::Value::Bool(self.password_auth_enabled),
        )
        .await?;
        upsert_setting(
            db,
            tenant_id,
            SettingKey::TwoFactorRequired,
            serde_json::Value::Bool(self.two_factor_required),
        )
        .await
    }
}

/// Result of resolving an OIDC-authenticated user.
#[cfg(feature = "oidc")]
pub enum OidcUserResolution {
    /// Found via existing link -> auto-login.
    LinkedUser(uuid::Uuid),
    /// Email match, user has link with another active provider -> require login with that provider.
    LinkViaOidcRequired {
        user_id: uuid::Uuid,
        existing_provider_id: uuid::Uuid,
    },
    /// Email match, user has password, no active OIDC links -> require password.
    LinkViaPasswordRequired { user_id: uuid::Uuid },
    /// The OIDC provider did not confirm that the email address is verified.
    /// This variant is returned when `email_verified` is `false` **or absent**
    /// (`None`). Only `email_verified: true` is accepted as proof of
    /// ownership. Authentication is rejected to prevent account takeover via
    /// an attacker-controlled IdP that omits or falsifies the claim.
    ///
    /// Also returned when an email address is found but no OIDC link exists
    /// and linking is not permitted in the current flow.
    EmailNotVerified,
    /// New user created with link.
    NewUser(uuid::Uuid),
    /// Not found and auto-create disabled.
    NotAllowed,
    /// User is deactivated.
    Deactivated,
}

/// Parameters for [`resolve_oidc_user`].
#[cfg(feature = "oidc")]
pub struct OidcUserParams<'a, C: ConnectionTrait> {
    pub db: &'a C,
    pub tenant_id: uuid::Uuid,
    pub provider_id: uuid::Uuid,
    pub oidc_subject: &'a str,
    pub email: &'a MaskedEmail,
    pub first_name: Option<&'a str>,
    pub last_name: Option<&'a str>,
    pub auto_create: bool,
    pub email_verified: Option<bool>,
}

/// Resolve an OIDC-authenticated user.
///
/// Resolution order:
/// 1. Query `user_oidc_links` for `(provider_id, oidc_subject)`. Found -> `LinkedUser`.
/// 2. Query `users` by email. Found:
///    a. Deactivated -> `Deactivated`.
///    b. Has link with another active, non-deleted provider -> `LinkViaOidcRequired`.
///    c. Has password_hash -> `LinkViaPasswordRequired`.
///    d. Otherwise -> `EmailNotVerified` (manual linking required).
/// 3. Not found: auto_create -> create user + link -> `NewUser`. Else -> `NotAllowed`.
#[cfg(feature = "oidc")]
pub async fn resolve_oidc_user<C: ConnectionTrait>(
    params: OidcUserParams<'_, C>,
) -> Result<OidcUserResolution> {
    let OidcUserParams {
        db,
        tenant_id,
        provider_id,
        oidc_subject,
        email,
        first_name,
        last_name,
        auto_create,
        email_verified,
    } = params;

    // Reject unverified email addresses before any DB lookup.
    // Both `Some(false)` and `None` (claim absent) are treated as unverified.
    // This prevents account takeover via a rogue IdP that omits the claim.
    if email_verified != Some(true) {
        return Ok(OidcUserResolution::EmailNotVerified);
    }

    // 1. Check for existing link
    let existing_link = UserOidcLink::find()
        .filter(user_oidc_link::Column::ProviderId.eq(provider_id))
        .filter(user_oidc_link::Column::OidcSubject.eq(oidc_subject))
        .one(db)
        .await
        .context_to()?;

    if let Some(link) = existing_link {
        // Load user to check if they are still active
        let linked_user = User::find_by_id(link.user_id).one(db).await.context_to()?;

        match linked_user {
            Some(u) if !u.is_active => return Ok(OidcUserResolution::Deactivated),
            Some(_) => return Ok(OidcUserResolution::LinkedUser(link.user_id)),
            None => {
                // Orphaned link — user was deleted; fall through to email lookup
            }
        }
    }

    // 2. Check for existing user by email
    let existing_user = uptrakit_shared_db::users::find_by_canonical_email(db, email)
        .await
        .context_to()?;

    if let Some(found_user) = existing_user {
        // 2a. Deactivated
        if !found_user.is_active {
            return Ok(OidcUserResolution::Deactivated);
        }

        // 2b. Has link with another active, non-deleted provider
        let other_link = UserOidcLink::find()
            .filter(user_oidc_link::Column::UserId.eq(found_user.id))
            .filter(user_oidc_link::Column::ProviderId.ne(provider_id))
            .find_also_related(OidcProvider)
            .all(db)
            .await
            .context_to()?;

        for (_, provider) in &other_link {
            if let Some(p) = provider
                && p.is_active
                && p.deactivated_at.is_none()
            {
                return Ok(OidcUserResolution::LinkViaOidcRequired {
                    user_id: found_user.id,
                    existing_provider_id: p.id,
                });
            }
        }

        // 2c. Has password
        if found_user.password_hash.is_some() {
            return Ok(OidcUserResolution::LinkViaPasswordRequired {
                user_id: found_user.id,
            });
        }

        // 2d. No existing link for this provider — manual linking required.
        // Auto-linking by email is intentionally not supported.
        return Ok(OidcUserResolution::EmailNotVerified);
    }

    // 3. Not found
    if !auto_create {
        return Ok(OidcUserResolution::NotAllowed);
    }

    // Auto-create user
    let user_id = crate::auth::token::generate_uuid();
    let now = OffsetDateTime::now_utc();

    let new_user = user::ActiveModel {
        id: Set(user_id),
        email: Set(email.clone()),
        first_name: Set(first_name.unwrap_or("").to_string()),
        last_name: Set(last_name.unwrap_or("").to_string()),
        password_hash: Set(None),
        is_active: Set(true),
        deactivated_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    };
    new_user.insert(db).await.context_to()?;

    // Create link
    let link = user_oidc_link::ActiveModel {
        id: Set(crate::auth::token::generate_uuid()),
        user_id: Set(user_id),
        provider_id: Set(provider_id),
        oidc_subject: Set(oidc_subject.to_string()),
        linked_at: Set(now),
    };
    link.insert(db).await.context_to()?;

    // Assign default 'user' role (role mapping may override this later via sync_oidc_roles)
    if let Ok(Some(user_role_entity)) = Role::find()
        .filter(role::Column::Name.eq("user"))
        .filter(role::Column::TenantId.is_null())
        .one(db)
        .await
    {
        let user_role_model = user_role::ActiveModel {
            tenant_id: Set(tenant_id),
            user_id: Set(user_id),
            role_id: Set(user_role_entity.id),
            assigned_at: Set(now),
        };
        #[expect(
            clippy::let_underscore_must_use,
            reason = "best-effort default role assignment; failure is not fatal for user creation"
        )]
        let _ = user_role_model.insert(db).await;
    }

    Ok(OidcUserResolution::NewUser(user_id))
}

/// Outcome of a guarded OIDC role sync. Deliberately NOT `#[non_exhaustive]`:
/// a closed outcome set callers must handle exhaustively — mirrors the
/// `LockoutVerdict` precedent (`access_grants.rs`) and the closed-enum
/// exception in coding-standards.md (a new outcome kind is a semantic
/// change every caller must see).
#[cfg(feature = "oidc")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoleSyncOutcome {
    /// Assignment replaced. Caller must invalidate + publish post-commit.
    Applied,
    /// The lockout guard kept the existing assignment unchanged; the login
    /// proceeds. Caller emits the `user_role.sync_lockout_prevented` Event.
    SkippedLockout { attempted_role_names: Vec<String> },
    /// No mapping configured / mapped set equals current — nothing written.
    NoChange,
}

/// Sync OIDC roles for a user based on provider configuration and ID token
/// claims.
///
/// Guarded (M1.6a): any role-set change runs the shrinking-mutation lockout
/// guard ([`uptrakit_shared_db::access_grants::check_lockout`]) before
/// writing — a mapped replace that would strip the tenant's last
/// `access:manage` holder (or the last global `system.access:manage`
/// holder) is skipped rather than applied; the login still succeeds
/// ([`RoleSyncOutcome::SkippedLockout`]). The early "nothing mapped" returns
/// below are unguarded by design (no write, nothing to lock); a set that
/// resolves equal to the current one also skips the guard (steady-state
/// login stays lock-free).
///
/// `txn` must be a real `DatabaseTransaction` obtained in SQLite `Immediate`
/// mode (e.g. via `uptrakit_shared_db::access_grants::begin_guarded`, or an
/// existing `Immediate` transaction) — a Deferred `begin()` compiles but
/// serializes nothing on SQLite; the concrete transaction type only rules
/// out pooled-autocommit misuse, it cannot express the mode itself.
#[cfg(feature = "oidc")]
pub async fn sync_oidc_roles(
    txn: &sea_orm::DatabaseTransaction,
    tenant_id: uuid::Uuid,
    default_tenant_id: uuid::Uuid,
    user_id: uuid::Uuid,
    provider: &oidc_provider::Model,
    claims: &serde_json::Value,
) -> Result<RoleSyncOutcome> {
    let role_claim_path = match &provider.role_claim_path {
        Some(path) if !path.is_empty() => path,
        _ => return Ok(RoleSyncOutcome::NoChange),
    };

    let mapping = &provider.role_mapping.0;
    if mapping.is_empty() {
        return Ok(RoleSyncOutcome::NoChange);
    }

    // Navigate the claims JSON to find the role values
    let claim_value = match navigate_json_path(claims, role_claim_path) {
        Some(v) => v,
        None => return Ok(RoleSyncOutcome::NoChange),
    };

    // Extract claim values as strings
    let claim_strings: Vec<&str> = match claim_value {
        serde_json::Value::Array(arr) => arr.iter().filter_map(|v| v.as_str()).collect(),
        serde_json::Value::String(s) => vec![s.as_str()],
        _ => return Ok(RoleSyncOutcome::NoChange),
    };

    // Map OIDC claim values to local role names via the provider's role_mapping
    let local_role_names: Vec<&str> = claim_strings
        .iter()
        .filter_map(|claim| mapping.get(*claim).map(|s| s.as_str()))
        .collect();

    if local_role_names.is_empty() {
        return Ok(RoleSyncOutcome::NoChange);
    }

    // Look up matching local roles. Ordered by name so a lockout denial's
    // `attempted_role_names` (below) is deterministic rather than depending
    // on SQLite's unspecified IN-list result order.
    let local_roles = Role::find()
        .filter(role::Column::Name.is_in(local_role_names))
        .filter(role::Column::TenantId.is_null())
        .order_by_asc(role::Column::Name)
        .all(txn)
        .await
        .context_to()?;

    if local_roles.is_empty() {
        return Ok(RoleSyncOutcome::NoChange);
    }

    let new_role_ids: std::collections::BTreeSet<uuid::Uuid> =
        local_roles.iter().map(|r| r.id).collect();
    let current_role_ids: std::collections::BTreeSet<uuid::Uuid> = UserRole::find()
        .filter(user_role::Column::TenantId.eq(tenant_id))
        .filter(user_role::Column::UserId.eq(user_id))
        .all(txn)
        .await
        .context_to()?
        .into_iter()
        .map(|ur| ur.role_id)
        .collect();

    if new_role_ids == current_role_ids {
        // Steady-state login (mapped set == current): no guard, no sentinel
        // lock, no write — the common path stays lock-free via this return.
        return Ok(RoleSyncOutcome::NoChange);
    }
    // Any set change runs the guard. No pre-lock superset shortcut: deciding
    // whether-to-lock from a read taken before the lock is an unguarded-shrink
    // hole the moment the serialization property is the deployment's (SQLite
    // Immediate) rather than the code's. The guard re-reads all authority
    // state after taking the sentinel lock; pure-add sets verdict Permitted.
    let ids: Vec<uuid::Uuid> = new_role_ids.iter().copied().collect();
    let verdict = uptrakit_shared_db::access_grants::check_lockout(
        txn,
        default_tenant_id,
        &uptrakit_shared_db::access_grants::GuardedMutation::SetUserRoles {
            tenant_id,
            user_id,
            new_role_ids: &ids,
        },
    )
    .await
    .context_to()?;
    // Exhaustive, no wildcard: a future verdict variant must be classified
    // here deliberately rather than silently falling into "denied".
    match verdict {
        uptrakit_shared_db::access_grants::LockoutVerdict::Permitted => {}
        uptrakit_shared_db::access_grants::LockoutVerdict::TenantLockout
        | uptrakit_shared_db::access_grants::LockoutVerdict::SystemLockout => {
            return Ok(RoleSyncOutcome::SkippedLockout {
                attempted_role_names: local_roles.iter().map(|r| r.name.clone()).collect(),
            });
        }
    }

    // Write phase inside a savepoint: a mid-write failure must not leave a
    // partial set for the caller's swallow-and-commit to persist. Roll back
    // EXPLICITLY on error — never lean on drop: DatabaseTransaction's Drop
    // only QUEUES the rollback (flushed on some later async op, an sqlx
    // internal, not a contract) and its lock-contention branch panics,
    // which under panic = "abort" kills the process.
    let sp = begin_immediate(txn).await.context_to()?; // SAVEPOINT on the outer txn (Immediate mode is a no-op here)
    let write_result = async {
        // Delete existing user_role rows for this user within this tenant
        UserRole::delete_many()
            .filter(user_role::Column::TenantId.eq(tenant_id))
            .filter(user_role::Column::UserId.eq(user_id))
            .exec(&sp)
            .await
            .context_to()?;

        // Insert mapped roles
        let now = OffsetDateTime::now_utc();
        for local_role in &local_roles {
            let user_role_model = user_role::ActiveModel {
                tenant_id: Set(tenant_id),
                user_id: Set(user_id),
                role_id: Set(local_role.id),
                assigned_at: Set(now),
            };
            user_role_model.insert(&sp).await.context_to()?;
        }

        Ok::<(), Report<AuthError>>(())
    }
    .await;
    match write_result {
        Ok(()) => {
            // RELEASE. `commit()` consumes `sp`; on failure there is no
            // handle left to explicitly roll back with -- `sp`'s Drop impl
            // (invoked as `commit()` unwinds) already queues a best-effort
            // `start_rollback()` internally. That queued rollback is the
            // same not-a-contract mechanism the comment above warns against
            // relying on, so make the failure loud rather than letting it
            // propagate silently: an operator seeing this log line knows
            // the outer transaction's eventual commit may be carrying
            // whatever that queued rollback did or didn't flush in time.
            if let Err(commit_err) = sp.commit().await {
                tracing::error!(
                    error = ?commit_err,
                    "savepoint RELEASE failed after OIDC role-sync write succeeded; \
                     outer transaction may carry inconsistent role state"
                );
                return Err(commit_err).context_to();
            }
        }
        Err(e) => {
            // Immediate ROLLBACK TO SAVEPOINT. Never `let _ =` here:
            // let_underscore_must_use is deny at the workspace root, and a
            // failed rollback is the one case the atomicity guarantee breaks.
            if let Err(rollback_err) = sp.rollback().await {
                tracing::error!(error = ?rollback_err, "savepoint rollback failed after OIDC role-sync write error");
            }
            return Err(e);
        }
    }

    Ok(RoleSyncOutcome::Applied)
}

/// Extract mapped local role names from OIDC claims without touching the DB.
#[cfg(feature = "oidc")]
pub fn extract_mapped_roles(
    provider: &oidc_provider::Model,
    claims: &serde_json::Value,
) -> Vec<String> {
    let role_claim_path = match &provider.role_claim_path {
        Some(path) if !path.is_empty() => path,
        _ => return vec![],
    };

    let mapping = &provider.role_mapping.0;
    if mapping.is_empty() {
        return vec![];
    }

    let claim_value = match navigate_json_path(claims, role_claim_path) {
        Some(v) => v,
        None => return vec![],
    };

    let claim_strings: Vec<&str> = match claim_value {
        serde_json::Value::Array(arr) => arr.iter().filter_map(|v| v.as_str()).collect(),
        serde_json::Value::String(s) => vec![s.as_str()],
        _ => return vec![],
    };

    claim_strings
        .iter()
        .filter_map(|claim| mapping.get(*claim).cloned())
        .collect()
}

/// Navigate a JSON value via a dot-separated path.
#[cfg(feature = "oidc")]
pub fn navigate_json_path<'a>(
    value: &'a serde_json::Value,
    path: &str,
) -> Option<&'a serde_json::Value> {
    let mut current = value;
    for segment in path.split('.') {
        current = current.get(segment)?;
    }
    Some(current)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    // ── AuthenticationSettings::default ──────────────────────────────

    #[test]
    fn default_has_password_auth_enabled() {
        let settings = AuthenticationSettings::default();
        assert!(
            settings.password_auth_enabled,
            "default should have password_auth_enabled = true"
        );
        assert!(
            !settings.two_factor_required,
            "default should have two_factor_required = false"
        );
    }

    // ── AuthenticationSettings::from_raw ─────────────────────────────

    #[test]
    fn from_raw_empty_map_defaults_to_true() {
        let raw: RawSettings = HashMap::new();
        let settings = AuthenticationSettings::from_raw(&raw);
        assert!(
            settings.password_auth_enabled,
            "empty settings map should default password_auth_enabled to true"
        );
    }

    #[test]
    fn from_raw_explicit_true() {
        let mut raw: RawSettings = HashMap::new();
        raw.insert(
            SettingKey::PasswordAuthEnabled.as_str().to_string(),
            serde_json::Value::Bool(true),
        );
        let settings = AuthenticationSettings::from_raw(&raw);
        assert!(
            settings.password_auth_enabled,
            "explicit true should set password_auth_enabled to true"
        );
    }

    #[test]
    fn from_raw_explicit_false() {
        let mut raw: RawSettings = HashMap::new();
        raw.insert(
            SettingKey::PasswordAuthEnabled.as_str().to_string(),
            serde_json::Value::Bool(false),
        );
        let settings = AuthenticationSettings::from_raw(&raw);
        assert!(
            !settings.password_auth_enabled,
            "explicit false should set password_auth_enabled to false"
        );
    }

    #[test]
    fn from_raw_non_bool_value_defaults_to_true() {
        let mut raw: RawSettings = HashMap::new();
        raw.insert(
            SettingKey::PasswordAuthEnabled.as_str().to_string(),
            serde_json::Value::String("not_a_bool".to_string()),
        );
        let settings = AuthenticationSettings::from_raw(&raw);
        assert!(
            settings.password_auth_enabled,
            "non-bool value should default password_auth_enabled to true"
        );
    }

    #[test]
    fn from_raw_two_factor_required_empty_map_defaults_to_false() {
        let raw: RawSettings = HashMap::new();
        let settings = AuthenticationSettings::from_raw(&raw);
        assert!(
            !settings.two_factor_required,
            "empty settings map should default two_factor_required to false"
        );
    }

    #[test]
    fn from_raw_two_factor_required_explicit_true() {
        let mut raw: RawSettings = HashMap::new();
        raw.insert(
            SettingKey::TwoFactorRequired.as_str().to_string(),
            serde_json::Value::Bool(true),
        );
        let settings = AuthenticationSettings::from_raw(&raw);
        assert!(
            settings.two_factor_required,
            "explicit true should set two_factor_required to true"
        );
    }

    #[test]
    fn from_raw_two_factor_required_explicit_false() {
        let mut raw: RawSettings = HashMap::new();
        raw.insert(
            SettingKey::TwoFactorRequired.as_str().to_string(),
            serde_json::Value::Bool(false),
        );
        let settings = AuthenticationSettings::from_raw(&raw);
        assert!(
            !settings.two_factor_required,
            "explicit false should set two_factor_required to false"
        );
    }

    #[test]
    fn from_raw_two_factor_required_non_bool_value_defaults_to_false() {
        let mut raw: RawSettings = HashMap::new();
        raw.insert(
            SettingKey::TwoFactorRequired.as_str().to_string(),
            serde_json::Value::String("not_a_bool".to_string()),
        );
        let settings = AuthenticationSettings::from_raw(&raw);
        assert!(
            !settings.two_factor_required,
            "non-bool value should default two_factor_required to false"
        );
    }

    // ── navigate_json_path ───────────────────────────────────────────

    #[cfg(feature = "oidc")]
    #[test]
    fn navigate_json_path_single_level() {
        let json = serde_json::json!({"role": "admin"});
        let result = navigate_json_path(&json, "role");
        assert_eq!(
            result.expect("single-level path should resolve"),
            &serde_json::json!("admin")
        );
    }

    #[cfg(feature = "oidc")]
    #[test]
    fn navigate_json_path_nested() {
        let json = serde_json::json!({"a": {"b": {"c": 42}}});
        let result = navigate_json_path(&json, "a.b.c");
        assert_eq!(
            result.expect("nested path a.b.c should resolve"),
            &serde_json::json!(42)
        );
    }

    #[cfg(feature = "oidc")]
    #[test]
    fn navigate_json_path_missing_intermediate_returns_none() {
        let json = serde_json::json!({"a": {"x": 1}});
        let result = navigate_json_path(&json, "a.b.c");
        assert!(
            result.is_none(),
            "missing intermediate key should return None"
        );
    }

    #[cfg(feature = "oidc")]
    #[test]
    fn navigate_json_path_empty_string_returns_none() {
        let json = serde_json::json!({"a": 1});
        // An empty string split by '.' yields one empty segment "",
        // and json.get("") returns None for a non-empty-key object.
        let result = navigate_json_path(&json, "");
        assert!(
            result.is_none(),
            "empty path should return None because get(\"\") finds no key"
        );
    }

    // ── extract_mapped_roles ─────────────────────────────────────────

    #[cfg(feature = "oidc")]
    fn build_test_provider(
        role_claim_path: Option<String>,
        role_mapping: HashMap<String, String>,
    ) -> oidc_provider::Model {
        use time::OffsetDateTime;
        use uptrakit_crypto::{EncryptedString, init_master_key};

        // Ensure a test master key is set (no-op if already initialized).
        let _ignored = init_master_key(zeroize::Zeroizing::new([0x42u8; 32]));
        let client_secret = EncryptedString::new(
            "test-secret".to_string(),
            "uptrakit:oidc_providers:client_secret",
        )
        .expect("master key initialized above");

        oidc_provider::Model {
            id: uuid::Uuid::nil(),
            tenant_id: uuid::Uuid::nil(),
            name: "test-provider".to_string(),
            slug: "test-provider".to_string(),
            logo_url: None,
            issuer_url: "https://issuer.example.com".to_string(),
            client_id: "test-client-id".to_string(),
            client_secret,
            scopes: "openid profile email".to_string(),
            auto_create_users: false,
            allow_private_network_issuers: false,
            role_claim_path,
            role_mapping: oidc_provider::RoleMapping(role_mapping),
            is_active: true,
            created_at: OffsetDateTime::UNIX_EPOCH,
            updated_at: OffsetDateTime::UNIX_EPOCH,
            deactivated_at: None,
        }
    }

    #[cfg(feature = "oidc")]
    #[test]
    fn extract_mapped_roles_empty_role_claim_path_returns_empty() {
        let provider = build_test_provider(None, HashMap::new());
        let claims = serde_json::json!({"roles": ["admin"]});
        let result = extract_mapped_roles(&provider, &claims);
        assert!(
            result.is_empty(),
            "None role_claim_path should return empty vec"
        );
    }

    #[cfg(feature = "oidc")]
    #[test]
    fn extract_mapped_roles_empty_role_mapping_returns_empty() {
        let provider = build_test_provider(Some("roles".to_string()), HashMap::new());
        let claims = serde_json::json!({"roles": ["admin"]});
        let result = extract_mapped_roles(&provider, &claims);
        assert!(
            result.is_empty(),
            "empty role_mapping should return empty vec"
        );
    }

    #[cfg(feature = "oidc")]
    #[test]
    fn extract_mapped_roles_array_claim_values() {
        let mut mapping = HashMap::new();
        mapping.insert("oidc-admin".to_string(), "admin".to_string());
        mapping.insert("oidc-viewer".to_string(), "viewer".to_string());
        let provider = build_test_provider(Some("realm.roles".to_string()), mapping);

        let claims = serde_json::json!({
            "realm": {
                "roles": ["oidc-admin", "oidc-viewer", "oidc-unknown"]
            }
        });
        let mut result = extract_mapped_roles(&provider, &claims);
        result.sort();

        assert_eq!(
            result,
            vec!["admin", "viewer"],
            "array claim values should be mapped to local role names"
        );
    }

    #[cfg(feature = "oidc")]
    #[test]
    fn extract_mapped_roles_string_claim_value() {
        let mut mapping = HashMap::new();
        mapping.insert("oidc-editor".to_string(), "editor".to_string());
        let provider = build_test_provider(Some("role".to_string()), mapping);

        let claims = serde_json::json!({"role": "oidc-editor"});
        let result = extract_mapped_roles(&provider, &claims);

        assert_eq!(
            result,
            vec!["editor"],
            "string claim value should be mapped to a single local role"
        );
    }

    #[cfg(feature = "oidc")]
    #[test]
    fn extract_mapped_roles_no_matching_claims_returns_empty() {
        let mut mapping = HashMap::new();
        mapping.insert("oidc-admin".to_string(), "admin".to_string());
        let provider = build_test_provider(Some("roles".to_string()), mapping);

        let claims = serde_json::json!({"roles": ["completely-different"]});
        let result = extract_mapped_roles(&provider, &claims);
        assert!(
            result.is_empty(),
            "no matching claim values should return empty vec"
        );
    }
}
