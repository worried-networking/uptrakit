//! Database data migration — `uptrakit-controller db-migrate`.
//!
//! Reads all rows from a source database and bulk-inserts them into a target
//! database. The target schema is created via the normal `run_migrations` path
//! before any data is written.
//!
//! See [`run`] for the top-level entry point.

mod error;
mod tables;

pub(crate) use error::DbMigrateError;
use error::Result;

use std::io::Write as _;
use std::path::Path;
use std::time::Instant;

use rootcause::prelude::*;
use sea_orm::{EntityTrait, PaginatorTrait};

use crate::cli::{Args, DbMigrateArgs};

/// Run the `db-migrate` subcommand.
///
/// This function:
/// 1. Initializes the master encryption key (same as `serve` mode).
/// 2. Connects to `source_db` and `target_db`.
/// 3. Runs schema migrations on the target.
/// 4. Checks that the target has no user data (unless `--force`).
/// 5. Prompts for confirmation (unless `--yes`).
/// 6. Deletes all seeded rows from the target.
/// 7. Copies every application table from source to target.
/// 8. Verifies row counts match on both sides.
/// 9. Prints a summary.
pub(crate) async fn run(args: &Args, migrate_args: &DbMigrateArgs) -> Result<()> {
    let started = Instant::now();

    // ── 1. Master key ────────────────────────────────────────────────────────
    crate::boot::init::init_master_key(args.master_key_from.as_deref())
        .map_err(|e| report!(DbMigrateError::Connection(format!("master key: {e}"))))?;

    // ── 2. Connect ───────────────────────────────────────────────────────────
    let src_url = &migrate_args.source_db;
    let dst_url = &migrate_args.target_db;

    // Validate URL schemes against enabled features before attempting connection.
    validate_url(src_url)
        .map_err(|e| report!(DbMigrateError::Connection(format!("source URL: {e}"))))?;
    validate_url(dst_url)
        .map_err(|e| report!(DbMigrateError::Connection(format!("target URL: {e}"))))?;

    eprintln!(
        "Connecting to source:  {}",
        crate::db::sanitize_url(src_url)
    );
    let src = crate::db::connect(&crate::db::DbConfig {
        url: src_url.to_string(),
        max_connections: crate::db::DEFAULT_MAX_CONNECTIONS,
    })
    .await
    .map_err(|e| report!(DbMigrateError::Connection(format!("source: {e}"))))?;

    eprintln!(
        "Connecting to target:  {}",
        crate::db::sanitize_url(dst_url)
    );
    let dst = crate::db::connect(&crate::db::DbConfig {
        url: dst_url.to_string(),
        max_connections: crate::db::DEFAULT_MAX_CONNECTIONS,
    })
    .await
    .map_err(|e| report!(DbMigrateError::Connection(format!("target: {e}"))))?;

    // ── 2b. Crypto read-init (read-enabling subset of boot's verify_and_migrate) ──
    // Verify the operator's master key can decrypt what is being migrated,
    // then enable ENC:v2/v3 entity reads. Deliberately NOT reencrypt_to_v3 —
    // it writes, and the source must stay untouched (recovery model).
    // Runs before clean_all so a wrong-key run rejects upfront while the target
    // is still intact.
    crate::boot::init::verify_master_key(&src)
        .await
        .map_err(|e| {
            report!(DbMigrateError::Connection(format!(
                "master key verify: {e}"
            )))
        })?;
    crate::reencrypt::register_column_aad_mappings();
    crate::boot::init::init_data_key_ring(&src)
        .await
        .map_err(|e| report!(DbMigrateError::Connection(format!("data key ring: {e}"))))?;

    // ── 3. Schema migrations on target ───────────────────────────────────────
    eprintln!("Running schema migrations on target…");
    crate::migration::run_migrations(&dst)
        .await
        .map_err(|e| report!(DbMigrateError::TargetMigration(e.to_string())))?;

    // ── 4. Safety check ──────────────────────────────────────────────────────
    let user_count = uptrakit_shared_db::entity::prelude::User::find()
        .count(&dst)
        .await
        .map_err(|e| {
            report!(DbMigrateError::Connection(format!(
                "counting target users: {e}"
            )))
        })?;

    if user_count > 0 && !migrate_args.force {
        bail!(DbMigrateError::TargetNotEmpty);
    }

    // ── 5. Confirmation prompt ───────────────────────────────────────────────
    if !migrate_args.yes {
        eprintln!();
        eprintln!("WARNING: This will ERASE ALL DATA in the target database and replace it");
        eprintln!("with data from the source database.");
        eprintln!();
        eprintln!("  Source: {}", crate::db::sanitize_url(src_url));
        eprintln!("  Target: {}", crate::db::sanitize_url(dst_url));
        eprintln!();
        eprint!("Type \"yes\" to continue: ");
        #[expect(
            clippy::let_underscore_must_use,
            reason = "best-effort flush before reading user input; failure here cannot be acted on and the read_line below still proceeds"
        )]
        let _ = std::io::stderr().flush();

        let mut line = String::new();
        std::io::stdin().read_line(&mut line).map_err(|e| {
            report!(DbMigrateError::Connection(format!(
                "reading confirmation: {e}"
            )))
        })?;

        if line.trim() != "yes" {
            bail!(DbMigrateError::MigrationAborted);
        }
    }

    // ── 6. Clean target ──────────────────────────────────────────────────────
    eprintln!("Cleaning target database…");
    tables::clean_all(&dst).await?;

    // ── 7. Copy tables ───────────────────────────────────────────────────────
    eprintln!("Copying tables…");
    let total_rows = tables::copy_all(&src, &dst, migrate_args.batch_size).await?;

    // ── 8. Verify ────────────────────────────────────────────────────────────
    eprintln!("Verifying row counts…");
    let verified = tables::verify_all(&src, &dst).await?;

    // Sanity-check: copy_all and verify_all should agree on totals.
    debug_assert_eq!(
        total_rows, verified,
        "copy total ({total_rows}) and verify total ({verified}) disagree"
    );

    // ── 9. Summary ───────────────────────────────────────────────────────────
    let elapsed = started.elapsed();
    eprintln!();
    eprintln!("Migration complete.");
    eprintln!("  Rows migrated : {total_rows}");
    eprintln!("  Elapsed       : {elapsed:.1?}");

    Ok(())
}

/// Validate that the URL scheme is supported by enabled features.
fn validate_url(url: &str) -> std::result::Result<(), String> {
    crate::db::DbConfig::from_args(
        Some(url.to_owned()),
        Path::new("/"),
        crate::db::DEFAULT_MAX_CONNECTIONS,
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::let_underscore_must_use,
        reason = "test code: discarding `init_master_key` return is idiomatic — it is a no-op on subsequent calls"
    )]

    use crate::cli::{Args, ControllerCommand};
    use clap::Parser;

    #[test]
    fn db_migrate_args_parse() {
        let args = Args::try_parse_from([
            "uptrakit-controller",
            "db-migrate",
            "--source-db",
            "sqlite::memory:",
            "--target-db",
            "sqlite::memory:",
        ])
        .expect("should parse db-migrate subcommand");

        let cmd = args.command.expect("command should be Some");
        match cmd {
            ControllerCommand::DbMigrate(ref db_args) => {
                assert_eq!(db_args.source_db, "sqlite::memory:");
                assert_eq!(db_args.target_db, "sqlite::memory:");
                assert_eq!(db_args.batch_size, 500);
                assert!(!db_args.force);
                assert!(!db_args.yes);
            }
        }
    }

    /// Integration test: migrate all seeded rows from one in-memory SQLite
    /// database to another and verify that row counts match exactly.
    #[tokio::test]
    async fn migrate_sqlite_to_sqlite_roundtrip() {
        use sea_orm::{ConnectOptions, Database};

        // Set up source DB: run migrations so we get the seeded rows
        // (1 tenant, 3 roles, 9 permissions, role_permission links, scheduled tasks).
        let src = {
            let opt = ConnectOptions::new("sqlite::memory:");
            let db = Database::connect(opt).await.expect("source db");
            crate::migration::run_migrations(&db)
                .await
                .expect("source migrations");
            db
        };

        // Set up target DB: run migrations to create the schema (which also
        // inserts seed data), then clean it so we start with a blank slate.
        let dst = {
            let opt = ConnectOptions::new("sqlite::memory:");
            let db = Database::connect(opt).await.expect("target db");
            crate::migration::run_migrations(&db)
                .await
                .expect("target migrations");
            db
        };

        // Wipe the seeded rows from the target so `copy_all` starts clean.
        super::tables::clean_all(&dst).await.expect("clean_all");

        // Copy every table from source to target.
        let copied = super::tables::copy_all(&src, &dst, 500)
            .await
            .expect("copy_all");

        // Verify row counts agree on both sides.
        let verified = super::tables::verify_all(&src, &dst)
            .await
            .expect("verify_all");

        assert_eq!(
            copied, verified,
            "copy_all and verify_all must agree on the total row count"
        );
        assert!(
            copied > 0,
            "at least the migration-seeded rows should have been migrated"
        );
    }

    /// Roundtrip for the tables added by the 2026-07 coverage fix, pinning:
    /// FK-safe insert order actually works (not just names listed), and
    /// `user_totp.secret` is decryptable at the destination (verify() is
    /// count-only by design and cannot catch ciphertext problems; copy moves
    /// ciphertext bytes verbatim under the same master key).
    ///
    /// Master-key note: `init_master_key` is set-once per process; whichever
    /// test initializes it first wins and `let _ =` swallows the
    /// already-initialized error. The assertion is self-consistent under any
    /// winner: encrypt and expose both use the live process-global key.
    #[tokio::test]
    async fn migrate_roundtrip_preserves_sensitive_tables() {
        use sea_orm::{ActiveModelTrait, ConnectOptions, Database, EntityTrait, Set};
        use time::OffsetDateTime;
        use uptrakit_shared_db::entity::prelude::Tenant;
        use uptrakit_shared_db::entity::{
            oauth_authorization_code, oauth_authorization_request, oauth_client, oauth_consent,
            oauth_refresh_token, service, service_merge_redirect, user, user_recovery_code,
            user_totp,
        };
        use uuid::Uuid;

        let _ = uptrakit_crypto::init_master_key(zeroize::Zeroizing::new([0x42u8; 32]));
        // AAD registry: required for the destination read of user_totp.secret.
        // Idempotent across tests — warns on already-initialized, never errors.
        crate::reencrypt::register_column_aad_mappings();

        let src = {
            let opt = ConnectOptions::new("sqlite::memory:");
            let db = Database::connect(opt).await.expect("source db");
            crate::migration::run_migrations(&db)
                .await
                .expect("source migrations");
            db
        };
        let now = OffsetDateTime::now_utc();

        let tenant_id = Tenant::find()
            .one(&src)
            .await
            .expect("query tenant")
            .expect("migrations seed a tenant")
            .id;

        let user_id = Uuid::now_v7();
        user::ActiveModel {
            id: Set(user_id),
            email: Set(uptrakit_shared_types::MaskedEmail::new(
                "mfa-user@example.com",
            )),
            first_name: Set("Mfa".to_string()),
            last_name: Set("User".to_string()),
            password_hash: Set(None),
            is_active: Set(true),
            deactivated_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&src)
        .await
        .expect("insert user");

        let totp_secret = "JBSWY3DPEHPK3PXP";
        user_totp::ActiveModel {
            id: Set(Uuid::now_v7()),
            user_id: Set(user_id),
            secret: Set(uptrakit_crypto::EncryptedString::new(
                totp_secret.to_string(),
                "uptrakit:user_totp:secret",
            )
            .expect("encrypt totp secret")),
            is_active: Set(true),
            enrolled_at: Set(Some(now)),
            last_used_step: Set(None),
            created_at: Set(now),
        }
        .insert(&src)
        .await
        .expect("insert user_totp");

        user_recovery_code::ActiveModel {
            id: Set(Uuid::now_v7()),
            user_id: Set(user_id),
            code_hash: Set("argon2-hash-placeholder".to_string()),
            created_at: Set(now),
            used_at: Set(None),
        }
        .insert(&src)
        .await
        .expect("insert user_recovery_code");

        let client_id = "test-client".to_string();
        oauth_client::ActiveModel {
            id: Set(client_id.clone()),
            client_name: Set("Test Client".to_string()),
            client_uri: Set(None),
            logo_uri: Set(None),
            redirect_uris: Set("[]".to_string()),
            default_scope: Set("mcp:read".to_string()),
            grant_types: Set("[\"authorization_code\"]".to_string()),
            response_types: Set("[\"code\"]".to_string()),
            token_endpoint_auth_method: Set("none".to_string()),
            client_secret_hash: Set(None),
            registration_access_token_hash: Set(None),
            created_via: Set("dcr".to_string()),
            created_at: Set(now),
            last_used_at: Set(None),
            revoked_at: Set(None),
            metadata_cached_at: Set(None),
            metadata_etag: Set(None),
            metadata_content_hash: Set(None),
            metadata_raw: Set(None),
            metadata_parse_error: Set(None),
            metadata_parse_error_at: Set(None),
            trusted_at: Set(None),
        }
        .insert(&src)
        .await
        .expect("insert oauth_client");

        let consent_id = Uuid::now_v7();
        oauth_consent::ActiveModel {
            id: Set(consent_id),
            user_id: Set(user_id),
            client_id: Set(client_id.clone()),
            scopes: Set("mcp:read".to_string()),
            cimd_content_hash_at_grant: Set(None),
            revalidation_required_at: Set(None),
            granted_at: Set(now),
            revoked_at: Set(None),
        }
        .insert(&src)
        .await
        .expect("insert oauth_consent");

        let request_id = Uuid::now_v7();
        oauth_authorization_request::ActiveModel {
            request_id: Set(request_id),
            client_id: Set(client_id.clone()),
            user_id: Set(user_id),
            redirect_uri: Set("https://client.example/cb".to_string()),
            scope: Set("mcp:read".to_string()),
            state: Set("state-placeholder".to_string()),
            code_challenge: Set("challenge-placeholder".to_string()),
            code_challenge_method: Set("S256".to_string()),
            resource: Set(String::new()),
            created_at: Set(now),
            expires_at: Set(now + time::Duration::minutes(10)),
            consumed_at: Set(None),
        }
        .insert(&src)
        .await
        .expect("insert oauth_authorization_request");

        oauth_authorization_code::ActiveModel {
            id: Set(Uuid::now_v7()),
            code_hash: Set("code-hash-placeholder".to_string()),
            request_id: Set(request_id),
            client_id: Set(client_id.clone()),
            user_id: Set(user_id),
            redirect_uri: Set("https://client.example/cb".to_string()),
            scope: Set("mcp:read".to_string()),
            code_challenge: Set("challenge-placeholder".to_string()),
            code_challenge_method: Set("S256".to_string()),
            resource: Set(String::new()),
            issued_at: Set(now),
            expires_at: Set(now + time::Duration::minutes(10)),
            consumed_at: Set(None),
        }
        .insert(&src)
        .await
        .expect("insert oauth_authorization_code");

        oauth_refresh_token::ActiveModel {
            id: Set(Uuid::now_v7()),
            family_id: Set(Uuid::now_v7()),
            parent_id: Set(None),
            token_hash: Set("token-hash-placeholder".to_string()),
            client_id: Set(client_id),
            user_id: Set(user_id),
            consent_id: Set(consent_id),
            scope: Set("mcp:read".to_string()),
            resource: Set(String::new()),
            issued_at: Set(now),
            expires_at: Set(now + time::Duration::hours(1)),
            family_expires_at: Set(now + time::Duration::hours(24)),
            rotated_at: Set(None),
            revoked_at: Set(None),
        }
        .insert(&src)
        .await
        .expect("insert oauth_refresh_token");

        let service_id = Uuid::now_v7();
        service::ActiveModel {
            id: Set(service_id),
            tenant_id: Set(tenant_id),
            capabilities: Set("[]".to_string()),
            hostname: Set("merge-target".to_string()),
            friendly_name: Set("merge-target".to_string()),
            ip_address: Set(None),
            status: Set(uptrakit_shared_types::ServiceStatus::Approved),
            enrollment_secret_hash: Set("hash-placeholder".to_string()),
            client_version: Set(None),
            last_seen_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
            ping_interval_seconds: Set(None),
            enrollment_token_id: Set(None),
            cert_lifetime_hours: Set(None),
            service_app_name: Set(None),
            is_embedded: Set(false),
            embedded_owner_key: Set(None),
        }
        .insert(&src)
        .await
        .expect("insert service");

        service_merge_redirect::ActiveModel {
            source_id: Set(Uuid::now_v7()),
            target_id: Set(service_id),
            redirected_at: Set(now),
        }
        .insert(&src)
        .await
        .expect("insert service_merge_redirect");

        let dst = {
            let opt = ConnectOptions::new("sqlite::memory:");
            let db = Database::connect(opt).await.expect("target db");
            crate::migration::run_migrations(&db)
                .await
                .expect("target migrations");
            db
        };
        super::tables::clean_all(&dst).await.expect("clean_all");
        let copied = super::tables::copy_all(&src, &dst, 500)
            .await
            .expect("copy_all");
        let verified = super::tables::verify_all(&src, &dst)
            .await
            .expect("verify_all");
        assert_eq!(copied, verified);

        assert_eq!(
            user_totp::Entity::find().all(&dst).await.expect("q").len(),
            1
        );
        assert_eq!(
            user_recovery_code::Entity::find()
                .all(&dst)
                .await
                .expect("q")
                .len(),
            1
        );
        assert_eq!(
            oauth_refresh_token::Entity::find()
                .all(&dst)
                .await
                .expect("q")
                .len(),
            1
        );
        assert_eq!(
            service_merge_redirect::Entity::find()
                .all(&dst)
                .await
                .expect("q")
                .len(),
            1
        );

        let dst_totp = user_totp::Entity::find()
            .one(&dst)
            .await
            .expect("query dst user_totp")
            .expect("dst user_totp row");
        assert_eq!(
            dst_totp.secret.expose_secret(),
            totp_secret,
            "TOTP secret must decrypt at the destination after copy"
        );
    }
}
