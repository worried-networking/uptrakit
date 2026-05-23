//! Boot-time OAuth configuration validation.
//!
//! Called once at controller startup when `oauth.mcp_enabled = true`.
//! Validates configuration, enforces the multi-controller guard, and
//! registers this process in the `oauth_controller_instances` table.

use rootcause::prelude::*;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, QueryFilter,
    Set, SqliteTransactionMode, TransactionOptions, TransactionTrait,
};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use thiserror::Error;
use time::OffsetDateTime;
use uptrakit_shared_db::entity::oauth_controller_instance;
use uptrakit_shared_macros::impl_report_conversion;
use uuid::Uuid;

const HEARTBEAT_FRESH_SECONDS: i64 = 90;
const STALE_TTL_HOURS: i64 = 24;

/// Configuration supplied by the caller at boot time.
///
/// `#[non_exhaustive]`: new fields may be added as the OAuth implementation
/// grows. Callers must construct via [`OAuthBootSettings::new`].
#[non_exhaustive]
pub struct OAuthBootSettings {
    /// The canonical host for this controller (e.g. `controller.example.com`).
    /// `None` means the setting is absent; boot will fail with
    /// [`OAuthBootError::CanonicalHostMissing`].
    pub canonical_host: Option<String>,
    /// Additional hostnames accepted as valid audience values.
    pub accepted_audience_hosts: Vec<String>,
    /// Raw bytes of the JWT signing secret — used only to derive the
    /// fingerprint. Never stored directly.
    pub jwt_signing_secret: Vec<u8>,
    /// When `true`, a peer with the same fingerprint does not abort boot.
    /// Intended for intentional multi-controller active/active deployments.
    pub allow_multi_controller_unsafe: bool,
}

impl std::fmt::Debug for OAuthBootSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuthBootSettings")
            .field("canonical_host", &self.canonical_host)
            .field("accepted_audience_hosts", &self.accepted_audience_hosts)
            .field("jwt_signing_secret", &"[REDACTED]")
            .field(
                "allow_multi_controller_unsafe",
                &self.allow_multi_controller_unsafe,
            )
            .finish()
    }
}

impl OAuthBootSettings {
    /// Construct a new [`OAuthBootSettings`].
    #[must_use]
    pub fn new(
        canonical_host: Option<String>,
        accepted_audience_hosts: Vec<String>,
        jwt_signing_secret: Vec<u8>,
        allow_multi_controller_unsafe: bool,
    ) -> Self {
        Self {
            canonical_host,
            accepted_audience_hosts,
            jwt_signing_secret,
            allow_multi_controller_unsafe,
        }
    }
}

/// Errors returned by [`validate_and_register`].
#[non_exhaustive]
#[derive(Debug, Error)]
pub enum OAuthBootError {
    #[error("oauth.canonical_host is required when oauth.mcp_enabled is true")]
    CanonicalHostMissing,
    #[error("another controller instance is active with a different signing-secret fingerprint")]
    PeerWithDifferentFingerprint,
    #[error(
        "another controller instance is active with the same fingerprint; \
         set oauth.allow_multi_controller_unsafe = true to permit"
    )]
    PeerWithSameFingerprintNotPermitted,
    #[error("database error")]
    Database(sea_orm::DbErr),
    #[error("OAuth settings error: {0}")]
    Settings(String),
}

pub(crate) type Result<T> = std::result::Result<T, Report<OAuthBootError>>;

impl_report_conversion! {
    sea_orm::DbErr => OAuthBootError::Database,
}

/// Validate OAuth boot configuration and register this controller instance.
///
/// Steps performed:
/// 1. Assert `canonical_host` is non-empty.
/// 2. Derive a SHA-256 fingerprint from `jwt_signing_secret`.
/// 3. Prune rows older than 24 h.
/// 4. Scan for rows with `last_seen_at` within the last 90 s (fresh peers).
/// 5. Reject boot if a peer with a **different** fingerprint is found.
/// 6. Reject boot if a peer with the **same** fingerprint is found and
///    `allow_multi_controller_unsafe = false`.
/// 7. Insert this controller's row.
///
/// Returns the newly generated `instance_id` on success.
///
/// The `now` parameter is injectable so tests can set a deterministic clock
/// without relying on wall-clock time.
///
/// Accepts any [`ConnectionTrait`] implementation so that callers can pass
/// either a bare [`DatabaseConnection`] or an already-open transaction,
/// making the secret-write and peer-check atomic when required.
pub async fn validate_and_register(
    db: &impl ConnectionTrait,
    settings: &OAuthBootSettings,
    now: OffsetDateTime,
) -> Result<Uuid> {
    // 1. canonical_host must be set and non-empty.
    if settings.canonical_host.as_deref().unwrap_or("").is_empty() {
        bail!(OAuthBootError::CanonicalHostMissing);
    }

    // 2. Compute fingerprint.
    let fp = fingerprint(&settings.jwt_signing_secret);
    let instance_id = Uuid::now_v7();

    // 3. Prune rows older than 24 h.
    let stale_cutoff = now - time::Duration::hours(STALE_TTL_HOURS);
    oauth_controller_instance::Entity::delete_many()
        .filter(oauth_controller_instance::Column::LastSeenAt.lt(stale_cutoff))
        .exec(db)
        .await
        .context_to()?;

    // 4. Scan for active rows (last_seen_at within the last 90 s).
    let fresh_cutoff = now - time::Duration::seconds(HEARTBEAT_FRESH_SECONDS);
    let peers = oauth_controller_instance::Entity::find()
        .filter(oauth_controller_instance::Column::LastSeenAt.gte(fresh_cutoff))
        .all(db)
        .await
        .context_to()?;

    // 5. Check peers.
    for peer in &peers {
        if peer.jwt_secret_fingerprint != fp {
            bail!(OAuthBootError::PeerWithDifferentFingerprint);
        }
        if !settings.allow_multi_controller_unsafe {
            bail!(OAuthBootError::PeerWithSameFingerprintNotPermitted);
        }
        tracing::warn!(
            peer_instance_id = %peer.instance_id,
            "multi-controller unsafe mode: peer with same fingerprint active"
        );
    }

    // 6. INSERT this controller's row.
    oauth_controller_instance::ActiveModel {
        instance_id: Set(instance_id),
        jwt_secret_fingerprint: Set(fp),
        started_at: Set(now),
        last_seen_at: Set(now),
    }
    .insert(db)
    .await
    .context_to()?;

    Ok(instance_id)
}

/// Compute a deterministic SHA-256 fingerprint of `secret`.
///
/// A domain-separation prefix is prepended so that the fingerprint cannot
/// be confused with other SHA-256 outputs in the system.
#[must_use]
pub fn fingerprint(secret: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"uptrakit-oauth-controller-fingerprint-v1");
    hasher.update(secret);
    format!("{:x}", hasher.finalize())
}

/// Spawn a background task that updates `last_seen_at` for `instance_id` every 30 s.
///
/// If the row has been deleted (e.g. externally pruned) the task stops itself.
pub fn spawn_heartbeat(db: DatabaseConnection, instance_id: Uuid) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            let now = OffsetDateTime::now_utc();
            let result = oauth_controller_instance::Entity::find_by_id(instance_id)
                .one(&db)
                .await;
            match result {
                Ok(Some(row)) => {
                    let mut active: oauth_controller_instance::ActiveModel = row.into();
                    active.last_seen_at = Set(now);
                    if let Err(e) = active.update(&db).await {
                        tracing::error!(error = %e, "oauth heartbeat update failed");
                    }
                }
                Ok(None) => {
                    tracing::warn!(
                        %instance_id,
                        "oauth heartbeat: instance row not found, stopping heartbeat"
                    );
                    break;
                }
                Err(e) => {
                    tracing::error!(error = %e, "oauth heartbeat query failed");
                }
            }
        }
    });
}

/// Boot-time OAuth state initialisation.
///
/// Reads the DB to resolve whether MCP OAuth should be enabled, then (when
/// enabled) generates or reloads the JWT signing secret and registers this
/// controller instance — all within a single `BEGIN IMMEDIATE` transaction
/// to prevent split-brain on rapid restart loops.
///
/// Returns `OAuthState::disabled()` fast-path when resolved-enabled is false.
pub async fn boot_oauth_state(db: &DatabaseConnection) -> Result<super::OAuthState> {
    // ── Step 1: Read resolve inputs (no transaction needed — reads only) ──
    let mcp_raw: Option<bool> =
        crate::settings_store::load_global_setting_raw(db, "oauth.mcp_enabled")
            .await
            .map_err(|e| report!(OAuthBootError::Settings(e.to_string())))?
            .and_then(|v| v.as_bool());
    let canonical_host_str: Option<String> =
        crate::settings_store::load_global_setting_raw(db, "oauth.canonical_host")
            .await
            .map_err(|e| report!(OAuthBootError::Settings(e.to_string())))?
            .and_then(|v| v.as_str().map(ToOwned::to_owned));

    // ── Step 2: Resolve → fast-path disabled ──
    if !super::resolve_mcp_enabled(mcp_raw, canonical_host_str.as_deref()) {
        return Ok(super::OAuthState::disabled());
    }

    if mcp_raw.is_none() {
        tracing::warn!(
            canonical_host = canonical_host_str.as_deref().unwrap_or(""),
            "OAuth auto-enabling: oauth.mcp_enabled row absent but canonical_host is \
             configured; to disable, write oauth.mcp_enabled = false"
        );
    }

    // ── Step 3: Read-only settings (outside tx) ──
    let accepted_audience_hosts: Vec<String> =
        crate::settings_store::load_global_setting_raw(db, "oauth.accepted_audience_hosts")
            .await
            .unwrap_or(None)
            .and_then(|v| v.as_array().cloned())
            .map(|arr| {
                arr.into_iter()
                    .filter_map(|v| v.as_str().map(ToOwned::to_owned))
                    .collect()
            })
            .unwrap_or_default();
    let allow_multi =
        crate::settings_store::load_global_setting_raw(db, "oauth.allow_multi_controller_unsafe")
            .await
            .unwrap_or(None)
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
    let dcr_enabled = crate::settings_store::load_global_setting_raw(db, "oauth.dcr_enabled")
        .await
        .unwrap_or(None)
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let cimd_enabled = crate::settings_store::load_global_setting_raw(db, "oauth.cimd_enabled")
        .await
        .unwrap_or(None)
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    // ── Step 4: BEGIN IMMEDIATE — secret read-or-generate + peer registration ──
    let tx = db
        .begin_with_options(TransactionOptions {
            sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate),
            ..Default::default()
        })
        .await
        .context_to()?;

    let signing_secret = crate::settings_store::load_or_generate_oauth_signing_secret(&tx)
        .await
        .map_err(|e| report!(OAuthBootError::Settings(e.to_string())))?;

    let boot_settings = OAuthBootSettings::new(
        canonical_host_str,
        accepted_audience_hosts,
        signing_secret,
        allow_multi,
    );
    let instance_id = validate_and_register(&tx, &boot_settings, OffsetDateTime::now_utc()).await?;

    tx.commit().await.context_to()?;

    // ── Step 5: Spawn heartbeat and construct live OAuthState ──
    spawn_heartbeat(db.clone(), instance_id);

    let canonical = super::canonical_url::load_canonical_url_config(db)
        .await
        .map_err(|e| report!(OAuthBootError::Settings(e.to_string())))?;
    let issuer = canonical.issuer().as_str().to_owned();

    Ok(super::OAuthState {
        enabled: true,
        canonical,
        signer: Arc::new(super::jwt::McpOAuthJwtSigner::new(
            &boot_settings.jwt_signing_secret,
        )),
        verifier: Arc::new(super::jwt::McpOAuthJwtVerifier::new(
            &boot_settings.jwt_signing_secret,
            issuer,
            vec![],
        )),
        clock: Arc::new(OffsetDateTime::now_utc),
        instance_id,
        dcr_enabled,
        cimd_enabled,
    })
}

#[cfg(all(test, feature = "db-sqlite"))]
mod tests {
    use super::*;
    use crate::test_harness::{insert_default_tenant, setup_migrated_db};

    fn now() -> OffsetDateTime {
        OffsetDateTime::now_utc()
    }

    fn minimal_settings() -> OAuthBootSettings {
        OAuthBootSettings::new(
            Some("controller.example.com".to_string()),
            vec![],
            b"test-secret".to_vec(),
            false,
        )
    }

    // ──────────────────────────────────────────────────────────────────────
    // Test 1 — missing canonical_host bails
    // ──────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn missing_canonical_host_bails() {
        let db = setup_migrated_db().await;
        let _ = insert_default_tenant(&db).await;
        let settings = OAuthBootSettings::new(None, vec![], b"test-secret".to_vec(), false);
        let err = validate_and_register(&db, &settings, now())
            .await
            .unwrap_err();
        assert!(matches!(
            err.current_context(),
            OAuthBootError::CanonicalHostMissing
        ));
    }

    // ──────────────────────────────────────────────────────────────────────
    // Test 2 — first boot registers instance
    // ──────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn first_boot_registers_instance() {
        let db = setup_migrated_db().await;
        let _ = insert_default_tenant(&db).await;
        let id = validate_and_register(&db, &minimal_settings(), now())
            .await
            .expect("first boot should succeed");
        // Verify row exists.
        let row = oauth_controller_instance::Entity::find_by_id(id)
            .one(&db)
            .await
            .expect("db query should succeed")
            .expect("row must exist");
        assert_eq!(row.instance_id, id);
    }

    // ──────────────────────────────────────────────────────────────────────
    // Test 3 — peer with different fingerprint bails
    // ──────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn peer_with_different_fingerprint_bails() {
        let db = setup_migrated_db().await;
        let _ = insert_default_tenant(&db).await;
        let now_t = now();
        validate_and_register(&db, &minimal_settings(), now_t)
            .await
            .expect("first boot should succeed");
        // Try to register second instance with a DIFFERENT secret.
        let settings2 = OAuthBootSettings::new(
            Some("controller.example.com".to_string()),
            vec![],
            b"different-secret".to_vec(),
            false,
        );
        let err = validate_and_register(&db, &settings2, now_t + time::Duration::seconds(10))
            .await
            .unwrap_err();
        assert!(matches!(
            err.current_context(),
            OAuthBootError::PeerWithDifferentFingerprint
        ));
    }

    // ──────────────────────────────────────────────────────────────────────
    // Test 4 — same fingerprint without unsafe flag bails
    // ──────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn same_fingerprint_without_unsafe_bails() {
        let db = setup_migrated_db().await;
        let _ = insert_default_tenant(&db).await;
        let now_t = now();
        validate_and_register(&db, &minimal_settings(), now_t)
            .await
            .expect("first boot should succeed");
        // Second instance with SAME secret, allow_multi = false.
        let err = validate_and_register(
            &db,
            &minimal_settings(),
            now_t + time::Duration::seconds(10),
        )
        .await
        .unwrap_err();
        assert!(matches!(
            err.current_context(),
            OAuthBootError::PeerWithSameFingerprintNotPermitted
        ));
    }

    // ──────────────────────────────────────────────────────────────────────
    // Test 5 — same fingerprint with unsafe flag warns + continues
    // ──────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn same_fingerprint_with_unsafe_flag_continues() {
        let db = setup_migrated_db().await;
        let _ = insert_default_tenant(&db).await;
        let now_t = now();
        validate_and_register(&db, &minimal_settings(), now_t)
            .await
            .expect("first boot should succeed");
        let settings2 = OAuthBootSettings::new(
            Some("controller.example.com".to_string()),
            vec![],
            b"test-secret".to_vec(),
            true,
        );
        let result =
            validate_and_register(&db, &settings2, now_t + time::Duration::seconds(10)).await;
        assert!(result.is_ok(), "should succeed with unsafe flag");
    }

    // ──────────────────────────────────────────────────────────────────────
    // Test 6 — stale row (>24 h) is pruned and not counted as a peer
    // ──────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn stale_row_is_pruned_not_counted() {
        let db = setup_migrated_db().await;
        let _ = insert_default_tenant(&db).await;
        // Insert an old row manually (last_seen_at = 25 h ago).
        let old_time = now() - time::Duration::hours(25);
        let stale_id = Uuid::now_v7();
        oauth_controller_instance::ActiveModel {
            instance_id: Set(stale_id),
            jwt_secret_fingerprint: Set("different-fingerprint-stale".to_string()),
            started_at: Set(old_time),
            last_seen_at: Set(old_time),
        }
        .insert(&db)
        .await
        .expect("insert stale row");
        // Boot should succeed — stale row is pruned before the peer check.
        let result = validate_and_register(&db, &minimal_settings(), now()).await;
        assert!(
            result.is_ok(),
            "stale row should not block boot: {:?}",
            result.err()
        );
        // Verify the stale row was deleted.
        let count = oauth_controller_instance::Entity::find_by_id(stale_id)
            .one(&db)
            .await
            .expect("db query should succeed");
        assert!(count.is_none(), "stale row must be pruned");
    }
}
