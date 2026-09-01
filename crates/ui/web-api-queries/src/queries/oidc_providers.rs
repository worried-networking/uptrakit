//! Database query helpers for the `oidc_provider` entity.
//!
//! All mutation helpers accept a `&sea_orm::DatabaseTransaction` opened as
//! `BEGIN IMMEDIATE` by the caller so that the audit row can be written in the
//! same transaction (`emit_stateful`).

use sea_orm::{ActiveModelTrait, EntityTrait, Set};
use std::collections::HashMap;
use time::OffsetDateTime;
use uptrakit_crypto::EncryptedString;
use uptrakit_shared_db::entity::oidc_provider::{self, RoleMapping};
use uptrakit_shared_db::entity::pending_oidc_flow;
use uuid::Uuid;

// ── Audit snapshot ────────────────────────────────────────────────────────────

/// Audit snapshot for an OIDC provider.
///
/// `client_secret` is skipped: it is an encrypted credential and must never
/// appear in audit log snapshots.
#[derive(uptrakit_audit_log::AuditView)]
#[audit(target_type = "oidc_provider")]
pub struct OidcProviderView {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub issuer_url: String,
    pub client_id: String,
    #[audit(skip)]
    pub client_secret: EncryptedString,
    pub scopes: String,
    pub auto_create_users: bool,
    pub allow_private_network_issuers: bool,
    pub role_claim_path: Option<String>,
    pub role_mapping: RoleMapping,
    pub is_active: bool,
    // deactivated_at is auto-skipped by macro (ends in _at and is Option)
}

impl From<&oidc_provider::Model> for OidcProviderView {
    fn from(m: &oidc_provider::Model) -> Self {
        Self {
            id: m.id,
            name: m.name.clone(),
            slug: m.slug.clone(),
            issuer_url: m.issuer_url.clone(),
            client_id: m.client_id.clone(),
            client_secret: m.client_secret.clone(),
            scopes: m.scopes.clone(),
            auto_create_users: m.auto_create_users,
            allow_private_network_issuers: m.allow_private_network_issuers,
            role_claim_path: m.role_claim_path.clone(),
            role_mapping: m.role_mapping.clone(),
            is_active: m.is_active,
        }
    }
}

// ── Parameters ────────────────────────────────────────────────────────────────

/// Parameters for creating a new OIDC provider.
pub struct CreateOidcProviderParams {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub name: String,
    pub slug: String,
    pub logo_url: Option<String>,
    pub issuer_url: String,
    pub client_id: String,
    pub client_secret: EncryptedString,
    pub scopes: String,
    pub auto_create_users: bool,
    pub allow_private_network_issuers: bool,
    pub role_claim_path: Option<String>,
    pub role_mapping: HashMap<String, String>,
    pub now: OffsetDateTime,
}

/// Parameters for partially updating an OIDC provider.
///
/// All fields are `Option` — only `Some` fields are applied.
/// `now` is always written to `updated_at`.
pub struct UpdateOidcProviderParams {
    pub name: Option<String>,
    pub slug: Option<String>,
    pub logo_url: Option<String>,
    pub issuer_url: Option<String>,
    pub client_id: Option<String>,
    pub encrypted_secret: Option<EncryptedString>,
    pub scopes: Option<String>,
    pub auto_create_users: Option<bool>,
    pub allow_private_network_issuers: Option<bool>,
    pub role_claim_path: Option<String>,
    pub role_mapping: Option<HashMap<String, String>>,
    pub now: OffsetDateTime,
}

// ── Transaction-aware helpers (for emit_stateful callers) ─────────────────────

/// Insert a new OIDC provider inside a caller-managed `BEGIN IMMEDIATE` transaction.
///
/// # Errors
///
/// Returns `sea_orm::DbErr` if the insert fails (e.g. unique constraint violation).
pub async fn create_oidc_provider_in_tx(
    tx: &sea_orm::DatabaseTransaction,
    params: CreateOidcProviderParams,
) -> Result<oidc_provider::Model, sea_orm::DbErr> {
    let model = oidc_provider::ActiveModel {
        id: Set(params.id),
        tenant_id: Set(params.tenant_id),
        name: Set(params.name),
        slug: Set(params.slug),
        logo_url: Set(params.logo_url),
        issuer_url: Set(params.issuer_url),
        client_id: Set(params.client_id),
        client_secret: Set(params.client_secret),
        scopes: Set(params.scopes),
        auto_create_users: Set(params.auto_create_users),
        allow_private_network_issuers: Set(params.allow_private_network_issuers),
        role_claim_path: Set(params.role_claim_path),
        role_mapping: Set(RoleMapping(params.role_mapping)),
        is_active: Set(false),
        created_at: Set(params.now),
        updated_at: Set(params.now),
        deactivated_at: Set(None),
    };
    model.insert(tx).await
}

/// Apply a partial update to an OIDC provider inside a caller-managed `BEGIN IMMEDIATE` transaction.
///
/// Accepts pre-computed values for fields that required validation or encryption
/// outside the transaction. Returns the updated model.
///
/// # Errors
///
/// Returns `sea_orm::DbErr` if the update fails.
pub async fn update_oidc_provider_in_tx(
    tx: &sea_orm::DatabaseTransaction,
    provider: oidc_provider::Model,
    params: UpdateOidcProviderParams,
) -> Result<oidc_provider::Model, sea_orm::DbErr> {
    let mut model: oidc_provider::ActiveModel = provider.into();
    if let Some(v) = params.name {
        model.name = Set(v);
    }
    if let Some(v) = params.slug {
        model.slug = Set(v);
    }
    if let Some(v) = params.logo_url {
        model.logo_url = Set(Some(v));
    }
    if let Some(v) = params.issuer_url {
        model.issuer_url = Set(v);
    }
    if let Some(v) = params.client_id {
        model.client_id = Set(v);
    }
    if let Some(v) = params.encrypted_secret {
        model.client_secret = Set(v);
    }
    if let Some(v) = params.scopes {
        model.scopes = Set(v);
    }
    if let Some(v) = params.auto_create_users {
        model.auto_create_users = Set(v);
    }
    if let Some(v) = params.allow_private_network_issuers {
        model.allow_private_network_issuers = Set(v);
    }
    if let Some(v) = params.role_claim_path {
        model.role_claim_path = Set(Some(v));
    }
    if let Some(v) = params.role_mapping {
        model.role_mapping = Set(RoleMapping(v));
    }
    model.updated_at = Set(params.now);
    model.update(tx).await
}

/// Soft-delete an OIDC provider inside a caller-managed `BEGIN IMMEDIATE` transaction.
///
/// Sets `deactivated_at = now`, `is_active = false`, `updated_at = now`.
/// Returns the updated model.
///
/// # Errors
///
/// Returns `sea_orm::DbErr` if the update fails.
pub async fn delete_oidc_provider_in_tx(
    tx: &sea_orm::DatabaseTransaction,
    provider: oidc_provider::Model,
    now: OffsetDateTime,
) -> Result<oidc_provider::Model, sea_orm::DbErr> {
    let mut model: oidc_provider::ActiveModel = provider.into();
    model.deactivated_at = Set(Some(now));
    model.is_active = Set(false);
    model.updated_at = Set(now);
    model.update(tx).await
}

/// Set the `is_active` flag on an OIDC provider inside a caller-managed `BEGIN IMMEDIATE` transaction.
///
/// Used for both activate and deactivate paths.  Returns the updated model.
///
/// # Errors
///
/// Returns `sea_orm::DbErr` if the update fails.
pub async fn set_provider_active_in_tx(
    tx: &sea_orm::DatabaseTransaction,
    provider: oidc_provider::Model,
    is_active: bool,
    now: OffsetDateTime,
) -> Result<oidc_provider::Model, sea_orm::DbErr> {
    let mut model: oidc_provider::ActiveModel = provider.into();
    model.is_active = Set(is_active);
    model.updated_at = Set(now);
    model.update(tx).await
}

// ── Pending flow purge ──────────────────────────────────────────────────────

/// Purges every pending OIDC flow, across all providers, inside a
/// caller-managed `BEGIN IMMEDIATE` transaction.
///
/// Used by the MCP-OAuth settings route when the canonical host changes: a
/// host change invalidates every in-flight OIDC login, not only the flows
/// tied to one provider.
///
/// # Errors
///
/// Returns `sea_orm::DbErr` if the delete query fails.
pub async fn purge_all_pending_flows_in_tx(
    tx: &sea_orm::DatabaseTransaction,
) -> Result<u64, sea_orm::DbErr> {
    pending_oidc_flow::Entity::delete_many()
        .exec(tx)
        .await
        .map(|r| r.rows_affected)
}

#[cfg(all(test, feature = "db-sqlite"))]
mod tests {
    #![expect(
        clippy::expect_used,
        reason = "test helpers: panics on setup failure are acceptable"
    )]

    use super::*;
    use sea_orm::{ConnectOptions, Database, DatabaseConnection, PaginatorTrait};

    async fn setup_test_db() -> DatabaseConnection {
        let opt = ConnectOptions::new("sqlite::memory:");
        let db = Database::connect(opt).await.expect("test db");
        uptrakit_shared_db::migration::run_migrations(&db)
            .await
            .expect("migrations");
        db
    }

    async fn insert_pending_flow(db: &DatabaseConnection, csrf_state: &str) {
        let now = OffsetDateTime::now_utc();
        pending_oidc_flow::ActiveModel {
            csrf_state: Set(csrf_state.to_string()),
            provider_id: Set(Uuid::now_v7()),
            pkce_verifier: Set(EncryptedString::plaintext_for_test(
                "test-verifier".to_string(),
            )),
            nonce: Set("test-nonce".to_string()),
            redirect_uri: Set("https://test.example.com/api/v1/auth/oidc/callback".to_string()),
            return_origin: Set(String::new()),
            created_at: Set(now),
            expires_at: Set(now + time::Duration::seconds(600)),
        }
        .insert(db)
        .await
        .expect("insert pending oidc flow");
    }

    #[tokio::test]
    async fn test_purge_all_pending_flows_in_tx_deletes_every_flow() {
        let db = setup_test_db().await;

        insert_pending_flow(&db, "state-a").await;
        insert_pending_flow(&db, "state-b").await;

        let tx = uptrakit_shared_db::begin_immediate(&db)
            .await
            .expect("begin immediate");
        let rows_affected = purge_all_pending_flows_in_tx(&tx)
            .await
            .expect("purge all pending flows");
        tx.commit().await.expect("commit");

        assert_eq!(rows_affected, 2);
        assert_eq!(
            pending_oidc_flow::Entity::find()
                .count(&db)
                .await
                .expect("count remaining pending flows"),
            0
        );
    }
}
