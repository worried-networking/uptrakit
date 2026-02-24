use crate::SettingKey;
use crate::auth::Result;
use crate::settings_store::{RawSettings, RawSettingsExt, upsert_setting};
#[cfg(feature = "oidc")]
use rootcause::prelude::*;
use sea_orm::ConnectionTrait;
#[cfg(feature = "oidc")]
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
#[cfg(feature = "oidc")]
use time::OffsetDateTime;
#[cfg(feature = "oidc")]
use uptrakit_shared_db::MaskedEmail;
#[cfg(feature = "oidc")]
use uptrakit_shared_db::entity::{
    oidc_provider, prelude::*, role, user, user_oidc_link, user_role,
};

/// Global authentication settings (password auth toggle).
/// OIDC config lives in the `oidc_providers` table.
#[derive(Clone, Debug)]
pub struct AuthenticationSettings {
    pub password_auth_enabled: bool,
}

impl Default for AuthenticationSettings {
    fn default() -> Self {
        Self {
            password_auth_enabled: true,
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
        Self {
            password_auth_enabled,
        }
    }

    pub async fn save(&self, db: &impl ConnectionTrait, tenant_id: uuid::Uuid) -> Result<()> {
        upsert_setting(
            db,
            tenant_id,
            SettingKey::PasswordAuthEnabled,
            serde_json::Value::Bool(self.password_auth_enabled),
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
    /// The OIDC provider explicitly reported that the email address is not
    /// verified (`email_verified: false`). Authentication is rejected to
    /// prevent account takeover via an attacker-controlled IdP.
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
    pub email: &'a str,
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

    // Reject explicitly-unverified email addresses before any DB lookup.
    if email_verified == Some(false) {
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
    let existing_user = User::find()
        .filter(user::Column::Email.eq(email))
        .one(db)
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
                && p.deleted_at.is_none()
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
        email: Set(MaskedEmail::new(email.to_string())),
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
        .one(db)
        .await
    {
        let user_role_model = user_role::ActiveModel {
            tenant_id: Set(tenant_id),
            user_id: Set(user_id),
            role_id: Set(user_role_entity.id),
            assigned_at: Set(now),
        };
        let _ = user_role_model.insert(db).await;
    }

    Ok(OidcUserResolution::NewUser(user_id))
}

/// Sync OIDC roles for a user based on provider configuration and ID token claims.
#[cfg(feature = "oidc")]
pub async fn sync_oidc_roles(
    db: &impl ConnectionTrait,
    tenant_id: uuid::Uuid,
    user_id: uuid::Uuid,
    provider: &oidc_provider::Model,
    claims: &serde_json::Value,
) -> Result<()> {
    let role_claim_path = match &provider.role_claim_path {
        Some(path) if !path.is_empty() => path,
        _ => return Ok(()),
    };

    let mapping = &provider.role_mapping.0;
    if mapping.is_empty() {
        return Ok(());
    }

    // Navigate the claims JSON to find the role values
    let claim_value = match navigate_json_path(claims, role_claim_path) {
        Some(v) => v,
        None => return Ok(()),
    };

    // Extract claim values as strings
    let claim_strings: Vec<&str> = match claim_value {
        serde_json::Value::Array(arr) => arr.iter().filter_map(|v| v.as_str()).collect(),
        serde_json::Value::String(s) => vec![s.as_str()],
        _ => return Ok(()),
    };

    // Map OIDC claim values to local role names via the provider's role_mapping
    let local_role_names: Vec<&str> = claim_strings
        .iter()
        .filter_map(|claim| mapping.get(*claim).map(|s| s.as_str()))
        .collect();

    if local_role_names.is_empty() {
        return Ok(());
    }

    // Look up matching local roles
    let local_roles = Role::find()
        .filter(role::Column::Name.is_in(local_role_names))
        .all(db)
        .await
        .context_to()?;

    if local_roles.is_empty() {
        return Ok(());
    }

    // Delete existing user_role rows for this user within this tenant
    UserRole::delete_many()
        .filter(user_role::Column::TenantId.eq(tenant_id))
        .filter(user_role::Column::UserId.eq(user_id))
        .exec(db)
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
        user_role_model.insert(db).await.context_to()?;
    }

    Ok(())
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
        use uptrakit_shared_db::crypto::EncryptedString;

        let client_secret = EncryptedString::new("test-secret".to_string())
            .expect("EncryptedString::new should succeed without master key in test mode");

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
            role_claim_path,
            role_mapping: oidc_provider::RoleMapping(role_mapping),
            is_active: true,
            created_at: OffsetDateTime::UNIX_EPOCH,
            updated_at: OffsetDateTime::UNIX_EPOCH,
            deleted_at: None,
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
